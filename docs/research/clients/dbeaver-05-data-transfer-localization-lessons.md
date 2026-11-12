# DBeaver Community — Data Transfer, NoSQL Scope, Localization, and Quality Lessons

> Part 5 of the DBeaver deep-source series. Companion documents: `dbeaver-01-architecture-drivers-connections.md`, `dbeaver-02-metadata-and-ddl.md`, `dbeaver-03-sql-editor-parsing-completion.md`, `dbeaver-04-results-viewer-data-editor.md`.
>
> Audience: the Hacivat team. You do not need to be a database expert to read this. Every database term is explained the first time it shows up.

---

## 1. Scope and sources

### 1.1 What this document covers

1. **Data transfer** (scope item 7): how DBeaver moves rows between a database and a file (export), a file and a database (import), and between two databases (database-to-database copy). That includes the pipeline interfaces, every built-in file format, CSV parsing and type guessing, column/type mapping between different database engines, creating target tables, batching and commit intervals, saved "tasks", and the native dump/restore tools (`pg_dump`, `mysqldump`).
2. **NoSQL in the Community edition** (scope item 8): what exists in the open-source build and what DBeaver only sells in its paid editions.
3. **Localization** (scope item 9): how DBeaver translates its UI, which languages ship, how complete they are, whether Turkish exists, how contributors translate, and what goes wrong.
4. **Quality lessons** (scope item 10): known weak points taken from GitHub issues (memory, UI freezes, big schemas, slow exports, driver downloads), with issue numbers.

Algorithm specifications (Section 4):

- ALG-DBV-XFER-01: type mapping between databases
- ALG-DBV-XFER-02: streaming export pipeline (row batching, formatting, file splitting)
- ALG-DBV-XFER-03: CSV import parsing and type inference
- ALG-DBV-XFER-04: database load batching, commit interval, and error recovery (added because it is where most import bugs come from)
- ALG-DBV-I18N-01: message lookup with fallback

### 1.2 Sources

| Source | Version / commit | License | How used |
|---|---|---|---|
| `dbeaver/dbeaver` (DBeaver Community) | `f78ac453859084a4654eac0599fb4c0514ef405b` (2026-11-11, branch `devel`), cloned to `.refs/repos/dbeaver` | Apache-2.0 | Source read only; no build, no scripts run |
| `dbeaver/dbeaver-common` (`org.jkiss.utils`: `CommonUtils`, CSV parser) | `033e46092f3b851e3a23993973d92f80a01e2252` (fetched file-by-file with `gh api`, saved in the session scratchpad) | Apache-2.0 | Read `CommonUtils.findBestCaseAwareMatch`, `csv/CSVParser.java` |
| DBeaver GitHub issues and pull requests | queried 2026-11-11 with `gh issue list` / `gh pr view` | n/a | Issue numbers cited inline |
| dbeaver.com edition comparison page | `https://dbeaver.com/edition/`, fetched 2026-11-11 | proprietary site | Paid-edition feature split |
| Weblate DBeaver project | `https://hosted.weblate.org/engage/dbeaver/` (search result only) | n/a | Translation workflow hint |
| DataGrip teardown notes | `.refs/datagrip-extract/report-*.md` (Turkish, non-decompiling) | DataGrip is proprietary | Behavior comparison only |

Paths below are relative to `.refs/repos/dbeaver/plugins/` unless they start with `test/`. Shorthand used in this document:

- `DT/` = `org.jkiss.dbeaver.data.transfer/src/org/jkiss/dbeaver/tools/transfer/`
- `MODEL/` = `org.jkiss.dbeaver.model/src/org/jkiss/dbeaver/model/`
- `TEST-DT/` = `test/org.jkiss.dbeaver.data.transfer.test/src/org/jkiss/dbeaver/tools/transfer/`
- `TEST-PLAT/` = `test/org.jkiss.dbeaver.test.platform/src/org/jkiss/dbeaver/tools/transfer/`

### 1.3 Clean-room note

DBeaver CE is Apache-2.0, a permissive license. Reusing its code with attribution is legally possible. Even so, this document describes algorithms in prose and our own pseudocode, as the README asks. Where a future implementer might want to port something nearly line by line (for example the CSV formula-escaping rules or the test vectors), the exact file and line range is given so attribution can be done properly.

---

## 2. Executive summary

1. **Everything is a pipe.** A transfer is a list of *pipes*. Each pipe has one *producer* (a source of rows: a table, a query, or a file) and one *consumer* (a destination: a file writer or a database table). A *processor* is the format plug-in (CSV, JSON, XLSX…). These are declared in `plugin.xml` through the `org.jkiss.dbeaver.dataTransfer` extension point. [Observed: `org.jkiss.dbeaver.data.transfer/plugin.xml:5-349`]
2. **Rows stream; they are never all loaded at once (mostly).** The producer runs one `SELECT` and pushes rows to the consumer one at a time through the same callback interface the results grid uses (`fetchStart` → `fetchRow`… → `fetchEnd`). [Observed: `DT/database/DatabaseTransferProducer.java:181-352`]
3. **Exports open a separate connection by default** (`openNewConnections = true`) and switch it to manual commit, because some drivers can only stream large objects or use server-side cursors inside a transaction. [Observed: `DT/database/DatabaseTransferProducer.java:224-262`, `DT/database/DatabaseProducerSettings.java:42-51`]
4. **Defaults:** fetch size 10,000 rows; optional "segments" mode reads 100,000 rows per `LIMIT/OFFSET` query (known to get slow on big MySQL tables, issue #3568); commit every 10,000 rows on import; multi-row `INSERT` batch 500; one worker thread. [Observed: `DT/database/DatabaseProducerSettings.java:42-51`, `DT/database/DatabaseConsumerSettings.java:55-75`, `DT/DataTransferSettings.java:55`]
5. **Ten export formats in CE:** CSV, DbUnit XML, HTML, JSON, Markdown table, SQL `INSERT`, TXT (fixed-width table), XML, "source code" (PHP arrays), and XLSX (from the `data.office` plugin, streaming Apache POI). There is **no Parquet**. [Observed: `org.jkiss.dbeaver.data.transfer/plugin.xml:77-251`, `org.jkiss.dbeaver.data.office/plugin.xml:8-36`]
6. **One import format in CE: CSV.** No XLSX, JSON or Parquet import in Community. [Observed: only `stream.csv` producer processor in `org.jkiss.dbeaver.data.transfer/plugin.xml:39-66`]
7. **CSV export is security-aware:** it prefixes values starting with `=`, `+`, `-`, `@`, tab, CR, LF (and their full-width Unicode lookalikes) with `'` to stop spreadsheet formula injection, and it has a solid test suite. [Observed: `DT/stream/exporter/DataExporterCSV.java:353-367`, `TEST-PLAT/DataExporterCSVTest.java:505-583`]
8. **CSV import guesses column types from the first 100 rows** using a tiny classifier (INTEGER → BIGINT → REAL, BOOLEAN, VARCHAR/NVARCHAR). It does **not** detect dates, and the result depends on row order in a few cases. [Observed: `DT/stream/importer/DataImporterCSV.java:75-160`, `DT/database/DatabaseTransferUtils.java:710-754`, `DT/stream/StreamDataImporterColumnInfo.java:52-141`]
9. **Type mapping between engines** is a layered heuristic: same driver class → copy the type verbatim; else ask the target dialect's converter (only PostgreSQL and Oracle have meaningful ones, plus a generic "VARCHAR without length" rule); else look up the same type name in the target's type list; else search for a "similar" type by data kind, scale and name substrings; else use the target's default type for that kind. [Observed: `MODEL/struct/DBStructUtils.java:387-567`]
10. **Target tables are created through the same object editors the UI uses** ("struct DDL"), with a fallback to hand-built `CREATE TABLE`. The fallback adds a `PRIMARY KEY` copied from the source; the struct path does not do that itself. [Observed: `DT/database/DatabaseTransferUtils.java:184-315, 401-540`]
11. **Batch size equals commit interval.** Rows are buffered in a JDBC batch and executed only when `rowsExported % commitAfterRows == 0` (or at the end). A failure prompts Stop / Retry / Ignore / Ignore All for the whole batch. [Observed: `DT/database/DatabaseTransferConsumer.java:408-505`]
12. **Tasks** (saved, re-runnable export/import/script jobs) are stored per project in `tasks.json`. The scheduler interface exists in CE, but no scheduler implementation is contributed; scheduling is a paid feature (Enterprise/Ultimate). [Observed: `org.jkiss.dbeaver.registry/src/org/jkiss/dbeaver/registry/task/TaskConstants.java:22`, `MODEL/task/DBTTaskScheduleConfiguration.java`; Observed: dbeaver.com/edition]
13. **Native dump tools** are wrapped by launching `pg_dump`/`pg_restore`/`psql`/`pg_dumpall` or `mysqldump`/`mysql` as child processes; the password is passed through `PGPASSWORD` / `MYSQL_PWD` environment variables or a temporary `.my.cnf`. [Observed: `org.jkiss.dbeaver.ext.postgresql/src/.../tasks/PostgreNativeToolHandler.java:40-80`, `org.jkiss.dbeaver.ext.mysql/src/.../tasks/MySQLNativeToolHandler.java:62-122`]
14. **NoSQL in CE is essentially absent.** MongoDB, Cassandra, Couchbase, Redis, Timestream, Neo4j and Salesforce appear in the driver list only as "not available in Community" stubs that advertise PRO editions. Elasticsearch works only through its SQL JDBC driver (relational view). [Observed: `org.jkiss.dbeaver.ext.generic/plugin.xml:1762-1832`, `.../OSGI-INF/l10n/bundle.properties:122-151`]
15. **Localization uses Eclipse NLS** (`*Messages.java` classes with one static field per key, backed by `*.properties`) plus OSGi `plugin.xml` `%key` lookups (`OSGI-INF/l10n/bundle_*.properties`). [Observed: `org.jkiss.dbeaver.core/src/org/jkiss/dbeaver/core/CoreMessages.java:1-412`, 52 `MANIFEST.MF` files with `Bundle-Localization`]
16. **15 UI languages, no Turkish.** en, fr, de, it, pl, ja, ko, pt_BR, ro, ru, zh, es, tw (Traditional Chinese, using a non-standard `_tw` suffix), uk, ar. About 12,000 English keys in ~213 resource families; every shipped locale has 100% key coverage as of 2026-11-10 thanks to a bulk "synchronize localizations" PR (#42805), with AI-assisted translation openly acknowledged (#42543). [Observed: `org.jkiss.dbeaver.registry/plugin.xml:28-44`; own count script over the repo]
17. **Changing language requires a restart**, because the locale is applied at JVM launch (`osgi.nl`). [Observed: `org.jkiss.dbeaver.launcher/src/.../DBeaverLauncher.java:2279-2291`, `org.jkiss.dbeaver.core/src/.../PrefPageGeneral.java:246-254`]
18. **Recurring weak points** in issues: OutOfMemory on "fetch all" and big imports (#34854, #11931, #17099), opening an 800 MB SQL file (#42570), UI freezes and deadlocks during startup or metadata loading (#42803, #42205, #9699, #35261), navigator slowness with 20,000+ objects (#39861, #11555), slow DDL generation on large schemas (#8473), `LIMIT offset` exports (#3568), CSV import type problems (#37855, #39423, #22483), and fragile driver downloads (#40237, #34758, #35466).
19. **Recommendation for Hacivat:** keep DBeaver's producer/consumer/processor split but make it async and back-pressured (Rust channels with bounded capacity), use keyset pagination instead of `OFFSET`, separate batch size from commit interval, give type mapping an explicit, testable rule table per engine pair, and use Project Fluent (`.ftl` files) for i18n so adding Turkish or any other language is a file drop, not a code change.

---

## 3. Detailed findings

### 3.1 Plugin layout for data transfer

| Bundle | Role | Notes |
|---|---|---|
| `org.jkiss.dbeaver.data.transfer` | Model layer: pipeline, producers, consumers, CSV importer, all text exporters, tasks `dataExport`/`dataImport`/`scriptExecute` | 17,241 lines of Java [Observed: `wc -l`] |
| `org.jkiss.dbeaver.data.transfer.ui` | Wizard pages: input objects, extract settings, output settings, mapping, load settings, preview | 45 Java files [Observed] |
| `org.jkiss.dbeaver.data.office` (+ `.ui`) | XLSX exporter (Apache POI streaming `SXSSFWorkbook`) | [Observed: `org.jkiss.dbeaver.data.office/src/.../export/DataExporterXLSX.java:21-22`] |
| `org.jkiss.dbeaver.tasks.native` (+ `.ui`) | Base classes to run external command-line tools as tasks | [Observed: `org.jkiss.dbeaver.tasks.native/src/.../AbstractNativeToolHandler.java`] |
| `org.jkiss.dbeaver.ext.postgresql`, `ext.mysql` | Concrete `pg_dump`/`mysqldump` handlers | [Observed: `.../ext/postgresql/tasks/*`, `.../ext/mysql/tasks/*`] |
| `org.jkiss.dbeaver.registry` | Task registry, `tasks.json` persistence | [Observed: `org.jkiss.dbeaver.registry/src/.../task/TaskManagerImpl.java:492-537`] |

This matches DBeaver's general rule that model bundles must not depend on UI bundles (`AGENTS-Architecture.md`). [Observed: repo root `AGENTS-Architecture.md`]

### 3.2 The pipeline model (producer → processor → consumer)

Key interfaces, all in `DT/`:

| Interface | What it is, in plain words | Key methods |
|---|---|---|
| `IDataTransferNode` | Anything that sits at one end of a pipe. Has a name, icon, "container" name (for example the connection name or output folder), and says whether its configuration is complete. | `getObjectName`, `getObjectFullName`, `isConfigurationComplete` |
| `IDataTransferProducer` | Source of rows. One method does the whole job: read data and feed it to the consumer. | `transferData(monitor, consumer, processor, settings, task, maxRows)` [Observed: `DT/IDataTransferProducer.java`] |
| `IDataTransferConsumer` | Destination. Also implements `DBDDataReceiver`, the same row-callback interface the results grid uses, so any code that can fill a grid can also fill an export. | `initTransfer`, `startTransfer`, `fetchStart`, `fetchRow`, `fetchEnd`, `finishTransfer(monitor, error, task, last)` [Observed: `DT/IDataTransferConsumer.java`] |
| `IDataTransferProcessor` | Marker interface for a format plug-in. | — |
| `IStreamDataExporter` | A processor that writes a file format. | `init(site)`, `exportHeader`, `exportRow(session, resultSet, row[])`, `exportFooter`, `dispose` [Observed: `DT/stream/IStreamDataExporter.java`] |
| `IStreamDataImporter` | A processor that reads a file format. | `readColumnsInfo(mapping, inputStream)` (for preview and type guessing), `runImport(monitor, streamDataSource, inputStream, consumer)` [Observed: `DT/stream/IStreamDataImporter.java`] |
| `IAppendableDataExporter` | Exporter that can append to an existing file. | `importData(site)`, `shouldTruncateOutputFileBeforeExport()` |
| `IDataTransferEventProcessor` | Hook run on START/FINISH or error: "show in file explorer", "execute shell command", "delete failed export files". | `processEvent`, `processError` [Observed: `DT/IDataTransferEventProcessor.java`, `plugin.xml:272-290`] |
| `IDataTransferAttributeTransformer` | Per-column value rewrite during database-to-database copy: set NULL, set constant, evaluate a JEXL expression. | `transformAttribute(...)` [Observed: `DT/IDataTransferAttributeTransformer.java`, `plugin.xml:257-270`] |

Registered nodes [Observed: `org.jkiss.dbeaver.data.transfer/plugin.xml:7-252`]:

| Node id | Type | Class | Works with |
|---|---|---|---|
| `database_producer` | producer | `DatabaseTransferProducer` | any `DBSDataContainer` (table, view, query result) |
| `database_consumer` | consumer | `DatabaseTransferConsumer` | any `DBSDataContainer` that can accept inserts |
| `stream_producer` | producer | `StreamTransferProducer` | file input; processor `stream.csv` (CSV importer) |
| `stream_consumer` | consumer | `StreamTransferConsumer` | file output; processors listed in 3.5 |

So the four combinations are: table → file (export), file → table (import), table → table (database copy, also called "migration" in the wizard), and, in theory, file → file (not offered in the UI). [Inferred: from the node types; the UI wizard only offers the first three]

**Execution.** `DataTransferSettings` holds the list of pipes. `DTTaskHandlerTransfer` starts `min(pipeCount, maxJobCount)` `DataTransferJob`s (an Eclipse background job each), grouped with a `JobGroup` when more than one. Each job loops: `acquireDataPipe()` (a `synchronized` "give me the next pipe index" counter) → `transferData(pipe)` → repeat until no pipes left or cancelled. [Observed: `DT/task/DTTaskHandlerTransfer.java:177-192, 268-376`; `DT/DataTransferJob.java:83-125`; `DT/DataTransferSettings.java:635-644`]

`maxJobCount` defaults to 1 (sequential). [Observed: `DT/DataTransferSettings.java:55, 78, 316`]

Before execution, pipes from a database source can be sorted in dependency order: plain tables first (parents before children by foreign key), then tables in foreign-key cycles, then views, using `DBStructUtils.sortTableList`. [Observed: `DT/DataTransferSettings.java:567-624`] The comparator is marked `//fixme rewrite` in the source and mixes indexes from three different lists, so the order is only reliable within one category. [Observed: `DT/DataTransferSettings.java:584`; Inferred: comparator returns `idx1 - idx2` where one index may be `-1`]. With `maxJobCount > 1` the dependency order is not enforced at runtime at all, because workers grab pipes concurrently. [Inferred: `acquireDataPipe` only hands out the next index; nothing waits for parent tables to finish]

**Error and cancel handling in the job.** On an exception the job returns an *OK* status carrying the exception ("Report as an OK status to avoid showing the error in the UI (it's handled by the caller)"); on cancel it returns `CANCEL_STATUS`. The producer is always closed in a `finally`. [Observed: `DT/DataTransferJob.java:108-124, 143-186`] DBeaver has unit tests for exactly these guarantees (`returnsCanceledStatusBeforeAcquiringPipe`, `closesProducerEvenWhenTransferFails`, `closeFailureDoesNotMaskTransferFailure`…). [Observed: `TEST-DT/DataTransferJobTest.java:65-205`, `TEST-DT/task/DTTaskHandlerTransferTest.java:99-170`]

### 3.3 The database producer (reading rows out of a database)

`DatabaseTransferProducer.transferData` [Observed: `DT/database/DatabaseTransferProducer.java:181-352`] does the following, in order:

1. Wraps everything in `DBExecUtils.tryExecuteRecover`, which retries once after reconnecting if the connection was lost. [Observed: line 198; Inferred: semantics from name and its use elsewhere]
2. Computes read flags. If the export was started from a results grid with "selected rows/columns only", it sets `FLAG_USE_FETCHED_ROWS` / `FLAG_USE_SELECTED_ROWS` / `FLAG_USE_SELECTED_COLUMNS` and exports what is already in memory instead of re-querying. [Observed: lines 203-214]
3. Decides whether to open a **separate connection** (`openNewConnections`, default `true`, ignored for embedded drivers like SQLite/H2). A separate connection means a long export does not block the user's SQL editor connection. [Observed: lines 216-238; default in `DatabaseProducerSettings.java:47`]
4. If it opened a new connection, or if the driver declares `FEATURE_LOB_REQUIRE_TRANSACTIONS`, it turns **auto-commit off** and sets a savepoint. The source comment says some drivers can read LOBs (large objects) only inside a transaction. [Observed: lines 245-262] A second reason, not stated in the comment: the PostgreSQL JDBC driver only honours fetch size (streams rows in chunks instead of loading the whole result) when auto-commit is off. [Inferred: well-known PgJDBC behavior; matches DataGrip's "fetch size" notes]
5. Optionally runs `SELECT COUNT(*)` first (`queryRowCount`, default `true`) so the progress bar has a total. If counting fails, it rolls back (PostgreSQL aborts the whole transaction after any error) and continues. [Observed: lines 264-285]
6. Reads data in one of two modes:
   - **Single query** (default): one `readData(... offset=-1, maxRows, fetchSize)` call. [Observed: lines 293-309]
   - **Segments**: a loop of `readData(offset, segmentSize)` with `offset += rowsFetched` until a segment returns fewer rows than `segmentSize` (default 100,000). [Observed: lines 310-328] This generates `LIMIT n OFFSET m` queries whose cost grows with `m`; issue #3568 shows MySQL timing out at offset 650,000. A side detail: the last segment's statistics are not added to the totals because the loop breaks before `accumulate`. [Observed: lines 320-326]
7. `finally`: roll back the read transaction, release the savepoint, restore auto-commit, close the isolated connection. [Observed: lines 332-349]

Settings defaults [Observed: `DT/database/DatabaseProducerSettings.java:35-51`]: `segmentSize=100000`, `fetchSize=10000`, `openNewConnections=true`, `queryRowCount=true`, `extractType=SINGLE_QUERY`.

Tasks serialize the producer as either `{type: "entity", project, entityId}` or `{type: "query", project, dataSource, query, defaultCatalog, defaultSchema}` plus an optional `dataFilter`. [Observed: `DT/database/DatabaseTransferProducer.java:374-418`]

### 3.4 The stream consumer (writing files)

`StreamTransferConsumer` is format-agnostic; the processor (exporter) does the formatting. [Observed: `DT/stream/StreamTransferConsumer.java`]

**Lifecycle**

- `initTransfer` resets per-source state (`initialized=false`, `firstRow=true`, file counter, byte counter). [Observed: lines 590-620]
- `fetchStart` (can be called several times in segment mode) lazily opens the output and calls `processor.init(site)`; computes column bindings; for "document" exporters (JSON of Mongo-style documents) keeps nested bindings, otherwise flattens nested attributes into leaf columns (`DTUtils.makeLeafAttributeBindings`); filters out columns the user unchecked; writes the header once. [Observed: lines 159-192]
- `fetchRow` (per row): checks split condition, fetches values, handles LOBs, calls `processor.exportRow`. [Observed: lines 195-247]
- `finishTransfer(last=false)` writes the footer and closes the file; `finishTransfer(last=true)` copies to clipboard if needed and runs event processors. [Observed: lines 638-720]

**Output file name** comes from a pattern with `${variables}`: `datasource`, `catalog`, `schema`, `table`, `timestamp`, `date`, `index`, `project`, `connectionType`, `scriptFilename`, `year`, `month`, `day`, `hour`, `minute`. Default pattern: `${table}_${timestamp}`; default folder: the user's home; default encoding: the platform default. [Observed: lines 87-117; `DT/stream/StreamConsumerSettings.java:121-139`] Object names are sanitized: letters and digits kept, every other character replaced by `_` with runs collapsed, truncated to 64 characters. [Observed: `StreamTransferConsumer.java:971-990`] When the folder is not writable, a fallback folder (system temp) is used. [Observed: lines 822-840; `DT/DTConstants.java:33-34`]

**Existing file conflicts**: ASK (default), APPEND (only for exporters that support it, otherwise downgraded to a renamed file), PATCHNAME (append `-<millis>`), OVERWRITE. "Write to single file" forces APPEND for the second and later sources. [Observed: lines 421-479]

**Stream stack** (outermost first) [Observed: lines 481-546]:

```
PrintWriter(OutputStreamWriter(charset))         — text exporters only
  [OutputStreamStatProxy]                        — only if split is on; counts bytes
    [ZipOutputStream, one entry]                 — only if "compress" is on
      StatOutputStream                           — total bytes for statistics
        BufferedOutputStream(100,000 bytes)
          FileOutputStream / caller's stream
```

A BOM (byte-order mark) is written first if requested. [Observed: lines 533-541]

**File splitting**: before writing any row except the first, if splitting is on and the output is text, the writer is flushed and if `bytesWritten >= maxOutFileSize` (default 10,000,000 bytes) the footer is written, the file closed, a new file `<name>_2.<ext>` (then `_3`, …) opened and the header written again. [Observed: lines 198-207, 581-590, 776-805; default `StreamConsumerSettings.java:139`] Because the byte counter sits *above* the ZIP stream, the limit is measured on uncompressed bytes even when compressing. [Inferred: stream order above] Binary exporters (XLSX) never split by bytes; XLSX has its own row-based sheet split.

**LOB handling** (binary large objects, e.g. images): per setting `SKIP` (write NULL), `INLINE` (default; pass to exporter, which may base64/hex it), or `FILES` (save each LOB to `<output>/files/<outputfile>-<n>.data` or `.jpg` and pass the path). Text LOBs (CLOB) are always inline. [Observed: lines 213-240, 316-356; enums in `StreamConsumerSettings.java:76-88`]

**Clipboard mode** writes into a `StringWriter` and copies at the end (optionally as HTML). [Observed: lines 365-374, 666-672]

### 3.5 Export formats in detail

All exporters extend `StreamExporterAbstract`, which provides the writer, the column list and `getValueDisplayString(column, value)` (formatting through the user's data-format profile). [Observed: `DT/stream/exporter/StreamExporterAbstract.java`]

| Processor id | Class | Important options (default) | Behavior worth knowing |
|---|---|---|---|
| `stream.csv` | `DataExporterCSV` | delimiter `,`; rowDelimiter `default` (OS line separator); header `top`/`none`/`bottom`/`both`; headerFormat `label`/`description`/`both`; headerQuoting `always`; quoteChar `"`; quoteAlways `disabled`/`all`/`strings`/`all but numbers`/`all including nulls`; quoteNever; escapeFormulas `true`; nullString empty; formatNumbers `false`; lineFeedEscapeString; formatArray `{ }` | Details in ALG-DBV-XFER-02. Doubles the quote character to escape it; multi-character quotes are not escaped. Refuses a delimiter that equals or starts with the quote. [Observed: `plugin.xml:143-168`; `DataExporterCSV.java:115-180, 234-420`] |
| `stream.json` | `DataExporterJSON` | printTableName `true`; formatDateISO `true`; exportJsonValues (as string or as raw JSON) | Writes `[ {"col": value, …}, … ]`, optionally wrapped in `{"table": [...]}`. Numbers and booleans are written with `toString()`, so a floating-point `NaN` or `Infinity` would produce invalid JSON. Single-column document results are serialized natively. [Observed: `DataExporterJSON.java` `exportHeader`…`exportRow`; Inferred: NaN consequence] |
| `stream.xml` | `DataExporterXML` | includeDoctype `false` | Tested in `TEST-DT/stream/exporter/DataExporterXMLTest.java`. |
| `stream.dbunit` | `DataExporterDbUnit` | upper-case table/column names; includeNullValues; nullValueString `[NULL]` | DbUnit dataset XML for Java test fixtures. Tested (`DataExporterDbUnitExpertRowTest.java`). |
| `stream.html` | `DataExporterHTML` | tableHeader, columnHeaders, extractImages | Images can be embedded. |
| `stream.markdown.table` | `DataExporterMarkdownTable` | nullString, formatNumbers, showHeaderSeparator, confluenceFormat, escapeCellContent | Line breaks become `<br>`, `|` is escaped; with escaping on, values are wrapped in backticks with the fence length chosen to exceed the longest backtick run inside the value. 30+ tests. [Observed: `TEST-DT/stream/exporter/DataExporterMarkdownTableTest.java:86-325`] |
| `stream.sql` | `DataExporterSQL` | rowsInStatement `10`; nativeFormat `true`; omitSchema; keywordCase `upper`; identifierCase `as is`; upsertKeyword `INSERT`/`INSERT ALL`/`UPDATE OR`/`UPSERT INTO`/`REPLACE INTO`/`ON DUPLICATE KEY UPDATE`/`ON CONFLICT`; insertOnConflict text; includeAutoGenerated `false` | Generates multi-row `INSERT … VALUES (…),(…);` groups of N rows. Values are rendered with `SQLUtils.convertValueToSQL` in the *source* dialect's native format. If the dialect does not support grouped rows and "use current dialect settings" is set, falls back to one row per statement. [Observed: `plugin.xml:187-209`; `DataExporterSQL.java:140-160, 222-400, 450-463`] |
| `stream.txt` | `DataExporterTXT` | batchSize `200` (minimum 200); min/max column length; showNulls; delimiters | Pretty ASCII table. Buffers up to 200 rows to compute column widths per batch, so widths can change between batches. [Observed: `DataExporterTXT.java:161-222, 355-380`; Inferred: width variation] |
| `stream.source.code` | `DataExporterSourceCode` | language `PHP < 5.4` / `PHP 5.4+` | Emits PHP array literals. |
| `stream.xlsx` | `DataExporterXLSX` (data.office) | rownumber, border, header font, true/false strings, trimString, exportSql (put the query in a sheet), splitByRowCount `1,048,575`, splitByColNum, dateFormat `MM/dd/yy`, appendStrategy, sheet name pattern | Uses POI `SXSSFWorkbook(100)`: only 100 rows are kept in memory, older rows are flushed to a temp file. Cells longer than 32,767 characters are truncated with a log warning. A new sheet is started when the row limit is reached or, with "split by column", for every distinct value of that column. [Observed: `org.jkiss.dbeaver.data.office/plugin.xml:8-36`; `DataExporterXLSX.java:63-120, 173, 430-445, 584-595`] |

Not present in CE: Parquet, Avro, ORC, XLSX import, JSON import. [Observed: no matches for "parquet" in any `plugin.xml` or Java file under `plugins/`]

### 3.6 Import (file → database)

**Stream producer and fake data source.** To reuse the database-to-database code path, the file is presented as a fake database: `StreamDataSource` / `StreamDataSourceContainer` / `StreamTransferSession` / `StreamTransferResultSet`, registered as data source provider `stream` with the `basic` dialect. A local statement `SELECT * FROM Stream` "executes" by reading the file. [Observed: `org.jkiss.dbeaver.data.transfer/plugin.xml:345-347`; `DT/stream/model/*`; `DT/stream/importer/DataImporterCSV.java:215-220`] This is a nice trick: the database consumer does not know the source is a file.

**CSV importer options** [Observed: `org.jkiss.dbeaver.data.transfer/plugin.xml:39-66`]: extension(s) `csv,tsv,txt`; encoding `utf-8`; delimiter `,`; header `top`/`none`; quoteChar `"`; strictQuotes `false`; escapeChar `\`; nullString empty; emptyStringNull; timestampFormat `yyyy-MM-dd[ HH:mm:ss[.SSS]]`; timestampZone; trimWhitespaces; and type sampling: `columnTypeSamplesCount=100`, `columnTypeMinimalLength=50`, `columnTypeIsByteLength`.

**Parser.** `org.jkiss.utils.csv.CSVParser` is a DBeaver fork of OpenCSV that supports multi-character separators/quotes/escapes (it matches the longest special token first). Rules: separator, quote and escape must be non-empty and different; a doubled quote inside a quoted field is a literal quote (RFC 4180); an escape inside quotes escapes the next quote or escape, otherwise it is literal; outside quotes the escape character is literal; a quoted field may span lines and the line break is re-inserted as `\n` (so CRLF inside a field becomes LF); an unterminated quote at end of file is an error. [Observed: `dbeaver-common/.../csv/CSVParser.java` constructor and `parseLine`, `writeEscape`, `writeQuotes`; `DataImporterCSV.java:236-241`] The "escape" default `\` means a value like `C:\temp\"x"` behaves differently from strict RFC 4180, which is why users asked for a real RFC 4180 mode (#22483). [Observed: issue #22483]

Type inference is ALG-DBV-XFER-03 below.

**Row loop** [Observed: `DataImporterCSV.java:204-302`]: skip empty lines; skip the header line; stop at `maxRows`; pad short rows with NULLs (long rows keep extra values, which the mapping ignores); optional trim; optional empty-string-to-NULL; optional custom NULL marker; push the row to the consumer; update progress every N rows.

**Date/time conversion** happens lazily in `StreamTransferResultSet.getAttributeValue`, and only when the *target* column is a date/time: parse with the configured `DateTimeFormatter`; if the parsed value contains a zone/offset convert to an instant; otherwise create a local timestamp (date-only → midnight) and optionally shift it to the configured zone; on parse failure pass the string through unchanged and let the database decide. [Observed: `DT/stream/StreamTransferResultSet.java:92-145`; `DT/stream/importer/StreamImporterAbstract.java:87-130`] Tested: `TEST-DT/stream/StreamTransferResultSetTest.java:43-82` (see test cases in ALG-DBV-XFER-03).

### 3.7 The database consumer (writing rows into a table)

**Mapping model.** For each source there is a `DatabaseMappingContainer` (table level) with `DatabaseMappingAttribute`s (column level). Each has a mapping type [Observed: `DT/database/DatabaseMappingType.java`]:

| Mapping type | Meaning |
|---|---|
| `existing` | Target table/column already exists; insert into it. |
| `create` | Target does not exist; create it. |
| `recreate` | Drop the target table and create it again (table level). |
| `skip` | Ignore this table/column. |
| `unspecified` | Not decided yet (invalid; wizard won't proceed). |

**Automatic column matching** (`DatabaseMappingAttribute.updateMappingType`, [Observed: `DT/database/DatabaseMappingAttribute.java:162-268`]):

1. If the column is already `skip`, or the table is `skip`, keep `skip`.
2. If the target table exists: compute a target name from the source label (query alias) or name, transformed to the target's identifier case; reload the target's columns if the cache is empty; find the best match with `findBestCaseAwareMatch` — the first exact (case-sensitive) match wins, else the first case-insensitive match. [Observed: `dbeaver-common/.../CommonUtils.java:1061-1074`]
3. For CSV files *without* a header row, match by position among visible, non-pseudo target columns sorted by ordinal position, but prefer a name match if one exists.
4. Found → `existing` (or `create` if the table is being recreated); not found → `create`.
5. If the target table does not exist → every column is `create`.

Name transformation for new objects (`DatabaseTransferUtils.getTransformedName`, [Observed: lines 321-351]) applies user preferences: name case DEFAULT/UPPER/LOWER, and space replacement ABSENT/UNDERSCORES/CAMELCASE; then quotes the result if the target dialect requires it. Default: dialect's own case rules (e.g. lower-case in PostgreSQL, upper-case in Oracle), spaces kept, quoted. [Observed: `DT/database/MappingNameCase.java:27-29`, `MappingReplaceMechanism.java:26-28`, `DT/internal/DataTransferPreferencesInitializer.java:31-39`]

**Target table creation** (`startTransfer` → `createTargetDatabaseObjects` → `createTargetTable`, [Observed: `DT/database/DatabaseTransferConsumer.java:644-745`]):

- Temporarily switches the session's default catalog/schema if the target lives elsewhere, and restores it afterwards.
- For document databases (dynamic metadata) it calls a special creator; otherwise it generates DDL with `generateTargetTableDDL`.
- `generateTargetTableDDL` [Observed: `DT/database/DatabaseTransferUtils.java:184-315`]:
  - Existing table with no `create` columns → no DDL.
  - Preferred path ("struct DDL", `USE_STRUCT_DDL = true`): use the same `SQLObjectEditor` the table designer uses: create a new table object, add a column object per `create` mapping with name, full type (with modifiers like `(255)` or `(10,2)`), length/precision/scale/NOT NULL copied from source if the type string had no modifiers, collation if applicable, comments copied from the source, then ask the editor for the `CREATE TABLE` / `ALTER TABLE ADD` script. [Observed: lines 401-540]
  - If that fails (no object manager for this driver), the user is asked whether to fall back to a hand-built script: `DROP TABLE` (for recreate), `CREATE TABLE t (\n\tcol type, …` and a `PRIMARY KEY (…)` taken from the source's best identifier if all its columns are mapped; or one `ALTER TABLE … ADD` per new column. [Observed: lines 226-313]
- The DDL is executed, and on failure the error message includes the full generated script, which helps users fix it manually. [Observed: `DatabaseTransferConsumer.java:737-745`]
- Permission check: a connection can forbid metadata edits or data import ("restricted by connection configuration"). [Observed: `DatabaseTransferConsumer.java:542-547`; `DatabaseTransferUtils.java:756-760`]

**Loading rows** is ALG-DBV-XFER-04. Settings and defaults [Observed: `DT/database/DatabaseConsumerSettings.java:55-75`]:

| Setting | Default | Effect |
|---|---|---|
| `openNewConnections` | true | Separate connection for the load. |
| `useTransactions` | true | Manual commit mode; commit every N rows. |
| `commitAfterRows` | 10,000 | Commit interval *and* batch flush interval. |
| `transferAutoGeneratedColumns` | true | Copy identity/serial columns too. Issue #35197 says unchecking it was not honoured in some flow. |
| `truncateBeforeLoad` | false | `TRUNCATE` (or `DELETE`) target first; only for `existing` tables and only at offset 0. [Observed: `DatabaseTransferConsumer.java:187-197`] |
| `useMultiRowInsert` / `multiRowInsertBatch` | false / 500 | Emit `INSERT … VALUES (…),(…)` with 500 rows per statement instead of a JDBC batch. |
| `skipBindValues` | false | Inline literals instead of `?` parameters. |
| `disableUsingBatches` | false | Execute every row immediately (slow but precise error reporting). |
| `ignoreDuplicateRows` | false | Swallow unique-key violations (forces commit after every row). |
| `useBulkLoad` | false | Use a database-specific bulk loader if the driver provides one (e.g. PostgreSQL `COPY`: `test/org.jkiss.dbeaver.ext.postgresql.test/.../PostgreCopyLoaderTest.java`). |
| `onDuplicateKeyInsertMethodId` | none | Dialect-specific upsert method (`ON CONFLICT`, `REPLACE`, …). |
| `disableReferentialIntegrity` | false | Turn off FK checks during load if the target supports it (`DBPReferentialIntegrityController`). [Observed: `DatabaseTransferConsumer.java:969-990`] |
| `enableQmLogging` | false | Whether every insert shows up in the Query Manager log. |
| `openTableOnFinish` | true | Open the table editor afterwards. |

**Preview.** The wizard can run the pipeline in preview mode: a `PreviewBatch` collects rows instead of inserting. [Observed: `DatabaseTransferConsumer.java:310-315, 1002-1025`; UI `org.jkiss.dbeaver.data.transfer.ui/.../pages/database/PreviewMappingDialog.java`]

### 3.8 Type mapping between databases (overview)

When the target column must be created, the target type comes from `DatabaseMappingAttribute.getTargetType` → `DBStructUtils.mapTargetDataType(targetContainer, sourceAttribute, addModifiers)`. The user can override it in the mapping page. Full algorithm: ALG-DBV-XFER-01.

Per-engine fallbacks ("default type for a data kind") [Observed]:

| Data kind | Generic JDBC (`JDBCDataSource.java:812`) | PostgreSQL (`PostgreUtils.java:103`) | MySQL (`MySQLDataSource.java:802`) | SQL Server (`SQLServerDataSource.java:492`) |
|---|---|---|---|---|
| BOOLEAN | standard SQL name, else first type of that kind | `bool` | `TINYINT(1)` | `bit` |
| NUMERIC | 〃 | `int` | `BIGINT` | `int` |
| STRING | 〃 | `varchar` | `VARCHAR` | `varchar` |
| DATETIME | 〃 | `timestamp` | `TIMESTAMP` | `datetime` |
| BINARY | 〃 | `bytea` | `BINARY` | `varbinary` |
| CONTENT (LOB) | 〃 | `bytea` | `LONGBLOB` | `varbinary` |
| ROWID | 〃 | `oid` | `BINARY` | `uniqueidentifier` |

Note how "NUMERIC → int" in PostgreSQL and SQL Server silently loses decimals if the earlier steps did not find a better match. That is the main risk of the fallback.

Dialect converters (`SQLDataTypeConverter.convertExternalDataType`) exist only for:

- PostgreSQL target: `xml/xmltype/sys.xmltype → xml`, `varchar2/nchar/nvarchar → varchar(len)`, `json/jsonb → jsonb`, `geometry/sdo_geometry → geometry`, `number(p,s) → numeric(p,s)`. [Observed: `org.jkiss.dbeaver.ext.postgresql/src/.../model/PostgreDialect.java:1184-1245`]
- Oracle target: `VARCHAR → VARCHAR2`, `XML/XMLTYPE → XMLTYPE`, `JSON/JSONB → JSON`, `GEOMETRY/GEOGRAPHY/SDO_GEOMETRY → SDO_GEOMETRY`, `NUMERIC → NUMBER`. [Observed: `org.jkiss.dbeaver.ext.oracle/src/.../model/OracleSQLDialect.java:574-610`]
- Any JDBC target: a source `varchar` with no length becomes the first available of `longtext`, `clob`, `text`, `string`, `nclob`. [Observed: `org.jkiss.dbeaver.model.jdbc/src/.../JDBCSQLDialect.java:50-56, 452-475`]

Length clamp: a global preference `maxTypeLengthMapping` (default 32,767, "Max Oracle VARCHAR data type length") caps lengths/precisions of created columns. [Observed: `DT/DTConstants.java:40, 59`; `DatabaseMappingAttribute.java:344-361`]

Historic complaints: SQL Server `datetime` mapped to PostgreSQL `date`, `nvarchar` to `char` (#3536); CSV `true/false` producing `BOOLEAN` on Oracle versions that do not support it (#37855); "Enhanced type mapping between databases" request (#10047); bigint not selectable as target (#19541).

### 3.9 Tasks, scheduling, and native tools

**Task model.** A task (`DBTTask`) has id, project, type, folder, properties map (the serialized wizard settings), run history and per-run log files. [Observed: `MODEL/task/DBTTask.java:38-76`] Task types are declared in `plugin.xml` (`org.jkiss.dbeaver.task` extension point) with a handler class: `dataExport`, `dataImport`, `scriptExecute` here; `pgDatabaseBackup`, `pgDatabaseRestore`, `pgDatabaseBackupAll`, `pgScriptExecute`, `pgToolTableVacuum`…, `mysqlDatabaseBackup`, `mysqlDatabaseRestore`, `mysqlToolOptimizeTable`… in the database plugins. [Observed: `org.jkiss.dbeaver.data.transfer/plugin.xml:295-326`; `org.jkiss.dbeaver.ext.postgresql/plugin.xml:857-916`; `org.jkiss.dbeaver.ext.mysql/plugin.xml:474-513`] Tasks are stored per project in `.dbeaver/tasks.json`. [Observed: `org.jkiss.dbeaver.registry/src/.../task/TaskConstants.java:22`; `TaskManagerImpl.java:492-537`; import transformer `org.jkiss.dbeaver.registry/plugin.xml:24-26`]

Serialization of producers/consumers uses a separate `org.jkiss.dbeaver.serialize` extension point so tasks can reference database objects by stable id (`DBUtils.getObjectFullId`) and re-resolve them later. The stream consumer is marked `supportsDistributedMode="false"` (it writes to local disk, so it cannot run on CloudBeaver servers). [Observed: `org.jkiss.dbeaver.data.transfer/plugin.xml:328-339`] Tests cover state migration between projects (`TEST-DT/task/DTTaskHandlerTransferImportTest.java:35-110`) and settings round-trips (`TEST-DT/database/DatabaseConsumerSettingsTest.java:131-318`).

**Scheduling.** `DBTScheduler`, `DBTTaskScheduleConfiguration` (MINUTELY, HOURLY, DAILY, WEEKLY, MONTHLY, ONE_TIME, EVENT "not supported yet") and `TaskRegistry.getActiveSchedulerInstance()` exist, but no CE plugin contributes a scheduler, so the method returns `null` in Community. [Observed: `MODEL/task/DBTTaskScheduleConfiguration.java`; `org.jkiss.dbeaver.registry/src/.../task/TaskRegistry.java:165-175`; no scheduler contribution in any CE `plugin.xml`] The edition page lists "Task scheduler" for Enterprise and Ultimate only. [Observed: dbeaver.com/edition, 2026-11-11]

**Native tools.** `AbstractNativeToolHandler.executeProcess` [Observed: `org.jkiss.dbeaver.tasks.native/src/.../AbstractNativeToolHandler.java:210-262`]:

1. Build the command line (per handler).
2. `ProcessBuilder` with working directory = folder of the binary; optionally merge stderr into stdout.
3. `setupProcessParameters` adds credentials to the environment.
4. Start; a `LogReaderJob` pumps the process output into the task log.
5. Poll every 100 ms; on cancel call `process.destroy()`; on exit validate the exit code.

PostgreSQL [Observed: `org.jkiss.dbeaver.ext.postgresql/src/.../tasks/PostgreNativeToolHandler.java:40-80`]: binary `pg_dump`, `pg_restore`, `pg_dumpall` or `psql` (plain-format restore) from the configured "local client" home; `--host/--port` from the *actual* connection configuration (which is the local end of an SSH tunnel when one is used [Inferred: `getActualConnectionConfiguration` naming, `NativeToolUtils.java:107-140`]); `--username`; password in `PGPASSWORD`. Backup adds `--format=<plain|custom|directory|tar>`, `--no-owner`, etc. [Observed: `PostgreDatabaseBackupHandler.java:106-120`]

MySQL [Observed: `org.jkiss.dbeaver.ext.mysql/src/.../tasks/MySQLNativeToolHandler.java:62-122`]: either `-u <user>` plus `MYSQL_PWD` env var, or a temporary `.my.cnf` with `[client] user=… password="…"` passed as `--defaults-file`, deleted on exit. Identifiers on Unix are passed unquoted (`escapeCLIIdentifier` returns the name as-is), but because `ProcessBuilder` does not use a shell this is not an injection risk. [Observed: `PostgreNativeToolHandler.java:86-97`; Inferred: no shell involved]

Note that `PGPASSWORD`/`MYSQL_PWD` are visible to other processes of the same user on some systems (`/proc/<pid>/environ` on Linux); MySQL itself documents `MYSQL_PWD` as insecure. [Inferred: general knowledge]

### 3.10 What DBeaver tests (and does not)

| Area | Tests | File |
|---|---|---|
| CSV export quoting, separators, newlines, formula escaping, Unicode, header quoting | ~40 parameterized tests | `TEST-PLAT/DataExporterCSVTest.java:100-690` |
| CSV import header/no header, type guessing, long numbers, empty file, sampling over blank lines, trailing missing column, custom escape | 8 tests | `TEST-PLAT/CSVImporterTest.java:65-194` |
| Markdown table escaping | ~30 tests | `TEST-DT/stream/exporter/DataExporterMarkdownTableTest.java` |
| XML, DbUnit exporters | yes | `TEST-DT/stream/exporter/DataExporterXMLTest.java`, `DataExporterDbUnitExpertRowTest.java` |
| Date parsing on import | 6 tests | `TEST-DT/stream/StreamTransferResultSetTest.java:43-82` |
| Consumer retry re-initializes exporter and header | 1 test | `TEST-DT/stream/StreamTransferConsumerTest.java:37-68` |
| Job cancellation/close semantics, parallel workers | ~15 tests | `TEST-DT/DataTransferJobTest.java`, `TEST-DT/task/DTTaskHandlerTransferTest.java` |
| Task settings persistence | yes | `TEST-DT/database/DatabaseConsumerSettingsTest.java` |
| **Type mapping (`mapTargetDataType`)** | **none found** | grep over `test/` |
| **Batch insert/commit/error recovery** | **none found** (only PG COPY loader) | grep over `test/` |

The two most bug-prone areas (type mapping, batch error recovery) are the least tested. Hacivat should invert that.

### 3.11 NoSQL in the Community edition

**What exists in CE**

- No document/key-value model implementation: no class in CE implements `DBSDocumentContainer`, and only the generic `isDynamicMetadata()` defaults exist. [Observed: grep over `plugins/`]
- The data-transfer and results code has hooks for document sources (JSON document export, dynamic target tables, document attributes), used by the paid editions. [Observed: `DataExporterJSON.isJsonDocumentResults`; `DatabaseTransferConsumer.java:198-246`; `DatabaseTransferUtils.createTargetDynamicTable`]
- **Stub drivers** that only show an "upgrade" page: MongoDB (`mongo-ce`), Cassandra (`cql-ce`), Salesforce, Couchbase, Redis, AWS Timestream, Neo4j. Example text: "MongoDB driver is not available in Community version … You can connect to MongoDB using the DBeaver PRO products: Lite, Enterprise, Ultimate, and Team Editions." [Observed: `org.jkiss.dbeaver.ext.generic/plugin.xml:1762-1832`; `.../OSGI-INF/l10n/bundle.properties:122-151`]
- **Elasticsearch / Open Distro Elasticsearch** through their SQL JDBC drivers (tables only, read-mostly). The official driver jar is Elastic License 2.0, not open source. [Observed: `org.jkiss.dbeaver.ext.generic/plugin.xml:9-17, 1840-1890`]
- Time-series/analytics engines with SQL front ends: ClickHouse, IoTDB, GreptimeDB, Timeplus, Manticore, DuckDB… these are SQL databases, not NoSQL in the document/key-value sense. [Observed: `plugins/` listing]
- WMI (Windows Management Instrumentation) via a custom non-JDBC driver — proof that DBeaver's model can host non-JDBC sources. [Observed: `org.jkiss.dbeaver.ext.wmi`]

**What is paid** [Observed: dbeaver.com/edition, fetched 2026-11-11; stub texts above]

| Capability | CE | Lite | Enterprise | Ultimate |
|---|---|---|---|---|
| MongoDB, Redis, Cassandra, Couchbase, DynamoDB, etc. ("NoSQL/BigData") | no (stubs) | yes | yes | yes |
| Task scheduler | no | no | yes | yes |
| Data compare, schema compare/migration | basic "simple compare" only (`org.jkiss.dbeaver.cmp.simple`) | no | yes | yes |
| Visual query builder, AI assistant | see doc 03 | yes | yes | yes |

The exact database list per edition is behind a link on that page and was not verified. [Unknown]

**Lesson for Hacivat.** DBeaver kept its core relational and forced NoSQL into a separate commercial layer. DataGrip forced Redis/Mongo into JDBC drivers (see `report-connectivity.md` §1). Hacivat already lists Redis and MongoDB as first-class targets, so its core data model must not assume "everything is a table with typed columns" (see Section 5).

### 3.12 Localization

**Mechanism 1: Java UI strings (Eclipse NLS).** Each plugin with user-visible text has a `*Messages.java` class extending `org.eclipse.osgi.util.NLS`, with one `public static String key;` field per message and a static initializer `NLS.initializeMessages(BUNDLE_NAME, X.class)`. The bundle name points to `*.properties` files next to the class (`CoreResources.properties`, `CoreResources_de.properties`, …). Code uses `CoreMessages.some_key` and `NLS.bind(pattern, args…)` for placeholders like `{0}`. [Observed: `org.jkiss.dbeaver.core/src/org/jkiss/dbeaver/core/CoreMessages.java:20-412`; usage `DT/DataTransferJob.java:164-167`] The repo rule says: never hard-code UI text; messages in exceptions stay English. [Observed: `AGENTS.md` "Hardcode" section]

**Mechanism 2: plugin.xml strings (OSGi bundle localization).** Labels in `plugin.xml` are written as `%key` and resolved from `OSGI-INF/l10n/bundle_<locale>.properties`, declared by `Bundle-Localization: OSGI-INF/l10n/bundle` in 52 manifests. [Observed: `grep Bundle-Localization */META-INF/MANIFEST.MF`]

**Mechanism 3: object property labels.** Database object properties shown in the property grid are annotated with `@Property`; their labels are looked up as `meta.<fully.qualified.Class>.<propertyName>.name|description|hint` in the declaring bundle's localization, then in an optional shared "localization owner" class (added 2026, PR #42339), then in superclasses' bundles, then fall back to a default (the property id). Details in ALG-DBV-I18N-01. [Observed: `MODEL/../runtime/properties/ObjectPropertyDescriptor.java:649-704`]

**Language selection.** The selectable languages come from the `org.jkiss.dbeaver.language` extension point. Labels are written in their own language and must not be localized. [Observed: `org.jkiss.dbeaver.registry/plugin.xml:28-44`] The chosen code is stored as global property `nl`; at the next start the launcher reads it and sets `osgi.nl` (defaulting to `en`, not the OS locale). The preference page asks to restart with a **hard-coded English** message ("You need to restart … to perform actual language change"). [Observed: `org.jkiss.dbeaver.registry/src/.../BasePlatformImpl.java:449-459`; `org.jkiss.dbeaver.launcher/src/.../DBeaverLauncher.java:2279-2291`; `org.jkiss.dbeaver.core/src/.../PrefPageGeneral.java:246-254`]

**Shipped locales and coverage** (own count, 2026-11-11, script in session scratchpad; counts keys present in each locale file for every base file that has translations):

| Locale | Label | Files | Keys translated / English keys | Values identical to English |
|---|---|---|---|---|
| ar | العربية | 208 | 12,031 / 12,031 | 0.6% |
| de | Deutsch | 211 | 12,031 / 12,031 | 5.8% |
| es | Español | 211 | 12,031 / 12,031 | 1.6% |
| fr | Français | 208 | 12,031 / 12,031 | 6.0% |
| it | Italiano | 211 | 12,031 / 12,031 | 3.1% |
| ja | 日本語 | 208 | 12,031 / 12,031 | 0.6% |
| ko | 한국어 | 208 | 12,031 / 12,031 | 0.5% |
| pl | Polski | 208 | 12,031 / 12,031 | 1.6% |
| pt_BR | Português (BR) | 212 | 12,031 / 12,031 | 2.2% |
| ro | Română | 211 | 12,031 / 12,031 | 2.5% |
| ru | Русский | 211 | 12,031 / 12,031 | 0.5% |
| tw | 繁體中文 | 213 | 12,031 / 12,031 | 0.6% |
| uk | Українська | 210 | 12,031 / 12,031 | 0.5% |
| zh | 简体中文 | 211 | 12,031 / 12,031 | 0.6% |
| **tr** | — | **0** | **not shipped** | — |

"Identical to English" includes legitimate cases (technical words like "SQL", "Schema", "JSON"), so it is an upper bound on untranslated strings. Several locales also carry 5–56 *stale* keys that no longer exist in English. [Observed: own count]

Full coverage is recent: PR #42805 (merged 2026-11-10, +88,069/−3,395 lines, 2,306 files) "Synchronize desktop Community localizations" completed all 14 locales over "209 resource families (11,975 English keys)" with automated validation (UTF-8, duplicate keys, placeholders, MessageFormat). PR #42543 (2026-11-03) added Polish "with the help of AI tools and reviewed and tested by me". [Observed: `gh pr view 42805`, `gh pr view 42543`] Before that, coverage was uneven (issues #1285 "German localization is incomplete", #14964, #21384 "hard-coded cases… part 5", #40859 Firebird without NLS).

**Turkish.** No Turkish locale files, no `tr` entry in the language registry. [Observed] Issue #34432 (2024) is a user reporting a "minor Turkish translation error" while "using DBeaver in Turkish"; with no `tr` bundles in the repo, the Turkish text that user saw must have come from somewhere else (e.g. Eclipse platform language packs or a third-party build). [Inferred]

**Workflow.** Contributions are pull requests editing `.properties` files (the contributor docs point to a "Localization" wiki page, which returned 404 on 2026-11-11). A hosted Weblate project for DBeaver exists but, at the time of the search snapshot, it held only a handful of strings, so the main desktop translations are not managed there. [Observed: web search results; `https://dbeaver.com/docs/wiki/Localization` 404] Translation fixes are typical "good first issues" (#32986/#32987 German wording, #17190 Russian).

**Encoding.** Translated `.properties` files are raw UTF-8 (all non-ASCII files decode as valid UTF-8; only a few lines still use `\uXXXX` escapes). [Observed: own scan; `grep -c '\\u0'` on `CoreResources_*.properties`] Traditionally Java `.properties` were ISO-8859-1; modern Java `PropertyResourceBundle` reads UTF-8. Which loader Eclipse NLS uses at this platform version was not checked. [Unknown]

**Naming pitfall.** Traditional Chinese uses `_tw` instead of the standard `zh_TW` (or `zh_Hant`). IntelliJ's ResourceBundle editor showed it as an unknown language; renaming was requested (#19589) and closed without the rename. [Observed: issue #19589; files `*_tw.properties` still present]

**Other observed pitfalls**

- Missing keys in Eclipse NLS do not crash: the field gets a placeholder text like "NLS missing message: KEY in: BUNDLE". [Inferred: documented Eclipse NLS behavior; not re-verified in this repo]
- Reflection on a private Equinox field (`localizationTracker`) to reach `BundleLocalization`. [Observed: `MODEL/../utils/RuntimeUtils.java:657-680`] Fragile across Eclipse upgrades.
- Hard-coded English in UI code still exists (restart dialog above; #21384 series).
- Changing language needs a restart.

### 3.13 Quality lessons from GitHub issues

All issues are in `dbeaver/dbeaver`; state as of 2026-11-11.

| Theme | Issues (state) | What we learn |
|---|---|---|
| OOM on huge result sets | #34854 "crashes when selecting Fetch All Data… Java heap space" at ~25k wide rows (closed); #17897 PostgreSQL OOM (closed); #17099, #21885 (open) | The grid keeps all fetched rows as Java objects. Hacivat: cap rows held in the UI, keep a columnar page cache, spill to disk. |
| OOM on import | #11931 "Java heap space error while importing data" 10M rows, even with segments and lower commit intervals (open) | Batch = commit interval = 10,000 rows of objects in memory, plus driver buffers. Hacivat: bounded channels, small insert batches independent of commit interval. |
| Opening huge SQL files | #42570 800 MB script OOM (open); #19104 3 GB SQLite (closed) | Execute big scripts by streaming from disk without opening them in the editor (DataGrip has "run file without opening"). |
| UI freezes / deadlocks | #42803 startup freeze in `BaseProjectImpl.getDataSourceRegistry()` (open, 2026-11-10); #42205 deadlock with detached navigator (closed); #41207 deleting indexes blocks UI thread; #38639 freeze on connection timeout; #9699 freeze during metadata load; #35261 freeze while typing (PostgreSQL); #29319 lock-up when a table holds `AccessExclusiveLock` (open) | Any metadata or network call reachable from the UI thread will eventually freeze someone. Hacivat: UI thread never awaits DB/network; all DB calls have timeouts; lock waits are cancellable. |
| Large schemas | #39861 navigator slow with 20,000+ views (open); #11555 navigator filter slow with many tables; #8473 slow DDL generation on large schemas (open); #12544 full-text search hang; #39765 TimescaleDB public schema "not responding" | Need virtualized trees, incremental metadata, background DDL generation, server-side filtering. |
| Slow export | #3568 `LIMIT offset` segments slow on MySQL; #6031 Excel export slow; #973 DB2 CSV slow | Keyset pagination or a single cursor; avoid per-cell styling overhead in XLSX. |
| Import type problems | #37855 BOOLEAN on Oracle; #39423 changing column type in CSV import fails; #16675 types wrong after export/import round-trip; #19219 Trino varchar; #35103 "universal column type text"; #38997 decimal separator option missing; #22483 RFC 4180 import | Type guessing needs locale-aware numbers, dates, explicit "all TEXT" option, and target-capability checks. |
| Transfer UX bugs | #41281 saved task exports whole table instead of selection; #36085 `${datasource}` empty; #35197 auto-generated columns option ignored; #23127 system columns appear | Task serialization must capture *exactly* what the user saw. |
| Driver downloads | #40237 "Driver file download failed" (open); #34758, #34757, #36107 dialog on every connect; #35466 PG driver download error loops; #20431 IBM i driver always fails | Maven-based runtime downloads break behind proxies/firewalls. Hacivat (native Rust drivers compiled in) mostly avoids this by design. |
| Memory leaks / threads | #15643 excessive thread count; #16383 big JSONB freeze/leak | Bound thread pools; stream large JSON into the value viewer lazily. |

---

## 4. Algorithm specifications

### ALG-DBV-XFER-01: Type mapping between databases

- **Problem:** When copying a table from engine A (say SQL Server) to engine B (say PostgreSQL), or importing a CSV into a new table, we must choose a column type in B for each source column so that every value fits without loss, the DDL is valid on B, and the result looks natural to a B user.

- **Inputs / outputs:**
  - Inputs: source column descriptor (`type_name`, `data_kind` ∈ {BOOLEAN, NUMERIC, STRING, DATETIME, BINARY, CONTENT, STRUCT, DOCUMENT, ARRAY, OBJECT, REFERENCE, ROWID, ANY, UNKNOWN}, `max_length`, `precision`, `scale`, `nullable`, source engine/dialect); target engine's type catalog (list of types with name and kind); target dialect rules; user preferences (max type length); `add_modifiers` flag (whether to append `(len)` / `(p,s)`).
  - Output: target type string, e.g. `numeric(10,2)` or `varchar(255)`.

- **Algorithm (DBeaver, as implemented):** [Observed: `MODEL/struct/DBStructUtils.java:387-567`; `DT/database/DatabaseMappingAttribute.java:326-361`]
  1. *User override*: if the user typed a target type in the mapping page, use it (with or without modifiers as requested).
  2. *Clamp lengths*: if the preference `maxTypeLengthMapping` (default 32,767) is set and the source length (or numeric precision) exceeds it, lower the source descriptor's length/precision to the cap. (This mutates the source descriptor.)
  3. *Same engine shortcut*: if the source is a real table column (or a query column bound to one) and the source and target data source objects are the **same Java class** and modifiers are wanted, return the source's full type name verbatim (e.g. `character varying(40)`).
  4. *Dialect converter*: if the target dialect implements `SQLDataTypeConverter` and it returns a name, use it. (PostgreSQL and Oracle have small tables of known foreign names; the generic JDBC rule turns length-less `varchar` into the first available of `longtext, clob, text, string, nclob`.)
  5. *Exact lookup*: look up the source type name (case-insensitive) in the target's type catalog; if not found and the name has modifiers like `decimal(10,2)`, retry with them stripped and keep the modifiers; if the name is `double`, try `DOUBLE PRECISION`.
  6. *Kind check*: discard the found type if `canConsume(source_kind, found_kind)` is false (target STRING accepts only STRING; target BINARY/CONTENT accepts STRING/BINARY/CONTENT; everything else requires equal kinds).
  7. *Similarity search* (if nothing found): collect candidate target types `possible` (ordered as the catalog lists them) whose kind is compatible, then:
     - exact lowercase name in `possible` → use it;
     - NUMERIC source: first candidate with equal precision and scale; else if source name contains `float`/`real` or 0 < scale ≤ 6 → first candidate whose name contains `float` or `real`; else if name contains `double` or 0 < scale ≤ 15 → first candidate containing `double`; else `int` ↔ `integer` synonym; else first candidate containing `int`;
     - STRING source whose name contains `text` or has no max length → `text`, else first candidate ending in `text`.
  8. *Default for kind*: target engine's `getDefaultDataTypeName(kind)` (table in §3.8); if that name is in `possible`, use that candidate's spelling.
  9. *Last resort*: first entry of `possible`.
  10. *Case fix*: if the chosen name equals the catalog name ignoring case, use the catalog's spelling.
  11. *Modifiers*: if requested, ask the target dialect for type modifiers for this (source column, chosen type, kind) and append them; if the dialect returns none and the type is `varchar`/`varchar2`, append `(100)`.

- **Data structures:** catalog = ordered list of (name, kind, precision, scale); mapping state per column = (mapping type, target name, target type override, transformer).

- **Complexity / performance notes:** O(T) per column where T is the number of target types (hundreds at most). Fine. The cost that matters is loading the target catalog, which DBeaver caches with the connection metadata.

- **Edge cases (exhaustive list for this problem; ✗ marks where DBeaver's heuristic is known or likely to go wrong):**
  1. Same engine, different versions (MySQL 8 → MySQL 5.6): shortcut copies `utf8mb4_0900_ai_ci`-dependent or `JSON`-typed columns verbatim; the older target may reject them. ✗
  2. Same engine family but different driver classes (PostgreSQL → Greenplum/Redshift/Yugabyte): shortcut skipped; falls through to name lookup, usually fine.
  3. Query column computed by an expression (no table column): kind comes from the JDBC result-set metadata; precision may be 0/unknown → step 7 may pick an unrelated numeric type. ✗
  4. `NUMERIC` without precision/scale (PostgreSQL unlimited `numeric`): equal-precision search matches nothing; scale `null` → falls to the `int` branch → may produce `int` and **lose decimals**. ✗
  5. Source `decimal(38,10)` into SQL Server: exact name exists, modifiers appended by dialect; OK. Into SQLite: affinity rules accept anything; OK.
  6. Scale between 7 and 15 maps to `double` (binary floating point) instead of `decimal`: money values lose exactness. ✗
  7. `float` vs `real` vs `double precision` naming differences (Oracle `BINARY_DOUBLE`, MySQL `DOUBLE`, PG `float8`).
  8. Unsigned integers (MySQL `INT UNSIGNED` up to 4,294,967,295) to engines without unsigned → must widen to `bigint`; DBeaver's name lookup finds `int` (signed) → overflow on large values. ✗ [Inferred]
  9. `BIGINT UNSIGNED` (up to 1.8e19) to PostgreSQL needs `numeric(20)`. ✗ [Inferred]
  10. Booleans: MySQL `TINYINT(1)`, SQL Server `bit`, Oracle (before 23ai) has no SQL BOOLEAN column type (#37855). ✗
  11. Strings without length (`text`, PG `varchar`) to engines requiring length → `longtext/clob/text/…` rule; to SQL Server `varchar(max)` would be correct but is not in the list (`text` is deprecated in SQL Server). ✗ [Inferred]
  12. Length semantics: characters vs bytes (Oracle `VARCHAR2(10 BYTE)` vs `CHAR`; MySQL utf8mb4 4 bytes/char); a 10-char Turkish string with `ç, ğ, ı, ö, ş, ü` needs up to 20 bytes in UTF-8. ✗
  13. Lengths above the target maximum (MySQL `VARCHAR` 65,535 bytes per row total; SQL Server 8,000 bytes non-max) → need LOB type; DBeaver clamps to 32,767 globally, which can still exceed MySQL's row limit. ✗
  14. National character types (`NVARCHAR`, `NCHAR`) to PG → `varchar` (correct; PG is UTF-8); to MySQL → `VARCHAR` with charset? not handled.
  15. `CHAR(n)` padding semantics differ (trailing spaces).
  16. Date/time: SQL Server `datetime` (3.33 ms precision), `datetime2(7)`, `datetimeoffset`; PG `timestamp` vs `timestamptz`; MySQL `DATETIME` vs `TIMESTAMP` (UTC-converted, 1970–2038 range); Oracle `DATE` includes time. #3536 mapped `datetime` → `date`. ✗
  17. Time zones: `timestamptz` → engine without zone type: must normalize to UTC and document it.
  18. Intervals (PG `interval`, Oracle `INTERVAL DAY TO SECOND`) → no equivalent in MySQL/SQL Server.
  19. UUID: PG `uuid`, SQL Server `uniqueidentifier`, MySQL `BINARY(16)` or `CHAR(36)`. ROWID kind default in PG is `oid` — wrong for UUIDs. ✗
  20. JSON: PG `json/jsonb`, MySQL `JSON`, SQL Server `nvarchar(max)` (or native `json` in 2025), Oracle `JSON` (21c+), SQLite `TEXT`.
  21. XML, geometry/geography (PostGIS vs SQL Server vs Oracle SDO), arrays (PG `int[]`), enums (PG/MySQL `ENUM`), sets (MySQL `SET`), ranges, `hstore`, network types (`inet`, `cidr`, `macaddr`), money (`money`), bit strings (`bit varying`).
  22. Binary: `bytea` vs `VARBINARY(n)` vs `BLOB`/`LONGBLOB` vs `IMAGE`.
  23. Identity/auto-increment: `serial`/`GENERATED … AS IDENTITY`/`AUTO_INCREMENT`/`IDENTITY(1,1)`; copying values into identity columns needs `OVERRIDING SYSTEM VALUE` (PG) or `SET IDENTITY_INSERT ON` (SQL Server). DBeaver copies the plain type only. [Inferred]
  24. Computed/generated columns: should be skipped or recreated as expressions.
  25. Collations: DBeaver copies the source collation only when the target knows it and the type is collatable. [Observed: `DatabaseMappingAttribute.java` `getTargetCollation`]
  26. Defaults and CHECK constraints: not mapped (dialect-specific expressions).
  27. Case of type names in catalogs (`VARCHAR` vs `varchar`) — handled by step 10.
  28. Unknown/user-defined types (PG domains, composite types, Oracle object types) → kind OBJECT/STRUCT → no compatible candidate → default for kind ("?" if no provider). ✗
  29. Target catalog empty (driver didn't report types) → default name only.
  30. Source kind UNKNOWN/ANY → no candidates; DBeaver returns default for UNKNOWN which is often `varchar` or "?" ✗.
  31. Argument order in step 7: candidates are filtered with `canConsume(candidate_kind, source_kind)`, the reverse of step 6. For a CONTENT (LOB) source this admits STRING candidates, so a BLOB could be mapped to a `varchar`-like type. [Observed: `DBStructUtils.java:457-461` vs `:447`; Inferred: consequence] ✗
  32. Catalog iteration order decides ties ("first candidate containing `int`" could be `tinyint`, `smallint`, `int`, `bigint`, `interval`… depending on order — note `interval` contains `int`). ✗

- **Test cases (input → expected).** DBeaver has no unit tests here; these are proposed golden tests for Hacivat, based on DBeaver's rules plus the fixes above:

  | Source (engine, type) | Target engine | DBeaver result (from the rules) | Hacivat expected |
  |---|---|---|---|
  | PG `integer` | PG | `integer` (shortcut) | `integer` |
  | PG `numeric(10,2)` | MySQL | name lookup `numeric` → MySQL alias `DECIMAL(10,2)` | `DECIMAL(10,2)` |
  | PG `numeric` (no p/s) | SQL Server | likely `int` via the int branch ✗ | `DECIMAL(38,10)` + warning "unbounded numeric narrowed" |
  | SQL Server `datetime` | PG | was `date` in #3536 ✗ | `timestamp(3)` |
  | SQL Server `datetimeoffset` | PG | uncertain | `timestamptz` |
  | SQL Server `nvarchar(50)` | PG | `varchar(50)` (PG converter) | `varchar(50)` |
  | Oracle `NUMBER(19,0)` | PG | `numeric(19,0)` (PG converter) | `bigint` (precision ≤ 18 → bigint; 19 → `numeric(19)`) |
  | MySQL `INT UNSIGNED` | PG | `int` ✗ | `bigint` |
  | MySQL `BIGINT UNSIGNED` | PG | `bigint` ✗ | `numeric(20,0)` |
  | MySQL `TINYINT(1)` | PG | `smallint`-ish | `boolean` if all values ∈ {0,1} and the source column is declared `TINYINT(1)`; else `smallint` |
  | PG `boolean` | Oracle 19c | `BOOLEAN` → fails (#37855) ✗ | `NUMBER(1)` + CHECK (0,1) |
  | PG `text` | SQL Server | `text` (deprecated) | `nvarchar(max)` |
  | PG `uuid` | MySQL | default for kind ✗ | `CHAR(36)` (or `BINARY(16)` by preference) |
  | PG `jsonb` | MySQL 8 | name not found → STRING default `VARCHAR(100)` ✗ | `JSON` |
  | CSV inferred `VARCHAR` max 37 chars, Turkish text | MySQL utf8mb4 | `VARCHAR(64)` (power of 2, characters) | `VARCHAR(64)` (characters, OK in utf8mb4) |
  | CSV inferred `VARCHAR` max 37 chars | Oracle `VARCHAR2(n BYTE)` default | `VARCHAR2(64)` bytes ✗ for multibyte | `VARCHAR2(64 CHAR)` |
  | PG `bytea` | MySQL | `BLOB`/`LONGBLOB` | `LONGBLOB` |
  | PG `int[]` | MySQL | default ✗ | `JSON` + warning |

- **Prior art and evidence:** DBeaver code above; DataGrip: type mapping is part of its import dialog ("mevcut veya yeni tablo; kolon eşleme; DDL + veri önizleme") and uses per-dialect "Domain" registries (Integer/Number/Text/Binary/Boolean/Identity…) to decide paste/import compatibility, which is closer to a rule table than DBeaver's name heuristics. [Observed: `.refs/datagrip-extract/report-grid.md` §1, §8] DataGrip also widens inferred CSV types along Bool→Int→BigInt→Double→String (`TypeMerger`).

- **Recommendation for Hacivat:** do *not* port DBeaver's substring heuristics. Instead:
  1. Normalize every source column to a **logical type** (an enum shared with the grid: `Bool, Int{bits, signed}, Decimal{p, s} | DecimalUnbounded, Float{bits}, Text{max_chars, fixed}, Bytes{max}, Date, Time{precision, tz}, Timestamp{precision, tz}, Interval, Uuid, Json, Xml, Geometry, Array(Box<Logical>), Enum(Vec<String>), Other(String)`).
  2. Have each engine crate provide two pure functions: `to_logical(native_type) -> Logical` and `from_logical(Logical, &TargetCaps) -> NativeType + Vec<Warning>`.
  3. Keep the "same engine and compatible version → verbatim" shortcut, but check the target server version's capability table.
  4. Every lossy step produces a typed warning shown in the mapping preview (DataGrip-style "DDL + data preview").
  5. Ship the golden table above as data-driven tests per engine pair.
- **Used by:** Hacivat (copy/import wizard, "create table from CSV"), Karagöz (if server-side copy between agent-managed databases is ever offered).

---

### ALG-DBV-XFER-02: Streaming export pipeline (row batching, formatting, file splitting)

- **Problem:** Export a result of unknown size (possibly hundreds of millions of rows) to a file in a chosen format without loading it into memory, with correct escaping, optional compression, optional splitting into several files, correct handling of large binary values, and clean behavior on cancel or error.

- **Inputs / outputs:**
  - Inputs: a row source (query or table), format processor + its options, output settings (folder, name pattern, encoding, BOM, compress, split size, single file vs per table, LOB policy, conflict policy, clipboard), fetch size.
  - Outputs: one or more files (or clipboard text), statistics (rows, bytes, time), list of written paths, event hooks fired.

- **Algorithm (DBeaver):** [Observed: `DT/database/DatabaseTransferProducer.java:181-352`; `DT/stream/StreamTransferConsumer.java:159-247, 358-590, 638-720`; `DT/stream/exporter/DataExporterCSV.java:115-420`]
  1. Open a dedicated read connection (unless embedded or exporting the grid's in-memory rows); set auto-commit off; set the fetch size (10,000).
  2. Optionally `COUNT(*)` for progress.
  3. Execute the query (single query, or `LIMIT/OFFSET` segments).
  4. On the first result-set callback: resolve the output path from the pattern (fallback folder if not writable); resolve conflicts (ask/append/rename/overwrite); build the stream stack (buffer 100 kB → stats → zip? → split counter? → charset writer); write BOM if requested; `exporter.init(options)`; compute column list (flattening nested structures unless the exporter is document-aware; dropping unchecked columns); `exporter.exportHeader`.
  5. For each row:
     1. If splitting is on, the output is text, and this is not the first row: flush the writer; if bytes written ≥ limit: `exportFooter`, close, open `<name>_<k+1>.<ext>`, `exportHeader`.
     2. Read every column value through its value handler.
     3. For binary LOB values apply the LOB policy (null / inline / save to `files/` and substitute path).
     4. `exporter.exportRow(row)`.
  6. On normal end: `exportFooter`, close (finish zip entry, flush, close).
  7. On last source: copy to clipboard if needed; fire FINISH event processors (open folder, run command). On error: fire error processors (e.g. delete partial files).
  8. Always: roll back the read transaction, restore auto-commit, close the dedicated connection.

  **CSV row formatting** (the most used exporter):
  1. Value → string with the user's display format, except numbers (unless "format numbers") and arrays, which use the native format (no thousands separators).
  2. Arrays: convert brackets between `{ }`, `[ ]`, `( )` outside double-quoted substrings.
  3. Decide "should quote" by strategy: `disabled` → quote non-number, non-date strings that start with a digit (to protect leading zeros / IDs from spreadsheet number conversion); `strings` → quote all non-empty non-number non-date; `all but numbers` → quote everything except numbers; `all` / `all including nulls` → quote everything.
  4. NULL: write `nullString` if set; else empty quoted `""` if `all including nulls`; else nothing (so NULL is an empty unquoted field, while an empty string becomes `""`).
  5. Formula guard (not for numbers): if the text starts with `= + - @ \t \r \n` or full-width `＝ ＋ － ＠`, prefix `'`.
  6. If a line-feed replacement string is set, replace `\r\n`/`\n` with it and force quoting.
  7. If the quote is a single character and appears in the value, double it and force quoting.
  8. Also force quoting if the value is empty (when the strategy applies), contains the delimiter, the quote, `\n` or `\r`.
  9. Write `[quote]value[quote]`, delimiter between fields, row delimiter after the row.

- **Data structures:** stream stack; per-consumer runtime parameters (conflict choices remembered "for all"); list of output paths; per-exporter state (header written, row count).

- **Complexity / performance notes:** O(rows × columns), constant memory except: TXT buffers 200 rows; XLSX keeps 100 rows in memory and spills the rest to POI temp files (temp disk ≈ output size or larger); SQL exporter buffers nothing. Flushing the writer for every row when splitting is on costs throughput. [Inferred] `LIMIT/OFFSET` segments are O(n²) on engines that scan skipped rows (#3568).

- **Edge cases (exhaustive list):**
  1. Empty result: header (and footer) only; JSON writes `[\n]`. Split never triggers.
  2. Zero columns (DDL statement exported by mistake): CSV writes an empty line per row.
  3. Duplicate column names (`SELECT a.id, b.id`): CSV header uses labels; JSON produces duplicate keys ✗ (valid JSON syntax, but most parsers keep the last). [Inferred]
  4. Column label vs name vs description (header format option).
  5. NULL vs empty string must be distinguishable (CSV: nothing vs `""`).
  6. Delimiter equal to or starting with the quote → configuration error (tested).
  7. Multi-character delimiter (e.g. `||`) and multi-character quote (not escaped by design).
  8. Values containing delimiter, quote, CR, LF, CRLF; leading/trailing spaces (kept as-is, tested).
  9. Formula injection prefixes including full-width forms; a negative *number* is not prefixed, but a negative number stored as *text* (`'-5'` in a VARCHAR) becomes `'-5` and changes the data on round trip ✗ (tested both behaviors: `TEST-PLAT/DataExporterCSVTest.java:505-583`).
  10. Strings starting with digits (`00123`) — quoted under the default strategy, but Excel still converts quoted numbers; only a `="00123"` trick or XLSX text cells preserve them.
  11. Unicode beyond BMP (emoji), RTL text (Arabic), Turkish dotted/dotless i — encoding must be UTF-8 or the user's choice; BOM needed for Excel on Windows to detect UTF-8.
  12. Encoding that cannot represent a character (export Turkish `ğ` to ISO-8859-1) → Java writer substitutes `?` silently ✗.
  13. Line endings: "default" = OS separator, so the same export differs between Windows and macOS.
  14. Numbers: locale decimal separator in UI format vs `.` in native format; big decimals (`1E+3` from BigDecimal), NaN/Infinity (invalid JSON).
  15. Dates/times: display format vs ISO; time zone of the session; `0000-00-00` MySQL zero dates.
  16. Binary values inline: CSV/JSON need hex/base64 representation; large ones should go to files.
  17. LOB file name collisions (ask / rename / overwrite).
  18. Text LOBs (CLOB) always inline: a 1 GB CLOB is read into a string ✗ (`IOUtils.readToString`). [Observed: `DataExporterCSV.java:240-247`]
  19. Nested types (PG composite, arrays of records, Mongo documents): flattened to leaf columns or exported as documents.
  20. Splitting: the row that crosses the limit is written to the *old* file, so files can exceed the limit by one row (and by the buffered amount); split applies only to text exporters; split + compression measures uncompressed bytes; split + "single file" for multiple tables.
  21. Existing file: append (CSV writes no new header when appending), rename with timestamp, overwrite, ask with "apply to all".
  22. Output folder missing → created; not writable → fallback temp folder (user may not notice where the file went ✗).
  23. File name pattern variables empty (`${datasource}` empty, #36085) or containing OS-forbidden characters (sanitized to `_`, max 64 characters per variable).
  24. Several exports in parallel writing to the same file name with the same `${timestamp}` (DBeaver pins the timestamp to the transfer start to keep it stable, #7352).
  25. Cancel in the middle: partial file remains unless the "failed export file cleaner" event processor is enabled.
  26. Driver error mid-stream: footer still written? DBeaver closes the exporter on error without footer → JSON/XML files are left syntactically incomplete ✗ (that is why the cleaner exists). [Observed: `StreamTransferConsumer.java:646-652`]
  27. Retry after error must create a fresh exporter and header (tested: `TEST-DT/stream/StreamTransferConsumerTest.java:37-68`).
  28. Clipboard target: whole output in memory; huge copies can OOM.
  29. Excel limits: 1,048,576 rows per sheet (DBeaver uses 1,048,575 data rows to leave room for the header), 16,384 columns, 32,767 characters per cell (truncated with a warning), sheet names ≤ 31 characters and no `[]:*?/\`.
  30. PostgreSQL requires a transaction for cursor-based fetching; MySQL Connector/J needs `useCursorFetch=true` or streaming mode (`fetchSize=Integer.MIN_VALUE`) to avoid buffering everything client-side. [Inferred: driver documentation]

- **Test cases (from DBeaver's tests, default options: delimiter `,`, quote `"`, strategy `disabled`, escapeFormulas on):** [Observed: `TEST-PLAT/DataExporterCSVTest.java`]

  | Input row | Expected CSV |
  |---|---|
  | `["", "", ""]` | `"","",""` (line 234-245) |
  | `["a,b","c","d"]` | `"a,b",c,d` (249-258) |
  | `["a\"b","c","d"]` | `"a""b",c,d` (262-271) |
  | `["a\nb","c","d"]` | `"a⏎b",c,d` (quoted, newline kept) (275-290) |
  | `[" a ","b "," c"]` | ` a ,b , c` (491-500) |
  | `"=1+1"` | `'=1+1` (505-530) |
  | `"=\"cmd\""` | `"'=""cmd"""` |
  | `"+1+1"`, `"-1+1"`, `"@SUM(A1)"` | `'+1+1`, `'-1+1`, `'@SUM(A1)` |
  | `"\t=1+1"` | `'⇥=1+1` (tab not quoted) |
  | `"\r=1+1"` | `"'\r=1+1"` (quoted because of CR) |
  | `"＝1+1"` (U+FF1D) | `'＝1+1` |
  | `"safe \"quoted\""` | `"safe ""quoted"""` |
  | `"=1+1"` with escapeFormulas=false | `=1+1` (565-572) |
  | number `-42` | `-42` (575-583) |
  | `["Привет","こんにちは","😀"]` | `Привет,こんにちは,😀` (587-596) |
  | header `A,B,A"B` with headerQuoting=never | `A,B,A"B` (142-152) |
  | header with quoteAlways=all, headerQuoting=when necessary: `PLAIN`, `A,B`, `A"B`, `A⏎B` | `PLAIN,"A,B","A""B","A⏎B"` (127-139) |
  | delimiter = quote | configuration error (178-186) |
  | delimiter `''`, quote `'` | configuration error (starts with quote) (188-196) |
  | quoteNever + header quoting | `A,B` (155-163) |

  Markdown (from `DataExporterMarkdownTableTest.java`): `"first|second⏎third"` → `first\|second<br>third`; with cell escaping, ``"a`b"`` → ``` ``a`b`` ```.

- **Prior art and evidence:** DBeaver as above. DataGrip: export is built on the same "extractor" mechanism as copy (CSV formats shared across copy/paste/import/export, scriptable extractors with a documented `ROWS/COLUMNS/OUT` API), plus "Execute to File" that streams query output without a grid. [Observed: `.refs/datagrip-extract/report-grid.md` §6, §8]

- **Recommendation for Hacivat:**
  - Pipeline as async tasks connected by **bounded** channels: `RowSource` (driver stream, emits record batches of ~1–4k rows, Arrow `RecordBatch` or our own columnar page) → `Formatter` (pure function batch → bytes) → `Sink` (file with rotation, zip/zstd, clipboard, or stdout for the MCP server). Back-pressure happens automatically when the sink is slow.
  - Use the same formatter traits for "copy as …" in the grid and for file export (DataGrip's extractor idea), and expose them to the MCP server so an agent can request "export query as CSV" without a UI.
  - Source side: no `OFFSET` segmentation. Use server cursors (PG `DECLARE … CURSOR` / portal `max_rows`; MySQL streaming; SQL Server default forward-only; SQLite step). If re-chunking is needed, use keyset pagination on the primary key.
  - Keep DBeaver's CSV rules (formula guard on by default, NULL vs empty distinction, digit-leading strings quoted) and port its test vectors (Apache-2.0 data; keep attribution in the test file header).
  - Split by *rows or bytes after the row is fully written*, and write split boundaries only between rows; rename partial files to `*.partial` until finished, then atomically rename — no half-written JSON left behind.
  - LOBs: stream them (never `read_to_string` a CLOB); for CSV/JSON choose hex/base64 or side files.
  - Crates: `csv` (writer), `rust_xlsxwriter` (constant-memory mode), `serde_json` (streaming serializer), `zip`/`zstd`, `encoding_rs` (non-UTF-8 output with explicit error on unmappable characters instead of `?`).
- **Used by:** Hacivat (grid export, query-to-file, MCP tool `export_query`), Karagöz (server-side export so large files never cross the network as rows; then transfer the file) — optional.

---

### ALG-DBV-XFER-03: CSV import parsing and type inference

- **Problem:** Read a CSV file of unknown dialect into rows, propose column names and SQL types for a new table, and convert text values to typed values during load.

- **Inputs / outputs:**
  - Inputs: byte stream; options (encoding, delimiter, quote, escape, header yes/no, strict quotes, trim, empty-as-NULL, NULL marker, timestamp pattern and zone, sample count, minimal length, byte-or-char length).
  - Outputs: column list `(name, kind, type_name, max_length)`; row stream of `Option<String>` values; typed values for date/time targets.

- **Algorithm (DBeaver):** [Observed: `DT/stream/importer/DataImporterCSV.java:75-302`; `DT/database/DatabaseTransferUtils.java:710-754`; `DT/stream/StreamDataImporterColumnInfo.java:52-141`; `MODEL/DBPDataKind.java:68-98`; `dbeaver-common/.../csv/CSVParser.java`]
  1. **Decode:** buffered reader (255 KiB), strip BOM for the selected charset, decode with that charset (default UTF-8). No charset detection.
  2. **Tokenize** with `CSVParser` (rules in §3.6). Skip lines that parse to zero fields.
  3. **Header:** the first non-empty line defines the column count. If header=top, names come from it (unquoted as SQL identifiers); empty names become `ColumnN` (1-based). If header=none, names are `Column1..N` and the first line is also used as a data sample.
  4. **Initialize** each column as kind UNKNOWN, type `VARCHAR`, max length = `columnTypeMinimalLength` (default 1 in code, 50 in the UI default).
  5. **Sample** up to `columnTypeSamplesCount` (100) further lines; for each value in columns `0..min(len(line), header_count)`:
     - classify the value:
       - empty → UNKNOWN (ignored);
       - first char is digit, `+`, `-` or `.` → try 32-bit int → `INTEGER`; 64-bit → `BIGINT`; Java `Double.parseDouble` → `REAL`; otherwise fall through;
       - `true`/`false` (any case) → `BOOLEAN`;
       - first char is not an ASCII Latin letter → `NVARCHAR` (kind STRING);
       - else `VARCHAR` (kind STRING).
     - if STRING: `max_length = max(max_length, next_power_of_two(len))` (characters, or bytes if the option says so), but never above `maxTypeLengthMapping` (32,767 default);
     - if STRING/NUMERIC/BOOLEAN: **widen**: replace current (kind, type) with the new one if the new kind's "commonality" is higher (UNKNOWN 0 < BOOLEAN = NUMERIC = DATETIME … 1 < STRING 2 < BINARY/CONTENT 3), or if equal and both numeric and the new type ranks higher (INTEGER 1 < BIGINT 2 < REAL 3; any other name ranks 0).
  6. Columns still UNKNOWN (all samples empty) → `VARCHAR`.
  7. **Load:** re-open the stream, skip the header, and for each line: stop at `maxRows`; pad short rows with NULL; trim if asked; `""` → NULL if asked; NULL marker → NULL; hand the row to the consumer. Unterminated quote at EOF → error.
  8. **Value conversion at load time:** if the target column is DATETIME, parse with the pattern; zone-aware results become instants; local results become timestamps (date-only → 00:00) shifted to the configured zone if any; unparseable → passed through as text. NUMERIC/BOOLEAN targets are converted by the target's value handlers.

- **Data structures:** per-column `(kind, type_name, max_length, mapping_metadata_present)`; parser state (`in_quotes`, `pending` multi-line token, current token buffer).

- **Complexity / performance notes:** O(file size) for load; sampling reads only ~100 lines. Each classification can throw/catch up to three number-format exceptions (slow on JVM; trivial in Rust). The file is read twice (sampling, then load).

- **Edge cases (exhaustive list; ✗ = DBeaver's behavior is surprising or wrong):**
  1. Empty file → zero columns (tested).
  2. File with only a header → columns all `VARCHAR`.
  3. Blank lines anywhere → skipped (tested: `"1\n\n2\n3\ntest"` still samples `test`).
  4. Ragged rows: shorter → NULL padding (tested); longer → extra fields ignored silently ✗ (should warn).
  5. Header names with spaces, quotes, reserved words, duplicates (`id,id`) → duplicates produce duplicate target columns ✗.
  6. Header-less file whose first line looks like data → included in sampling (correct).
  7. BOM present (UTF-8/UTF-16) → stripped if it matches the chosen charset; a UTF-8 BOM read as Windows-1254 would leave garbage in the first header name ✗.
  8. Wrong encoding (Windows-1254 Turkish file read as UTF-8) → replacement characters, no detection ✗.
  9. Quoted fields with embedded delimiter, quote (`""`), CR/LF (CRLF normalized to LF) ✗ for byte-exact data.
  10. Backslash default escape: `"a\"b"` → `a"b`, which differs from RFC 4180 (`"a\""` …) and breaks round-trip of values ending in `\` (#22483) ✗. Custom escape `~` keeps a literal backslash (tested: `4,"value\""test"` → `value\"test`).
  11. Unterminated quote at EOF → error "Un-terminated quote sequence".
  12. Leading zeros (`007`, ZIP codes, phone numbers) → INTEGER → zeros lost ✗.
  13. Numbers with thousands separators (`1,234`), locale decimal comma (`3,14` — Turkish/German), currency (`₺12`), percentages → STRING or mis-split by delimiter; no decimal-separator option (#38997) ✗.
  14. Exponent notation `1e5` → REAL; hex `0x1F` → NVARCHAR (starts with digit, not Latin letter) ✗ category; Java-specific `1.5f`, `2d` → REAL ✗ (Java float suffixes accepted by `Double.parseDouble`).
  15. `NaN`, `Infinity` → VARCHAR; `+Infinity`, `-Infinity` → REAL ✗.
  16. A lone `-` or `.` → NVARCHAR ✗ (should be STRING).
  17. Integer just above 2^31 → BIGINT; above 2^63 → REAL (precision loss!) ✗ — should be DECIMAL.
  18. Decimal money values (`19.99`) → REAL (binary float) ✗ — should be DECIMAL with inferred scale.
  19. Mixed `1` and `true` in one column → order-dependent: `true` then `1` becomes INTEGER (numeric "wider" than boolean rank 0); `1` then `true` stays INTEGER; later `true` values then fail to insert ✗.
  20. `VARCHAR` vs `NVARCHAR` is decided by the **first** string sample only (equal commonality, not numeric → no change later): `"abc"` then `"Ölçü"` → VARCHAR; `"Ölçü"` then `"abc"` → NVARCHAR ✗ order-dependent; and a value starting with a digit but non-numeric (`2024-03-05`, `12abc`) is classed NVARCHAR.
  21. Dates/times are never inferred → VARCHAR unless the user changes the target type ✗.
  22. Booleans as `yes/no`, `Y/N`, `1/0`, `evet/hayır` → not booleans.
  23. Only the first 100 rows are sampled: a long string at row 10,001 overflows `VARCHAR(64)` → insert error mid-load ✗.
  24. Length rounding to the next power of two (37 → 64), counted in characters by default; byte length optional; capped at 32,767.
  25. Whitespace around values: kept unless trim is on; leading whitespace makes `" 12"` an NVARCHAR ✗.
  26. Empty string vs NULL: empty is a string unless "empty as NULL"; NULL marker (e.g. `\N` from MySQL dumps) must be configured.
  27. Timestamps with zone offsets (`+09:00`) → converted to instant (tested); date-only with offset → falls back to local date (tested); unparseable → string passed to the DB (tested).
  28. Daylight-saving gaps when shifting local times to a zone (nonexistent 02:30) — Java resolves by shifting forward. [Inferred]
  29. Delimiter auto-detection — not offered; TSV requires `\t` typed as delimiter.
  30. Very long single field (MBs) — held in memory as one string.
  31. Turkish header names `İl`, `ılçe` and case-insensitive matching to existing columns: Java `equalsIgnoreCase` treats `ı`, `I`, `i`, `İ` as equal in some pairs, so `il` may match `IL` *and* `İL`. [Observed: `CommonUtils.findBestCaseAwareMatch` uses `equalsIgnoreCase`; Inferred: Java char-wise case folding consequence] ✗ ambiguity.

- **Test cases (from DBeaver's tests):** [Observed: `TEST-PLAT/CSVImporterTest.java:65-194`; `TEST-DT/stream/StreamTransferResultSetTest.java:43-82`]

  | Input | Options | Expected |
  |---|---|---|
  | `a,b,c,d` | header none | names `Column1..Column4` |
  | `a,b,c,d` | header top | names `a,b,c,d` |
  | `1,2.0,abc,false` | header none | NUMERIC/INTEGER, NUMERIC/REAL, STRING, BOOLEAN |
  | `2147483648,-9223372036854775808,1` | header none | BIGINT, BIGINT, INTEGER |
  | `` (empty) | — | 0 columns |
  | `1\n\n2\n3\ntest` | header none | 1 column, STRING |
  | `,` | header none | 2 columns, both STRING |
  | `ID,TEXT_VALUE\n14\n` | header top, trim | one row `["14", null]` |
  | `"id","text_value"\n4,"value\""test"\n5,"one and two"` | escape `~` | rows `["4","value\\\"test"]`, `["5","one and two"]` |
  | `2024-03-05 10:15:30.123` | pattern `yyyy-MM-dd[ HH:mm:ss[.SSS]]` | timestamp 2024-03-05 10:15:30.123 |
  | `2024-03-05` | same | timestamp 2024-03-05 00:00 |
  | `2024-03-05 10:15:30` | same, zone UTC | local time shifted from system zone to UTC |
  | `2024-03-05 10:15:30 +09:00` | `yyyy-MM-dd HH:mm:ss XXX` | instant 2024-03-05T01:15:30Z |
  | `2024-03-05 +09:00` | `yyyy-MM-dd XXX` | local 2024-03-05 00:00 |
  | `not a date` | default | string `not a date` |

  Additional Hacivat tests (expected behavior after fixes): `007` → TEXT (leading zero rule); `19.99,5.00` → DECIMAL(4,2); `3,14` with decimal comma locale `tr` and delimiter `;` → DECIMAL; `true,1,false` → BOOLEAN only if the user enabled "1/0 as boolean", else TEXT; `Ölçü` → TEXT (no VARCHAR/NVARCHAR split; target-specific later); `2024-03-05` (all rows) → DATE; mixed `2024-03-05` and `05.03.2024` → TEXT with a hint "two date formats detected".

- **Prior art and evidence:** DBeaver above. DataGrip samples ~10,000 characters to detect the format, has named persistent CSV formats shared across copy/paste/import/export, a tolerant parser with big-field protection, and widens types Bool→Int→BigInt→Double→String. [Observed: `.refs/datagrip-extract/report-grid.md` §8]

- **Recommendation for Hacivat:**
  1. Use the `csv` crate (RFC 4180 by default, configurable escape, flexible record lengths, byte-records for speed).
  2. Detect encoding from BOM, else validate UTF-8 on the first 64 KiB, else offer candidates via `chardetng` with Windows-1254 (Turkish) prominent when the UI locale is `tr`; always show a preview before import.
  3. Sniff the dialect from a sample (delimiter among `, ; \t |` with the most consistent field count, quote char, header heuristic: first row all non-numeric and distinct while later rows contain numbers).
  4. Type inference as a lattice per column over the **whole file in a first pass** (or a configurable sample with a "scan entire file" option): `Empty < Bool < Int32 < Int64 < Decimal(p,s) < Float < Text`, plus a parallel `Date/Time/Timestamp(tz)` branch that collapses to Text on conflict; leading-zero rule (`0\d+` → Text); locale-aware decimal and thousands separators; track max char length and max byte length separately; produce a confidence and example value per decision.
  5. Order-independent merging (lattice join is commutative and associative), so shuffled files give the same types — make that a property test.
  6. Never classify by "first letter is Latin" — Unicode text is just Text; the *target* decides `NVARCHAR` vs `VARCHAR`.
  7. On load, keep a per-row error channel (row number, column, raw value, error) and a "rejects file" (DataGrip has an error file); don't abort the whole import on one bad row unless asked.
- **Used by:** Hacivat (import wizard, "open CSV as table", MCP tool `import_csv`), Karagöz (if import runs server-side for large files).

---

### ALG-DBV-XFER-04: Database load batching, commit interval, and error recovery

- **Problem:** Insert a stream of rows into a target table quickly (batches), safely (transactions), and recoverably (what happens when row 734,212 fails?).

- **Inputs / outputs:** row stream; target table and column mapping; options (use transactions, commit every N, multi-row insert batch size, disable batches, ignore duplicates, upsert method, bulk load, truncate first, disable FK checks). Output: statistics, error report.

- **Algorithm (DBeaver):** [Observed: `DT/database/DatabaseTransferConsumer.java:176-535, 542-625`]
  1. Open a target connection (isolated by default). Check "import data" permission. If `useTransactions`, switch to manual commit (remember the old mode); else force auto-commit.
  2. At the first rows callback: optionally truncate the target (only for existing tables, only at offset 0); build column mappings (skip pseudo columns, unchecked columns and — if configured — auto-generated columns); create per-column transformers; pick target value handlers.
  3. If bulk load is enabled and the driver offers a bulk loader → use it; else ask the target object for an `insertData` batch (a prepared `INSERT` with `?` placeholders, or multi-row `VALUES` if configured).
  4. For each row: read every source value with the source value handler; convert to the target type with the target value handler (`getValueFromObject`); run transformers; add to the batch; increment `rowsExported`; call `insertBatch(force=false)`.
  5. `insertBatch`: `needCommit = force || ignoreDuplicates || rowsExported % commitAfterRows == 0`. If bulk loading, flush when `needCommit`. Otherwise, when `needCommit` (or per-row mode), execute the batch with "reconnect and retry once" wrapping; on error:
     - unique-key violation with ignore-duplicates → skip;
     - "ignore all" chosen earlier → skip;
     - headless (CLI/CloudBeaver task) → fail;
     - else ask the user **Stop / Retry / Ignore / Ignore All**.
     Then, if transactions are used and nothing was cancelled, `COMMIT`.
  6. At end: `insertBatch(force=true)`, finish bulk load or close the batch; call `afterDataChange` hooks.
  7. Close: if still in manual mode, **roll back** anything uncommitted ("every uncommitted data here is considered as fail"), restore the old auto-commit mode, close the session and isolated connection.
  8. After the last pipe: run post-transfer actions (e.g. re-enable referential integrity), refresh the navigator, optionally open the table.

- **Data structures:** pending batch (rows × columns of converted values); counters; remembered "ignore all" flag.

- **Complexity / performance notes:** Throughput is dominated by round trips: a JDBC batch of 10,000 rows is usually one or a few network round trips. Memory: up to `commitAfterRows` converted rows held at once (10,000 × row width) — plus driver buffers — which explains import OOMs with wide rows (#11931). Multi-row `INSERT` statements of 500 rows can hit parameter limits (SQL Server 2,100 parameters per statement → 500 rows × 5 columns already exceeds it). [Inferred: SQL Server limit is documented]

- **Edge cases (exhaustive list):**
  1. Batch size equals commit interval: user cannot have "small batches, big transactions" or the reverse ✗.
  2. A failing batch with "Ignore" drops the whole batch (up to 10,000 rows), not just the bad row ✗. [Inferred: whole `executeBatch` is skipped]
  3. Some drivers continue after a failed row in a batch (MySQL with `continueBatchOnError`), others stop (PG) — the partial state differs per engine.
  4. PostgreSQL aborts the transaction after any error: after "Ignore", the following statements fail with "current transaction is aborted" unless a savepoint/rollback was issued ✗ [Inferred]. (Issue #41291 shows the same class of PG bug in another feature.)
  5. "Retry" after the server already applied part of the batch → duplicates.
  6. Cancel: monitor checked before each batch; uncommitted rows are rolled back at close, committed batches remain (partial import).
  7. Ignore duplicates forces commit per row → very slow.
  8. Auto-commit mode (`useTransactions=false`): every batch is permanent immediately.
  9. Identity columns: inserting explicit values needs `IDENTITY_INSERT ON` (SQL Server) / `OVERRIDING SYSTEM VALUE` (PG identity); sequences need resetting afterwards (`setval`) ✗ not handled generically. [Inferred]
  10. Foreign keys: load order (parents first) — DBeaver sorts pipes, but parallel jobs break the order; cyclic FKs need deferred constraints or disabled checks.
  11. Triggers on target fire per row.
  12. Truncate on a table referenced by FKs fails (PG needs `CASCADE`, SQL Server forbids `TRUNCATE` on referenced tables).
  13. Type conversion failures (text `abc` into int) happen inside the batch → reported per batch, not per row.
  14. Time zones on conversion (target session zone vs source).
  15. Same-table copy (source = target) → reading while writing.
  16. Very wide rows / LOBs → memory per batch.
  17. Lost connection mid-load → reconnect-and-retry may re-send a batch that was actually committed.
  18. Read-only target connection or restricted permissions → error before start.

- **Test cases (proposed for Hacivat; DBeaver has none for this logic):**

  | Scenario | Expected |
  |---|---|
  | 25,000 rows, batch 1,000, commit 10,000, no errors | 3 commits (10k, 20k, 25k); 25 batch executions |
  | Row 12,345 violates NOT NULL, policy "skip bad rows" | 24,999 rows loaded; rejects file has exactly row 12,345 with the server error |
  | Same with policy "stop" | rows 1–10,000 committed, rest rolled back; report says "stopped at row 12,345; 10,000 rows committed" |
  | PG target, error in batch, policy skip | savepoint per batch; after rollback-to-savepoint, the batch is retried row-by-row; transaction remains usable |
  | Cancel after 15,000 rows | 10,000 committed, 5,000 rolled back; report states both numbers |
  | SQL Server, 20 columns, multi-row VALUES | rows per statement ≤ floor(2100/20) = 105 |
  | Identity column copied | SQL Server: `SET IDENTITY_INSERT t ON` wrapped; PG: `OVERRIDING SYSTEM VALUE` + `setval` after load |

- **Prior art and evidence:** DBeaver above; DataGrip import uses batch 2,000 (`database.batch.size=2000`), an error file, and MSSQL `IDENTITY_INSERT` handling. [Observed: `.refs/datagrip-extract/report-grid.md` §8; `report-connectivity.md` §3]

- **Recommendation for Hacivat:** separate `batch_rows` (default ~1,000 or by bytes ≈ 1–4 MiB) from `commit_every` (default 50,000 or "single transaction"); use native fast paths (PG `COPY FROM STDIN BINARY`, MySQL multi-row `INSERT` sized by `max_allowed_packet`, SQL Server TDS bulk copy, SQLite one transaction with a prepared statement); on batch failure, fall back to row-by-row for that batch inside a savepoint to isolate bad rows; write rejects to a file; always report "committed rows" precisely; make the policy explicit (`stop | skip_row | skip_batch`). For audit (Karagöz), log one event per import with row counts, not per row.
- **Used by:** Hacivat, Karagöz (enforces write permission and records the audit event for imports that run through it).

---

### ALG-DBV-I18N-01: Message lookup with fallback

- **Problem:** Show every UI string in the user's language, falling back gracefully when a translation is missing, with placeholders filled in, without crashing, and (for Hacivat) without code changes when a language is added.

- **Inputs / outputs:** message key (+ arguments), requested locale; output: display string.

- **Algorithm (DBeaver, three paths):**
  - **A. Java UI strings (Eclipse NLS):** at class-load time, for class `XMessages` with bundle base name `B`, load the candidate files `B_<lang>_<COUNTRY>.properties`, `B_<lang>.properties`, `B.properties` (most specific first) and assign each `public static String` field from the first file that defines it; fields with no value anywhere get a "missing message" placeholder text. At use time the code reads the field directly (no lookup cost) and `NLS.bind` substitutes `{0}`, `{1}`… [Observed: `CoreMessages.java:406-409`; Inferred: Eclipse NLS fallback semantics]
  - **B. plugin.xml strings:** `%key` values resolved by the OSGi `BundleLocalization` service against `OSGI-INF/l10n/bundle[_locale].properties` with standard Java locale fallback. [Observed: `Bundle-Localization` headers; Inferred: OSGi spec behavior]
  - **C. Object property labels** (`ObjectPropertyDescriptor.getLocalizedString`, [Observed: `MODEL/../runtime/properties/ObjectPropertyDescriptor.java:649-704`]):
    1. If the annotation gives an explicit label (not the default marker), return it unchanged (not localized).
    2. Key = `meta.<DeclaringClass FQN>.<propertyName>.<name|description|hint>`; look it up in the declaring class's bundle localization for the locale.
    3. Missing → if `@Property(localizationOwner = X.class)` is set, try `meta.<X FQN>.<propertyName>.<type>` in X's bundle.
    4. Still missing → walk up superclasses; for each parent that declares the same getter **in a different bundle**, try `meta.<Parent FQN>.<propertyName>.<type>` there.
    5. Still missing → log at debug level for names and return the default (the property id for names, `null` for hints).
    6. If the found value equals the key itself, treat it as missing.
  - **Locale choice:** global setting `nl` → launcher sets `osgi.nl` (default `en`) at startup; change requires restart. [Observed: `DBeaverLauncher.java:2279-2291`]

- **Data structures:** per-bundle `ResourceBundle` caches; static fields in `*Messages` classes.

- **Complexity / performance notes:** NLS path is O(1) at use (fields). Property path does reflection and up to several bundle lookups per property descriptor, then caches the descriptor. Locale switch at runtime is impossible because static fields are initialized once.

- **Edge cases (exhaustive list):**
  1. Key missing in the target locale → fallback to English (good) — but English then appears mixed into a Turkish UI.
  2. Key missing everywhere → "NLS missing message…" placeholder text (NLS) or property id (properties).
  3. Stale keys in locale files (present in translation, removed from English) → ignored, but confuse translators (5–56 per locale observed).
  4. Placeholder mismatch (`{0}` missing or `{2}` added in translation) → wrong text or `MessageFormat` exception; DBeaver now validates placeholders in its sync PR (#42805).
  5. `MessageFormat` apostrophe rule: a single `'` starts a quoted literal, so `Don't {0}` breaks — translators must write `''`. French and Turkish text use apostrophes frequently (Turkish suffixes after proper nouns: `PostgreSQL'e`, `MySQL'den`) ✗ high risk for Turkish.
  6. Plurals: `.properties` + `MessageFormat` have no plural categories unless `choice` format is used; Turkish has `one/other` CLDR plural categories but the noun usually stays singular after numbers ("3 tablo"), while English needs "3 tables" — a format system must let each language decide.
  7. Grammatical case/suffix harmony in Turkish (`tabloya`, `şemaya`, `veritabanına`) depends on the last vowel of the *inserted* word: you cannot build "Connect to {0}" by concatenation; need selectors or rephrasing ("{0} bağlantısı açılsın mı?").
  8. Concatenated messages (code joins two translated strings) → wrong word order in SOV languages like Turkish and Japanese.
  9. Mnemonic `&` accelerators in menu labels; collisions within a menu after translation.
  10. Text expansion (German, Turkish ~30% longer than English) → truncated buttons.
  11. Right-to-left (Arabic) mirroring.
  12. Locale codes: non-standard `tw` vs `zh_TW`/`zh-Hant`; `pt_BR` vs `pt`; Turkish `tr` vs `tr_TR` must fall back properly.
  13. Encoding of the files (UTF-8 vs ISO-8859-1 with `\u` escapes).
  14. Hard-coded strings that bypass the system (restart dialog; #21384 series).
  15. Language change requires restart.
  16. Turkish case mapping: `"i".toUpperCase()` in a Turkish locale is `İ`, so code that upper-cases SQL keywords or identifiers with the *default* locale breaks (`select` → `SELECT` becomes `SELECT` with dotted İ). DBeaver code generally uses `Locale.ENGLISH` for this (e.g. `DBStructUtils.java` uses `toLowerCase(Locale.ENGLISH)`), which is the correct defensive habit. [Observed: `MODEL/struct/DBStructUtils.java:425`]
  17. Sorting/collation of translated lists (Turkish `ç` after `c`, `ı` before `i`).
  18. Date/number formatting must follow the UI locale independently of the message language (e.g. English UI with Turkish number format).

- **Test cases:**

  | Setup | Lookup | Expected |
  |---|---|---|
  | `en` has `a=Open`, `tr` has `a=Aç` | `a` in `tr` | `Aç` |
  | `tr` lacks `b`, `en` has `b=Close` | `b` in `tr` | `Close` (+ missing-key telemetry in dev builds) |
  | neither has `c` | `c` | dev: visible marker `⟦c⟧`; release: English source text from code-side default |
  | `tr_TR` requested, only `tr` exists | `a` | `Aç` |
  | `zh-TW` requested, file named `tw` | `a` | DBeaver: not matched by standard negotiation ✗; Hacivat: use BCP 47 `zh-Hant` |
  | Property `meta.X.name.name` missing, parent P (other bundle) has it | label | P's translation |
  | Message `Delete {0} rows?` with `$count=1` / `$count=5` | en | "Delete 1 row?" / "Delete 5 rows?" |
  | same in tr | tr | "1 satır silinsin mi?" / "5 satır silinsin mi?" |
  | Turkish apostrophe `{0}'e bağlan` (MessageFormat) | tr | DBeaver-style: apostrophe swallowed ✗; Fluent: literal apostrophe works |

- **Prior art and evidence:** DBeaver above. DataGrip (IntelliJ platform) uses `*Bundle.properties` with `DynamicBundle` and ships official language packs as separate plugins (Chinese, Japanese, Korean); no Turkish pack. [Inferred: general knowledge of IntelliJ localization; not in the DataGrip teardown notes]

- **Recommendation for Hacivat:**
  1. **Project Fluent** (`fluent-bundle`, `fluent-langneg`, `unic-langid` crates): one `.ftl` file per locale per area (`locales/en/grid.ftl`, `locales/tr/grid.ftl`), loaded at runtime from an embedded directory plus an optional user/override directory, so a new language = new folder of `.ftl` files + one entry in `locales/manifest.toml` (name in its own language, direction). No code change, no recompilation for community packs.
  2. Fallback chain via `fluent-langneg` negotiation: requested (`tr-TR`) → `tr` → `en`. English is always complete and compiled in.
  3. Fluent selectors handle plurals and Turkish suffix harmony through variables (`{ $object -> [table] tabloya *[other] nesneye }`), and apostrophes are plain text.
  4. Live language switching (re-render with a new bundle) — no restart.
  5. CI checks: every key in `en` exists in each shipped locale (or is explicitly allowed to fall back), no stale keys, variables used in translations ⊆ variables in English, no `{`/`}` syntax errors, UTF-8 only.
  6. Keep keys stable and descriptive (`grid-export-dialog-title`), avoid concatenation in code, and use `Locale::ENGLISH`-equivalent (ASCII-only) case mapping for SQL keywords/identifiers.
  7. Translation workflow: PRs on `.ftl` files + optional Weblate/Pontoon (Pontoon was built for Fluent); allow AI-assisted drafts only with human review, as DBeaver now does.
  8. Share the same `.ftl` sources between Hacivat desktop, Perde web (via `@fluent/bundle` in JS) and Karagöz messages that reach users.
- **Used by:** Hacivat, Karagöz (CLI/log messages shown to users), Perde (web UI, emails).

---

## 5. Implications for Hacivat / Karagöz / Perde

### 5.1 Architecture to copy (with changes)

| DBeaver idea | Keep? | Hacivat form |
|---|---|---|
| Producer / processor / consumer split, registered by id | Yes | Rust traits `RowSource`, `Format` (encoder/decoder), `RowSink`; registry keyed by id, so the MCP server and CLI can list formats. |
| Consumer = same row-callback interface as the grid | Yes | One `RowStream` type (async stream of column batches) used by grid, export, copy, MCP. |
| File source modelled as a fake database | Yes (as an adapter) | `CsvSource` implements `RowSource` with a schema; importing becomes "copy from source A to sink B". |
| Separate connection for long transfers | Yes | Per-task session from the connection actor (doc 01); honours "single session mode" for temp tables. |
| Turning off auto-commit to stream | Yes, engine-specific | PG: portal with `max_rows`; MySQL: streaming read; SQLite: step. |
| `LIMIT/OFFSET` segments | **No** | Cursor streaming; keyset pagination when chunks are required. |
| Batch = commit interval | **No** | Independent `batch_rows/bytes` and `commit_every`; savepoint isolation for bad rows. |
| Type mapping by name substrings | **No** | Logical type model + per-engine `to_logical/from_logical` + warnings + golden tests (ALG-01). |
| CSV formula guard and quoting rules | Yes | Port rules and test vectors with Apache-2.0 attribution. |
| CSV type inference from 100 rows | Improve | Whole-file lattice inference, dates, locale numbers, leading zeros, order-independent (ALG-03). |
| Tasks persisted as JSON per project | Yes | `tasks/` directory in the Hacivat project, human-readable, versioned schema; free local scheduler (cron-like) since this is open source. |
| Native dump tools through env passwords | Partly | Prefer `PGPASSFILE` temp file with 0600 or stdin; never env vars for secrets where avoidable; detect tools on PATH; support running them on the Karagöz host instead. |
| Event processors (open folder, run command, cleanup) | Yes | Post-export hooks; "run command" off by default and never available to MCP callers without explicit approval. |

### 5.2 Karagöz and Perde specifics

- **Server-side transfer:** Karagöz runs next to the database. Large exports/imports can run *on the agent* (database → file on the agent → compressed transfer to Hacivat over the P2P channel), avoiding row-by-row latency over the WAN. Imports can upload the file first, then load locally on the agent with `COPY`.
- **Authorization:** Karagöz must check "export"/"import" permissions per database and per table (DBeaver's equivalent is the connection-level "restricted by connection configuration" check). Exports are a data-exfiltration risk; Perde roles should have a separate `export` permission, with row limits.
- **Audit:** one audit event per transfer: who, what source (query text hash + text), target, rows, bytes, duration, outcome. Do not log row data.
- **Masking:** if Karagöz applies masking (see `access/` docs), the export path must use the same masked stream as the grid; DBeaver's "export from in-memory grid rows" shortcut shows why both paths must share one source.
- **Perde** never carries the data; it may store task definitions for team sharing (without secrets).

### 5.3 NoSQL

- DBeaver CE proves that a relational-only core leaves NoSQL as a second-class or paid add-on. Hacivat's transfer pipeline should define rows as `Value` records that can hold nested JSON/BSON documents, and formats should declare whether they support nesting (JSON/NDJSON yes, CSV no → flatten with dotted paths or serialize as JSON text, user's choice).
- Redis export/import: dedicated formats (e.g. NDJSON of `{key, type, ttl, value}`), not forced into tables.

### 5.4 Quality guardrails derived from DBeaver's issue history

1. UI thread never performs DB/network/file I/O; enforce with a lint/architectural test.
2. Every DB call has a timeout and a cancel path; lock waits surface as "waiting for lock held by PID …" with a cancel button (#29319).
3. Memory budget for result sets in the grid, with spill-to-disk and a clear "showing first N rows" banner (#34854).
4. Large script files are executed by streaming from disk without loading into the editor (#42570).
5. Navigator/tree virtualization and server-side filtering for 10k+ objects (#39861, #11555).
6. Golden tests for type mapping and import inference per engine pair (the untested DBeaver areas).
7. Drivers compiled in (Rust) → no runtime Maven downloads (#40237 class of bugs disappears); for optional native libraries (Oracle Instant Client) provide offline installers.

---

## 6. i18n considerations

1. **Turkish is a gap in DBeaver** (no `tr` locale in 15 shipped languages) — Hacivat shipping first-class English + Turkish is a real differentiator for Turkish users.
2. **Message format:** use Fluent, not `.properties` + `MessageFormat` (apostrophe trap, no plural/select). Turkish needs selectors for vowel-harmony suffixes and allows singular nouns after numbers.
3. **Locale codes:** use BCP 47 (`tr`, `tr-TR`, `zh-Hant`), never ad-hoc codes like DBeaver's `tw`.
4. **Case mapping:** SQL keyword/identifier upper/lower-casing must be locale-independent (ASCII rules or Unicode default case mapping, never the UI locale). Identifier matching for mapping columns should use: exact match → Unicode case-fold match (`caseless` or ICU4X `CaseMapper::fold`) → for Turkish text, show ambiguity (`I/ı/İ/i`) instead of silently picking the first match (DBeaver uses Java `equalsIgnoreCase`).
5. **Data formats in transfer:** CSV import must accept Turkish decimal comma and `;` delimiter (Excel in Turkish locale writes `;`-separated CSV with `,` decimals); exports should offer "Excel (Turkish)" presets. Encoding detection should consider Windows-1254 and ISO-8859-9.
6. **Export headers and generated SQL stay untranslated**: column names come from the database; generated keywords (`INSERT INTO`) are SQL, not UI text. Only dialog text, options and messages are localized.
7. **Translator workflow:** CI enforces completeness for `en` and `tr`; other community locales may be partial and fall back to English per key. Keep a glossary (connection = bağlantı, schema = şema, table = tablo, column = sütun, row = satır, commit = onayla/commit (decide once), rollback = geri al) to avoid DBeaver-style inconsistency fixed only in bulk later (#42805).
8. **Language switch without restart** (Fluent bundles are swappable at runtime).
9. **Accessibility of length**: design UI with ~35% text expansion; test with a pseudo-locale (`en-XA`-style accented, lengthened strings) in CI screenshots.

---

## 7. Risks, pitfalls, open questions

### 7.1 Pitfalls observed in DBeaver (avoid)

- `OFFSET`-based segment export degrades quadratically (#3568).
- Batch size coupled to commit interval; "Ignore" drops whole batches; per-batch error granularity.
- Type mapping heuristics with substring matching and catalog-order tie-breaks; reversed `canConsume` arguments in the similarity step; NUMERIC defaults to `int` on PG/SQL Server.
- CSV type inference: order-dependent VARCHAR/NVARCHAR and bool/int; no dates; leading zeros lost; `Double.parseDouble` quirks (`1.5f`); only 100 sampled rows.
- Text LOBs fully read into memory during CSV export.
- Partial JSON/XML files on errors unless a cleanup processor is enabled.
- Silent `?` substitution when exporting to a charset that cannot represent characters.
- Pipe dependency sort marked `fixme`, ignored under parallel jobs.
- Language switch requires restart; hard-coded English strings remain; non-standard `tw` code; reflection into Equinox internals.
- Secrets passed via environment variables to native tools.

### 7.2 Open questions

- [Unknown] Exact database list per paid edition (behind a link on dbeaver.com/edition).
- [Unknown] Which properties loader Eclipse NLS uses at this platform version (UTF-8 vs ISO-8859-1) — files are UTF-8 and the product evidently works, so the runtime must accept UTF-8.
- [Unknown] Where the Turkish UI text reported in #34432 came from.
- [Unknown] Whether `DBSDataManipulator.ExecuteBatch` clears its rows after a failed `execute` (decides whether "Retry" re-sends the same rows) — implementation is per-engine in `model.jdbc`/`ext.*`, not traced here.
- [Unknown] Behavior of `DBExecUtils.tryExecuteRecover` with partially executed batches after reconnect (risk of duplicates).
- For Hacivat: should large transfers default to running on Karagöz when the database is agent-attached? (Likely yes for > N MB; needs product decision.)
- For Hacivat: Arrow as the in-memory batch format (great for Parquet export and analytics, heavier dependency) vs a custom compact columnar page. Decide together with the grid design (`dbeaver-04`).

### 7.3 Licensing notes

- DBeaver CE and `dbeaver-common`: Apache-2.0. Porting CSV escaping rules and test vectors is allowed with attribution and a NOTICE entry.
- Elasticsearch JDBC driver referenced by DBeaver: Elastic License 2.0 (not OSI open source) — do not bundle in Hacivat.
- DataGrip: proprietary; only behavior was compared, from non-decompiling notes.

---

*Sources (web):* [DBeaver edition comparison](https://dbeaver.com/edition/), [Weblate DBeaver project](https://hosted.weblate.org/engage/dbeaver/), [CloudBeaver localization docs](https://dbeaver.com/docs/cloudbeaver/CloudBeaver-localization), DBeaver GitHub issues/PRs cited by number (`https://github.com/dbeaver/dbeaver/issues/<n>`, `.../pull/<n>`).
