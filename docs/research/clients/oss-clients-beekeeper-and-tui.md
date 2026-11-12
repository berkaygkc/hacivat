# Open-Source DB Clients — Beekeeper Studio, TUI clients (harlequin, rainfrog, gobang), and notable others

Status: research input. Date: 2026-11-11.
Grid algorithms (virtualization, auto-size, selection, clipboard) are specified in `ui-02-code-editor-and-data-grid-components.md` (ALG-GRID-01..04) and referenced here; this document adds algorithms for **result paging**, **pending edits → DML**, **statement risk classification / confirm-in-transaction**, and **query cancellation**.

---

## 1. Scope and sources

| Project | Version / commit | License | Notes |
|---|---|---|---|
| Beekeeper Studio (community) | app 6.2.2, commit `42eb053803e9f18bf152aab0888d166cfc9eb236` (2026-11-10) | **GPL-3.0** for community code; `**/src-commercial/**` under a **commercial license** (`LICENSE-COMMERCIAL.md`); `apps/ui-kit` dual **AGPL**/commercial | `.refs/repos/beekeeper-studio` — **prose and own pseudocode only** |
| harlequin | 2.17.0, commit `fb6af7b877f14009ecb11fd3a1bf619aec26634f` | MIT | `.refs/repos/harlequin` |
| rainfrog | 0.4.6, commit `cc460672c9698b396d8eefc288fcc657d2eefb3a` | MIT | `.refs/repos/rainfrog` |
| gobang | 0.1.0-alpha.5, commit `7b1b5f7eba3ea98a5d254a12ea31f383ee7737d1` (last commit 2023-01-25) | MIT | `.refs/repos/gobang` |
| Outerbase Studio (formerly LibSQL Studio) | GitHub API reads only | **AGPL-3.0** | prose only |
| Antares SQL | GitHub API metadata | MIT | locale list via API |
| Sqlectron GUI | GitHub API metadata | MIT | — |
| TablePlus / Postico / Navicat | public product pages/docs only | proprietary | **no reverse engineering**; observations tagged [Inferred: public docs/UX] |
| Tabulator | via Beekeeper dependency `tabulator-tables` ^6.7.1 | MIT | — |

Rules followed: GPL/AGPL/commercial sources described in prose; no code copied. MIT sources still described primarily in pseudocode; exact files noted for possible reuse with attribution.

---

## 2. Executive summary

1. **Beekeeper Studio** = Electron 44 + **Vue 2.7** renderer + DB drivers (knex, pg, mysql2, better-sqlite3, mssql, …) running in an **Electron utility process** connected to renderer windows via `MessagePort`s **[Observed: apps/studio/package.json:84-167,221; src/lib/utility/UtilityConnection.ts:18-42; src-commercial/entrypoints/main.ts:64,260]**.
2. Beekeeper's table view drives **Tabulator** with a fake AJAX URL and a custom request function that calls the database: page size 100 (config), fetch `limit + 1` rows to know whether a next page exists, offset pagination or opaque `pageState` tokens for databases without offsets (Cassandra-like) **[Observed: TableTable.vue:396, 434-436, 1243-1256, 2013-2100; default.config.ini:75]**.
3. Beekeeper edits are buffered as **pending inserts/updates/deletes** keyed by primary key; tables without PKs are read-only; sorting/filtering warns that pending changes will be discarded; save produces SQL (previewable) and applies it **[Observed: TableTable.vue:283, 390-396, 520-557, 1459-1500, 1829-1890]**.
4. Query editor: CodeMirror 6 (plus legacy CodeMirror 5), vim mode, statement identification with **sql-query-identifier**, parameters/placeholders, run current vs run all/selection, Esc to cancel, results capped by `maxResults = 50000` **[Observed: package.json:54-57,92,153; TabQueryEditor.vue:327-329, 702-703, 941-1000, 1447-1453, 1772-1817; default.config.ini:39-45]**.
5. Autocomplete config shows thoughtful details: keyword casing follows the typed prefix; identifiers quoted only when needed (special characters or mixed case on case-folding dialects) **[Observed: default.config.ini:47-66]**.
6. Saved queries: `FavoriteQuery` (title, text ≤ 2,000,000 chars, excerpt, database, connection hash, folder, float `position` for drag-ordering) and history `UsedQuery` (status, number of records, connection id) in a local app DB via TypeORM **[Observed: common/appdb/models/favorite_query.ts:9-45; used_query.ts:6-45]**.
7. **Beekeeper has no UI i18n** in the app source (no i18n library, no locale files) — only translated READMEs **[Observed: grep/find for i18n/locale in apps/studio; README-*.md files]**. Lesson: retrofitting i18n later is expensive (see RedisInsight's ongoing migration in the companion doc).
8. Plugins in Beekeeper run as iframes with a typed message API **[Observed: src/services/plugin/PluginManager.ts:32; types.ts:13,283]** — a good model for Hacivat's sandboxed UI plugins.
9. **harlequin** (Python/Textual, MIT): adapter plugins via Python entry points; result sets stored as **Arrow tables** in `textual-fastdatatable`; precise row-limit semantics: *hard* limit with a `+1` **overflow probe** and *soft* display caps, explicitly documented **[Observed: src/harlequin/plugins.py:4-79; query.py:35-110; pyproject.toml:26-28]**.
10. harlequin formats numbers with the OS locale (thousands separators) and warns when the locale is `C/POSIX` **[Observed: locale_manager.py:1-40]** — an example of "data locale" separate from UI language.
11. **rainfrog** (Rust, ratatui + sqlx + tokio, MIT): driver trait with **non-blocking start/poll** (`start_query`, `get_query_results` → `Finished | ConfirmTx | Pending | NoTask`), **server-side cancellation** (`pg_cancel_backend(pid)`), and **confirm-in-transaction** for `UPDATE/DELETE` (runs inside a transaction, shows rows affected, asks commit/rollback) while DDL (`ALTER/DROP/TRUNCATE`) requires confirmation before running **[Observed: src/database/mod.rs:101-160, 264-340; postgresql.rs:92-235; app.rs:359-374]**.
12. rainfrog converts every value to `String` at fetch time and uses the simple query protocol (`raw_sql(...).fetch_many`) **[Observed: postgresql.rs:602, 673-700]** — simple, but loses types (e.g., TIMESTAMPTZ normalized to UTC) and holds all rows in memory.
13. **gobang** (Rust, tui-rs 0.15 + sqlx 0.5, MIT) is unmaintained (last commit 2023-01); page size 200 with `LIMIT offset, n`, loads next page when selection reaches a page boundary; filter text is interpolated into SQL; copy joins cells with `,` without escaping **[Observed: src/database/mod.rs:12; sqlite.rs:236-247; app.rs:270; components/table.rs:200-215]** — instructive anti-patterns.
14. **Outerbase Studio** (AGPL): browser-first, driver abstraction including an iframe driver; DOM grid with 2-axis visible range calculation **[Observed: src/drivers listing; table-optimized/use-visibility-calculation.ts]**.
15. **Antares SQL** (MIT, Electron + Vue) ships **20 UI locales** including ar-SA and he-IL (RTL) — but **no Turkish** **[Observed: src/renderer/i18n listing via GitHub API]**.
16. TablePlus/Postico/Navicat feel "native and fast" because of: native widgets and text, instant startup, multi-tab connections with color-coded environments, safe-mode / pending-changes-commit workflows, keyboard-first navigation, and strict avoidance of loading full tables **[Inferred: public product pages/docs/UX]**.
17. Recommended Hacivat patterns distilled: (a) driver work off the UI thread with start/poll + cancellation; (b) `limit+1` probes and cursor/keyset paging; (c) pending-change buffer with SQL preview and optimistic concurrency; (d) risk-classified execution with confirm-in-transaction; (e) i18n from day one with Fluent.

---

## 3. Detailed findings

### 3.1 Beekeeper Studio

#### 3.1.1 Architecture

- Monorepo with `apps/studio` (Electron app), `apps/ui-kit` (Vue/Web Components UI kit: data editor, entity list, SQL/Mongo/Surreal text editors, table, tree, super-formatter) **[Observed: apps/ui-kit/lib/components listing]**.
- Renderer: Vue 2.7 + Vuex store; build via Vite/esbuild **[Observed: package.json:167; vite.config.mjs, esbuild.mjs]**.
- Main process entrypoint lives in `src-commercial/entrypoints/main.ts` (commercial license); it forks an Electron **utility process** and creates `MessageChannelMain` pairs to connect windows with the utility process **[Observed: src-commercial/entrypoints/main.ts:64, 260]**. The renderer-side `ElectronUtilityConnectionClient` implements the DB client interface by sending messages to the utility process; `UtilityConnection` holds the port per session id **[Observed: src/lib/utility/ElectronUtilityConnectionClient.ts:8; UtilityConnection.ts:18-42]**.
  - Hacivat analogue: Rust core in the Tauri process with async tasks; UI talks via commands/channels (ui-01 ALG-UI-01/02). Equivalent isolation without a separate Node process.
- Database clients: a `BasicDatabaseClient` abstract class with dialect-specific subclasses (postgresql, mysql, mariadb, sqlite, sqlserver, cockroach, redshift, bigquery, redis, tidb, starrocks, …) **[Observed: src/lib/db/clients listing]**. Abstract operations include `selectTop(table, offset, limit, orderBy, filters, schema, selects)`, `selectTopSql(...)`, `selectTopStream(...)`, `queryStream(query, chunkSize)` **[Observed: BasicDatabaseClient.ts:453-459]**.
- SSH tunnels via `ssh2`, local port picked between 10000–60000 **[Observed: src/lib/db/tunnel.ts:15, 211-226]**.
- Config is layered INI files: `default.config.ini`, `system.config.ini`, `user.config.ini`, `local.config.ini` **[Observed: apps/studio listing]** — user-tunable limits (page size, max results, widths, autocomplete casing).

#### 3.1.2 Connection handling (behavioral)

- Saved connections, connection folders, pinned connections, used connections, encrypted cloud credentials and token cache are separate app-DB entities **[Observed: common/appdb/models listing: saved_connection.ts, ConnectionFolder.ts, PinnedConnection.ts, used_connection.ts, CloudCredential.ts, token_cache.ts]**.
- Lost connection modal + connection interface components **[Observed: components/LostConnectionModal.vue, ConnectionInterface.vue]**.
- Per-dialect feature flags drive UI (`dialectData.disabledFeatures`, `usesOffsetPagination`, `disallowedSortColumns`) **[Observed: TableTable.vue:620-623, 2018]**.

#### 3.1.3 Table view (Tabulator)

- Tabulator configured with remote pagination: `pagination: true`, `paginationMode: 'remote'`, `paginationSize: limit`, custom pagination element, `ajaxURL: "http://fake"` with `ajaxRequestFunc` pointing to the component's data fetch **[Observed: TableTable.vue:1243-1256]**.
- Data fetch (behavior): computes offset from page number when offset pagination is supported, otherwise looks up a stored opaque page-state token for that page; requests `limit + 1` rows; if more than `limit` came back, sets `hasNextPage` and drops the extra row; if the returned field list differs from known columns, refreshes table metadata; stores new page-state tokens; resets pending changes; fills an internal index column from primary key values **[Observed: TableTable.vue:2013-2100]**.
- Initial sort: comment acknowledges ordering can be slow but some databases need an ORDER BY for LIMIT; can be disabled per dialect **[Observed: TableTable.vue:612-623]**.
- "Jump to last page" configurable per database (`allowSkipToLastPage`) — because COUNT(*) and deep OFFSET can be expensive **[Observed: TableTable.vue:430-433]**.
- Column widths config: `largeFieldWidth 300`, `maxColumnWidth 1000`, `minColumnWidth 100`, `maxInitialWidth 500`, `defaultColumnWidth 125`; data-type colors on by default **[Observed: default.config.ini:66-80]**.
- Copy uses Tabulator's range copy as plain text; paste maps clipboard values positionally onto visible data columns **[Observed: TableTable.vue:1124, 1702]**.
- Column state persistence via a `TabulatorPersistence` entity with an id composed of workspace, connection, database, schema and table **[Observed: TableTable.vue:612-616; models/TabulatorPersistence.ts]**.

#### 3.1.4 Editing model

- `pendingChanges = {inserts, updates, deletes}`; count shown in toolbar; saving disabled while running; tab marked "unsaved" when count > 0 **[Observed: TableTable.vue:49, 140-172, 390-396, 520-536, 745-746]**.
- Updates keyed by `(table, primary key values, column)`; editing a cell back to its original value removes the pending update; editing a row that is a pending insert modifies the insert instead; deleting a pending insert just discards it **[Observed: TableTable.vue:1544-1650, 1744-1823]**.
- Tables without PK: editing disabled (`entityType === 'table' && !primaryKeys.length`) **[Observed: TableTable.vue:557]**.
- Save: optional SQL preview (`applyChangesSql`) and apply (`applyChanges`) in the backend **[Observed: TableTable.vue:1844-1890]**.
- Sorting or filtering with pending changes triggers a warning that N pending changes will be discarded **[Observed: TableTable.vue:283]**.

#### 3.1.5 Query editor

- CM6 packages + `@uiw/codemirror-theme-*`; vim mode with ex-commands **[Observed: package.json:54-77; lib/editor/vim.ts, vimExCommands.ts]**.
- Statement identification via `sql-query-identifier` (MIT library) with dialect, identifying tables/columns and parameters **[Observed: lib/db/sql_tools.ts:2-3; TabQueryEditor.vue:1772-1797]**.
- Run modes: run current statement vs run tab/selection (configurable primary/secondary actions) **[Observed: default.config.ini:41-45; TabQueryEditor.vue:1677-1714]**.
- Cancel: Esc (or Ctrl-Esc in vim keymap) calls `runningQuery.cancel()` **[Observed: TabQueryEditor.vue:327-329, 998-1000, 1447-1453]**. Running a new query cancels the previous one **[Observed: TabQueryEditor.vue:1752-1753]**.
- Dry-run and active transaction flags passed to the query call **[Observed: TabQueryEditor.vue:1815]**.
- Result truncation flag propagated to UI (`truncated`) with `maxResults = 50000` **[Observed: TabQueryEditor.vue:2129; default.config.ini:40]**.
- Query edit history and merge/diff viewers exist **[Observed: components/editor/QueryEditHistory.vue, DiffViewer.vue, MergeManager.vue]**.

#### 3.1.6 Saved queries and history

- Favorites with folders and fractional `position` (insert between neighbors without renumbering) **[Observed: favorite_query.ts:34-37; QueryFolder.ts]**.
- Query text excluded from default selects (`select: false`) so listing is cheap; `excerpt` stored for lists **[Observed: favorite_query.ts:21-25]**.
- History records status, row count, connection id; `database` defaults to `'[blank]'` **[Observed: used_query.ts]**.
- `QueryAudit` entity exists **[Observed: models/QueryAudit.ts]** (relevant for Hacivat's local audit).

#### 3.1.7 i18n

- None in app code. All UI strings inline English (e.g., "Sorting or Filtering will discard … pending change(s)") **[Observed: TableTable.vue:283]**. READMEs translated into 11 languages **[Observed: repo root]**.

### 3.2 harlequin (MIT)

- Textual TUI; DuckDB default adapter; other databases via adapter plugins discovered with Python entry points group `harlequin.adapter` **[Observed: plugins.py:4-79]**.
- Adapter API: `HarlequinAdapter` → `HarlequinConnection.execute(query) → HarlequinCursor | None`, `cancel()`, `get_catalog()`, `get_completions()`; cursor `set_limit(n)` and `fetchall()` returning an Arrow-backed table **[Observed: adapter.py:36-141, 227]**; `IMPLEMENTS_CANCEL` toggles the cancel button **[Observed: app.py:457]**.
- Query execution in Textual workers (`@work(thread=True, exclusive=True)`) — exclusive workers cancel previous ones **[Observed: app.py:1574-1597]**.
- `RowLimit`: hard limit passed to the cursor; `detect_overflow` fetches one extra row to know whether results were truncated; soft caps (`display_limit`, layout `max_rows`) applied after fetch; `ResultSet` records `truncated`, `fetched_row_count` (excluding probe row), `elapsed` (fetch time excluding execution) **[Observed: query.py:35-110]**.
- Results viewer: column label shows name + short type; max column width = max(20, half the container − padding); record count formatted with locale grouping **[Observed: results_viewer.py:315-327]**.
- Export formats: csv/tsv (by extension, compression), parquet, json, orc, feather (fewer on Windows) **[Observed: copy_formats.py:36-343]**.
- Locale: number formatting via OS locale with `--locale` override **[Observed: locale_manager.py]**.

### 3.3 rainfrog (MIT)

- Stack: ratatui 0.30, crossterm 0.29, sqlx 0.8 (tokio), `ratatui-textarea` editor with search, arboard clipboard, optional DuckDB and Oracle **[Observed: Cargo.toml:41-119]**.
- Event loop: tick rate 4 Hz, frame rate 15 fps defaults, configurable **[Observed: tui.rs:64-105]**; DB results polled each loop via `get_query_results()` **[Observed: app.rs:359-374]**.
- `Database` trait (async, `?Send`) with start/poll/abort, transactions (`start_tx`, `commit_tx`, `rollback_tx`), menu/column loading as spawned `JoinHandle`s, `preview_rows_query` **[Observed: database/mod.rs:101-160]**.
- Cancellation: Postgres records the backend PID of the connection before running, and `abort_query` runs `SELECT pg_cancel_backend(pid)` on another connection, plus aborts the local task **[Observed: postgresql.rs:92-235]**. DuckDB uses a worker thread with an `AtomicBool` checked between row fetches and native interrupt **[Observed: duckdb.rs:19-199, 512-527]**.
- Statement classification with sqlparser: DDL (`ALTER INDEX/VIEW/ROLE/TABLE`, `DROP`, `TRUNCATE`) → **Confirm** before execution; `DELETE`/`UPDATE` → **Transaction** (execute in tx, show rows affected, confirm commit); `EXPLAIN ANALYZE` of a destructive statement inherits the inner statement's type (because ANALYZE executes it); parsing of Postgres `EXPLAIN (ANALYZE off)` options handled precisely **[Observed: mod.rs:255-340]**.
- Values: converted to strings per type; NULL → `"NULL"` with `is_null`; parse errors → `_ERROR_` **[Observed: postgresql.rs:673-700]**.
- Favorites and history components; vim keybindings; scroll table with selection modes **[Observed: src/components listing; scroll_table.rs]**.

### 3.4 gobang (MIT, unmaintained)

- tui 0.15, crossterm 0.20, sqlx 0.5 **[Observed: Cargo.toml:19-25]**.
- Paging: 200 rows per page via `LIMIT {offset}, {limit}` and next-page load triggered when the selected index hits a multiple of 200 **[Observed: database/mod.rs:12; sqlite.rs:236-247; app.rs:270]**.
- Filter text inserted directly in `WHERE {filter}` (by design: user types SQL) **[Observed: sqlite.rs:236; mysql.rs:234]**.
- Column paging (`column_page_start`) for horizontal navigation in a terminal **[Observed: components/table.rs:26-42, 222-240]**.
- Copy of selected range joins values with `","` without quoting **[Observed: table.rs:210]** — produces broken CSV for values containing commas/quotes/newlines.

### 3.5 Outerbase Studio (AGPL-3.0, prose only)

- Browser-first studio supporting SQLite/libSQL/Turso, MySQL, Postgres, with drivers including an **iframe driver** (host page provides query execution) — enables embedding the studio in other products **[Observed: src/drivers listing via GitHub API]**.
- Grid: DOM-based optimized table with fake padding rows/columns; visible range computed per scroll (rows by fixed height, columns by linear accumulation) **[Observed: table-optimized/*]**. See ui-02 §3.3.8.
- Notable ideas: saved docs/boards (`saved-doc`, `board-source`, `board-storage` drivers) — dashboards built from queries **[Observed: src/drivers listing]**.

### 3.6 Antares SQL (MIT)

- Electron + Vue; `vue-i18n`-style locale modules: ar-SA, ca-ES, cs-CZ, de-DE, en-US, es-ES, fr-FR, he-IL, id-ID, it-IT, ja-JP, ko-KR, nl-NL, pt-BR, ru-RU, uk-UA, uz-UZ, vi-VN, zh-CN, zh-TW, with a `supported-locales.ts` registry **[Observed: GitHub API listing of src/renderer/i18n]**. Notable: TypeScript locale files (code changes needed to add a language) vs. Hacivat's requirement of data-only language packs.

### 3.7 Sqlectron (MIT)

- Early Electron SQL client (lightweight, simple); split core (`sqlectron-db-core`) usable in terminal and GUI **[Inferred: project structure/README]**. Notable idea: shared DB core library between GUI and CLI — same as Hacivat's core crate used by app, CLI and MCP server.

### 3.8 TablePlus, Postico, Navicat (closed source; public observations only)

All statements **[Inferred: public product websites, documentation, and common UX knowledge]**:
- **Native toolkit** (TablePlus: native per OS; Postico: AppKit) → instant startup, native text rendering, native scroll physics, system menus and shortcuts.
- **Lazy everything**: table opens with a page of rows (e.g., 300/1000) and fetches more on scroll or via pagination; row count estimates from catalogs rather than `COUNT(*)`.
- **Safe editing workflow**: edits are staged (highlighted), committed explicitly (TablePlus "⌘S to commit"), with "Code review" / SQL preview of pending changes; "Safe mode" options that require confirmation or password for executing statements against production.
- **Connection color tags / environment labels** (production red, staging yellow) shown in window chrome — reduces mistakes.
- **Multiple tabs and windows per connection**, quick open (⌘P-style) for tables/views/functions, keyboard-first filters.
- **Inline filters** above the grid with per-column operators, generating `WHERE` clauses; quick "copy as SQL insert/update", "copy as CSV/JSON".
- **Row detail side panel** for wide rows/JSON; structure tab alongside data tab.
- Navicat: heavyweight feature set (modeling, data sync, scheduling, backup) — illustrates feature breadth but heavier UI.
- What makes them feel fast: never blocking UI on network; immediate visual feedback (skeleton rows), small fixed-size pages, keeping editors and grids responsive during queries, and native-level text/scroll rendering.

---

## 4. Algorithm specifications

### ALG-DATA-01: Table data paging with overflow probe, offset/keyset/opaque-token modes

- **Problem:** Browse a table or query result page by page (or with infinite scroll) without COUNT(*) on every view, without deep OFFSET costs where avoidable, and with correct "has next page" knowledge.
- **Inputs / outputs:** Inputs: target (table or query), page size `n`, sort spec (columns, directions, null ordering), filter spec, mode (`Offset` | `Keyset` | `Token`), current page cursor. Outputs: rows (≤ n), `hasNext`, next cursor, optional `estimatedTotal` (from statistics) and `exactTotal` (on demand).
- **Algorithm:**
  1. Choose mode: `Token` if the driver exposes opaque paging state (Cassandra-like) **[Observed: Beekeeper pageState handling]**; `Keyset` if the sort is a total order (sort keys include a unique key; append PK columns to the sort to make it total); otherwise `Offset`.
  2. Build the query requesting **n + 1** rows (overflow probe) **[Observed: Beekeeper limit+1; harlequin detect_overflow]**.
     - Offset: `… ORDER BY s LIMIT n+1 OFFSET k·n` (dialect-specific syntax: `FETCH FIRST`, `TOP`, `ROWNUM`).
     - Keyset: `WHERE (s1, s2, …, pk) > (v1, v2, …, vpk)` using row-value comparison where supported; otherwise expand to `s1 > v1 OR (s1 = v1 AND (s2 > v2 OR …))`; respect DESC per column by flipping operators; handle NULLs per `NULLS FIRST/LAST` semantics by splitting predicates.
     - Token: pass the stored token for the page.
  3. Execute with a statement timeout; on success: `hasNext = rows.len() > n`; drop the extra row; next cursor = (Offset: k+1) | (Keyset: sort-key values of the last kept row) | (Token: driver-returned token, appended to a token history to support "previous page").
  4. Totals: show `estimatedTotal` from catalog stats (e.g., `pg_class.reltuples`, `information_schema.TABLES.TABLE_ROWS`) labeled "≈"; compute exact `COUNT(*)` only on explicit user request, with timeout and cancellation.
  5. "Jump to last page": in Offset mode requires exact count (expensive) → allow per-dialect toggle **[Observed: Beekeeper allowSkipToLastPage]**; in Keyset mode implement by reversing the sort, fetching n rows, and reversing them.
  6. For infinite scroll (grid), map pages to ALG-UI-01 row windows: the core keeps fetched pages in the result store; the grid's total extent = fetched rows + (hasNext ? placeholder) until exhausted.
  7. Invalidate all cursors when sort or filter changes or after applying edits (generation bump).
- **Data structures:** `PageCursor` enum; token history `Vec<Token>`; per-table key info (PK/unique columns).
- **Complexity / performance notes:** Offset paging costs O(k·n) server-side for deep pages; keyset is O(log N + n) with a matching index. The probe costs one row.
- **Edge cases (exhaustive list):** table without PK/unique key (keyset impossible → Offset; editing read-only); sort on nullable columns (NULL handling in keyset predicates); sort on expressions or collations differing between client and server (keyset values must be compared server-side — never filter client-side); duplicates in sort key (append PK to make total order); concurrent inserts/deletes (Offset may skip/duplicate rows; Keyset stable); views/materialized views (no PK; offset only); databases requiring ORDER BY for LIMIT/OFFSET (SQL Server `OFFSET FETCH` requires ORDER BY) **[Observed: Beekeeper comment TableTable.vue:616-618]**; page size 0; last page exactly n rows (probe proves no next); empty table; very wide rows (page by bytes not only rows: reduce n when row size estimate × n > budget); binary/huge columns (exclude from default select or truncate server-side via `LEFT()`/`substring` for preview); filters with parameters (always bind, never interpolate — gobang anti-pattern **[Observed]**); time zone sensitive sort keys; collation-dependent string sort (Turkish collation `tr-TR` sorts "ı" differently — server decides).
- **Test cases (input → expected):**
  1. 250 rows, n = 100, Offset → pages of 100, 100, 50 with hasNext true, true, false.
  2. Exactly 200 rows → page 2 hasNext false (probe returns 100 rows only).
  3. Sort by `last_name` (non-unique) → engine appends PK; keyset predicate uses (last_name, id).
  4. Sort `created_at DESC NULLS LAST` with NULLs present → keyset continues correctly into NULL block.
  5. Concurrent insert of a row sorting before current page → keyset next page unaffected; offset page shows duplicate (documented).
  6. SQL Server without explicit sort → engine adds ORDER BY PK (or `(SELECT NULL)` when no PK, with warning).
- **Prior art and evidence:** Beekeeper data fetch (GPL; behavior only) **[Observed: TableTable.vue:2013-2100]**; harlequin RowLimit (MIT) **[Observed: query.py:35-63]**; gobang LIMIT offset paging (MIT) **[Observed]**.
- **Used by:** Hacivat (table view, MCP paged reads); Karagöz (enforces max rows per request policy server-side).

### ALG-EDIT-01: Pending change set → DML with optimistic concurrency and preview

- **Problem:** Let users edit cells, insert and delete rows in a grid; stage changes; preview the SQL; apply atomically; detect conflicting concurrent modifications.
- **Inputs / outputs:** Inputs: table metadata (PK/unique keys, column types, generated/identity columns, defaults), original row snapshots for edited rows, edits. Outputs: ordered list of parameterized statements, preview text, apply result per statement, refreshed rows.
- **Algorithm:**
  1. Identity of a row = PK values (or a chosen unique key; if none → read-only, optionally allow `ctid`/`ROWID` with warning per dialect).
  2. Change set entries: `Insert{temp_id, values}`, `Update{key, col → (old, new)}`, `Delete{key}`. Rules: editing a cell back to its original value removes that column from the update (and removes the update if empty) **[Observed: Beekeeper behavior]**; editing a pending insert modifies the insert; deleting a pending insert drops it; deleting a row with pending updates replaces updates with a delete; un-delete restores updates.
  3. Generate statements in order: deletes, updates, inserts (avoids unique-key collisions when users delete then re-insert the same key; configurable).
  4. **Optimistic concurrency** (per update/delete): `WHERE pk = :pk AND (col_i = :old_i OR (col_i IS NULL AND :old_i IS NULL))` for each modified column (and optionally all columns) — mirrors Compass's "update unless changed in background" query **[Observed: compass hadron-document generateUpdateUnlessChangedInBackgroundQuery, see companion doc]**. Exclude types without equality (json in PG, floating point approximations, large blobs) → fallback to PK-only with a warning or compare hash/`xmin` (PG) / rowversion (SQL Server) when available.
  5. Use `RETURNING`/`OUTPUT` (where supported) to refresh rows after apply; otherwise re-select by key.
  6. Execute all in one transaction; if any statement affects 0 rows under concurrency checks → abort transaction, mark conflict on that row, keep the change set for user resolution ("reload row", "overwrite", "discard").
  7. Preview: render statements with literals substituted *for display only* (dialect quoting), with a banner "parameters will be bound".
  8. After success: bump result generation (ALG-UI-01), clear change set, write local audit record (Hacivat) / send to Karagöz audit when remote.
- **Data structures:** `ChangeSet { inserts: IndexMap<TempId, Row>, updates: HashMap<Key, BTreeMap<ColId, (Old, New)>>, deletes: HashSet<Key> }`; undo stack of change-set diffs.
- **Complexity / performance notes:** O(#changes); batch inserts into multi-row INSERT up to parameter limits (PG 65535 params, SQL Server 2100, SQLite default 32766).
- **Edge cases (exhaustive list):** composite PKs; PK column edited (update must use old key in WHERE); generated/identity columns (omit from INSERT unless overriding); defaults (omit column vs explicit NULL — distinguish "unset" from NULL in insert UI); triggers modifying rows (RETURNING reflects them); case-sensitive identifiers and reserved words (always quote); NULL vs empty string; floating point equality in concurrency check; large text/BLOB edits (stream or size cap); timezone conversions on edit; editing a view (updatable views only); multiple tabs editing same table; transaction mode where user already has an open manual transaction (apply inside it, don't commit); read-only connections/roles (Karagöz denies — surface policy error); exceeding parameter limits; FK violations (report constraint name and row); MySQL `sql_safe_updates`.
- **Test cases (input → expected):**
  1. Edit `name` 'a'→'b' then 'b'→'a' → change set empty.
  2. Insert row then delete it before save → no statements.
  3. Update row whose `name` was changed concurrently by another session → 0 rows affected → conflict reported, transaction rolled back.
  4. Composite PK (a,b) with edit of `b` → `UPDATE … SET b=:new WHERE a=:a AND b=:old_b`.
  5. 10,000 inserts in PG → batched into ⌈10000×cols/65535⌉ statements.
- **Prior art and evidence:** Beekeeper pending changes (GPL, behavior) **[Observed: TableTable.vue]**; Compass conditional update (SSPL, behavior) **[Observed]**; TablePlus staged commits **[Inferred: public docs]**.
- **Used by:** Hacivat; Karagöz (policy check on generated DML).

### ALG-EXEC-01: Statement risk classification and confirm-in-transaction

- **Problem:** Prevent accidental destructive changes while keeping normal queries frictionless.
- **Inputs / outputs:** Inputs: parsed statements (dialect parser), connection safety level (`dev`/`staging`/`production` tag), user settings, Karagöz policy. Output per statement: `Normal | ConfirmBefore | RunInTransactionThenConfirm | Deny` and a human-readable reason (Fluent id).
- **Algorithm:**
  1. Parse statements; if parse fails → treat as `ConfirmBefore` on production-tagged connections, `Normal` otherwise (configurable).
  2. Classify (rainfrog baseline **[Observed: mod.rs:264-340]**, extended):
     - DDL `DROP`, `TRUNCATE`, `ALTER … DROP`, `ALTER` of tables/indexes/views/roles → `ConfirmBefore` (cannot be safely previewed in tx on MySQL where DDL auto-commits; in PG DDL is transactional → could use tx mode).
     - `DELETE`/`UPDATE` → `RunInTransactionThenConfirm` (show affected row count; optionally sample of affected rows via `RETURNING` limited to 100).
     - `DELETE`/`UPDATE` **without WHERE** → elevate: `ConfirmBefore` + require typing table name on production.
     - `INSERT … SELECT`, `MERGE`, `COPY FROM` → `RunInTransactionThenConfirm` where supported.
     - `EXPLAIN ANALYZE <stmt>` (incl. PG option form) → class of `<stmt>`; plain EXPLAIN → Normal **[Observed]**.
     - CTEs with data-modifying sub-statements (`WITH d AS (DELETE …) SELECT …`) → class of the most dangerous inner statement **[Observed: rainfrog inspects SetExpr Update/Delete inside queries, mod.rs:366-367]**.
     - `GRANT/REVOKE/CREATE USER` → ConfirmBefore.
     - Procedure calls (`CALL`, `EXEC`) → unknown side effects → ConfirmBefore on production.
  3. Connection tag adjusts: dev → only DROP/TRUNCATE confirm; production → full rules; user may disable per connection (except Karagöz-enforced policies).
  4. In transaction mode: begin; execute; capture rows affected; show confirm dialog (commit/rollback) with a timeout that auto-rolls back (e.g., 5 minutes) to avoid holding locks indefinitely; warn that locks are held while the dialog is open.
  5. Karagöz policy `Deny` overrides everything (enforced at Karagöz, not only in UI).
- **Data structures:** classification enum; policy table.
- **Complexity / performance notes:** parse cost negligible vs. execution.
- **Edge cases (exhaustive list):** dialect differences in transactional DDL; autocommit off/on; user already in manual transaction (nested → use savepoint); multi-statement batch with mixed classes (confirm once with summary); statements in comments/strings (parser handles); vendor-specific destructive commands (`FLUSHALL` in Redis, `db.dropDatabase()` in Mongo — separate classifiers); triggers causing cascades (row counts underestimate); `ON DELETE CASCADE` (warn: affected rows in other tables not counted); very long-running tx while confirm open (lock waits for other sessions); connection drop while awaiting confirm (server rolls back; inform user).
- **Test cases (input → expected):**
  1. `DELETE FROM t WHERE id=1` on production → tx mode, dialog shows "1 row affected".
  2. `DELETE FROM t` → ConfirmBefore + type-to-confirm.
  3. `EXPLAIN (ANALYZE off) DELETE FROM t` → Normal **[Observed: has_analyze_option semantics]**; `EXPLAIN (ANALYZE) DELETE …` → tx mode.
  4. `WITH x AS (UPDATE t SET a=1 RETURNING *) SELECT * FROM x` → tx mode.
  5. MySQL `DROP TABLE t` → ConfirmBefore (no tx preview).
- **Prior art and evidence:** rainfrog (MIT) **[Observed]**; TablePlus safe mode **[Inferred]**.
- **Used by:** Hacivat (UI), Karagöz (policy enforcement on remote execution).

### ALG-EXEC-02: Query cancellation (client task + server-side cancel)

- **Problem:** Stop a running query promptly and reliably, freeing server resources and keeping the session usable.
- **Inputs / outputs:** Inputs: running query handle (task id, connection id, server session id/PID/thread id), driver capabilities. Output: `Cancelled | AlreadyFinished | CancelFailed(reason)` and session state.
- **Algorithm:**
  1. Before execution, record the server-side session identifier: PG backend PID (or use the protocol cancel key via `tokio-postgres` `CancelToken`), MySQL `CONNECTION_ID()`, SQL Server SPID, ClickHouse `query_id` (set explicitly), MongoDB `opId`/`comment` tag for `killOp`, Redis (blocking commands → `CLIENT KILL`/`CLIENT UNBLOCK`).
  2. On cancel: (a) send the server-side cancel via a **separate** connection/protocol channel (`pg_cancel_backend`/CancelRequest **[Observed: rainfrog uses pg_cancel_backend]**, `KILL QUERY <id>` for MySQL, attention signal for TDS, `KILL QUERY WHERE query_id=…` for ClickHouse, `killOp` for Mongo); (b) concurrently stop consuming the client stream and abort the local task; (c) for embedded engines (SQLite/DuckDB) use the engine interrupt API plus a cancellation flag checked between rows **[Observed: rainfrog duckdb.rs AtomicBool + native interrupt]**.
  3. Wait up to T (e.g., 2 s) for the driver to report cancellation; if the session is in an unknown state (protocol mid-stream), discard the connection from the pool and reconnect transparently.
  4. If inside a user transaction, report that the transaction is now aborted (PG) and must be rolled back.
  5. UI: cancel button + Esc; never block the UI thread awaiting cancellation (rainfrog notes this TODO **[Observed: duckdb.rs:70-73]**).
- **Data structures:** `RunningQuery { task: JoinHandle, cancel: CancelHandle, started_at, conn_id }`.
- **Complexity / performance notes:** O(1).
- **Edge cases (exhaustive list):** query finished between click and cancel (race → AlreadyFinished); cancel during result streaming (client side only needed); cancel arrives at the server for the *next* query (PID reuse race — use protocol cancel keys or query ids rather than PID-only when possible); network partition (cancel channel unreachable → drop connection); P2P via Karagöz (cancel must be forwarded to Karagöz, which owns the DB connection); pooled connections; permissions lacking for `pg_cancel_backend` on other roles' sessions (same role is allowed); MySQL `KILL QUERY` requiring privileges; long DDL not interruptible.
- **Test cases (input → expected):** `SELECT pg_sleep(60)` cancelled at 1 s → returns ≤ 2 s, session usable; DuckDB long scan cancelled → returns, session preserved **[Observed: rainfrog test cancellation_interrupts_native_work_and_preserves_the_session]**; cancel after completion → AlreadyFinished.
- **Prior art and evidence:** rainfrog (MIT) **[Observed]**; Beekeeper `runningQuery.cancel()` (GPL, behavior) **[Observed]**; harlequin `IMPLEMENTS_CANCEL` + exclusive workers **[Observed]**.
- **Used by:** Hacivat; Karagöz (remote execution endpoint must implement cancel).

Grid algorithms: see ui-02 ALG-GRID-01 (virtualization), ALG-GRID-02 (auto-sizing), ALG-GRID-03 (selection), ALG-GRID-04 (clipboard).

---

## 5. Rust-specific learnings (from rainfrog, gobang, harlequin's design)

1. **Never block the render loop on I/O.** rainfrog's start/poll model (spawn tokio task, poll each frame) keeps the TUI responsive **[Observed]**; in Tauri, commands are async and events stream progress — same principle.
2. **Typed values end-to-end.** Converting to `String` at fetch time (rainfrog/gobang) loses type info needed for editing, sorting, clipboard fidelity and NULL vs empty distinction. Hacivat: decode into Arrow arrays with per-dialect type mapping; keep original textual representation for exotic types.
3. **Use the extended protocol / prepared statements** for parameters; use simple protocol only for multi-statement scripts typed by users (with server-side splitting awareness).
4. **Stream results** (`fetch` as a stream) with backpressure into the result store; apply hard limits server-side (LIMIT injection only when safe, otherwise stop consuming and cancel).
5. **Session state matters**: timezone (TIMESTAMPTZ display), search_path, transaction state — show them in the status bar; never silently normalize to UTC (rainfrog does) **[Observed]**.
6. **Cancellation is per-driver work** — design the driver trait with explicit `cancel()` capabilities (as rainfrog's trait comment says: "Stopping work in the database … must be implemented by the driver") **[Observed: mod.rs:127-129]**.
7. **Plugin drivers**: harlequin's entry-point adapters show the value of a stable adapter API; for Rust, consider a WASM or out-of-process (gRPC/stdio) driver protocol for third-party databases rather than dynamic libraries.
8. **Terminal UIs are a useful second frontend** (SSH sessions on servers where Karagöz runs) — the core/UI split (ui-01 ALG-UI-02) enables a ratatui frontend later.

---

## 6. Implications for Hacivat / Karagöz / Perde

- **Hacivat**: implement ALG-DATA-01, ALG-EDIT-01, ALG-EXEC-01/02 in the core; environment color tags on connections (production red) shown in window chrome and tabs; pending-changes workflow with SQL preview; history and favorites with folders and fractional ordering; query text stored separately from list metadata; layered config with documented limits (page size, max results) like Beekeeper's INI but in TOML.
- **Karagöz**: implements the server side of cancellation (ALG-EXEC-02) and enforces ALG-EXEC-01 `Deny`/confirm policies for remote sessions; records audit events with classification.
- **Perde**: shows audit events with classification and the confirm decisions; manages environment tags centrally for org connections.
- **Plugins**: Beekeeper-style iframe plugins with a typed postMessage API and capability manifests.

---

## 7. i18n considerations

- Beekeeper and Compass ship English-only UIs; RedisInsight is retrofitting i18n (see companion doc) — Hacivat must externalize strings from the first commit (Fluent), with a lint rule forbidding string literals in UI components (allow-list for data-like strings).
- Antares stores locales as TS modules — Hacivat stores `.ftl` data files loaded at runtime so adding a language needs no code change.
- Data locale vs UI locale (harlequin's number formatting example **[Observed]**): Hacivat settings expose both.
- Confirmation dialogs (ALG-EXEC-01) must use Fluent plurals (`{ $count -> [one] row *[other] rows }`); Turkish plural rules (one/other) handled by CLDR.
- Type-to-confirm prompts compare identifiers exactly (no locale case-folding — Turkish `İ/i` issues).
- Keyboard shortcuts documented per layout (Turkish Q/F).

---

## 8. Risks, pitfalls, open questions

- Pitfall: deep OFFSET paging on large tables; prefer keyset where possible.
- Pitfall: COUNT(*) on every open (TablePlus/Beekeeper avoid; Hacivat must too).
- Pitfall: string-typed values (rainfrog/gobang) — avoid.
- Pitfall: SQL built by string interpolation for filters (gobang) — always bind parameters; for user-typed raw WHERE fragments, label clearly as raw SQL.
- Pitfall: unescaped CSV copy (gobang).
- **[Unknown]** Exact Beekeeper behavior of `applyChanges` concurrency checks (backend code not traced in detail).
- **[Unknown]** Outerbase iframe driver security model details (AGPL code not studied deeply).
- Open question: should Hacivat offer "confirm-in-transaction" for MySQL DML given autocommit defaults? (Yes, tx works for DML; DDL needs ConfirmBefore.)
