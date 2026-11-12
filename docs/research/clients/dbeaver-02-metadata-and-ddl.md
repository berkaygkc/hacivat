# DBeaver Community: Metadata Introspection, Caches, Refresh, Object Filters and DDL Generation

> Part 2 of the DBeaver source-analysis series. Read together with
> `dbeaver-01-architecture-drivers-connections.md` (object model, drivers, connections) and
> `dbeaver-03-sql-editor-parsing-completion.md` (completion consumes the caches described here).

---

## 1. Scope and sources

### 1.1 What this document covers

How a database client finds out *what is inside* a database (schemas, tables, columns, keys, indexes, views,
routines) and keeps that knowledge in memory; how and when that knowledge is thrown away and re-read; how users
narrow what is loaded (object filters); how DDL text ("the `CREATE TABLE …` statement for this table") is
produced; how object edits become `ALTER` statements; and what DBeaver CE offers for schema comparison.

Out of scope here (covered by sibling documents): connection setup and driver loading (doc 01), SQL parsing and
completion (doc 03), result-set editing (doc 04), data transfer (doc 05).

### 1.2 Sources

| Source | Version / commit | License | Notes |
|---|---|---|---|
| DBeaver Community, `github.com/dbeaver/dbeaver` | commit `f78ac453859084a4654eac0599fb4c0514ef405b` (2026-11-11), shallow clone at `.refs/repos/dbeaver` | Apache-2.0 | Read only. No build, no tests run. All paths below are relative to the repo root. |
| DBeaver GitHub issues (via `gh` CLI, read only) | state as of 2026-11-11 | n/a | Issues #42474, #42465, #42468, #39295, #39169, #21928, #6398 and in-code references #40920, #20383, #11917, #7512. |
| DataGrip 2024.3.5 teardown notes (Turkish) | `.refs/datagrip-extract/report-connectivity.md`, `report-nosql-model.md` | DataGrip is proprietary; notes are behavioral observations only | Used only for comparison; nothing copied. |

Path abbreviations used below:

| Abbrev. | Path |
|---|---|
| `MODEL` | `plugins/org.jkiss.dbeaver.model/src/org/jkiss/dbeaver/model` |
| `JDBC` | `plugins/org.jkiss.dbeaver.model.jdbc/src/org/jkiss/dbeaver/model/impl/jdbc` |
| `PG` | `plugins/org.jkiss.dbeaver.ext.postgresql/src/org/jkiss/dbeaver/ext/postgresql` |
| `MY` | `plugins/org.jkiss.dbeaver.ext.mysql/src/org/jkiss/dbeaver/ext/mysql` |
| `MS` | `plugins/org.jkiss.dbeaver.ext.mssql/src/org/jkiss/dbeaver/ext/mssql` |
| `GEN` | `plugins/org.jkiss.dbeaver.ext.generic/src/org/jkiss/dbeaver/ext/generic` |
| `ORA` | `plugins/org.jkiss.dbeaver.ext.oracle/src/org/jkiss/dbeaver/ext/oracle` |
| `TEST` | `test/` |

Licensing note for implementers: DBeaver CE is Apache-2.0, so reuse with attribution is legally possible. This
document nevertheless describes algorithms in prose and our own pseudocode; no DBeaver code is reproduced beyond
short identifiers, and SQL catalog queries are summarized (which system tables are joined, on which keys) rather
than copied.

### 1.3 Mini-glossary (for readers who are not database experts)

- **Metadata / catalog / introspection.** Every relational database stores a description of its own contents in
  special "system" tables (PostgreSQL: `pg_catalog.pg_class`, `pg_attribute`…; MySQL: `information_schema.*` and
  `SHOW …` commands; SQL Server: `sys.objects`, `sys.columns`…). Reading these tables to learn what exists is
  called *introspection*. The result is the *metadata model*.
- **Catalog / database / schema.** Containers. PostgreSQL: server → database → schema → table. MySQL: server →
  "database" (which JDBC calls a *catalog*) → table; no schemas. SQL Server: server → database → schema → table.
- **JDBC `DatabaseMetaData`.** A standard Java API (`getSchemas()`, `getTables()`, `getColumns()`…) that every
  JDBC driver implements with its own internal queries. A generic client can use it without knowing anything about
  the database, at the cost of speed and completeness.
- **OID.** PostgreSQL's numeric object identifier. Every table, schema, type has one. Joining by OID is faster and
  unambiguous compared to joining by name.
- **DDL (Data Definition Language).** `CREATE`, `ALTER`, `DROP` statements. "Generate DDL" means producing the
  `CREATE …` text that would recreate an object.
- **Lazy loading.** Only reading something from the database when it is first needed (e.g. when the user expands
  a tree node), then remembering it.
- **Cache invalidation.** Deciding when remembered metadata is stale and must be re-read. Notoriously hard.
- **Navigator.** DBeaver's left-hand tree ("Database Navigator") of connections → databases → schemas → tables.

---

## 2. Executive summary

1. DBeaver's metadata lives in an **in-memory object graph of Java objects** (`DBSObject` subclasses per database
   type). It is never persisted to disk; every new connection session re-reads metadata from the server
   [Observed: issue #42474 is an open feature request for exactly this].
2. Each container object (schema, catalog) owns several **typed caches**: a flat list cache, a "struct" cache
   (parents + their children, e.g. tables + columns), a "composite" cache (multi-row objects such as indexes and
   constraints whose rows reference columns), and "lookup" caches that can fetch a single object by name.
3. Loading is **lazy and per container**: the first time anyone asks for the tables of a schema, one query reads
   *all* tables of that schema. Columns can be read either for one table or for the whole schema in one query
   ("bulk"); DBeaver chooses based on who asked (navigator expansion of one table vs. a schema-wide operation).
4. The **"fully cached" flag** is the central state bit. Once set, lookups never hit the database again until the
   cache is cleared. Lookup caches add a **negative cache** (`missingNames`) to avoid re-querying unknown names.
5. **Refresh is manual and coarse**: the user presses F5 on a node, the node's object clears its caches and
   re-reads; re-read objects are **merged into the existing Java objects** (identity-preserving) so that open
   editors keep valid references.
6. Since recent versions the SQL editor **recognizes DDL statements** (`CREATE/DROP/ALTER/RENAME` of
   table/view/index/schema…) and schedules a **targeted schema- or catalog-level refresh**, coordinated with the
   transaction (refresh after commit, compensating refresh after rollback).
7. There is **no change detection** (no watermarks like PostgreSQL `xmin` age or SQL Server `modify_date`). Every
   refresh re-reads the full container. DataGrip, in contrast, does incremental introspection with per-object
   state numbers and a persisted model.
8. A **separate "metadata" JDBC connection** is opened by default (except for embedded databases or single-connection
   mode) so that navigator queries never interfere with the user's transaction in the SQL editor.
9. **Object filters** (include/exclude masks such as `orders_*`) exist per object type, either globally per
   connection or per parent container. Some drivers push them into SQL (`LIKE`), others filter client-side after
   loading everything. The mask language has sharp edges (`_` is a wildcard; exact names are case-insensitive).
10. PostgreSQL introspection uses **hand-written `pg_catalog` queries keyed by OID**, with version gates
    (e.g. partition columns only on ≥10) and a pluggable "server type" for forks (Redshift, CockroachDB, Greenplum…).
11. MySQL uses `SHOW FULL TABLES`, `information_schema.COLUMNS/STATISTICS/KEY_COLUMN_USAGE/CHECK_CONSTRAINTS` and
    `SHOW TABLE STATUS` (loaded lazily, only when "additional info" properties are viewed).
12. SQL Server uses `sys.all_objects`, `sys.all_columns`, `sys.default_constraints`, `sys.extended_properties`
    (`MS_Description` comments), `sys.indexes/index_columns`, always **qualified with the database name**
    (`[db].sys.x`) so one connection can introspect many databases; Babelfish gets substitute sub-queries.
13. The **generic JDBC fallback** (`ext.generic`, ~40 dialect subclasses) uses `DatabaseMetaData` plus a
    declarative "meta model" that maps result-set columns by index for drivers with broken column names.
14. **DDL generation has two strategies**: ask the server for authoritative text (MySQL `SHOW CREATE TABLE`,
    PostgreSQL `pg_get_viewdef`, SQL Server `sys.sql_modules`) or **synthesize** it from the model through the same
    *object manager* code that is used for editing (PostgreSQL tables, SQL Server tables, generic tables).
15. Synthesized table DDL = optional commented `DROP` + `CREATE TABLE (columns, inline constraints)` + separate
    statements for things that cannot be nested (indexes, comments, grants, storage), ordered by a per-manager list
    of child types.
16. Multi-table DDL is **ordered by foreign-key dependencies** with a simple iterative algorithm; tables in cycles
    get `CREATE TABLE` first and `ALTER TABLE … ADD FOREIGN KEY` afterwards when the dialect supports splitting.
17. Object editing goes through a **command model** (`DBECommandContext`): every UI change is a command with undo,
    commands are grouped per object, merged (e.g. many renames → one), aggregated into a `CREATE` when the object
    is new, turned into SQL "persist actions", executed, and then reflected back into the cache.
18. Community edition's **schema compare** (`cmp.simple`) is a property-by-property report over navigator nodes; it
    does **not** generate migration scripts (DataGrip and DBeaver commercial editions do).
19. Known weak points: full re-reads on large dictionaries (Oracle schema with 14k tables: completion popup never
    appears within 60 s, Indexes folder takes ~30 s, per #42465), per-table query storms (#42468, fixed for PG
    indexes), `synchronized` cache loaders that block other threads, and locale-sensitive upper-casing in cache keys
    (a Turkish-locale hazard).
20. For Hacivat: copy the *shape* (typed per-container caches, bulk-vs-single loading, identity-preserving merge,
    DDL-triggered targeted refresh, separate metadata connection), and go beyond DBeaver with an **immutable
    snapshot catalog**, **persistent cache + incremental watermark refresh** (DataGrip-style), single-flight
    loading, and pure-function DDL generators with golden tests.

---

## 3. Detailed findings

### 3.1 The object model in one picture

```
DBPDataSourceContainer  (connection config, filters, navigator settings)          [doc 01]
 └─ DBPDataSource (live connection; e.g. PostgreDataSource)
     ├─ execution contexts: Main, Metadata (separate JDBC connection), isolated ones
     └─ DBSObjectContainer ... (PostgreDatabase → PostgreSchema)                      <- owns caches
          ├─ TableCache       : JDBCStructLookupCache<Schema, TableBase, TableColumn>
          ├─ ConstraintCache  : JDBCCompositeCache<Schema, TableBase, Constraint, ConstraintColumn>
          ├─ IndexCache       : JDBCCompositeCache<Schema, TableBase, Index, IndexColumn>
          ├─ ProceduresCache  : JDBCObjectLookupCache<Schema, Procedure>
          └─ DataTypeCache, ExtensionCache, AggregateCache ...
```

Key interfaces [Observed: `MODEL/struct/*`, `MODEL/DBPRefreshableObject.java`]:

| Interface | Meaning |
|---|---|
| `DBSObject` | Anything in the database tree: has `getName()`, `getParentObject()`, `getDataSource()`, `isPersisted()` (false for an object the user is creating but has not saved yet). |
| `DBSObjectContainer` | Has children; exposes `cacheStructure(monitor, scope)` with scope bits `STRUCT_ENTITIES=1`, `STRUCT_ATTRIBUTES=2`, `STRUCT_ASSOCIATIONS=4`, `STRUCT_ALL=7` [Observed: `MODEL/struct/DBSObjectContainer.java:34-46,91`]. |
| `DBSEntity` / `DBSTable` | A table-like thing with `getAttributes()`, `getConstraints()`, `getAssociations()` (foreign keys), `getIndexes()`. |
| `DBPRefreshableObject` | `refreshObject(monitor)` returns *self or a replacement*; `null` means "object no longer exists" [Observed: `MODEL/DBPRefreshableObject.java:28-38`]. |
| `DBPScriptObject` | `getObjectDefinitionText(monitor, options)`: the DDL/source text; options map controls what is included [Observed: `MODEL/DBPScriptObject.java:31-78`]. |
| `DBPUniqueObject` | Provides a unique name for overloaded objects (e.g. functions `f(int)` and `f(text)`); caches key by it. |

**Lazy properties.** A getter annotated with `@Property` whose first parameter is a `DBRProgressMonitor` is
automatically considered *lazy* (it may hit the database), and the property sheet loads it in the background
[Observed: `MODEL/../runtime/properties/ObjectAttributeDescriptor.java:83-88`]. `@LazyProperty(cacheValidator=…)`
lets the UI ask "is this value already cached?" so it can render immediately when it is [Observed:
`MODEL/meta/LazyProperty.java`; MySQL `AdditionalInfoValidator` at `MY/model/MySQLCatalog.java:133-149`]. This is
an elegant convention: *the signature itself declares "this may be expensive"*.

### 3.2 The cache class family

All caches implement `DBSObjectCache<OWNER, OBJECT>` [Observed: `MODEL/struct/cache/DBSObjectCache.java`]:
`getAllObjects`, `getCachedObjects` (never queries), `getObject(name)`, `getCachedObject(name)` (never queries),
`isFullyCached`, `cacheObject`, `setCache`, `removeObject(obj, resetFullCache)`, `renameObject`, `clearCache`.

| Class | File (lines) | Shape | Used for |
|---|---|---|---|
| `AbstractObjectCache` | `MODEL/struct/cache/AbstractObjectCache.java` (408 lines) | list + lazily built name→object hash map + `fullCache` flag | base of all |
| `JDBCObjectCache` | `JDBC/cache/JDBCObjectCache.java` (186) | one query → list | sequences, extensions, events |
| `JDBCObjectLookupCache` | `JDBC/cache/JDBCObjectLookupCache.java` (150) | list + "load one by name" + negative cache | schemas, procedures, triggers |
| `JDBCStructCache` | `JDBC/cache/JDBCStructCache.java` (320) | parents + per-parent child caches (`IdentityHashMap<parent, SimpleObjectCache>`) | tables + columns |
| `JDBCStructLookupCache` | `JDBC/cache/JDBCStructLookupCache.java` (170) | struct + lookup by name | tables + columns (PG, MySQL, MSSQL, Oracle) |
| `JDBCCompositeCache` | `JDBC/cache/JDBCCompositeCache.java` (474) | objects grouped per parent; each object assembled from several rows | indexes, PK/UK/FK constraints |
| `JDBCObjectWithParentCache` | `JDBC/cache/JDBCObjectWithParentCache.java` (331) | composite without row children | triggers |
| `JDBCBasicDataTypeCache` / `JDBCLazyDataTypeCache` | `JDBC/cache/…` | data types | types |

#### 3.2.1 `AbstractObjectCache` internals [Observed: `MODEL/struct/cache/AbstractObjectCache.java`]

- State: `objectList` (ordered list, `null` = never loaded), `objectMap` (built on first name lookup), `fullCache`
  (volatile boolean), `caseSensitive` (default true), optional `listOrderComparator`, and a private `cacheSync`
  monitor object (lines 41-50).
- **Case sensitivity is auto-detected**: the first time an object is cached, if the dialect stores unquoted
  identifiers in *mixed* case (`storesUnquotedCase() == MIXED`, e.g. SQL Server or MySQL on case-insensitive file
  systems), the cache flips to case-insensitive and upper-cases keys (lines 257-265, 296-309).
- **Duplicate names**: when two objects share a key, the first one wins in the map unless a subclass declares the
  duplicate valid (lines 245-255). Both remain in the list.
- `setCache(list)` replaces the list, drops the map, sets `fullCache = true` (lines 179-185).
- `clearCache()` sets list and map to `null`, `fullCache = false` (lines 171-177).
- `removeObject(obj, resetFullCache)` removes from list and map; if `resetFullCache` it also clears the flag so the
  next access re-reads (lines 121-134).
- **`mergeCache(newList)`** (lines 205-224): for each newly fetched object, if an object with the *same name* is
  already cached, the *old Java object is kept* in its place. Reason given in the comment: "they might be
  referenced somewhere". Quadratic in list size (nested loop by name).
- **`deepCopyCachedObject(src, dst)`** (lines 316-361): copies every non-static, non-transient, non-final field
  from a freshly loaded object into the old one via reflection; nested caches in the destination are *cleared*
  rather than copied (so children get re-read lazily); `@PropertyGroup` sub-objects are copied recursively. This is
  how "refresh one table" keeps object identity.
- `clearChildrenOf(parent)` removes all objects whose parent is `parent` (lines 275-293).

#### 3.2.2 `JDBCObjectCache.loadObjects` [Observed: `JDBC/cache/JDBCObjectCache.java:72-166`]

1. `getAllObjects` / `getObject` call `loadObjects` unless `isFullyCached()` or the monitor is in "force cache"
   mode (lines 72-89).
2. `loadObjects` is **`synchronized` on the cache** (line 91): concurrent callers serialize; the second caller
   then sees `fullCache == true` and returns.
3. In unit-test mode it returns immediately (line 97) — tests populate caches by hand with `setCache`.
4. Only for **persisted** owners it opens a *metadata session* (`DBUtils.openMetaSession`, line 111), runs
   `prepareObjectsStatement` with fetch size `METADATA_FETCH_SIZE` (line 115), and calls `fetchObject` per row,
   skipping `null` or invalid ones. A hard cap of 1,000,000 objects logs a warning and stops (lines 51, 132-135).
5. Cancellation mid-read returns *without* publishing anything (partial results are discarded).
6. `handleCacheReadError` lets a subclass swallow errors (e.g. "feature not supported") (lines 149-152, 182).
7. `addCustomObjects` hook, optional sort, then `mergeCache` + `invalidateObjects` hook (lines 156-165).

#### 3.2.3 `JDBCStructCache.loadChildren` — bulk vs per-parent [Observed: `JDBC/cache/JDBCStructCache.java:111-224`]

This is the most important loader; ALG-DBV-META-02 formalizes it. Highlights:

- Children are stored per parent in an `IdentityHashMap<parent, SimpleObjectCache>` (line 52). "Children cached for
  parent P" means *P is a key in that map*, even with an empty list (line 294-299).
- `forObject == null` means **bulk**: load children of *all* parents with one query (e.g. all columns of a schema,
  `WHERE c.relnamespace = ?`), then match each row to its parent by the parent-name column (line 160-175).
- `forObject != null` means **single parent**: query with `WHERE c.oid = ?`.
- After a successful bulk read, every parent that received no rows gets an *empty* child list (so it will not be
  queried again) and the `childrenCached` flag is set (lines 199-219) — **unless the bulk result was completely
  empty**, which is treated as "maybe this driver cannot do bulk reads" (comment mentions SQLite) and nothing is
  marked (lines 200-203). Subclasses can override this with `isEmptyBulkChildrenResultComplete()` (line 80).
- Since recently, a subclass may split the bulk read into **batches of parents** (`getBulkChildrenBatches`), and
  results are only assigned after *all* batches finished, so a cancellation in the middle never marks anything
  complete (lines 124-131, 139-196). Oracle uses this when a table filter is active: batches of 500 table names
  in an `IN (…)` list, falling back to the owner-wide query above 10,000 tables [Observed:
  `ORA/model/OracleSchema.java:744-745,951-970`; tests in `TEST/org.jkiss.dbeaver.ext.oracle.test/.../OracleBulkColumnCacheTest.java`].

#### 3.2.4 `JDBCCompositeCache.loadObjects` — assembling multi-row objects [Observed: `JDBC/cache/JDBCCompositeCache.java:270-468`]

Indexes and constraints arrive as *one row per column* (an index on `(a, b)` is two rows). The composite cache:

1. Returns early if already loaded for that parent (or globally) (lines 276-283).
2. For a bulk request (`forParent == null`) first ensures **tables and all columns** are loaded (lines 286-290),
   because row assembly needs column objects.
3. Reads rows; for each row: determines parent name and object name (default `<PARENT>_#DBOBJ` when the name is
   empty, e.g. unnamed constraints in some engines, lines 60, 319-322, 470-472), resolves the parent table, skips
   parents already cached, groups rows into `ObjectInfo{object, rows[], broken}` in a `TreeMap` by object name
   (sorted, deterministic).
4. A row that cannot be resolved (e.g. column number not found) marks the **whole object as broken** and it is
   dropped (lines 357-366) unless `isEmptyObjectRowsAllowed()`.
5. "Feature not supported" SQL errors are swallowed with a debug log (lines 378-383).
6. Publishes: global list (bulk only), per-parent lists, empty lists for parents with no objects (lines 404-445).
7. Then calls `cacheChildren` for each object (attach column refs), sorted by ordinal position, and **a second
   pass `cacheChildren2`** — needed because foreign keys refer to unique keys of other tables that must be cached
   first (lines 447-465, comment at 110-114).

#### 3.2.5 Lookup caches and the negative cache [Observed: `JDBC/cache/JDBCStructLookupCache.java:52-71`, `JDBCObjectLookupCache.java:53-72`]

`getObject(name)`: return from cache if present; if the cache is fully loaded, or the name is in `missingNames`, or
the connection is down → `null`; otherwise run the lookup query *for that one name* (`prepareLookupStatement(…,
objectName)`), cache the hit or remember the miss. `setCache` and `clearCache` reset `missingNames`.

Why it matters: SQL completion and hyperlinking constantly ask "does `foo` exist in schema `public`?". Without the
negative cache every keystroke could trigger a catalog query.

`reloadObject` handles **case ambiguity**: the lookup may return several rows (names differing only by case); it
prefers the exact-case match and returns the first other match only if the dialect says lookups are
case-insensitive (`JDBCStructLookupCache.java:120-137`).

#### 3.2.6 The "force cache usage" monitor [Observed: `MODEL/runtime/LocalCacheProgressMonitor.java`, `MODEL/runtime/DBRProgressMonitor.java:56`]

`DBRProgressMonitor.isForceCacheUsage()` defaults to false. `LocalCacheProgressMonitor` wraps a monitor and returns
true; every cache checks it before querying. SQL completion and result-set metadata binding use it when the
connection setting "read extra metadata" is off [Observed: `plugins/org.jkiss.dbeaver.model.sql/.../completion/SQLCompletionAnalyzer.java:1033-1034`,
`MODEL/exec/DBExecUtils.java:769-770`]. Effect: completion only proposes what was already loaded, never blocks on
the network. This is a clean, cheap mechanism — "a read-only view of the cache" passed down the call stack.

### 3.3 How a navigator expansion turns into queries

**Navigator meta-model.** The tree structure per database is declared in the driver plugin's `plugin.xml`:
nested `<folder type=…>` and `<items property="tables">` elements [Observed:
`plugins/org.jkiss.dbeaver.ext.postgresql/plugin.xml:140-250`]. `property="tables"` means "call `getTables(monitor)`
on the parent object via reflection". `visibleIf` expressions hide folders (e.g. foreign keys only if
`dataSource.serverType.supportsForeignKeys()`). Labels are i18n keys (`%tree.tables.node.name`).

**Node loading** [Observed: `MODEL/navigator/DBNDatabaseNode.java`]:

- `getChildren(monitor)` (line 221): if `childNodes == null` it calls `loadChildren` which walks the meta tree
  (line 417) and for each `<items>` calls `loadTreeItems` (line 632) → reads the property value (a collection of
  `DBSObject`) via `PropertyValueReader`, wrapped in `DBExecUtils.tryExecuteRecover` (reconnect-and-retry on
  connection loss).
- During `loadTreeItems`, items are skipped if hidden, if *system* objects are hidden (`showSystem` setting —
  except when the list contains a single schema/catalog, so you never see an empty tree), DBA objects in
  "hide folders" mode, schemas in "merge entities" mode, and items not matching the **object filter**
  (lines 695-731).
- **UI threading**: the SWT content provider never blocks on a lazy node; it returns a "Loading…" placeholder and
  runs `TreeLoadService` in a background job [Observed:
  `plugins/org.jkiss.dbeaver.ui.navigator/src/.../database/DatabaseNavigatorContentProvider.java:93-102`].

**Worked example (PostgreSQL)**, user expands `mydb → public → Tables → orders → Columns`, then `Indexes`:

| Step | Cache call | SQL (summarized) |
|---|---|---|
| Expand database "Schemas" | `SchemaCache.getAllObjects` | `pg_namespace LEFT JOIN pg_description`, filter `nspname LIKE` if a schema filter exists, `ORDER BY nspname` [`PG/model/PostgreDatabase.java:1456-1490`] |
| Expand "Tables" | `TableCache.getAllObjects(schema)` | `pg_class LEFT JOIN pg_description WHERE relnamespace=? AND relkind NOT IN ('i','I','c')`, plus partition bound/key expressions on PG≥10 [`PG/model/PostgreSchema.java:879-899`] |
| Expand "orders → Columns" | `TableCache.getChildren(schema, orders)` → **single-parent** load | `pg_attribute JOIN pg_class LEFT JOIN pg_attrdef (default) LEFT JOIN pg_description LEFT JOIN pg_depend (owned sequence) WHERE NOT attisdropped AND c.oid=? ORDER BY attnum` [`PostgreSchema.java:922-951`] |
| Expand "orders → Indexes" | `IndexCache.getObjects(schema, orders)` | first **all constraints of the schema** (bulk, to avoid one constraint query per unique index — fix for #42468) [`PostgreSchema.java:1154-1165`], then `pg_index JOIN pg_class (index) JOIN pg_class (table) LEFT JOIN pg_description WHERE i.indrelid=?` with `pg_get_expr` for predicate/expression indexes and size/scan stats [`PostgreSchema.java:1169-1203`]; expression index columns resolved by an **extra query per expression column** `pg_get_indexdef(oid, n, true)` (line 1253). |
| "Generate DDL" for the whole schema | `cacheStructure(STRUCT_ALL)` | bulk tables, bulk columns (`WHERE c.relnamespace=?`), bulk constraints, bulk indexes, inheritance [`PostgreSchema.java:417-442`] |

Observation: the per-table vs per-schema choice is entirely driven by *which API the caller used*. Navigator
expansion of one table → per-table. Anything schema-wide (DDL of schema, ER diagram, compare, completion of
`schema.`) → bulk. Mixed patterns produce **N+1 query storms** when a loop calls a per-table API across many
tables; #42468 ("PostgreSQL: opening the Indexes folder runs one constraint query per table", closed) is one
such case that was fixed by prefetching the bulk constraint list.

### 3.4 Refresh semantics

#### 3.4.1 Object-level refresh (what F5 does)

1. `NavigatorHandlerRefresh` collects selected nodes; a folder node is replaced by its parent (refreshing "Tables"
   means refreshing the schema). Refreshes are queued; requests for the same data source are **serialized by
   key** `ds:<connection-id>`, identical repeated manual refreshes are coalesced, and nodes that are children of
   an already-refreshed node in the same batch are skipped [Observed:
   `plugins/org.jkiss.dbeaver.ui.navigator/src/.../actions/NavigatorHandlerRefresh.java:185-330`].
2. `DBNDatabaseNode.refreshNode` (line 353): locked nodes are skipped; if the object is a `DBPRefreshableObject` and
   persisted, call `refreshObject(monitor)` with reconnect-and-retry. If it returns `null`, the node removes
   itself from its parent (object was dropped). Otherwise `refreshNodeContent` → `reloadChildren` (lines 384-400,
   887-901).
3. **What each `refreshObject` clears is hand-written per class.** Examples:
   - `PostgreSchema.refreshObject`: clears extension, table, constraint, procedure, index caches; resets
     privileges and statistics; re-reads *the schema row itself* through the parent's schema lookup cache; and
     re-reads data types [Observed: `PostgreSchema.java:499-514`].
   - `PostgreTableBase.refreshObject`: clears only *this table's* entries in the constraint and index composite
     caches (`clearObjectCache(this)`), then `TableCache.refreshObject` [Observed: `PG/model/PostgreTableBase.java:261-269`].
   - `JDBCStructLookupCache.refreshObject`: if the parent list was never fully loaded → load it; else re-run the
     *single-object lookup*, clear the object's child (column) cache, and `deepCopyCachedObject(new → old)`; if the
     object vanished → remove it [Observed: `JDBCStructLookupCache.java:73-98`]. Note the special case that strips
     quotes from a name which was quoted at creation time (#20383, lines 76-82).
   - `PostgreTableReal.refreshObject` also resets row count / disk size and, optionally, schema statistics;
     `PostgreTable` resets super/sub-table lists and the policy cache; views drop their cached source text.
   - `MySQLCatalog.refreshObject`: clears all its caches and flags in one go [Observed: `MY/model/MySQLCatalog.java:537-554`].
   - `SQLServerSchema.refreshObject`: clears all its caches and data types [Observed: `MS/model/SQLServerSchema.java:184-195`].
   - **Refreshing a whole PostgreSQL connection** (`PostgreDataSource.refreshObject`) **shuts down and re-opens the
     contexts**, clears the database cache and re-initializes — effectively a reconnect [Observed:
     `PG/model/PostgreDataSource.java:526-545`]. Data sources that are not refreshable are reconnected outright
     [Observed: `plugins/org.jkiss.dbeaver.registry/.../DataSourceDescriptor.java:813-826`].
4. **Child node reconciliation** in `loadTreeItems` (lines 735-775): for each freshly loaded object, look for an old
   node with the same meta and an "equal" object — same class and same unique name at *every ancestor level*
   (`equalObjects`, lines 905-920). If found, reuse the old node (calling `reloadObject` to point it at the new
   object) and recursively reload its children if they had been loaded; otherwise create a new node. Old nodes
   not matched are disposed — **except non-persisted ones** (objects being created in an open editor), which are
   preserved so that the user does not lose work (#40920, lines 763-777).

#### 3.4.2 DDL-triggered refresh from the SQL editor (new, transaction-aware)

After each successful statement the SQL editor runs an **object-operation recognizer** over the parsed statement
(ANTLR-based semantic model) producing `SQLObjectOperation{operation ∈ CREATE|DROP|ALTER|RENAME, kind ∈
TABLE|VIEW|INDEX|SCHEMA|DATABASE|CATALOG|FUNCTION|PROCEDURE|SEQUENCE|SYNONYM|OTHER, qualifiedNameParts}`
[Observed: `MODEL/sql/SQLObjectOperation.java:26-60`;
`plugins/org.jkiss.dbeaver.model.sql/src/.../semantics/SQLObjectOperationRecognizer.java:60-459`;
`plugins/org.jkiss.dbeaver.ui.editors.sql/src/.../SQLEditor.java:4505-4530`].

- The operation is mapped to a **refresh target**: whole data source, one catalog, or one schema — never a single
  table [Observed: `plugins/org.jkiss.dbeaver.model.sql/src/.../SQLMetadataRefreshTargetResolver.java:64-160`].
  Unqualified names resolve against the context's current default catalog/schema.
- A user preference `MetadataRefreshBehavior ∈ {ASK, ALWAYS, NEVER}` decides whether to show a notification with
  a "Refresh" action, refresh silently, or do nothing [Observed: `.../SQLPreferenceConstants.java:28-50`].
- **Transactions** [Observed: `.../SQLMetadataRefreshTransactionCoordinator.java:45-410`]:
  - Auto-commit → refresh immediately.
  - Manual commit: behavior depends on whether DDL is transactional on that database (`DBCDDLTransactionBehavior`
    = TRANSACTIONAL / IMMEDIATE / IGNORED). In PostgreSQL DDL is transactional, so the refresh is deferred:
    targets accumulate while the transaction is open, and are refreshed on **commit**; on **rollback** a
    *compensating* refresh is offered for targets that had been refreshed while uncommitted (because the
    navigator may now show tables that no longer exist). Savepoint rollback triggers compensation too. Even if
    the preference is NEVER, compensation is upgraded to ALWAYS to repair a wrong navigator (line 300-305).
  - If the SQL editor's connection is not the navigator's metadata connection, uncommitted DDL is *invisible* to
    the navigator (different session), so DBeaver only informs the user instead of refreshing.

This is the closest DBeaver gets to DataGrip's "RefreshAffectedObjects" introspection task, but at schema
granularity rather than object granularity.

#### 3.4.3 What does *not* exist

- **No time-based or event-based invalidation.** Changes made by other users or tools are invisible until manual
  refresh (long-standing complaint #6398 "dbeaver should update everything on clicks", open).
- **No persistent cache.** #42474 (open) proposes storing raw metadata rows per cache in the workspace and
  replaying them through the same `fetchObject` code, with a background re-read afterwards. #42465 (closed PR,
  Oracle-only snapshot) contains the clearest public numbers on why this matters: one Oracle schema with 14,161
  tables / 162,842 columns / 15,531 indexes / 740 packages over a 100 ms link — `SCHEMA.` completion on a cold
  connection did not appear within 60 s and opening Indexes took 28–31 s. #39295 (open) asks for an offline
  metadata copy for completion.
- **No incremental refresh** (no `xmin`, `modify_date`, `LAST_DDL_TIME` watermarks).

### 3.5 Separate metadata connection and isolated contexts

[Observed: `JDBC/JDBCRemoteInstance.java:95-200`; `MODEL/../ModelPreferences.java:205,338`; `MODEL/DBUtils.java:2133-2146`]

- Every "instance" (database) has a **main context** (`TYPE_MAIN`) and optionally a **metadata context**
  (`TYPE_METADATA`), each being its own JDBC connection.
- `SeparateConnectionBehavior` preference: `ALWAYS`, `NEVER`, or `DEFAULT` = separate unless the driver is
  *embedded* (SQLite, H2, DuckDB — file locking makes a second connection harmful) or the connection is in
  "force single connection" mode.
- The metadata context is opened with **auto-commit on** (`connect(monitor, true, …)`), so catalog reads never
  hold locks or long transactions, and a failed statement in the user's transaction (PostgreSQL "current
  transaction is aborted") does not break the navigator.
- All caches use `DBUtils.openMetaSession` which picks the metadata context when available
  (`getDefaultContext(meta=true)`).
- Caveat handled explicitly: **temporary tables** live only in the session that created them, so the metadata
  connection cannot see them. PostgreSQL reads temp-table columns through the *query* session instead
  (`readTemporaryTableColumns`, `PostgreSchema.java:845-875`).
- `openIsolatedContext(purpose, initFrom)` opens a further throwaway connection initialized from an existing one;
  the object editor uses it when a save must not disturb the caller's transaction (`OPTION_ISOLATED_EXECUTION`,
  `MODEL/edit/DBECommandContext.java`; `MODEL/impl/edit/AbstractCommandContext.java:101-108`).

DataGrip comparison: DataGrip also introspects on its own session unless "single session mode" is on
[report-connectivity §3]. Both tools agree this separation is necessary.

### 3.6 Object filters

#### 3.6.1 Model [Observed: `MODEL/struct/DBSObjectFilter.java`; `plugins/org.jkiss.dbeaver.registry/src/.../FilterMapping.java`; `DataSourceDescriptor.getObjectFilter`]

- `DBSObjectFilter{name, description, enabled, include: List<String>, exclude: List<String>, caseSensitive}`.
- Stored per connection in a `FilterMapping` keyed by **object type class name**, with one **global** filter and
  any number of **per-container** filters keyed by the container's full UI name (prefixed with the instance name
  when it differs from the connection name).
- Lookup walks the requested type's interfaces and superclasses to find a mapping; a container-specific filter
  wins when it is enabled and non-empty, otherwise the global one is used (`firstMatch=false` mode). The editor
  uses `firstMatch=true` to get exactly the stored filter.
- Navigator applies `filter.matches(name)` client-side for every loaded child (DBNDatabaseNode.java:713-731); for
  overloaded objects it matches either the plain name or the overload signature.

#### 3.6.2 Matching rules (ALG-DBV-META-04 has the full spec)

- A mask is a *pattern* if it contains `%`, `*`, `_` or `?`; otherwise it is an **exact name compared
  case-insensitively regardless of the case-sensitivity flag** [Observed: `DBSObjectFilter.java` `matchesPattern`;
  `MODEL/sql/SQLUtils.java:427-430`].
- Pattern translation to regex: `*`/`%` → `.*`, `?`/`_` → `.`, regex metacharacters escaped, backslash escapes the
  next wildcard (`\_` means a literal underscore) [Observed: `SQLUtils.java:433-465`].
- Include list: at least one must match (if non-empty). Exclude list: none may match.
- Empty strings in the lists are ignored.
- Comma-separated input string splits the masks (`SQLUtils.splitFilter`, line 411).

Pitfall worth remembering: **`_` is a single-character wildcard**, so a filter `my_table` also matches `myXtable`.
Users rarely expect this; it comes from SQL `LIKE` semantics.

#### 3.6.3 Server-side filtering (pushdown) — inconsistent across drivers

| Driver | Where the filter is applied | Evidence |
|---|---|---|
| Generic JDBC | Only a **single include mask** is pushed into `DatabaseMetaData.getTables(…, pattern)`; anything else loads all and filters client-side | `GEN/model/meta/GenericMetaModel.java:572-597` |
| PostgreSQL schemas | Full include/exclude pushed via `JDBCUtils.appendFilterClause` (`(nspname LIKE ? OR …) AND NOT (nspname LIKE ? OR …)`), and `pg_catalog` is always force-included | `PostgreDatabase.java:1464-1489`; `JDBC/JDBCUtils.java:716-786` |
| PostgreSQL tables | **Not pushed**; all tables of the schema are loaded, navigator filters | `PostgreSchema.java:879-899` |
| MySQL tables | Pushed into `SHOW FULL TABLES … WHERE Tables_in_x LIKE '…'` when the preference `META_USE_SERVER_SIDE_FILTERS` (default true) is on; include masks translated `*`→`%`, but **exclude masks are not translated** (`NOT LIKE '<raw mask>'`) | `MY/model/MySQLCatalog.java:576-637` |
| SQL Server tables | Pushed into `o.name LIKE '…'`; masks are not translated; when both include and exclude masks exist the generated condition appears to lack an `AND`/`OR` between the include group and the first exclude term | `MS/model/SQLServerSchema.java:413-428` [Inferred: code reading only, not reproduced] |
| Oracle tables/views/MVs | Carefully pushed with bind parameters, `ESCAPE`, case-insensitive includes via `UPPER()`, case-*sensitive* (conservative) excludes, both normal and `UNION` query variants; extensively unit-tested | `TEST/org.jkiss.dbeaver.ext.oracle.test/src/.../OracleTableFilterTest.java` |

Important semantic consequence pinned by an Oracle test: when a filter restricts the loaded list and the cache is
marked full, **a name outside the filter cannot be resolved later** (`getObject("FILTERED_OUT_TABLE")` returns
null without a query), but objects resolved *before* the list was loaded remain resolvable
(`filteredFullCacheDoesNotResolveMissingTableNames`, OracleTableFilterTest.java:260-286). In practice: a filter in
the navigator also hides tables from completion and from foreign-key targets.

### 3.7 PostgreSQL-specific metadata reader

[Observed: `PG/model/PostgreSchema.java`, `PostgreDatabase.java`, `PostgreDataSource.java`, `PG/model/impls/*`]

- **Every catalog join is by OID**, with descriptions from `pg_description` filtered by
  `classoid = '<catalog>'::regclass` and `objsubid` (0 = the object itself, n = column n).
- **Relations** (`pg_class`): kinds `r` (table), `v` (view), `m` (materialized view), `S` (sequence), `f`
  (foreign table), `p` (partitioned table); indexes `i/I` and composite types `c` are excluded from the table list.
  A row with `relispartition` is mapped to a synthetic kind `R` (partition) when the server supports partitions.
  Object creation is delegated to the **server type** (`createRelationOfClass`), so forks can return their own
  subclasses.
- **Columns** (`pg_attribute`): excludes dropped columns; default expression via `pg_get_expr(adbin, adrelid,
  true)`; identity/serial detection via `pg_depend` (`deptype='i'` linking a sequence to the column).
- **Constraints** (`pg_constraint`): one row per constraint (not per column); column membership is the arrays
  `conkey`/`confkey` of 1-based attribute numbers, resolved against the cached columns. Constraint types
  `c,f,n,p,u,t,x` → CHECK, FOREIGN KEY, NOT NULL, PRIMARY KEY, UNIQUE, constraint trigger, exclusion. Check
  expressions via `pg_get_constraintdef`. Mismatched FK arity is logged and truncated (lines 1060-1095).
- **Indexes** (`pg_index`): key vector `indkey`, operator classes `indclass`, per-column options `indoption`
  (bit 0 = DESC — DBeaver notes that `pg_am` flags are unreliable since 9.6), predicate (`indpred`) and expression
  (`indexprs`) through `pg_get_expr`, size and scan count when supported.
- **Version gates** are inline `isServerVersionAtLeast(10,0)` checks (partition expressions) and server-type
  capability methods (`supportsPartitions`, `supportsSequences`, `supportsRelationSizeCalc`,
  `supportsPGConstraintExpressionColumn`, …).
- **Server types / forks**: `PostgreServerExtension` implementations override reading for Redshift, CockroachDB,
  YellowBrick, RisingWave, Greenplum, etc.; for example some forks return native table DDL via `readTableDDL`
  [Observed: `PG/model/impls/redshift/PostgreServerRedshift.java:316`, `impls/cockroach/PostgreServerCockroachDB.java:231`].
  This is DBeaver's equivalent of DataGrip's "family + extension fallback" (YUGABYTE→POSTGRES).
- **Statistics** (row counts, sizes) are separate lazy reads (`collectObjectStatistics`, `pg_total_relation_size`
  per schema) [Observed: `PostgreSchema.java:665-700`].

### 3.8 MySQL-specific metadata reader

[Observed: `MY/model/MySQLCatalog.java`, `MySQLTableBase.java`]

- Catalog ("database") level caches: tables, indexes, unique keys, check constraints, procedures, packages
  (MariaDB), triggers, events, sequences (MariaDB) (lines 567-1130).
- Tables: `SHOW FULL TABLES FROM \`db\`` (plain `SHOW TABLES` on MySQL 4); view vs table from `Table_type`.
- Columns: `information_schema.COLUMNS WHERE TABLE_SCHEMA=? [AND TABLE_NAME=?] ORDER BY ORDINAL_POSITION` — the
  same struct-cache bulk/single pattern.
- Indexes: `information_schema.STATISTICS` ordered by table, index, `SEQ_IN_INDEX`.
- Unique keys: `information_schema.KEY_COLUMN_USAGE WHERE REFERENCED_TABLE_NAME IS NULL`; check constraints:
  `information_schema.CHECK_CONSTRAINTS` (joined with `TABLE_CONSTRAINTS`).
- **Lazy "additional info"**: engine, row format, auto-increment, sizes, create/update times come from
  `SHOW TABLE STATUS FROM db`, read once for the whole catalog only when a property needing it is displayed
  (`cacheTablesAdditionalInfo`, lines 448-472).
- **DDL**: `SHOW CREATE TABLE` / `SHOW CREATE VIEW` — the server's own authoritative text; synthesized DDL only for
  not-yet-persisted tables (`MySQLTableBase.getDDL`, lines 143-170).
- Known performance issue: foreign-key/relations query on large MySQL 5.7 instances without a `TABLE_SCHEMA`
  predicate scanning all schemas (#39169, open).

### 3.9 SQL Server-specific metadata reader

[Observed: `MS/model/SQLServerSchema.java`, `MS/SQLServerUtils.java`]

- All catalog views are referenced **fully qualified with the database**: `[db].sys.all_objects`, so a single
  connection can browse every database on the instance without `USE` (`getSystemTableName`, SQLServerUtils.java:193-240).
- **Babelfish** (PostgreSQL emulating SQL Server): `getSystemTableName` substitutes in-line sub-queries over
  `pg_catalog` that mimic `sys.default_constraints`, `sys.all_columns`, `sys.index_columns`, etc. — a pragmatic
  "compatibility view" layer.
- Tables: `sys.all_objects WHERE type IN ('U','S','V','TT') AND schema_id=<id>` plus `MS_Description` extended
  property for comments and `sys.tables.is_external` when supported; temp tables in `tempdb` have their generated
  suffix stripped.
- Columns: `sys.all_columns JOIN all_objects LEFT JOIN default_constraints LEFT JOIN extended_properties LEFT JOIN
  computed_columns`, plus `COLUMNPROPERTY(…,'charmaxlen')` when available, ordered by object and column id.
- Indexes: `sys.indexes, sys.index_columns, sys.all_objects` joined, ordered by object, index, `key_ordinal`;
  index types heap/clustered/nonclustered/XML/spatial/columnstore/hash enumerated.
- **Source/DDL of modules** (views, procedures, functions, triggers): `SELECT definition FROM sys.sql_modules
  WHERE object_id=…`, or `OBJECT_DEFINITION(id)` on Azure / data-warehouse; on SQL error retries the other form
  (SQLServerUtils.java:260-305). Views' text is **rewritten from `CREATE` to `ALTER`** (or `CREATE OR ALTER` on
  v16+, non-Microsoft drivers) so that "save" re-executes it (changeCreateToAlterDDL, lines 410-424).
- Tables' DDL is **synthesized** (`SQLServerTable.getObjectDefinitionText → DBStructUtils.generateTableDDL`),
  because SQL Server has no `SHOW CREATE TABLE` (#22139 was a historical NVARCHAR length bug in this path).

### 3.10 Generic JDBC fallback

[Observed: `GEN/model/meta/GenericMetaModel.java` (1178 lines), `plugins/org.jkiss.dbeaver.ext.generic/plugin.xml:1928-1960`]

- `GenericMetaModel` is a strategy object with ~80 overridable methods: `loadSchemas`, `prepareTableLoadStatement`,
  `prepareTableColumnLoadStatement`, `prepareUniqueConstraintsLoadStatement`, `prepareForeignKeysLoadStatement`,
  `prepareIndexesLoadStatement`, `getTableDDL`, `getViewDDL`, `getProcedureDDL`, flags like
  `supportsNullSchemas`, `isTrimObjectNames` (Firebird pads names with spaces), `supportNestedForeignKeys`
  (Informix needs FKs outside the `CREATE TABLE` body), `isSystemTable` (`TABLE_TYPE` contains `SYSTEM`).
- Default implementations call `DatabaseMetaData` (`getSchemas`, `getTables`, `getColumns`, `getPrimaryKeys`,
  `getImportedKeys`, `getIndexInfo`) with wildcard escaping of names.
- `loadSchemas` contains a decade of driver workarounds: try `getSchemas(catalog, pattern)`, fall back to
  `getSchemas()` (optionally switching the connection catalog first, for Netezza), accept `TABLE_OWNER` when
  `TABLE_SCHEM` is empty, skip schemas belonging to another catalog, retry without catalog if exactly one catalog
  returned zero schemas, and **drop a single unnamed schema** as "most likely fake" (lines 148-312).
- An extension point `org.jkiss.dbeaver.generic.meta` allows declaring per-driver meta models **declaratively in
  XML**, including mapping logical columns (`TABLE_NAME`, `TYPE_NAME`, …) to **result-set column indexes** for
  drivers whose `DatabaseMetaData` result sets have wrong column names ("indexed" meta model).
- 43 Java classes extend `GenericMetaModel` across the repo (for DuckDB, H2, Informix, SAP HANA, Snowflake…).

DataGrip comparison: DataGrip has a "GenericSQL" fallback for ~19 drivers and an `jdbcMetadataWrapper` extension
point to patch broken driver metadata [report-connectivity §1]. Same idea, different packaging.

### 3.11 DDL generation

#### 3.11.1 Entry points

- `DBPScriptObject.getObjectDefinitionText(monitor, options)` on any object.
- `DBStructUtils.getTableDDL` (lines 170-192): if the object implements `DBPScriptObject` and returns non-empty
  text use it; otherwise synthesize via `generateTableDDL`.
- `DBStructUtils.generateTableDDL` (lines 118-138): look up the registered `SQLObjectEditor` for the table class
  (object managers are registered in each driver's `plugin.xml`); if it is a `SQLTableManager` call
  `getTableDDL` and render actions to a script; otherwise emit a comment "Can't generate DDL".
- `DBStructUtils.generateObjectDDL` (lines 141-157): same for any object via its manager's create command.
- `generateTableListDDL` (lines 204-288): many tables, dependency-ordered (ALG-DBV-META-06).

#### 3.11.2 Options [Observed: `MODEL/DBPScriptObject.java:31-78`]

| Option key | Effect |
|---|---|
| `refresh` | Re-read cached source (views/procedures) before returning |
| `useFQN` | Fully qualify names |
| `script.includeDrop` / `ddl.skipDrops` | Include/skip the commented `DROP` |
| `script.format.compact` | One-line output |
| `ddl.skipForeignKeys` / `ddl.onlyForeignKeys` | Split FKs out (for cyclic dependencies) |
| `ddl.separateForeignKeys` | Default true: in multi-table output put FKs in separate statements |
| `ddl.skipUniqueKeys` / `ddl.skipIndexes` | Omit constraints/indexes |
| `ddl.includeNestedObjects` | For schemas: include all contained objects |
| `ddl.includeComments` / `ddl.includePermissions` / `ddl.includePartitions` | Add `COMMENT ON`, `GRANT`, partitions |
| `ddl.objectFilter` | A `DBSObjectDDLFilter` deciding per object (used by the AI feature to limit schema DDL sent to an LLM: `TEST/org.jkiss.dbeaver.model.ai.test/.../AISchemaDDLFilterTest.java`) |
| `ddl.source` / `object.save` / `object.alter` | Context: generating for display vs saving |

#### 3.11.3 Synthesis pipeline for a table (ALG-DBV-META-03 has the full spec)

`SQLTableManager.getTableDDL` (`MODEL/impl/sql/edit/struct/SQLTableManager.java:245-388`) builds a
`StructCreateCommand` for the table and **aggregates** a create-command for each column, constraint (PK/UK),
check constraint, foreign key and index (skipping hidden and inherited ones, and those excluded by options or
filter), plus database-specific extras. `addStructObjectCreateActions` (lines 85-171) then sorts the nested
commands by the manager's `getChildTypes()` order (PostgreSQL: column, constraint, foreign key, policy, index —
`PG/edit/PostgreTableManager.java:46-52`), asks each for a **nested declaration** (a fragment that goes inside the
parentheses — e.g. `id int4 NOT NULL`, `CONSTRAINT pk PRIMARY KEY (id)`), and for commands without a nested
declaration (indexes, comments) asks for **separate persist actions** emitted after the `CREATE TABLE`.

Column declaration = quoted name + data type + type modifiers (length/precision) + `NOT NULL` + `DEFAULT`
(`SQLTableColumnManager.java:54-100,186-210,304-345`). Defaults are quoted only for string/date kinds when the text
does not already start with a quote, letter, `(` or `[` — a heuristic, because the model stores defaults as raw
SQL expressions.

PostgreSQL-specific layers [Observed: `PG/edit/PostgreTableManager.java`, `PostgreTableManagerBase.java`,
`PG/model/impls/PostgreServerExtensionBase.java:201-340`]:

- Forks may return native table DDL (`readTableDDL`) which then wins entirely.
- Partitions: `CREATE TABLE p PARTITION OF parent FOR VALUES …` with no column list.
- Table modifiers: `INHERITS (…)`, `PARTITION BY …`, `USING <access method>`, `WITH (storage params)`,
  `TABLESPACE`.
- Extra actions after the table: `COMMENT ON TABLE/COLUMN/CONSTRAINT`, `ALTER COLUMN SET STORAGE` for non-default
  storage, triggers, rules, policies, grants.
- PK-backing indexes are skipped (they are implied by the constraint); partitions do not get a `DROP` line.
- Column comments are additionally rendered **inline as `-- comment`** after each column in the readable DDL,
  with the comma placed *before* the comment (pinned by `PostgreTableBaseTest`).

#### 3.11.4 Edit → ALTER: the command model

[Observed: `MODEL/edit/DBECommandContext.java`, `MODEL/impl/edit/AbstractCommandContext.java`,
`MODEL/impl/sql/edit/SQLObjectEditor.java`, `SQLStructEditor.java`]

- Every UI edit (rename column, change type, add index) becomes a `DBECommand` added to the editor's
  `DBECommandContext` together with a *reflector* (UI update for undo/redo).
- On save (`saveChanges`, AbstractCommandContext.java:85-160):
  1. Reconnect if needed; **validate** all commands (e.g. "Empty table name", "Table 'x' already exists" checked
     against the cache — SQLObjectEditor.java:445-468).
  2. Optionally switch to an isolated connection; switch auto-commit off if the DB supports transactional DDL.
  3. `getCommandQueues` (lines 593-725): group commands per object; **merge** consecutive commands (a command's
     `merge(prev)` may return itself, the previous command, a new composite command, or `null` meaning "both
     cancel out", e.g. create+delete); apply manager-specific filters; let a pending `StructCreateCommand`
     **aggregate** all nested commands of a new table so a brand-new table is emitted as one `CREATE TABLE`
     instead of `CREATE` + N `ALTER`s; move **rename commands to the head** (#7512) so later commands use the new
     name; collapse multiple renames into first-old→last-new and drop renames of objects created in the same batch
     (#11917, SQLObjectEditor.java:577-604).
  4. Execute each command's persist actions (`NORMAL`, `OPTIONAL` — failure ignored, `FINALIZER` — runs even after
     an error, `COMMENT` — skipped) on a `META_DDL` session; **commit after each command** when in manual mode,
     because most databases cannot roll DDL back anyway (lines 162-305, comment at 233-235).
  5. On error: roll back the current command's transaction and rethrow.
  6. Finally: run reflectors, call `updateModel()` on executed commands (e.g. mark object persisted, remove deleted
     objects from caches, fire model events), clear queues, notify listeners.
- PostgreSQL table `ALTER` generation only covers table-level properties (partition key, OIDs, row-level security,
  tablespace) — column/constraint changes are handled by their own managers
  (`PostgreTableManager.java:160-194`).

#### 3.11.5 Schema compare in Community Edition

[Observed: `plugins/org.jkiss.dbeaver.cmp.simple/src/.../CompareObjectsExecutor.java` (383 lines)]

- Input: two or more navigator nodes. Recursively: compare `@Property` values of the objects (optionally lazy
  properties and DDL scripts), then match children **by display name**, recurse; missing children mark a
  difference. `DBSObjectContainer.cacheStructure(STRUCT_ALL)` is called first to bulk-load.
- Named-object property values are compared by name only. Output is an HTML-like report (`CompareReportRenderer`).
- **No diff-to-migration-script.** DataGrip's model diff produces ordered Creation/Alteration/Remotion operations
  through a script generator [report-nosql-model §0.5]; DBeaver CE does not.

---

## 4. Algorithm specifications

### ALG-DBV-META-01: Metadata cache invalidation and refresh

- **Problem:** Keep an in-memory catalog consistent with the server after (a) the user explicitly refreshes a node,
  (b) the user runs DDL in the SQL editor, (c) the user edits objects through the UI — without breaking references
  held by open editors and without re-reading more than necessary.
- **Inputs / outputs:** Input: a refresh request `{node | target ∈ DataSource | Catalog(name) | Schema(cat?, name)}`
  and the current cache graph. Output: updated caches; navigator node events (ADD/REMOVE/REFRESH); the same Java
  object instances for objects that still exist.
- **Algorithm (DBeaver behavior, distilled):**
  1. Normalize request: folder node → parent object node. Coalesce identical pending manual requests. Serialize
     requests by key `connection-id` (one refresh per connection at a time). Skip nodes that are descendants of a
     node already refreshed in this batch.
  2. For an object `O` implementing `refreshObject`:
     1. Clear the caches *owned by* `O` (container-level: all child caches; table-level: only this table's slice
        in shared composite caches, via `clearObjectCache(O)`; plus per-object derived state: statistics, source
        text, privileges).
     2. Ask `O`'s *parent cache* to refresh `O`'s own row:
        - if the parent list was never fully loaded → load it fully;
        - else run the single-object lookup; if found → clear `O`'s child cache, deep-copy fields from the new
          instance into `O`; if not found → remove `O` from the parent cache (`resetFullCache=false`) and return
          `null`.
  3. If `refreshObject` returned `null`: remove the node from its parent and dispose it. Else reconcile children
     (ALG-DBV-META-07).
  4. After the batch: fire a "metadata refreshed" event per affected connection (completion and other listeners
     drop their own derived caches).
  5. DDL path: classify each successful statement → `SQLObjectOperation`; resolve to the narrowest container
     target (schema > catalog > data source; unqualified names use current defaults); if auto-commit or DDL is
     non-transactional → refresh now (subject to ASK/ALWAYS/NEVER); if transactional and the editor uses the
     navigator's session → defer to commit; on rollback → compensating refresh of targets refreshed during the
     transaction (forced even if preference is NEVER).
  6. Editor-save path: each executed command's `updateModel()` updates the cache directly (insert new object,
     remove deleted, rename key via `renameObject`) — no re-query.
- **Data structures:** per-cache `{list, name_map (lazy), full: bool, missing_names: set, children: IdentityMap<parent, child_cache>}`;
  refresh queue `[PendingRefresh{nodes, keys, completion, user_initiated}]` + `active_keys: set`.
- **Complexity / performance:** Refreshing a schema is O(size of schema) queries-wise (≈ 1 query per cache type when
  later accessed in bulk, or N queries if accessed per table). Merge is O(n²) by name (`mergeCache`) — fine for
  thousands, noticeable for 100k objects. No incremental mode: a 15k-table schema is fully re-read.
- **Edge cases (exhaustive list):**
  1. Object dropped on server → lookup returns nothing → node removed; open editor for it must close or show "deleted".
  2. Object renamed on server → appears as drop + create; identity is lost (old node disposed, new node created).
  3. Object replaced with a different kind under the same name (table → view) → `equalObjects` compares class, so
     node is replaced.
  4. Name differs only by case (PostgreSQL `"Foo"` and `foo`) → lookup prefers exact case; case-insensitive
     dialects return the first other match.
  5. Object created in UI but not saved (non-persisted) → never queried; preserved across parent refresh (#40920).
  6. Name stored with quotes at creation time → quotes stripped before lookup (#20383).
  7. Refresh during an in-progress load of the same cache → `synchronized` loaders serialize; second caller sees
     fresh state.
  8. Cancellation mid-refresh → cache stays cleared (not half-filled); next access re-reads.
  9. Connection lost during refresh → reconnect-and-retry (`tryExecuteRecover`); on data-source node failure the
     connection is disconnected and an error shown.
  10. Refresh of a node that is locked (being edited/refreshed) → skipped with a warning.
  11. Uncommitted DDL in a different session than the navigator → invisible; DBeaver informs instead of refreshing.
  12. DDL in manual-commit mode followed by rollback → navigator may show phantom objects until compensation runs.
  13. Savepoint rollback → compensation for targets refreshed since.
  14. `DROP SCHEMA x` while `x` is the current default schema → target widens to catalog/data source.
  15. Statement not recognized (vendor-specific DDL, dynamic SQL in a procedure) → no refresh; user must F5.
  16. Same object visible under two navigator paths → each occurrence refreshed via model events.
  17. Negative cache: a table created by another session after a failed lookup stays "missing" until refresh.
  18. Statistics properties (sizes) may be stale independently of structure; they reset only on table refresh
      (optionally schema statistics too).
- **Test cases (input → expected):**
  1. Cache full with tables {a,b}; server now {a,c}; refresh schema → list {a,c}; node a is the *same* object instance as before; b disposed; c new.
  2. Refresh table `a` whose columns were loaded; server added column `x` → `a` same instance, column cache cleared, next `getAttributes` returns new list including `x`.
  3. Refresh table `a` after server `DROP TABLE a` → `refreshObject` returns null, node removed.
  4. Lookup `zzz` (absent) twice → exactly one query (negative cache); after `clearCache` → one more query.
  5. Filtered full cache (`VISIBLE*`), lookup `REFERENCED_TABLE` before list load → resolved and kept; after list load, lookup `FILTERED_OUT_TABLE` → null with no query (DBeaver test `OracleTableFilterTest.filteredFullCacheDoesNotResolveMissingTableNames`).
  6. Execute `CREATE TABLE s.t(...)` in auto-commit with behavior ALWAYS → schema `s` refreshed once; with NEVER → no refresh.
  7. Manual commit, PostgreSQL, `CREATE TABLE t`, then `ROLLBACK` after the user accepted an early refresh → compensating refresh offered/forced; `t` disappears.
  8. Two quick F5 presses on the same node → one refresh job.
- **Prior art and evidence:** DBeaver files cited in §3.2, §3.4. DataGrip [report-connectivity §4]: incremental
  introspection with per-object state numbers and per-schema watermarks (PostgreSQL `age(xmin) <= #TXAGE` and
  current xid; SQL Server `modify_date` + checksums; Oracle `last_ddl_time`), "RefreshAffectedObjects" after DDL,
  "CheckUptodateness" task, persisted model per data source. DataGrip is strictly better at large-schema
  freshness; DBeaver's identity-preserving merge and transaction-aware DDL refresh are worth copying.
- **Hacivat recommendation:**
  - Catalog = immutable `Arc<CatalogSnapshot>` per connection published through `arc_swap::ArcSwap`; readers
    (UI, completion, MCP) never lock. Refresh builds a *new* snapshot for the affected subtree and swaps;
    identity is a stable `ObjectKey {kind, qualified_name, server_oid_or_object_id?}`, not a pointer, so "identity
    preservation" is free.
  - Per container: `LoadState = NotLoaded | Loading{generation} | Loaded{at, watermark, generation} | Failed{err, at}`.
  - Refresh levels (DataGrip-like): `Names` (L1), `Structure` (L2), `Sources` (L3); refresh-after-DDL targets the
    *object* when the statement names one (fall back to schema).
  - Incremental refresh with watermarks: PostgreSQL `xmin`-age + `txid_current()`; SQL Server `modify_date` +
    existence diff; MySQL `information_schema.TABLES.UPDATE_TIME/CREATE_TIME` (weak → schema-level re-read of
    names, full read of changed tables); SQLite `PRAGMA schema_version`; Oracle `LAST_DDL_TIME`.
  - Transaction-aware DDL refresh exactly like DBeaver: defer on transactional DDL engines, compensate on rollback.
  - Negative cache with TTL (e.g. 30 s) instead of "until refresh", so objects created elsewhere eventually appear.
  - Persist snapshots to `<data_dir>/catalog/<conn-uuid>.sqlite` (format version, per-schema watermark), load at
  connect, then validate in background (what #42474 proposes for DBeaver).

### ALG-DBV-META-02: Lazy hierarchical cache loading (struct cache, bulk vs per-parent)

- **Problem:** Load parents (tables) and their children (columns) on demand, minimizing round trips: one query per
  container when the caller needs many parents' children, one query per parent when only one is needed, and never
  repeat work.
- **Inputs / outputs:** Input: `owner` (schema), optional `forObject` (one table) or bulk; parent-name column used
  to route bulk rows. Output: per-parent child lists in the cache; `children_complete` flag for bulk.
- **Algorithm:**
  ```
  load_children(owner, for_parent?):
    if force_cache_mode or cancelled: return
    if for_parent is None and bulk_complete: return
    if for_parent and (not for_parent.persisted or has_children_entry(for_parent)): return
    if for_parent is None: load_parents(owner)              # full parent list (single query)
    batches = for_parent ? [None] : bulk_batches(owner)     # [None] = one unrestricted query
    if batches is empty:
        if not cancelled and parents_fully_cached: bulk_complete = true
        return
    collected = map<parent, list<child>>
    for batch in batches:
        if cancelled: return                                 # nothing published
        rows = query_children(owner, for_parent, batch)
        for row in rows:
            if cancelled: return
            p = for_parent ?? parent_by_name(row[parent_name_col])   # skip row if name null or unknown
            if has_children_entry(p): continue                # someone else loaded it meanwhile
            c = build_child(p, row); if c: collected[p].push(c)
    if cancelled: return
    for (p, cs) in collected: if not has_children_entry(p): set_children(p, cs)
    if for_parent is None:
        if collected is empty and not empty_bulk_is_complete(): return   # driver may not support bulk
        bulk_parents = batches == [None] ? all_parents : union(batches)
        for p in bulk_parents: if not has_children_entry(p): set_children(p, [])
        bulk_complete = true
    elif for_parent not in collected: set_children(for_parent, [])
  ```
  Oracle batching policy: if a name filter restricted the parent list and ≤ 10,000 parents are cached → batches of
  500 names (`IN (?,…)`), else one owner-wide query; an empty bulk result counts as complete only when the filter
  is active.
- **Data structures:** `IdentityMap<Parent, ChildCache>`; `bulk_complete: bool`; parent cache with name map.
- **Complexity / performance:** Bulk: 1 query (or ⌈P/500⌉) returning all columns of a schema — O(total columns) rows;
  per-parent: 1 query per table. The crossover favors bulk once more than a handful of tables will be needed;
  DBeaver lets the *caller* decide, which causes N+1 storms when per-table APIs are used in loops (#42468).
  Holding the cache lock (`synchronized`) for the whole read blocks concurrent readers of the same cache.
- **Edge cases (exhaustive list):**
  1. Driver cannot read children of all parents at once (SQLite PRAGMA-based) → empty bulk result → do not mark complete.
  2. Bulk result legitimately empty (all tables have zero columns, or filtered list) → must be marked complete only if known valid.
  3. Child row names a parent not in the parent cache (filtered out, created concurrently, case mismatch) → skip row.
  4. Parent name column NULL → skip row.
  5. Child rows for the same parent split across batches → collected before publishing.
  6. Cancellation between batches → nothing published; next call retries all (test `cancellationBeforeSecondBatchDoesNotMarkBulkLoadComplete`).
  7. Parent list load cancelled/unfinished → bulk must not be marked complete (tests `canceledParentLoadDoesNotPreventBulkColumnRetry`, `unfinishedParentLoadWithoutCancellationDoesNotMarkBulkLoadComplete`).
  8. A parent's children loaded individually before the bulk load → bulk skips its rows (no overwrite).
  9. Concurrent single-parent load during bulk load → `has_children_entry` check before publishing.
  10. New (non-persisted) parent → never queried; gets an empty child cache on first access.
  11. Partitioned or inherited tables whose columns are inherited → database-specific `build_child` filtering.
  12. Names with trailing spaces (Firebird) → trimming when reading parent-name column.
  13. Huge schema exceeding max cache size (1,000,000) → truncation with warning.
  14. Temporary tables invisible to metadata connection → read via query session.
  15. Columns whose type is unknown to the client → child still built with raw type name.
- **Test cases (input → expected):** (from `TEST/org.jkiss.dbeaver.ext.oracle.test/.../OracleBulkColumnCacheTest.java`)
  1. Filter `TABLE_*`, 502 cached tables → 2 batches (500 + 2); second batch SQL has `TABLE_NAME IN (?,?)` bound to `TABLE_500`, `REFERENCED_TABLE`.
  2. Same filter, 10,001 tables → 1 batch, owner-wide query (`WHERE c.OWNER=?`, no `IN`).
  3. No filter → owner-wide query.
  4. Filter active, zero cached tables → `load_children` twice performs **no** query.
  5. Single-table request → `WHERE c.OWNER=? AND c.TABLE_NAME=?` (no `IN`).
  6. Our own: bulk returns rows for tables {a,b} of {a,b,c} → c gets empty list, bulk_complete=true; a subsequent `children(c)` performs no query.
  7. Our own: SQLite-like driver returns 0 rows for bulk → bulk_complete stays false; `children(a)` triggers per-table query.
- **Prior art and evidence:** `JDBC/cache/JDBCStructCache.java:111-224`; `ORA/model/OracleSchema.java:744-745,951-970`;
  PostgreSQL bulk/single SQL `PostgreSchema.java:922-951`. DataGrip: levels L1/L2/L3 and fragment introspection;
  one batch per schema per level [report-connectivity §4].
- **Hacivat recommendation:** Make the *loader*, not the caller, choose: a request for children of table T
  schedules a **single-parent query immediately** and, if ≥ K (e.g. 3) distinct tables of the same schema were
  requested within a short window or the schema has < N (e.g. 2,000) tables, upgrades to a **bulk load** for the
  schema. Use single-flight (`tokio::sync::OnceCell`/`Shared` futures keyed by `(container, cache_kind)`) so
  concurrent requests share one query. Run independent bulk queries (columns, constraints, indexes) **in
  parallel** on a small metadata connection pool. Publish atomically per container.

### ALG-DBV-META-03: Table DDL composition

- **Problem:** Produce a correct, readable `CREATE TABLE` script for a table from the in-memory model (when the
  server has no authoritative DDL function), including constraints, indexes, comments, and database-specific
  clauses, controllable by options.
- **Inputs / outputs:** Input: table object (with loaded columns, constraints, FKs, indexes), options map, dialect.
  Output: ordered list of "persist actions" (statements or comments), rendered to a script with delimiters.
- **Algorithm:**
  1. If an object filter option is present and excludes the table → empty.
  2. If `only_foreign_keys` → for each FK (skip hidden/inherited/filtered) emit its standalone `ALTER TABLE … ADD
     CONSTRAINT … FOREIGN KEY` and stop.
  3. If the table is persisted, the dialect wants a drop line, and `skip_drops` is not set → emit comment
     `-- Drop table` and a **commented-out** `-- DROP TABLE <fqn>;`.
  4. Create the aggregate `StructCreateCommand(table)`. Aggregate create-commands for: every column (skip
     hidden/pseudo and inherited); every PK/UK constraint unless `skip_unique_keys`; every check constraint; every
     FK unless `skip_foreign_keys`; every index unless `skip_indexes` (dialects exclude PK-backing indexes);
     dialect extras.
  5. If the dialect can return native DDL for the persisted table → use it as the first action and skip 6–8.
  6. Sort nested commands by the manager's child-type order (columns, constraints, FKs, …).
  7. Open `CREATE <TABLE|VIEW|UNLOGGED TABLE|TEMPORARY TABLE|FOREIGN TABLE> <name> (` (partition-of form has no
     column list). For each nested command: if it yields an inline declaration, append it (comma-separated; when a
     previous declaration ends with a `-- comment`, insert the comma *before* the comment; in compact mode strip
     inline comments); otherwise collect its standalone actions (e.g. `CREATE INDEX`).
  8. Close `)`, append table modifiers (inheritance, partition key, access method, `WITH (…)`, tablespace),
     insert the `CREATE TABLE` action at position 0 of the generated actions.
  9. Append extra actions: table/column/constraint comments (if requested or when saving), storage settings,
     triggers, rules, policies, grants.
  10. Render: each action's script + delimiter; comments as comment lines; blank lines between groups.
- **Data structures:** `StructCreateCommand{object_commands: LinkedMap<object, nested_command>}`; persist action
  `{title, script, type ∈ NORMAL|OPTIONAL|FINALIZER|COMMENT}`.
- **Complexity / performance:** O(columns + constraints + indexes). Cost is dominated by *loading* those children
  (3–4 queries per table if not bulk-loaded; DDL for a whole schema bulk-loads first via `cacheStructure`).
- **Edge cases (exhaustive list):**
  1. Table with zero columns (PostgreSQL allows it) → `CREATE TABLE t ()`.
  2. Partition table → `PARTITION OF parent FOR VALUES …`, no columns, no drop line.
  3. Inherited columns/constraints (PG `INHERITS`) → skipped in body, `INHERITS (…)` clause added.
  4. Identity / serial columns → type rendered as declared (`serial` vs `int4 DEFAULT nextval(…)`) — dialect-specific.
  5. Defaults that are expressions vs literals → quoting heuristic; risk of wrongly quoting a function call starting with a non-letter.
  6. Column comments containing `'`, `--`, `"`, newlines → `COMMENT ON` string-escaped (`''`), inline `--` comment kept on one line.
  7. Reserved-word or mixed-case identifiers → dialect quoting.
  8. Expression and partial indexes → definitions from server (`pg_get_indexdef`, predicate).
  9. Foreign keys to tables not yet created (cycles) → split via `skip_foreign_keys` / `only_foreign_keys`.
  10. FK referencing a table in another schema → fully qualified reference.
  11. Unnamed constraints → synthetic names (`<PARENT>_#DBOBJ`) must not leak into DDL; dialect generates `table_pkey`-style names or omits `CONSTRAINT name`.
  12. Generated/computed columns (SQL Server `computed_columns`, PG `GENERATED ALWAYS AS`) → expression rendered, no type defaults.
  13. Collation per column; tablespace; storage params; `WITH OIDS` on old PG.
  14. Hidden/pseudo columns (`oid`, `ctid`, `ROWID`) → excluded.
  15. Views → separate path (server definition text), triggers appended.
  16. Unknown data type (driver did not report it) → type name emitted as-is with a debug log.
  17. Compact mode → single line, inline comments stripped.
  18. Native server DDL available (MySQL `SHOW CREATE TABLE`, some PG forks) → used verbatim; options like
      `skip_indexes` cannot be honored.
- **Test cases (input → expected)** (from DBeaver tests):
  1. PG regular table `test_schema.test_table(column1 int4)` → `CREATE TABLE test_schema.test_table (\n\tcolumn1 int4 NULL\n);\n` (`PostgreTableBaseTest.generateTableDDL_whenTableHasOneColumn…`, lines 97-116).
  2. Two columns → `\tcolumn1 int4 NULL,\n\tcolumn2 varchar NULL` (lines 118-139).
  3. Column comment `PM's approver of record` with include-comments → body line `\tcolumn1 int4 NULL, -- PM's approver of record` and after the table `-- Column comments` + `COMMENT ON COLUMN test_schema.test_table.column1 IS 'PM''s approver of record';` (lines 141-171).
  4. Comment containing `--` and `"double quotes"` → kept verbatim inline, escaped only for `'` in `COMMENT ON` (lines 173-208).
  5. Generic new table with PK → `\tCONSTRAINT NEWTABLE_PK PRIMARY KEY (Column1)` inside the body (`GenericBaseTableDDLTest`, lines 150-195).
  6. Generic persisted table → starts with `-- Drop table\n\n-- DROP TABLE CATALOG_GENERIC.TABLE_GENERIC;\n\n` (lines 260-282).
  7. Generic required columns → `COLUMN1 DATE NOT NULL,` … (lines 310-335).
  8. Comment statements: `COMMENT ON FOREIGN TABLE test_schema."testForeignTable" IS 'Test comment';`, likewise VIEW / MATERIALIZED VIEW (PostgreTableBaseTest lines 227-283).
- **Prior art and evidence:** `MODEL/impl/sql/edit/struct/SQLTableManager.java:85-388`,
  `SQLTableColumnManager.java`, `MODEL/impl/sql/edit/SQLStructEditor.java`, `PG/edit/PostgreTableManager*.java`,
  `PG/model/impls/PostgreServerExtensionBase.java:289-340`; MySQL native DDL `MY/model/MySQLTableBase.java:143-170`.
  DataGrip generates DDL from its model through per-kind Create/Alter/Drop producers and a scenario ordering step
  [report-nosql-model §0.5].
- **Hacivat recommendation:** Implement `fn table_ddl(table: &TableModel, opts: &DdlOptions, dialect: &dyn DdlDialect) -> Vec<DdlStatement>`
  as a **pure function over the snapshot** (no I/O), with golden-file tests per dialect (`insta` snapshots). Keep
  a typed IR (`CreateTable{name, columns, inline_constraints, modifiers}`, `CreateIndex`, `Comment`, `Grant`) and a
  per-dialect renderer, so the same IR also serves schema diff/migration. Prefer server-authoritative text where
  it is *exact* (MySQL `SHOW CREATE TABLE`, SQLite `sqlite_schema.sql`, PG `pg_get_viewdef`/`pg_get_functiondef`/
  `pg_get_indexdef`/`pg_get_constraintdef`), but always also be able to synthesize (for cross-database copy and
  diff). Never quote defaults heuristically: store defaults as `DefaultExpr::Raw(String)` from the server and emit
  verbatim.

### ALG-DBV-META-04: Object filter matching and server-side pushdown

- **Problem:** Let users restrict which objects are loaded/shown with include/exclude masks, applied consistently
  client-side and (when possible) server-side to reduce load.
- **Inputs / outputs:** Input: object type, parent container, name(s); filter `{enabled, include[], exclude[],
  case_sensitive}`. Output: boolean match; or SQL predicate + bind parameters.
- **Algorithm (DBeaver):**
  1. Resolve filter: mapping by object type (walk interfaces, then superclasses); container-specific filter if
     enabled and non-empty, else the global filter for that type.
  2. Not applicable if disabled or both lists empty → match everything.
  3. Compile masks lazily: empty strings ignored; mask containing any of `% * _ ?` → regex (`*`,`%`→`.*`;
     `?`,`_`→`.`; escape regex metachars; `\x` escapes wildcard `x`), case-insensitive unless `case_sensitive`;
     otherwise keep as plain string compared with **case-insensitive equality** always.
  4. Match: if include non-empty, at least one include must match (whole-string match); then no exclude may match.
     Overloaded routines: match if any of (plain name, signature) matches.
  5. Pushdown (best drivers): single include → `col LIKE ?`; general → `(col LIKE ? OR …) AND NOT (col LIKE ? OR …)`
     with binds `make_sql_like(mask)` (`*`→`%`, `?`→`_`), `ESCAPE` for backslash; case-insensitive includes as
     `UPPER(col) LIKE UPPER(?)`; excludes case-sensitive (conservative: never hide more than the client would).
- **Data structures:** compiled `Vec<Pattern | Literal>` per list, invalidated on edit.
- **Complexity / performance:** O(masks) per name; regex compiled once. Pushdown reduces rows transferred for big schemas.
- **Edge cases (exhaustive list):**
  1. `_` in a literal table name is a wildcard (`my_table` matches `myXtable`) → user must write `my\_table`.
  2. Literal masks ignore `case_sensitive` (always case-insensitive) while pattern masks honor it — inconsistent.
  3. Empty mask in include list → ignored (not "match empty name").
  4. Include list containing only empty strings → treated as no include → match all.
  5. Disabled filter → ignored entirely.
  6. Container filter present but empty → falls back to global filter (in non-first-match mode).
  7. Filter references a container by display name; renaming the container orphans the filter.
  8. Pushdown with different case rules than client matching → server may return rows the client then hides (OK) or hide rows the client would show (bug); Oracle tests pin "excludes stay case-sensitive".
  9. Mask with regex metacharacters (`a+b`, `x.y`) → escaped.
  10. Trailing lone backslash → dropped.
  11. Driver ignores pattern arguments in `DatabaseMetaData` → client-side filter still applies.
  12. Filter on schemas must never hide the system catalog needed for types (`pg_catalog` force-included in PostgreSQL).
  13. Filtered full cache → names outside the filter become unresolvable for completion/FK targets.
  14. Unicode names and locale-dependent case folding (Turkish `i`/`İ`) in `UPPER()` on the server vs Java regex case-insensitivity (Unicode-aware only with flags).
- **Test cases (input → expected):** (from `OracleTableFilterTest`)
  1. includes `ABC`, `*JOB_1234*`; excludes `BAD*`, `OLD` → SQL contains `(UPPER(O.OBJECT_NAME)=UPPER(?) OR UPPER(O.OBJECT_NAME) LIKE UPPER(?)` and `AND NOT (O.OBJECT_NAME LIKE ?`; binds `ABC`, `%JOB_1234%`, `BAD%`, `OLD`.
  2. View filter exclude `HIDDEN*` only → `O.OBJECT_TYPE='VIEW' AND NOT (O.OBJECT_NAME LIKE ?` bind `HIDDEN%`.
  3. MV include `MV?` → bind `MV_`.
  4. Disabled filter `MATCH*` → no `LIKE` in SQL.
  5. Include `A\_B*`, exclude `OLD?`, plus empty masks → binds `A\_B%`, `OLD_`; empty masks ignored.
  6. Case-sensitive filter with includes `LITERAL\*NAME`, `LITERAL\?NAME*`, `ESCAPED\_NAME%`, exclude `LITERAL\?NAME_BAD*` → `matches("LITERAL*NAME")` true, `matches("LITERAL%NAME")` false, `matches("LITERAL?NAME_GOOD")` true, `matches("LITERAL?NAME_BAD1")` false.
  7. Include `foo*`, exclude `bar*`: case-insensitive → `UPPER(col) LIKE UPPER(?)` for include, `col LIKE ?` for exclude; case-sensitive → `col LIKE ?` for both.
  8. Our own: filter include `orders`, name `ORDERS` with case_sensitive=true → DBeaver returns true (literal compare ignores case) — Hacivat should return false.
- **Prior art and evidence:** `MODEL/struct/DBSObjectFilter.java`, `MODEL/sql/SQLUtils.java:411-492`,
  `JDBC/JDBCUtils.java:716-786`, registry `FilterMapping.java`, `TEST/.../OracleTableFilterTest.java`. DataGrip:
  "scope tree" per data source with patterns, `@` = current, `*` = all, negative entries = exclusion
  [report-connectivity §4]; scope controls *introspection* (what is loaded), not just display.
- **Hacivat recommendation:** Use **glob syntax only** (`*`, `?`, `[...]`, with `\` escaping) — do not treat `_` and
  `%` as wildcards — and a separate "regex:" prefix for power users. Apply case-sensitivity uniformly per dialect
  (identifiers' folding rules). Compile once to (a) a Rust matcher (`globset`) and (b) a dialect SQL predicate with
  binds and `ESCAPE`; always run the client matcher after pushdown. Model filters as DataGrip-style **introspection
  scope** (what is loaded) and keep a separate cheap **view filter** (what is displayed) so completion can still
  resolve objects outside the view filter. Key filters by stable object identifiers, not display names.

### ALG-DBV-META-05: Composite object assembly (indexes / constraints from per-column rows)

- **Problem:** Build multi-column objects (index on (a,b), FK (a,b)→(x,y)) from catalog rows, grouped by parent
  table and object name, robust to partial or inconsistent metadata.
- **Inputs / outputs:** Rows with `{parent_name, object_name, column ref(s), ordinal, …}` (MySQL/SQL Server/Generic:
  one row per column; PostgreSQL: one row per object with column-number arrays). Output: per-table list of objects
  with ordered column references.
- **Algorithm:**
  1. Ensure parents and all their columns are loaded (bulk).
  2. For each row: parent = for_parent or lookup(parent_name) (skip if unknown/empty); skip if parent already
     cached; object name = row value or `<PARENT>_#DBOBJ`; get-or-create `ObjectInfo` in a name-sorted map; fetch
     row refs (resolve column numbers/names to column objects); if none → mark object broken (unless allowed).
  3. On "feature not supported" errors: log and treat as empty.
  4. Publish per-parent lists (skipping broken objects in the global list), empty lists for all other parents.
  5. Attach rows sorted by ordinal; second pass for cross-object references (FK → referenced UK).
- **Data structures:** `Map<Parent, TreeMap<String, ObjectInfo{object, rows, broken}>>`.
- **Complexity / performance:** O(rows). PostgreSQL index expressions need an extra per-expression-column query
  (`pg_get_indexdef(oid, n)`), which can become N extra queries on expression-heavy schemas.
- **Edge cases:** missing referenced table for FK (dropped concurrently, filtered) → FK skipped with warning;
  FK column count ≠ referenced column count → truncated; expression index column (attnum 0); `INCLUDE` columns
  (SQL Server `is_included_column`, PG 11+); descending columns (PG `indoption` bit 0); partial index predicates;
  unnamed constraints; duplicate names across tables (key includes parent); constraints on views (excluded in PG
  navigator); invalid/disabled constraints and `NOT VALID`; deferrable FKs; primary key backed by index with a
  different name.
- **Test cases (input → expected):**
  1. MySQL STATISTICS rows `(t1, idx_ab, a, 1), (t1, idx_ab, b, 2)` → one index `idx_ab(a,b)` in order.
  2. PG `pg_constraint` row `conkey={2,1}` for PK → columns in array order (2nd attribute first).
  3. FK row whose `confrelid` table is filtered out → FK omitted, warning logged.
  4. Row with unknown column number → object marked broken and dropped.
  5. Bulk read returns nothing for table t3 → t3 gets empty list, no later query.
- **Prior art and evidence:** `JDBC/cache/JDBCCompositeCache.java:270-472`; `PostgreSchema.java:974-1146,1148-1285`;
  `MySQLCatalog.java:691-950`; `SQLServerSchema.java:515-830`.
- **Hacivat recommendation:** Prefer catalog queries that return **one row per object with arrays/aggregates**
  (PostgreSQL arrays, `string_agg`/`json_agg`; MySQL `JSON_ARRAYAGG` on 8.0; SQL Server `STRING_AGG` 2017+ or
  `FOR XML PATH`) to avoid client-side grouping bugs, and return index definitions via `pg_get_indexdef(oid)` in the
  *same* query (no per-column follow-ups). Represent column refs as attribute numbers + names, resolved against the
  snapshot after load.

### ALG-DBV-META-06: Dependency ordering for multi-table DDL

- **Problem:** Emit DDL for many tables so that every `REFERENCES` target exists before it is referenced; handle
  cycles.
- **Inputs / outputs:** Set of tables/views with their FK associations. Output: ordered lists `simple`, `cyclic`,
  `views`.
- **Algorithm (DBeaver):**
  1. Split views (emitted last, *not* dependency-sorted — a TODO in the code).
  2. `simple ← tables with no FKs`.
  3. Repeat until no progress: move a table to `simple` if every FK targets a table already in `simple` or itself
     (self-references allowed).
  4. Remaining tables are `cyclic` (or reference tables outside the input set, or unresolved references).
  5. Emit `simple` with full DDL; for `cyclic` tables that support splitting: first all `CREATE TABLE` without FKs,
     then all FK-only statements (when `separate_foreign_keys` is true, the default); others emit full DDL in input order.
- **Complexity / performance:** O(T² · F) in the worst case (repeated passes). Fine for hundreds; a Kahn topological
  sort would be O(T + F).
- **Edge cases:** FK to a table outside the selection → table is treated as cyclic (pessimistic) even though no
  cycle exists; self-referencing FK → simple; mutual FKs → cyclic; unresolved FK target (`null`) → cyclic; views on
  views → may be emitted in wrong order; materialized views depending on tables (PG) — classified as views?
  (depends on `isView`); partitions must follow parents; inherited tables must follow parents (PG `INHERITS`);
  sequences used by defaults must precede tables (PG schema DDL emits sequences first).
- **Test cases:** A→B, B→C, C → [C, B, A] simple; A↔B → both cyclic; split output: `CREATE A`, `CREATE B`,
  `ALTER A ADD FK`, `ALTER B ADD FK`; A→A only → simple; A→X where X not in input → cyclic (DBeaver) / simple with
  external dependency note (recommended).
- **Prior art and evidence:** `MODEL/struct/DBStructUtils.java:204-386`; PostgreSQL schema DDL order (types → sequences
  → tables → procedures → grants) `PostgreSchema.java:562-640`.
- **Hacivat recommendation:** Build a dependency graph over **all** object kinds (types, sequences, tables, views,
  functions, triggers, partitions, inheritance) using catalog dependency data (PG `pg_depend` / `pg_rewrite` for
  views; SQL Server `sys.sql_expression_dependencies`; MySQL view text parse as fallback); Tarjan SCC for cycles;
  break table cycles by deferring FKs; treat references outside the selection as already satisfied.

### ALG-DBV-META-07: Navigator child reconciliation on refresh

- **Problem:** After re-reading a container, update the tree without collapsing expanded subtrees or closing editors
  of unchanged objects.
- **Inputs / outputs:** Old child nodes, new object list. Output: new child node list reusing old nodes when the
  object is "the same".
- **Algorithm:** for each new object (after hidden/system/filter checks): find an old node with the same meta-type
  and `equal_objects(old.obj, new)` (same class and same unique name at every ancestor level) → retarget it
  (`reloadObject`), recursively reload its children if they were loaded, keep it; else create a new node. Then
  dispose old nodes of this meta-type not matched, except non-persisted ones.
- **Complexity:** O(old × new) per meta-type (linear scans over a linked list).
- **Edge cases:** rename (seen as remove+add — expanded state lost); case-only rename; type change; reordering
  (new order adopted); duplicates by name in the same folder (functions without unique names); filter changes
  (filtered-out nodes disposed); non-persisted nodes; disposed parent during a long load (checked twice); system
  object visibility toggles.
- **Test cases:** old [a(expanded), b], new [a, c] → a same node still expanded, b disposed, c new; new [A] for old
  [a] on case-sensitive DB → different object, a disposed.
- **Prior art:** `MODEL/navigator/DBNDatabaseNode.java:632-800,887-920`.
- **Hacivat recommendation:** UI tree state keyed by `ObjectKey` path (stable across snapshots); diff old vs new
  snapshot children with a hash map (O(n)); expose a rename hint when the server provides a stable id (PG OID,
  SQL Server object_id) so renames keep expansion state.

### ALG-DBV-META-08: DDL-statement → refresh target classification

- **Problem:** After executing user SQL, decide which part of the cached catalog is stale.
- **Inputs / outputs:** Parsed statement, dialect, current default catalog/schema. Output: set of targets.
- **Algorithm:** recognize `CREATE|DROP|ALTER|RENAME` × object kind with qualified name parts (multi-statement
  scripts split first; `ALTER … RENAME TO` → RENAME). Map: object in schema → `Schema(cat?, schema)` using name
  parts or current defaults; `CREATE/DROP SCHEMA` → `Catalog` or data source; `CREATE/DROP DATABASE` → data source;
  unknown → data source. Fall back to `query.type == DDL` with no recognized operation → data-source-level
  notification.
- **Edge cases:** `DROP TABLE a, b, c` (multiple targets); `ALTER TABLE x RENAME TO y`; `CREATE TABLE … AS SELECT`;
  `SELECT INTO` (SQL Server creates a table); temp tables (should not refresh); `DO $$ … $$` blocks / procedures
  executing dynamic DDL (unrecognizable); `USE db` / `SET search_path` (changes defaults, not catalog); `TRUNCATE`
  (no metadata change); `GRANT/REVOKE` (privileges only); `COMMENT ON` (descriptions); quoted identifiers; schema
  qualifier with database prefix (`db.schema.table`); MySQL `RENAME TABLE a TO b, c TO d`.
- **Test cases:** `create table s1.t(x int)` → Schema(null,"s1"); `drop schema s2` → Catalog/DataSource; `alter
  table t rename to u` with default schema `public` → Schema(null,"public"); `create database d` → DataSource;
  `select 1` → none.
- **Prior art:** `SQLObjectOperationRecognizer.java`, `SQLMetadataRefreshTargetResolver.java`,
  `SQLMetadataRefreshTransactionCoordinator.java`. DataGrip: RefreshAffectedObjects introspection task.
- **Hacivat recommendation:** Classify with the same parser used for execution (sqlparser-rs / own parser), produce
  **object-level** targets (`Table(schema, name)`), and re-introspect only those objects (plus their schema's name
  list for CREATE/DROP/RENAME). Treat `COMMENT ON` and `GRANT` as targeted property refreshes.

### ALG-DBV-META-09: Edit command queue merging (UI edits → minimal DDL)

- **Problem:** Turn a sequence of UI edits into the minimal correct DDL batch with undo support.
- **Algorithm:** group commands per object (identity); for each queue, fold commands left-to-right with
  `merge(prev)`: returns self (keep both), prev (absorbed), other (composite) or null (cancel both); manager-level
  filter; a pending CREATE of a new table aggregates all nested commands for children of that table; rename
  commands moved to the head; rename chains collapsed; renames of created objects dropped. Validate; execute per
  command with per-command commit; `updateModel` after success.
- **Edge cases:** create then delete same object (no SQL); rename then rename back (no-op? DBeaver emits rename
  from first-old to last-new which may equal → should be suppressed); change type then rename column (rename must
  run first); add column then add index on it (order by child types); failure in the middle with non-transactional
  DDL (partial application — DBeaver commits per command so earlier commands stay applied); concurrent server
  change between load and save (no optimistic check).
- **Test cases:** rename a→b then b→c → single `RENAME a TO c`; create t + add column x + add PK → one `CREATE
  TABLE t (x …, CONSTRAINT … PRIMARY KEY (x))`; create t then delete t → no statements.
- **Prior art:** `AbstractCommandContext.java:85-305,593-725`, `SQLObjectEditor.java:429-610`, `SQLStructEditor.java`.
- **Hacivat recommendation:** Represent edits as a *desired model* rather than a command log: diff `original
  snapshot → edited model` with the same diff engine used for schema compare, generate ordered DDL from the diff,
  show a preview, execute in one transaction where DDL is transactional (PostgreSQL, SQL Server, SQLite), and check
  a pre-save watermark to detect concurrent changes.

---

## 5. Implications for Hacivat / Karagöz / Perde

### 5.1 Hacivat (desktop client)

1. **Crate layout:** `catalog-model` (pure types, snapshot, `ObjectKey`, diff), `catalog-introspect-{pg,mysql,mssql,sqlite,…}`
   (async loaders, SQL in `include_str!` files with named sections like DataGrip's `---- RetrieveTables ----`),
   `catalog-cache` (persistence, watermarks, single-flight), `catalog-ddl` (IR + per-dialect renderers),
   `catalog-diff` (compare + migration).
2. **Snapshot architecture.** `CatalogSnapshot { containers: Map<ObjectKey, ContainerState>, objects: Arena<Obj> }`,
   published via `ArcSwap`. Loading never mutates a published snapshot. This removes DBeaver's `synchronized`
   loader contention and its reflection-based `deepCopyCachedObject`.
3. **Load-state machine per (container, cache kind):** `NotLoaded → Loading(gen) → Loaded{at, watermark} | Failed`.
   Expose state to the UI (spinner, "stale since…", retry) and to the MCP server (freshness metadata in replies).
4. **Bulk policy owned by the loader** (ALG-02 recommendation), with parallel independent queries over a small
   metadata pool (2–3 connections), single-flight dedupe, and cancellation that never publishes partial state.
5. **Separate metadata connection** by default, auto-commit, read-only where possible (PG
   `SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY`), application name `Hacivat - Metadata`; single
   connection for SQLite/DuckDB; temp-table metadata from the query session.
6. **"Cache-only" read mode** like `LocalCacheProgressMonitor`: completion and hover use `CatalogView::cached()`
   which never triggers I/O but can *enqueue* a background load for missing parts (DBeaver does not enqueue; it
   just shows nothing).
7. **Persistent cache + incremental refresh** (DataGrip advantage, DBeaver #42474): store per-connection
   snapshot in SQLite with format version; on connect, show cached immediately, validate in background using
   watermarks; per-schema `last_validated_at`.
8. **DDL-aware refresh** at object granularity, transaction-aware (copy DBeaver's commit/rollback/savepoint
   handling).
9. **Filters** as introspection scope + view filter (ALG-04 recommendation), glob syntax, stable keys.
10. **DDL** via typed IR + renderers + golden tests; prefer server functions where exact; unify edit-save, DDL view,
    schema compare and cross-database copy on one diff/IR pipeline (DataGrip's approach, which DBeaver CE lacks).
11. **Lazy-property convention:** in Rust, separate *structural* fields (always loaded with the object) from
    *expensive* facets (`stats`, `source_text`, `privileges`) stored in side tables with their own load state —
    the equivalent of DBeaver's "getter takes a monitor ⇒ lazy" rule, but explicit in types (`Facet<T>`).
12. **Scale targets** (from #42465): 15k tables / 160k columns per schema must give completion in < 1 s warm (from
    persisted cache) and < 10 s cold over 100 ms RTT; navigator folders must render progressively (stream names
    first, details later).

### 5.2 Karagöz (agent next to the database)

- Run introspection **on the agent** (≈0 ms RTT to the DB) and ship *compressed snapshot deltas* to Hacivat over the
  P2P channel. A 160k-column catalog compresses to a few MB; a delta after DDL is tiny. This makes Karagöz-backed
  connections faster than direct ones on high-latency links.
- **Metadata is an authorization surface.** Apply RBAC at the agent: a user allowed only schema `sales` must not
  receive names of other schemas (DBeaver's filters are a UX feature, not security). Implement as a mandatory
  server-side scope that intersects with the user's view filter, enforced in SQL pushdown *and* in a post-filter.
- Record audit events for metadata reads at a coarse level (connect, full introspection, DDL view of sensitive
  objects) — not per keystroke.
- Watermark checks (cheap queries) can run periodically on the agent and push "schema changed" notifications to
  connected clients — solving DBeaver's #6398 complaint without polling from every desktop.

### 5.3 Perde (control plane)

- Never store full catalogs by default (they reveal business structure). Optionally store a **hashed schema
  fingerprint** per discovered database to show "schema changed since last week" in the console and to drive
  drift alerts.
- If teams want shared annotations (comments, virtual foreign keys, data classification tags), store them keyed by
  `ObjectKey`, separate from the catalog itself.

### 5.4 MCP server (inside Hacivat)

- Tools `list_schemas`, `list_tables(schema, pattern)`, `describe_table`, `get_ddl(object, options)` answer from the
  snapshot with `freshness: {loaded_at, watermark, stale: bool}`; a `refresh_metadata(target)` tool that respects
  the same single-flight and RBAC rules. DBeaver's `ddl.objectFilter` option (used to bound schema DDL sent to an
  LLM) is a useful precedent: let the MCP layer cap DDL size and filter objects before returning.

---

## 6. i18n considerations

- **Navigator labels are resources, not code.** DBeaver declares folder labels as `%tree.tables.node.name` in
  `plugin.xml` resolved from `OSGI-INF/l10n/bundle*.properties`; PostgreSQL ships 15 locales (ar, de, es, fr, it,
  ja, ko, pl, pt_BR, ro, ru, tw, uk, zh, default) and **there is no Turkish `_tr` bundle anywhere in the repo**
  [Observed: `plugins/org.jkiss.dbeaver.ext.postgresql/OSGI-INF/l10n/`; `find … -name '*_tr.properties'` → 0].
  Hacivat: folder names ("Tables", "Columns", "Indexes"…) and object-type names must be message keys (Fluent),
  identical across dialects where the concept matches, overridable per dialect where terms differ ("Database" vs
  "Catalog").
- **Generated SQL is never localized.** DBeaver mixes this: DDL comment lines like `-- Drop table`,
  `-- Column comments` are English literals inside the generator, while action *titles* come from
  `ModelMessages` (localized). Hacivat: generated SQL text (including comments inside it) is English and stable
  (tests depend on it); titles shown in UI (progress, history) are localized separately.
- **Locale-sensitive case folding (Turkish hazard).** `AbstractObjectCache` upper-cases cache keys with the JVM
  default locale (`name.toUpperCase()` at `AbstractObjectCache.java:93,307`) but uses `Locale.ENGLISH` in
  `renameObject` (lines 141-142); `JDBCCompositeCache.getDefaultObjectName` also uses the default locale (line 471).
  On a Turkish-locale JVM `"items".toUpperCase()` is `"İTEMS"` (dotted capital I) while the English form is
  `"ITEMS"`, so a rename in a case-insensitive cache can fail to find the old key [Inferred from Java semantics;
  not reproduced]. Hacivat: never use locale-aware case mapping for identifiers. Use per-dialect folding rules
  (PostgreSQL folds unquoted identifiers to lower case for ASCII letters; SQL Server compares by collation;
  MySQL table-name case depends on `lower_case_table_names`), implemented with ASCII-only or explicit Unicode
  simple case folding — Rust's `to_uppercase`/`to_lowercase` are locale-independent, which is good, but also
  differ from what a Turkish-collation server does for `i/I`.
- **Filter masks and Turkish names:** a case-insensitive filter `*fiş*` must match `FİŞ` when the server's collation
  is Turkish, but `FIŞ` (dotless) under a default collation. Document that client-side matching uses Unicode
  simple folding and may differ from the server; push filters down where possible so the server's collation
  decides.
- **Sorting** navigator children: DBeaver uses a name comparator (optionally case-insensitive,
  `NAVIGATOR_SORT_*` preferences). Hacivat should sort with ICU4X collation for the UI locale (so Turkish users see
  `ç` after `c`, `ı` before `i`), but never use that order for anything persisted or for DDL output order.
- **Object comments** (PostgreSQL `COMMENT ON`, SQL Server `MS_Description`) are user data in any language; store
  and render as UTF-8, escape for SQL only when generating DDL, and keep inline `-- comment` rendering one-line
  (newlines in comments must be flattened or emitted only in `COMMENT ON`).

---

## 7. Risks, pitfalls, open questions

### 7.1 Lessons from DBeaver's weak points

| Pitfall | Evidence | Avoid in Hacivat by |
|---|---|---|
| Full re-read on every session; no persistence | #42474 (open), #42465 (Oracle 14k tables: completion > 60 s, Indexes ~30 s), #39295 | Persistent snapshot + background validation |
| N+1 query storms when per-object APIs are called in loops | #42468 (PG indexes → one constraint query per table, fixed by prefetch), per-expression `pg_get_indexdef` calls | Loader-owned bulk policy; aggregate queries |
| Slow catalog queries missing a schema predicate | #39169 (MySQL relations query) | Always constrain by schema; `EXPLAIN` our catalog queries in CI against large fixtures |
| UI freezes when metadata is read on the UI thread / under locks | #21928 (freeze expanding Indexes, MSSQL), `synchronized` loaders | Never block UI; immutable snapshots; per-request timeouts |
| No change detection | #6398 | Watermarks; agent push notifications |
| Inconsistent filter pushdown across drivers | §3.6.3 (MySQL exclude not translated; SQL Server include+exclude composition) | One filter compiler shared by all dialects, tested per dialect |
| `_` as a filter wildcard surprises users | `SQLUtils.isLikePattern` | Glob syntax |
| Filter in navigator also hides objects from completion | OracleTableFilterTest | Separate scope vs view filter |
| Heuristic default-value quoting | `SQLTableColumnManager.BaseDefaultModifier` | Store raw default expressions |
| DDL generation silently falls back to "Can't generate DDL" comment when no manager is registered | `DBStructUtils.java:136-137` | Compile-time exhaustive match over dialect × object kind |
| Per-command commit during save leaves partial changes on error | `AbstractCommandContext.java:233-240` | Single transaction when DDL is transactional; preview + explicit warning otherwise |
| Locale-dependent upper-casing of keys | §6 | Dialect folding functions |

### 7.2 Open questions

- [Unknown] How DBeaver's commercial editions (Enterprise/Ultimate) persist or diff metadata — not in the CE repo.
  The CE schema compare is report-only.
- [Unknown] Whether #42474 (persistent metadata for all databases) will land and in what format; worth re-checking
  before finalizing Hacivat's cache format, as the design (store raw rows, replay through fetch functions) is
  interesting: it makes the cache robust to model-code changes because the *rows*, not the objects, are stored.
- [Unknown] Exact default of `SHOW_METADATA_REFRESH_NOTIFICATION` after the recent change (the parser maps legacy
  `true/false` to ASK/NEVER); not checked in the preference initializer.
- [Inferred, needs runtime check] The SQL Server table-filter SQL composition when both include and exclude masks
  are present (§3.6.3).
- Open design question for Hacivat: should the metadata snapshot be shared between Hacivat instances via Karagöz
  (one introspection per database per agent, many readers)? Pros: load reduction on production databases. Cons:
  RBAC must be applied per reader, so the agent stores the full snapshot and filters per user.
- Open design question: how much of PostgreSQL's dependency graph (`pg_depend`) to load by default — it is large
  on big databases; load lazily for DDL ordering and "show dependencies" only.
