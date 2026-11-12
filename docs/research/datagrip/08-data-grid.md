# 08 — Data Grid (DataGrip 2024.3.5)

> Part of the Hacivat research library. Conventions: `../README.md`. Related: 07 (execution engine feeding the grid, result tabs, transactions, in-editor results), 09 (schema/data compare and DDL; Modify Table for default/NULL settings), 10 (Export/Import from the Database Explorer, native dump tools, data loaders in the tools context, Execute to File), 11 (settings DV-*, CSV-*, URL click settings, advanced settings), 12 (locale formatting, plural/choice messages), 13 (families PASTE, CSV, WIDEN, DML, PAGE, FILTER). Introspection model (keys, columns, types, virtual FKs) is documented in 00–05; sessions and transaction plumbing in 03.

---

## 1. Scope and sources

**Scope.** Everything about the data editor and viewer ("the grid"): where it appears and what each surface can do; data model and indices; row identity; paging, fetching, sampling, auto-refresh; LOB truncation; editing, cell editors per type, validation and pending changes; DML generation and submission; transactions from the grid; value editor, record view, aggregate view and status-bar aggregator; display types and presentation; server and local filtering, quick filters, find-in-page; sorting; column operations; selection model; view modes (table/transpose/tree/text/chart/geo) and the column statistics header; copy/paste and paste formats; extractors, aggregators and loaders (full scripting API surface); Export Data and Import dialogs (every option); CSV Formats dialog and "file as table"; FK navigation (Related Rows, referenced/referencing results, quick documentation); data compare; full action/shortcut inventory.

**Sources.**

| Source | Used for | License |
|---|---|---|
| Plugins `grid-core-impl`, `grid-impl`, `DatabaseTools` non-class resources (`META-INF/plugin.xml`, `DatabasePlugin.xml`, bundles `DataGridBundle` (393 keys), `GridCsvBundle`, `DatabaseBundle`; searchable-options indexes; `extensions/data/{extractors,aggregators,loaders}` Groovy/JS scripts and their header comments; `extensions_old/*` hashes; registry keys) — extracted under `.refs/datagrip-extract/{gridimpl,gridcore,dbplugin}` and `.refs/scratch-extracts/bundles/en` | actions, shortcuts (`keyboard-shortcut` / `use-shortcut-of`), labels, messages, settings, scripting API, defaults | Proprietary (described, never copied) |
| Platform keymaps `lib/app.jar` → `keymaps/$default.xml`, `Mac OS X 10.5+.xml` (non-class resources) | resolving `use-shortcut-of` references to concrete keys | Proprietary platform resource; same keymaps exist in IntelliJ Community (Apache-2.0) |
| Class-name inventories `grid-core-impl.txt` (501), `grid-impl.txt` (889), `database-plugin.txt`, `jdbc-console.txt` | mechanism names only (`MutationsStorage`, `GridPagingModel`, `TypeMerger`, `CsvFormatParser`, `ColumnFilterByGroup$*`, `*RowsGenerator`, `StatisticsTableHeader`, `GridAutoRefresher`, `Default*EditorFactory`, `BinaryDisplayType`, `NumberDisplayType`, `XlsxValuesExtractor`, nested-table models) | names only |
| Offline help (DataGripHelp plugin, Oct 2024; text under `.refs/scratch-extracts/help/txt`): `data-editor-and-viewer`, `tables-view-data`, `tables-filter`, `tables-sort`, `cells`, `rows`, `columns`, `submitting-and-reverting-changes`, `compare-data`, `data-extractors`, `data-loaders`, `export-data`, `import-data`, `import-from-excel-files`, `editing-csv-and-tsv-files`, `viewing-query-results`, `settings-tools-database-data-views`, `settings-tools-database-csv-formats`, `full-text-search-for-databases` | documented behavior and option lists | JetBrains docs |
| Help screenshots (`help-datagrip-assets.jar`, viewed only): Export Data dialog, Import dialog (mapping settings), find-in-grid bar | defaults visible in UI (unchecked export options, "Write errors to file" checked with temp path, `<Auto> (C1)` source columns, find bar toggles) | JetBrains docs |
| Prior notes `.refs/datagrip-extract/report-grid.md` (Turkish) | starting map; re-verified | internal |

---

## 2. Executive summary

1. **Three layers**: `grid-core-impl` (DB-agnostic model: `GridModel`, paging/sorting/filtering models, mutations, CSV engine, extractors, type converters), `grid-impl` (Swing UI, value editor, local filter, aggregates, charts, statistics header, auto-refresh), DB plugin (sessions, dialect DML generators, FK navigation, data diff, import/export dialogs). [Observed]
2. **Seven surfaces share one grid**: table editor, result tab (Services), in-editor result, DSV file "Data" tab (editable), tabular-file Data tab via scripted loader (read-only), import preview, compare viewer — with a documented capability matrix (WHERE/ORDER BY fields, *Filter by* and *Sort via ORDER BY* exist only in the table editor; auto-transpose never applies to query results). [Observed: help]
3. **≈140 grid actions** with stable ids `Console.TableResult.*` / `TableResult.*`; most default keys are *borrowed* from platform actions via `use-shortcut-of` (Add Row = Generate = Alt+Insert/⌘N; Clone Row = Duplicate = Ctrl+D; Delete Rows = Delete Line = Ctrl+Y/⌘⌫; Go to Row = Go to Line = Ctrl+G/⌘L; Reload = Refresh = Ctrl+F5/⌘R). Grid *Submit* reuses *Execute*'s key (Ctrl/⌘+Enter); *Commit* and *Submit and Commit* share Ctrl+Alt+Shift+Enter. [Observed: plugin.xml + keymaps]
4. **Paging**: default page **500**; presets + "All" + custom; *Set Default Page Size*; *Count Rows* runs `SELECT COUNT(*)` on click ("of N"); *Change Table Sampling* by percent; *Auto-Refresh* with preset/custom interval (seconds), Pause, Disabled; floating pagination control position is configurable. Table editors page with `LIMIT/OFFSET` (`database.grid.paging.offset=true`). [Observed]
5. **LOBs**: per-value byte limit; truncated cells display "{0} of {1}"; partial copy warns with a link to change the limit ("Change Size Limit" → "Apply and Reload"); cloning truncated rows is refused. [Observed]
6. **Editing is an overlay** (`MutationsStorage`); invalid text stays as an *unparsed value* ("Incorrect value") with typed errors (*Expected boolean value*, *Invalid number*, *Expected timestamp…*, *Invalid UUID*, *Invalid bit string*, *Expected JSON*, *Expected object ID*…); out-of-range values prompt "Value Is out of Range. Set "{0}"?". [Observed]
7. **Cell editors per type**: text (with value completion and optional immediate completion), numeric, boolean (text dropdown with single-letter typing `t/f/d/n/g/c` or checkbox), date (calendar), time, timestamp, temporal-with-zone (offset −12:00…+14:00), binary/blob (text or hex), UUID-binary, bit strings, JSON, Mongo ObjectId, ranges; all format-based editors are lenient parsers bound to the display format. [Observed: class names, bundle]
8. **Submit**: pending changes → DML tasks (`UpdateRowTask`, `MultiInsertRowsTask`, `DeleteRowsTask`, `SelectUsingKeys`…) rendered by dialect generators; *Preview Pending Changes* shows SQL; Auto vs Manual tx; *Submit changes immediately* setting; leaving with unsubmitted changes asks "Changes are not submitted. Data will be lost. Continue?". [Observed]
9. **Filtering**: WHERE field (history, completion, `Ctrl+Alt+Shift+F`), *Filter by* quick predicates (operands from cell, clipboard, NULL, placeholder; LIKE and NULL templates), **local filter** (Excel-like per-column value/count checklist, "Local Filter By Cell Value", remembered toggle, can be on by default), **find on page** with Match case / Regex / Words and *Filter rows*. [Observed]
10. **Sorting**: header click sends ORDER BY (table editor) or sorts locally; unstacked unless Alt+Click (gesture configurable); level numbers; keyboard Alt+Shift+Up/Down, add-asc/desc, clear (Ctrl+Alt+Shift+Backspace); default sort by numeric PK asc/desc. [Observed]
11. **Views**: Table/Tree/Text (+ Transpose), Record view (1/2-column), Value editor (Value/Record/Aggregates tabs, dock bottom/right), Chart (9 types, split vs replace), Geo Viewer (JCEF), **column statistics header** (Off/Compact/Detailed). [Observed]
12. **Display types** for binary (Detect, Hex, Hex and ASCII, UUID, UUID with time-low/high swapped, Text) and numeric (Number, Timestamp seconds, Timestamp milliseconds) columns; per-column/cell highlighting language. [Observed: bundle]
13. **Copy/paste**: Copy = current extractor ("Copy Using Data Extractor ({0})"); paste format = *Detect* / *Single Value* / any CSV format, with notifications naming the detected format. [Observed]
14. **Scripting**: extractors (`COLUMNS, ALL_COLUMNS, ROWS, OUT, FORMATTER, TRANSPOSED, TABLE, DIALECT, PROJECT`), aggregators (same API, single value), **loaders** (`LOADER.load{ctx}` with `FILE` parameter and a data consumer: `consumeColumns(names, types)`, `consume(row)`; header metadata `extensions`, `displayName`, `tableFirstFormat`; dependencies fetched with `@Grab`). [Observed]
15. **Export Data dialog**: Source, Extractor, Add columns (Computed/Generated), Transpose, Add table definition (DDL), Add column header, Add row header, Output query to separate sheet (xlsx), Output file/directory, preview (none for binary), Copy to Clipboard / Export to File(s). [Observed]
16. **Import dialog**: mapping tree (file/tabular/table/query sources → table mappings), per-source settings (path, charset, format/loader, headers, trim, column types), per-mapping settings (schema, table new/existing, column mapping `<Auto>`, encoding, write errors to file, inconvertible → NULL, disable indexes/triggers + lock table), Data/DDL preview with editable DDL. [Observed]
17. **CSV Formats**: predefined CSV, TSV, Pipe-separated, Semicolon-separated; options value/row separator, NULL text (Undefined/Empty string/custom), row prefix/suffix, ordered quotation patterns (left/right/escape `<duplicate>` or char), Quote values (Never/When needed/Always), Trim whitespaces, First row/column is header; 10-record preview. [Observed]
18. **Related Rows** (F4) navigates FKs/virtual FKs both ways (multi-value, "Navigate and filter all" default); quick doc shows referenced/referencing records; *Go to Referenced Result* / *Referencing Result* navigate nested results (cursors, nested tables). [Observed]
19. **Compare Data** with Tolerance and Detect column insertion; rows limited by page size. [Observed]
20. Hacivat: Arrow-backed pages, PK-keyed pending changes, parameterized DML with affected-row checks and re-read, keyset paging, sandboxed extractor/loader plugins; ALG-GRID-01..23 below.

---

## 3. Detailed findings

### 3.1 Architecture and data model

- `GridModel<Row, Column>` with listeners; `GridRow`, `GridColumn` (`JdbcGridColumn`, `JdbcColumnDescriptor`), `DataConsumer$Row/$Column` fed by loaders; `GridListModelBase`, `DataGridListModel`; nested documents via `GridModelWithNestedTables`, `HierarchicalColumnsDataGridModel` (`ColumnNamesHierarchyNode`), `DynamicNestedTable`, `StaticNestedTable`, `NestedTablesDataGridModel` (row/column cache strategies), `NestedTableGridPagingModel$StaticMultiPageModel`, `NestedTablesSortingModel`. [Observed: names]
- Every data source is a **`GridDataHookUp`** = loader + paging model + sorting model + filtering model + mutator: `DatabaseTableGridDataHookUp` (table editor), `ConsoleGridDataHookUp`/`DataBusGridDataHookUp` (query results), `CsvDocumentDataHookUp`/`DocumentDataHookUp` (CSV/text file edited through the grid and written back to the document), `DasGridDataHookUp`, `JpaGridDataHookUp`, `ImmutableDataHookUp` (read-only, e.g. loader files), nested-table hook-ups. [Observed]
- Requests: `GridDataRequest` (with `Context`, `DatabaseContext`, owner) tagged with `GridRequestSource(RequestPlace, Phase)` so the UI knows which component triggered a reload. Hook-up manager: `GridDataHookUpManager` / `DbGridDataHookUpManager`, `CachedGridDataHookUp`. [Observed]
- Types: `LogicalType`; `DataConverter` graph (BinaryToText, BitStringToText, BooleanToNumber/Text/Binary, DateToNumber/Text/Timestamp, MapToText, MoneyToText, NumberToText/Timestamp, ObjectToText, StringUuidToText, TimeToNumber/Text/Temporal, TimestampToTemporal/Text, UuidToText); `Compatibility` (Full, CompatibleLossy, IncompatibleLossless, IncompatibleLossy, Incompatible) for paste/import conversions. [Observed]
- **Domains** (`Domain`, `DomainRegistry` with `CommonLimits`, `VersionedDomainRegistry`): BinaryDomain, BinaryMaxDomain, BitDomain, BooleanDomain, IntegerDomain, NumberDomain, Number2Domain (+ MySQL variants), FloatDomain, Float1Domain, FractionDomain, IdentityDomain, CustomSuitabilityDomain (Greater/LessOrEqual/Size) — per-dialect value ranges used to validate edits, choose editors and suggest import types. [Observed]
- Cardinality estimates (`Cardinality$Elements/Fraction/MinMax/Scalable/Text`) — feed the statistics header and local-filter lists [Inferred from names].
- Selection: `GridSelection`, `GridSelectionTracker` (undoable), `GridSelectionGrower`; `HiddenColumnsSelectionHolder`. Column limit `grid.tables.columns.limit=2000`; exceeding it shows "Could not display table data. Too many columns." / "Unable to display table with {0} columns. Please try an alternative representation, such as HTML." [Observed]
- Rendering: render cache (`database.grid.render.cache=true`, `database.grid.cache.factor=3`, `database.grid.render.cache.fast.scroll=true` → low-quality while fast scrolling); color layers (stripe, mutations, search, selection, hierarchical selection). [Observed]
- Status bar widgets: **Grid Position** ("[row:{0} col:{1}]"-style, plural-aware "cell/cells", "row/rows") and **Aggregator**. [Observed: `status.bar.grid.*`, `TablePositionWidgetHelper.*`]

### 3.2 Surfaces and capability matrix

| Capability | Table editor (Database Explorer → Edit Data / F4 / double-click) | Result tab (Services) | In-editor result | DSV file *Data* tab | Tabular file via loader | Import preview | Compare viewer |
|---|---|---|---|---|---|---|---|
| Edit cells / rows | yes (needs row identity) | yes when result maps to one keyed table | yes | yes (writes text back) | **read-only** | no | no |
| WHERE / ORDER BY text fields | **yes** | no ("Available only in the data editor") | no | no | no | no | no |
| *Filter by* quick predicates | **yes** | no | no | no | no | no | no |
| *Sort via ORDER BY* | **yes** (option) | no — local sort only | local | local | local | — | local (same as data editor) |
| Local filter, Find on page | yes | yes | yes | yes | yes | — | — |
| Auto-transpose setting | applies | **never** | never | applies (help on DSV) | — | — | — |
| Chart | split mode | **replaces** table | replaces table | split mode | — | — | — |
| Add/Clone/Delete/Rename **columns** | no (DDL via Modify Table) | no | no | **yes** | no | — | — |
| Pin Tab / Rename Tab | — | yes | — | — | — | — | — |

[Observed: help `tables-filter`, `tables-sort`, `tables-view-data`, `editing-csv-and-tsv-files`, `data-loaders`, `viewing-query-results`]

Opening a table: double-click, F4 (`Jdbc.OpenEditor.Data` = Edit Source key), toolbar *Edit Data*, context *Edit Data*. Closing: tab close. A DSV file can be partially edited: select a fragment in the text before *Edit as Table* ("A fragment of a file interpretable as a table" file type). [Observed]

### 3.3 Toolbar, Options menu and floating toolbar

**Toolbar order (table editor / result tab):** paging (First, Previous, *rows* indicator with page-size popup, Next, Last) · Reload Page · Update Interval (Auto-Refresh) · Cancel Running Statements · Add Row · Delete Row · Revert Selected · Preview Pending Changes · Submit · Tx mode + isolation · (Manual) Submit and Commit, Roll Back · Go to DDL ("DDL") · Find on Current Page · Show Chart · Data Extractors dropdown · Export Data · Copy to Database · Compare Data · View as · Show Options Menu; result tabs add Pin Tab. [Observed: help + screenshot]

**Data Extractors dropdown:** sections *Built-in*, *CSV*, *Scripted* (bundle `settings.extractors.*`), plus toggles *Skip Computed Columns*, *Skip Generated Columns*, *Configure CSV Formats…*, *Go to Scripts Directory*; footer hint "Binary extractors (like Excel XLSX) are available in "Export Data" action". [Observed]

**Show Options Menu:** Show Geo Viewer · Show Value Editor · Show Aggregate View · Sort via ORDER BY (table editor only) · Show Filter · Paste Format › · Reset View · View Query · Copy Query to Console · Open Data View Settings… (+ Open Data Views Appearance Settings…). [Observed]

**Grid floating toolbar ("Quick Actions Popup Toolbar")** appears near a selected cell depending on content: Related Rows, Open URL, Open File URI by default; customizable (*Customize This Toolbar* → Menus and Toolbars › Grid Floating Toolbar), disable (*Disable This Toolbar* or setting "Show the quick actions popup toolbar for cells"). **Floating pagination** can be moved: *Move This Paging To Left / Center / Right / Editor Toolbar*, matching setting "Position of the grid pagination control: Grid bottom (floating) · Grid bottom left (floating) · Grid bottom right (floating) · Data editor toolbar"; *Pagination* toggle hides it. [Observed: bundle, help `cells`]

### 3.4 Complete action and shortcut inventory

Win/Linux keys from `$default`; macOS from `Mac OS X 10.5+` where defined, otherwise the platform's automatic Ctrl→⌘ conversion for keymaps whose parent is `$default` [Inferred: platform behavior]. "via X" = `use-shortcut-of` X. [Observed: `gridimpl/META-INF/plugin.xml`, `dbplugin/META-INF/DatabasePlugin.xml`, keymaps]

**Paging, reload, cancel**

| Action id | Label | Win/Linux | macOS | Notes |
|---|---|---|---|---|
| `Console.TableResult.FirstPage` / `LastPage` | First Page / Last Page | — | — | |
| `Console.TableResult.PreviousPage` / `NextPage` | Previous / Next Page | Ctrl+Alt+Up / Ctrl+Alt+Down | ⌘⌥↑/↓ | |
| `Console.TableResult.ChangePageSize` (group) | Change Page Size | — | — | presets, "All" ("Show all rows"), custom dialog "Change Page Size", *Set Default Page Size* ("Change Default: {0}") |
| `Console.TableResult.CountRows` | Count Rows | — | — | "Click to update (runs SELECT COUNT(*) FROM …)"; shows "of {0}"; "Querying…" |
| `Console.TableResult.ChangeTableSample` (group) | Change Table Sampling | — | — | "Sampling {0} Percent", "Sampling Disabled", *Disable* |
| `PageAutoRefreshGroup` ("Auto-Refresh") | Update Interval | — | — | preset intervals, *Custom…* ("Update interval in seconds"), *Paused*, *Disabled*; button *Pause Auto-Refresh* |
| `Console.TableResult.Reload` | Reload Page | Ctrl+F5 (via Refresh) | ⌘R | also applies new page size |
| `Console.Jdbc.Cancel` | Cancel Running Statements | Ctrl+F2 (via Stop) | ⌘F2 | |
| `Console.TableResult.ShowPaginationAction` | Pagination | — | — | toggle floating paging |

**Cell editing and values**

| Action id | Label | Win/Linux | macOS | Notes |
|---|---|---|---|---|
| `Console.TableResult.EditValue` | Edit / View (read-only) | Enter, Alt+Enter, F2 | same | label switches "View"/"Edit" (choice format View/Edit) |
| `Console.TableResult.GotoReferencedResult` | Go to Referenced Result | double-click, Enter, Alt+Enter, F2 | same | only when the cell holds a nested result/cursor; otherwise edit wins [Inferred] |
| `Console.TableResult.GotoReferencingResult` | Referencing Result | Ctrl+B (via Go to Declaration) | ⌘B | back to parent result |
| `Console.TableResult.EditValueMaximized` | Show Value Editor / Open in Value Editor / View Image | Shift+Enter, Alt+Shift+Enter | same | |
| `Console.TableResult.MaximizeEditingCell` | Open in Value Editor | Ctrl+Alt+Shift+M | ⌘⌥⇧M | while a cell editor is active |
| `Console.TableResult.HideEditMaximized` | Hide Value Editor | Shift+Esc (via Hide Active Window) | ⇧⎋ | |
| `Console.TableResult.SetNull` | Set NULL | Ctrl+Alt+N | ⌘⌥N | |
| `Console.TableResult.SetDefault` | Set DEFAULT | Ctrl+Alt+D | ⌘⌥D (Ctrl+Alt+Shift+D in "macOS System Shortcuts") | |
| `Console.TableResult.ClearCell` | Clear Field | — | — | empty string / empty value |
| `Console.TableResult.InsertRandomUUID` | Generate UUID | — | — | |
| `Console.TableResult.LoadFile` | Load File… | — | — | "Choose a file to upload to the selected database table row/column." |
| `Console.TableResult.SaveLobAs` | Save LOB… | — | — | fetches full value |
| `Console.TableResult.ChangeColumnLanguage` / `.ChangeCellEditorLanguage` / `.Header.ChangeColumnLanguage` | Set Highlighting Language… | — | — | per column (header) or editor |
| `Console.TableResult.ChangeCellEditorFileEncoding` | Change Encoding… | via Change File Encoding | | value editor for binary-as-text |
| `Console.TableResult.ColumnDisplayTypeChange` (group) | Change Display Type | — | — | see §3.12 |
| `Console.TableResult.CompareCells` | Compare Cells… | Ctrl+Shift+D | ⌘⇧D | "Show the difference between selected cells' values" |
| `Console.TableResult.RevertSelected` | Revert Selected | Ctrl+Alt+Z (via Revert) | ⌘⌥Z | scope = selection or whole grid |

**Rows and columns**

| Action id | Label | Win/Linux | macOS | Notes |
|---|---|---|---|---|
| `Console.TableResult.AddRow` | Add Row | Alt+Insert (via Generate) | ⌘N | disabled when the context forbids inserts |
| `Console.TableResult.CloneRow` | Clone Row | Ctrl+D (via Duplicate) | ⌘D | appended at the end |
| `Console.TableResult.DeleteRows` | Delete Selected Rows / Delete Row(s) | Ctrl+Y (via Delete Line); help also lists Delete | ⌘⌫ | confirmation texts exist: "One row selected. Are you sure you want to delete row %s?" / "%s rows selected…" |
| `Console.TableResult.SelectRow` | Select Row | Shift+Space | ⇧Space | |
| `Console.TableResult.AddColumn` | Add Column | Alt+Shift+Insert (via Column Mode) | ⌘⇧8 | DSV/document grids; dialog "New Column Name" |
| `Console.TableResult.CloneColumn` | Clone Column | Ctrl+Alt+Shift+D | ⌘⌥⇧D | |
| `Console.TableResult.DeleteColumns` | Delete Selected Columns | Alt+Shift+Delete | ⌥⇧⌦ | for Mongo: "Delete Column Across Collection" |
| `Console.TableResult.RenameColumn` | Rename… | — | — | DSV/documents |
| `Console.TableResult.HideColumn` / `HideOtherColumns` / `ShowAllColumns` | Hide Column / Hide Other Columns / Show All Columns | — | — | |
| `Console.TableResult.ColumnsList` | Columns List | Ctrl+F12 (via File Structure) | ⌘F12 | speed search; hidden shown struck-through |
| `Console.TableResult.ColumnVisibility` | Show/Hide Columns | Space (in Columns List) | Space | |
| `Console.TableResult.CopyColumnName` | Copy Column Name | — | — | |
| `IncreaseColumnWidth` / `DecreaseColumnWidth` / `ResetColumnsWidth` | Expand / Narrow Down Selected Columns / Reset Columns Width | Ctrl+Alt+Shift+Right / Left / Up | ⌃⌥→ / ← / ↑ | via tool-window resize keys |
| `Console.TableResult.Csv.SetFirstRowIsHeader` | First Row Is Header | — | — | DSV grids |

**Selection**

| Action id | Label | Win/Linux | macOS |
|---|---|---|---|
| `TableResult.GrowSelection` / `ShrinkSelection` | Extend / Shrink Selection | Ctrl+W / Ctrl+Shift+W | ⌥↑ / ⌥↓ |
| `TableResult.SelectColumn` | Select Column | Ctrl+W (shares Extend Selection key) | ⌥↑ |
| `TableResult.SelectNextOccurrence` / `UnselectPreviousOccurrence` / `SelectAllOccurrences` | multi-cell occurrence selection | Alt+J / Alt+Shift+J / Ctrl+Alt+Shift+J | ⌃G / ⌃⇧G / ⌃⌘G |
| `$SelectAll` (Select All) | Select All | Ctrl+A | ⌘A |

**Sort and filter**

| Action id | Label | Win/Linux | macOS | Notes |
|---|---|---|---|---|
| `Console.TableResult.ColumnSortAsc` / `Desc` | Ascending / Descending | Alt+Shift+Up / Down (via Move Line) | ⌥⇧↑/↓ | replaces sorting |
| `Console.TableResult.ColumnSortAddAsc` / `AddDesc` | Add Ascending / Add Descending | — | — | stacked |
| `Console.TableResult.ColumnSortReset` | Clear Sorting | Ctrl+Alt+Shift+Backspace | ⌘⌥⇧⌫ | |
| `Console.TableResult.EditFilterCriteria` | Edit Filter Criteria | Ctrl+Alt+Shift+F | ⌘⌥⇧F | same key as Full-Text Search in other contexts |
| `Console.TableResult.EditOrdering` | Edit ORDER BY Clause | — | — | |
| `Console.TableResult.ToggleFilters` | Show Filter ("Show Row Filter") | — | — | |
| `Console.TableResult.Filter.Custom` (group) | Filter by | — | — | table editor only |
| `Console.TableResult.ColumnLocalFilterToggle` | Enable Local Filter | — | — | |
| `Console.TableResult.ColumnLocalFilter` | Set Local Filter | — | — | popup "Local Filter For '{0}'" |
| `Console.TableResult.ColumnLocalFilterWithCell` | Local Filter By Cell Value | — | — | |
| `Console.TableResult.FindInGrid` | Find on Current Page | Ctrl+F (via Find) | ⌘F | |

**View, panels, navigation**

| Action id | Label | Win/Linux | macOS | Notes |
|---|---|---|---|---|
| `Console.TableResult.ViewAs` (group): `ViewAsTable`, `ViewAsTreeTable`, `ViewAsExtractor`, `Transpose` | Table / Tree / Text / Transpose | — | — | |
| `Console.TableResult.ShowRecordView` / `ToggleRecordView` | Show Record View / Record View | Ctrl+Shift+Enter | ⌘⇧↩ | |
| `Console.TableResult.AggregateView` | Show Aggregate View | — | — | |
| `Console.TableResult.EnableAllAggregators` / `DisableAllAggregators` | | — | — | Aggregates tab gear |
| `Console.TableResult.CopyAggregatorResult` | Copy Aggregation Result (SUM) | Ctrl+Shift+C (via Copy Paths) | ⌘⇧C | default aggregator chosen in status-bar widget |
| `Console.TableResult.ShowChart` / `CloseChart` / `SwitchTo.ChartView` / `SwitchTo.TableView` | Chart | — | — | |
| `GridGeoViewer` / `GridGeoViewerClose` | Show / Hide Geo Viewer | — | — | |
| `StatisticsPanel.Show` (group): `.Off` / `.Compact` / `.Detailed` | Show Column Statistics | — | — | |
| `Console.TableResult.NavigateAction` | Related Rows | F4 (via Edit Source) | ⌘↓ | |
| `Console.TableResult.OpenWebUrlAction` / `OpenLocalFileAction` | Open URL / Open File URI | F4 | ⌘↓ | subject to URL Click Settings |
| `GotoRow` | Row… (Go to Row) | Ctrl+G (via Go to Line) | ⌘L | "<row>[:<column>] or <row>[,<column>]" |
| `Jdbc.OpenEditor.Grid.DDL` | DDL (Go to DDL) | Ctrl+Alt+Home (via Go to Related) | ⌃⌘↑ | |
| `Console.TableResult.ShowQuery` / `CopyQueryToConsole` | View Query / Copy Query to Console | — | — | Esc closes the query pane |
| `Console.TableResult.ResetView` | Reset View | — | — | unsorted, original order, all columns shown |
| `Console.TableResult.RenameTab` | Rename Tab | — | — | result tabs |

**Copy, export, compare, submit, transactions**

| Action id | Label | Win/Linux | macOS | Notes |
|---|---|---|---|---|
| `Console.TableResult.Copy` | Copy / "Copy Using Data Extractor ({0})" | Ctrl+C (via Copy) | ⌘C | current extractor |
| `Console.TableResult.PasteFormat` (group) | Paste Format › Detect / Single Value / each CSV format | — | — | |
| `Console.TableResult.ExportToClipboard` | Export Table to Clipboard | — | — | ignores page size |
| `Console.TableResult.ShowDumpDialogGrid` / `ShowDumpDialogAction` | Export Data… (in consoles: "Execute to File(s)") | — | — | §3.21 |
| `Console.TableResult.ChooseExtractor` (group) | Change Data Extractor | — | — | |
| `Console.TableResult.SkipComputedColumns` / `SkipGeneratedColumns` | toggles | — | — | |
| `Console.TableResult.Copy.Csv.Settings` | Configure CSV Formats… | — | — | |
| `Console.TableResult.GoToExtractorsScriptsDirectory` / `GoToAggregatorsScriptsDirectory` | Go to Scripts Directory | — | — | |
| `Console.TableResult.ImportTable` | Copy to Database… | — | — | Import dialog with this grid as source |
| `Console.TableResult.CompareWith` (group) / `DatabaseView.ShowContentDiff` | Compare Data | — | — | |
| `Console.TableResult.PreviewDml` | Preview Pending Changes | — | — | |
| `Console.TableResult.Submit` | Submit | Ctrl+Enter (via `Console.Jdbc.Execute`) | ⌘↩ | |
| `Console.TableResult.SubmitAndCommit` | Submit and Commit | Ctrl+Alt+Shift+Enter | ⌘⌥⇧↩ | Manual mode only |
| `Console.Transaction.Commit` / `Rollback` / `RevertAndRollback` | Commit / Roll back / Roll Back | Commit = Ctrl+Alt+Shift+Enter | | RevertAndRollback also discards pending grid changes [Inferred from id] |
| `Console.TableResult.ChangeIsolation` (group) | Tx Isolation | — | — | |
| `Console.TableResult.EditMaximized.*` | Change Type / Move to Bottom / Move to Right / Toggle Formatting / Toggle Soft-Wrap / Change Layout | — | — | value editor & record view |
| `Console.TableResult.CustomizeGridFloatingToolbar` / `DisableGridFloatingToolbar` / `MoveGridFloatingPaging.*` | toolbar placement | — | — | |
| `DataViews.Settings` / `.Appearance` | Open Data Views (Appearance) Settings… | — | — | |

Keymap folders: *Database › Data Editor*, *Results Toolbar*, *Value Editor and Aggregate View* (› Value Editor, › Aggregate View). [Observed: `Database.KeymapGroup.*`]

Help discrepancy: the cell context-menu table lists *Delete Row* as **Alt+Delete** while the toolbar/Rows topics say Ctrl+Y or Delete; plugin.xml binds Delete Rows to Delete Line (Ctrl+Y) and Delete Columns to Alt+Shift+Delete. Treat Alt+Delete as a doc error [Inferred; verify doc 13 DML family].

### 3.5 Row identity

- UPDATE/DELETE need a unique row identifier: primary key, otherwise a unique key [Inferred: generator naming `WhereClause`, `SelectUsingKeys`]; without one, the table opens read-only ("Table is read-only.<br>{0}"; on submit: "Cannot Apply Changes — This table is read-only. Changes cannot be applied."). [Observed: bundle] Physical row ids (`ctid`, `ROWID`, `%%physloc%%`) are not evidenced. [Unknown → doc 13 DML-03]
- Query results are editable when the result maps to a single table with its key columns present [Inferred; help shows editing in result sets; verify DML-10].
- Hacivat fallback order (recommendation): PK → unique NOT NULL key → unique nullable key with all values non-null in the row → (opt-in) physical row id (PG `ctid` with `xmin` check, Oracle `ROWID`, SQLite `rowid`) → read-only.

### 3.6 Paging, fetching, sampling, auto-refresh

- **Page size**: default 500 ("the number of returned rows is limited to 500"); *Limit page size to* unchecked = unlimited; toolbar rows indicator opens presets, *All*, a custom dialog, and *Set Default Page Size* (changes the setting). Applying a new size requires *Reload Page*. [Observed]
- **Prefetch size** (setting) = rows per round trip (JDBC fetch size); scrollable result sets when supported (`database.scrollable.result.sets=true`); table editors page via `LIMIT/OFFSET` (`database.grid.paging.offset=true`), with dialect alternatives (TOP/FETCH FIRST/ROWNUM in `WrapInSelect` generators). [Observed]
- **Count Rows**: indicator shows "N rows" for the page; clicking *Count Rows* runs `SELECT COUNT(*)` (task `CountAll`) and shows "of {total}"; "Querying…" while running. [Observed]
- **Table sampling**: *Change Table Sampling* offers percentages ("Sampling {0} Percent") or *Sampling Disabled*; maps to DB sampling clauses (`TABLESAMPLE`) [Inferred]. Available for dialects that support it [Inferred].
- **Auto-refresh** (`GridAutoRefresher`, `PageAutoRefreshGroup`): toolbar *Update Interval* with preset intervals, *Custom…* in seconds, *Paused*, *Disabled*; refresh happens only while the grid is showing (`processAutoRefreshWhenShown`); a dialog "Auto-Refresh" with *Refresh* / *Pause Auto-Refresh* exists for conflicts [Inferred: likely shown when pending changes block refresh]. [Observed: names, bundle]
- In-editor results: paging optional ("Enable paging in in-editor results by default"). Multiple result sets: up to 100 per execution (`database.result.set.limit=100`). [Observed]
- Loading placeholder: "Load Data" button for deferred loaders; "Loading data…". [Observed: bundle]

### 3.7 Large values (LOB)

- "Maximum number of bytes loaded per value" (tooltip "Values exceeding the limit will not be loaded fully"); larger values loaded partially; cell shows "{0} of {1}"; progress "bytes loaded"; copy shows "Data was copied partially. To copy full data, change size limit for values" (link opens "Change Size Limit" dialog with **Apply and Reload**); Clone Row refused ("Cannot clone row because it contains truncated value."); *Save LOB…* writes the full value; *Load File…* sets a cell from a file; *View Image* in the value editor (help: default 1024 bytes loaded, raise the limit for larger images); "Extracting Cell Content…" progress for large extractions. [Observed]

### 3.8 Editing and cell editors

**Starting and finishing an edit.** Enter / Alt+Enter / F2 / double-click / start typing. Enter confirms, Esc cancels. Typing with a multi-cell selection writes the same value into every selected cell. Ctrl+Space opens **value completion** built from already loaded values of the column (local only); setting "Enable immediate completion in grid text cells" opens it while typing. Read-only cells show *View* instead of *Edit*. [Observed]

**Editor factories** (`GridCellEditorFactory` with `ValueParser`, `ValueFormatter`, `IsEditableChecker`, `DefaultValueToText`): [Observed: class names; behavior Inferred from names + help]

| Type family | Editor | Behavior |
|---|---|---|
| text/varchar/clob | `DefaultTextEditorFactory$GridTextCellEditor`, resizable (`ResizableCellEditorsSupport`) | in-cell multi-line text; completion; highlighting language applies |
| numeric | `DefaultNumericEditorFactory$NumericEditor` | parse with custom number format (decimal/grouping separators, patterns); "Invalid number" |
| boolean | `DefaultBooleanEditorFactory` (`TextBooleanCellEditor` with dropdown + `ChooseBooleanAction`, `CheckboxBooleanCellEditor`) | text mode: Space toggles; letters `t` true, `f` false, `d` default, `n` null, `g` generated, `c` computed; checkbox mode: NULL/DEFAULT via context menu; "Expected boolean value" |
| date | `DefaultDateEditorFactory` with calendar pane | custom date pattern |
| time / timestamp / temporal | `DefaultTimeEditorFactory`, `DefaultTimestampEditorFactory`, `DefaultTemporalEditorFactory` | patterns for datetime, datetime+tz, time, time+tz, date; display time zone; "Expected timestamp", "Expected time", "Expected timestamp with time zone"; zone offset must be within −12:00..+14:00 |
| generic format-based | `FormatBasedGridCellEditor` (+ `Bounded…`, `WithBrowseButton`), `FormatBasedGridCellEditorFactory$LenientFormatter`, `$NullCompletionProvider` | lenient parsing; browse button opens a richer editor (calendar, file) |
| binary/blob | `DefaultBlobEditorFactory$BlobTextCellEditor`, `UuidBinaryEditorFactory` | edits according to display type (text/hex/UUID); "Invalid UUID" |
| bit string | — | "Invalid bit string" |
| ranges (PG) | — | "Invalid range value" |
| JSON / documents | value editor with JSON language; Mongo `ObjectId`, `UUID(...)`, `MinKey/MaxKey` constructs | "Expected JSON", "Expected object ID", "Expected UUID construction", "Expected MinKey/MaxKey construction" |
| nested tables / arrays | tree view or nested-table navigation | "Navigation to nested tables is unavailable for static grid / during loading of rows" |

**Special values.** *Set NULL* (Ctrl+Alt+N), *Set DEFAULT* (Ctrl+Alt+D — uses the column default expression; new rows get DEFAULT or NULL for unspecified cells), *Generate UUID*, *Clear Field*, *Load File…*. Default expression and nullability are edited in Modify Table (doc 09). [Observed]

**Validation.** Wrong input is kept as `UnparsedValue` ("Incorrect value" tooltip with the specific parse error; generic "Failed to parse data", "Unexpected data format") and blocks submit. Out-of-range numbers/dates go through `BoundaryValueResolver`/`DatabaseBoundaryValueResolver` (`Rules`, `Infinity`): popup "Value Is out of Range. Set "{0}"?" offers the clamped/infinite value. [Observed]

**Undo.** Edits are undoable in the grid (`GridSelectionTracker` undoable operations); after submit "Cannot undo database modification" / "Cannot redo…" [Observed: bundle `notification.content.can.not.undo.redo.database.modification`].

### 3.9 Pending changes and submission

- Pending storage: `MutationsStorageImpl` (counters for modified rows, rows, rows with unparsed values); `Mutation`, `MutationData`, `MutationRow`, `MutationType`, `CellMutation(.Builder)`, `ColumnMutation`, `DatabaseMutation`, `DmlMutationContext`, `DatabaseMutatorImpl$PendingContexts`; colors via `MutationsColorLayer` (modified, inserted, deleted, error). [Observed]
- **Submit** (Ctrl/⌘+Enter, toolbar arrow): Auto tx → executed and committed; Manual → executed inside the session transaction; commit with *Submit and Commit* (Ctrl+Alt+Shift+Enter) or *Roll Back*. **Submit changes immediately** (Data Modification setting) submits each edit as it is confirmed. *Preview Pending Changes* (`DataGridPreviewDmlAction`) shows the generated SQL. Generated DML appears in the Output log (`DataAuditor` events). [Observed]
- **Revert Selected** (Ctrl+Alt+Z): scope = changes inside the selection; empty selection → whole grid. [Observed]
- **Leaving with unsubmitted changes** (close tab, reload, page switch [Inferred]): dialog "Ignore Unsubmitted Changes — Changes are not submitted. Data will be lost. Continue?". [Observed: bundle]
- **Delete confirmations** exist ("One row selected. Are you sure you want to delete row %s?", "%s rows selected. Are you sure you want to delete the selected rows?", column equivalents) — shown for immediate-submit / document grids [Inferred: in normal mode deletes are just marked].
- DDL source conflicts (routines/views) are handled by three-way merge ("Merge" → diff, *Save changes and finish merging*) and the data-source option "Warn when editing outdated DDL" — these concern object sources, not data rows. **No optimistic-concurrency check for data rows is evidenced** [Unknown → doc 13 DML-20].
- Query errors during submit open "Query Error" with *Details*; unknown errors "An unknown problem occurred (see Details)". [Observed]

### 3.10 Transactions from the grid

- *Tx* dropdown on the grid toolbar: **Auto** ("the current transaction is committed automatically when you submit your local changes") / **Manual** (changes accumulate until *Submit and Commit* or *Roll Back*); isolation submenu (Database Default, Read Uncommitted, Read Committed, Repeatable Read, Serializable — doc 07 §3.8). The grid shares the session with consoles attached to it, so a Manual transaction opened in a console also covers grid submits [Inferred: session model, doc 03]. [Observed: help]
- Hacivat note: show the pending-transaction badge on both grid and console; never auto-commit grid edits when the session is Manual.

### 3.11 Value editor, record view, aggregate view

- **Value editor** (`EditMaximized`, tabs **Value**, **Record**, **Aggregates**): open with Shift+Enter / Alt+Shift+Enter / Options › Show Value Editor; docks at the bottom or right (*Move to Bottom/Right*); Enter = newline, Ctrl+Enter = apply, Esc = revert and leave; *Toggle Soft-Wrap*; *Toggle Formatting* pretty-prints one-line JSON/XML (`JsonMinimizedFormatDetector`); *Change Type* (cell type, e.g. text ↔ JSON ↔ number for document stores); *Change Encoding…*; *View Image* for image bytes; "No cell selected" placeholder. [Observed]
- **Record view** (Ctrl+Shift+Enter): side panel with Field/Value columns for the current row; *Change Layout* → "Change to Two Column Layout" / "Change to Single Column Layout"; link "To value editor". Edits through the record view are ordinary pending changes. [Observed]
- **Aggregate view** (Aggregates tab): runs every enabled aggregator over the selection; gear → enable/disable individual scripts, *Enable/Disable All Aggregators*, *Go to Scripts Directory*; "Not enough values" when nothing numeric; "Calculating…". **Status-bar Aggregator widget** shows one chosen aggregator ("No aggregator selected" otherwise) and *Show All Aggregators*; *Copy Aggregation Result* copies the widget's aggregator value (SUM by default). [Observed]
- **Quick documentation** (Ctrl+Q) on cells: long text in full; for records, referenced and referencing rows (auto-loaded, `database.automatically.load.related.data.in.cell.quick.doc=true`), limited by "Show first N data rows in preview"; transposed toggle. For a multi-cell selection it shows a summary. [Observed]

### 3.12 Display types and presentation

- **Binary display types** (`BinaryDisplayType`): *Detect*, *Hex*, *Hex and ASCII*, *UUID*, *UUID with time-low and time-high swapped* (MySQL `UUID_TO_BIN(x,1)` layout), *Text*. 16-byte binaries default to UUID; "Automatically detect binary values" sets the default. **Numeric display types** (`NumberDisplayType`): *Number*, *Timestamp (Seconds)*, *Timestamp (Milliseconds)* (epoch rendering). Stored per column (`TableResultPanel$ColumnAttributes$DisplayTypesInfo`). [Observed]
- **Highlighting language** per column (header) or per cell editor: HTML, CSS, XML, JSON, RegExp, SQL… gives coding assistance inside the value editor. [Observed]
- **Formats**: custom number format (decimal separator default `.`, grouping separator — "Must be empty or a single character", Infinity text, NaN text, number pattern); date/time patterns for 5 kinds + display time zone ("Invalid time zone: {0}"). [Observed]
- **Booleans**: text with dots marking true values, or checkboxes. **NULL**: distinct rendering (color "Data Grid//Null data"); **errors**: "Data Grid//Error data"; **images**: "Data Grid//Image data"; zebra: "Alternating row color"; custom font (size, line height) for grids only. [Observed]
- **URLs and files**: cells containing `http(s)://` or `file://` URIs are clickable when allowed (URL Click Settings: Standard links (HTTP), Secure links (HTTPS), Local file links; "If no protocol is specified, assume HTTP"; click gesture configurable); otherwise notification "Enable the option to open {0} URLs in the Data Viewer and Editor settings"; errors "File {0} does not exist", "Could not open. URL contains unsupported symbols." [Observed]
- **Column header icons** show key/index/FK markers (screenshot); sort markers with level numbers. [Observed]

### 3.13 Filtering

- **WHERE field** (Show Filter / Ctrl+Alt+Shift+F): text of a WHERE clause without the keyword; Enter applies; clear icon resets; history arrow or Alt+Down opens history (size = *Filter history size*); SQL completion and LIKE wildcards; error "Invalid filter criteria". "Remember whether the filter is ON" (Other settings) persists visibility. Table editor only. [Observed]
- **Filter by** (context menu on one or more cells, table editor only): `ColumnFilterByGroup` builds predicates from templates — equality/inequality and comparisons (`FilterTemplate`), `LIKE` (`LikeFilterTemplate`), `IS [NOT] NULL` (`NullFilterTemplate`) — with operands from the selected grid cells (`GridOperand`), the clipboard (`ClipboardOperand`), NULL (`NullOperand`) or a placeholder to fill in (`PlaceholderOperand`); results are `PreparedColumnFilterExpression`s appended to the WHERE text (`PredicatesWhereClause`, `PlainWhereClause`, `ValuesWhereClause`). Multi-cell selection on one column yields IN-style lists [Inferred from `ValuesWhereClause`]. Exact menu labels are produced in code (not in bundles) [Unknown → doc 13 FILTER family].
- **Local filter** (`LocalFilterPopup`, `LocalFilterState`): *Enable Local Filter* shows a funnel icon per header; popup "Local Filter For '{col}'" lists distinct loaded values with **Value** and **Count** columns, a text field, status "Select a checkbox to filter rows" / "Number of matches {0}"; multiple columns combine (AND); *Local Filter By Cell Value* filters by the current cell; toggling off keeps conditions; "Enable local filter by default" setting. Filters only loaded rows. [Observed]
- **Find on current page** (Ctrl+F): search bar with Match case (Cc), Regex (.*), Words (W), multi-line toggle, Prev/Next arrows and **Filter rows** checkbox (hide non-matching rows); matches highlighted, current match distinct; multi-occurrence selection actions apply. [Observed: help + screenshot]
- **Full-Text Search** across databases (Ctrl+Alt+Shift+F on Database Explorer selection): options Match case; Contains / Starts with / Ends with / Matches / LIKE pattern; column scope Text columns / Only columns with full-text search indexes (PG `@@ plainto_tsquery`, MySQL/MariaDB `MATCH … AGAINST … IN NATURAL LANGUAGE MODE`) / Numeric columns / All columns; "Show first N matches per table/view"; results in Find tool window; Cassandra uses several queries per table (no OR). Details doc 10. [Observed]

### 3.14 Sorting

- Header sort marker cycles unsorted → ascending → descending (DSV help: "toggle between… ascending, descending, and initial unsorted state"); unstacked by default; stacked with the "Add columns to sorting" gesture (default Alt+Click) or *Add Ascending/Descending*; level numbers next to markers; *Clear Sorting*. Error "Invalid sorting" for bad ORDER BY text. [Observed]
- **Sort via ORDER BY** (table editor only; settings default + Options menu toggle): each sort re-queries; off → local sort of the current page only. Every server sort appears in the Output tab. **Sort tables by numeric primary key** (Ascending / Descending) applies an ORDER BY on open. [Observed]
- ORDER BY text field (Edit ORDER BY Clause) with its own history. [Observed]

### 3.15 Columns and selection

- Reorder by dragging headers; resize by dragging borders or Expand/Narrow/Reset width actions; hide (Hide Column / Hide Other Columns / Columns List + Space), show all; Reset View restores order/visibility/sorting. Structure tool window lists all columns. [Observed]
- DSV/document grids: Add Column (dialog "New Column Name"), Clone Column, Delete Selected Columns, Rename…; for Mongo "Delete Column Across Collection". [Observed]
- Selection: click gutter numbers to select rows (Ctrl+Click to add), Shift+Space selects row, Ctrl+W/Ctrl+Shift+W grow/shrink (cell → column/row → all [Inferred]), occurrence multi-select (Alt+J…), Select All. [Observed]

### 3.16 View modes, charts, geo, statistics

- **Table / Tree / Text** (+ **Transpose**, combinable). Tree = key/value tree for JSON/arrays/documents; Text = whole grid rendered by the selected extractor (read-only text editor). *Automatically transpose tables*: Never / If a table has one record / Always (not for query results). [Observed]
- **Nested tables** (arrays of records, Oracle nested tables, ref cursors, documents): navigation into a nested table and back (*Go to Referenced/Referencing Result*), breadcrumb-like stack (`NestedTablesAware$NonEmptyStack`); errors when grid is static or loading. [Observed: names, bundle]
- **Chart** (`table.chart.tab.name=Chart`; registry `database.grid.show.chart.tab=true`): types Bar, Pie, Area, Line, Scatter, Bubble, Stock, AreaRange, Histogram; default type chosen from data types; *Show Series Settings*: chart type selector, X axis column + modifier (none / group / group and sort), Groups, Y axis column + modifier; *Add new series* / *Remove*; hover shows exact value; *Export to PNG*. Split under the table for tables/DSV, replaces the table for results (*Switch to Chart/Table*). [Observed]
- **Geo Viewer** (`geo-viewer.html`, JCEF): PostgreSQL/PostGIS tables and tabular files; formats WKT, WKB, PostGIS geometry SRID 4326; row ↔ pin navigation; detachable from Services; JCEF enablement prompt + restart; recovery by removing the JCEF flag. [Observed]
- **Column statistics header** (`StatisticsTableHeader`, `StatisticsPanelMode`): *Show Column Statistics › Off / Compact / Detailed* — a panel above each column header summarizing the loaded values (e.g. distinct count, nulls, min/max, top values) [Inferred: exact metrics not documented in help → doc 13]. [Observed: action names, classes]

### 3.17 Copy, paste and paste formats

- **Copy** (Ctrl+C): the selection rendered by the current extractor ("Copy Using Data Extractor ({0})"); only the current page; Ctrl+A then Ctrl+C copies the page; **Export Table to Clipboard** copies everything regardless of page size; *Copy Column Name*; *Copy Aggregation Result*. Skip Computed/Generated toggles affect SQL extractors. Values with separators are quoted per format ("DataGrip will honor the contents of such cells"). [Observed]
- **Paste** (Ctrl+V): `GridPasteProvider` → `TableDataParser` → `TableDataParseResult`; **Paste Format** submenu: *Detect* (default), *Single Value*, and every configured CSV format (`ChoosePasteFormatAction$PasteType`, `FormatParser`, `SingleValueParser`). Notifications ("Paste" group): "Text was pasted using *{fmt}* format ({sep}-separated).", "*{fmt}* ({sep}-separated) format was detected for pasted text.", "Text was pasted as single value.", "No CSV format was detected. Text was pasted as single value." CSV lines replace selected rows; missing rows are created; types converted automatically. Algorithm ALG-GRID-01. [Observed]

### 3.18 Extractors and aggregators — scripting API

[Observed: bundled scripts' header comments and usage; help `data-extractors`] (described in our own words)

**Location & naming.** User-editable copies under *Scratches and Consoles › Extensions › Database Tools and SQL › data › {extractors, aggregators, loaders}*. Extractor file name `<DisplayName>.<outputExtension>.<groovy|js>` (e.g. `PHP.array.groovy` → menu "PHP", output `.array`); aggregators `<NAME>.groovy`. The last selected extractor becomes the default for newly opened tabs; MongoDB always defaults to JSON. Hashes of previous bundled versions (`extensions_old/…`, 32-hex lines) let the IDE upgrade unmodified user copies silently [Inferred from content]. Errors: "No script engine found for file extension '{0}'" (+ "Please install plugin '{0}'"), "Failed to load script: {0}", "Invalid extractor name: '{0}'". [Observed]

**Header contract** (each bundled script starts with a comment listing): `COLUMNS` (list of DataColumn), `ROWS` (iterable of DataRow), `OUT` (append), `FORMATTER` (format(row,col), formatValue(obj,col), getTypeName(obj,col), isStringLiteral(obj,col)), `TRANSPOSED` (boolean), plus `ALL_COLUMNS`, `TABLE`, `DIALECT`; DataRow = rowNumber(), first(), last(), data() list, value(column); DataColumn = columnNumber(), name(). [Observed]

| Binding | Members | Meaning |
|---|---|---|
| `COLUMNS` | `columnNumber()`, `name()` | selected columns (all when exporting) |
| `ALL_COLUMNS` | same | all columns of the result/table |
| `ROWS` | `rowNumber()`, `first()`, `last()`, `data()`, `value(column)`, `hasValue(column)` (used by bundled scripts); Groovy collection helpers | **streamed** rows |
| `OUT` | `append(String)` (chainable) | output sink |
| `FORMATTER` | `format(row, column)`, `formatValue(value, column)`, `getTypeName(value, column)` (e.g. `json`), `isStringLiteral(value, column)` | grid-consistent formatting |
| `TRANSPOSED` | boolean | grid transposed |
| `TABLE` | `getName()`, `getParent().getName()`; null for query results | INSERT target |
| `DIALECT` | `getDbms()` + `isOracle, isMysql, isPostgres, isBigQuery, isRedshift, isGreenplum, isVertica, isMicrosoft, isSybase, isDb2, isHsqldb, isH2, isDerby, isSqlite, isExasol, isClickHouse, isCassandra, isHive, isSpark, isSnowflake, isMongo, isCouchbase, isCockroach, isTransactSql, isDocumentOriented` | dialect-specific escaping |
| `PROJECT` | IDE project | e.g. read keyword-case code style |

Scripts may call IDE utilities (XML/JSON escaping, line splitting, unpluralize). Hacivat must provide equivalents.

**Extractor menu** (help "Supported file formats"): *Built-in* — SQL Inserts, SQL Updates, Where Clause (.sql); *CSV* — CSV (.csv), TSV (.tsv), Pipe-separated (.txt) and user CSV formats; *Scripted* — CSV, TSV, Excel (.xlsx/.xls via Export only — `XlsxExtractorFactory`, `XlsxValuesExtractor` with shared strings), HTML (groovy), HTML (js), JSON, Markdown, One-row, SQL-Insert-Multirow, SQL-Insert-Statements, Pretty, XML. Help-suggested community extractors: TSV-Icelandic, Dbunit dataset, Table-Text / Fixed-length-fields, PHP-Array, Markdown-JavaScript. [Observed]

**Bundled script extractors (behavior, paraphrased):**

| Extractor | Output | Notable rules |
|---|---|---|
| CSV-Groovy (.csv) | comma-separated | quote with `"` only if value contains separator, quote or newline; double embedded quotes; header row |
| HTML-Groovy / HTML-JavaScript (.html) | HTML table | XML-entity escaping; newlines → `<br/>`; HTML-looking values passed through (JS variant); transposed layout |
| JSON-Groovy (.json) | array of objects keyed by column | recursive maps/arrays, 2-space indent; numbers/booleans raw; strings escaped; json/jsonb values emitted raw |
| Markdown-Groovy (.md) | pipe table | escapes Markdown metacharacters (`\` `` ` `` `*` `_` `{}` `[]` `()` `<>` `#` `+` `-` `.` `!` pipe `~`) |
| One-row (.sql) | one row as a comma list | string literals quoted, dialect escapes |
| Pretty-Groovy (.txt) | ASCII box table | widths from content, multi-line values split, transposed variant |
| SQL-Insert-Statements (.sql) | one INSERT per row | keyword case from code style; `schema.table` or placeholder `MY_TABLE`; `NULL`; `''` escaping; `N'…'` for Microsoft; backslash escaping for MySQL |
| SQL-Insert-Multirow (.sql) | single multi-row INSERT | same escaping |
| XML-Groovy (.xml) | XML | element names sanitized; collection items named by unpluralized parent or `item` |

**Bundled aggregators** (numeric = numbers or strings parseable as decimals; 128-bit decimal arithmetic; "Not enough values" when none): SUM, AVG, MIN, MAX, MEDIAN (middle or mean of two middles), COUNT (selected cells), COUNT_NUMS (numeric cells), ROWS, COLS, COEFFICIENT_OF_VARIATION (stddev / mean). Aggregators and single-value extractors are interchangeable. [Observed]

### 3.19 Data loaders — scripting API

[Observed: `extensions/data/loaders/{JSON,XLS,Parquet,SHP}.groovy` structure; help `data-loaders`] (described, not copied)

- **Header metadata** in the first comment line: `extensions = json` (semicolon list, e.g. `xls;xlsx`), `displayName = …`, optional `tableFirstFormat=true|false`. *Table-first* formats (e.g. spreadsheets) open on the Data tab; others (JSON) keep text first; advanced setting *Open file as table if detected by scripted loader*: **All** / **Table-first formats** / **None** ("Automatically add 'Data' tab with table representation if there is a scripted loader for that file").
- **Entry point**: the script registers a closure with `LOADER.load { ctx -> … }`; `ctx.getParameters()["FILE"]` is the file path; `ctx.getDataConsumer()` receives output.
- **Data consumer**: `consumeColumns(String[] names, Class[] types)` declares columns once; `consume(Object[] row)` emits a row (an empty array emits an empty row); `consume(name, value)` emits a single named scalar (used when the file holds a single value).
- **Dependencies**: scripts pull third-party libraries with Groovy `@Grab` Maven coordinates (Jackson for JSON; POI for XLS; Parquet/Hadoop; GeoTools/JTS for shapefiles incl. CRS → EPSG transforms) — implies network access on first use [Inferred].
- **JSON loader semantics**: top-level array of objects → one row per object, columns allocated in first-seen key order across all objects (missing keys → NULL), column type = class of the first value; array of arrays → positional columns; array of scalars → one column; top-level object → name/value pairs; nested structures become map/list values (rendered by the tree view/JSON extractor). See ALG-GRID-23.
- Loader views are **read-only** (`ImmutableDataHookUp`); importing them uses the Import dialog with a *Loader* field.

### 3.20 Related rows and FK navigation

- *Related Rows* (F4, Go To submenu, floating toolbar): lists records the current one references and records referencing it (FKs and virtual FKs); popup when several targets; unavailable when none. Multi-value selection supported. Advanced setting **Related Rows action behavior**: *Navigate and filter all* (default; open target filtered with all selected values) / *Navigate and select first*. Go To submenu also has Navigation Bar, Database Explorer, Related Symbol, DDL, Row. [Observed]
- *Go to Referenced Result* / *Referencing Result* (Ctrl+B) for nested results ("Referenced Result is not available" errors). [Observed]

### 3.21 Export Data dialog (every option)

Opened from the grid toolbar (*Export Data…*), Database Explorer (*Import/Export › Export Data to File(s)*), or a console (*Execute to File(s)*). Title "Export Data". [Observed: help + bundle `settings.database.DumpDialog.*` + screenshot]

| Control | Values / behavior | Default (screenshot) |
|---|---|---|
| Source | read-only description (`db.schema.table` or query) | — |
| Extractor | any extractor incl. Excel (xlsx) | last used |
| Add columns: Computed / Generated | include computed and auto-generated columns in SQL output | — |
| Transpose | export transposed | unchecked |
| Add table definition (DDL) | prepend CREATE script (.sql) | — |
| Add column header | header row (CSV-like) | unchecked |
| Add row header | leading row-number column | unchecked |
| Output query to separate sheet | xlsx: extra "Query" sheet with the source query | — |
| Output file / Output directory | file for one source; directory for many ("Export to Files"); file chooser "Choose Output File or Directory" | `<home>/<name>.<ext>` |
| Export preview | first rows rendered; "Extractor is binary. Preview is not available." | — |
| Buttons | Copy to Clipboard · Export to File(s) · Cancel | — |

Validation messages: "Output path should not be empty", "Directory does not exist", "Not a directory", "Not a file", "Parent directory does not exist", "Cannot write file: {0}", "Confirm Replace File — {0} already exists. Do you want to replace it?". Progress: "Running query…", "Initializing output…", "Saving "{0}"…", "{n} rows". Completion notification composed with choice formats (e.g. "N rows saved to file", "K tables (N rows) saved from X to directory"). Options shown depend on the extractor. [Observed]

### 3.22 Import dialog (every option)

Opened by *Import Data from File(s)…* (schema → new table; table → append), dragging a file onto a schema/table, *Copy Table to…* / dragging tables, grid *Copy to Database…*, or *Import to Database…* from a file editor. Title "Import" / "Import '{0}' Table" / "Import "{0}" File". [Observed]

**Layout**: (1) mapping tree (sources with child mappings; toolbar Add Alt+Insert, Remove Alt+Delete, Up/Down, Edit F4, Edit On Selection, Back Ctrl+Alt+Left, Forward Ctrl+Alt+Right; tabs per mapping); (2) settings pane; (3) preview pane. A "Target schema for all sources" selector exists for multi-source imports. [Observed]

**Source settings**

| Source kind | Fields |
|---|---|
| CSV/DSV file | Path · Charset · Format (dropdown incl. `<Detect>`; edit opens CSV format settings; "detected" label) · First row is header · First column is header · Trim whitespaces · Column / Type grid |
| Tabular file (loader) | Path · Loader · First row is header · Column / Type grid |
| Table | Table |
| Query | Target (data source/schema to run it against) · Query |

**Mapping settings**: Schema (any data source) · Table (name; *Existing* / *New* badge; *Auto Generate* name; create-new-table pencil adds a node) · Mapping grid **Target Column** ↔ **Source Column** (`<Auto> (C1)` default by position/name; add/remove/reorder; "to: Type" for new tables) · Encoding · **Write errors to file** [path] (checked by default with a temp path, per screenshot) · **Insert inconvertible values as null** · **Disable indexes and triggers, lock table (may be faster)** (only when target has indexes/triggers). Hidden properties visible in bundle: *Use cached data*, *Data Only*, *Defines Table*. [Observed]

**Preview pane**: *Data Preview* of the source (with "file: X format: Y" + *Modify Source*, *Reload Page*) and of the target; **DDL Preview** — editable CREATE TABLE that will be executed. [Observed]

**Messages**: "Columns are not mapped", "Choose source table", "Table is empty", "Name of the table is empty", "Target schema is not selected", "{0} in {1}", confirmation "Table Schema Will Be Changed — Are you sure that you want to modify a table schema in the database?", success "{0} imported to {1}" (link opens table), "Import failed", "Incomplete definition", "Select source to see the preview". [Observed]

Related: *Restore with bcp*, pg_restore/psql/mysql restore, Run SQL Script — doc 10.

### 3.23 CSV Formats dialog and file as table

**CSV Formats** (Settings › Database › Data Editor and Viewer › CSV Formats, or *Configure CSV Formats…*; same editor as "Configure CSV Format for <file>"): [Observed: help + bundle]

| Control | Values |
|---|---|
| Formats list | Add / Remove / Copy / Up / Down; predefined **CSV**, **TSV**, **Pipe-separated**, **Semicolon-separated** (older names "Comma-separated (CSV)", "Tab-separated (TSV)"); menus present each as "with header" / "without header" |
| Value separator | Comma · Tab · Semicolon · Pipe · Space · custom string |
| Row separator | Newline · custom (e.g. `;`) |
| Null value text | Undefined (no NULL marker) · Empty string · custom (e.g. `\N`) |
| Add row prefix/suffix | Row prefix, Row suffix strings |
| Quotation | ordered list of patterns (Left Quote, Right Quote, Escape Method: `<duplicate>` or a custom escape char — rendered "Escape: duplicate" / "Escape with: {0}"); **first pattern used for writing**, all patterns accepted when reading [Inferred from "If there is more than one pattern, the first pattern is used"] |
| Quote values | Never · When needed (value contains value or row separator) · Always |
| Trim whitespaces | checkbox |
| First row is header / First column is header | checkboxes |
| Preview | 10 records |

**File as table**: *Edit as Table* (`FileEditor.OpenDataEditor`) for DSV, *View as Table* (`FileEditor.OpenDataViewer`) read-only; dialog "Configure CSV Format for <file>"; Text/Data tabs at the editor bottom; edits rewrite exact text ranges (`ValueRange`, `CsvDocumentDataMutator`, "Updating file…"); *Change CSV Format of File…*; "First Row Is Header" toggle in header popup; *Import to Database…* toolbar button. File type "CSV/TSV Data"; tabular data files "Tabular data file". Detection sample `database.csv.preview.char.count=10000`. Type inference `TypeMerger` (`BooleanMerger`, `IntegerMerger`, `BigIntegerMerger`, `DoubleMerger`, `StringMerger`). [Observed]

### 3.24 Data compare

- *Compare Data* (Database Explorer › Tools, or grid toolbar → pick the other table/result): tables, views, materialized views, result sets, mixed kinds; rows limited by page size. Diff viewer colors: differing rows, differing cells, equal rows; **Tolerance** = number of columns allowed to differ for two rows to count as "equal" (then differing cells highlighted); **Detect column insertion** — with unequal column counts, ignore the most-different column(s) instead of the trailing ones; sorting inside the viewer. Table Diff color page includes "Excluded from diff" / fuzzy-match colors. [Observed] Algorithm ALG-GRID-12.

### 3.25 Defaults and registry keys relevant to the grid

| Key / setting | Value | Evidence |
|---|---|---|
| Limit page size to | 500 | help |
| Maximum bytes per value (image preview context) | 1024 | help |
| `database.grid.paging.offset` | true | registry |
| `database.result.set.limit` | 100 | registry |
| `database.batch.size` | 2000 (batch size for DML/import [Inferred]) | registry |
| `database.csv.preview.char.count` | 10000 | registry |
| `grid.tables.columns.limit` | 2000 | registry |
| `database.grid.show.chart.tab` | true | registry |
| `database.grid.render.cache` / `.fast.scroll` / `database.grid.cache.factor` | true / true / 3 | registry |
| `database.automatically.load.related.data.in.cell.quick.doc` | true | registry |
| `database.scrollable.result.sets` | true | registry |
| Related Rows behavior | Navigate and filter all | bundle/help |
| Add columns to sorting | Alt+Click | help |
| Decimal separator | `.` | help |
| Other DV defaults | see doc 11 §3.5 ([Unknown] items → doc 13 SET) | |

---

## 4. Algorithm specifications

### ALG-GRID-01: Paste format detection and shape mapping
- **Problem:** Interpret clipboard text pasted into a grid as a 2-D block (or a single value) and map it onto the selection.
- **Inputs / outputs:** clipboard (text + optional HTML/internal flavor), target selection (anchor, selected range, column domains, read-only flags, transposed?), paste format (Detect | named CSV format | Single value) → `Block{rows: Vec<Vec<Cell>>, header: Option<Vec<String>>}` + per-cell parse results + a user notification naming the format used.
- **Algorithm:**
  1. If format ≠ Detect, parse with that format (ALG-GRID-17 reader) → step 6. Notify "pasted using *F*".
  2. Internal grid flavor (copied from Hacivat) → use typed values verbatim (NULLs preserved) → step 7.
  3. HTML flavor with `<table>` → cells from text content (`<br>` → `\n`).
  4. Else run ALG-GRID-02 on the text restricted to the user's format list (TSV, CSV, Semicolon, Pipe, user formats); pick the best consistent candidate (≥2 columns or ≥2 rows, consistent column count, no quote errors). Notify "*F* format was detected".
  5. No candidate → **single value**: whole text (newlines included) into the anchor cell, or into every selected cell. Notify "No CSV format was detected. Text was pasted as single value."
  6. Header: drop the first row only if the chosen format says *First row is header* or all its values equal target column names (case-insensitive) [Inferred]; apply the format's NULL text.
  7. Shape: 1×1 block + multi-cell selection → fill all; else start at the anchor (top-left of selection); rows beyond the end → append new rows (inserts); columns beyond the end → truncate with a warning; transposed view → swap axes before mapping; skip read-only/generated/computed targets (warn).
  8. Convert each string through the target column's editor parser (ALG-GRID-15) → value or `Unparsed{text, err}`.
- **Data structures:** `PasteFormat`, `Block`, `CellParse = Ok(Value) | Unparsed`.
- **Complexity:** O(n) over clipboard; cap at e.g. 50 MB / 1 M cells with confirmation.
- **Edge cases:** Excel TSV with quoted multi-line cells; trailing newline (ignore final empty row); CRLF/CR; decimal comma with semicolon CSV vs comma CSV; leading `=` formulas (text); leading zeros into text vs int; JSON array text (single value unless JSON format chosen); Markdown pipe table (border trim); mixed NULL markers; empty clipboard (no-op); clipboard with only separators (`,,,` → 4 empty cells); a single tab (`\t` → two empty cells); selection spanning hidden columns (skip hidden); pasting into a filtered/sorted view (map by view index → model row); pasting more rows than page size (new rows appended at end of model, not at view position); pasting into a deleted row (reject); pasting onto a cell currently being edited (goes into the editor text, not the grid); value longer than LOB limit; RTL text unchanged.
- **Test cases:** `a\tb\n1\t2` → 2×2 TSV; `x` → single; `"a,b",c` → 2 cols `a,b`|`c`; `1,5;2,7` into numeric with TR display format → Semicolon, cells 1.5, 2.7; `1\n2\n3` into 1-row selection of a 1-row table → updates row 1, inserts 2 rows; `\N` with a format whose NULL text is `\N` → NULL; same with CSV (Undefined) → literal `\N`; doc 13 PASTE-01..24.
- **Prior art:** [Observed: `ChoosePasteFormatAction$FormatParser/SingleValueParser/PasteType`, `GridPasteProvider$TableDataParser`, notifications, help "Paste CSV lines"].
- **Used by:** Hacivat.

### ALG-GRID-02: CSV dialect sniffing (format detection)
- **Problem:** Detect separator, quote/escape, header, NULL text, encoding for a file or clipboard sample.
- **Inputs / outputs:** bytes (≤10,000 chars after decoding), candidate formats → `CsvDialect{sep, row_sep, quotes, quote_policy, header, null_text, trim, encoding, bom}` + confidence.
- **Algorithm:**
  1. Encoding: BOM → UTF-8 strict → UTF-16 by NUL pattern → user/OS fallback (e.g. Windows-1254) with visible indicator.
  2. Row separator: count `\r\n`, `\n`, `\r` outside quotes; most frequent wins.
  3. For each separator ∈ {`,`, `\t`, `;`, `|`, user} × quote ∈ {`"`, `'`, none}: tolerant tokenize (quoted fields may contain separators/newlines; doubled quote = escape; backslash escape as alternative); field counts over complete rows (drop the last partial row).
  4. Score = consistency (share of rows with modal count) × [modal count > 1] × quote-error penalty × prior (tab wins if consistently present; then comma > semicolon > pipe).
  5. Header if: first-row fields non-empty and unique, and ≥1 column where rows 2..N parse as number/date/bool but row 1 doesn't, or names look like identifiers.
  6. NULL text: `\N` as whole field → `\N`; distinguish empty unquoted vs `""` when the format specifies.
  7. Preselect in "Configure CSV Format" with 10-row preview; user can override.
- **Edge cases:** single-column files (fallback by extension `.tsv`); quoted header; trailing separator on every line (drop empty last column); `#` comment lines; ragged rows (pad NULL + mark); records crossing the 10,000-char boundary; Excel TR/DE semicolon + decimal comma; NUL bytes; mixed quotes; `sep=;` first line; empty file (no format; Data tab shows empty grid); file with only a header; BOM + `sep=` line together; very long single line (>sample) → treat whole sample as one partial record → fall back to extension.
- **Test cases:** doc 13 CSV-01..20; `a;b\n1,5;2\n` → semicolon; `a,b\n"x,y",1\n` → comma with quotes; `id|name\n1|a` → pipe, header.
- **Prior art:** [Observed: `CsvFormatResolverCore`, `CsvFormatParser$ImproperQuotedValueRange`, registry 10,000]; Python `csv.Sniffer` (PSF), `csv-sniffer` crate (MIT).
- **Used by:** Hacivat (open file as table, import, paste); Karagöz (server-side import jobs, optional).

### ALG-GRID-03: Type widening (column type inference)
- **Problem:** Infer a column type from string samples (import into new table, CSV column typing).
- **Algorithm:** lattice per column starting at `Unknown`; per non-NULL sample classify `Boolean`, `Integer` (i32), `BigInteger` (i64/arbitrary), `Double`, `String` (+ Hacivat `Date`, `Timestamp`, `Uuid`, `Json`); merge: `Unknown ⊔ x = x`; `Boolean ⊔ Integer = Integer` only if booleans were 0/1 else `String`; `Integer ⊔ BigInteger = BigInteger`; `{Integer,BigInteger} ⊔ Double = Double` (Hacivat: `Decimal(p,s)` tracking max precision/scale); `x ⊔ String = String`. Track max length, nullable if any NULL/empty. Stop early at `String`.
- **Edge cases:** leading zeros (`007` → String if length>1 and leading zero); thousands separators; locale decimal commas (only per format); `NaN`/`Infinity`; > i64; scientific notation; empty vs NULL; Turkish booleans (not by default); mixed date formats (String); TZ; UUID case; trailing spaces with Trim off; `-0`; `+5`; `1e400` (overflow → String/Decimal).
- **Test cases:** doc 13 WIDEN-01..18; `[1, 2, 3000000000]` → BigInteger; `[1, 2.5]` → Double/Decimal(2,1); `[true, 1]` → String.
- **Prior art:** [Observed: `TypeMerger$*Merger`].
- **Used by:** Hacivat (import, file-as-table); Karagöz import endpoints.

### ALG-GRID-04: Pending-changes overlay
- **Problem:** Hold user edits without touching loaded data; render, revert, preview and submit them.
- **Data structures:** `PendingChanges { updates: IndexMap<RowKey, RowUpdate{cells: BTreeMap<ColId, NewValue>, original: RowSnapshot}>, inserts: Vec<NewRow{temp_id, cells}>, deletes: IndexSet<RowKey>, column_ops (DSV only) }`; `NewValue = Value | Null | Default | Generated | Unparsed{text, err}`; `RowKey` = identity values (§3.5), never view index.
- **Algorithm:**
  1. Edit cell: inserted row → update its cells; deleted row → reject; else upsert `updates[key].cells`; if new == original → remove (no-op edit); if row has no remaining cells → remove row entry.
  2. Delete rows: inserted rows dropped from `inserts`; existing → `deletes`, their updates discarded.
  3. Revert(selection): remove matching cell updates / inserts / deletes in range; empty selection → all.
  4. Render: overlay lookup by (RowKey, col); layers stripe → mutation → search → selection.
  5. Paging/refresh: overlay persists; header shows "N pending changes"; reload re-matches by RowKey; vanished keys → orphaned changes (shown in preview, flagged).
  6. Submit enabled iff no `Unparsed`.
  7. *Submit changes immediately* → run ALG-GRID-05/06 after every confirmed edit, overlay never holds >1 row.
  8. Leaving the grid with non-empty overlay → "Changes are not submitted. Data will be lost. Continue?".
- **Edge cases:** editing a key column (WHERE keeps original key); duplicate keys in results (disable editing); rows without identity; transposed edits; same table in two grids (Hacivat: overlay per grid; on submit, refresh other grids of the same table); truncated LOB cells (full replacement only); edit then delete then revert (restores original + edit? → revert restores to pre-delete state including the edit [Hacivat choice]); clone of an inserted row; insert then edit key; set DEFAULT on a column without default (→ NULL or error per DB); undo stack interplay (undo of an edit after submit not allowed).
- **Test cases:** edit A→B→A ⇒ no pending changes; insert row, delete it ⇒ empty; edit row 1, delete row 1, revert ⇒ row 1 edited (Hacivat); page 2 edit, go to page 1 ⇒ counter 1.
- **Prior art:** [Observed: `MutationsStorageImpl`, `MutationsColorLayer`, `UnparsedValue`, dialogs].
- **Used by:** Hacivat.

### ALG-GRID-05: DML generation for edits
- **Problem:** Turn the overlay into correct, minimal, dialect-correct DML.
- **Algorithm:**
  1. Order: deletes → updates → inserts [Inferred; verify DML-15]; within each, user order.
  2. **UPDATE** per row: only changed columns; `Default` → `DEFAULT` (unsupported in SQLite → error); generated/identity excluded; WHERE on original key values (`IS NULL` never for PK).
  3. **DELETE**: per row, or batched `WHERE k IN (…)` for single-column keys (`BaseDeleteRowsGenerator`; ClickHouse `ALTER TABLE … DELETE`).
  4. **INSERT**: multi-row (`MultiInsertRowsTask`; Oracle `INSERT ALL`), omitting Default/Generated columns; all-default row → `DEFAULT VALUES` (PG/MSSQL/SQLite) or `() VALUES ()` (MySQL); fetch generated keys (`RETURNING`/`OUTPUT`/`LAST_INSERT_ID()`/`SCOPE_IDENTITY()`).
  5. Values as bind parameters; literal rendering only for *Preview* (extractor literal rules).
  6. Qualified, quoted identifiers per dialect.
  7. Preview lists statements in execution order.
- **Edge cases:** composite keys; float/timestamp keys (bind exact originals); case-sensitive names; reserved words; PG casts for json/array/enum/uuid/bytea/interval/range; MySQL BIT; MSSQL rowversion (never update); computed columns; triggers affecting other rows; TZ conversions; float display precision; updatable views; result aliases → base columns (PG RowDescription OID/attnum, MySQL org_table/org_name, MSSQL `sp_describe_first_result_set`); batch size limit (`database.batch.size=2000`) → split; MSSQL 2100-parameter limit → split multi-row inserts; Oracle 1000-item IN limit.
- **Test cases:** doc 13 DML family; single cell edit on PG `t(id pk, a)` → `UPDATE "public"."t" SET "a" = ? WHERE "id" = ?`; insert with all defaults on MySQL → `INSERT INTO t () VALUES ()`.
- **Prior art:** [Observed: task/generator class names, Preview action].
- **Used by:** Hacivat; Karagöz receives parameterized DML and authorizes per table/column.

### ALG-GRID-06: Affected-row verification and re-read
- **Algorithm:** one transaction for the batch (Manual: user's tx; Auto: implicit tx + commit); after each UPDATE/DELETE require `rows_affected == 1`: 0 → conflict ("row changed or deleted by someone else") → rollback (Auto) or stop (Manual); >1 → rollback + error. Optional optimistic check (original values of edited columns, PG `xmin`, MSSQL `rowversion`). After commit, `SELECT … WHERE key IN (…)` (`SelectUsingKeys`) to refresh server-generated values; clear overlay.
- **Edge cases:** MySQL found vs changed rows (`CLIENT_FOUND_ROWS`); `SET NOCOUNT ON` triggers; ClickHouse async mutations; inserts without identity (re-read by inserted values); partial failure in Manual (keep remaining pending); deadlock/serialization failure (offer retry); connection loss mid-batch (tx state unknown → force reload).
- **Prior art:** [Observed: `SelectUsingKeys`; Unknown: DataGrip count checks → DML-18/19].
- **Used by:** Hacivat; Karagöz ("max affected rows" policy).

### ALG-GRID-07: Server-side filter/sort query wrapping
- **Algorithm:** table editor: `SELECT <cols> FROM <table> [WHERE (<user_where>)] [ORDER BY <user_order | header sort | numeric PK default>] <paging>`; query result: wrap `SELECT * FROM (<query>) q …` (`WrapInSelect`; Oracle no `AS`; MSSQL ORDER BY-in-subquery rules). User text inserted in parentheses **followed by a newline** (protects against trailing `--` comments); *Filter by* predicates built as AST then rendered; header sorts as quoted column references; validate user text by parsing as an expression before sending ("Invalid filter criteria" / "Invalid sorting").
- **Edge cases:** semicolons (reject); alias references; ORDER BY ordinals; inner ORDER BY/LIMIT; CTEs (MSSQL can't wrap — hoist); procedure calls (not wrappable); `FOR UPDATE`; collation; WHERE referencing columns hidden in the grid (fine); WHERE using bind-like `?` (treat as text, warn).
- **Prior art:** [Observed: `WrapInSelectTask`, `ConditionQueryRequest`, `GridFilterAndSortingComponent`].
- **Used by:** Hacivat.

### ALG-GRID-08: Keyset vs offset paging
- **Algorithm:** tables with unique ordered key → keyset (`WHERE (k) > (?) ORDER BY k LIMIT n`), Previous = reverse order + reverse, Last = `ORDER BY … DESC LIMIT n`; arbitrary jumps / non-unique ORDER BY → offset or keyset with PK tiebreaker. Results: keep cursor/portal open and fetch next n; Previous from client buffer (ring of K pages) or re-execute with offset.
- **Edge cases:** NULL ordering per DB; mixed directions (OR-chain expansion); concurrent inserts; page size change mid-way; cursor invalidated by commit (PG `WITH HOLD`/re-execute); MSSQL lacking row-value comparison.
- **Prior art:** [Observed: `database.grid.paging.offset=true` → DataGrip uses OFFSET]; Winand "Use the index, Luke".
- **Used by:** Hacivat.

### ALG-GRID-09: LOB truncation and lazy load
- **Algorithm:** fetch at most `max_bytes` per variable-length value (PG `substring(col from 1 for n)` + `octet_length(col)` in table editors; driver partial reads); store `Truncated{prefix, total_len}`; render "n of total"; full-value actions (value editor, copy, Save LOB, clone, export) trigger `SELECT col FROM t WHERE key = ?` (needs identity; results without identity → partial-copy warning with link to raise the limit, "Apply and Reload"). Editing a truncated value requires full load first.
- **Edge cases:** multi-byte truncation on char boundary; image header sniffing; truncated JSON (no pretty-print); unknown total ("≥ n"); export must bypass truncation (stream full values).
- **Prior art:** [Observed: bundle "{0} of {1}", partial copy, clone refusal, Change Size Limit dialog].
- **Used by:** Hacivat.

### ALG-GRID-10: Extractor / aggregator engine
- **Algorithm:** registry = native built-ins + scripts in `extensions/data/extractors/*.{rhai,lua,js}` named `Name.ext.lang`; context = {columns, all_columns, rows (lazy), out (backpressured sink), formatter, transposed, table, dialect}; sandbox (time/memory limits; no FS/network unless granted); export streams DB cursor → script → file. Aggregators: same context, single string output, debounced off-thread on selection change, cancel on new selection. Upgrade policy via hashes of bundled versions.
- **Edge cases:** huge selections; script errors (notification with line); non-numeric cells; decimals beyond f64; transposed export; nested documents; binary values (per display type); script producing invalid UTF-8; script never terminating (timeout); name collisions between bundled and user scripts (user wins).
- **Prior art:** [Observed: bindings, bundled scripts, `extensions_old` hashes, error notifications].
- **Used by:** Hacivat (copy, export, text view, MCP "export result").

### ALG-GRID-11: Local filter
- **Algorithm:** per column `allowed: Option<HashSet<ValueKey>>` + optional text predicate; candidate list = distinct values in loaded rows with counts (sorted by count desc, then value; top-K + search box); row visible iff all column filters accept; status "Number of matches N"; *By Cell Value* sets `allowed = {cell}`; recompute view permutation (ALG-GRID-13) incrementally; NULL and empty string distinct; toggle off keeps state.
- **Edge cases:** only loaded rows (banner); pending inserted rows (always visible); floats compared exactly; 100k distinct values (cap list); values changed by pending edits (filter on displayed new value); case sensitivity (exact by default); local filter + server WHERE combined (local applies after).
- **Test cases:** 3 rows `a,a,b` → popup `a(2), b(1)`; check `b` → 1 row, "Number of matches 1".
- **Prior art:** [Observed: `LocalFilterState`, `LocalFilterPopup`, bundle].
- **Used by:** Hacivat.

### ALG-GRID-12: Data diff with tolerance
- **Algorithm:** align columns by name (case-insensitive) else position; unequal counts + Detect column insertion → drop the column(s) with most mismatches over a sample (greedy), else drop trailing; rows: join by shared key, else sort by compared columns and run Myers diff on row hashes; for paired candidates compute differing columns d; equal if d=0, "equal with differences" if d ≤ tolerance (highlight cells), else delete+insert.
- **Edge cases:** int vs numeric normalization; NULL = NULL; float epsilon; large sets (server-side `EXCEPT`/hash — Hacivat addition); duplicates (multiset); different column types with same names (compare normalized text).
- **Prior art:** [Observed: help compare-data].
- **Used by:** Hacivat.

### ALG-GRID-13: Model index ↔ view index mapping
- **Algorithm:** `view_rows: Vec<ModelRow>` after local filter + local sort; `view_cols: Vec<ModelCol>` after hide/reorder; transpose swaps axes at render; tree/text views have their own converters. Selection stored in model coordinates. Newtypes `ModelRow(u32)`, `ViewRow(u32)`.
- **Prior art:** [Observed: `ModelIndex`, `ViewIndex`, `RawIndexConverter` variants].
- **Used by:** Hacivat.

### ALG-GRID-14: Filter-by predicate generation
- **Problem:** Build WHERE predicates from a cell selection and a template, append them safely to the WHERE field.
- **Inputs / outputs:** selected cells (column, value, type), template ∈ {`=`, `<>`, `<`, `<=`, `>`, `>=`, `LIKE`, `NOT LIKE`, `IS NULL`, `IS NOT NULL`}, operand ∈ {grid values, clipboard text, NULL, placeholder}, current WHERE text → new WHERE text + caret position (for placeholder).
- **Algorithm:**
  1. Group selected cells by column; per column collect distinct values.
  2. If template is `=` and a column has >1 value → `col IN (v1, v2, …)`; `<>` → `col NOT IN (…)`; comparisons use the first value only (or disabled for multi-value).
  3. NULL values: `=` with NULL → `col IS NULL`; IN-list with NULL → `(col IN (…) OR col IS NULL)`.
  4. LIKE: operand text; with grid operand, use the value as-is (user edits wildcards later); escape `%`/`_` only if the "exact" variant is chosen [Hacivat].
  5. Render literals with the dialect literal renderer (strings quoted/escaped, `N'…'` for T-SQL Unicode, dates as typed literals `DATE '…'`/`TIMESTAMP '…'`, binary as hex literals, booleans per dialect); quote identifiers.
  6. Combine columns with `AND`; append to existing WHERE: `(<old>) AND <new>` if old non-empty.
  7. Placeholder operand: insert `col = ` and select the empty operand position in the WHERE field for typing.
  8. Apply (re-query) unless placeholder.
- **Edge cases:** values with quotes/newlines; very long values (truncated LOB → refuse with message); JSON/array/geometry columns (equality may be unsupported — offer `::text =` in PG or disable); float values (equality on display rounding → use exact stored representation); timestamps with TZ; binary display types (UUID shown vs bytes stored → render bytes literal); case-insensitive collations; clipboard containing multiple lines (→ IN list); existing WHERE ending with a `--` comment (newline before `AND`); existing WHERE with OR (parenthesize).
- **Test cases:** cells `name='O''Brien'` → `name = 'O''Brien'`; two cells 1,2 in `id` → `id IN (1, 2)`; NULL cell with `=` → `col IS NULL`; old WHERE `a=1 -- x` + new `b=2` → `(a=1 -- x\n) AND b = 2`.
- **Prior art:** [Observed: `ColumnFilterByGroup$FilterTemplate/LikeFilterTemplate/NullFilterTemplate/GridOperand/ClipboardOperand/NullOperand/PlaceholderOperand`, `ValuesWhereClause`].
- **Used by:** Hacivat; MCP tool "filter table".

### ALG-GRID-15: Value display formatting and lenient parsing (round-trip)
- **Problem:** Show values with user formats and parse edited text back without losing precision.
- **Inputs / outputs:** `Value`, column logical type, settings (number format, date/time patterns, display TZ, boolean mode, binary/number display type) → display string; text → `Value | Unparsed{kind}`.
- **Algorithm:**
  1. Format: NULL → styled `<null>` marker (not text); numbers via decimal/grouping separators and optional pattern, `Infinity`/`NaN` texts; temporal via kind-specific pattern in display TZ (timestamptz converted; naive timestamp unchanged); binary per display type (Hex, Hex+ASCII, UUID, swapped UUID, Text with encoding); number display types: epoch seconds/millis → timestamp text.
  2. Parse (lenient, per type): trim; numbers accept both the display format and invariant format (`.` decimal), reject ambiguous grouping (e.g. `1.234` when `.` is grouping and `,` decimal → 1234); booleans accept `true/false/t/f/1/0/yes/no` (+ letters for special values in text mode); temporal: try display pattern, then ISO-8601 variants, then epoch digits (if number display type); TZ offsets validated (−12:00..+14:00); UUID 32/36-char hex; binary hex with/without `0x`; JSON validated by parser.
  3. Errors map to typed messages (Invalid number, Expected boolean value, Expected timestamp…).
  4. Round-trip guarantee: `parse(format(v)) == v` for every non-lossy display; when display is lossy (rounded float, truncated LOB), editing starts from the exact stored value text, not the display.
  5. Out-of-range → offer boundary value (ALG: max/min of domain or ±infinity where DB supports).
- **Edge cases:** negative zero; very small/large doubles (scientific in display); DECIMAL(38,10) beyond f64; PG `infinity` timestamps; MySQL zero dates `0000-00-00`; Oracle DATE with time part; time 24:00:00 (PG allows); leap seconds; DST gaps in display TZ (show offset); Turkish locale with `,` decimal and `.` grouping; Arabic-Indic digits input (normalize); boolean text mode with `d`/`g`/`c` letters on non-boolean columns (ignored).
- **Test cases:** `1.234,5` with TR format → 1234.5; `1,234.5` with TR format → Unparsed(Invalid number); `2026-11-11T10:00:00+03:00` into timestamptz with display TZ Europe/Istanbul → displays `2026-11-11 10:00:00.000000 +03:00`; `+15:00` offset → Unparsed("Offset out of range").
- **Prior art:** [Observed: `FormatBasedGridCellEditorFactory$LenientFormatter`, `BoundaryValueResolver`, display types, settings].
- **Used by:** Hacivat (grid, record view, import conversion).

### ALG-GRID-16: Aggregator numerics
- **Problem:** Compute selection aggregates exactly and fast.
- **Inputs / outputs:** selected cells (values + display strings), enabled aggregators → map name → result string.
- **Algorithm:**
  1. Single pass over selection: `count_cells`, distinct rows/cols sets (for ROWS/COLS), numeric extraction: native numerics → Decimal (128-bit / arbitrary), strings parsed with invariant + display number format; NULL ignored.
  2. SUM = Σ; AVG = Σ / n_num (scale = max(input scale, 10) [Hacivat]); MIN/MAX on numbers (fallback to string compare when no numbers [Hacivat: optional]); MEDIAN: collect numerics, select k-th via quickselect (n odd → middle; even → mean of two); COUNT = cells; COUNT_NUMS = n_num; CV = sample stddev / mean (n_num ≥ 2 and mean ≠ 0, else "Not enough values").
  3. Format with the grid number formatter.
  4. n_num = 0 for numeric aggregators → "Not enough values".
- **Complexity:** O(n) except MEDIAN O(n) average; cap selection (e.g. 10 M cells) and stream from server for full-table aggregates (`SELECT SUM(col)`) when the selection is "whole column" and data is paged [Hacivat addition].
- **Edge cases:** mixed types; overflow (Decimal); `NaN`/`Infinity` (propagate or skip — skip [Hacivat]); booleans (not numeric); currency strings (`$1,000`, not numeric); hidden columns inside a range (excluded); transposed selection; pending edited values (use new values).
- **Test cases:** `[1, 2, "3", NULL, "x"]` → SUM 6, AVG 2, COUNT 5, COUNT_NUMS 3, MEDIAN 2; `[0.1, 0.2]` SUM → `0.3` (not `0.30000000000000004`); `[5]` CV → Not enough values.
- **Prior art:** [Observed: bundled aggregators behavior, "Not enough values"].
- **Used by:** Hacivat (status bar, aggregate panel, MCP "summarize selection").

### ALG-GRID-17: CSV writer and reader (format-driven)
- **Problem:** Serialize/parse rows using a user-defined CSV format (separators, prefix/suffix, NULL text, quote patterns, policy, trim, headers).
- **Inputs / outputs:** `CsvFormat{value_sep, row_sep, row_prefix, row_suffix, null_text: Undefined|Empty|Text(s), quotes: Vec<(left,right,escape: Duplicate|Char(c))>, policy: Never|WhenNeeded|Always, trim, first_row_header, first_col_header}`; rows ↔ text.
- **Algorithm (write):** for each row: emit prefix; for each value: NULL → null_text (Undefined → empty, unquoted); else text = formatted value; needs_quote = policy Always, or WhenNeeded and text contains value_sep, row_sep, left/right quote, or prefix/suffix sequences [Hacivat adds: leading/trailing whitespace when trim is on]; if quoting use pattern 0: escape occurrences of right quote by doubling (Duplicate) or by preceding escape char (also escape the escape char); join with value_sep; emit suffix + row_sep. Header row/column first when configured.
- **Algorithm (read):** streaming state machine: at field start, try each quote pattern's left char (any pattern accepted); inside quotes consume until matching right quote not escaped; unquoted fields end at value_sep or row_sep (strip prefix/suffix at row boundaries); trim when enabled (outside quotes only); NULL detection: unquoted field equal to null_text (Empty → unquoted empty field is NULL, quoted `""` is empty string); improper quotes (quote char mid-field, unterminated quote) → keep raw text and mark range (`ImproperQuotedValueRange`) rather than fail; keep each value's text range for write-back (`ValueRange`).
- **Edge cases:** multi-char separators (`||`); row separator `;` with values containing newlines; prefix `(` suffix `),` (SQL-ish); escape char equal to quote char; Never policy with value containing separator (output is ambiguous — warn on export); trailing row separator at EOF; empty last line; CRLF vs LF row separator mismatch (accept both when row sep is Newline); BOM; first column header with transposed data; header names duplicated or empty (auto-name `C1..Cn`, as the Import dialog shows).
- **Test cases:** value `a"b` with `"`/Duplicate, WhenNeeded → `"a""b"`; same with escape `\` → `"a\"b"`; NULL with Empty null text → `` (empty) and reading `,,` → NULL,NULL while `"",""` → '',''; prefix `[` suffix `]` sep `,` → `[1,2]`; Never + value `a,b` → `a,b` (round trip breaks, warning).
- **Prior art:** [Observed: `CsvRecordFormat`, `QuotationPolicy`, `Quotes{left,right,escape}`, `CsvFormatParser`, help CSV Formats]; RFC 4180; `csv` crate (MIT/Unlicense).
- **Used by:** Hacivat (copy/paste/export/import/file editing); Karagöz (export endpoints).

### ALG-GRID-18: Import pipeline (file/table/query → table)
- **Problem:** Load rows from a source into a new or existing table with mapping, conversion, error capture and speed options.
- **Inputs / outputs:** source (CSV with format/charset, loader file, table, query), mapping {schema, table (new|existing), column pairs (target ← source | Auto), types for new columns}, options {write_errors_to: Option<Path>, inconvertible_as_null, disable_constraints_and_lock}, batch size → `ImportReport{rows_ok, rows_failed, errors_file, duration}`.
- **Algorithm:**
  1. Resolve source schema: CSV → header or `C1..Cn`, types via ALG-GRID-03 over a sample; loader → declared columns; table/query → result metadata.
  2. Auto mapping: by case-insensitive name, else by position; report "Columns are not mapped" if none.
  3. New table: generate CREATE TABLE (editable DDL Preview); confirm schema changes ("Table Schema Will Be Changed") when altering an existing table [Inferred: when user edits target columns].
  4. If disable option: dialect-specific (PG `ALTER TABLE … DISABLE TRIGGER ALL` + `LOCK TABLE`, MySQL `ALTER TABLE … DISABLE KEYS` + `LOCK TABLES`, MSSQL `ALTER INDEX … DISABLE` / `NOCHECK CONSTRAINT`), always re-enabled in `finally`.
  5. Stream source rows; convert each value to the target type (ALG-GRID-15 parse + `Compatibility`); inconvertible → NULL (option) or row error.
  6. Insert in batches (`database.batch.size`, default 2000) with parameterized multi-row INSERT or native bulk paths (PG `COPY FROM STDIN`, MySQL `LOAD DATA LOCAL` if allowed, MSSQL bulk copy) [Hacivat]; on batch failure, retry row-by-row to isolate bad rows; write failures (line number, source text, error) to the errors file; continue.
  7. Transaction: per batch commit (DataGrip behavior [Unknown]); Hacivat option "all-or-nothing".
  8. Notify "{source} imported to {table}" with link; on fatal error "Import failed".
- **Edge cases:** encoding mismatches (BOM vs selected charset); header row with duplicates; more source columns than target (unmapped ignored); fewer (target defaults/NULL); NOT NULL target with NULL source and inconvertible_as_null on (still fails); identity columns (skip or `IDENTITY_INSERT ON`); generated columns (skip); target in another data source (cross-connection streaming); query source with parameters; triggers disabled → trigger side effects missing (documented warning); huge files (streaming, progress); empty file ("Table is empty"); Excel dates as serial numbers; Excel formulas (computed values); cancel mid-import (partial data; report).
- **Test cases:** doc 13 IMP family; CSV `id,name\n1,a\nx,b` into `t(id int, name text)` with errors file → 1 row imported, errors file has line 3; same with inconvertible-as-null → 2 rows (id NULL).
- **Prior art:** [Observed: Import dialog fields/messages, properties `InconvertibleAsNull`, `DisableConstraints`, `ErrorsPath`].
- **Used by:** Hacivat; Karagöz (server-side import with permission check on target table; rate limits).

### ALG-GRID-19: Go to Row parsing
- **Problem:** Parse the "Go to Row" input.
- **Algorithm:** accept `<row>`, `<row>:<column>`, `<row>,<column>` (bundle "<row>[:<column>] or <row>[,<column>]"); row = 1-based integer in view order (over all pages? → if beyond the loaded page, load the page containing it [Inferred]); column = 1-based index or column name (case-insensitive, quoted names allowed); trim spaces; invalid → inline error.
- **Edge cases:** `0` or negative; row beyond total; column name containing `:` or `,` (allow quoting `"a:b"`); transposed view (row/column refer to model axes); hidden column (unhide or error); filtered view (row number refers to visible rows).
- **Test cases:** `12` → row 12, current column; `12:3` → row 12, col 3; `12,name` → row 12, column `name`; `abc` → error.
- **Prior art:** [Observed: bundle `row.column.or.row.column`, `action.row.choice.col.text`].
- **Used by:** Hacivat.

### ALG-GRID-20: Column statistics header
- **Problem:** Show per-column summaries above headers (Off / Compact / Detailed).
- **Algorithm:** over loaded rows (or a server-side summary query when the user asks for "whole table"): per column compute null count, distinct count (exact up to N, then HyperLogLog), min/max (comparable types), top-K values with counts (Space-Saving), numeric mean; Compact = one line (e.g. distinct + nulls); Detailed = mini histogram/top values; recompute incrementally on page load, debounced; skip LOB/binary.
- **Edge cases:** mixed types in document grids; very wide tables (lazy per visible column); pending edits (stats on displayed values); transposed view (stats per row → disable).
- **Test cases:** column `[a,a,NULL]` → distinct 1, nulls 1, top `a×2`.
- **Prior art:** [Observed: `StatisticsTableHeader`, `StatisticsPanelMode`, actions Off/Compact/Detailed; metrics Inferred].
- **Used by:** Hacivat.

### ALG-GRID-21: Find on page with row filtering
- **Problem:** Search loaded cells with case/regex/word options and optionally hide non-matching rows.
- **Algorithm:** matcher = literal (case-folded unless Match case) | regex (compiled; invalid → red field) | whole-word wrapper; search over **display strings** of visible columns in view order; collect matches; Next/Prev cycle with wrap; *Filter rows* → view permutation keeps rows with ≥1 match; selection actions (Select All Occurrences) select matched cells.
- **Edge cases:** NULL cells (match `null` text? → no, unless searching the NULL marker [Hacivat: no]); values truncated (search prefix only; note); Unicode case folding incl. Turkish dotted/dotless I (locale-aware option); newlines in values (multi-line toggle); 1M cells (incremental, cancel on new query); hidden columns ignored.
- **Test cases:** "john" case-insensitive on screenshot data → 4 rows with Filter rows on.
- **Prior art:** [Observed: find bar toggles, `DataGridSearchSession$ToggleFilteringAction`].
- **Used by:** Hacivat.

### ALG-GRID-22: Auto-refresh scheduler
- **Problem:** Periodically reload a grid without disrupting the user.
- **Algorithm:** state {interval: Option<Duration>, paused}; timer runs only while the grid is visible; on tick: skip if a request is running, if pending changes exist (prompt or skip [Hacivat: skip + badge]), or if a cell editor is open; else reload the current page preserving selection by RowKey and scroll position; custom interval ≥ 1 s; Pause keeps interval; Disabled clears it.
- **Edge cases:** connection lost (back off exponentially); long queries longer than interval (no overlap); Manual tx open (reload inside tx sees own changes); multiple grids on same table (independent timers); laptop sleep (fire once on wake).
- **Test cases:** interval 5 s, query takes 7 s → refreshes every ~7 s, never concurrent.
- **Prior art:** [Observed: `GridAutoRefresher` (refresh only when shown), `PageAutoRefreshGroup` (intervals, custom, pause)].
- **Used by:** Hacivat.

### ALG-GRID-23: JSON file → table (loader semantics)
- **Problem:** Present a JSON file as a table.
- **Algorithm:** streaming parse; top-level array of objects → columns = keys in first-seen order across all objects (dynamic column allocation), rows = objects, missing → NULL; array of arrays → columns `C1..Cmax`; array of scalars → single column; top-level object → name/value table; scalars → single cell; nested objects/arrays stay as structured values (tree view / JSON extractor); column type = type of the first non-null value, widened via ALG-GRID-03 [Hacivat improvement over first-value typing].
- **Edge cases:** heterogeneous arrays (objects and scalars mixed → scalars in a `value` column); duplicate keys in an object (last wins); JSON Lines (`.jsonl` — Hacivat addition); huge files (stream, page); BOM; NaN/Infinity tokens (non-standard → String); key order differences between rows.
- **Test cases:** `[{"a":1},{"b":2}]` → columns a,b; rows (1,NULL),(NULL,2); `[[1,2],[3]]` → C1,C2 rows (1,2),(3,NULL).
- **Prior art:** [Observed: bundled JSON loader structure and data-consumer calls].
- **Used by:** Hacivat (file-as-table, import).

---

## 5. Implications for Hacivat

1. **Data plane**: Arrow `RecordBatch` pages with a `Value` enum for edits/unparsed (`Null, Bool, I64, U64, F64, Decimal, Text, Bytes, Uuid, Date, Time, Timestamp{tz}, Interval, Json, Array, Geometry, Truncated{prefix,len}, Unparsed{text,err}, Default`).
2. **Column metadata** from drivers to enable editing of query results (source table/column ids).
3. **Grid UI**: canvas-based virtualized grid with layered colors; record view; value editor using the SQL editor component; floating cell toolbar; statistics header; local filter popup; find bar with Filter rows.
4. **Capability matrix as data**: each surface declares capabilities (server filter, server sort, editable, columns editable, chart mode) — UI and MCP derive available actions from it (mirrors §3.2).
5. **Action ids and keymap**: keep stable English action ids (`grid.submit`, `grid.addRow`…) and default bindings close to DataGrip's (Ctrl/⌘+Enter submit, Ctrl+Alt+N NULL, Ctrl+Alt+D DEFAULT, Shift+Enter value editor, F4 related rows, Ctrl+G go to row) for muscle-memory compatibility; keymaps as data files.
6. **Pending changes** keyed by row identity; parameterized DML in one transaction; affected-row checks; re-read; preview; unsubmitted-changes guard.
7. **Formats**: native CSV/TSV/JSON/Markdown/SQL INSERT/UPDATE/Pretty/HTML/XLSX extractors; user extractors, aggregators and loaders in Rhai or Lua (sandboxed, no `@Grab`-style network fetch); same binding names in English.
8. **Paging**: keyset for tables with PK, cursors for results; LOB lazy load; auto-refresh only when visible.
9. **Import/Export**: one pipeline shared by grid, explorer and MCP; native bulk paths; errors file; option names aligned with DataGrip's for familiarity.
10. **Safety**: grid edits, exports and imports pass through Karagöz authorization (table/column permissions, row limits for DELETE/export); Perde "no export" policies enforced at Karagöz.

## 6. i18n considerations

- **Display vs data formats**: number/date display uses locale (TR `1.234,56`, `11.11.2026`); copy/paste in SQL/CSV extractors defaults to invariant formats; paste parsing tries invariant first then display patterns (ALG-GRID-15).
- CSV from Turkish Excel: semicolon + decimal comma + Windows-1254 (ALG-GRID-02).
- Boolean letters (`t/f/d/n/g/c`) are English mnemonics; offer localized aliases (`e/h`) but keep English always working.
- Local filter, value completion, find: Unicode case folding with a Turkish-aware option (`I/ı/İ/i`).
- Aggregator, extractor and loader names: translate display labels, keep script IDs/file names English.
- Messages with counts use plural-aware formats (DataGrip relies on `ChoiceFormat`: "row/rows", "Export to File/Files", "{0} of {1}") — Hacivat uses Fluent plurals; Turkish has no plural agreement after numerals but still needs separate forms for "1 satır" vs zero/unknown wording.
- Keyboard shortcuts with Alt+Shift+letter may collide with Turkish-Q AltGr combinations on Windows; verify (doc 12).
- Column names, data values, CSV headers: never translated; RTL values rendered with bidi isolation in cells.

## 7. Risks, pitfalls, open questions

- Unknown: DataGrip's exact row-identity fallback, DML ordering, affected-row checks, concurrency detection → doc 13 DML family.
- Unknown: exact *Filter by* menu labels and multi-value behavior; statistics metrics; table-sampling SQL per dialect; Auto-Refresh preset values; whether import commits per batch → doc 13 FILTER/IMP families.
- Help inconsistency: *Delete Row* listed as Alt+Delete in one table vs Ctrl+Y/Delete elsewhere (plugin.xml: Ctrl+Y via Delete Line).
- Ctrl+Alt+Shift+F is both *Edit Filter Criteria* (grid) and *Full-Text Search* (explorer) — context-sensitive shortcuts must be documented in Hacivat's keymap UI.
- Wrapping user queries for filter/sort can change semantics; offset paging is slow and unstable; keyset needs unique ordering.
- Script extensions are a security surface (Groovy runs with IDE privileges and can `@Grab` from the network) — Hacivat must sandbox and forbid implicit downloads.
- Geo viewer needs an embedded browser + tile network — offline mode must degrade gracefully.
- "Disable indexes and triggers" import option can silently skip business logic — Karagöz should allow orgs to forbid it.
- Exporting "all rows" bypasses page size — must still respect Karagöz row/export policies.
