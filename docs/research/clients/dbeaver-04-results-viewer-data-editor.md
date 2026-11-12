# DBeaver Community: Results Viewer and Data Editor (source analysis)

> Part 4 of 5 of the DBeaver deep-dive. Scope: how DBeaver fetches, displays, formats, filters, edits and saves tabular data ("the grid"). Companion documents: `dbeaver-01` (architecture, drivers, connections), `dbeaver-02` (metadata, DDL), `dbeaver-03` (SQL editor, parsing, completion), `dbeaver-05` (data transfer, localization, lessons).

---

## 1. Scope and sources

| Item | Value |
|---|---|
| Project | DBeaver Community Edition |
| Repository | https://github.com/dbeaver/dbeaver |
| Commit studied | `f78ac453859084a4654eac0599fb4c0514ef405b` (2026-11-11, branch `devel`, shallow clone) |
| Local path | `.refs/repos/dbeaver` (read only, nothing built or executed) |
| License | Apache License 2.0 (every Java file carries the Apache-2.0 header). Code reuse is legally possible with attribution and NOTICE handling; this document still describes algorithms in prose and pseudocode. |
| Comparison baseline | DataGrip 2024.3.5 grid report (`.refs/datagrip-extract/report-grid.md`, Turkish notes, behaviour-level teardown without decompilation). DataGrip is proprietary: only behaviour is compared, nothing is copied. |

Plugins (OSGi bundles) read for this document. All paths below are relative to `.refs/repos/dbeaver/plugins/` unless they start with `test/`.

| Bundle | What lives there |
|---|---|
| `org.jkiss.dbeaver.model` | UI-free data model: `DBDAttributeBinding*`, `DBDRowIdentifier`, `DBDDataFilter`, `DBDValueHandler`, `DBDContent`, `DBExecUtils`, `DBUtils`, virtual model `DBV*`, batch executors (`ExecuteBatchImpl`, `ExecuteInsertBatchImpl`), data updater (`DBDResultSetDataUpdater`), SQL filter rewriting (`SQLSemanticProcessor`, `StandardSQLDialectQueryGenerator`), formatters. |
| `org.jkiss.dbeaver.model.jdbc` | JDBC implementation: `JDBCTable` (SELECT/INSERT/UPDATE/DELETE generation), `JDBCStatementImpl` (limit/offset emulation), value handlers for JDBC types, LOB wrappers. |
| `org.jkiss.dbeaver.model.sql` | `SQLQueryDataContainer`, `SQLQueryTransformerCount`. |
| `org.jkiss.dbeaver.ui.editors.data` | The viewer itself: `ResultSetViewer` (5,761 lines), `ResultSetModel`, `ResultSetRow`, `ResultSetPersister`, `ResultSetDataReceiver`, `ResultSetJobDataRead`, spreadsheet/plaintext presentations, `LightGrid` custom canvas grid (5,009 lines), panels, value editors. |
| `org.jkiss.dbeaver.ui.editors.json/.xml/.image/.hex` | Content viewers registered through the `org.jkiss.dbeaver.dataManager` extension point. |
| `org.jkiss.dbeaver.ext.postgresql` (+ `.ui`) | PostgreSQL-specific value handlers and value managers. |
| `org.jkiss.dbeaver.ext.sqlite`, `.oracle`, `.db2`, `.exasol` | Pseudo-column (ROWID) declarations. |

Tests consulted (under `test/`): `org.jkiss.dbeaver.test.platform/.../model/data/resultset/DBDResultSetDataUpdaterTest.java`, `.../model/data/DBDReferenceUtilsTest.java`, `.../model/impl/data/formatters/NumberDataFormatterTest.java`, `.../model/impl/data/transformers/EpochTimeAttributeTransformerTest.java`, `.../model/impl/jdbc/data/handlers/JDBCDateTimeValueHandlerTest.java`, `org.jkiss.dbeaver.model.sql.test/.../SQLSemanticProcessorLineCommentsTest.java`, `org.jkiss.dbeaver.ext.postgresql.test/.../PostgreValueParserTest.java`.

Evidence tags follow the library README: **[Observed: path:lines]**, **[Inferred]** (with basis), **[Unknown]**.

---

## 2. Executive summary

1. **Two-layer design.** Everything that touches the database (read, filter, generate DML, detect keys) lives in UI-free model bundles; the SWT viewer only holds an in-memory row list and dispatches jobs. Hacivat should copy this split: a `grid-core` crate with no UI dependency. [Observed: `AGENTS-Architecture.md`; bundle layout]
2. **A row is an `Object[]` plus a sparse "old values" map.** `ResultSetRow` keeps the current values array, a state byte (NORMAL / ADDED / REMOVED) and a map `column → original value` filled the first time a cell is edited. Pending changes are therefore an overlay keyed by row object, not a separate store. [Observed: `ui.editors.data/.../resultset/ResultSetRow.java:30-170`]
3. **Every column is a "binding".** `DBDAttributeBinding` joins three things: the driver's result-set column metadata, the real table column (if one could be resolved) and the value handler that reads/writes/formats the value. Nested bindings model struct fields, array elements and document keys. [Observed: `model/.../data/DBDAttributeBinding.java:41-392`]
4. **Default page ("segment") size is 200 rows.** "Fetch next page" re-runs the query. By default (`resultset.reread.on.scroll = true`) it re-reads *from row 0* up to `old + 200` rows and replaces the whole model, which keeps results consistent but makes scrolling cost grow quadratically. [Observed: `model/.../ModelPreferences.java:357-358`; `ResultSetJobDataRead.java:101-106`; `ResultSetDataReceiver.java:178-186`]
5. **Offsets are usually emulated on the client.** Unless the user enables "use SQL to limit fetch size" (`resultset.maxrows.sql`, default false), DBeaver calls JDBC `setMaxRows(offset + limit)` and then skips `offset` rows with `absolute()` or repeated `next()`. [Observed: `model/.../DBUtils.java:1659-1733`; `model.jdbc/.../exec/JDBCStatementImpl.java:238-275`; `model.jdbc/.../JDBCUtils.java:513-534`]
6. **"Has more data" is a heuristic:** `rows_received >= page_size`. There is no look-ahead row, so an exact multiple of 200 shows a phantom "more" that yields an empty fetch. [Observed: `ResultSetDataReceiver.java:189`]
7. **Row identity order:** primary-key index → unique index whose columns are all NOT NULL → unique/PK constraint (nullable columns only if the DB allows nullable unique) → pseudo ROWID column (Oracle `ROWID`, SQLite `rowid`, PostgreSQL `oid` only on legacy `WITH OIDS` tables; **no `ctid`**) → user-defined "virtual unique key" → "use all columns" (opt-in, warns about multi-row updates). Views skip physical keys entirely. [Observed: `model/.../exec/DBExecUtils.java:584-712`; `model/.../DBUtils.java:1437-1484`; `ext.postgresql/.../PostgreConstants.java:99-107`]
8. **Save order is DELETE → INSERT → UPDATE**, one statement per row (JDBC batch only when many rows share an identical NULL pattern), inside a savepoint when auto-commit is off; the first failure stops and rolls back to the savepoint. [Observed: `model/.../data/resultset/DBDResultSetDataUpdater.java:369-475`; `model/.../impl/data/ExecuteBatchImpl.java:97-247`]
9. **WHERE uses only key columns, never old values** (no optimistic concurrency). A row that changed on the server since it was read is silently overwritten. DBeaver warns only when an UPDATE/DELETE touched **zero** rows; it does not detect **more than one** row being touched. [Observed: `model.jdbc/.../struct/JDBCTable.java:461-521, 1065-1098`; `ResultSetPersister.java:576-616`]
10. **INSERT omits NULL columns** (so server defaults apply) unless *all* values are NULL, in which case it emits `INSERT … DEFAULT VALUES` or `VALUES(DEFAULT, …)`. Consequence: a user who explicitly types NULL into a column that has a DEFAULT gets the default, not NULL. [Observed: `model/.../impl/data/ExecuteInsertBatchImpl.java:72-77, 119-200`]
11. **After save, rows are re-read by key** (`RS_EDIT_REFRESH_AFTER_UPDATE`, default true) so triggers, defaults and generated columns appear; auto-increment keys are taken from JDBC `getGeneratedKeys`. [Observed: `ResultSetPersister.java:180-223, 628-694, 714-827`; `ResultSetViewer.java:4771-4815`]
12. **Server-side filter on a SQL query result** is applied by parsing the query with JSQLParser (1-second timeout) and injecting `WHERE (orig) AND (filter)` / extra `ORDER BY` items; if parsing fails or the filter references a column that is not a real table column, the query is wrapped: `SELECT * FROM (<query>) z_q WHERE … ORDER BY …`. [Observed: `model/.../sql/parser/SQLSemanticProcessor.java:64-125, 211-311`; `model/.../impl/sql/StandardSQLDialectQueryGenerator.java:192-232`]
13. **Ordering strategy "SMART"** (default): clicking a header sorts locally if the whole result is already loaded, otherwise re-queries with `ORDER BY`. Local sorting discards pending edits. [Observed: `ResultSetViewer.java:2704-2778`]
14. **Values are typed by pluggable "value handlers"** chosen per data source and type through the `org.jkiss.dbeaver.dataTypeProvider` extension point; PostgreSQL adds handlers for json/jsonb, hstore, bit/varbit, interval, money, geometry, arrays, composites, refcursor. Fetch errors become a per-cell `DBDValueError` instead of failing the whole page. [Observed: `model.jdbc/.../JDBCStandardValueHandlerProvider.java`; `ext.postgresql/.../PostgreValueHandlerProvider.java`; `ResultSetDataReceiver.java:118-154`]
15. **LOBs** are wrapped in `DBDContent`; a CLOB smaller than 10,000 characters (`content.memory.maxsize`) is pulled into memory, larger ones are streamed to a temporary file. Content viewers (text, JSON, XML, HTML, hex/binary, image) are chosen by MIME match level. [Observed: `model.jdbc/.../JDBCContentCLOB.java:88-136`; `ui.editors.data/.../ContentPanelEditor.java:300-335, 429-433`]
16. **A "virtual model"** stored per connection (`virtual-models` in `data-sources.json`) holds user-defined unique keys, foreign keys, computed columns (JEXL expressions), colour rules and value transformers. It is how DBeaver makes views and key-less tables editable and navigable. [Observed: `model/.../virtual/DBVEntity.java`, `DBVModelSerializerModern.java:49-210`; `registry/.../DataSourceSerializerModern.java:138-175`]
17. **Copy/paste is TSV-centric and has escaping gaps:** advanced copy wraps a cell in quotes when it contains the delimiter but never doubles embedded quotes; paste parsing does not un-double `""` and drops a trailing empty cell. [Observed: `SpreadsheetPresentation.java:530-677, 876-944`]
18. **FK navigation** builds a filter on the target table: `col = v` for one row, `col IN (…)` for several rows, and a disjunctive normal form `(a=1 AND b=10) OR (a=2 AND b=20)` for composite keys over several rows. This is the one part with a dedicated unit test. [Observed: `model/.../data/DBDReferenceUtils.java:45-191`; `test/.../DBDReferenceUtilsTest.java:37-75`]
19. **Compared with DataGrip:** DataGrip uses explicit page navigation with LIMIT/OFFSET, an `UnparsedValue` state for bad input, a DML preview and re-select by key; neither tool uses optimistic concurrency. DBeaver goes further on *making things editable* (virtual keys, ROWID) and on *transform/colour* customisation; DataGrip is stricter (read-only without PK/unique).
20. **Recommendation for Hacivat:** keyset-or-cursor paging with a look-ahead row, protocol-level column origin (PG `RowDescription` table OID/attnum, MySQL `org_table`), strict key selection with opt-in fallbacks, parameterised DML with `rows_affected == 1` enforcement inside one transaction, optional old-value (optimistic) WHERE, and a real CSV/TSV codec for clipboard.

---

## 3. Detailed findings

### 3.1 Glossary (for readers who are not database specialists)

| Term | Meaning in this document |
|---|---|
| Result set | The table-shaped answer a database returns for a query: a list of columns (with types) and a stream of rows. |
| Fetch / fetch size | Reading rows from the server. *Fetch size* is a hint telling the driver how many rows to transfer per network round trip. It does not limit the total. |
| Max rows / limit / page / segment | The number of rows the client asks for at once. DBeaver calls a page a *segment*. |
| Offset | How many leading rows to skip. |
| Keyset paging | Paging by "give me rows after the last key I saw" (`WHERE id > 1234 ORDER BY id LIMIT 200`) instead of OFFSET; stable and fast on large tables. |
| Primary key (PK) | Column(s) whose values uniquely identify each row; never NULL. |
| Unique key / unique index | Like a PK but may allow NULLs in some databases (and NULLs do not collide, so a nullable unique key does **not** identify a row). |
| ROWID / ctid / rowid | A physical address of a row maintained by the engine (Oracle `ROWID`, SQLite `rowid`, PostgreSQL `ctid`). Changes under some operations (PostgreSQL `ctid` changes on every UPDATE and after VACUUM FULL). |
| DML | Data manipulation statements: INSERT, UPDATE, DELETE. |
| Auto-commit | Each statement is its own transaction and is permanent immediately. With auto-commit off, changes wait for COMMIT or ROLLBACK. |
| Savepoint | A bookmark inside a transaction you can roll back to without abandoning the whole transaction. |
| LOB / BLOB / CLOB | Large object (binary / character). Drivers often return a handle that must be read separately. |
| Value handler | DBeaver's per-type adapter that reads a value from the driver, binds it to a statement, converts user input and formats for display. |
| Binding | DBeaver's object describing one column of a result set and how it maps to a table column. |
| Virtual model | User-defined metadata (keys, FKs, colours, computed columns) DBeaver stores locally because the database does not have it. |
| Presentation | A way of rendering the same result set: grid ("spreadsheet") or fixed-width text. |
| Record mode | Transposed view: one row shown as a vertical list of column/value pairs. |
| Panel | A side view attached to the grid (value viewer, aggregates, metadata, grouping, references). |

### 3.2 Layering and the main objects

```
ResultSetViewer (SWT control, 5.7k lines)          ui.editors.data
 ├─ ResultSetModel        rows + bindings + filter + change counter
 │   └─ ResultSetRow[]    values Object[] + state + changes map
 ├─ IResultSetPresentation  (spreadsheet | plaintext | contributed, e.g. GIS)
 │   └─ LightGrid (custom Canvas, virtual rendering)
 ├─ Panels (value viewer, aggregates, metadata, grouping, references)
 ├─ ResultSetDataReceiver  (DBDDataReceiver: fetchStart/fetchRow/fetchEnd)
 ├─ ResultSetJobDataRead   (background read job, one at a time)
 └─ ResultSetPersister     (save pipeline; extends model-level updater)

DBSDataContainer.readData(source, session, receiver, filter, offset, maxRows, flags, fetchSize)
 ├─ JDBCTable            → builds SELECT … FROM t [alias] WHERE … ORDER BY …
 └─ SQLQueryDataContainer / SQLQueryJob → user SQL + filter injection
DBSDataManipulator.insertData/updateData/deleteData → ExecuteBatch → JDBC
DBExecUtils.bindAttributes → resolves table columns + row identifiers
```

[Observed: `ui.editors.data/.../resultset/ResultSetViewer.java`, `ResultSetModel.java:49-110`, `ResultSetDataReceiver.java:42-233`, `ResultSetJobDataRead.java:60-160`, `ResultSetPersister.java:56-140`; `model.jdbc/.../struct/JDBCTable.java:126-620`; `model/.../exec/DBExecUtils.java:759-1029`]

The presentation layer is pluggable through the extension point `org.jkiss.dbeaver.resultset.presentation`; the Community edition ships *spreadsheet* (order 100, supports record mode and hints) and *plaintext* (order 200), and the GIS bundle contributes a map presentation. Panels come from `org.jkiss.dbeaver.resultset.panel`: `value-view` (default on), `column-aggregate`, `results-metadata`, `results-grouping`, `results-references`. [Observed: `ui.editors.data/plugin.xml:101-170`; `data.gis.view/plugin.xml:82`]

### 3.3 The in-memory model

#### 3.3.1 `ResultSetRow`
- Fields: `rowNumber` (physical index in fetch order), `visualNumber` (index in the current view after local filtering/sorting), `Object[] values`, `byte state` (`STATE_NORMAL=1`, `STATE_ADDED=2`, `STATE_REMOVED=3`), `Map<DBDAttributeBinding,Object> changes`, optional `ColorInfo` (row/cell foreground/background). [Observed: `ResultSetRow.java:30-75`]
- `changes` maps a **top-level** column to its *original* value, recorded on the first edit. For nested edits (a field inside a struct, an element inside an array) it additionally maps the *nested* binding to its *top* binding as a back-reference so the persister knows which root value to write. `clearChange(attr)` removes the entry and every back-reference to it. [Observed: `ResultSetRow.java:105-140`]
- Equality is by `(rowNumber, visualNumber)`, not by content. [Observed: `ResultSetRow.java:158-168`]
- `release()` frees driver resources of every current and original value (LOB handles). [Observed: `ResultSetRow.java:142-150`]

#### 3.3.2 `ResultSetModel`
- Holds `attributes` (all bindings), `visibleAttributes` (sorted by the filter's visual position), the optional `documentAttribute` (when the result is a single document/LOB column — used by NoSQL-style sources), the `DBDDataFilter`, `singleSourceEntity` (the one table all columns come from, if any), `curRows`, optional `filteredRows` for the quick filter, `totalRowCount`, `changesCount`, `updateInProgress`. [Observed: `ResultSetModel.java:49-110`]
- **Read-only status** is computed lazily: "Update in progress" → container-level reasons (not connected, connection read-only, data source read-only, no edit permission) → "No unique key defined" only when `resultset.edit.disableIfKeyMissing` is true (default false). [Observed: `ResultSetModel.java:152-180`; `DBExecUtils.java:1089-1116`]
- **Metadata change detection** (`updateMetaData`): metadata is "changed" if column count differs, a column's meta attribute appears/disappears, or any of label, name, entity, ordinal position, required flag, max length, precision, scale, type id or type name differs. For "dynamic metadata" sources (document stores) a same-source check keeps the state. Unchanged metadata keeps the user's column order, hidden columns and filters across re-execution. [Observed: `ResultSetModel.java:728-820`; `DBExecUtils.java:726-739`]
- **Document flattening:** if the only column is a DOCUMENT and `resultset.transform.complex.type` is true (default), its nested bindings become the visible top-level columns. [Observed: `ResultSetModel.java:855-899`; `ModelPreferences.java:353`]
- **Change counting:** an ADDED or REMOVED row counts once; a NORMAL row counts its number of changed columns. [Observed: `ResultSetModel.java:224-235`]
- **Release in background:** releasing old rows (and their LOB handles) runs in a separate task with a 5-second wait, because freeing LOBs can block. [Observed: `ResultSetModel.java:1107-1117`]

#### 3.3.3 `DBDAttributeBinding` hierarchy

| Class | Role |
|---|---|
| `DBDAttributeBinding` (abstract) | Common API: name/label, ordinal position, meta attribute (driver metadata), entity attribute (resolved table column, may be null), value handler, value renderer, nested bindings, row identifier, referrers (FKs), `lateBinding()` (runs attribute transformers). [Observed: `model/.../data/DBDAttributeBinding.java:41-392`] |
| `DBDAttributeBindingMeta` | A real top-level column of the result set. Carries the pseudo-attribute marker (ROWID) and the row identifier. |
| `DBDAttributeBindingNested` (abstract) → `DBDAttributeBindingType` | Field of a struct/composite/document. |
| `DBDAttributeBindingNested` → `DBDAttributeBindingElement` | Element *i* of an array/collection. |
| `DBDAttributeBindingCustom` | A virtual (computed) column defined in the virtual model; never written back. [Observed: `DBDResultSetDataUpdater.java:574-576` — `isVirtualColumn` skips them in INSERT/UPDATE] |

Transformers attached by `lateBinding` (registered in `model/plugin.xml:68-140`): `ComplexTypeAttributeTransformer` and `MapAttributeTransformer` (STRUCT/DOCUMENT → nested columns, on by default), `ArrayAttributeTransformer` (arrays → element columns, on by default), and opt-in per-column `EpochTimeAttributeTransformer` (units seconds…nanoseconds, .NET ticks, Win32 FILETIME, OLE date, SQLite Julian; optional zone), `RadixAttributeTransformer` (2/8/10/16/32, bits, prefix, unsigned), `URLAttributeTransformer`. GIS adds a geometry transformer. [Observed: `model/plugin.xml:68-140`; `data.gis/plugin.xml:12`]

### 3.4 The read pipeline

1. A UI action (open table, execute query, refresh, filter, sort, scroll) calls `runDataPump(container, filter, offset, maxRows, focusRow, saveHistory, scroll, refresh, finalizer)`. If no execution context matches the container (cross-database navigation), the container's default context is used. Auto-refresh timers are cancelled. [Observed: `ResultSetViewer.java:4593-4640`]
2. `queueDataPump` **clears the queue and enqueues only the newest job**; a tiny scheduler job polls every 50 ms until no job is running, then starts the head of the queue. Intermediate requests (e.g. rapid clicks in the references panel) are dropped. A data-read lock (`acquireDataReadLock`) prevents concurrent reads into one model. [Observed: `ResultSetViewer.java:4642-4710`]
3. `ResultSetJobDataRead.run` sets flags (`FLAG_READ_PSEUDO` always — ask for ROWID; `FLAG_FETCH_SEGMENT` when offset > 0; `FLAG_REFRESH`), applies the reread-on-scroll rewrite (offset → 0, maxRows += offset), opens a session tagged USER or USER_FILTERED and calls `dataContainer.readData(…)` inside `DBExecUtils.tryExecuteRecover` (reconnect-and-retry on recoverable errors). A UI-side visualiser shows progress; if a cancel request does not finish within `resultset.cancel.timeout` (default 5,000 ms) the job is detached so the UI is not blocked. [Observed: `ResultSetJobDataRead.java:89-200`; `DBExecUtils.java:209-306`]
4. For a table, `JDBCTable.readData` builds `SELECT <cols> FROM <qualified table> [AS] [alias] <WHERE …> <ORDER BY …>`. An alias is generated whenever there is a data filter or a ROWID request and the dialect supports aliases (needed for composite/structured columns in PostgreSQL/Oracle). If the dialect has `*` it selects `alias.*` plus the ROWID expression; otherwise it enumerates columns. If no ORDER BY was requested and the ordering strategy is SERVER_SIDE, a default order by the best key is added. Fetch size is set if requested. Rows are pulled until `maxRows` is reached or the monitor is cancelled. [Observed: `model.jdbc/.../struct/JDBCTable.java:141-262, 268-312`]
5. For SQL editor results, `SQLQueryJob` first substitutes parameters, then calls `dialect.addFiltersToQuery(…)` if a filter is active, then executes. [Observed: `ui.editors.sql/.../execute/SQLQueryJob.java:515-560`]
6. `ResultSetDataReceiver.fetchStart` (first segment only) builds bindings from driver metadata and pushes metadata into the model. `fetchRow` asks each binding's value handler for the cell; **any exception becomes a `DBDValueError` in that cell** and the error message is logged once per (column, error class). `fetchEnd` (first segment only) runs `DBExecUtils.bindAttributes` (table/key resolution, §3.8), then calls `setData` or `appendData`, computes `hasMoreData`, and refreshes the presentation on the UI thread; optionally it triggers an automatic row count. [Observed: `ResultSetDataReceiver.java:93-215`]

### 3.5 Paging, fetch size and counting

| Setting (key) | Default | Effect | Evidence |
|---|---|---|---|
| `resultset.maxrows` | 200 | Segment size; minimum 1 (`MIN_SEGMENT_SIZE`). Can be overridden per viewer (`segmentFetchSize`) or typed in the status bar. | `ModelPreferences.java:239,358`; `ResultSetViewer.java:4549-4563`; `ResultSetPreferences.java:25` |
| `resultset.reread.on.scroll` | true | Next segment re-reads from row 0 and replaces rows. | `ModelPreferences.java:238,357`; `ResultSetJobDataRead.java:101-106` |
| `resultset.maxrows.sql` | false | Use a dialect LIMIT transformer instead of JDBC `setMaxRows`. | `ModelPreferences.java:230,370`; `DBUtils.java:1684-1688` |
| `resultset.fetch.size` | false | Force fetch-size hint; when forced with no explicit value, 10,000 (`DEFAULT_READ_FETCH_SIZE`), and for a limited read the hint becomes `offset + maxRows`. | `DBExecUtils.java:77, 308-321` |
| `resultset.autofetch.next.segment` | true | Scrolling to the bottom triggers `readNextSegment`. | `ResultSetPreferences.java`, defaults list |
| `resultset.automatic.row.count` | false | Issue a COUNT after each first fetch. | `ResultSetDataReceiver.java:203-205` |
| `resultset.cancel.timeout` | 5000 ms | Detach a read job that ignores cancel. | `ResultSetJobDataRead.java:170-186` |

**Next page (`readNextSegment`)** is refused if a quick (local) filter is active, if the query is flagged as data-modifying-on-refresh and the user declines, if there is no more data, or if a previous next-page is still running. Otherwise it requests `offset = current row count`, `maxRows = segment`, without moving the cursor. [Observed: `ResultSetViewer.java:4426-4466`]

**Fetch all (`readAllData`)** asks for confirmation (`CONFIRM_RS_FETCH_ALL`), then reads from current row count with `maxRows = -1`. [Observed: `ResultSetViewer.java:4475-4510`]

**Refresh** preserves the user's depth: if the focused row is beyond the first page, it re-reads `ceil((visualRow+1)/segment) * segment` rows. [Observed: `ResultSetViewer.java:4341-4362`]

**How offset/limit reach the server** (`DBUtils.makeStatement`): only a parsed *plain SELECT* is limited (DML must not be limited because SQL Server's `setMaxRows` also caps update counts). With a LIMIT transformer the text becomes `… LIMIT off, len` / `… LIMIT len OFFSET off` / `… LIMIT off+len` depending on dialect, and is skipped if the query already contains ` LIMIT `, ` OFFSET `, ` INTO `, ` UPDATE `, ` PROCEDURE ` or ` FETCH ` (regex on the upper-cased text). Without a transformer `setLimit(offset, limit)` sets `setMaxRows(offset+limit)` and later skips `offset` rows (`absolute(offset)` if the driver supports scrolling, otherwise `next()` in a loop). [Observed: `DBUtils.java:1659-1733`; `model/.../impl/sql/QueryTransformerLimit.java:30-110`; `JDBCStatementImpl.java:238-275`; `JDBCUtils.java:513-534`]

**Row count** (`DBUtils.readRowCount`, button or automatic): for SQL results `SQLQueryTransformerCount` wraps `SELECT COUNT(*) FROM (<query without ORDER BY>) dbvrcnt`; if the dialect lacks subqueries or has a broken ambiguous-count, it rewrites the select list to `count(*)` (or `count(DISTINCT …)`), refusing GROUP BY / HAVING. Code comments cite issues #4652 (remove ORDER BY), #2541 (trim delimiters), #40210 (wrap unparsable DuckDB FROM-first / `TABLE` / CTE queries anyway). [Observed: `model.sql/.../transformers/SQLQueryTransformerCount.java:43-149`]

### 3.6 Value handlers and formatting

**Interface.** `DBDValueHandler` (extends `DBDValueRenderer`): `getValueObjectType`, `getValueContentType` (MIME), `fetchValueObject(session, resultSet, type, index)`, `bindValueObject(session, statement, type, index, value)`, `getValueFromObject(session, type, object, copy, validate)` (converts user input or pasted text), `createNewValueObject`, `releaseValueObject`, `getSupportedOperators` (which filter operators the column allows), `getComparator` (local sort), and from the renderer `getValueDisplayString(type, value, format)`. [Observed: `model/.../data/DBDValueHandler.java:28-146`]

**Selection.** `DataTypeProviderRegistry.getValueHandlerProvider` first tries non-global providers that declare the data source and type, then global ones. PostgreSQL's provider is declared for data sources `postgresql` and `greenplum`, all type names. [Observed: `registry/.../datatype/DataTypeProviderRegistry.java:131-165`; `ext.postgresql/plugin.xml:774-786`]

**Standard JDBC mapping** (by DBeaver "data kind"): BOOLEAN → boolean; STRING → string (or content/LOB for LONGVARCHAR when the long-as-LOB flag is on); NUMERIC → number (per-type formatter); DATETIME → date/time; BINARY/CONTENT → content handler (or UUID when the type name is `UUID`); ARRAY → array; STRUCT → struct; REFERENCE → reference; ROWID → object. Unknown kinds return null → default handler. [Observed: `model.jdbc/.../handlers/JDBCStandardValueHandlerProvider.java`]

**PostgreSQL overrides** (decision order): no type name → "unknown" handler (protects against half-compatible forks); `record` → struct; JDBC ARRAY → PG array; STRUCT named json/jsonb → JSON (OpenGauss quirk) else struct; DATE/TIME/TIMETZ → PG date-time; TIMESTAMP(TZ) → `java.time` handler when the server type supports it; by name: varbyte → content, json/jsonb → JSON (`PostgreContentJSON`), hstore, bit/varbit, refcursor (opens the cursor as a nested result), money, geometry/geography (Redshift variant), interval; serial types → number; `OTHER` or string kind → PG string; else standard. [Observed: `ext.postgresql/.../model/data/PostgreValueHandlerProvider.java`]

**Display formats.** `DBDDisplayFormat` has three modes: `UI` (pretty, locale formatted), `EDIT` (what an inline editor shows), `NATIVE` (valid SQL literal). Copy, export and "generate SQL" choose the mode. [Observed: `model/.../data/DBDDisplayFormat.java`] DBeaver's own tests pin the NATIVE contract for date/time: values are single-quoted exactly once, already-quoted fallbacks are preserved, embedded quotes escaped. [Observed: `test/.../JDBCDateTimeValueHandlerTest.java:69-121`]

**Data formatter profiles.** `DBDDataFormatterProfile` (global or per connection, with a `Locale`) creates formatters for type ids `number`, `date`, `time`, `timetz`, `timestamp`, `timestamptz`. Defaults: `yyyy-MM-dd`, `HH:mm:ss`, `HH:mm:ss Z`, `yyyy-MM-dd HH:mm:ss.SSS`, `yyyy-MM-dd HH:mm:ss.SSS Z`. Number options: grouping, group size, min/max integer and fraction digits (max fraction ≥ 10), use type scale, rounding mode, native special values (NaN/Infinity), scientific notation for small values with a pattern (default `0.###E0`) and exponent separator, exclude ID columns from grouping. `resultset.format.datetime.native` / `numeric.native` (default false) switch to the database's own text. [Observed: `model/.../data/DBDDataFormatterProfile.java:35-62`; `DBConstants.java:107-113`; `formatters/NumberFormatSample.java`; `ModelPreferences.java:217-218,350-351`]

Test-pinned number behaviour [Observed: `test/.../NumberDataFormatterTest.java:44-133`]:
- `1.5E-12` with scientific-small-values on → `"1.5E-12"`; negated → `"-1.5E-12"`; with the option off → `"0"`.
- `0` → `"0"`; `0.5` → `"0.5"`; `1234567890.012345` → `"1,234,567,890.012"` (US locale, default max fraction digits affect the output).
- A locale with comma decimal separator renders `"1,5E-12"`.
- Invalid patterns, patterns without exponent, and an exponent separator clashing with locale separators are rejected; a custom exponent separator round-trips (`"1.5e-12"` parses back to the same BigDecimal).

Epoch transformer tests [Observed: `test/.../EpochTimeAttributeTransformerTest.java:93-134`]: milliseconds/UTC `0 → "1970-01-01 00:00:00.000"`, `42 → "…00.042"`, `-1 → "1969-12-31 23:59:59.999"`; seconds/Europe/Paris `0 → "1970-01-01 01:00:00"`; nanoseconds/UTC `420 → "1970-01-01 00:00:00.000000420"`, and all reverse-parse to the original number.

PostgreSQL array/text parser tests [Observed: `test/org.jkiss.dbeaver.ext.postgresql.test/.../PostgreValueParserTest.java:91-146`]: `"{TRUE,FALSE}"` → `[true,false]` for `bool[]`; nested numeric arrays parse into nested collections; malformed array literals are passed back unchanged (e.g. `"{{1.1,22.22},,44.44}},{1.1,22.22},{3.3,}"` stays a string); `""` parses to an empty array.

### 3.7 Presentations, record mode, panels and value editors

- **Spreadsheet** uses `LightGrid`, a custom SWT `Canvas` (not a native table) that paints only visible cells; it supports column/row headers, record mode (rows become columns), hints, colour layers, boolean checkboxes, whitespace display, "mark occurrences". [Observed: `ui.editors.data/.../controls/lightgrid/LightGrid.java:50`; `ResultSetPreferences.java` keys `resultset.show.*`]
- **Plain text** renders an ASCII table with configurable max column width (default 255), tab size, delimiters (leading/trailing/top/bottom), NULL display, line numbers. [Observed: `plaintext/PlainTextPresentation.java`, `PlainTextFormatter.java`; `ResultSetPreferences.java` `resultset.text.*`]
- **Record mode** is a toggle; `resultset.behavior.autoSwitchMode` (default false) switches to record mode automatically when a re-executed query returns ≤ 1 row and metadata changed; a decorator can force it. [Observed: `ResultSetViewer.java:2295-2330, 2815-2840`]
- **Value editing modes** (`IValueController.EditType`): INLINE (in-cell editor), PANEL (value viewer panel), EDITOR (separate editor tab). Value managers are registered by data kind/type name/data source in `org.jkiss.dbeaver.dataManager`: boolean, numeric, string, date-time, array, reference, struct, content, object, UUID, URL preview, ROWID, default; PostgreSQL adds an enum manager, a bit-string manager and maps json/jsonb to the content manager. [Observed: `ui.editors.data/plugin.xml:18-75`; `ext.postgresql.ui/plugin.xml:157-180`]
- **Content (stream) viewers** are chosen per value by match level: EXCLUSIVE > PRIMARY > DEFAULT > APPLIES. Registered: `plaintext` (text/plain), `json` (PRIMARY when the content MIME is `text/json`, otherwise APPLIES), `xml`, `html`, `binary` (hex), `image` (PRIMARY only if an image decoder recognises the bytes, otherwise NONE). The user's choice per column is remembered. [Observed: `ContentPanelEditor.java:300-335, 392-433`; `ui.editors.json/.../JSONStreamValueManager.java:37-40`; `ui.editors.image/.../ImageStreamValueManager.java:46-57`; `model/.../utils/ContentUtils.java` `isJSON/isXML/isTextContent`]
- **Panels:** value viewer; column aggregates (pluggable `org.jkiss.dbeaver.aggregateFunction`); result metadata; grouping (re-queries with `GROUP BY` and counts, option "show duplicates only"); references (rows of related tables, §3.13). [Observed: `ui.editors.data/plugin.xml:6-12, 127-170`]

### 3.8 Column origin and row identity

`DBExecUtils.bindAttributes` runs once per new result (first segment) and decides which table each column comes from and which key identifies rows. [Observed: `DBExecUtils.java:759-985`]

1. **Owner entity.** If the viewer is a table editor, the table is known. Otherwise, if the statement came from the SQL editor, the parsed `SQLQuery` yields "entity metadata" (single source table) with a completeness score.
2. **Per column**, if the driver reports a source table for the column (`getEntityMetaData`) DBeaver resolves it in the navigator cache; if the query-level score is higher, or the owner is a view, the query-level table wins (comment: MySQL reports the base table instead of the view name). If the driver gives nothing and the select item is a plain column, the parser's table is used.
3. **Real column name.** If the driver's name equals the label, and the select item at that position is a plain column whose real name differs, the parser's name (unquoted or case-transformed) is used (issues #11194, #12008 in comments).
4. **Pseudo columns.** If the name matches a pseudo attribute of the entity (ROWID), the binding is marked and a fake table attribute is created.
5. **Table column lookup** happens only if there is no parsed query, the query is `SELECT *`, or the item is a plain column/`*` — expressions are never bound. All attributes are loaded at once to avoid per-column catalog queries.
6. **Type sanity check.** If the result-set data kind differs from the table column's kind and the driver did not report a table name, the binding is skipped ("probably an alias that conflicts with a column name"). If binding changes the value handler (e.g. strings turned out to be LOB or enum), already fetched values are converted.
7. **Row identifiers.** `bindUniqueIdentifiers` creates one `DBDRowIdentifier` per distinct entity using `getBestIdentifier` (ALG-DBV-GRID-01), then `lateBinding` runs transformers, then each identifier's attribute list is reloaded from the bindings; if any key column is missing from the result, the identifier's attribute list is cleared (it becomes *incomplete*). [Observed: `DBExecUtils.java:933-1029`; `model/.../data/DBDRowIdentifier.java:80-103`]
8. **Single source.** `detectSingleSourceTable` returns the entity only if all non-pseudo columns with identifiers point to the same entity; otherwise the result is multi-source (UPDATE still works per entity; INSERT/DELETE need a single source). [Observed: `DBExecUtils.java:1132-1150`; `ResultSetPersister.java:512-526`]

Pseudo columns declared: Oracle `ROWID` (`$alias.ROWID`), SQLite `rowid` (also recognises `oid`, `_rowid_`), DB2 and Exasol ROWID; PostgreSQL only `oid` and only when the table has OIDs and the server supports them (removed in PG 12). A grep for `ctid` in all plugins finds no use. [Observed: `ext.oracle/.../OracleConstants.java:188-196`; `ext.sqlite/.../SQLiteTable.java:45-65`; `ext.postgresql/.../PostgreConstants.java:99-107`, `PostgreTable.java:230-242`]

Read-only reasons surfaced to the user per column (`getAttributeReadOnlyStatus`): "Null meta attribute", "Attribute is read-only", the binding's own status (e.g. "no corresponding table column"), "No unique key. Row modification is not available.", "Underlying entity doesn't support data modification/update". With `resultset.edit.key.use_all_columns` (default false) the missing-key status is ignored. A global policy flag can disable editing entirely. [Observed: `DBExecUtils.java:1032-1087`; `ResultSetViewer.java:2384-2425`]

### 3.9 Editing in memory

- **Cell update** (`ResultSetModel.updateCellValue`): for scalar values, if the *string forms* of old and new value are equal, nothing happens. On the first edit of a column in a NORMAL row the original is stored in `changes`; complex values (`DBDValue`, e.g. struct, array, LOB) are cloned first so the original can be restored (if cloning is unsupported a debug log says undo is impossible). Nested edits write into the root value and register a nested→root back-reference. If the new value restores the original, the change entry is removed and counts recomputed. ADDED/REMOVED rows do not record changes. [Observed: `ResultSetModel.java:500-595, 656-716`]
- **Add row**: inserted at a visual position (`resultset.edit.new.row.after`, default true → after current), optionally copying the current row; state ADDED. Paste with "insert new rows" uses `preserveNewRows`. [Observed: `ResultSetModel.java:987-1037`; `ResultSetViewer.java:4910-…`]
- **Delete row**: an ADDED row is physically removed; a NORMAL row is marked REMOVED (shown struck-through until save). [Observed: `ResultSetModel.java:1047-1059`]
- **Undo/redo**: `ResultSetUndoRedoManager`, depth `resultset.edit.undoLevel` capped at 10,000; cleared after save or reject. [Observed: `ResultSetPreferences.java` `MAX_EDIT_UNDO_LEVEL`; `ResultSetPersister.java:428-436`]
- **Reject** restores originals (releasing replaced values), removes ADDED rows, un-marks REMOVED rows, refreshes hint caches. [Observed: `ResultSetPersister.java:383-424`]
- **Generate UUID** handler and "set NULL"/"set default" style actions exist as handlers. [Observed: `handler/GenerateUUIDHandler.java`]

### 3.10 Saving changes

Entry: `ResultSetViewer.applyChanges`. Steps (details in ALG-DBV-GRID-02):

1. If `resultset.confirm.beforeSave` is on, or the connection type has "confirm data change" (production-type connections), a **Save preview dialog** shows counts (inserts/updates/deletes), the generated script, and — when deletes exist and the database supports referential integrity — "delete cascade" / "deep cascade" options; it also exposes "use fully qualified names". [Observed: `ResultSetViewer.java:4732-4764`; `ResultSetPersister.java:154-178`; `model/.../ResultSetSaveSettings.java`]
2. `createDataPersister` → if any row is changed or deleted, `checkEntityIdentifiers`: needs a connection; INSERT/DELETE need a single source; an empty virtual key triggers the "No unique key" dialog (define custom key / use all columns); deletes need a valid default identifier; every updated column needs a valid identifier. [Observed: `ResultSetViewer.java:4890-4909`; `ResultSetPersister.java:512-574`]
3. `prepareStatements` (delete, insert, update) → `execute` in a `DataUpdaterJob`; options carry `OPTION_FULLY_QUALIFIED_NAMES`. [Observed: `DBDResultSetDataUpdater.java:95-113, 260-305`; `DataUpdaterJob.java`]
4. Completion: on success (or partial success under auto-commit) `reflectChanges` updates only rows whose statements executed (key values from generated keys copied into cells, changes cleared, ADDED→NORMAL, REMOVED rows dropped); status line "N inserted, M deleted, K updated"; warning and confirmation dialog if any statement touched zero rows. Then either a full refresh (if the entity declares `FEATURE_DATA_REFRESH_AFTER_UPDATE`) or a per-row re-read by key (`RowRefreshJob`) when `resultset.edit.refreshAfterUpdate` is true (default). [Observed: `ResultSetPersister.java:428-475, 576-616, 714-827`; `ResultSetViewer.java:4771-4815`]
5. `RS_COMMIT_ON_EDIT_APPLY` / `RS_COMMIT_ON_CONTENT_APPLY` control whether applying an inline/LOB edit also commits. "Smart commit" connections switch auto-commit off before saving. [Observed: `ResultSetPreferences.java`; `DBDResultSetDataUpdater.java:338-347`; `DBCSmartTransactionManager.java`]

### 3.11 Filters and ordering

**Filter model.** `DBDDataFilter` = list of `DBDAttributeConstraint` (one per binding, including nested) + free-text `where` + free-text `order` + `anyConstraint` (OR instead of AND) + `useDisjunctiveNormalForm`. A constraint holds *either* a free-text `criteria` (e.g. `> 5`) *or* an `operator` + `value` (+ `reverseOperator` for NOT), plus `orderPosition`/`orderDescending`, `visible`, `visualPosition`, `entityAlias`, options. Operators: `= <> > >= < <= IS NULL IS NOT NULL BETWEEN IN ILIKE LIKE NOT LIKE REGEX SOUNDS CONTAINS CONTAINS KEY`; a value handler can restrict which are offered. [Observed: `model/.../data/DBDDataFilter.java:33-412`; `DBDAttributeConstraintBase.java:31-60`; `model/.../exec/DBCLogicalOperator.java:30-150`]

**Where filters come from:** the filter text box (free SQL WHERE with autocompletion), column header filter icons, the "filter by value" context menu (equals/not equals/IN list/LIKE/IS NULL), the value-filter popup with distinct values (served by the dictionary accessor `JDBCTable.getDictionaryAccessor`), FK navigation. Filters per table are persisted in `DataFilterRegistry` and restored when the table is reopened. [Observed: `ResultSetFilterPanel.java`, `valuefilter/*`, `DataFilterRegistry.java:182-290`; `JDBCTable.java:1010-1460`]

**SQL generation.** See ALG-DBV-GRID-04.

**Ordering.** Header click toggles none → ASC → DESC → none (multi-column: new columns appended at max position + 1). Strategy `resultset.order.mode`: SMART (default) sorts locally when everything is loaded, else server-side; CLIENT_SIDE always local; SERVER_SIDE always re-query (with a confirmation `CONFIRM_ORDER_RESULTSET`). A free-text ORDER BY in the filter forces server side and is cleared when the user clicks a header ("can't use both"). If the data source cannot order result sets server-side the user is warned and sorting falls back to local. **Local sort calls `rejectChanges()` first** (pending edits are lost). [Observed: `ResultSetViewer.java:2704-2778`; `OrderingStrategy.java`]

**Default ordering policy** `resultset.order.policy`: DEFAULT (none), PRIMARY_KEY_ASC, PRIMARY_KEY_DESC; applied client-side after a fresh open without a saved filter, or server-side in `JDBCTable.readData` when the strategy is SERVER_SIDE. [Observed: `OrderingUtils.java:37-100`; `ResultSetViewer.java:4240-4300`; `JDBCTable.java:194-206`]

**Quick filter** (`SearchQuickFilterInfo`) is purely local: a row is kept if *any* column's `toString()` matches; disables next-page fetches while active. [Observed: `ResultSetModel.java:109-146`; `ResultSetViewer.java:4427-4429`]

### 3.12 Find / replace

`SpreadsheetFindReplaceTarget` implements the Eclipse find/replace interfaces: case sensitivity, whole word, regex (Java `Pattern`), incremental search, replace and replace-all over cells. Replace runs `pattern.matcher(oldText).replaceAll(newText)` on the *display string* and pushes the result through the value controller (so it becomes a pending edit), tracking processed cells to avoid infinite loops in replace-all; grid redraw is deferred to a job during replace-all. An overlay (`ResultsetFindReplaceOverlay`) provides the inline search bar. [Observed: `spreadsheet/SpreadsheetFindReplaceTarget.java:60-431`]

### 3.13 Foreign-key navigation and the references panel

- For an FK column, "navigate" builds a filter on the referenced table from the selected rows (`resolveAssociationNavigation`); "references" (reverse direction) builds a filter on the referencing table (`resolveReferenceNavigation`). Virtual FKs from the virtual model are included (`findVirtualReferrers`). Navigation opens the target in the same viewer (with history back/forward) or a new editor. [Observed: `model/.../data/DBDReferenceUtils.java:45-191`; `DBDAttributeBinding.java:356-381`; `ResultSetReferenceMenu.java`]
- The references panel shows referencing rows for the current selection and re-queries as the selection moves; the data-pump queue's "keep only the newest job" rule exists for this panel. [Observed: `panel/references/ReferencesPanel.java`; `ResultSetViewer.java:4642-4648` comment]

### 3.14 LOB handling

- `DBDContent`: length, MIME type, display string, `getContents(monitor) → DBDContentStorage`, `updateContents(storage)`, `resetContents()`. [Observed: `model/.../data/DBDContent.java:25-73`]
- JDBC: if the driver returns a `String` for a content column it is wrapped as in-memory chars and never converted to binary (issue #494 comment). CLOB contents shorter than `content.memory.maxsize` (default 10,000) are loaded with `getSubString` (falls back to the reader); larger ones are copied into a temp file using the content encoding; the driver handle is released afterwards. [Observed: `JDBCContentValueHandler.java` `fetchColumnValue`; `JDBCContentCLOB.java:88-136`; `ModelPreferences.java:200,337`]
- Preferences `content.cache.clob` (true), `content.cache.blob` (false), `content.cache.maxsize` (1,000,000), `resultset.edit.maxtextsize` (100) and `resultset.binary.stringMaxLength` (32) are shown on the preferences page, but a repository-wide search finds no other reference to these keys outside preference pages and message bundles. [Observed by grep; **Inferred**: these are legacy or read through generic code paths not found by name; treat as unverified]
- Binary display formats: hex, hex-native, hex-string, base64, string (`BinaryFormatter*`), default chosen by `resultset.binary.representation`. [Observed: `model/.../impl/data/formatters/Binary*.java`; `ModelPreferences.java:231,371`]
- LOB edits apply to `DBDContent` and, for keys, the persister clones content values (`DBDValueCloneable`) because a LOB used in a WHERE must hold the *original*; a non-cloneable LOB key aborts the save with "can't be used as a key". [Observed: `DBDResultSetDataUpdater.java:139-158`]

### 3.15 Copy, advanced copy, copy-as and paste

- **Plain copy** (Ctrl+C): tab-separated, OS line separator, display format UI, no header. **Advanced copy** (Ctrl+Shift+C) dialog: header, row numbers, quote cells (default on), force quotes, column/row delimiter, quote string, display format (UI/EDIT/NATIVE), copy as HTML table. **Cut** copies then sets each editable cell to NULL. If a selected cell is binary, the raw bytes are also put on the clipboard as a byte-array transfer. [Observed: `ResultSetCopySettings.java`; `handler/ResultSetHandlerCopySpecial.java:110-230`; `SpreadsheetPresentation.java:530-677`]
- **Copy as <format>** reuses the data-transfer exporters (CSV, JSON, Markdown, SQL INSERT, XML, HTML, …) with a stream consumer writing to the clipboard; per-format settings are remembered. [Observed: `handler/ResultSetHandlerCopyAs.java:70-300`]
- **Paste** (Ctrl+V) into a selection: every selected editable cell gets the clipboard value (binary columns accept clipboard bytes). **Advanced paste** (Ctrl+Shift+V) parses the clipboard as a TSV grid (ALG-DBV-GRID-07), optionally inserts new rows, optionally maps a "NULL mark" text to NULL, can ignore quotes; values are converted by `getValueFromObject(…, validate=true)`; read-only columns are skipped silently; multi-range selections are refused. [Observed: `SpreadsheetPresentation.java:679-850, 853-874`; `ResultSetPasteSettings.java`]

### 3.16 Virtual model (DBV)

| Element | Purpose | Evidence |
|---|---|---|
| `DBVEntity` | Per table or per query container; holds the elements below plus a description-column list (columns used as human-readable labels for FK lookups; length bounds 4–1000). | `model/.../virtual/DBVEntity.java` |
| `DBVEntityConstraint` | Virtual unique key; may be "use all columns"; `getBestIdentifier()` returns the first complete constraint or creates an empty `VIRTUAL_PK`. | `DBVEntity.java:398-440`; `DBVEntityConstraint.java:37-140` |
| `DBVEntityForeignKey` | Virtual FK to another (possibly cross-connection) entity; used by navigation and references. | `DBVEntityForeignKey.java` |
| `DBVEntityAttribute` | Custom (computed) column with a JEXL expression, default value, description, transform settings (custom transformer, include/exclude, options). | `DBVEntityAttribute.java:19, 46, 272-345` |
| `DBVColorOverride` | Colour rule: column, operator, values (or range), single-column vs whole row, fg/bg (two colours for gradients). | `DBVColorOverride.java:29-36` |
| `DBVGroupRowStriping` | Alternate background by groups of equal values. | `DBVGroupRowStriping.java` |

Persistence: serialised as JSON under `virtual-models` in the project's `data-sources.json` (`attributes` with `custom`/`expression`/`transforms`, `constraints`, `foreign-keys`, `colors`, striping `columns`). [Observed: `DBVModelSerializerModern.java:49-210`; `registry/.../DataSourceSerializerModern.java:138-175`]

### 3.17 Quirks and probable defects found while reading

| # | Observation | Evidence | Impact |
|---|---|---|---|
| Q1 | Local multi-column sort uses the comparator of the *clicked* column for every sort key. | `ResultSetModel.java:1254-1292` (comparator taken from `columnElement`, not from the binding of each order constraint) | Mixed-type multi-sorts can misorder. |
| Q2 | Default server-side PK ordering joins key columns and appends one direction: `a,b DESC` sorts only `b` descending. | `OrderingUtils.java:68-100` | PRIMARY_KEY_DESC on composite keys is wrong. |
| Q3 | Advanced copy quotes but does not double embedded quotes. | `SpreadsheetPresentation.java:640-646` | Pasting into spreadsheets/CSV parsers corrupts cells containing `"`. |
| Q4 | Paste parser keeps `""` inside quoted cells literally and drops a trailing empty cell. | `SpreadsheetPresentation.java:876-944` | Round-trip copy→paste is lossy. |
| Q5 | Scalar "no change" detection compares `toString()` forms. | `ResultSetModel.java:532-538` | Typing `1.0` over `1` (BigDecimal) is a change; typing the same text over a value whose `toString` differs from display can be a no-op or a spurious change. |
| Q6 | No multi-row guard: only "zero rows affected" is reported; batched counts of `SUCCESS_NO_INFO` are assumed to be 1. | `ResultSetPersister.java:576-616`; `ExecuteBatchImpl.java:311-330` | With "use all columns" keys or duplicate rows a single edit may update many rows unnoticed. |
| Q7 | Explicit NULL is omitted from INSERT when other columns are non-null → DEFAULT wins. | `ExecuteInsertBatchImpl.java:72-77, 167-176`; the updater test `explicitNullIsIncludedInInsert` asserts only the *prepared* statement keeps the NULL. | Surprising data. |
| Q8 | Re-read-on-scroll by default: fetching page *n* transfers `n×200` rows. | `ResultSetJobDataRead.java:101-106` | O(n²) traffic on deep scrolls. |
| Q9 | Client-side offset skipping when SQL limit is off. | `JDBCStatementImpl.java:238-275` | Deep pages read and discard all previous rows. |
| Q10 | Quick filter matches `toString()` of raw values (dates, LOB objects) instead of displayed text. | `ResultSetModel.java:119-130` | Searching for what you see may fail. |
| Q11 | `BETWEEN` renders the same value twice when the constraint has a single value. | `StandardSQLDialectQueryGenerator.java:362-372` | [Inferred: depends on whether UI supplies an array; not verified at runtime] |
| Q12 | Local sort discards pending edits without asking. | `ResultSetViewer.java:2772-2778` | Data-loss surprise. |

Issue numbers referenced inside the code at the places above: #424 (nullable unique indexes are not identifiers), #494, #2541, #3829 (prefer natural keys over ROWID), #4652, #6311 (filters reset on refresh), #6927 (no table prefix when the column is not a table column), #11194, #12008, #40210. [Observed: comments in `DBUtils.java:1450`, `JDBCContentValueHandler.java`, `SQLQueryTransformerCount.java`, `DBExecUtils.java:670`, `ResultSetModel.java:1180`, `SQLSemanticProcessor.java:350`, `DBExecUtils.java:858-872`] A broader GitHub issue survey is in `dbeaver-05`.

### 3.18 DBeaver vs DataGrip (grid)

| Topic | DBeaver CE | DataGrip 2024.3.5 (report-grid.md) | Better for Hacivat |
|---|---|---|---|
| Paging UX | Infinite scroll by segments (200) + "fetch all"; reread from 0 by default | Explicit First/Prev/Next/Last pages, page size presets, Count Rows on demand | Both: infinite scroll for consoles, page bar for table editors |
| Offset mechanism | JDBC maxRows + client skip (default) or LIMIT transformer | `database.grid.paging.offset=true` (LIMIT/OFFSET) | Keyset for tables; cursor/portal for queries |
| Row identity | PK → NOT NULL unique → ROWID/oid → virtual key → all columns | PK/unique only; otherwise "Missing row identifier" → read-only | Strict default + explicit opt-in fallbacks |
| Old-value concurrency check | None | None | Add it (opt-in per connection, default on for production) |
| Bad input | Converter throws → error dialog | `UnparsedValue` kept in cell, blocks submit, hover shows error | DataGrip approach |
| DML preview | Save preview dialog (opt-in / prod connections) | "Preview pending changes" always available | Always available |
| After save | Re-read rows by key; generated keys via JDBC | `SelectUsingKeys` re-read | Same, plus `RETURNING`/`OUTPUT` where available |
| Filter on query results | JSQLParser injection, fallback subquery wrap | `SELECT * FROM (q) WHERE … ORDER BY … LIMIT` wrapping [I] | Wrap by default; inject only when parse is exact and needed |
| Local filter | Quick filter (any column, toString) | Excel-style per-column value checklist | DataGrip-style + text search on display strings |
| Virtual keys/FKs, colour rules, computed columns | Yes (virtual model) | Virtual FKs; no colour rules found | Virtual keys/FKs yes; colour rules later |
| Copy formats | TSV, advanced copy, copy-as any exporter | Extractor scripts (Groovy/JS), CSV formats, transposed variants | Native extractor set + scripting later |
| Aggregates | Aggregate panel | Status-bar aggregators (SUM, AVG, MEDIAN…) | Status bar, async |

---

## 4. Algorithm specifications

### ALG-DBV-GRID-01: Unique-key (row identifier) detection for editing

- **Problem:** Decide whether a result set is editable and, per source table, which columns identify a row uniquely so that UPDATE/DELETE affect exactly that row.
- **Inputs / outputs:** Input: result columns with resolved origins `(table, column)` (from ALG-DBV-GRID-05), catalog metadata per table (PK, unique constraints, unique indexes with column nullability, table kind view/table, pseudo columns), virtual-model keys, user preferences. Output: per table either `RowIdentifier{table, key_kind, columns}` (complete), `Incomplete{reason}`, or `None{reason}`; plus a per-column read-only reason.
- **Algorithm (DBeaver, as observed):**
  1. Group result columns by origin table; skip columns without origin ("no corresponding table column").
  2. For each table `T` (first time seen):
     1. If `T` is a document container → document identity (out of scope for SQL).
     2. If `T` is not a view: collect candidates in this order — the primary index if it is an *identifier index* (unique and every column NOT NULL), then the first other identifier index, then every constraint that is unique and whose columns are all NOT NULL (or the database supports nullable unique constraints). Errors reading indexes/constraints are ignored.
     3. Among candidates whose columns are **all present in the result** ("good referrer"): return the first PRIMARY KEY; otherwise remember the first unique one; a non-referrer constraint is accepted as-is.
     4. If none: if the result contains a pseudo column of type ROWID for `T`, use it.
     5. Otherwise return the virtual entity's best identifier: first *complete* virtual key (unique type and ≥1 column or "use all columns"); if none exists, create an empty `VIRTUAL_PK` (incomplete).
  3. Attach the identifier to every column of `T`; reload its column list from the result; if any key column is missing → clear (incomplete).
  4. At save time: an incomplete virtual key opens a dialog — *Custom unique key* (pick columns) or *Use all columns* (all columns that map to table columns; persisted in the virtual model). Deletes require a valid identifier; updates require one per changed column.
  [Observed: `DBExecUtils.java:584-712, 987-1029`; `DBUtils.java:1437-1484`; `DBVEntity.java:398-440`; `DBDRowIdentifier.java:41-103`; `ValidateUniqueKeyUsageDialog.java:91-140`; `ResultSetPersister.java:512-574`]
- **Data structures:** `RowIdentifier { table_id, kind: Pk|UniqueConstraint|UniqueIndex|PhysicalRowId|Virtual|AllColumns, key_columns: Vec<ResultColumnIdx>, complete: bool }`; `ReadOnlyReason` enum (NotConnected, ConnectionReadOnly, NoPermission, NoOrigin, Expression, NoKey, KeyColumnMissing(name), ViewNotUpdatable, Aggregated, MultiSourceInsert).
- **Complexity / performance:** O(columns + Σ constraints) per result; catalog lookups must be cached (DBeaver forces "load all attributes once" to avoid N catalog queries — `DBExecUtils.java:882-890`). Never block the first paint on this: DBeaver runs it inside the fetch job before data is shown; Hacivat should show rows immediately and enable editing when key detection finishes.
- **Edge cases (exhaustive list):**
  1. Table with PK but the query does not select all PK columns → not a "good referrer"; DBeaver falls through to unique/ROWID/virtual. Hacivat: offer "add key columns invisibly" (re-query with hidden key columns) for table editors; for console queries report "Key column `id` not in result".
  2. Unique index with a nullable column (#424) → rejected, since two NULLs do not collide.
  3. Unique constraint with nullable columns on a DB that treats NULLs as equal (e.g. SQL Server unique index, PG 15 `NULLS NOT DISTINCT`) → acceptable only if the engine guarantees it; DBeaver uses a data-source flag `supportsNullableUniqueConstraints`.
  4. Partial unique index (`WHERE active`) → not a row identifier for all rows. DBeaver does not appear to check predicates [Unknown]; Hacivat must exclude partial and expression indexes.
  5. Deferrable unique constraint → still identifies rows at statement end; acceptable.
  6. Views: DBeaver skips physical keys; updatable views need a virtual key. Hacivat: for PG simple views, resolve base table via `pg_rewrite`/`information_schema.view_column_usage` as an enhancement.
  7. Self-join (`FROM t a JOIN t b`) → two aliases of the same table; DBeaver keys by entity object identity, so both aliases share one identifier → ambiguous. Hacivat: key by *range variable* (alias), not table.
  8. Joins of several tables → per-table identifiers; UPDATE allowed per table, INSERT/DELETE refused (multi-source).
  9. Columns renamed with aliases (`SELECT id AS user_id`) → origin from protocol still maps; DBeaver relies on driver metadata or parser.
  10. Same column selected twice → two bindings to one table column; editing either must update both cells.
  11. Expressions, aggregates, DISTINCT, GROUP BY, window functions, UNION → no origin → read-only (DBeaver: expressions are never bound, `DBExecUtils.java:882-890`).
  12. Pseudo ROWID: Oracle ROWID stable within a session unless row movement; SQLite rowid stable unless `VACUUM` on tables without INTEGER PRIMARY KEY; PostgreSQL `ctid` changes on every UPDATE → only safe for a single statement right after read and with `xmin` check (DBeaver avoids it).
  13. WITHOUT ROWID SQLite tables → no rowid.
  14. "Use all columns" with duplicate rows → UPDATE/DELETE hits all duplicates; with floating point/LOB/JSON columns equality may never match (zero rows) — DBeaver warns the user in the dialog text.
  15. Key column is a LOB → must clone original; if not cloneable abort.
  16. Key value NULL (only possible with virtual keys) → predicate becomes `IS NULL` (§ALG-02).
  17. Table metadata unavailable (permissions) → identifier missing; read-only reason "cannot read table metadata".
  18. Case-sensitivity mismatch between driver-reported name and catalog name (`"User"` vs `user`) → normalise using dialect identifier rules (#12008).
  19. Result from stored procedure / `RETURNING` / CTE with DML → no origin.
  20. Connection read-only / no edit permission → read-only regardless of key.
  21. Schema changed after fetch (key dropped) → re-validate at save.
- **Test cases (input → expected):**
  - DBeaver has no direct unit test for `getBestIdentifier`; cases below are derived from code reading [Inferred from `DBExecUtils.java:584-712`].
  - `t(id PK, name)`, `SELECT * FROM t` → PK(id), editable.
  - `t(id PK, name)`, `SELECT name FROM t` → no good referrer; no ROWID (PG) → virtual empty → editing shows "No unique key" on save.
  - `t(a INT NULL UNIQUE, b)` → rejected (#424) → virtual.
  - `t(a INT NOT NULL UNIQUE, b)` (no PK) → Unique(a).
  - `v` (view over t) → virtual only.
  - Oracle `SELECT t.*, ROWID FROM t` on a table without PK → PhysicalRowId.
  - Hacivat additional: `SELECT a.id, b.id FROM t a JOIN t b ON …` → two identifiers keyed by alias; editing `b.id` updates via `b`'s key value.
- **Prior art and evidence:** DBeaver as cited; DataGrip report: "PK/unique key; yoksa Missing row identifier → salt okunur. ctid/ROWID izi yok" (PK/unique only, otherwise read-only; no ctid/ROWID use).
- **Recommendation for Hacivat:** Strict default identical to DataGrip (PK → non-partial, non-expression unique with all NOT NULL columns). Fallbacks behind explicit, per-table, persisted user choices: (1) virtual key, (2) physical row id where stable (SQLite rowid, Oracle ROWID), (3) PostgreSQL `ctid` **only** combined with `xmin` equality and only for the row as last read, (4) all-columns with mandatory `rows_affected == 1` guard. Key by range variable. Run detection asynchronously after first paint.
- **Used by:** Hacivat (grid editor), Karagöz (policy: forbid all-column keys on protected databases).

### ALG-DBV-GRID-02: DML generation and execution on save

- **Problem:** Turn the in-memory overlay (changed cells, added rows, removed rows) into correct, safe SQL and apply it atomically, then reconcile the grid with what the server stored.
- **Inputs / outputs:** Input: rows with state and change map, bindings with identifiers, dialect capabilities, transaction mode, save settings (cascade, deep cascade, qualified names). Output: either executed statements with statistics and reconciled rows, or a script (for preview), or an error with partial-execution information.
- **Algorithm (DBeaver, as observed):**
  1. Collect rows: NORMAL with changes → updates; ADDED → inserts; REMOVED → deletes. For each updated row, compute the change set: if a nested element of an array (or of a composite requiring whole-value updates) changed, write the whole root value; otherwise write leaf changes; independent top-level changes are kept. Group changed columns by their row identifier (one UPDATE per table per row). [Observed: `ResultSetPersister.java:226-246, 324-380`; `DBDResultSetDataUpdater.java:578-599`]
  2. Prepare DELETEs: key = default identifier columns, values = *current* cell values. If cascade is requested and the DB supports referential integrity: for every incoming FK that is not already ON DELETE CASCADE, add a DELETE on the referencing table using the key values mapped through the FK columns, placed *before* the parent delete (one level; "deep cascade" flag exists in settings). [Observed: `DBDResultSetDataUpdater.java:221-256`; `ResultSetPersister.java:249-299`]
  3. Prepare INSERTs (single source required): if the result is a single document → insert the document; else one entry per non-virtual column with its value; mark "needs generated keys" if an auto-generated column is NULL. [Observed: `DBDResultSetDataUpdater.java:171-205`]
  4. Prepare UPDATEs: SET = changed non-virtual columns with current values; WHERE = identifier columns with values; if a key column itself was changed, the WHERE uses the **original** value from the change map (LOB originals are cloned). [Observed: `DBDResultSetDataUpdater.java:119-168`]
  5. Execute in one session: if the container uses smart commit and auto-commit is on → turn auto-commit off. Read auto-commit state; if off and savepoints supported → set a savepoint. Run all DELETEs, then INSERTs, then UPDATEs; each statement through `insertData/updateData/deleteData` batches. On the first error: mark the statement not executed, roll back to the savepoint (if auto-commit off) and stop. Cancellation stops without error. Finally release the savepoint. [Observed: `DBDResultSetDataUpdater.java:316-475, 533-547`]
  6. SQL text (JDBC tables):
     - UPDATE: `UPDATE <table> [alias]\n\tSET [alias.]c1=?,c2=?\n\tWHERE [alias.]k1=? AND k2 IS NULL` — alias `DEFAULT_TABLE_ALIAS` only if the dialect supports aliases in UPDATE; NULL key values become `IS NULL` and are not bound; key columns use dialect type casts (`dialect.getTypeCastClause`); value handlers implementing `DBDValueBinder` can emit custom bind expressions (e.g. `?::jsonb`). [Observed: `JDBCTable.java:445-555, 1065-1098`]
     - DELETE: `DELETE FROM <table> [alias]\n\tWHERE …` same predicate rules. [Observed: `JDBCTable.java:557-617`]
     - INSERT: `INSERT INTO <table> (cols)\n\tVALUES (?,?)` listing only non-NULL, non-pseudo columns; if all values are NULL → `INSERT INTO t DEFAULT VALUES` when supported, else list all columns with `DEFAULT` for columns having a default. A dialect may substitute `UPSERT INTO` (Phoenix-like) for both INSERT and UPDATE. Optional multi-row VALUES for data transfer. [Observed: `ExecuteInsertBatchImpl.java:72-200`; `JDBCTable.java:379-431, 1060-1063`]
     - Batching: statements are reused and JDBC-batched only when >1 row, the driver supports batches, and consecutive rows have the **same NULL pattern** (because NULLs change the SQL text); otherwise executed one by one with row count and generated keys read after each. Unknown batch counts (negative) are assumed to be 1. [Observed: `ExecuteBatchImpl.java:97-247, 311-330`]
     - Script generation (preview) inlines values with `convertValueToSQL(NATIVE)` while skipping quoted regions; named parameters are not handled ("FIXME"). [Observed: `ExecuteBatchImpl.java:250-309`]
  7. Reconcile: generated keys are matched by column name, else to the first auto-generated column; rows whose statements executed are updated (ADDED→NORMAL, changes cleared, REMOVED dropped). If any statement reported 0 rows → warning + confirmation dialog (not an error). Then per-row refresh by key (`SELECT … WHERE k=v` per row, skipped for document stores and rows whose key is still NULL) or full refresh. A refresh failure with "rollback on error" enabled rolls back the open transaction. [Observed: `ResultSetPersister.java:428-475, 576-616, 628-694, 714-827`]
- **Data structures:** `PendingChanges { updates: Map<RowKey, Map<ColIdx, (old, new)>>, inserts: Vec<RowDraft>, deletes: Set<RowKey> }`; `DmlStatement { kind, table, set: Vec<(Col, Value)>, where_: Vec<(Col, KeyValue)>, expect_rows: Exactly(1) | AtLeast(0) }`.
- **Complexity / performance:** One round trip per row unless batched; DBeaver's NULL-pattern batching helps bulk edits. Hacivat: pipeline statements (PG extended protocol pipelining; MySQL multi-statement off by default — loop) inside a single transaction; for >1000 edits prefer `UPDATE … FROM (VALUES …)` [Inferred].
- **Edge cases (exhaustive list):**
  1. Editing a key column → WHERE must use the original key; subsequent edits in the same session must use the *new* key after save.
  2. Two edits on the same row in different tables (join result) → two UPDATEs in a deterministic order.
  3. Delete + insert of the same key in one save → DELETE first (DBeaver order) avoids unique violations; UPDATE swapping two unique values needs deferred constraints or a temporary value.
  4. Insert into a table with identity column left empty → omit column and read generated key (or `RETURNING *`).
  5. Explicit NULL into a column with DEFAULT → must insert NULL (DBeaver Q7 inserts DEFAULT). Distinguish `Value::Null` from `Value::Default` (DataGrip has "Set DEFAULT").
  6. All columns NULL/empty → `DEFAULT VALUES` (PG, SQLite, SQL Server) vs `VALUES ()` (MySQL) vs not supported.
  7. Generated (computed) columns → never in SET/INSERT.
  8. Trigger modifies row / RLS hides row after update → refresh by key may return 0 rows; keep last known values and flag "row no longer visible".
  9. UPDATE affects 0 rows (row deleted or changed concurrently, or key with float imprecision) → DBeaver warns; Hacivat should treat as conflict and roll back the batch unless the user accepts.
  10. UPDATE affects >1 row → DBeaver does not detect; Hacivat must roll back.
  11. Float / timestamp precision in keys → equality may fail; prefer integer/uuid keys; for all-column keys exclude floats/LOBs/JSON.
  12. NULL key values → `IS NULL` (not `= NULL`).
  13. Oracle empty string = NULL → dialect flag (DBeaver has `useEmptyStringForNulls` in filters).
  14. LOB values in WHERE → most engines cannot compare LOBs; exclude LOB columns from all-column keys.
  15. JSON/jsonb, arrays, composite, enum, interval, money, geometry → need explicit casts on bind (`?::jsonb`, `CAST(? AS …)`); DBeaver uses `DBDValueBinder` and `getTypeCastClause`.
  16. Case-sensitive / reserved identifiers → always quote using dialect rules.
  17. Auto-commit ON: partial application on failure (statements already executed stay) → DBeaver advances the baseline for executed rows; Hacivat: always wrap the save in an explicit transaction even in auto-commit mode (begin/commit), unless the user disables it.
  18. Auto-commit OFF without savepoint support → rollback of the whole user transaction on error (dangerous); warn.
  19. Cancellation mid-save → leave remaining statements pending (DBeaver test `canceledExecutionLeavesStatementPendingWithoutError`), roll back the explicit transaction.
  20. Cascade delete across FKs: one level only in DBeaver; cycles; FKs already ON DELETE CASCADE skipped.
  21. Connection lost during save → state unknown; reconnect must not replay automatically.
  22. Read-only replica / `default_transaction_read_only` → server error; surface clearly.
  23. Upsert-only engines (Phoenix) → UPSERT for both.
  24. Very long values in script preview → truncate in preview only.
  25. Preview text must equal executed SQL (DBeaver generates both from the same batch code path — good property).
- **Test cases (input → expected):**
  - From DBeaver `test/org.jkiss.dbeaver.test.platform/.../DBDResultSetDataUpdaterTest.java`:
    - `explicitNullIsIncludedInInsert` (45-69): new row with nullable column = NULL and auto-generated column = NULL → prepared INSERT has 2 key attributes, both NULL, and `needKeys() == true`.
    - `repeatedPreparationDoesNotDuplicateStatements` (71-86): calling `prepareStatements` twice → exactly 1 insert statement.
    - `customDocumentValueIsUsedForInsert` (88-106): document-based result → INSERT carries the document attribute with the custom document value.
    - `customKeyAttributeAndValueAreUsedForDelete` (108-136): subclass overrides key attribute/value → DELETE uses them.
    - `canceledExecutionLeavesStatementPendingWithoutError` (138-164): monitor cancelled → `executeStatements` returns null and the insert is not marked executed.
    - `executeStatementsWithoutExecutionContextReturnsError` (166-176): no context → `DBCException("No execution context")`.
  - SQL shape tests [Inferred from `JDBCTable.java:461-521`, `ExecuteInsertBatchImpl.java:119-200`]:
    - Update `name` on PK `id=5` (alias supported) → `UPDATE public.t x\n\tSET x.name=?\n\tWHERE x.id=?` binds `["new", 5]`. (Exact alias token is `DEFAULT_TABLE_ALIAS`; not verified at runtime.)
    - Virtual key `(a,b)` with `b` NULL → `… WHERE a=? AND b IS NULL`, binds `[a]`.
    - Insert `{id: NULL(identity), name: "x"}` → `INSERT INTO t (name)\n\tVALUES (?)`.
    - Insert all NULL on PG → `INSERT INTO t DEFAULT VALUES`.
  - Hacivat-specific: UPDATE returning 2 rows → transaction rolled back, error "2 rows matched, expected 1"; concurrent change with optimistic mode → 0 rows → conflict dialog showing server values.
- **Prior art and evidence:** DBeaver as cited. DataGrip: Insert/MultiInsert/UpdateRow/DeleteRows/WhereClause/SelectUsingKeys tasks per dialect, submit then re-select by keys, DML preview, no optimistic concurrency ("Optimistic concurrency yok → eklenmeli").
- **Recommendation for Hacivat:** Parameterised DML generated by a dialect trait from a typed `DmlStatement`; always inside an explicit transaction; enforce `rows_affected == 1` per row statement; optional optimistic WHERE (compare original values of changed columns, or `xmin` on PG / `ROWVERSION` on SQL Server); use `RETURNING *` (PG, SQLite ≥ 3.35, MariaDB ≥ 10.5 for INSERT/DELETE) or `OUTPUT inserted.*` (SQL Server) to reconcile in the same round trip, else re-select by key; preview = exactly the statements to run with values rendered via the NATIVE formatter; never omit explicit NULLs.
- **Used by:** Hacivat; Karagöz (audit each executed DML with old/new values when the policy requires).

### ALG-DBV-GRID-03: Result-set paging and fetching

- **Problem:** Show the first rows fast, load more on demand, avoid unbounded memory, and keep pages consistent.
- **Inputs / outputs:** Input: data container (table or SQL), filter, page size `S`, current loaded count `n`, mode (first, next, all, refresh), dialect capabilities, preferences. Output: rows appended or replaced, `has_more` flag, statistics.
- **Algorithm (DBeaver, as observed):**
  1. First read: `offset = 0`, `maxRows = S` (min 1; default 200).
  2. Next: `offset = n`, `maxRows = S`. If reread-on-scroll (default) → `offset = 0`, `maxRows = n + S`, and the model replaces all rows.
  3. All: `offset = n`, `maxRows = -1` (unbounded), after confirmation.
  4. Refresh: `offset = 0`, `maxRows = ceil((focus+1)/S)·S`.
  5. Statement: plain SELECT only. If LIMIT-by-SQL is enabled and the query has no LIMIT/OFFSET/INTO/UPDATE/PROCEDURE/FETCH token: append `LIMIT off,len` / `LIMIT len OFFSET off` / `LIMIT off+len` by dialect. Else JDBC `setMaxRows(off+len)` and skip `off` rows client-side.
  6. Fetch-size hint only if forced (pref) or passed: `off+len` for bounded reads, else 10,000.
  7. Read rows until `maxRows` or cancel; per-cell errors become error values.
  8. `has_more = maxRows > 0 && received >= maxRows`.
  9. Concurrency: newest request wins; one read at a time per viewer; cancel timeout detaches stuck jobs.
  [Observed: `ResultSetViewer.java:4319-4510, 4549-4710`; `ResultSetJobDataRead.java:89-160`; `ResultSetDataReceiver.java:156-215`; `DBUtils.java:1659-1733`; `QueryTransformerLimit.java`; `JDBCStatementImpl.java:238-275`; `DBExecUtils.java:308-321`]
- **Data structures:** Hacivat: `PageCursor { mode: Keyset{order_cols, last_key} | Portal{name} | Offset{n}, page_size, has_more }`; columnar page buffers (Arrow-like) with a row cap and memory cap.
- **Complexity / performance:** DBeaver default re-read: Σ_{k=1..m} kS = O(m²S) rows for m pages; client-side skip: O(offset) per page. Keyset: O(S·log N) per page with an index. Server-side cursor/portal: O(S) per page, but holds a snapshot/transaction (PG: portal lives inside a transaction; with auto-commit you need `WITH HOLD` cursors or keep a transaction open).
- **Edge cases (exhaustive list):**
  1. Total rows = exact multiple of S → DBeaver shows "more" then fetches 0. Fix: request S+1 and keep S.
  2. Query already contains LIMIT/OFFSET/TOP/FETCH FIRST → do not add another (DBeaver regex is uppercase token-based and misses e.g. `TOP 10`, `FETCH FIRST` without surrounding spaces, or keywords in string literals — false positives/negatives).
  3. `SELECT … FOR UPDATE`, `SELECT INTO`, `INSERT … SELECT`, `WITH … UPDATE … RETURNING` → must not be re-executed for paging (side effects). DBeaver asks for confirmation when a container is marked data-modifying (`verifyQuerySafety`).
  4. Non-deterministic order (no ORDER BY) + offset re-execution → duplicates/missing rows between pages. Reread-from-0 hides this; keyset requires a total order (append PK as tiebreaker).
  5. Data changed between pages → offset drift; cursor/snapshot avoids it.
  6. Volatile functions (`random()`, `now()`) → re-execution changes values.
  7. Multiple result sets / statements producing several results → paging per result.
  8. Very wide rows / LOBs → page by memory, not just row count; fetch LOB prefixes.
  9. Fetch-all on billions of rows → require confirmation and memory cap; offer "export to file" instead.
  10. Cancel while fetching → keep received rows, mark incomplete.
  11. Driver ignores `setMaxRows` → client must stop reading (DBeaver checks `isMaxRowsFetched`).
  12. SQL Server: `setMaxRows` also limits DML counts (reason DBeaver limits only plain SELECT).
  13. Quick filter active → DBeaver blocks next page (filtering would be inconsistent).
  14. Sorting locally vs server mid-pagination → SMART decides by `has_more`.
  15. Row count for complex queries → wrapping can be expensive; make it explicit and cancellable.
  16. Server-side cursors in PG need a transaction; in auto-commit mode use a dedicated read transaction or `WITH HOLD`.
- **Test cases (input → expected):**
  - [Inferred from code] `S=200`, table of 450 rows, reread on: page1 `LIMIT/maxRows 200` → 200 rows, has_more; page2 `offset 0, max 400` → 400 rows (replaced), has_more; page3 `max 600` → 450 rows, has_more=false.
  - Reread off, LIMIT-by-SQL off: page2 → `setMaxRows(400)`, skip 200 via `absolute(200)` → 200 rows appended.
  - LIMIT-by-SQL on, MySQL-style extended limit: `SELECT * FROM t` → `SELECT * FROM t\nLIMIT 200, 200`.
  - Query `SELECT * FROM t LIMIT 10` → not transformed (regex hit); `setMaxRows(200)` still applied.
  - Exactly 200 rows → has_more=true; next fetch returns 0 → has_more=false.
  - Hacivat keyset: `ORDER BY created_at, id`, last `(2026-01-01, 77)` → `WHERE (created_at, id) > ($1,$2) ORDER BY created_at, id LIMIT 201`.
- **Prior art and evidence:** DBeaver cited. DataGrip: explicit page navigation, `database.grid.paging.offset=true`, page size presets/All/Custom, Count Rows on demand, `database.result.set.limit=100` (max result sets), prefetch via fetch size.
- **Recommendation for Hacivat:** Two modes. *Table editor:* keyset paging on the chosen identifier (PK) with S+1 look-ahead, page bar plus infinite scroll, Count on demand. *SQL console:* stream from one execution — PG portal (`Execute` with max rows) / `DECLARE CURSOR` inside the console's transaction, MySQL streaming result, SQLite stepping — and keep the cursor open while the tab is open (with an idle timeout and "cursor closed, re-run?" UX); never re-execute for paging unless the statement is a side-effect-free SELECT. Memory cap per result (e.g. 256 MB) with "export instead" prompt.
- **Used by:** Hacivat; Karagöz (enforce max rows/bytes per request when proxying).

### ALG-DBV-GRID-04: Server-side filter and order rewriting

- **Problem:** Apply grid filters and sorting (column constraints, free WHERE text, ORDER BY) to the server query, for both table browsing and arbitrary SQL results.
- **Inputs / outputs:** Input: base query (table name or SQL text), `DataFilter`, dialect, column bindings. Output: rewritten SQL text (inline literals for SQL results; bind parameters possible for tables).
- **Algorithm (DBeaver, as observed):**
  1. **Condition string** (`appendConditionString`):
     - DNF mode (composite FK navigation): all constraints must be `IN` (not NOT) with equal value counts → `(a = v1 AND b = w1) OR (a = v2 AND b = w2)`; NULL → `IS NULL` (or `= ''` for empty-string-as-null dialects); zero values → `a IS NULL AND b IS NULL`.
     - Otherwise join constraints with AND (or OR if "any"), wrapping each in parentheses when there are ≥2; append free WHERE text as `<op> (where)`.
     - Per constraint: column reference = qualified real column if the binding maps to a table column with the same name, else quoted label/name; dialect type cast may wrap the column. Free criteria: if it does not start with a letter or `= > < !`, prefix `=`. Operator form: NULL value → operator expression for 0-arg operators, else `IS [NOT] NULL`; `=` with array → `= v1 OR col = v2 …`; N-ary `IN` → `IN (v1,…)`, adding `IS NULL OR col` when the list contains NULL, and just `IS NULL` when it contains only NULLs; values inline via `convertValueToSQL(NATIVE)` or `?` with cast. [Observed: `StandardSQLDialectQueryGenerator.java:85-190, 305-461`]
  2. **ORDER BY** (`appendOrderString`): for each ordered constraint, use the column name if it is a simple name (regex for identifiers and dotted composite fields) and not duplicated in the result; else the 1-based column position; then append free ORDER text. [Observed: `StandardSQLDialectQueryGenerator.java:234-303`]
  3. **Table container:** `SELECT … FROM t alias WHERE <cond> ORDER BY <order>` (ALG-03 step).
  4. **SQL query container** (`getQueryWithAppliedFilters`): if "force subselect" pref (default false) → wrap. Else try injection: parse with JSQLParser (comments stripped, dialect line comments masked, bracket quoting and backslash escapes enabled per dialect; 1-second timeout on a daemon pool). Only `PlainSelect` is accepted. For each condition constraint (non-dynamic), find its table in FROM/JOINs and validate that the column exists in that table; set the constraint's alias. If any check fails and the select has no WITH clause → wrap. Combine: `WHERE (original) AND (filter)`; extend ORDER BY with new elements (by position when the name is not simple or duplicated and the dialect supports it; qualified by table only when joins exist). Any exception → wrap. Wrapping: `SELECT * FROM (\n<query>\n) z_q WHERE … ORDER BY …` with labels as column names. [Observed: `StandardSQLDialectQueryGenerator.java:192-232`; `SQLSemanticProcessor.java:82-125, 211-397, 478-512`]
- **Data structures:** Hacivat: `Filter { predicates: Vec<Predicate{col, op, values, negate}>, raw_where: Option<String>, combinator: And|Or, order: Vec<(ColRef, Dir, Nulls)>, raw_order: Option<String> }` rendered by a dialect trait into `(sql, params)`.
- **Complexity / performance:** Parse cost bounded by timeout; wrapping is O(1) but may defeat some planners (most modern planners inline subqueries; MySQL < 5.7 materialised derived tables).
- **Edge cases (exhaustive list):**
  1. Query ends with `;` or trailing comment → strip before wrapping (DBeaver strips comments for parse; wrapping uses the original text — trailing `-- comment` without newline would comment out `) z_q`; DBeaver inserts `\n` before `)` which avoids this).
  2. Original query has ORDER BY + LIMIT → wrapping preserves the limited set (semantically "filter the first N"), injection would change semantics (filter before limit). Neither strategy is universally right; make it explicit.
  3. Duplicate column names in result (`SELECT a.id, b.id`) → wrapped query `SELECT * FROM (…) z_q WHERE id = 1` is ambiguous/invalid; DBeaver orders by position in that case, but WHERE has no position form.
  4. Column aliases equal to other table columns (#6927) → do not qualify with table.
  5. Expressions as columns (`count(*) AS c`) → only filterable through wrapping.
  6. Window functions/aggregates in select → filter must apply after aggregation → wrap (injection into WHERE would be wrong; DBeaver's column-existence validation catches it because `c` is not a table column).
  7. CTE (`WITH`) queries → DBeaver injects into the outer select directly when validation failed but a WITH list exists.
  8. UNION / INTERSECT → not PlainSelect → wrap.
  9. Dialect-specific syntax JSQLParser cannot parse (DuckDB FROM-first, PG `TABLE t`, T-SQL `TOP`, MySQL `#` comments — tested) → wrap.
  10. String literals with backslash escapes (MySQL) vs standard doubling → parser flags (tested).
  11. NULL semantics: `col = NULL` never matches → `IS NULL`; `NOT IN (…, NULL)` is always unknown → must exclude NULLs and add `IS NOT NULL` explicitly (DBeaver handles NULL in IN for the positive case).
  12. Case-insensitive search (`ILIKE`) not available everywhere → `LOWER(col) LIKE LOWER(?)` fallback; Turkish dotless-i issues in `LOWER` depend on collation.
  13. Free WHERE text injection risk → it is the user's own SQL; still, for Karagöz-governed connections apply policy after rendering.
  14. Inline literals for dates/timestamps/binary/JSON need dialect literal syntax (DBeaver uses NATIVE display strings).
  15. Very large IN lists (FK navigation on 10k rows) → batch or use a temporary table/array parameter (`= ANY($1)` in PG).
  16. Ordering by a column with NULLs → engines differ (NULLS FIRST/LAST); expose explicitly.
  17. ORDER BY position is unsupported by some dialects → name required.
  18. Composite/struct fields `(col).field` (PG) and quoting of dotted names.
- **Test cases (input → expected):**
  - DBeaver parser tests (`test/org.jkiss.dbeaver.model.sql.test/.../SQLSemanticProcessorLineCommentsTest.java:80-158`): `SELECT * FROM actor # comment` → source table `actor`; `SELECT * FROM actor #comment` → table `actor`, no alias; `'# not a comment'` preserved; `` `weird#name` `` and `[weird#name]` preserved; `'a\'#b'` preserved; `name = 'O\'Brien' # tail` → WHERE present; `'it''s'` handled; standard dialect unaffected by `--` comments.
  - [Inferred from code] Wrap: query `SELECT a, b FROM t ORDER BY a`, filter `b > 5`, order `a DESC` → `SELECT * FROM (\nSELECT a, b FROM t ORDER BY a\n) z_q WHERE z_q.b > 5 ORDER BY z_q.a DESC` (exact quoting/qualification depends on bindings).
  - Inject: `SELECT * FROM t WHERE x = 1`, filter on table column `y IS NULL` → `SELECT * FROM t WHERE (x = 1) AND (y IS NULL)` (JSQLParser re-prints the statement, so original formatting and comments are lost).
  - IN with NULL: values `[1, NULL, 3]` → `col IS NULL OR col IN (1,3)`.
  - DNF from `DBDReferenceUtilsTest` (37-75): rows `(trip=1,seg=10)`, `(trip=2,seg=20)` → filter has DNF flag, constraints `trip IN [1,2]`, `seg IN [10,20]` → rendered `(trip = 1 AND seg = 10) OR (trip = 2 AND seg = 20)`.
- **Prior art and evidence:** DBeaver cited. DataGrip: server-side WHERE and ORDER BY boxes with history and completion; typed predicates from a dialect PredicatesHelper; query wrapping `SELECT * FROM (q) WHERE … ORDER BY … LIMIT` [I in report].
- **Recommendation for Hacivat:** For SQL results, **wrap by default** (predictable: "filter what you see"), qualify with result *labels*, and refuse WHERE on duplicate labels with a clear message; offer "push down into query" only via a real AST rewrite on our own parser (lossless tree, keeps comments) when the user explicitly edits the query. For tables, render parameterised SQL. Use `= ANY($1)` arrays for PG IN lists; NULL-aware rendering; locale-independent literal rendering.
- **Used by:** Hacivat; Karagöz (policies evaluated on the final rendered SQL).

### ALG-DBV-GRID-05: Column origin resolution for query results

- **Problem:** For an arbitrary SELECT, map each output column to `(schema, table, column)` so that it can be edited, navigated (FK) and formatted with table-level metadata.
- **Inputs / outputs:** Input: driver column metadata (label, name, table/schema/catalog if provided, type), parsed query (select items, FROM tables, `*` position), catalog cache. Output: per column `Origin{table, column}` or `None(reason)`; possibly a changed value handler and converted already-fetched values.
- **Algorithm (DBeaver):** see §3.8 steps 1–8. [Observed: `DBExecUtils.java:759-985`]
- **Data structures:** `ColumnOrigin { range_var: Option<String>, table: TableId, column: ColumnId, confidence: Protocol|Parser|Guess }`.
- **Complexity / performance:** O(columns) lookups in cached catalog; DBeaver preloads all columns of each table once.
- **Edge cases (exhaustive list):**
  1. Driver reports base table for view columns (MySQL) → prefer the view when the query targets a view.
  2. Driver gives no table info (some PG/ODBC drivers, SQLite expressions) → parser fallback only for plain columns.
  3. Alias equal to another table's column name with different type (#PG workaround) → skip.
  4. `SELECT *` mixed with explicit items (`SELECT *, now()`) → ordinal mapping must account for the star's expansion.
  5. Quoted vs unquoted names and server case folding (PG lower, Oracle upper, MySQL OS-dependent).
  6. Same table twice (self join) → resolve by alias.
  7. Subqueries / CTEs → origin through derived tables only with a real resolver.
  8. Columns from functions returning tables (`generate_series`, `unnest`) → no origin.
  9. Value handler changes after binding (string → enum/LOB) → convert fetched rows (DBeaver does).
  10. Temporary tables not in catalog cache → no origin unless catalog refreshed.
- **Test cases (input → expected) [Inferred]:**
  - PG: `SELECT u.id, u.name AS n FROM users u` → protocol RowDescription gives (table OID users, attnum 1/2) → origins users.id, users.name.
  - `SELECT id + 1 FROM users` → table OID 0 → None(Expression).
  - MySQL: `SELECT * FROM v_users` → driver org_table = `users` → DBeaver uses the view entity (query-level).
- **Prior art and evidence:** DBeaver cited; DataGrip report recommends PG `RowDescription` table OID/attnum and MySQL `org_table`, enabling single-table query results to be editable.
- **Recommendation for Hacivat:** Protocol first (PG `RowDescription` table OID + attribute number — exact even through aliases; MySQL `org_table`/`org_name`; SQLite `sqlite3_column_table_name`/`origin_name`; SQL Server `sp_describe_first_result_set` with `@browse_information_mode = 1`), parser second (our resolver from the SQL editor crate), never guess by name alone.
- **Used by:** Hacivat.

### ALG-DBV-GRID-06: Clipboard copy serialisation (plain and advanced)

- **Problem:** Serialise a (possibly sparse) rectangular cell selection to text that spreadsheets and editors parse back correctly.
- **Inputs / outputs:** Input: selected cells in visual order, selected columns, settings (header, row numbers, delimiters, quote string, quote/force quote, display format, HTML). Output: text, optional HTML table, optional raw bytes.
- **Algorithm (DBeaver):** header line = column labels joined by the column delimiter (prefixed with `#` if row numbers). Iterate cells row-major; on a new row emit padding delimiters for skipped trailing columns, the row delimiter and the row number; fill gaps between non-adjacent selected columns with delimiters; render each value with the binding's renderer in the chosen display format; quote if force-quotes, or if quoting is enabled, there is more than one cell, and the text contains the column or row delimiter (no escaping of embedded quotes); HTML escapes text into `<td>`. Cut sets copied editable cells to NULL. [Observed: `SpreadsheetPresentation.java:530-677`]
- **Data structures:** `CopyFormat { col_sep, row_sep, quote, quote_policy: Never|Minimal|Always, header, row_numbers, display: Ui|Edit|Native }`.
- **Complexity:** O(cells).
- **Edge cases (exhaustive list):** embedded quotes (must double → `"a""b"`); embedded tabs/newlines (must quote); leading/trailing spaces (Excel trims unless quoted); NULL vs empty string (need a NULL marker option); values starting with `=`, `+`, `-`, `@` (CSV/formula injection when pasted into spreadsheets — prefix with `'` optionally); very large selections (stream, cap clipboard size); binary values (hex vs base64); locale decimal separator in UI format vs NATIVE; non-rectangular selection; hidden columns; record mode (transposed); multi-line header labels; RTL text; line separator `\r\n` vs `\n` (Excel on Windows expects CRLF).
- **Test cases (input → expected):** [Inferred from code] cells `[["a","b"],["c\td","e"]]`, quote on → `a\tb\n"c\td"\te`; cell `x"y` with force quotes → DBeaver `"x"y"` (defect Q3), correct RFC 4180 output `"x""y"`.
- **Prior art:** DataGrip: extractor-based copy (CSV, TSV, HTML, SQL INSERT/UPDATE, WHERE clause, Markdown, Pretty) with transposed variants; CSV quoting "gerekirse tırnak, tırnak ikileme" (quote when needed, double quotes).
- **Recommendation for Hacivat:** One RFC 4180-compliant codec shared by copy, paste, export and import; formats: TSV (default), CSV, Markdown, JSON, SQL INSERT/UPDATE, WHERE-IN list, HTML; always produce both `text/plain` and `text/html` flavours.
- **Used by:** Hacivat.

### ALG-DBV-GRID-07: Clipboard paste parsing into the grid

- **Problem:** Parse clipboard text into a 2-D array and apply it to a target range, creating rows if needed, converting to column types.
- **Inputs / outputs:** Input: clipboard text, target selection, settings (insert new rows, split rows, ignore quotes, NULL mark), column types/read-only flags. Output: pending edits / new rows; rejected cells.
- **Algorithm (DBeaver):** tokenizer with column delimiter `\t`, row delimiter `\n`, `\r` ignored, quote `"`. A quote at the start of an empty cell starts a quoted cell only if a matching closing quote followed by tab/CR/LF/end exists later; inside, everything up to such a closing quote is literal (including tabs/newlines). If row splitting is off, `\n` behaves like a column delimiter. Final non-empty cell appended. Target: single contiguous range only; empty grid → origin (0,0). With "insert new rows" → insert N new rows at the current position; else append rows at the end while the paste overflows and there is no explicit range end. Fill row by row from the range start column, skipping read-only columns, stopping at the last grid column or the range's right edge (when the range spans >1 column) and at the bottom edge. Each value → NULL if equal (case-insensitive) to the NULL mark, else `getValueFromObject(text, validate=true)`. [Observed: `SpreadsheetPresentation.java:679-944`]
- **Edge cases (exhaustive list):** doubled quotes inside quoted cells (DBeaver keeps both — Q4); trailing empty cell dropped (Q4); `\r\n` and lone `\r` (old Mac) line endings; trailing newline at end (Excel adds one → must not create an empty row); CSV with commas or semicolons (locale) instead of tabs (auto-detect by consistent field count, as DataGrip does); single value pasted over a multi-cell selection (fill all — DBeaver's non-advanced paste does this); more columns than the grid → truncate with warning; paste into read-only columns (DBeaver skips silently — should report); type conversion failures (DataGrip keeps `UnparsedValue`); dates in locale formats; booleans `t/f/yes/no/1/0`; NULL marker vs empty string; very large clipboard (stream); pasting HTML tables from browsers (prefer `text/html` flavour); record mode (transposed target).
- **Test cases (input → expected):** [Inferred from code] `a\tb\nc\td` → `[[a,b],[c,d]]`; `"x\ty"\tz` → `[[x\ty, z]]`; `"a""b"` → DBeaver `a""b` (Hacivat must yield `a"b`); `a\t` → DBeaver `[[a]]` (Hacivat must yield `[[a, ""]]`); `a\nb\n` → `[[a],[b]]` (no third empty row).
- **Prior art:** DataGrip: format auto-detected (consistent column count >1), else single value; per-cell parse, error → Unparsed.
- **Recommendation for Hacivat:** Shared RFC 4180 parser with delimiter sniffing (tab, comma, semicolon, pipe) on the first 10k chars; HTML table parsing when present; per-cell typed parse with an Unparsed state; report skipped cells in a toast.
- **Used by:** Hacivat.

### ALG-DBV-GRID-08: Foreign-key navigation filter construction

- **Problem:** From selected rows, open the referenced (or referencing) table filtered to the related rows.
- **Inputs / outputs:** Input: association (FK) with ordered column pairs, selected rows, value handlers. Output: target table + filter.
- **Algorithm (DBeaver):** validate referenced constraint exists and both sides are referrers with equal column counts; resolve the real target entity (through the virtual model); `use_dnf = columns > 1 && rows > 1`; for each column pair create a constraint on the target column: single row → `= v`; multiple rows → value array with operator IN (if DNF or the handler supports IN) else `=` (rendered as OR chain). Reverse navigation swaps roles. [Observed: `DBDReferenceUtils.java:45-191`]
- **Edge cases:** NULL FK values (no referenced row; exclude from IN); duplicate values across rows (dedupe); composite keys with partial NULLs (MATCH SIMPLE semantics: any NULL → no reference); thousands of selected rows (chunk or array parameter); cross-database virtual FKs (target on another connection); type mismatch between FK and PK columns (cast); missing source column in result (error "Attribute … is missing in result set").
- **Test cases:** DBeaver `DBDReferenceUtilsTest.compositeAssociationNavigationPreservesSelectedKeyPairs` (37-75): FK `(trip_id, segment_id)` → rows `(1,10)`, `(2,20)` → DNF on, constraints `trip_id IN [1,2]`, `segment_id IN [10,20]` (pairs preserved by DNF rendering — **not** the cross product `(1,20)`, `(2,10)`).
- **Prior art:** DataGrip "Related Rows" both directions, `WHERE fk IN (...)`, virtual FK support.
- **Recommendation for Hacivat:** Same semantics; render composite multi-row as row-value IN where supported (`(a,b) IN ((1,10),(2,20))` in PG/MySQL), DNF otherwise; dedupe; skip NULL tuples.
- **Used by:** Hacivat.

### ALG-DBV-GRID-09: Local quick filter and local sort

- **Problem:** Filter/sort the already-loaded rows without a server round trip.
- **Algorithm (DBeaver):** quick filter keeps rows where any column's `toString()` matches the search predicate; new segments are filtered on append; visual numbers renumbered. Local sort: restore physical order, then stable sort by the order constraints using a comparator (value handler comparator if any, else case-insensitive string compare for strings, else generic `compareDataValues`), negated for DESC. [Observed: `ResultSetModel.java:109-146, 909-940, 1254-1300`]
- **Edge cases:** compare displayed text vs raw values (dates/numbers formatted by locale); NULL ordering (first/last, consistent with server); mixed types in one column (JSON, Mongo); natural vs lexicographic order for strings; collation and locale (Turkish `i/İ`, `ı/I`; German `ß`); numeric strings; pending edits during sort (DBeaver drops them — must keep); added rows (keep at their visual place or sort them too, must be explicit); performance on 1M rows (sort indices, not rows; off the UI thread).
- **Test cases [Inferred]:** column values `["b", "A", null, "a"]` ASC with case-insensitive compare → `["A","a","b",null]` or nulls first depending on `compareDataValues` (unverified).
- **Recommendation for Hacivat:** Operate on a permutation vector (DataGrip's ModelIndex/ViewIndex idea), ICU4X collation with the user's locale, explicit NULL placement, per-column type comparators, never discard edits; quick filter on display strings with optional regex and per-column Excel-style value lists.
- **Used by:** Hacivat.

---

## 5. Implications for Hacivat / Karagöz / Perde

**Architecture**
- `grid-core` crate (no UI): `ResultModel { columns: Vec<ColumnMeta>, pages: Vec<ColumnarPage>, view: Permutation, pending: PendingChanges, filter: Filter, cursor: PageCursor }`. UI (Tauri + canvas grid such as Glide, as the DataGrip report suggests) receives pages over IPC.
- `ColumnMeta { label, db_type, logical_type, nullable, origin: Option<ColumnOrigin>, identifier: Option<RowIdentifierRef>, read_only_reason: Option<ReadOnlyReason>, handler: ValueHandlerId }`.
- `Value` enum as in the DataGrip report (Null, Bool, I64, U64, F64, Decimal, Text, Bytes, Uuid, Date, Time, Timestamp(Tz), Interval, Json, Array, Composite, Geometry, Truncated{prefix,len}, Unparsed{text,err}, Default, Error{msg}). DBeaver's `DBDValueError` per cell is worth copying (never fail a whole page for one bad cell).
- Value handlers = trait objects selected by `(dialect, type name/oid, family)` with fallback chain (dialect-specific → family → generic), mirroring DBeaver's provider registry; each handler implements decode, encode (bind + cast), parse(user text, locale), render(Ui|Edit|Native), comparator, supported filter operators.

**Fetching**
- Implement ALG-03 recommendations: S+1 look-ahead, keyset for tables, cursor/portal for console results; memory caps; "fetch all" with confirmation; Count on demand and cancellable.
- One in-flight read per result tab, newest request wins (DBeaver's queue rule), cancel with escalation (driver cancel → close → mark session broken).

**Editing and saving**
- Strict identity by default (ALG-01), fallbacks opt-in and stored in a Hacivat "virtual model" file per connection (keys, FKs, later colour rules), versioned and exportable, English keys.
- Save = one explicit transaction, `rows_affected == 1` enforcement, optional optimistic checks, `RETURNING`/`OUTPUT` reconciliation, DML preview identical to execution.
- Distinguish NULL, DEFAULT and empty string in the editor (DBeaver conflates NULL and DEFAULT on INSERT).
- Keep edits across local sort/filter; keyed by row identity, not by visual index.

**Filtering**
- Typed predicate builder + raw WHERE/ORDER boxes; render via dialect; wrap SQL results by default; PG arrays for IN.

**Karagöz**
- When Hacivat talks through Karagöz, the agent should receive the *structured* DML intent (table, key, changed columns, old/new values) in addition to SQL so that it can enforce column-level write permissions and log a precise audit event; reject all-columns keys and multi-row effects under policy.
- Enforce max rows/bytes per fetch at the agent to protect servers (DBeaver's "fetch all" can pull millions of rows).

**Perde**
- No data-plane role. Store only org-wide defaults (e.g. "confirm data changes on production connections", like DBeaver's connection-type `confirmDataChange`), and policies that Karagöz enforces.

---

## 6. i18n considerations

- **DBeaver approach:** UI strings in Eclipse NLS bundles per package (`ResultSetMessages*.properties`, `SpreadsheetMessages*.properties`, `EditorMessages*.properties`) with locales ar, de, es, fr, it, ja, ko, pl, pt_BR, ro, ru, tw (zh-TW), uk, zh. **No Turkish bundle** exists for the grid. Some user-visible strings are hard-coded English in code (e.g. "Update in progress", "No unique key defined", "You can't perform this operation on a multiple range selection.", "Cancel operation timed out"). [Observed: `ui.editors.data/.../resultset/internal/ResultSetMessages_*.properties`, `spreadsheet/SpreadsheetMessages_*.properties`; `ResultSetModel.java:155-178`; `SpreadsheetPresentation.java:703-706`]
- **Lesson:** every read-only reason, save report, warning and error must be a message key with parameters (ICU MessageFormat / Fluent) — Hacivat's `ReadOnlyReason` enum maps to keys, never to inline English.
- **Number/date formatting must follow the formatter profile's locale**, not the UI language: a Turkish UI user may want `1.234,56` (tr-TR) or ISO formats. DBeaver separates "profile locale" from UI locale — copy that. Default to ISO-8601 for dates/timestamps regardless of locale (DBeaver defaults `yyyy-MM-dd HH:mm:ss.SSS`), locale grouping/decimal for numbers only in UI format, never in NATIVE/SQL or copy-as-SQL.
- **Parsing user input** must accept both the locale format and the canonical format; with tr-TR, `1,5` is 1.5 but `1,500` is ambiguous — show the parsed interpretation in an Unparsed/preview hint.
- **Turkish casing pitfalls:** case-insensitive quick filter, local sort and "NULL mark" comparisons (DBeaver uses `equalsIgnoreCase`/`compareToIgnoreCase`, which are locale-independent in Java but do not implement Turkish rules) — in Rust use ICU4X case folding with the user's locale for *UI-text* matching, and locale-independent ASCII folding for SQL keywords and identifiers (DBeaver correctly uses `toUpperCase(Locale.ENGLISH)` in `QueryTransformerLimit`). Server-side `ILIKE`/`LOWER` behaviour on `İ/ı` depends on the database collation; document it in the filter help.
- **CSV/clipboard delimiters** differ by locale (Excel tr-TR uses `;` for CSV because `,` is the decimal separator); paste sniffing must handle this.
- **Column labels** come from the database and must never be translated; header tooltips (type, origin, read-only reason) are translated.
- **Bidirectional text and Arabic/Hebrew data** in cells: render with proper bidi isolation (DBeaver relies on SWT).

---

## 7. Risks, pitfalls, open questions

**Risks / pitfalls (learned from DBeaver)**
1. Silent overwrites: no optimistic concurrency, no multi-row guard (Q6). Hacivat must ship the guard in v1.
2. Data surprises: NULL becomes DEFAULT on insert (Q7); local sort drops edits (Q12).
3. Performance cliffs: quadratic re-reads (Q8), client-side offset skipping (Q9), full COUNT wrapping on complex queries, unbounded "fetch all".
4. Lossy clipboard (Q3, Q4) and formula injection when pasting to spreadsheets.
5. Filter rewriting changes semantics depending on whether it was injected or wrapped (filter before vs after LIMIT/aggregation) and reformats the user's SQL (JSQLParser re-printing loses comments and layout).
6. Physical row ids (`ctid`) are tempting but unsafe; DBeaver deliberately avoids `ctid`.
7. Key detection on views/joins is heuristic; wrong origins lead to updates of the wrong table. Prefer protocol origins.
8. Preferences that appear unused (`content.cache.*`, `resultset.edit.maxtextsize`, `resultset.binary.stringMaxLength`) — settings that do nothing erode trust; Hacivat should test that every setting has an effect.
9. A 5,761-line viewer class plus a 5,009-line custom grid: feature density in one class makes regressions likely. Split Hacivat's grid into small, testable state machines (fetch, edit, save, filter) with no UI code.
10. Local multi-sort comparator defect (Q1) and composite PK-DESC ordering defect (Q2) show the value of property tests on sort/filter rendering.

**Open questions**
- [Unknown] Whether DBeaver's dictionary/value-filter popup is bounded for huge distinct sets (code suggests paging via `getValues(offset, pageSize)` — `JDBCTable.java:1231`).
- [Unknown] Exact rendering of `DEFAULT_TABLE_ALIAS` and how `DBDValueBinder` casts appear per dialect (would need runtime trace).
- [Unknown] Whether partial/expression unique indexes are excluded by any dialect's `isUnique()` implementation (PostgreSQL index model not checked here; see `dbeaver-02`).
- [Unknown] Real-world frequency of the "phantom has_more" page and of the BETWEEN rendering (Q11) — runtime verification needed.
- For Hacivat: should console results keep a server-side cursor (consistent, but holds locks/snapshots and a transaction) or materialise to a local spill file (DuckDB/Arrow IPC) after the first N pages? Proposed: cursor while the tab is focused and the connection is in a transaction anyway; spill to local Arrow file for very large results on explicit request.
- For Hacivat: how to present "use all columns" safely — require a fresh `SELECT count(*) … WHERE <all columns>` check before each update? Proposed: rely on the `rows_affected == 1` guard inside the transaction and roll back otherwise.
