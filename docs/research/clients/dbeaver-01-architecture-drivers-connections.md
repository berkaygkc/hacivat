# DBeaver Community — Part 1: Architecture, Drivers, Connections & Networking

> Series: `dbeaver-01` (this file) · `dbeaver-02-metadata-and-ddl` · `dbeaver-03-sql-editor-parsing-completion` · `dbeaver-04-results-viewer-data-editor` · `dbeaver-05-data-transfer-localization-lessons`
>
> Audience: the Hacivat team. The reader is assumed **not** to be a database-tooling expert, so every term is explained the first time it appears.

---

## 1. Scope and sources

### 1.1 What this document covers

1. DBeaver's overall architecture: OSGi plugins ("bundles"), the core object model (`DBPDataSourceContainer`, `DBPDataSource`, `DBSObject`, execution contexts), the navigator (tree) model, extension points, how a new database is added, and the generic JDBC fallback.
2. Driver management: driver descriptors, library resolution from Maven, version specifications, class loaders, driver properties, connection URL templates and URL parsing.
3. Connections and networking: the connection configuration model and its persistence, network handlers (SSH tunnels with jump hosts, SOCKS proxy, SSL), authentication models (native, pgpass, shell-command, Windows/AD, cloud), the connection lifecycle, isolated contexts (separate connections for metadata and SQL editors), keep-alive, reconnect/invalidate, transaction modes (auto / manual / "smart" commit), cancellation, read-only mode, driver-property sanitization.
4. Algorithm specifications (ALG-DBV-CONN-xx) for the reusable algorithms, compared with DataGrip and turned into recommendations for Hacivat (Rust), Karagöz and Perde.

Metadata loading, the SQL editor, the result grid and data transfer are covered in parts 02–05.

### 1.2 Sources

| Source | Version / commit | License | How used |
|---|---|---|---|
| DBeaver Community (`github.com/dbeaver/dbeaver`) | commit `f78ac453859084a4654eac0599fb4c0514ef405b` (2026-11-11), shallow clone at `.refs/repos/dbeaver` | Apache-2.0 (`LICENSE.md`) | Source read only; nothing built or executed |
| DBeaver repo docs `AGENTS.md`, `AGENTS-Architecture.md`, `AGENTS-New-Database-Driver.md` | same commit | Apache-2.0 | Architecture conventions |
| DBeaver GitHub issues | queried 2026-11-11 via `gh issue list` | n/a (public discussion) | Weak points, user pain |
| DataGrip 2024.3.5 teardown notes (Turkish) | `.refs/datagrip-extract/report-connectivity.md`, `report-editor.md`, `report-grid.md` | DataGrip proprietary; notes are our own clean-room observations | Comparison only |

Licensing note: DBeaver CE is Apache-2.0, so ideas **and** code may be reused with attribution and a NOTICE entry. This document still uses our own pseudocode; where an exact file would be worth porting, the path is given so a later implementer can decide.

### 1.3 Path abbreviations used in evidence tags

All paths are relative to the repository root `.refs/repos/dbeaver/`.

| Abbrev. | Full path |
|---|---|
| `MODEL/` | `plugins/org.jkiss.dbeaver.model/src/org/jkiss/dbeaver/` |
| `JDBC/` | `plugins/org.jkiss.dbeaver.model.jdbc/src/org/jkiss/dbeaver/model/impl/jdbc/` |
| `REG/` | `plugins/org.jkiss.dbeaver.registry/src/org/jkiss/dbeaver/registry/` |
| `SSH/` | `plugins/org.jkiss.dbeaver.net.ssh/src/org/jkiss/dbeaver/model/net/ssh/` |
| `SSHJ/` | `plugins/org.jkiss.dbeaver.net.ssh.sshj/src/org/jkiss/dbeaver/model/net/ssh/` |
| `PG/` | `plugins/org.jkiss.dbeaver.ext.postgresql/` |
| `MY/` | `plugins/org.jkiss.dbeaver.ext.mysql/` |
| `MS/` | `plugins/org.jkiss.dbeaver.ext.mssql/` |
| `GEN/` | `plugins/org.jkiss.dbeaver.ext.generic/` |
| `SQLED/` | `plugins/org.jkiss.dbeaver.ui.editors.sql/src/org/jkiss/dbeaver/ui/editors/sql/` |
| `TESTP/` | `test/org.jkiss.dbeaver.test.platform/src/org/jkiss/dbeaver/` |

---

## 2. Executive summary

1. **Plugin monolith.** DBeaver CE is ~162 OSGi bundles on Eclipse RCP (Java 21, SWT). Each database is a "model" bundle (`ext.postgresql`) plus an optional "UI" bundle (`ext.postgresql.ui`); model bundles must never depend on UI. Everything is wired by XML extension points in `plugin.xml`. [Observed: `AGENTS.md`, `AGENTS-Architecture.md`, `plugins/` listing]
2. **Three-level object model.** `DBPDataSourceContainer` (the saved connection — config, driver, state; lives forever) → `DBPDataSource` (the live session object created on connect, discarded on disconnect) → `DBSObject` tree (catalogs, schemas, tables, columns…). UI code must never hold a `DBPDataSource` reference, because reconnect replaces it. [Observed: `MODEL/model/DBPDataSource.java:20-27`]
3. **Instances and execution contexts.** A data source has one or more `DBSInstance`s (in PostgreSQL: *one per database*), and each instance owns several **execution contexts** = physical JDBC connections: a *main* context, an optional separate *metadata* context, and *isolated* contexts per SQL editor or data-transfer task. [Observed: `JDBC/JDBCRemoteInstance.java:41-259`]
4. **Database support is declared, not coded.** A database is added through `plugin.xml`: `<datasource>` (provider class, dialect, navigator tree), `<driver>` entries (class, URL template, default port, library files), `<dataSourceType>`, auth models, network handlers. 193 drivers and 72 data-source providers are declared in CE. Many databases need **zero Java code** because they reuse the generic JDBC provider with per-driver `<parameter>` flags. [Observed: `PG/plugin.xml:68-500`, `GEN/plugin.xml`; counted via grep]
5. **Navigator tree is metadata-driven.** The tree (Databases → Schemas → Tables → Columns…) is described in XML (`<tree>/<folder>/<items property="tables">`) and filled by calling Java getters by reflection; `visibleIf` expressions hide nodes per server capability. [Observed: `PG/plugin.xml:140-376`, `MODEL/model/navigator/DBNDatabaseNode.java:1005-1075`]
6. **Drivers come from Maven at runtime.** Library paths such as `maven:/org.postgresql:postgresql:RELEASE[42.7.14]` are resolved against Maven Central (version spec `RELEASE`, `LATEST`, ranges `[a,b)`, regex `{…}`), transitive dependencies are resolved with "nearest wins", files are downloaded to a cache and loaded into a **per-driver `URLClassLoader`**. No checksum/signature verification of downloaded jars was found. [Observed: `REG/driver/DriverLibraryMavenArtifact.java`, `REG/maven/MavenArtifact.java:361-509`, `REG/driver/DriverLibraryAbstract.java:238-285`]
7. **URL templates are a small grammar.** `jdbc:postgresql://{host}[:{port}]/[{database}]` supports parameters `{x}`, optional `[ ]`, alternatives `|`, repeats `...`, named groups `{g:…}`; the same template both *generates* URLs from form fields and *recognizes* pasted URLs (form ↔ URL round-trip). [Observed: `MODEL/model/StringTemplate.java` (1950 lines), `MODEL/model/DatabaseURL.java`, tests `TESTP/model/impl/jdbc/DatabaseURLTest.java`]
8. **Connect pipeline is a fixed sequence**: resolve secrets → BEFORE_CONNECT shell events → credentials (origin → credential provider → password prompt) → proxy handler → tunnel handler (rewrites host/port to `127.0.0.1:<random>`) → provider `openDataSource` → per-context: auth model `initAuthentication` → driver properties → `Driver.connect` (optionally with timeout) → set read-only/auto-commit/isolation → bootstrap SQL → AFTER_CONNECT. [Observed: `REG/DataSourceDescriptor.java:1062-1296`, `JDBC/JDBCDataSource.java:183-329`]
9. **SSH: two engines, shared sessions, jump hosts.** SSH tunnels are pluggable (`sshj` default, `jsch` legacy). Up to 5 jump hosts are chained by port-forwarding through each hop. Sessions to the same SSH destination are **reference-counted and shared** across connections; port forwards are also refcounted. [Observed: `SSH/SSHTunnelImpl.java:200-243`, `SSH/AbstractSessionController.java:50-597`, `SSH/SSHConstants.java`]
10. **Proxy support is a global-JVM hack.** SOCKS proxy is implemented by installing a global `ProxySelector` that guesses which connection is being opened from a thread-local "active context" keyed by host/port. [Observed: `MODEL/runtime/net/GlobalProxySelector.java`, `MODEL/model/impl/net/SocksProxyImpl.java`]
11. **SSL is per-database, not generic.** Each provider ships its own SSL "network handler" that only translates UI fields into driver properties (`sslmode`, `sslrootcert`…); in multi-user mode certs are written to temp files. [Observed: `PG/plugin.xml:807-840`, `PG/src/.../PostgreDataSource.java:276-420`]
12. **Auth models are an extension point** (`dataSourceAuth`): `native` (user/password), `shell_command` (password = stdout of a command — great for cloud IAM tokens), `postgres_pgpass`, SQL Server (Windows, NTLM, AD password/MSI/MFA/integrated, custom), Oracle OS auth, BigQuery/Snowflake/ClickHouse SSO/OIDC, MotherDuck token… Auth models can also bring **extra driver libraries** and get their own class loader. [Observed: `REG/plugin.xml:83-108`, `MS/plugin.xml:615-679`, `REG/driver/DriverDescriptor.java:1185-1245`]
13. **Liveness = 3-second monitor job.** A background job (every 3 s) pings connections that exceed their keep-alive interval, closes idle connections, rolls back idle transactions, and disconnects everything on OS sleep (default on macOS) then re-validates on wake. [Observed: `MODEL/runtime/jobs/DataSourceMonitorJob.java:60-330`]
14. **Recovery = three-phase invalidate.** A failed operation whose error is classified `CONNECTION_LOST` triggers *invalidate*: BEFORE (close) → INVALIDATE (reopen network handlers, then every context) → AFTER; then the operation is retried (default 1 retry). `TRANSACTION_ABORTED` triggers a rollback instead. [Observed: `MODEL/model/exec/DBExecUtils.java:209-306`, `MODEL/runtime/jobs/InvalidateJob.java:142-400`]
15. **"Smart commit"** = stay in auto-commit until the first data-modifying statement, then switch to manual commit; optionally switch back after commit/rollback. Classification is "first keyword not in {SELECT, SHOW, USE, SET, EXPLAIN}", which misclassifies CTEs (`WITH … SELECT`) — an open issue. [Observed: `MODEL/model/exec/DBExecUtils.java:432-454`, `MODEL/model/impl/sql/AbstractSQLDialect.java:59-60,737-769`; issues #14588, #37349]
16. **Cancellation** = `Statement.cancel()` from another thread, escalating (if configured) to `Thread.interrupt()`/driver-specific `cancelQuery()` on the connection. [Observed: `JDBC/JDBCDataSource.java:947-970`, `MODEL/model/runtime/AbstractJob.java:180-290`]
17. **Read-only** is enforced twice: `Connection.setReadOnly(true)` (+ PG `readOnlyMode=always`) and a client-side JSQLParser check in the SQL editor — which lets unparsable statements through. [Observed: `JDBC/JDBCDataSource.java:308-311`, `SQLED/SQLEditor.java:3181-3190`, `MODEL/model/sql/SQLQuery.java:820-838`]
18. **Security lessons.** Saved credentials are AES-encrypted with a **hard-coded key** (obfuscation, not protection) unless an OS secret store is used; headless SSH accepts any host key; downloaded driver jars are not checksum-verified; but DBeaver *does* sanitize dangerous driver properties (MySQL `allowLoadLocalInfile`, `autoDeserialize`…). [Observed: `MODEL/model/impl/app/BaseProjectImpl.java:95,263-265`, `SSHJ/SSHJSessionController.java:128-139`, `MY/src/.../MySQLConnectionValidator.java`]
19. **Biggest structural weakness for us:** connection count explodes (PG: databases × {main, meta} + one per editor), and every driver lives in one JVM. DataGrip isolates drivers in a separate process; Hacivat should isolate per-connection **actors** and keep the connection budget explicit.
20. **Most reusable ideas:** declarative driver/URL catalogue, auth-model plug-ins (esp. shell-command), refcounted SSH session sharing, 3-phase invalidate, monitor job, driver-property sanitizer, connection "types" (dev/test/prod) carrying safety defaults.

---

## 3. Detailed findings

### 3.1 Bundle architecture

**Terms.** *OSGi* is a Java module system: the application is a set of "bundles" (jar files with a `META-INF/MANIFEST.MF` declaring dependencies). *Eclipse RCP* (Rich Client Platform) is the desktop framework DBeaver is built on. An *extension point* is a named "socket" declared in a bundle's `plugin.xml`; other bundles plug *extensions* (XML elements, often naming a Java class) into it. This is how DBeaver adds databases, editors, panels, formatters and so on without the core knowing about them.

- Layout: `plugins/` (162 `MANIFEST.MF` files → ~162 bundles), `features/` (19 Eclipse feature groupings such as `org.jkiss.dbeaver.ce.feature`, `db.feature`), `product/` (product definitions), `test/` (27 test bundles mirroring production bundles). [Observed: `ls plugins features test`, `AGENTS.md`]
- Build: Maven + Tycho; dependencies come from Eclipse P2 repositories rather than Maven; the repo depends on sibling repos `dbeaver-common` and `datadam-api` (`project.deps`). [Observed: `AGENTS.md`, `project.deps`]
- Naming convention for interfaces [Observed: `AGENTS-Architecture.md`]:

| Prefix | Meaning | Examples |
|---|---|---|
| `DBP*` | Platform-level capability | `DBPDataSource`, `DBPDataSourceContainer`, `DBPDriver` |
| `DBS*` | Database **S**tructure (metadata objects) | `DBSObject`, `DBSSchema`, `DBSTable` |
| `DBC*` | **C**onnectivity / execution | `DBCExecutionContext`, `DBCSession`, `DBCStatement` |
| `DBD*` | **D**ata values / formatting | `DBDValueHandler`, `DBDDataFilter` |
| `DBR*` | **R**untime (progress, jobs) | `DBRProgressMonitor` |
| `DBW*` | net**W**ork | `DBWHandlerConfiguration`, `DBWTunnel` |
| `DBA*` | **A**ccess/auth | `DBAAuthModel`, `DBAAuthCredentials` |
| `DBN*` | **N**avigator tree nodes | `DBNDatabaseNode` |
| `DBX*` | navigator tree descriptors from XML | `DBXTreeItem`, `DBXTreeFolder` |

- Core bundles relevant to this document:

| Bundle | Role |
|---|---|
| `org.jkiss.dbeaver.model` | All interfaces + platform-neutral implementations (jobs, QM = query manager, navigator model, SQL model) |
| `org.jkiss.dbeaver.model.jdbc` | JDBC implementation base classes (`JDBCDataSource`, `JDBCExecutionContext`, caches) |
| `org.jkiss.dbeaver.registry` | Registries that read `plugin.xml`: providers, drivers, Maven, auth models, network handlers; connection persistence |
| `org.jkiss.dbeaver.net.ssh` (+ `.sshj`, `.jsch`, `.ui`) | SSH tunnel abstraction + two implementations |
| `org.jkiss.dbeaver.ext.generic` | Generic JDBC provider used by dozens of databases |
| `org.jkiss.dbeaver.ext.<db>` / `.ui` | Database-specific model / UI |
| `org.jkiss.dbeaver.ui.editors.connection` | Connection dialogs and preference pages |

- Extension points that matter for connectivity (declared mostly in `plugins/org.jkiss.dbeaver.registry/plugin.xml:9-22`): `dataSourceProvider`, `dataSourceAuth`, `networkHandler`, `mavenRepository`, `driverManager` (driver categories), `dataSourceHandler` (hooks before connect/after disconnect), `dataSourceStorage`, `language`; plus `org.jkiss.dbeaver.net.ssh` (SSH implementations), `org.jkiss.dbeaver.generic.meta` (generic metadata overrides), `org.jkiss.dbeaver.postgresql.serverType` (PostgreSQL flavours). ~85 extension points exist overall. [Observed: grep of `extension-point` across `plugins/*/plugin.xml`]

**Implication.** The extension-point idea is excellent, but Eclipse's reflective, XML-string-driven wiring is why DBeaver is hard to refactor and slow to start. Hacivat can get the same "declare a database in data" benefit with Rust traits + a compiled-in registry + optional TOML/JSON catalogue files (§5).

### 3.2 Core object model

```
DBPPlatform / DBWorkbench          (singletons: registries, preferences, workspace)
 └─ DBPProject
     └─ DBPDataSourceRegistry       (all saved connections of a project)
         └─ DBPDataSourceContainer  (= DataSourceDescriptor; persistent "connection")
              ├─ DBPDriver          (= DriverDescriptor)
              ├─ DBPConnectionConfiguration (host, port, db, user, props, handlers, bootstrap…)
              ├─ DBVModel           (virtual model: virtual keys, colors – see part 04)
              ├─ DBSObjectFilter[]  (navigator filters per object type)
              └─ DBPDataSource      (live; null when disconnected)
                   ├─ SQLDialect
                   ├─ DBPDataSourceInfo (server/driver capabilities, versions)
                   └─ DBSInstance[]  (PG: one per database; most DBs: one)
                        ├─ main   DBCExecutionContext ─ JDBC Connection
                        ├─ meta   DBCExecutionContext ─ JDBC Connection (optional)
                        └─ isolated contexts (SQL editors, transfer jobs…)
                   └─ DBSObject tree (catalogs/schemas/tables/…; cached, see part 02)
```

- `DBPDataSourceContainer` exposes identity (`getId`, `getDriver`), configuration (`getConnectionConfiguration` vs. `getActualConnectionConfiguration` = the *resolved* copy after tunnels/auth rewrote it), flags (`isConnectionReadOnly`, `isSavePassword`, `isDefaultAutoCommit`, `isForceUseSingleConnection`, `isTemporary`, `isHidden`), filters, virtual model, network handlers, lifecycle (`connect(monitor, initialize, reflect)`, `disconnect`, `reconnect`), task registration (`acquire/release` so disconnect can stop running jobs), events, preference store (per-connection overrides of global prefs), tags and extensions. [Observed: `MODEL/model/DBPDataSourceContainer.java:49-346`]
- `DBPDataSource` is deliberately thin: container, `DBPDataSourceInfo`, `getDataSourceFeature(id)` (string feature flags such as `datasource.limit-affects-dml`, `datasource.lob-require-transactions`, `datasource.max-string-type-length`, `datasource.document-data-source`), `getSQLDialect()`, `initialize(monitor)`. Its Javadoc explicitly warns not to keep references to it from UI because "datasource instance may be refreshed at any moment". [Observed: `MODEL/model/DBPDataSource.java:20-110`]
- `DBPDataSourceProvider` (one per database family) declares features (`FEATURE_CATALOGS`, `FEATURE_SCHEMAS`), lists connection properties (from the JDBC driver's `getPropertyInfo`), builds connection URLs, picks the auth model, and `openDataSource(monitor, container)` creates the live object. [Observed: `MODEL/model/DBPDataSourceProvider.java:22-115`, `JDBC/JDBCDataSourceProvider.java:53-140`]
- `DBSObject` is minimal: name, description, `isPersisted`, `getParentObject()`, `getDataSource()`. Capabilities are added by more interfaces (`DBSObjectContainer.getChildren/getChild/cacheStructure`, `DBSEntity`, `DBPRefreshableObject`, `DBPScriptObject`…). [Observed: `MODEL/model/struct/DBSObject.java:30-46`, `MODEL/model/struct/DBSObjectContainer.java:29-91`]
- `DBSInstance`: `getDefaultContext(monitor, meta)`, `getAllContexts()`, `openIsolatedContext(monitor, purpose, initFrom)`, `shutdown`, `getExclusiveLock`. [Observed: `MODEL/model/struct/DBSInstance.java:30-71`]
- `DBCExecutionContext`: id, name, `openSession(monitor, purpose, task)`, `checkContextAlive`, `invalidateContext(monitor, phase)`, `getContextDefaults()` (current catalog/schema). Sessions are lightweight wrappers opened per operation, tagged with a **purpose** used for logging and query-manager filtering: `USER`, `USER_FILTERED`, `USER_SCRIPT`, `UTIL`, `META`, `META_DDL`. [Observed: `MODEL/model/exec/DBCExecutionContext.java:34-122`, `MODEL/model/exec/DBCExecutionPurpose.java:32-40`]
- Transactions are a capability of the context via `DBCTransactionManager`: isolation get/set, auto-commit get/set, savepoints, commit, rollback, `getDDLTransactionBehavior`, transaction listeners. [Observed: `MODEL/model/exec/DBCTransactionManager.java:29-77`]

**Why the container / live split matters.** It cleanly separates *what the user saved* (survives restarts, can be edited while disconnected) from *what exists only while connected* (sockets, caches). Hacivat should keep exactly this split: `ConnectionProfile` (serde, no I/O) vs. `LiveConnection` (actor handle).

### 3.3 How a new database is added

The repo's own guide [Observed: `AGENTS-New-Database-Driver.md`] and the PostgreSQL plugin show the pattern:

1. Create `plugins/org.jkiss.dbeaver.ext.<db>/` with `MANIFEST.MF`, `plugin.xml`, `pom.xml`; optional `.ui` sibling.
2. Implement `DBPDataSourceProvider` (usually extending `JDBCDataSourceProvider`) and register it under `org.jkiss.dbeaver.dataSourceProvider`.
3. Implement `JDBCDataSource` subclass and an `SQLDialect` (usually extending `JDBCSQLDialect`).
4. Declare the navigator `<tree>` (or inject into the generic tree with `treeInjection`).
5. Declare `<drivers>` with `<driver>` entries.
6. "For many drivers, updating `plugin.xml` alone is enough" — i.e., reuse `ext.generic`.

Anatomy of a driver declaration [Observed: `PG/plugin.xml:380-412`]:

| XML item | Meaning |
|---|---|
| `id`, `label`, `class` | Driver id, display name, JDBC driver class (`org.postgresql.Driver`) |
| `dataSourceType` | Branding/type group (icons, logos) declared by `<dataSourceType>` [`PG/plugin.xml:69-131`] |
| `sampleURL` | URL template (§3.6), e.g. `jdbc:postgresql://{host}[:{port}]/[{database}]` |
| `defaultPort`, `defaultDatabase`, `defaultUser` | Form defaults |
| `supportedConfigurationTypes="MANUAL,URL"` | Whether the connection can be entered as fields, as a raw URL, or both |
| `promoted`, `categories` | Ranking/categorisation in the driver picker |
| `dialect` | SQL dialect id |
| `<replace provider="generic" driver="postgresql"/>` | Migrates connections that used an older driver id to this one |
| `<file type="jar" path="maven:/org.postgresql:postgresql:RELEASE[42.7.14]" bundle="!drivers.postgresql"/>` | Library to download from Maven, *unless* the product ships the `drivers.postgresql` bundle |
| `<file … optional="true" if="properties.gsslib=='sspi'">` | Optional/conditional libraries (e.g. Windows SSPI Kerberos library only if the user selected it) |
| `<file type="license" …>` | License text shown before download |
| `<parameter name="serverType" value="postgresql"/>` | Driver *parameters* read by DBeaver code (not sent to the driver) |
| `<property name="connectTimeout" value="20"/>` | Default JDBC *connection properties* sent to the driver |

Generic-JDBC databases rely heavily on `<parameter>` flags; the most frequent across CE are `query-get-active-db`, `query-set-active-db` (how to read/switch the current schema), `supports-references`, `supports-truncate`, `omit-catalog`, `supports-indexes`, `read-only-meta-data`, `active-entity-type`, `read-only-data`, `supports-multi-insert`, `omit-schema`, `supports-scroll`, `ddl-drop-column-short`, `supports-multiple-results`, `script-delimiter`, `meta-model`, `supports-ddl-transactions`, `identifier-quotes`, `stored-unquoted-case`… [Observed: frequency count of `<parameter name=` over `plugins/*/plugin.xml`]. Example: Derby server declares `query-get-active-db = SELECT CURRENT SCHEMA FROM SYSIBM.SYSDUMMY1`, `query-set-active-db = SET CURRENT SCHEMA = ?`, `dual-table = SYSIBM.SYSDUMMY1`. [Observed: `GEN/plugin.xml` (derby_server driver block)]

Generic metadata quirks are handled by the `org.jkiss.dbeaver.generic.meta` extension: a `<meta>` element maps a driver class (or id) to a `GenericMetaModel` subclass and can even remap JDBC `DatabaseMetaData` result columns by index (`<object type="table"><column id="TABLE_NAME" index="3"/>…`) for drivers that return non-standard result sets. [Observed: `GEN/plugin.xml:1929-1995`, `PG/plugin.xml:63-65`]

**PostgreSQL "server types".** One provider serves many PG-protocol products. `org.jkiss.dbeaver.postgresql.serverType` registers PostgreSQL, EDB, Redshift, TimescaleDB, Yellowbrick, YugabyteDB, CockroachDB, Google Cloud SQL, CrateDB, Materialize, RisingWave (plus Greenplum, GaussDB, Kingbase from other bundles). Each has a Java class with capability methods (`supportsForeignKeys()`, `supportsTriggers()`, `supportsPartitions()`…) and attributes such as `turnOffPreparedStatements` (for PgBouncer compatibility, issue #10742) or `cloudServer`. The navigator tree hides folders with `visibleIf="dataSource.serverType.supportsX()"`. [Observed: `PG/plugin.xml:9-21,147-200`, `PG/src/.../PostgreDataSource.java:301-310`]

> This is the same "family + fallback" idea as DataGrip's `POSTGRES_LIKE` families with `extensionFallback` (YUGABYTE→POSTGRES) [DataGrip report-connectivity §1]. Both tools converge on: *one protocol implementation, many vendors, capability flags per vendor*.

### 3.4 Navigator (tree) model

- The tree is declared per provider in `<tree>`: `<folder type="…PostgreTable" label="%tree.tables.node.name">` contains `<items path="table" property="tables" icon="#table">`; nested `<folder>`/`<items>` describe children; `visibleIf`, `virtual`, `inline`, `navigable`, `recursive="../.."` (partitions of partitions) refine behaviour. Labels use `%key` → `plugin.properties` for localisation. [Observed: `PG/plugin.xml:140-376`]
- At runtime `DBNDatabaseNode.loadChildren` walks the descriptor; for an `<items property="tables">` it finds a Java getter named `getTables` either with no parameters or with a single `DBRProgressMonitor` parameter and calls it by reflection; dotted properties (`defaultInstance.languages`) are resolved step by step. [Observed: `MODEL/model/navigator/DBNDatabaseNode.java:417-530,1005-1075`]
- Object properties shown in property sheets come from `@Property` annotations on getters (AGENTS.md). [Observed: `AGENTS.md` "Annotations"]

**Assessment.** Declarative trees let new databases reuse UI for free, but reflection-by-string means typos fail at runtime ("Can't find property … read method" is only logged as a warning). In Rust, prefer a typed `NodeKind` enum + per-dialect "tree schema" data (folder kinds, child kinds, capability predicates) evaluated against a typed metadata model.

### 3.5 Driver management

#### 3.5.1 Driver descriptor

`DriverDescriptor` holds the XML attributes above *and* user customisations: every field has an `orig*` copy so "Reset to defaults" works (`resetToDefaults`), custom libraries, custom connection properties, `threadSafe` (default true; if false every context uses a single-thread statement lock), `embedded` (file DBs like SQLite — affects separate-connection policy and sleep handling), `singleConnection`, `anonymousAccess`, `allowsEmptyPassword`, `instantiable`, `customDriverLoader`, `licenseRequired`, `supportsDistributedMode`. User overrides are persisted to a drivers config file (`saveDrivers`). [Observed: `REG/driver/DriverDescriptor.java:294-480,1026-1100`; `JDBC/JDBCExecutionContext.java:73-78`]

Library entries are parsed lazily (`pendingLibraryConfigs` → `ensureLibrariesInitialized`) into one of: `DriverLibraryMavenArtifact` (`maven:/…`), `DriverLibraryRepository` (`repo:/…`), `DriverLibraryBundle` (`bundle:/…`), `DriverLibraryRemote` (http URL), `DriverLibraryLocal` (file/dir). A library is skipped entirely if its `bundle` attribute does not match the product (`bundle="drivers.x"` = only when the bundle is shipped; `bundle="!drivers.x"` = only when it is not). [Observed: `REG/driver/DriverDescriptor.java:1026-1045`, `REG/driver/DriverLibraryAbstract.java:82-110`, `REG/driver/DriverUtils.java:59-76`]

This dual mechanism is how the *same* `plugin.xml` serves both the "batteries included" installer (jars pre-bundled) and the slim build (jars downloaded on first connect).

#### 3.5.2 Maven resolution

- Repositories: Maven Central (`https://repo1.maven.org/maven2/`) and Sonatype snapshots by default, extensible via the `mavenRepository` extension point and user-defined repos (not in distributed/server mode), plus a local repo under the install data area; repos can be *scoped* to group ids. [Observed: `REG/plugin.xml:131-139`, `REG/maven/MavenRegistry.java:80-115,227-276`]
- Reference syntax `maven:/group:artifact[:classifier]:fallbackVersion` plus an optional `[preferredVersion]` suffix; e.g. `RELEASE[42.7.14]` means "version spec RELEASE, but prefer 42.7.14". [Observed: `REG/maven/MavenArtifactReference.java:63-96`, `REG/driver/DriverLibraryMavenArtifact.java:116-132`]
- Version spec resolution (`MavenArtifact.resolveVersion`) — see ALG-DBV-CONN-02. Supports `RELEASE` (skips betas), `LATEST`, `SNAPSHOT`, Maven ranges `[1.0,2.0)`, regex in braces `{^42\..*}`, and exact versions. [Observed: `REG/maven/MavenArtifact.java:361-509`]
- Not-found results are cached negatively (`notFoundArtifacts`) until reset. [Observed: `REG/maven/MavenRegistry.java:193-225`]
- Transitive dependencies: breadth-wise expansion with **nearest-wins** mediation by artifact id; duplicates are marked rather than removed; POM-only dependencies skipped; `ignore-dependencies="true"` per file. [Observed: `REG/driver/DriverDependencies.java:45-140`, `REG/driver/DriverLibraryMavenArtifact.java:278-311`]
- Download: to a temp file named by digest, then atomic move into the cache; skip if file exists and size > 0. CRC is computed only for distributed sync. **No SHA-1/SHA-256 or signature verification** of jars. [Observed: `REG/driver/DriverLibraryAbstract.java:238-285`; absence confirmed by grep for `sha1|sha256|checksum` in `REG/maven`, `REG/driver`]
- UI: a download dialog lists missing files and licenses; driver files can be upgraded (`resetVersions`). Users report download failures behind proxies or for specific drivers (#40237, #41442) and asked for automatic updates (#199, open since early days). [Observed: GitHub issues]

#### 3.5.3 Class loading

- `DriverLoaderDescriptor.loadDriver` → `validateFilesPresence` (download if needed) → build a `DriverClassLoader extends URLClassLoader` over the jar URLs, whose parent is the provider bundle's class loader (or a global-libraries loader) → `Class.forName(driverClass)` → instantiate. Zip archives in the library list are extracted to a `zip-cache`. Native libraries (`System.loadLibrary`) are looked up first in driver library directories (`findLibrary`). [Observed: `REG/driver/DriverLoaderDescriptor.java:136-250`, `REG/driver/DriverClassLoader.java:20-79`, `REG/driver/DriverUtils.java:79-90`]
- **One loader per (driver, auth model)**: if an auth model contributes extra libraries (e.g. a cloud SDK), the driver gets a separate loader keyed by the auth-model id. [Observed: `REG/driver/DriverDescriptor.java:1185-1245`]
- All drivers run inside the DBeaver JVM. DataGrip instead runs JDBC drivers in a separate `jdbc-console` JVM with per-driver class loaders and remote proxies [DataGrip report-connectivity §3]; DBeaver has a hook `openConnectionDetached` for a detached process but CE returns `null`. [Observed: `REG/DataSourceDescriptor.java:1106-1109`]

#### 3.5.4 Driver properties

- Effective JDBC properties for a connection = `internal props` (from the data source class, e.g. PG `ApplicationName`, SSL props, `prepareThreshold=0`, `readOnly`) ← `driver default <property>` entries ← `user connection properties`, later entries overwriting earlier. [Observed: `JDBC/JDBCDataSource.java:447-486`, `PG/src/.../PostgreDataSource.java:276-318`]
- The property editor is populated by asking the driver itself (`Driver.getPropertyInfo(url, props)`), excluding `user`/`password`, with provider overrides of defaults (MySQL: `characterEncoding=UTF-8`, `tinyInt1isBit=false`, `interactiveClient=true`). [Observed: `JDBC/JDBCDataSourceProvider.java:53-140`, `MY/src/.../MySQLDataSourceProvider.java:59-97`]
- Smart defaults are encoded in comments with issue numbers, e.g. MySQL `zeroDateTimeBehavior` hack (#4103), PG `prepareThreshold=0` for PgBouncer (#10742). [Observed: `MY/src/.../MySQLDataSource.java:166-168`, `PG/src/.../PostgreDataSource.java:301-310`]

### 3.6 Connection URL templates

**Term.** A JDBC *URL* is the single string a Java driver needs to connect (`jdbc:postgresql://db:5432/app?sslmode=require`). Users either fill a form (host/port/database) or paste a URL; a good client converts in both directions.

- Template mini-language (`StringTemplate`): text, `{param}`, `[optional]`, `a|b` alternatives, `...` repeat, `{group:…}` named groups, `<…>` alternative parameter brackets, `\` escape. It parses into a tree (`Sequence`, `Text`, `Optional`, `Parameter`, `Alternatives`, `Repeat`) and offers: `prepareString(params)` (generate), `extractSingletonParametersMap` (recognize via a compiled Java regex), `extractAllParametersTree` (recursive-descent recognizer for repeats/groups), `replaceParameters(text, map)` (surgical rewrite preserving the rest). [Observed: `MODEL/model/StringTemplate.java:20-46,53-160,433-1400`]
- `DatabaseURL.generateUrlByTemplate`: if the config has a URL and no host/port/server/database, return the URL as-is; otherwise build the parameter map (host, port, server, database also exposed as `folder` and `file`, user only when non-empty) and render. Optional groups whose parameters are missing disappear (so `[:{port}]` vanishes when no port). [Observed: `MODEL/model/DatabaseURL.java:128-185`]
- `DatabaseURL.extractConfigurationFromUrl`: recognise a pasted URL with the driver's template and map params back to host/port/database/server/user/password. [Observed: `MODEL/model/DatabaseURL.java:238-283`]
- Parameter regexes: `{host}`/`{port}`/default `[\w\-_.~]+`, `{file}`/`{folder}` `.+`, query params `[^&=]+` = `[^&]*`, database allows percent-encoding. [Observed: `MODEL/model/DatabaseURL.java:46-55,285-294`]
- `replaceHostAndPort(driver, url, host, port)` (new, tested, not yet wired into the CE connect path): redirect a *custom* URL to a tunnel endpoint, preserving encoded database names, query strings, vendor syntax (`;databaseName=`, Teradata `DBS_PORT=`), IPv6 literals; it **refuses** ambiguous URLs (multi-host, missing port, trailing junk) rather than silently returning the original URL. [Observed: `MODEL/model/DatabaseURL.java:201-236`, `TESTP/model/impl/jdbc/DatabaseURLTest.java:32-130`]
- Known gap: with configuration type `URL`, `DriverDescriptor.getConnectionURL` returns the user's URL verbatim, so the SSH-tunnel host rewrite (`DBWUtils.updateConfigWithTunnelInfo`) does not change a raw URL; users hit "SSH tunnel is not used with Custom connection type" (#38364) and "Connection with URL still uses portnumber" (#38336). `replaceHostAndPort` looks like the in-progress fix. [Observed: `REG/driver/DriverDescriptor.java:1386-1396`, `MODEL/model/net/DBWUtils.java:43-61`; Inferred: link between the new function and these issues]

Compare DataGrip: its template language has `{name:type/regex:default}`, `[...]` optional, `<sep,item>` repetition, `{:identifier}={:param}` pass-through, multiple templates per driver tried in order [DataGrip report-connectivity §1]. Both tools use "one grammar for render + parse". DataGrip's typed parameters with inline regex and defaults are slightly richer; DBeaver's is richer in groups/alternatives and has a real test suite we can mine (§4 ALG-DBV-CONN-01).

### 3.7 Connection configuration model & persistence

`DBPConnectionConfiguration` fields [Observed: `MODEL/model/connection/DBPConnectionConfiguration.java:42-135`]:

| Group | Fields |
|---|---|
| Endpoint | `hostName`, `hostPort`, `serverName`, `databaseName`, `url`, `configurationType` (MANUAL / URL) |
| Credentials | `userName`, `userPassword`, `authModelId`, `authProperties` |
| Driver | `properties` (JDBC props), `providerProperties` (DBeaver-level options per provider) |
| Network | `handlers: List<DBWHandlerConfiguration>` (SSH, proxy, SSL…) |
| Behaviour | `connectionType` (dev/test/prod), `connectionColor`, `keepAliveInterval`, `closeIdleConnection`, `closeIdleInterval` |
| Bootstrap | `DBPConnectionBootstrap`: `defaultCatalogName`, `defaultSchemaName`, `defaultAutoCommit`, `defaultTransactionIsolation`, `initQueries`, `ignoreErrors` |
| Events | `Map<DBPConnectionEventType, DBRShellCommand>`: shell commands BEFORE/AFTER CONNECT/DISCONNECT |
| Profiles | `configProfileSource`, `configProfileName` (shared network/auth profiles) |
| Runtime | `runtimeAttributes` (not persisted) |

Variables usable in templates, events and init queries: `${host}`, `${host.tunnel}`, `${port}`, `${server}`, `${database}`, `${user}`, `${password}`, `${url}`, `${connection.type}`, `${datasource}`, `${project.path}`, `${driver.id}`, `${properties.*}`, `${auth.*}`, `${tag.*}`, `${date}`, `${time}`… [Observed: `MODEL/model/connection/DBPConnectionConfiguration.java:44-96`]

**Connection types** carry *safety defaults* [Observed: `MODEL/model/connection/DBPConnectionType.java:35-130`]:

| Type | Auto-commit | Confirm SQL execute | Confirm data change | Smart commit | Close idle txn after | Close idle connection after |
|---|---|---|---|---|---|---|
| `dev` | yes | no | no | no | 1800 s | 14400 s |
| `test` | yes | no | yes | no | 900 s | 7200 s |
| `prod` | **no** | **yes** | **yes** | no | 600 s | 3600 s |

(Exact column meaning inferred from constructor argument order: `autocommit, confirmExecute, confirmDataChange, smartCommit, smartCommitRecover, autoCloseTransactions, closeIdleTransactionPeriod, autoCloseConnections, closeIdleConnectionPeriod`. [Inferred: field list at lines 110-125 vs. constructor calls])

**Persistence** (`.dbeaver/` folder inside each project): `data-sources*.json` (connections, folders, virtual models, saved filters, connection types, driver overrides, auth profiles, network profiles) and `credentials-config*.json` (secrets). The serializer writes `configuration { host, port, server, database, url, configurationType, home, type, color, keepAlive, closeIdle…, properties, provider-properties, auth-model, events, handlers, bootstrap }`, plus `tags`, `custom-properties`, `extensions`, `security.permission-restrictions`. [Observed: `MODEL/model/app/DBPDataSourceRegistry.java:44-54`, `REG/DataSourceSerializerModern.java:122-292,1108-1360`]

Secrets: `credentials-config.json` is encrypted with AES/CBC using a **constant key embedded in the source** (`LOCAL_KEY_CACHE`), so anyone with the file can decrypt it; projects can opt into a secret storage (`isUseSecretStorage`) backed by Eclipse secure storage / OS keychain. [Observed: `MODEL/model/impl/app/BaseProjectImpl.java:95,252-265`, `MODEL/model/impl/app/DefaultValueEncryptor.java:43-74`, `REG/DataSourceRegistry.java:539,952,1181`]

Compare DataGrip: it splits shareable `dataSources.xml` (commit to git) from machine-local `dataSources.local.xml` and never writes passwords into XML; passwords live in the OS keychain with four retention modes (never / until restart / for session / forever) [DataGrip report-connectivity §2]. DataGrip's split is better; Hacivat should copy it.

### 3.8 Network handlers

**Terms.** A *network handler* is anything between the client and the database server: an *SSH tunnel* (client opens an SSH session to a bastion and asks it to forward a local port to the database host), a *proxy* (SOCKS: a generic TCP relay), *SSL/TLS* (encryption of the database protocol itself). A *jump host* is an intermediate SSH server you must pass through to reach the bastion.

- Extension point `networkHandler`; handler descriptors have `type` = `tunnel` | `proxy` | `config`, `order`, `secured` (has secrets), `desktop` (desktop-only), a property schema and a class. Registered: `ssh_tunnel` (order 1), provider SSL handlers (`postgre_ssl`, `mysql_ssl`, …, order 2, type `config`), `socks_proxy` (order 3), plus ClickHouse/Databend/TiDB-lake/SQL Server specific ones. [Observed: `plugins/org.jkiss.dbeaver.net.ssh/plugin.xml:8-36`, `REG/plugin.xml:59-71`, `PG/plugin.xml:807-840`, `MY/plugin.xml:365-380`]
- Connect order inside `connect0`: proxy handler first (`initializeHandler`), then tunnel (`initTunnelHandler` returns a *rewritten* `DBPConnectionConfiguration`), all under the container's exclusive lock; only one tunnel and one proxy handler are active (`getActiveNetworkHandlers` returns ≤ 2). `config`-type handlers (SSL) are not "opened"; the data source reads their properties when building driver props. [Observed: `REG/DataSourceDescriptor.java:745-755,1186-1240,1298-1335`]

#### 3.8.1 SSH tunnel

- `SSHTunnelImpl.initializeHandler`: pick implementation (`implementation` property; default `sshj`; registry of `SSHSessionControllerDescriptor`s), load host chain = jump hosts (up to `MAX_JUMP_SERVERS = 5`, stop at first disabled) + target host, compute port-forward config (local host/port — `0` → find a free port; remote host/port default to the DB host/port), then `acquireSession` hop by hop, then `DBWUtils.updateConfigWithTunnelInfo` sets host = local host (or `127.0.0.1`) and port = local port and regenerates the URL. [Observed: `SSH/SSHTunnelImpl.java:66-243`, `SSH/SSHUtils.java:205-245`, `SSH/SSHConstants.java:26-52`, `MODEL/model/net/DBWUtils.java:43-61`]
- Auth per hop: sealed type `Password | KeyFile(path, passphrase) | KeyData(inline key, passphrase) | Agent(authSockPath)`. Agent = Pageant on Windows, `SSH_AUTH_SOCK` (or custom socket) elsewhere. Required-credential detection (whether to prompt) currently checks only the **last** host ("TODO: Check jump hosts as well"); jump-host passwords are force-saved. [Observed: `SSH/config/SSHAuthConfiguration.java:20-41`, `SSH/SSHTunnelImpl.java:108-126`, `SSH/SSHUtils.java:343-385`, `SSH/AbstractSessionController.java:130-180`]
- sshj session: connect timeout (default 10 000 ms), keep-alive interval (`aliveInterval`, ms → s), host-key verification against DBeaver's known_hosts file and `~/.ssh/known_hosts*`; **bypassed with a promiscuous verifier in headless mode or when `bypassHostVerification` is set**. [Observed: `SSHJ/SSHJSessionController.java:60-170`]
- Port forward = a Java thread with a `ServerSocket` bound to local host/port, handing each accepted socket to sshj's `LocalPortForwarder`. [Observed: `SSHJ/SSHJSession.java:180-238`]
- **Session sharing** (`AbstractSessionController`): sessions are keyed by `SSHHostConfiguration` (user, host, port, auth); a `ShareableSession` refcounts *which data sources* use it and *how many times*; port forwards are refcounted too. Sharing can be disabled globally (`-Ddbeaver.ssh.disableSessionSharing`) or per handler (`shareTunnels=false`); test connections never share. Jump sessions open a forward on the previous hop to `127.0.0.1:0 → nextHost:22` and then a direct session to that local endpoint. See ALG-DBV-CONN-04. [Observed: `SSH/AbstractSessionController.java:50-597`, `SSH/SSHUtils.java:59`]
- Missing: reading `~/.ssh/config` host aliases/`ProxyJump` (#21394, labelled PRO), multiple forwards per connection for clusters (#19773), socket forwarding (#10273). A recent regression: queries hang during concurrent recovery of data sources sharing an SSH tunnel (#42151). [Observed: GitHub issues]

#### 3.8.2 SOCKS proxy

`SocksProxyImpl` does nothing per connection except ensure a global `ProxySelector` is installed. When the JVM opens any socket, `GlobalProxySelector.select(uri)` calls `DBExecUtils.findConnectionContext(host, port, path)` which looks at a thread-local/global list of containers currently "in context initiation" (`startContextInitiation` … `finishContextInitiation` brackets around every connect) and returns that container's SOCKS settings. Fragile: depends on drivers opening sockets on the same thread and using `java.net.Socket` with proxy selection; drivers with their own networking (Netty, native) bypass it. [Observed: `MODEL/model/impl/net/SocksProxyImpl.java:38-90`, `MODEL/runtime/net/GlobalProxySelector.java:40-150`, `MODEL/model/exec/DBExecUtils.java:84-160`, `JDBC/JDBCExecutionContext.java:110,163`]

#### 3.8.3 SSL/TLS

Per database. PostgreSQL: handler fields CA cert, client cert, client key (as file paths on desktop or inline PEM values), `sslMode` (`disable…verify-full`), `sslFactory`, optional "SSL proxy". The data source converts them into driver props `ssl=true`, `sslrootcert`, `sslcert`, `sslkey`, `sslmode`, `sslfactory`, `sslpasswordcallback`; inline PEMs are written to tracked temp files; client keys go through a key manager that may convert formats. If SSL is not configured the server type may add defaults (`initDefaultSSLConfig`). MySQL: if SSL is off, DBeaver explicitly sets `useSSL=false`. [Observed: `PG/plugin.xml:807-840`, `PG/src/.../PostgreDataSource.java:276-420`, `MY/src/.../MySQLDataSource.java:138-165`]

### 3.9 Authentication models

Extension point `dataSourceAuth`; each `<authModel>` has id, label, class, `default`, `desktop`, `<datasource id/driver/if>` applicability, `<replace model="native"/>` (supersede the generic native model for that provider), property groups for the UI, and optional extra libraries. [Observed: `REG/plugin.xml:83-108`, `MS/plugin.xml:615-679`, `REG/DataSourceAuthModelDescriptor.java`]

Interface `DBAAuthModel<CREDENTIALS>` [Observed: `MODEL/model/access/DBAAuthModel.java:28-137`]:

| Method | When | Purpose |
|---|---|---|
| `createCredentials` / `loadCredentials` / `saveCredentials` / `provideCredentials` | config time | (De)serialize model-specific credential fields |
| `initAuthentication(monitor, ds, creds, config, props)` | every physical connect | Put user/password/tokens into JDBC props; may change host/port/db (URL is then regenerated) |
| `endAuthentication` | after connect attempt | Cleanup (e.g. clear temp tokens) |
| `refreshCredentials` | optional | Re-obtain short-lived tokens (flag `REFRESH_CREDENTIALS_ON_CONNECT=false` in JDBCDataSource) |
| `collectConnectionProperties(…, collectSecuredProps)` | property editor, non-interactive | Show which props the model will add |
| `isCredentialsPromptRequired` | before connect | Whether to ask the user |

Catalogue in CE [Observed: grep of `dataSourceAuth` across `plugins/*/plugin.xml`]:

| Model id | Databases | Notes |
|---|---|---|
| `native` | all | user/password (default) |
| `shell_command` | all (desktop, behind preference `connection.shellAuthModelEnabled`) | Password = first line of stdout of a user command, run at each connect after explicit user approval, with timeout; never persisted. Ideal for `aws rds generate-db-auth-token`, `gcloud sql generate-login-token`, `vault read`. (#39968) [Observed: `MODEL/model/impl/auth/AuthModelDatabaseShellCommand.java:40-150`, tests `TESTP/model/impl/auth/AuthModelDatabaseShellCommandTest.java:90-150`] |
| `postgres_pgpass` | PostgreSQL | Reads `~/.pgpass` / `%APPDATA%/postgresql/pgpass.conf` / `$PGPASSFILE` — ALG-DBV-CONN-07 |
| `sqlserver_database`, `sqlserver_windows`, `sqlserver_ntlm`, `sqlserver_ad_password`, `sqlserver_msi`, `sqlserver_mfa`, `sqlserver_ad_integrated`, `sqlserver_custom` | SQL Server | Maps to MS JDBC `authentication=` / `integratedSecurity=true` / `authenticationScheme=JavaKerberos|NTLM`; some unavailable with jTDS driver via `if="object.getDriverParameter('jtds') == null"` |
| Oracle native / OS auth | Oracle | |
| `google_bigquery`, Spanner | GCP | service account / user auth |
| `snowflake_snowflake` | Snowflake | |
| `clickhouse_cloud_sso`, `clickhouse_oidc` | ClickHouse | SSO/OIDC browser flows |
| `motherduck_token`, `libsql_token_jdbc`, `cloudflare_d1_native`, `cdata_native_url_builder`, `oceanbase_native` | misc | tokens / URL builders |

Kerberos for most databases and AWS/Azure IAM helpers are **paid-edition features** in DBeaver (issues labelled `PRO`, e.g. #21012, #22695, #6232; AWS IAM requests #41332, #39426). [Observed: GitHub issue labels; Inferred: CE/PRO split from labels]

Credential sources, in order, at connect [Observed: `REG/DataSourceDescriptor.java:1146-1185`]:
1. The connection's *origin* (e.g. a cloud catalogue) if it implements `DBACredentialsProvider`.
2. Shared/team credentials (`isSharedCredentials`, secret subject selection).
3. The registry's global `DBACredentialsProvider` (used by server products).
4. Legacy path: if password not saved and driver not anonymous, ask the user (dialog).

### 3.10 Connection lifecycle

Detailed in ALG-DBV-CONN-03. Notable points:

- `connect` refuses to run if `connecting` (a plain boolean, not a lock) or already connected; it validates driver files (may trigger download UI) first. [Observed: `REG/DataSourceDescriptor.java:1062-1104`]
- Shell events: `BEFORE_CONNECT` command (must be approved by the user via `UIServiceShellCommands.validateByUser`), optional wait with timeout, child processes tracked and terminated at disconnect. Bug: invalidate/reconnect does not rerun the before-connect command (#7887). [Observed: `REG/DataSourceDescriptor.java:1471-1520`]
- Provider `openDataSource` → data source constructor opens the main context (and the meta context lazily). With `initialize=true`, `DBPDataSource.initialize` reads server info and base metadata in a META session; if the meta context is in manual-commit mode it is temporarily switched to auto-commit to avoid leaving a transaction open. [Observed: `REG/DataSourceDescriptor.java:1417-1469`]
- Connection open timeout: `connection.open.timeout` (default **0 = no timeout**; the driver's own `connectTimeout`/`loginTimeout` of 20 s for PG are set as driver props). If > 0, the connect runs in a separate task and gives up after `timeout + 2000 ms` (the zombie thread may still finish). [Observed: `MODEL/ModelPreferences.java:188,325`, `JDBC/JDBCDataSource.java:216-286`, `PG/plugin.xml:400-401`]
- Failure path: terminate child processes, shut down partial data source, close tunnel, reset shared credentials, set `connectFailed`, rethrow. [Observed: `REG/DataSourceDescriptor.java:1262-1295`]
- Disconnect: `releaseDataSourceUsers` (calls `beforeDisconnect` on acquirers, cancels running jobs and waits up to 3 s each), BEFORE_DISCONNECT events, optional "sync remote data source", `dataSource.shutdown` (closes all contexts — each close runs on a separate thread with `connection.close.timeout` 5 s; rolls back if not auto-commit), close tunnel, AFTER_DISCONNECT, reset secrets. [Observed: `REG/DataSourceDescriptor.java:1531-1670`, `JDBC/JDBCDataSource.java:496-545`, `MODEL/ModelPreferences.java:191,328`]
- `reconnect` = `disconnect` + `connect` (heavy: rebuilds caches). Lighter recovery is *invalidate* (§3.12).

### 3.11 Execution contexts: main, metadata, isolated

- **Main context** is opened when the instance is created; its name is the client application name. [Observed: `JDBC/JDBCRemoteInstance.java:95-109`]
- **Metadata context** (`initializeMetaContext`): opened on first metadata read when preference `database.meta.separate.connection` is `ALWAYS`, or `DEFAULT` and the driver is not embedded and the connection is not forced single-connection; otherwise the main context is reused. Rationale: metadata queries must not block on, or be blocked by, the user's long-running query or open transaction. [Observed: `JDBC/JDBCRemoteInstance.java:111-139`, `MODEL/ModelPreferences.java:205,338`]
- **Isolated contexts** (`openIsolatedContext(purpose, initFrom)`) are opened by SQL editors (preference `EDITOR_SEPARATE_CONNECTION`, default: separate unless single-connection forced), data transfer producers/consumers, some admin features. `initFrom` copies the active catalog/schema; the editor then restores the schema remembered for that script and commits so the `search_path`/`USE` change doesn't leave a transaction open. [Observed: `SQLED/SQLEditor.java:879-995`, `JDBC/JDBCRemoteInstance.java:149-162`, `PG/src/.../PostgreDataSource.java:440-465`, data transfer: `plugins/org.jkiss.dbeaver.data.transfer/src/.../DatabaseTransferProducer.java:237`, `…/DatabaseTransferConsumer.java:553`]
- **PostgreSQL multiplies this per database**: each `PostgreDatabase` is a `JDBCRemoteInstance` with its own main+meta contexts, opened lazily when the user expands that database (`checkInstanceConnection`) — PG cannot switch database on an open connection. With "show all databases", N databases → up to 2N connections plus editors. Users hit `max_connections` (#36180, infinite loop when lazy loading hits the limit) and asked for separate connections per database for other engines (#17263). [Observed: `PG/src/.../PostgreDatabase.java:56,228-247`; GitHub issues]
- Context names are sent to the server as `ApplicationName` (PG `setClientInfo`) — formatted `"DBeaver <version> - <purpose>"` or a user template with `${context.name}`/`${context.id}`, so DBAs can see "DBeaver 25.x - Main", "- Metadata", "- SQLEditor <Script-3.sql>". [Observed: `MODEL/model/DBUtils.java:2393-2430`, `PG/src/.../PostgreDataSource.java:664`]

Compare DataGrip: *session ≠ connection*; clients (console, grid, introspector) attach to sessions; "single session mode" puts all on one connection; a session manager UI lists/attaches/detaches [DataGrip report-connectivity §3]. DBeaver's model is less flexible (a context is fixed to its consumer) but simpler.

### 3.12 Keep-alive, idle handling, sleep, invalidate, recovery

- `DataSourceMonitorJob` runs every 3 s (system job). For each connected container: skip if a ping is already in flight; first end idle transactions/connections (based on the **last user activity time**, not wall time); then if `keepAliveInterval > 0` and `now − lastCheck > interval`, schedule a `KeepAlivePingJob`. [Observed: `MODEL/runtime/jobs/DataSourceMonitorJob.java:60-330`]
- Ping = for every context `checkContextAlive` → `JDBCUtils.isConnectionAlive`: closed? → skip if any task is active on the container (don't ping a busy connection) → run the dialect's test SQL (e.g. `SELECT 1`) or `Connection.isValid(timeoutSeconds)` (bug #42471 passed milliseconds; now rounded up), fallback `getMetaData().getTables(...fake name...)`; all inside a task with `connection.validation.timeout` (10 s) + 2 s. [Observed: `MODEL/runtime/jobs/KeepAlivePingJob.java:54-100`, `JDBC/JDBCUtils.java:464-520`]
- Dead context → invalidate the *whole* data source; consecutive failures are counted; after > 5 failures the next invalidate disconnects on failure; after 10 the monitor stops pinging that data source. [Observed: `MODEL/runtime/jobs/DataSourceMonitorJob.java:64-65,234-257`, `KeepAlivePingJob.java:66-95`]
- Idle connection close: if `closeIdleConnection` and idle > interval → `DisconnectJob` + notification. Idle transaction end: for each context in manual-commit with `updateCount > 0` started before the last user activity → `EndIdleTransactionsJob` (asks/commits/rolls back). [Observed: `MODEL/runtime/jobs/DataSourceMonitorJob.java:260-320`]
- OS sleep: `connection.closeOnSleep` (default **true on macOS**); on sleep detection disconnect all non-embedded connections; on wake (> 20 s gap between monitor runs) invalidate all connected sources. [Observed: `MODEL/runtime/jobs/DataSourceMonitorJob.java:66-180`, `MODEL/ModelPreferences.java:190,327`]
- **Three-phase invalidate** (`InvalidateJob.invalidateDataSource`): collect dependent data sources (those sharing the same SSH session via `getDependentDataSources`), acquire a per-container task lock (skip if another invalidate already ran — `TASK_PROCESED`), confirm with the UI handler, then for each phase BEFORE / INVALIDATE / AFTER: invalidate network handlers first (SSH: BEFORE = release session, INVALIDATE = reconnect it), then every context of every instance (BEFORE = close, INVALIDATE = reconnect preserving auto-commit/isolation, keeping the context object so editors don't notice). If all contexts fail and `disconnectOnFailure`, disconnect with a "destination database unreachable" error. [Observed: `MODEL/runtime/jobs/InvalidateJob.java:142-330`, `JDBC/JDBCExecutionContext.java:260-271`, `SSH/AbstractSessionController.java:106-124`]
- **Automatic recovery on error** (`DBExecUtils.tryExecuteRecover`): run the operation; on failure ask the dialect's `DBPErrorAssistant.discoverErrorType`; `CONNECTION_LOST` → invalidate data source and retry (`execute.recover.retryCount`, default 1); `TRANSACTION_ABORTED` → roll back the failed transaction (with QM logging disabled to avoid re-triggering smart commit, #9066) and retry; nested calls share a thread-local recovery state so a failed recovery is not retried recursively. [Observed: `MODEL/model/exec/DBExecUtils.java:209-306`, `MODEL/runtime/jobs/InvalidateJob.java:350-400`, `MODEL/ModelPreferences.java:183-184,317-318`]
- User pain: slow recovery after network drops (#6903, open, macOS, performance), lost connection in script editor prevents reconnection (#38030), re-connection popup steals focus (#15848), filters lost after reconnection (#35536). [Observed: GitHub issues]

### 3.13 Transactions: auto, manual, smart

- At context connect: isolation from bootstrap (if set) → `setAutoCommit(default)` → read back actual auto-commit (drivers may refuse) → bootstrap queries → if manual commit, immediately `COMMIT` to release locks taken by init SQL. [Observed: `JDBC/JDBCExecutionContext.java:103-185`]
- `isAutoCommit()` is cached; if unknown it is read in a task with a 5 s timeout, defaulting to *manual* if the connection hangs ("Setting to manual commit mode") — the safe choice. [Observed: `JDBC/JDBCExecutionContext.java:63,351-385`]
- **Smart commit** (per connection type or per connection preference): before each user statement in the SQL editor, if the context is in auto-commit and the statement "modifies" (dialect `isTransactionModifyingQuery`: strip comments, take first keyword, it must be a KEYWORD and not in `{SELECT, SHOW, USE, SET, EXPLAIN}`), switch to manual commit. If that very statement fails, switch back. After the user commits or rolls back, `recoverSmartCommit` may switch back to auto (`smart-commit-recover`). See ALG-DBV-CONN-06. [Observed: `MODEL/model/exec/DBExecUtils.java:432-454,521-566`, `MODEL/model/impl/sql/AbstractSQLDialect.java:59-60,737-769`, `SQLED/execute/SQLQueryJob.java:584-610`, `plugins/org.jkiss.dbeaver.core/src/.../TransactionMonitorToolbar.java:306-322`]
- Weakness: `WITH x AS (…) SELECT …` starts with `WITH` → classified as modifying → opens a transaction (#14588, #37349). The *query manager* (QM) also records whether each executed statement was transactional to show the "pending transaction" counter. [Observed: `MODEL/model/qm/meta/QMMStatementExecuteInfo.java:77`; issues]
- DDL behaviour per dialect (`DBCDDLTransactionBehavior`, preference `transaction.ddl.behavior` default `AUTO`) — whether DDL implicitly commits. [Observed: `MODEL/ModelPreferences.java:286,427`]

DataGrip notes "Smart commit: not present in this version" [DataGrip report-connectivity §7]; DataGrip has Auto/Manual + "commit on close?" + pending-transaction badge. Smart commit is a genuinely nice DBeaver idea but needs a real classifier.

### 3.14 Cancellation

- Long operations run in `AbstractJob`s with a progress monitor that tracks a stack of "blocking objects" (statements, connections). On cancel, the job schedules a `JobCanceler` for the **innermost** block: `JDBCStatementImpl.cancelBlock` → data source `cancelStatementExecute` → `Statement.cancel()` (ignored if unsupported). If `execute.cancel.checkTimeout` > 0 (default 0 = disabled) and the job is still running, the next outer block is cancelled: `JDBCConnectionImpl.cancelBlock` → `cancelCurrentExecution(connection, thread)` → default `Thread.interrupt()`; PostgreSQL overrides with reflective `PgConnection.cancelQuery()`; Oracle/ClickHouse have overrides. [Observed: `MODEL/model/runtime/AbstractJob.java:180-290`, `MODEL/model/runtime/BlockCanceler.java:25-45`, `JDBC/exec/JDBCStatementImpl.java:119-126`, `JDBC/exec/JDBCConnectionImpl.java:718-725`, `JDBC/JDBCDataSource.java:947-970`, `PG/src/.../PostgreDataSource.java:696-704`]
- Admin "kill" exists separately in session managers (MySQL `KILL QUERY ?` / `KILL CONNECTION ?`, SQL Server `KILL <spid>`), not wired as automatic escalation. [Observed: `MY/src/.../session/MySQLSessionManager.java:91-92`, `MS/src/.../session/SQLServerSessionManager.java:86`]

### 3.15 Read-only mode and safety

- Container flag `isConnectionReadOnly`: JDBC `setReadOnly(true)` after connect (unless the driver's read-only is "broken", e.g. ClickHouse), PG `readOnly=true` + `readOnlyMode=always` (driver sends `SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY`), DB2/HANA specific settings. [Observed: `JDBC/JDBCDataSource.java:308-311,835-838`, `PG/src/.../PostgreDataSource.java:313-316`, `plugins/org.jkiss.dbeaver.ext.clickhouse/src/.../ClickhouseDataSource.java:389`]
- SQL editor pre-check: if read-only and any query `isMutatingStatement()` (JSQLParser AST is DROP/DELETE/UPDATE/INSERT/CREATE*/ALTER*/MERGE, or a SELECT that writes) or is DDL → error dialog. If JSQLParser cannot parse the statement, `statement == null` → **not** considered mutating (relies on the server-side flag). [Observed: `SQLED/SQLEditor.java:3170-3190`, `MODEL/model/sql/SQLQuery.java:812-838`]
- Grid and value editors become read-only; navigator shows a lock overlay. [Observed: `plugins/org.jkiss.dbeaver.ui.editors.data/src/.../ResultSetViewer.java:2277`, `MODEL/model/navigator/DBNDataSource.java:192-202`]
- Permissions: `hasModifyPermission(PERMISSION_EXECUTE_SCRIPTS)` and connection-type restrictions (`connectionModifyRestrictions`) — used by team/server products. [Observed: `SQLED/SQLEditor.java:3172-3178`, `MODEL/model/connection/DBPConnectionType.java:126`]

### 3.16 Driver-property sanitization (security)

`JDBCDataSource.validateConnectionProperties` runs **after** auth and substitution but **before** the driver parses the URL, when the app is multi-user or `connection.sanitizeDriverProperties` (default true). `AbstractJDBCConnectionValidator` rejects, case-insensitively and also after one level of percent-decoding, any restricted name found as a word in the URL, property names or values; drivers can whitelist safe values. MySQL's list: `autoDeserialize`, `allowLocalInfile`, `allowLoadLocalInfileInPath`, `allowLoadLocalInfile`, `allowUrlInLocalInfile`, `propertiesTransform`, `useConfigs`, `credentialType` (booleans allowed when `false`/`no`). These properties let a *malicious server* read local files or trigger Java deserialization in the client. See ALG-DBV-CONN-10. [Observed: `JDBC/JDBCDataSource.java:247-256,349-362`, `JDBC/AbstractJDBCConnectionValidator.java:30-100`, `MY/src/.../MySQLConnectionValidator.java:25-48`, `MODEL/ModelPreferences.java:193,329`]

This is directly relevant to Karagöz/Perde: connection definitions that arrive from a remote control plane are *untrusted input*.

---

## 4. Algorithm specifications

### ALG-DBV-CONN-01: Connection URL template — render, recognize, rewrite

- **Problem:** Convert between structured connection fields and a driver URL in both directions, and redirect a URL to a tunnel endpoint without corrupting it.
- **Inputs / outputs:**
  - `render(template, params: Map) -> Result<String>`
  - `recognize(template, url, match_entire: bool) -> Option<ParamTree>`
  - `rewrite_endpoint(template, url, host, port) -> Result<String>`
- **Algorithm:**
  1. *Lex* the template into terms: `TEXT`, `\x` escapes, `{`, `}`, `<`, `>`, `:` (group name separator), `|`, `...`, `[`, `]`.
  2. *Parse* to a tree: `Sequence(name?)`, `Text`, `Parameter(name)`, `Optional(child)`, `Alternatives(branches)`, `Repeat(child)`. `{name}` with no inner structure is a parameter; `{g:…}` is a named group (a sequence whose values are grouped).
  3. *Analyse*: compute for each parameter whether it is mandatory (appears outside any Optional/Alternative) and whether it repeats.
  4. *Render*: visit; `Text` → literal; `Parameter` → next value from the source or **fail this branch** if absent; `Optional` → render child, if it fails emit nothing; `Alternatives` → first branch that succeeds; `Repeat` → render child repeatedly until it fails (consuming successive values); named `Sequence` → pull the next group value-set. Top-level failure → "missing mandatory parameters" error.
  5. *Recognize*: if the template has no repeats/groups, compile to one regex (each parameter → named capture with its parameter-specific regex; Optional → `(?:…)?`; Alternatives → `(?:a|b)`) and match; otherwise run a backtracking recursive-descent matcher over the tree producing a `ParamTree` (params + child groups). With `match_entire=false`, a prefix match is accepted (trailing text ignored).
  6. *Rewrite endpoint*: append a synthetic catch-all parameter `{__suffix}` (`.*`) to the template; use strict regexes for host (`[\p{L}\p{N}_\-.~]+` or bracketed IPv6), port (`[0-9]+`), user/password/database (unreserved or `%XX`); recognize; accept the suffix only if it is empty, or starts with `?`, `;` or `#`, or lies inside an existing query string; then replace exactly the host and port spans (bracket IPv6 host); otherwise **error** ("cannot redirect").
- **Data structures:** template AST (enum), compiled regex cache keyed by template string (DBeaver: `WeakHashMap` under a lock), `ParamTree { params: Map<String, Vec<String>>, groups: Map<String, Vec<ParamTree>> }`.
- **Complexity:** rendering O(template size × values); regex recognition linear in practice; tree recognition can backtrack exponentially on pathological templates (keep templates small; cap input length).
- **Edge cases (exhaustive list):**
  1. Missing optional parameter → whole optional fragment disappears (`[:{port}]`).
  2. Missing mandatory parameter → error, not a half-built URL.
  3. User name absent → `[{user}:{password}@]` disappears (DBeaver changed this deliberately to avoid stray `:`/`@`).
  4. Config has a URL but no fields → return URL verbatim.
  5. Database name with spaces, `/`, `?`, `&`, Unicode, percent-encoding (`my%20db`, `%D0%B1…`).
  6. Query params with empty value (`empty=`), valueless params (`flag`), `=` inside values (`expression=a=b=c`), `;` and spaces inside values, Unicode values.
  7. Malformed query (`=value`, `valid=value&broken`, `&=missing`) → not recognised (entire match).
  8. IPv6 host `[2001:db8::1]`.
  9. Multi-host URLs (`first:5432,second:5432`) → refuse to rewrite.
  10. Template with literal port (`{host}:1234`) → refuse to rewrite (no port parameter to replace).
  11. Same parameter twice in template (`{host}/{host}`) with different values in URL → refuse rewrite.
  12. Vendor-specific separators: `;databaseName=`, Teradata `/DATABASE=x,DBS_PORT=y`, Oracle `@host:port/service`, Snowflake `/?[db=…]`, SQLite `jdbc:sqlite:{file}` with Windows paths and `%USERNAME%`.
  13. Encoded credentials (`p%40ss`, `p%3A%2F…`) must survive; invalid percent escapes (`p%4G`, `p%`) → error.
  14. Unknown scheme vs template (`jdbc:mysql` against a postgres template) → error.
  15. Trailing junk after a recognised URL (`…/demo trailing`, `/demo/other`) → error in rewrite; accepted in prefix-recognition mode.
  16. Empty template → for rewrite fall back to the generic template `[jdbc:]{driver}://[{user}:{password}@]{host}[:{port}][/{database}]`.
  17. Port with leading zeros (`05432`) is preserved on recognition.
  18. Case: `MyDriver://UsEr:PaSs@Host.Name` — case is preserved in untouched parts.
- **Test cases (input → expected)** — taken from DBeaver's tests [Observed: `TESTP/model/impl/jdbc/DatabaseURLTest.java:32-300`, `TESTP/model/impl/jdbc/StringTemplateTest.java:38-230`]:
  1. recognize(`jdbc:postgresql://{host}[:{port}]/[{database}]`, `jdbc:postgresql://localhost:5432/dvdrental`) → host=localhost, port=5432, database=dvdrental.
  2. recognize(`jdbc:teradata://{host}/DATABASE={database},DBS_PORT={port}`, `jdbc:teradata://localhost/DATABASE=test,DBS_PORT=1234`) → host=localhost, database=test, port=1234.
  3. recognize(`jdbc:oracle:thin:@{host}[:{port}]/{database}`, `jdbc:oracle:thin:@localhost/orcl`) → host=localhost, database=orcl, no port.
  4. recognize(`jdbc:sqlserver://{host}[:{port}][;databaseName={database}]`, `jdbc:sqlserver://localhost`) → host only.
  5. recognize(`jdbc:sqlite:{file}`, `jdbc:sqlite:C:\Users\%USERNAME%\Documents\Chinook.db`) → file = whole path.
  6. recognize(`jdbc:mysql://{host}[:{port}]/[{database}]`, `jdbc:mysql://mysql-rfam-public.ebi.ac.uk:4497/Rfam?useSSL=false&serverTimezone=UTC`) (prefix mode) → host, port 4497, database Rfam.
  7. generic-with-params recognize `jdbc:mysql://localhost/demo?empty=&custom.option[0]=a%20b&expression=a=b=c&path=/tmp/данные; x&other=last` → `{empty:"", custom.option[0]:"a%20b", expression:"a=b=c", path:"/tmp/данные; x", other:"last"}`.
  8. generic recognize `…/demo?=value`, `…?valid=value&broken`, `…?valid=value&=missing` (entire) → no match; `jdbc:mysql://localhost/demo` → match.
  9. rewrite(pg template, `jdbc:postgresql://db.example:5432/my%20db%2Fdata?sslmode=require&host=db.example&port=5432&empty=&name=a%2Bb&name=a+b#section`, localhost, 49505) → identical except authority `localhost:49505` (note: `host=`/`port=` *query params* untouched).
  10. rewrite(sqlserver template, `jdbc:sqlserver://db.example:1433;databaseName=master;encrypt=true;password={a;b}`) → `jdbc:sqlserver://localhost:49505;databaseName=master;encrypt=true;password={a;b}`.
  11. rewrite(pg template, `jdbc:postgresql://[2001:db8::1]:5432/данные?option=%2520`) → `jdbc:postgresql://localhost:49505/данные?option=%2520`.
  12. rewrite(snowflake `jdbc:snowflake://{host}[:{port}]/?[db={database}]`, `…db.example:443/?warehouse=wh`) → `jdbc:snowflake://localhost:49505/?warehouse=wh`.
  13. rewrite(empty template, `jdbc:custom://user:p%40ss@db.example:1234/demo?host=db.example&port=1234`) → credentials and query preserved; with `user:p%4Gss` → error.
  14. rewrite(pg template, each of `first:5432,second:5432/demo`, `db.example/demo` (no port), `db.example:invalid/demo`, `db.example:5432extra/demo`, `…/demo trailing`, `…/demo/other`, `jdbc:mysql://…`, `jdbc:postgresql:///demo`, `""`) → error.
  15. render(`[jdbc:]{driver}://[{user}:{password}@]{host}[:{port}][/{database}]`, {driver:wtf, host:myhost, database:dbname}) → `wtf://myhost/dbname` (round trip of test at StringTemplateTest:152).
  16. replaceParameters(`jdbc:teradata://{host}/DATABASE={database}[,DBS_PORT={port}]`, `jdbc:teradata://Host.Name/DATABASE=My_database,DBS_PORT=05432`, {host:127.0.0.1, port:6000}) → `jdbc:teradata://127.0.0.1/DATABASE=My_database,DBS_PORT=6000`.
  17. replaceParameters(`jdbc:test://{host}[:{port}]/{database}`, `jdbc:test://Host.Name/My_database`, {host, port}) → none (port absent in URL so cannot replace).
- **Prior art and evidence:** DBeaver `StringTemplate` + `DatabaseURL` (Apache-2.0; tests are excellent fixtures to port); DataGrip URL templates with typed params, `<sep,item>` repeats and multiple templates per driver [DataGrip report-connectivity §1].
- **Recommendation for Hacivat:** Rust drivers mostly take *structured* configs (tokio-postgres `Config`, mysql_async `OptsBuilder`, tiberius `Config`), so URLs are a UX layer, not the connection mechanism. Implement: (a) a tiny template engine (or per-scheme hand-written parsers using the `url` crate for URL-like schemes + special parsers for SQL Server `;k=v` and Oracle-style), (b) round-trip tests from the list above, (c) a rule: **never pass a user URL to a driver after a tunnel without structured rewrite; refuse when ambiguous** (DBeaver's newest behaviour). Also support libpq `key=value` DSNs and `postgres://` URIs, `mysql://`, `sqlserver://`, `redis://`, `mongodb://`/`mongodb+srv://` because these are what users paste today.
- **Used by:** Hacivat (connection dialog, paste URL, import from other tools), Karagöz (normalise discovered endpoints), Perde (store canonical structured form, never raw URLs with secrets).

### ALG-DBV-CONN-02: Driver library resolution (versions, dependencies, download)

- **Problem:** Given declared driver libraries (some Maven coordinates with version specs), produce a list of local files to load, downloading what is missing, choosing versions, and resolving transitive dependencies.
- **Inputs / outputs:** `libraries: [LibrarySpec]`, product bundle set, platform (OS/arch), repository list → `files: [Path]` or error; side effect: cache populated, driver config saved.
- **Algorithm:**
  1. Filter declared libraries: drop if `bundle` condition fails (`x` requires bundle x present; `!x` requires absent), if `disabled`, or if OS/arch mismatch.
  2. For each downloadable library, check the resolved-file map; if every resolved file exists → nothing to do; else add to download candidates.
  3. Resolve each candidate's version: parse `group:artifact[:classifier]:fallback[preferred]`; spec handling:
     - `RELEASE` → repository metadata `<release>`; if it looks like a beta → ignore; if none → latest of all versions.
     - `LATEST` → metadata `<latest>`.
     - `SNAPSHOT` (only on snapshot repos) → latest snapshot.
     - `{regex}` → newest version matching regex.
     - `[a,b)`/`(a,b]` ranges → newest version inside range (Maven `VersionRange`).
     - anything else → exact version.
     - A `preferredVersion` (from `[…]` suffix or user choice) overrides the spec unless the user forced a version.
  4. Search repositories: current owner's repository first, then all enabled repositories ordered by `order` and filtered by group-id scopes, then repositories declared in the owning POM, then the local repository; cache "not found" results.
  5. Transitive dependencies (unless `ignore-dependencies`): BFS from each root; for each dependency id, keep the node with the **smallest depth** (nearest wins); mark others duplicate; skip POM-only deps; recursively expand non-duplicates. Merge per-root maps.
  6. Show the user the list + licenses (UI), download each file: skip if exists and non-empty; download to `temp/<digest(target)>`, then move into the cache.
  7. Build a class loader over all files (extract zip archives first) with the provider's loader as parent; load and instantiate the driver class; on failure report "required jar files are missing".
- **Data structures:** `MavenArtifactReference`, per-repository metadata cache, `DependencyNode { owner, library, children, depth, duplicate }`, `resolvedFiles: Map<Library, Vec<FileInfo>>`.
- **Complexity:** dominated by network round trips (metadata + POM per artifact). Large drivers (Snowflake, BigQuery, Athena) pull 50–150 jars.
- **Edge cases (exhaustive list):**
  1. Offline machine with no cached files → connection impossible; must explain clearly.
  2. Proxy / corporate Maven mirror → need custom repositories and auth.
  3. Repository returns HTML error page with 200 → saved as a "jar" (no checksum validation in DBeaver).
  4. Partially downloaded file after crash → avoided by temp + move.
  5. Same artifact needed in two versions by two roots → last root wins in merged map (DBeaver) → classpath conflict.
  6. `RELEASE` metadata points to a beta → skipped.
  7. Version list empty → error.
  8. Regex spec invalid → error "Bad version pattern".
  9. Optional dependencies → only when `load-optional-dependencies`.
  10. User pins a version (`forcedVersion`) → never auto-upgrade.
  11. License required and not accepted → refuse to load.
  12. Native libraries (`.dll/.so/.dylib`) inside driver dirs → `findLibrary` lookup.
  13. Distributed/server mode → files synchronised from a server store instead of downloaded.
  14. Two auth models adding different SDKs → separate class loaders per auth model.
  15. Driver class name changes between versions (DataGrip notes ClickHouse) → descriptor must allow per-version class names.
- **Test cases:**
  1. `RELEASE[42.7.14]` with metadata release 42.7.15 and no user pin → preferred 42.7.14 used. [Inferred from `initArtifactReference` + `getArtifactVersion`]
  2. Spec `[1.0,2.0)` with versions {0.9, 1.0, 1.5, 2.0} → 1.5.
  3. Spec `{^42\.7\..*}` with {42.6.0, 42.7.3, 42.7.10} → 42.7.10 (semantic, not lexicographic, comparison).
  4. Dependency graph A→B(1.0), A→C→B(2.0) → B 1.0 (depth 1 beats depth 2).
  5. `bundle="!drivers.postgresql"` when product contains `drivers.postgresql` → library skipped.
- **Prior art and evidence:** `REG/driver/*`, `REG/maven/*` (Apache-2.0). DataGrip: artifact catalogue `database-artifacts.xml` (64 artifacts / 224 versions), channels, driver↔server version constraints, `BestDriverValidator` [DataGrip report-connectivity §5].
- **Recommendation for Hacivat:** Native Rust drivers are compiled in, so this mostly disappears — a strong advantage (no first-connect download, no JVM). Keep the *idea* for: (a) optional out-of-process "driver plugins" (Oracle Instant Client, ODBC, DuckDB extensions, JDBC bridge for exotic DBs via a sidecar), (b) downloadable dialect/catalog data packs. Requirements: SHA-256 pinned in a signed manifest, HTTPS only, atomic writes, offline-first cache, explicit license acceptance, and per-version metadata (class/entry point may change by version).
- **Used by:** Hacivat (plugin/sidecar manager), Karagöz (if it ever embeds helpers), Perde (hosts signed manifests).

### ALG-DBV-CONN-03: Connection open pipeline

- **Problem:** Turn a saved connection profile into live, configured physical connections, applying secrets, auth, network handlers and defaults in the right order, with clean rollback on failure.
- **Inputs / outputs:** `ConnectionProfile`, secret store, UI callbacks → `LiveDataSource` with main (and lazily meta) contexts, or error with all partial resources released.
- **Algorithm (DBeaver order):**
  1. Guard: if connecting or connected → return false.
  2. Ensure driver files present (may download, ALG-02).
  3. Resolve secrets (secret store / credentials file); select shared credentials if team mode.
  4. Copy config → `resolved` (never mutate the saved one).
  5. Run BEFORE_CONNECT shell command (after user approval; optionally wait with timeout); call `dataSourceHandler.beforeConnect` extensions.
  6. Credentials: origin provider → global provider → prompt (if not saved and not anonymous); cancel → abort.
  7. Provider hook `patchConnectionProperties`.
  8. Under the container's exclusive lock: init proxy handler; init tunnel handler (prompt for SSH secrets if needed; returns rewritten config with local host/port and regenerated URL).
  9. `provider.openDataSource(resolved)` → constructor creates instance(s) → main context connect:
     1. Build props: internal (provider) ← driver defaults ← user props.
     2. URL = explicit URL or template render; apply driver substitution if configured.
     3. Pick auth model; check applicability; `loadCredentials`; `initAuthentication` (may change host/port/db → regenerate URL).
     4. `fixupConnectionProperties` (e.g. SSL key manager), `validateConnectionProperties` (sanitizer, ALG-10), provider `validateConnectionParameters`.
     5. Create driver instance; `acceptsURL` (only a warning flag); `Driver.connect(url, props)` (with timeout task if configured).
     6. `endAuthentication`, configurer `afterConnection`.
     7. If read-only: `setReadOnly(true)`.
     8. Set isolation (if configured) and auto-commit; read back real auto-commit.
     9. Bootstrap queries (variables substituted, validated; errors fatal unless `ignoreErrors`).
     10. Register context; init context state (default schema, copy from `initFrom`); if manual commit → `COMMIT`.
  10. If `initialize`: open META session, temporarily force auto-commit, `dataSource.initialize()` (server info, base metadata), dialect `afterDataSourceInitialization`.
  11. If monitor cancelled meanwhile → treat as failure.
  12. AFTER_CONNECT events. Mark connect time.
  13. On any failure: terminate child processes, shutdown partial data source, close tunnel, reset shared credentials, record error, rethrow.
- **Data structures:** `ConnectionProfile` (immutable), `ResolvedConfig` (copy), handler stack with cleanup closures, `ConnectError { stage, cause, user_message_key }`.
- **Complexity:** network-bound; typically 2–6 round trips for the DB plus SSH handshake per hop.
- **Edge cases (exhaustive list):**
  1. Auth model changes host/port (cloud SQL connector) after the tunnel already rewrote them → must not undo tunnel rewrite (DBeaver regenerates URL from the resolved config; order matters).
  2. Password not saved + SSH key passphrase not saved → two prompts; cancel in either aborts cleanly.
  3. Driver ignores `connectTimeout` → open-timeout task needed; zombie thread may later succeed → must close that late connection.
  4. Driver returns `null` connection with `acceptsURL=false` → "Invalid JDBC URL" message.
  5. Auto-commit cannot be set → continue, read actual state.
  6. Bootstrap query fails → fatal or warning by flag; failed bootstrap in manual mode leaves transaction → COMMIT/ROLLBACK.
  7. Default schema no longer exists → warn, keep server default.
  8. Server requires password change on login (PG 28P01, MSSQL 18488) → DataGrip has a dedicated flow; DBeaver has `DBAUserPasswordManager` adapters. [Observed: `PG/src/.../PostgreDataSource.java:685-692` (PostgresUserPasswordManager)]
  9. Cancel pressed during handshake → partial resources released.
  10. Concurrent connect from two UI actions → guard flag; DBeaver's `connecting` is a non-atomic boolean (race-prone).
  11. Init-time metadata read fails (insufficient privileges on catalog tables) → connect should still succeed with degraded metadata.
  12. Shell command hangs → timeout and terminate.
- **Test cases:**
  1. Profile with SSH tunnel + wrong DB password → error at stage `Driver.connect`, tunnel closed, no listening local port remains.
  2. Profile with `bootstrap.initQueries = ["SET search_path TO app"]`, manual commit → after connect `SHOW search_path` = `app` and no open transaction.
  3. Read-only profile on PG → `SHOW default_transaction_read_only` = `on` in the new session.
  4. Cancel during SSH auth prompt → `connect` returns false, no data source.
  5. Driver that ignores timeouts with `open_timeout=5s` → error after ~7 s, late connection closed when it arrives.
- **Prior art and evidence:** `REG/DataSourceDescriptor.java:1062-1470`, `JDBC/JDBCDataSource.java:183-329`, `JDBC/JDBCExecutionContext.java:103-185`. DataGrip: ordered *connection interceptor* chain (PasswordExpiration, PgPass, Credentials, NoCredentials, AwsProfile, SSH, SSL, Cloud SQL proxy…) [DataGrip report-connectivity §2–3].
- **Recommendation for Hacivat:** Model the pipeline as DataGrip-style ordered `ConnectStep`s (each returns an optional async cleanup guard) — easier to extend and test than DBeaver's hard-coded method. Steps: `ResolveSecrets → ShellHooks → Credentials → Transport (direct | SSH chain | SOCKS | Karagöz-P2P) → Tls → AuthModel → Sanitize → DriverConnect → SessionInit(read-only, tx mode, isolation, app name, bootstrap) → Introspect(lazy)`. Use structured errors with a `stage` so the UI can say "SSH hop 2 failed: host key changed" in EN/TR. Run each live connection as a tokio actor owning the socket; connect timeouts via `tokio::time::timeout` + guaranteed drop of late connections.
- **Used by:** Hacivat (all connections), Karagöz (its own connections to local DBs use the same pipeline minus UI prompts).

### ALG-DBV-CONN-04: SSH tunnel chain with jump hosts and shared sessions

- **Problem:** Reach a database through 0–5 jump hosts and a bastion, exposing a local endpoint; share SSH sessions and forwards among connections that use the same bastion.
- **Inputs / outputs:** `hosts = [jump1..jumpN, target]` each `{user, host, port, auth}`, forward `{local_host, local_port(0=auto), remote_host, remote_port}`, `owner` (connection id) → local endpoint `(host, port)` + handle for release/invalidate.
- **Algorithm:**
  1. For i in 0..len(hosts):
     - i == 0: `direct = sharedSessions.get(hosts[0])` if sharing allowed else new; wrap as `DirectSession(shared, forward_if_last)`.
     - i > 0: `JumpSession(origin=sessions[i-1], dest=hosts[i], forward_if_last)`.
     - `connect`:
       - Shared session: if it has no users → really connect (TCP, host key check, auth) and register in `sharedSessions`; add/increment `users[owner]`.
       - Jump session: on previous hop set up forward `127.0.0.1:0 → hosts[i].host:hosts[i].port`; create a direct (shareable) session to that local endpoint (re-using auth of hosts[i]); if this is the last hop, set up the DB forward on it.
       - Port forward on shareable session: if an identical forward exists → increment its usage and reuse resolved local port; else bind a listener, start forwarding, record usage=1.
  2. Rewrite the connection config host/port to the resolved local endpoint.
  3. Release (`disconnect` for owner): remove forwards (decrement; close listener at 0); decrement `users[owner]` (remove at 0); if no users → unregister and close SSH session; for jump sessions also release the previous hop's forward and session.
  4. Invalidate: BEFORE → release; INVALIDATE → reconnect the same chain (re-register).
  5. Dependents: the set of owners of the shared session = connections that must be invalidated together.
- **Data structures:** `sharedSessions: Map<HostKey, ShareableSession>`, `ShareableSession { users: Map<OwnerId, AtomicCount>, forwards: Map<ForwardSpec, (Resolved, AtomicCount)>, client }`.
- **Complexity:** O(hops) handshakes on first connect; O(1) for subsequent connections to the same bastion.
- **Edge cases (exhaustive list):**
  1. Local port fixed by user and already in use → bind error; auto (0) avoids.
  2. Acquire fails at hop k → earlier hops leak (DBeaver comment: "previously acquired sessions will not be released") → must release in reverse order.
  3. Same bastion, different credentials → must be different keys (DBeaver keys by full host config including auth).
  4. Shared session dies → all dependent connections fail simultaneously → invalidate them as a group (DBeaver does via `getDependentDataSources`), and avoid concurrent re-invalidation (DBeaver task lock `TASK_PROCESED`; regression #42151 shows how tricky this is).
  5. Host key unknown/changed → prompt (desktop) vs reject; never auto-accept silently (DBeaver headless auto-accepts — don't copy).
  6. Encrypted key without saved passphrase → prompt; DBeaver only checks the last hop for required prompts (TODO in code).
  7. Agent auth with no agent running → clear error.
  8. Keep-alive must be configured on every hop; idle NATs drop sessions.
  9. Remote host is `localhost` (DB on the bastion itself) → forward to `127.0.0.1` *on the bastion*.
  10. Cluster databases (Redis Cluster, Mongo replica sets) return internal node addresses → one forward is not enough (#19773; DataGrip requires all nodes in the URL, one forward each).
  11. Test connection must not share sessions (DBeaver: container null → no sharing).
  12. IPv6 bastions.
- **Test cases:**
  1. Two connections A, B via the same bastion → one SSH TCP connection; closing A keeps B working; closing B closes SSH.
  2. A and B forward to the same DB host:port → one listener, two users; release order A,B and B,A both end with listener closed.
  3. Chain jump1→jump2→bastion with jump2 auth failure → error names hop 2; no listener, no open SSH sessions remain.
  4. Kill the bastion SSH session → both A and B report connection lost; one invalidate reconnects SSH once and both contexts recover.
- **Prior art and evidence:** `SSH/SSHTunnelImpl.java:200-243`, `SSH/AbstractSessionController.java:50-597`, `SSHJ/SSHJSession.java`, `SSHJ/SSHJSessionController.java`. DataGrip: global SSH configs referenced by id, per-URL-host forwards, shared tunnel manager, retry + re-prompt [DataGrip report-connectivity §2]. Its Rust recommendation: refcounted manager keyed by (ssh config, remote host, remote port), `127.0.0.1:0` default [DataGrip report-connectivity §6].
- **Recommendation for Hacivat:** `russh` + `russh-keys`/`ssh-key`; support password, key file, inline key, agent, and **read `~/.ssh/config` incl. `ProxyJump`** (DBeaver lacks this; users ask for it). Prefer *in-process streams* over local TCP listeners: `russh` `channel_open_direct_tcpip` gives an `AsyncRead+AsyncWrite` stream; tokio-postgres (`connect_raw`), mysql_async (custom stream support is limited — fall back to a loopback listener), tiberius (accepts any `AsyncRead+AsyncWrite`) can use it directly → no local port exposed to other local users. Keep refcounted session sharing and group invalidation; release in reverse order on partial failure; known_hosts with explicit trust-on-first-use prompt (EN/TR strings).
- **Used by:** Hacivat; Karagöz (if it reaches DBs on other hosts); the same "transport" abstraction is where the **Karagöz P2P link** plugs in.

### ALG-DBV-CONN-05: Liveness monitor, invalidate and automatic recovery

- **Problem:** Detect dead connections, keep idle ones alive, end idle transactions, and transparently recover from network loss without losing the user's editor state.
- **Inputs / outputs:** set of live data sources, per-connection keep-alive/idle settings, last user-activity time, OS sleep signal → pings, invalidations, disconnections, notifications.
- **Algorithm:**
  1. Every 3 s (if not shutting down): detect OS sleep; on entering sleep and `closeOnSleep` → disconnect all non-embedded; if > 20 s since last tick (woke up) → invalidate all.
  2. For each connected data source not currently being pinged:
     1. If idle-disconnect enabled and `now − lastUserActivity > closeIdleInterval` → disconnect (notify).
     2. Else if idle-transaction timeout and some context is in manual commit with `updateCount > 0` and its transaction started before last user activity, and no execution in progress → end idle transactions job.
     3. Else if `keepAlive > 0` and `now − lastCheck > keepAlive` → ping job (unless failures ≥ 10).
  3. Ping job: for each context: if closed → dead; if any task is active on the container → alive (skip); else run test SQL or `isValid(ceil(timeout/1000))` with timeout. Any dead → invalidate (disconnect on failure if failures > 5); success resets failure counter.
  4. Invalidate (three phases BEFORE, INVALIDATE, AFTER) over the data source + dependents (shared SSH), under a per-container task lock that de-duplicates concurrent invalidations; network handlers before contexts; contexts reconnect preserving auto-commit/isolation and re-run bootstrap; all-failed → optional forced disconnect.
  5. Operation-level recovery: wrap user/metadata operations; on error classify via dialect error assistant: `CONNECTION_LOST` → invalidate + retry (N=1); `TRANSACTION_ABORTED` → rollback context + retry; others → rethrow; never retry inside a failed recovery on the same thread.
- **Data structures:** `checkCache: Map<DsId, Instant>`, `pingInFlight: Set<DsId>`, `failedAttempts: Map<DsId, u32>`, per-thread `RecoveryState { depth, failed }`.
- **Complexity:** O(#connections) per tick; pings are async jobs.
- **Edge cases (exhaustive list):**
  1. Long-running query on the connection → do not ping it (would block or interfere); DBeaver checks "active task".
  2. Ping itself hangs → timeout; `isValid` takes seconds (bug #42471 passed ms).
  3. Retrying a non-idempotent statement after `CONNECTION_LOST` (e.g. INSERT that actually committed before the socket died) → **duplicate writes**. DBeaver retries generically; Hacivat should retry only reads/metadata, or ask.
  4. Manual-commit transaction in progress when the connection dies → transaction is lost; must tell the user rather than silently reconnect.
  5. Session state lost on reconnect: temp tables, `SET` variables, prepared statements, `search_path`/`USE` → re-apply bootstrap + remembered default schema; warn about temp objects.
  6. Laptop sleep while a query runs.
  7. Shared SSH session → invalidate dependents together; avoid N parallel invalidations (#42151).
  8. Server restarts quickly vs. stays down → backoff; stop after K attempts and show "disconnected".
  9. Embedded databases (SQLite) → never closed on sleep.
  10. Idle-transaction auto-end must not fire during user activity (uses last activity, not txn age alone).
  11. PgBouncer transaction pooling → session state not reliable at all.
- **Test cases:**
  1. Kill server backend (`pg_terminate_backend`) of the meta context → next metadata read recovers transparently with one retry.
  2. Kill backend of an editor context during an open manual transaction → user sees "transaction lost" error; context reconnects; auto-commit/isolation preserved.
  3. Keep-alive 60 s, NAT drops idle TCP after 120 s → connection survives 10 min idle.
  4. 6 consecutive failed pings → data source disconnected with notification.
- **Prior art and evidence:** `MODEL/runtime/jobs/DataSourceMonitorJob.java`, `KeepAlivePingJob.java`, `InvalidateJob.java`, `MODEL/model/exec/DBExecUtils.java:209-306`. DataGrip: keep-alive and auto-disconnect on a shared timer, "commit?" prompt for open transactions on auto-disconnect [DataGrip report-connectivity §3].
- **Recommendation for Hacivat:** One supervisor task per live connection (actor) with a `tokio::time::interval`; protocol-level liveness where possible (PG: empty simple query `""` or `SELECT 1`; MySQL `COM_PING`; SQL Server `SELECT 1`; Redis `PING`; Mongo `{ping:1}`); track tx state from the protocol (PG ReadyForQuery `I/T/E`, MySQL status flags, MSSQL `@@TRANCOUNT`). Recovery policy: auto-retry only idempotent/metadata/read operations; never silently retry DML; show a non-modal "reconnected — session state reset" banner (EN/TR). Sleep detection via OS hooks (macOS `NSWorkspaceWillSleepNotification`, Windows power broadcast, systemd-logind) or by clock-jump detection like DBeaver.
- **Used by:** Hacivat; Karagöz (agent-side connection pool health); Perde (agent heartbeat uses the same backoff logic).

### ALG-DBV-CONN-06: Smart commit mode

- **Problem:** Give users auto-commit convenience for reads but automatic protection (an explicit transaction) as soon as they modify data.
- **Inputs / outputs:** statement text, current tx mode, dialect → possibly switch the session to manual commit before executing; after commit/rollback possibly switch back.
- **Algorithm (DBeaver):**
  1. If smart commit disabled or session already manual → do nothing.
  2. Strip comments; take first token; uppercase; if not a dialect KEYWORD → not modifying; if in `{SELECT, SHOW, USE, SET, EXPLAIN}` → not modifying; else modifying.
  3. If modifying → `setAutoCommit(false)` then execute; if that statement throws → `setAutoCommit(true)` (nothing pending).
  4. On commit/rollback event and `smartCommitRecover` → background job sets auto-commit back to true.
- **Edge cases (exhaustive list):**
  1. `WITH … SELECT` (read) → misclassified as modifying (#14588, #37349).
  2. `WITH … INSERT/UPDATE/DELETE` (PG data-modifying CTE) → modifying (correct by accident).
  3. `SELECT … FOR UPDATE`, `SELECT func_with_side_effects()`, `SELECT nextval()` → classified read but take locks / have effects.
  4. `CALL proc()`, `EXEC proc`, `DO $$…$$` → modifying (conservative; fine).
  5. `BEGIN`/`START TRANSACTION` typed explicitly → should switch to manual and *not* send an extra BEGIN.
  6. `COMMIT`/`ROLLBACK` typed manually → should trigger recover.
  7. `SET` statements are excluded but `SET TRANSACTION ISOLATION…` affects the next transaction.
  8. DDL in MySQL/Oracle auto-commits implicitly → manual mode gives false sense of safety; PG DDL is transactional.
  9. `EXPLAIN ANALYZE DELETE …` actually executes the DELETE in PG → classified read!
  10. Leading comments / hints (`/*+ … */`) → must be stripped first.
  11. Multi-statement batch where only the 3rd modifies.
  12. Parenthesised `(SELECT …) UNION …` → first token `(`.
  13. `VALUES (…)` standalone query → keyword, not excluded → misclassified.
  14. MySQL `LOCK TABLES`, `HANDLER`, `LOAD DATA`.
- **Test cases:** `SELECT 1` → stays auto; `/* x */ update t set a=1` → manual; `WITH c AS (SELECT 1) SELECT * FROM c` → expected stay auto (DBeaver: manual — bug); `EXPLAIN ANALYZE DELETE FROM t` → expected manual (DBeaver: auto — bug); failing `INSERT` (constraint) → back to auto, no pending transaction.
- **Prior art and evidence:** `MODEL/model/exec/DBExecUtils.java:432-454,521-566`, `MODEL/model/impl/sql/AbstractSQLDialect.java:59-60,737-769`, `SQLED/execute/SQLQueryJob.java:584-610`. DataGrip: no smart commit; has "pending transaction" badge and DML guards [DataGrip report-connectivity §7, report-editor §5].
- **Recommendation for Hacivat:** Implement smart commit on top of a **real statement classifier** (sqlparser-rs AST per dialect, plus a token-level fallback): categories `Read`, `ReadWithLocks`, `Write`, `Ddl(implicit_commit: bool)`, `TxControl(Begin|Commit|Rollback|Savepoint)`, `SessionControl`, `Call(unknown effects)`, `Unknown`. Switch to manual on `Write`, `ReadWithLocks`, `Call`, `Unknown`; warn on implicit-commit DDL; `EXPLAIN ANALYZE` inherits the inner statement's class. Track tx state from the protocol, not from guesses. Default ON for `prod` connection type.
- **Used by:** Hacivat; Karagöz (policy: "require explicit transaction for writes" can reuse the classifier for audit tagging).

### ALG-DBV-CONN-07: pgpass credential lookup

- **Problem:** Find a PostgreSQL password from the user's pgpass file without storing it in the client.
- **Inputs / outputs:** host, port, database, user (may be empty), optional SSH host, optional explicit "pgpass hostname" override → `(user?, password)` or error.
- **Algorithm (DBeaver):**
  1. File = `$PGPASSFILE` or `%APPDATA%/postgresql/pgpass.conf` (Windows) or `~/.pgpass`; missing → error.
  2. Host candidates: if the connection host is empty/`localhost`/`127.0.0.1` and an SSH tunnel is configured → first try the SSH host; an explicit provider property overrides it; then try the connection host. Port defaults to the driver default (5432). Database and host taken from the *original* (not tunnel-rewritten) config.
  3. For each line: trim; skip empty or `#`; replace `\\` and `\:` with placeholders, split on `:`, need ≥ 5 fields, restore escapes.
  4. Match host, port, database: field `*` matches anything, otherwise **case-insensitive** equality. If config user empty → take the first matching line's user+password; else require user match (or `*`), and set user if the line's user is not `*`.
  5. No match → error "No matches in pgpass".
- **Edge cases (exhaustive list):**
  1. Escaped colons and backslashes in passwords (`pa\:ss`, `pa\\ss`).
  2. More than 5 fields when a password contains an unescaped colon → DBeaver ignores extras (password truncated at first colon) — libpq treats the rest of the line as password? [Unknown: verify libpq behaviour; libpq requires escaping]
  3. CRLF files → trim handles.
  4. Case sensitivity: libpq compares exactly; DBeaver ignores case → may pick a wrong line for case-sensitive DB names.
  5. File permissions: libpq ignores the file if group/world readable (Unix); DBeaver doesn't check.
  6. `localhost` vs Unix socket paths in host field (libpq matches `localhost` for socket connections).
  7. Tunnel: libpq would match against the *tunnel* host (`127.0.0.1`); DBeaver matches the real host first — friendlier.
  8. Credentials cached for the session (#26852 complains pgpass changes aren't picked up).
  9. Comment lines with leading spaces.
- **Test cases:** line `db.example:5432:app:alice:s3cr\:et` + config (db.example, 5432, app, alice) → password `s3cr:et`; config with empty user → user alice; line `*:*:*:bob:pw` with user `alice` → no match; host `DB.EXAMPLE` vs `db.example` → DBeaver match (libpq: no match).
- **Prior art and evidence:** `PG/src/org/jkiss/dbeaver/ext/postgresql/model/AuthModelPgPass.java:45-204`; DataGrip PgPass interceptor runs first in the chain [DataGrip report-connectivity §2]; DataGrip's Rust note: wildcard, `\:` escape, 0600 check.
- **Recommendation for Hacivat:** Implement **libpq-exact** semantics (case-sensitive, 0600 check on Unix with a clear warning, escape rules, first-match), plus DBeaver's useful extension of trying the real host when tunnelled. Re-read the file on each connect (avoid #26852). Same pattern for `~/.my.cnf` (MySQL), `.netrc`, and PG service files (`pg_service.conf`).
- **Used by:** Hacivat; Karagöz (agent can use local pgpass so no credentials leave the host).

### ALG-DBV-CONN-08: Execution-context topology (main / metadata / isolated)

- **Problem:** Decide how many physical connections to open and which consumer uses which, balancing isolation (no blocking, independent transactions) against server connection limits.
- **Inputs / outputs:** driver traits (embedded, single-connection), preferences (`meta.separate`, `editor.separate`: ALWAYS/NEVER/DEFAULT), consumer kind → context to use (existing or new).
- **Algorithm (DBeaver):**
  1. Instance creation opens `main`.
  2. First metadata need: `separate = ALWAYS ? true : NEVER ? false : !embedded && !forceSingle`; if separate open `meta` else reuse `main`.
  3. SQL editor attach: if editor-separate (same rule) → open isolated context named `SQLEditor <file>` initialised from main's defaults, then restore the editor's remembered catalog/schema; else share `main`.
  4. Data transfer/export/import: open isolated contexts for producer/consumer (configurable).
  5. Multi-database engines that cannot switch DB on a connection (PG) → one instance per database, each with its own `main`+`meta`, opened lazily on first expand.
  6. On editor close → close its isolated context (job).
- **Edge cases (exhaustive list):**
  1. Server `max_connections` reached → lazy loads fail; DBeaver can loop (#36180).
  2. Embedded/file databases with exclusive locks (SQLite in some modes, H2 embedded, Derby) → must be single connection.
  3. Temp tables created in an editor are invisible to metadata context and other editors (expected but surprising).
  4. `SET` / `USE` in one editor doesn't affect others.
  5. Pooled middleware (PgBouncer) → many client connections fine, but session features break.
  6. Cloud DBs that bill per connection or have tiny limits (serverless).
  7. Read replicas / multi-host failover URLs → each context may land on a different host.
- **Test cases:** PG with 3 databases expanded + 2 editors, defaults → 3×2 + 2 = 8 server connections (DBeaver); with meta=NEVER, editor=NEVER → 3; SQLite → 1 regardless.
- **Prior art and evidence:** `JDBC/JDBCRemoteInstance.java:95-162`, `SQLED/SQLEditor.java:879-995`, `PG/src/.../PostgreDatabase.java:228-247`. DataGrip: sessions decoupled from consumers, single-session mode, session manager UI [DataGrip report-connectivity §3].
- **Recommendation for Hacivat:** Make the connection budget explicit per profile (`max_connections`, default e.g. 4) with a session router: `Introspection` (pooled, short-lived, read-only, auto-commit), `Console(id)` (sticky, owns tx/session state), `Grid` (reads from console session or short-lived), `Transfer` (dedicated). PG multi-database: open per-database introspection connections lazily and *close them when idle* (DBeaver keeps them). Show a "sessions" panel like DataGrip. For Karagöz-mediated connections, every session is a multiplexed stream over one P2P link, so the budget is enforced at Karagöz (which owns the real DB connections).
- **Used by:** Hacivat; Karagöz (enforces per-user session limits).

### ALG-DBV-CONN-09: Cancellation escalation

- **Problem:** Stop a running statement promptly, even with uncooperative drivers.
- **Inputs / outputs:** job, stack of blocking objects (connection → statement) → cancelled statement or killed connection.
- **Algorithm (DBeaver):**
  1. User cancels job → `runBlockCanceler`: take innermost block (statement) → `Statement.cancel()` on a separate job (driver sends protocol cancel: PG CancelRequest on a new socket; MySQL Connector/J `KILL QUERY` on a new connection; MSSQL attention packet).
  2. If `checkTimeout > 0` and still running after timeout → cancel next outer block (connection) → `cancelCurrentExecution`: default `Thread.interrupt()` on the blocked thread; PG `cancelQuery()`.
  3. Unsupported cancel (feature-not-supported) → ignored silently.
- **Edge cases:** cancel arrives before statement starts / after it finished (race → cancels the *next* statement on some drivers); protocol cancel goes over a *new socket* → must traverse the same SSH tunnel/TLS/proxy; PG cancel key invalid with PgBouncer; MySQL `KILL QUERY` needs privileges; cancelling a COMMIT; interrupting a thread in socket read may close the connection (then invalidate); drivers ignoring cancel → only closing the socket works.
- **Test cases:** `SELECT pg_sleep(60)` cancel → `57014 query_canceled` within 1 s; via SSH tunnel → same; MySQL `SELECT SLEEP(60)` → returns 1 (interrupted) within 1 s.
- **Prior art and evidence:** `MODEL/model/runtime/AbstractJob.java:180-290`, `JDBC/JDBCDataSource.java:947-970`, `PG/src/.../PostgreDataSource.java:696-704`. DataGrip: "cancel running and queued", escalation driver cancel → close → kill process [DataGrip report-connectivity §3; Rust notes on per-DB cancel].
- **Recommendation for Hacivat:** Per-driver `CancelHandle` obtained at statement start (PG `CancelToken` — must be created with the same transport/TLS: use `cancel_query_raw` over a fresh tunnel stream; MySQL second connection `KILL QUERY <id>`; MSSQL attention/`KILL`; SQLite/DuckDB `interrupt()`; ClickHouse `KILL QUERY WHERE query_id`; Mongo `killOperations`). Escalation ladder with UI states `Cancelling…` → `Cancelled` / `Force-closing…`: protocol cancel → after N s close socket → mark session broken → lazy reconnect.
- **Used by:** Hacivat; Karagöz (agent-side cancel on behalf of remote client; must authorise the cancel to the same user/session).

### ALG-DBV-CONN-10: Driver/connection property sanitization

- **Problem:** Prevent a connection definition (possibly from an untrusted source or a malicious server operator) from enabling driver features that compromise the client machine.
- **Inputs / outputs:** final URL + properties map, per-driver restricted list, per-driver allow rules → ok / reject with property name.
- **Algorithm (DBeaver):**
  1. Only when multi-user or `sanitizeDriverProperties` (default on).
  2. If URL unknown → reject.
  3. For URL text and every property name and value: if text contains `%`, URL-decode once (ignore malformed escapes); for each restricted name, regex `\b<name>\b` case-insensitive → reject — unless (name, value) is allowed (e.g. boolean restricted property with value `false`/`no`).
  4. Runs after auth and driver substitution (they may add properties) and before the driver parses anything.
- **Edge cases (exhaustive list):** double-encoding (`%2561llow…`) bypasses a single decode; property names embedded in values (`propertiesTransform` inside another prop) caught by scanning values; legitimate values containing a restricted word (false positive); driver-specific URL syntaxes (`;k=v`, `?k=v`, `(k=v)` Oracle descriptor); case variants; Unicode homoglyphs; properties set via `useConfigs` bundles (also restricted).
- **Test cases:** `jdbc:mysql://h/db?allowLoadLocalInfile=true` → reject; `…?allowLoadLocalInfile=false` → allow; property `autoDeserialize=TRUE` → reject; URL `…?x=%61utoDeserialize` → reject (decoded once).
- **Prior art and evidence:** `JDBC/AbstractJDBCConnectionValidator.java:30-100`, `MY/src/org/jkiss/dbeaver/ext/mysql/model/MySQLConnectionValidator.java:25-48`, `JDBC/JDBCDataSource.java:349-362`.
- **Recommendation for Hacivat:** With native Rust drivers the dangerous surface is smaller but real: **disable MySQL `LOAD DATA LOCAL INFILE` handling by default** (mysql_async requires an explicit `LocalInfileHandler` — never install a permissive one), disallow PG `sslrootcert=system`-style surprises only when configs come from Perde, restrict arbitrary file paths (SSL keys, SQLite files) in configs received from Perde to user-approved directories, and treat Perde-provided connection definitions as untrusted: allow-list property keys per driver instead of deny-listing.
- **Used by:** Hacivat (configs imported or synced from Perde), Karagöz (configs it receives), Perde (validation at write time).

---

## 5. Implications for Hacivat / Karagöz / Perde

### 5.1 Architecture mapping (DBeaver concept → Hacivat)

| DBeaver | Hacivat (Rust) recommendation |
|---|---|
| OSGi bundles + `plugin.xml` | Cargo workspace; one crate per backend family behind feature flags (`hacivat-pg`, `hacivat-mysql`, `hacivat-mssql`, `hacivat-sqlite`, `hacivat-redis`, `hacivat-mongo`, `hacivat-clickhouse`); a compiled-in `Registry` of `Backend` trait objects; **data catalogue** files (TOML/JSON, embedded with `include_str!`) for driver metadata: ids, display names (i18n keys), default ports, URL templates, capability flags, vendor "server types". |
| `DBPDataSourceContainer` / `DBPDataSource` | `ConnectionProfile` (serde, versioned schema, no I/O) / `LiveConnection` (actor handle; dropped on disconnect). UI holds profile ids and handles, never internals. |
| `DBSInstance` + execution contexts | `SessionRouter` with explicit budget and roles (Introspection, Console, Grid, Transfer); per-database sub-routers for PG. |
| `<parameter>` flags for generic JDBC | `Capabilities` struct per backend + per server-type overrides (PG family: Redshift, Cockroach, Yugabyte, Timescale, Neon, Supabase…). |
| `serverType` extension | `ServerFlavor` enum detected after connect (`version()`, `SHOW crdb_version`, `SELECT extversion FROM pg_extension WHERE extname='timescaledb'`), capability overlay. |
| Auth model extension point | `AuthMethod` trait: `Password`, `PgPass`, `MyCnf`, `ShellCommand`, `AwsRdsIam` (aws-sigv4, 15-min tokens, refresh on reconnect), `AzureAd` (token for tiberius), `GcpIam`, `Kerberos` (feature flag), `KaragozDelegated` (no client secrets at all). |
| Network handlers (tunnel/proxy/config) | `Transport` enum/chain producing an async byte stream: `Direct`, `SshChain(Vec<Hop>)`, `Socks5`, `HttpConnect`, `KaragozP2P(agent_id)`; `Tls` as a separate layer configured per backend (rustls). |
| Maven driver download | Not needed for native drivers; keep a signed-manifest plugin downloader for sidecars/data packs. |
| Monitor job, invalidate, recovery | Per-connection supervisor task; protocol-aware liveness; safe retry policy (no silent DML retries). |
| Connection types dev/test/prod | Keep, with safety defaults (manual commit + confirm on prod, colored chrome, smart-commit default on). Perde can *enforce* types from org policy. |
| `credentials-config.json` (hard-coded key) | OS keychain via `keyring` crate; `secrecy` + `zeroize`; DataGrip's 4 retention modes; never write secrets in profile files; profiles split into shareable vs local like DataGrip. |
| Driver-property sanitizer | Allow-list per backend for any config not authored locally by the user. |

### 5.2 Karagöz and Perde

- **The tunnel abstraction is the integration point.** DBeaver's insight — a network handler rewrites the endpoint and everything above stays unchanged — maps directly onto Hacivat's P2P design: `Transport::KaragozP2P` yields a stream (QUIC stream or relayed stream) that the database driver uses as if it were a TCP socket. All higher layers (dialect, introspection, grid) are oblivious to whether traffic is direct, SSH or P2P.
- **Authorisation must live at Karagöz.** In DBeaver all policy (read-only, permissions, confirmations) is client-side and bypassable. Since Hacivat↔Karagöz traffic is P2P, Karagöz must (a) own the real database credentials, (b) authenticate the Hacivat user (Perde-issued identity), (c) enforce read-only/role policy *server-side* (session-level `default_transaction_read_only`, DB roles, statement classifier from ALG-06 for audit/deny), and (d) record audit events locally. Hacivat's client-side checks become UX only.
- **Cancel and session budget at Karagöz.** Karagöz should multiplex many Hacivat sessions over a small number of real DB connections per user, apply ALG-08 budgets, and implement ALG-09 cancel on the DB side.
- **Configs from Perde are untrusted input.** Apply ALG-10 allow-lists in both Hacivat and Karagöz.
- **Discovery → profiles.** Karagöz discovering Docker containers produces `ConnectionProfile`s with `Transport::KaragozP2P` and `AuthMethod::KaragozDelegated`; no secrets reach Hacivat or Perde.

### 5.3 Concrete design rules distilled

1. Immutable profile + resolved copy per connect; never mutate saved config during connect.
2. Ordered connect steps with cleanup guards; structured, stage-tagged errors with i18n message keys.
3. Explicit connection budget and session roles; lazy + idle-closing introspection sessions.
4. SSH: refcounted sessions/forwards, group invalidation, `~/.ssh/config` support, in-process streams, strict host keys with TOFU prompt.
5. Liveness via protocol pings; recovery retries only safe operations.
6. Transactions: protocol-tracked state; smart commit on a real classifier; prod defaults to manual commit.
7. Read-only enforced at the server session level (and at Karagöz), client check is advisory.
8. Secrets only in OS keychain; profile files shareable.
9. Security allow-list for driver options; MySQL local-infile off.
10. URL paste/import supported with round-trip tests; refuse ambiguous tunnel rewrites.

---

## 6. i18n considerations

- DBeaver localises via Eclipse NLS: `*Messages.java` + `*Messages_<locale>.properties` per bundle (e.g. `REG/internal/RegistryMessages_*.properties`) and `plugin.xml` `%key` references to `OSGI-INF/l10n/bundle_<locale>.properties` (e.g. PG ships `bundle_{ar,de,es,fr,it,ja,ko,pl,pt_BR,ro,ru,tw,uk,zh}.properties`). [Observed: file listings]
- Shipped UI languages (registry `language` extension): en, fr, de, it, pl, ja, ko, pt_BR, ro, ru, zh, es, **tw** (non-standard code for Traditional Chinese — should be `zh-TW`/`zh-Hant`), uk, ar. **No Turkish.** [Observed: `plugins/org.jkiss.dbeaver.registry/plugin.xml:28-45`] (Full coverage analysis is in part 05.)
- Connectivity-specific gaps: network-handler and auth-model property labels/descriptions are often hard-coded English in `plugin.xml` (`label="Host"`, `description="SSH target host name or IP address"`, MySQL SSL `label="SSL"`), exception messages are English by policy ("messages in exceptions should be in English"), progress sub-tasks are English strings (`"Connect to " + target`, `"Initialize tunnel"`). [Observed: `plugins/org.jkiss.dbeaver.net.ssh/plugin.xml:20-35`, `MY/plugin.xml:366-380`, `AGENTS.md`, `REG/DataSourceDescriptor.java:1210-1235`]
- Recommendations for Hacivat:
  - Every user-visible connect-stage message is a message id with arguments (Fluent `.ftl`), e.g. `connect-stage-ssh-hop-failed = SSH hop { $index } ({ $host }) failed: { $reason }` / TR: `SSH atlama { $index } ({ $host }) başarısız: { $reason }`. Driver/server error texts are shown verbatim *plus* a localised category ("Authentication failed", "Kimlik doğrulama başarısız").
  - Backend catalogue entries carry i18n keys, not display strings; adding a language = adding `.ftl` files, no code changes (hard requirement).
  - Language codes: use BCP-47 (`tr`, `zh-Hant`), not ad-hoc codes like DBeaver's `tw`.
  - Turkish pitfalls in this area: case-insensitive comparisons of host names, keywords and pgpass fields must use locale-independent (ASCII/Unicode simple) folding — Java's `toUpperCase()` in a Turkish locale turns `i` into `İ`, a classic bug. DBeaver mitigates with `toUpperCase(DEF_LOCALE)` in the dialect but plain `equalsIgnoreCase` elsewhere. Rust: use `to_ascii_uppercase`/`eq_ignore_ascii_case` for protocol keywords and identifiers; never locale-sensitive casing for protocol data. [Observed: `MODEL/model/impl/sql/AbstractSQLDialect.java:748` uses `DEF_LOCALE`]
  - Connection-type names (dev/test/prod) and colors should be localisable labels over stable ids.

---

## 7. Risks, pitfalls, open questions

### 7.1 Pitfalls observed in DBeaver (avoid)

1. **Connection explosion** (PG databases × contexts + editors) → server limits (#36180). Budget explicitly.
2. **Silent generic retry** after connection loss can duplicate non-idempotent writes.
3. **Hard-coded encryption key** for saved credentials gives a false sense of security (#6991 discussion).
4. **Headless SSH accepts any host key**; Hacivat CLI/MCP mode must not.
5. **No checksum on downloaded jars** — supply-chain risk.
6. **Global-JVM proxy selector** with thread-local guessing — breaks with async/native networking. In Rust, make proxying part of the transport explicitly.
7. **URL-mode connections bypass tunnel rewrite** (#38364, #38336).
8. **Smart commit keyword heuristic** misclassifies CTEs and `EXPLAIN ANALYZE` (#14588, #37349).
9. **Client-only read-only check** passes unparsable statements.
10. **`connecting` boolean guard** (not atomic) — race on double connect.
11. **Re-auth not refreshed on reconnect** (`REFRESH_CREDENTIALS_ON_CONNECT = false`) — short-lived cloud tokens (15-min IAM tokens) expire; Hacivat must re-run the auth step on every physical (re)connect.
12. **Invalidate doesn't rerun BEFORE_CONNECT hooks** (#7887).
13. **Recovery UX**: slow reconnect after network changes (#6903), modal popups stealing focus (#15848), lost filters (#35536).
14. **Concurrent invalidation of SSH-sharing data sources can hang** (#42151).

### 7.2 Open questions

- [Unknown] Exact libpq behaviour for unescaped colons in pgpass passwords vs DBeaver's split (verify against libpq source `fe-connect.c` `passwordFromFile`).
- [Unknown] Whether `DatabaseURL.replaceHostAndPort` is wired into paid editions' connect path (not used in CE at this commit).
- [Unknown] Behaviour of `openConnectionDetached` in DBeaver EE/Team (detached process) — CE stub returns null; could inform whether DBeaver is moving to out-of-process drivers like DataGrip.
- [Inferred] Column semantics of `DBPConnectionType` constructor booleans (derived from field order); verify with the setters before relying on exact defaults.
- For Hacivat: how much of the PG session state (temp tables, prepared statements, `SET`s) should be replayed after reconnect vs. just reported? Proposal: replay bootstrap + default schema; report the rest.
- For Karagöz: per-user DB connection pooling vs. per-session real connections — impacts `SET`/temp-table semantics for consoles. Likely: console sessions get dedicated real connections (sticky), introspection/grid reads are pooled.

### 7.3 Licensing notes

DBeaver CE is Apache-2.0: porting `StringTemplate`/`DatabaseURL` test vectors and algorithms is permitted with attribution (keep NOTICE, mark modified files). Driver jars referenced by DBeaver have their own licenses (shown via `<file type="license">`); irrelevant for native Rust drivers but relevant if Hacivat ever offers a JDBC sidecar.
