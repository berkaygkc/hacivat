# 10 — Tools: History, Local History, Diagrams, Import/Export, Native Dump/Restore, Run Scripts, Docker, Sharing (DataGrip 2024.3.5)

> Part of the Hacivat research library. Conventions: `../README.md`. Related: 07 (execution, Execute to File, Output/Services window), 08 (CSV engine, extractors/aggregators, Import/Export dialogs at grid level, paste/type inference — owned there), 09 (SQL Generator, layouts, DDL data sources, Database Changes submit flow), 11 (settings catalog), 13 (behavioral corpus families IMP, EXP, HIST, DUMP). Connectivity/drivers/introspection/credential storage: 00–05 (`.idea/dataSources*.xml` shape is in 01 §3; scope persistence in 04).

---

## 1. Scope and sources

**Scope.** Everything DataGrip offers *around* query execution and data editing for keeping, moving and sharing work:
query history; Local History (platform) for consoles/files; database-object source versions and the Database Changes window (summary — details in 09); ways to store queries (virtual views, live templates, attached files, run configurations, scratches); the Files tool window; full-text search in data; schema and plan **diagrams** (every setting, toolbar/context action, layout, graph analysis, export format); import and export entry points (dialog internals in 08); **native dump/restore tools** (pg_dump, pg_restore, psql, mysqldump, mysql, bcp) with every exposed option and validation rule; External Tools (psql/sqlplus/sqlcmd/sqlite3); **Run SQL Script** and *Database Script* run configurations; Services tool window and Docker integration; **data-source sharing** (project vs global, copy/paste XML, settings.zip); Settings Sync / Export Settings; on-disk layout of these artefacts.

**Sources.**

| Source | Used for | License |
|---|---|---|
| Offline help (DataGripHelp plugin, Oct 2024; text at `.refs/scratch-extracts/help/txt/`): `run-a-query` (Query History), `store-your-queries`, `local-history`, `database-changes-tool-window`, `full-text-search-for-databases`, `creating-diagrams`, `diagrams`, `import-data`, `import-from-excel-files`, `data-loaders`, `export-data`, `import-sql-dump-files`, `using-database-cli-tools`, `run-sql-files`, `run-debug-configuration-database-script`, `files-tool-window`, `managing-data-sources`, `sharing-your-ide-settings`, `docker`, `docker-containers`, `running-a-dbms-image`, `run_postgresql_sakila_database_in_docker_container` | documented behavior | JetBrains docs |
| Help screenshots in `help-datagrip-assets.jar` (viewed, not copied): `db_create_full_dump_with_pg_dump.png`, `db_restore_ms_sql_server_table_data.png`, `db_export_data_dialog.png`, `db_diagrams_settings.png`, `db_diagram_measurements.png`, `db_run_debug_configurations_script_files.png` | exact dialog layouts and defaults | JetBrains docs |
| `DatabaseBundle` (`database.cli.integration.*`, `dialog.message.*` validators, `progress.text.*`), `DataGridBundle` (export notification), `DiagramBundle`/`GraphBundle` (layouts, analysis), `LangBundle` | labels, validation messages, plural formats | Proprietary (short labels quoted) |
| Plugin XML (`dbplugin/META-INF/plugin.xml`, `DatabasePlugin.xml`, `dgimpl/META-INF/plugin.xml`, `DatabaseRegistryKeys.xml`) | EPs `database.cli.runTargetProvider`, `database.dataImporter`; registry `custom.cli.run.target.providers=false`; DataGrip overriding `DatabaseView.ImportDataSources` with an empty action | Proprietary |
| Class-name inventories (`classes/database-plugin.txt` etc.) | `database/cli/**` framework, placeholders, per-tool configurations/validators | names only |
| IntelliJ Community (Apache-2.0): `platform/lang-impl/.../execution/console/ConsoleHistoryController.java`, `PrefixHistoryModel.kt`, `HistoryRootType.java`, `ConsoleRootType.java`; `platform/editor-ui-api/.../UISettingsState.kt`; `platform/lvcs-impl/.../LocalHistoryImpl.kt` | console-history persistence/dedup/limit; Local History storage | Apache-2.0 |
| User's own DataGrip 2024.3 config/system dirs — **directory and file names, sizes and separator counts only** (method in doc 00 §3) | history file layout, LocalHistory files, installed plugins | n/a |
| Third-party plugin descriptor `dev.niels.SQLServerBackupAndRestore` 1.0.8 (user-installed; `META-INF/plugin.xml` only) | evidence of a feature gap (SQL Server BACKUP/RESTORE) | [Unknown] (GitHub `nvdweem/intellij-sqlserver-backup-restore`) |

---

## 2. Executive summary

1. **Query History** = the IntelliJ platform console-history model: an ordered, *move-to-end deduplicated* list of exact statement texts, capped by "Console commands history size" (platform default **300**), persisted as a plain-text file per **data source** (`consoles/.history/db/<data-source-uuid>.sql`, entries separated by a fixed separator line). Dialog: filter, preview, Paste (double-click), Delete. No timestamps, durations, outcomes or row counts. [Observed: IC sources; own config layout]
2. **Local History** (platform): automatic revisions for consoles and files, labels, search, revert file / fragment / deleted file, Create Patch; retention 5 *working* days (`localHistory.daysToKeep`, 0 disables), binary store `LocalHistory/changes.storageData` + `changes.storageRecordIndex` in the system dir, in-memory fallback, cleared on IDE upgrade. [Observed]
3. **Database Changes** window lists locally edited object sources (views, routines, triggers); Submit Ctrl+K, Roll back Ctrl+Alt+Z, Show Diff Ctrl+D, Group by, Expand/Collapse (details doc 09). [Observed]
4. **Full-text search** in data: Match case; Contains/Starts with/Ends with/Matches/LIKE pattern; column filter Text / Only FTS-indexed (PG `@@ plainto_tsquery`, MySQL/MariaDB `MATCH … AGAINST … NATURAL LANGUAGE MODE`) / Numeric / All; "Show first N matches per table/view"; per-source SQL preview; Cassandra splits OR into per-column queries. [Observed]
5. **Diagrams** (bundled `uml` plugin, yFiles renderer): categories Key columns / Columns / Virtual foreign keys / Comments (schema) and Attributes (plan); 20+ layouts; Analyze Graph (focus on selection/neighbourhood/paths/cycles, centrality ×4, auto-clustering ×3, graph characteristics); export to **yEd Live, diagrams.net, Graphviz Online (±positions), Mermaid Live, PlantUML server**; clipboard/file as **Mermaid, PlantUML, Graphviz DOT (±positions), GraphML, drawio, IDEA .uml**; PNG; print. [Observed]
6. **Import** entry points: *Import Data from File(s)* (schema → new table, table → append), drag file to explorer, *Copy Tables to…*/drag tables, *Copy to Database…* from a grid, *Import to Database…* from a CSV editor; one Import dialog with a mapping tree for file/table/query sources (details 08). [Observed]
7. **Export** entry points: *Export Data to File(s)* (explorer), *Export Data…* (grid), *Execute to File(s)* (console), *Export Table to Clipboard*, Copy; SQL Generator with 5 file layouts. Notification text uses nested ChoiceFormat plurals. [Observed]
8. **Native tools** = a generic CLI framework: argument model → form UI + **editable command line** that is re-lexed back into arguments; placeholders `{timestamp}`, `{database}`, `{data_source}` in output paths; per-tool validators; *Where to run: Locally / In a Docker container* (Docker target hidden behind registry `custom.cli.run.target.providers=false`). [Observed]
9. **pg_dump** dialog: Path, Statements (Auto / Insert / Insert with columns / Copy), Database, Schemas, Tables, Format (File / Directory / Tar archive / Custom-format archive), Clean database, Create database, Add "IF EXISTS", Data only, Out path. **pg_restore**: + Single transaction; **psql** for plain SQL. **mysqldump**: 13 checkboxes (DROP TABLE, DISABLE KEYS, LOCK TABLES, DROP TRIGGER, INSERT DELAYED, extended INSERT, complete INSERT, no CREATE, create options, routines, lock all tables, no tablespaces, no data). **bcp** export/restore per table with Target/Source file. [Observed: bundle, screenshots]
10. Validators encode tool rules: `-A` and `-B` exclusive; `--if-exists` requires `--clean`; `--clean` and `--data-only` exclusive; bcp `-c` and `-w` exclusive; directory format needs a non-existing directory for dump and an existing one for restore. [Observed: bundle]
11. **No SQL Server BACKUP/RESTORE UI** in DataGrip — a third-party plugin exists for it (installed on the studied machine). [Observed]
12. **Run SQL Script** / *Database Script* run configurations: multiple targets (data sources or schemas, "Recent" picker), Script text **or** Script files, schema-switch warning, before-launch tasks (incl. "Disconnect Data Source"), temporary configs, Store as project file; output in Run tool window; "maximum 1000 DML statements" per file (help). [Observed]
13. **Docker**: the Docker plugin is **not in the 2024.3.5 bundle** (help text claiming "bundled" is shared IntelliJ text; tutorial pages say "install"); it adds Services › Docker (images, containers, Exec, Inspect, Show Processes, Attach, Show Files, Compose). JetBrains' sample compose files carry the JDBC URL as a **comment** on the service. No container→data-source discovery. [Observed]
14. **Data-source sharing**: project (default) vs global level (*Make Global*); *Copy Data Source* → XML on clipboard **without passwords** (unless embedded in a URL-only JDBC URL); *Import from Clipboard*/*Paste Data Source*; *Duplicate*; settings.zip export covers **global** data sources only; IDEA's "Import Data Sources from project files" is **disabled in DataGrip** (action overridden). [Observed]
15. **Settings Sync** (JetBrains Account; bundled `settingsSync`) syncs categories incl. *Tools | Database* and *CSV Formats*; Settings Repository (Git) optional; manual ZIP export/import. [Observed]
16. Hacivat: structured history (SQLite + FTS5, per-statement metadata, redaction) instead of the text-list model; Local-History-like revision store for consoles; Mermaid `erDiagram`/DOT/GraphML/SVG export; argument-model CLI builder with secret-safe credential passing; dumps run **on Karagöz** for remote DBs; Docker discovery (ALG-TOOL-06) as a differentiator; portable, secret-free data-source export (ALG-TOOL-09).

---

## 3. Detailed findings

### 3.1 Query history

**UI** [Observed: help `run-a-query` §"Use the Query History dialog"; bundle `dialog.title.query.history`]
- Toolbar *Browse Query History* (Ctrl+Alt+E; action `Console.History.Browse`) on consoles. Title "{console} Query History".
- Two panes (list of entries; read-only highlighted preview editor in the console's dialect). Typing filters the list. **Double-click** (or OK button labelled with the platform *Paste* action text) puts the entry into the console editor (replacing the editor text with the chosen entry — platform behavior `setConsoleText`). **Delete** removes an entry. Dialog is non-modal; initial selection is the newest entry. [Observed: IC `ConsoleHistoryController.BrowseAction`]
- Platform console actions also merged: *Next/Previous history entry* (`Console.History.Next/Previous`) — in a multiline console these navigate only when the caret is on the last line, with **prefix filtering** (entries starting with the text before the caret). Whether DataGrip's SQL console binds them by default: [Unknown].

**Model and persistence** [Observed: IC `PrefixHistoryModel.kt`, `ConsoleHistoryController.ModelHelper`, `HistoryRootType`, `ConsoleRootType`, `UISettingsState.consoleCommandHistoryLimit = 300`]
- Entries are plain strings. *Add* = remove any existing identical string, append at the end, then drop the oldest if size ≥ limit → **exact-text move-to-end dedup**; whitespace or comment differences produce distinct entries.
- Limit = "Console commands history size" (ApplicationBundle `editbox.console.history.limit`), default 300.
- File = `<scratch root>/consoles/.history/<consoleTypeId>/<history name>.<ext>`; content = entries joined by `\n<fixed separator line>\n`; written in the IDE default charset on settings save; loaded by splitting on the separator. A statement that itself contains the separator line would split on reload (theoretical).
- **DataGrip specifics** [Observed: own config dir, names/counts only]: `consoles/.history/db/<uuid>.sql` — 18 history files whose names match 18 of 38 `consoles/db/<uuid>/` folders (which hold `console.sql`, `console_N.sql` …) → **history is per data source** (all consoles of one data source share one list); largest file holds ~250 entries (≤ 300 limit).
- Not stored: timestamps, duration, row counts, outcome, database/schema, parameters, error text. These exist only transiently in the Output log (doc 07). [Observed: format]
- What gets recorded: statements executed from consoles. Whether statements from attached SQL files, grid-generated DML, run configurations or Execute to File are recorded: [Unknown → doc 13 HIST-05..07 to add].
- Related but separate: *Filter history size* for grid WHERE/ORDER BY filter combos (`settings.filter.history.size`, doc 08/11).

### 3.2 Local History

[Observed: help `local-history`; IC `LocalHistoryImpl.kt`; own system dir names]
- Records revisions automatically on edits, external changes and IDE events (system labels such as tests/VCS); consoles are files under the scratch root and are covered (console tab › *Local History › Show History*).
- Actions: *File › Local History › Show History* (file, folder, multiple files, project), *Put Label* (user label), search by label name **or content** within revisions, *Revert* (whole file), copy fragment from diff (chevrons), *Revert Selection* for deleted files (on parent folder history), *Create Patch* from a revision (sharing).
- Retention: last **5 working days** (days on which the file changed), Advanced Settings › IDE › "Duration of storing changes in Local History" or `-DlocalHistory.daysToKeep=N`; **0 disables**. Also a maximum size; revisions not guaranteed; **cleared when installing a new IDE version**.
- Storage: `<system dir>/LocalHistory/changes.storageData` + `changes.storageRecordIndex` (binary change-list storage); if the storage cannot be opened an **in-memory** implementation is used; a background flusher writes periodically; labels carry an id, a name and (system labels) a color. Location movable via `idea.system.path`.
- Database object sources (views/routines) are tracked separately (DbSrc storage, §3.3), not by Local History.

### 3.3 Database object sources and Database Changes (summary)

[Observed: help `database-changes-tool-window`; class names `DbSrcBackend`, `DbSrcChangesTracker`, `DbSrcChangeList`, `DbSrcChangesBrowser`, `DbSrcDiffRequestProvider`, `ClearDbSrcStorageAction`]
- Appears only after an object source was edited. Toolbar: Submit (Ctrl+K), Roll back (Ctrl+Alt+Z, Manual transaction mode), Show Diff (Ctrl+D), Group by (object type), Expand All / Collapse All. "Clear Stored Sources" wipes the local source store. Full submit/merge flow → doc 09.

### 3.4 Ways to store queries

[Observed: help `store-your-queries`]
| Mechanism | What it gives | Notes |
|---|---|---|
| Virtual view | named SELECT appearing as an object in Database Explorer; open → grid | IDE-only object (doc 04/09) |
| Live template | abbreviation → statement text (Editor › Live Templates › SQL group, context SQL) | `?` placeholders become execution parameters (doc 07 Parameters dialog) |
| SQL files in attached directories | indexed, Find in Files, Go to File, Structure popup groups *DDL / DML / Select statements* | file attached to a data source via editor toolbar; dialect per file |
| *Database Script* run configuration | stored statement(s) + targets; SELECT results go to Services | can be stored as project file (§3.11) |
| Scratch files / consoles | under *Scratches and Consoles* | consoles persist per data source (§3.1) |

### 3.5 Files tool window, attached directories, scratches

[Observed: help `files-tool-window`, `import-sql-dump-files`]
- Views: Files (attached dirs), Open Files, Changed Files (VCS), Scratches and Consoles (scratch files, query consoles, extension scripts). Alt+Left/Right switches; Group Tabs toggle.
- *Attach Directory to Project* (toolbar/context) makes a directory part of the project (indexed; files runnable via run configurations); detach is the inverse. Context menu: New (also *Query Console* in Scratches view), Cut/Copy, Copy Path/Reference (Absolute, File Name, From Content Root, From Source Root), Run '<file>' (Ctrl+Shift+F10 / Alt+Shift+R per help page), Local History, External Tools.
- Options: Enable Preview Tab, Open Files/Directories with Single Click, Always Select Opened File, Excluded Files, show *Scratches and Consoles*, Sort by Name/Type/Modification time, Folders Always on Top, File Details (size/timestamp). Space = quick preview popup.

### 3.6 Full-text search in data

[Observed: help `full-text-search-for-databases`; bundle `FullTextSearch.*`, `full.text.search.first.N.queries`]
- Scope: selection of databases/schemas/tables in Database Explorer; *Tools › Full-Text Search* (Ctrl+Alt+Shift+F; `DatabaseView.FullTextSearch`). Results in the Find tool window grouped per table; double-click opens the object; tab can be pinned.
- Match modes (generated predicate per column): **Contains** `LIKE '%q%'`, **Starts with** `LIKE 'q%'`, **Ends with** `LIKE '%q'`, **Matches** `LIKE 'q'` (exact), **LIKE pattern** (user wildcards `%`/`_` passed through). **Match case** on/off (off → case-insensitive comparison; mechanism per DBMS [Inferred: `LOWER()`/`ILIKE`/collation]).
- Column filter: **Text columns** (types supporting LIKE, incl. Oracle DATE), **Only columns with full-text search indexes** (PG/MySQL/MariaDB only, uses native FTS predicate), **Numeric columns**, **All columns** (e.g. JSON).
- **Show first N matches per table/view** limit. Preview pane shows all generated statements per data source; for Cassandra (no OR) one query per column.

### 3.7 Diagrams

**Entry, content, rendering** [Observed: help `creating-diagrams`, `diagrams`; `DiagramBundle`; screenshots]
- Requires the bundled **Diagrams** (`uml`) plugin. *Diagrams › Show Diagram* (editor tab) / *Show Diagram Popup* for a data source, schema or table(s); providers `DbDiagramProvider` (schema) and `QueryPlanProvider` (explain plans, doc 07). Rendering by yFiles ("Powered by yFiles" watermark).
- Node = table/view card: header with icon + name; **non-key columns first, then a separator, then primary-key columns at the bottom**; type right-aligned ("With types aligned" style); column icons encode FK / PK / indexed / NOT NULL combinations (16 combinations in help). Edges labelled with the FK column; when the referencing and referenced column names differ the label is `child_col:parent_col`.
- Colors: object color from *Tools › Set Color* (refresh diagram); edge colors via Editor › Color Scheme › Diagrams.
- Navigation: pan (wheel/Shift+wheel), zoom, **magnifier** (hold Alt), "Select in Database Explorer", Go To group (`DbDiagrams.SourceActionsGroup.GoTo`).

**Settings › Tools › Diagrams** [Observed: screenshot `db_diagrams_settings.png`, bundle]
| Group | Item | Default (screenshot) |
|---|---|---|
| Categories enabled on open | Database Schema Diagram: Key columns, Columns, Virtual foreign keys, Comments | all on |
| | Graphical Explain Plan: Attributes | on |
| | Show Difference: Details | on |
| Scope | Default Scope | All |
| Appearance | Node item style (Plain / With types aligned) | With types aligned |
| | Shorten node items on length | 60 |
| | Show grid by default; Enable animations; Enable node items syntax highlighting | on |
| Layout | Default Layout | Hierarchic |
| | Layout on category switch (Never / With current layouter / With light layouter) | With light layouter |
| | Layout animation duration (ms) | 1500 |
| | Fit content after layout; Do relayout when new elements were added | on |

**Layouts** (context menu › Layout) [Observed: `DiagramBundle diagram.layout.*`]: Hierarchic, Hierarchic Compact, Orthogonal Groups, Compact (orthogonal), Directed (Bottom to Top), Components, Channel, Series Parallel, Circular, Edge Bundled Circular, Radial, Edge Bundled Radial, Single Cycle, Tree layouts (Compact, Balloons, Edge Bundled Balloons, Family, Generic, Wide), Organic, Edge Bundled Organic, Random, Custom/Diagram Specific. Orientation (Top→Bottom, Bottom→Top, …), Apply Current Layout, Route Edges / Auto Route Edges, Merge Edges by Sources/Targets, Align (Top/Middle/Bottom/Left/Center/Right), Distribute Horizontally/Vertically; Appearance (grid, edge labels, bridges, edge shapes); Behavior (snap to grid, structure view, fit after layout).

**Toolbar**: toggles Key columns, Columns (or plan Attributes), Virtual FKs, Comments; Zoom in/out (NumPad±), Actual size, Fit content, Apply layout, Fit to window edges, Copy diagram, Copy selection (rubber-band), Export (submenu), Refresh, Settings, Help. **Context menu extras**: Selected Element Actions, Content (New note Alt+Insert, Show Categories), Delete, Analyze Graph, Layout, Orientation, Appearance, Behavior.

**Analyze Graph** [Observed: help + `DiagramBundle dialog.diagram.*`]
- Focus On Selected Nodes / On Node Neighbourhood… / On Paths Between Two Nodes… / On Cycles… (directed or undirected; show all, containing all selected, containing any selected) / Drop Focus.
- Measure Centrality…: number of connected edges (in/out/all), graph centrality (reciprocal of eccentricity), node betweenness, closeness; option "Consider edge directions"; labels 0.00–1.00 drawn on nodes (screenshot: brightest = most important); Drop Centrality Labels; Ctrl+Z undoes.
- Automatic Clustering…: biconnected components, edge betweenness (iteratively remove highest-betweenness edge; quality/time ratio), K-Means (by positions); Drop Clustering.
- Show Graph Characteristics…: node/edge count, average degree, density, self-loops, simple, multiple edges, connected, biconnected, strongly connected, acyclic, bipartite, planar, forest, tree.

**Export** [Observed: help samples; described in our words]
| Target | Formats |
|---|---|
| Open in Web Editor | yEd Live, diagrams.net, Graphviz Online, Graphviz Online with Positions, Mermaid Live, PlantUML Web Server |
| Copy to Clipboard | Mermaid, PlantUML, Graphviz DOT, Graphviz DOT with Positions (and others listed in the submenu) |
| Export to File | yEd `.graphml`, JGraph `.drawio`, Graphviz `.dot`, `.dot` with positions, Mermaid `.md`, PlantUML, IDEA `.uml` (internal, reopen by dragging into editor) |
| Export to Image | PNG |
| Print | — |

Format rules observed in the help samples:
- **Mermaid**: `classDiagram` with `direction BT`; one `class` per table with members written `type name` (non-key columns first, PK last); relations `child --> parent : label`; names that are not valid Mermaid identifiers are replaced by synthetic ids (`nodeN`). Views appear as classes without relations.
- **PlantUML**: plain theme, top-to-bottom, orthogonal lines; members `name: type`; tables with awkward names declared with an alias plus quoted display name (which can include the table comment, e.g. `"actor_1 /* backup table */"`); relations drawn as colored plain arrows with quoted labels.
- **Graphviz DOT**: `digraph "<schema>"`, `splines=ortho`, record-shaped nodes whose label has three compartments (name | non-key columns | key columns, left-justified lines); edges with label = FK column(s); "with positions" variant adds fixed `pos="x,y!"` from the current layout.
- Column types are the introspected type text verbatim (enums/sets inline with their value lists).

### 3.8 Import (overview — dialog internals in doc 08)

[Observed: help `import-data`, `import-from-excel-files`, `data-loaders`; bundle; plugin.xml EP `database.dataImporter` with `GenericImportManager` for `UNKNOWN`]
| Entry point | Source | Target behavior |
|---|---|---|
| Explorer › Import/Export › *Import Data from File(s)…* on a **schema** | CSV/TSV/DSV or loader-backed files (multi-select) | **new table** per file |
| same on a **table** | file | **append** to existing table |
| Drag file from Files tool window onto schema/table | file | as above |
| *Copy Tables to…* / drag tables onto schema/database/table / Ctrl+Shift+A "copy tables to" | tables (any data source) | new or existing table (cross-DB copy) |
| Grid *Copy to Database…* (`Console.TableResult.ImportTable`) | result set or query | table |
| CSV editor *Import to Database…* (`FileEditor.ImportToDatabase`) | open CSV file | table |
| Explorer › SQL Scripts › *Run SQL Script…* (`DatabaseView.ImportFromSql`) | `.sql` dump | executes script (§3.11) |
| *Restore with …* (native) | dump files | §3.10 |

- Dialog anatomy: mapping tree (Add Alt+Insert, Remove Alt+Delete, Edit F4, Edit On Selection, Back/Forward Ctrl+Alt+Left/Right), settings pane for the selected node (**file source**: Path, Charset, Format + CSV format editor, First row is header, First column is header, Trim whitespaces, Column/Type list; **loader source**: Path, Loader, First row is header, columns; **table source**: Table; **query source**: Target, Query; **mapping**: Schema (any data source), Table (+ *Auto Generate*, create-new), column Mapping list (add/remove/move), Target Column ↔ Source Column, Encoding, *Write errors to file*, *Insert inconvertible values as null*, *Disable indexes and triggers, lock table (may be faster)* — only shown when the target has indexes/triggers), preview pane (Data Preview with *Modify Source*/*Reload Page*; **DDL Preview** editable). Result notification "{n} imported to {table}" with a link; error summary "N errors occurred/saved to …" (ChoiceFormat). [Observed]
- Batch size registry `database.batch.size=2000`. [Observed]
- **Scripted loaders** (Excel `.xls/.xlsx`, JSON, Parquet, Shapefile): scripts under *Scratches and Consoles › Extensions › Database Tools and SQL › data › loaders* declaring extensions/display name/table-first flag, consuming a FILE parameter and pushing columns then rows to a data consumer; third-party libraries fetched at runtime through Groovy `@Grab` (cache dir `grape/` in the system dir [Observed: dir name]). Advanced setting *Open file as table if detected by scripted loader* (All / None / Table-first formats). Excel loader reads the first sheet and evaluates formulas. [Observed: scripts, help]

### 3.9 Export (overview — extractors in doc 08, generator in doc 09)

[Observed: help `export-data`; bundle; screenshot `db_export_data_dialog.png`]
- **Structure**: *SQL Scripts › SQL Generator* (Ctrl+Alt+G) with output to clipboard / file / console and **File Output Options › Layout**: *File per object by schema*, *File per object by schema and database*, *File per object*, *File per object with order* (numbered), *File per object by schema and type*; *Generate DDL to Query Console*, *Generate DDL to Clipboard*, *Request and Copy Original DDL* (server-stored DDL); MySQL/MariaDB *Skip DEFINER clause*. *Dump to DDL Data Source* / *Dump to associated DDL Data Source* (doc 09).
- **Data**: *Import/Export › Export Data to File(s)* (explorer, one file per object); grid *Export Data…* (full data, not just the page); console *Execute to File(s)* (label is a ChoiceFormat on object count); *Export Table to Clipboard* (full data via the current extractor); Copy (selection, **current page only**; Ctrl+A then Ctrl+C = whole page).
- Export Data dialog (left: Source, Extractor, extractor-dependent options; right: **Export preview**, absent for binary extractors): *Add column* Computed / Generated (SQL extractors), *Transpose*, *Add table definition (DDL)*, *Add column header*, *Add row header*, *Output query to a separate sheet* (xlsx), *Output file* (or directory for many objects). Buttons Cancel / Copy to Clipboard / Export to File. Screenshot default: CSV extractor with all three checkboxes **off**, output path defaulting to `<home>/<table>.csv`.
- The last chosen extractor becomes the default for newly opened editors; MongoDB always defaults to JSON (Extended JSON v2).
- Progress texts "{n} of {m} bytes ({p}%) written" and completion notification built from one nested ChoiceFormat ("{rows} rows saved [from {source}] to {file|directory}") — an i18n anti-pattern, see §6. [Observed: `export.rows.saved.to.file.message`]

### 3.10 Native dump/restore tools

**Framework** [Observed: class names under `com/intellij/database/cli/**`; bundle `database.cli.integration.*`; Inferred: roles]
- Configuration per tool (`PgDumpConfiguration`, `PgRestoreConfiguration`, `PsqlConfiguration`, `MysqldumpConfiguration`, `MysqlConfiguration`, `MssqlBcpDumpConfiguration`, `MssqlBcpRestoreConfiguration`) built from **arguments** (`BooleanCliArgument`, `StringCliArgument`, `DefaultValueCliArgument`) organised in **argument groups** (e.g. `PgDumpArgumentGroup`: Database; `MysqldumpArgumentsGroup`: Source / Out Path / CheckBoxes; `PgRestoreArgumentGroup`: Path to Dump) with UI types (checkbox, combo, path field, uneditable single-parameter field).
- **Placeholders**: `DatabasePlaceholder`, `TablePlaceholder`, `GenerativePlaceholder`, `ArgumentBasedPlaceholder` with a substitutor → output path patterns `{timestamp}`, `{database}`, `{data_source}` ("Allowed substitution patterns" hint under Out path). Default out path in screenshot: `<home>/{data_source}-{timestamp}-dump.sql`.
- **Command-line editor**: the lower text area shows the generated arguments (e.g. `--file="…"`); the user may edit it; a dedicated lexer (`_DbCliLexer`, `DbCliLexerAdapter`, `DbCliTokenType`) parses it back so form and text stay in sync.
- `CliCredentialsManager` (`CliCredentials`, `ConnectionData`) supplies host/port/user/password from the data source (passing mechanism [Unknown → doc 13 DUMP-02]); messages "Enter username and password", "Password request is not allowed here".
- `DatabaseObjectsPreparer` maps the explorer selection to database/schema/table fields (read-only fields in the dialog, pre-filled by context).
- `DumpConfigurationsHolder` / `RestoreConfigurationsHolder` remember last settings (tool path etc.).
- Run via `CliRunner` on a `CliRunTarget`: *Where to run:* **Locally** or **In a Docker container** (Server, Container combos; "Loading…"; "Container is not selected"; paths suffixed "(in container)"; "Container file system is not accessible"; container file chooser) — Docker target provider registered only when registry `custom.cli.run.target.providers` ("Enables extensions for dump/restore tools (e.g. Docker)") is true.
- Progress "Dumping with {tool}… ({target})" / "Restoring with {tool}…"; completion "{tool} finished successfully" / "{tool} failed with error code {n}"; output streamed to the Run tool window.
- Actions: per tool *Export with "{tool}"…* / *Restore with "{tool}"…* plus generic *Native Export…* / *Native Restore…* (`DatabaseView.Dump.Native`, `DatabaseView.Restore.Native`) and for PostgreSQL a *Restore* dialog with **two tabs: pg_restore and psql**. Restore items missing ⇒ JDBC URL not parseable (tools need host/port/db).
- Requirements: tools installed on the same workstation; full explicit path; WSL paths accepted (e.g. `//wsl$/<distro>/usr/bin/mysqldump`).

**Option inventory** (labels [Observed]; flag mapping [Inferred: tool manuals, PostgreSQL/MySQL/Microsoft docs])

| Tool / dialog | Field | Likely flag(s) |
|---|---|---|
| pg_dump (Export with "pg_dump") | Path to pg_dump | executable |
| | Statements: Auto / Insert / Insert with columns / Copy | default (COPY) / `--inserts` / `--column-inserts` / COPY explicitly — difference between *Auto* and *Copy* [Unknown] |
| | Database / Schemas / Tables (prefilled from selection) | `-d` / `-n …` / `-t …` |
| | Format: File / Directory / Tar archive / Custom-format archive | `-Fp` / `-Fd` / `-Ft` / `-Fc` |
| | Clean database / Create database / Add "IF EXISTS" / Data only | `-c` / `-C` / `--if-exists` / `-a` |
| | Out path (patterns) | `--file=` (or `-f`) |
| pg_dumpall | mentioned as installed next to pg_dump; separate dialog/field "Databases to dump" belongs to mysqldump — pg_dumpall entry point [Unknown] | |
| pg_restore | Path, Path to dump, Database (and object scope), Single transaction, Clean database, Create database, Add "IF EXISTS", Data only | `-d`, `-1`, `-c`, `-C`, `--if-exists`, `-a`; input path positional |
| psql (plain SQL dumps) | Path, Path to dump, Database | `-f file`, `-d` |
| mysqldump | Path, Databases to dump / Tables to dump, Out path | `--databases …` / `db table …`, `--result-file=` |
| | Add DROP TABLE before CREATE TABLE | `--add-drop-table` |
| | Add DISABLE KEYS before each INSERT | `--disable-keys` |
| | Add LOCK TABLES before each table dump | `--add-locks` |
| | Add DROP TRIGGER before CREATE TRIGGER | `--add-drop-trigger` |
| | Use INSERT DELAYED (up to MySQL 5.5) | `--delayed-insert` |
| | Use single INSERT for multiple rows | `--extended-insert` |
| | Include column names in each INSERT | `--complete-insert` |
| | Export without table creation | `--no-create-info` |
| | Include all table options in CREATE TABLE | `--create-options` |
| | Include stored routines in the dump | `--routines` |
| | Lock all tables for the duration of export | `--lock-all-tables` |
| | Export schema without tablespaces | `--no-tablespaces` |
| | Export schema without data | `--no-data` |
| mysql (restore) | Path, Path to dump, Database | `< file` / `--execute="source …"` [Unknown], `-D` |
| bcp export | Path to bcp, Table (`db.schema.table`, read-only), Target file | `"<table>" out "<file>"` |
| bcp restore | Path to bcp, Table, Source file | `"<table>" in "<file>"` (screenshot shows exactly this generated text) |

**Validation rules** [Observed: bundle `dialog.message.*`]: path to executable wrong; path to file empty; *Path must point on file*; *Path must point to a non-existing directory* (pg_dump directory format); *Path must point to an existing directory* (pg_restore directory dump); `-A/--all-databases` with `-B/--databases` forbidden; `--if-exists` requires `-c/--clean`; `-c/--clean` with `-a/--data-only` forbidden; bcp `-c` with `-w` forbidden; Docker container not selected.

**Gap**: no native SQL Server `BACKUP DATABASE`/`RESTORE DATABASE`, no Oracle Data Pump, no SQLite `.backup`/`VACUUM INTO`, no MongoDB `mongodump`. A community plugin "SQL Server Backup And Restore" (Backup, Backup and Download, Restore from `.bak`; notes cloud-managed SQL Server unsupported) fills the SQL Server part. [Observed: plugin descriptor]

### 3.11 Run SQL Script and *Database Script* run configurations

[Observed: help `run-sql-files`, `run-debug-configuration-database-script`, `import-sql-dump-files`; screenshot; bundle]
- Ways to run a file: Run/Debug Configurations dialog (Run › Edit Configurations, Alt+Shift+F10 → 0) → **Database Script**; Files tool window *Run '<file>'* (temporary configuration); Explorer *SQL Scripts › Run SQL Script…* (file chooser; any file, no project attachment needed); **drag** a file onto a data source/database/schema; toolbar *Current File* run widget; Search Everywhere (Shift Shift) + Ctrl+Shift+F10.
- Configuration fields: **Name**; **Allow multiple instances** (off → starting again offers to stop the running one); **Store as project file** (default `.idea/runConfigurations`, else `.idea/workspace.xml`); **Target data source / schema** list (add, remove, move up/down, **Recent** button; dialect-dependent; a data-source target shows its default schema; "missing data source"/"<unknown>" placeholders; errors "No configured target data sources", "Missing target data sources"); **Execute: Script text | Script files** (files list with add/remove/reorder; ⚠ marker when a script contains **schema switching**); **Before launch** tasks (Run External tool, Run Another Configuration, Launch Web Browser, **Disconnect Data Source**; reorder Alt+Up/Down); *Show this page*; *Activate tool window* (default on); *Focus tool window*.
- Multiple files × multiple targets in one configuration; "Run Script Anyway?" confirmation exists for warnings. [Observed]
- Limits/messages: help states "maximum number of DML statements that you can run from an SQL file is 1000"; bundle "Using batch mode, maximum number of INSERT/UPDATE/DELETE statements is {0}" (batched execution of consecutive DML; exact semantics [Unknown → doc 13 RUNFILE-01]). Result sets from SELECTs in run configurations go to Services.
- Output: Run tool window (per-run tab; *Pause Output* suspends display only; pin tab); stop with toolbar Stop or Ctrl+F2 (choose process). Typical dump workflow: run structure file first, then data file, against the same target.
- Editor data-source attachment (2024.1+): files are attached to a **data source** (sessions managed automatically); Switch Data Source / Detach Data Source in the editor selector or Sessions view (doc 07).

### 3.12 External CLI tools

[Observed: help `using-database-cli-tools`]
- DataGrip highlights meta-commands of psql (`\d…`), sqlplus (`DESCRIBE`), sqlcmd (`:r`), sqlite3 (`.tables`) but **does not execute** them. Recommended: Settings › Tools › External Tools entry (Name, Group, Description, Program, Arguments with macros such as `$FilePath$` and quoting rules, Working directory); run from Tools › External Tools, the Files context menu, or a keymap shortcut; output in Run tool window.

### 3.13 Services tool window and Docker

[Observed: help `docker*`, `running-a-dbms-image`, `run_postgresql_sakila_database_in_docker_container`; installation plugin list]
- DataGrip 2024.3.5 bundle (34 plugin dirs) contains **no Docker plugin**; the studied machine has `clouds-docker-impl` installed in the user plugin dir. Help topic `docker` says "bundled" (shared IntelliJ text) while DataGrip tutorials say "install" — treat as **not bundled**.
- With the plugin: Settings › Tools › Build › Docker connections (Docker Desktop, Colima, Rancher Desktop dockerd; contexts import); Services (Alt+8) › Docker: pull images (type repository, Ctrl+Enter), *Create Container* (Create Docker Configuration dialog; "Randomly publish all exposed ports" via Modify options), container actions **Exec** (command history), **Inspect** (JSON), **Show Processes** (`docker top`), **Attach** (entrypoint console), **Show Files** (needs full `ls`), Build Log, hide stopped containers; Docker Compose run configurations (compose files, services list, "Recreate containers: None"); SELinux `:z` advanced option.
- Data source creation remains **manual** (host `localhost`, mapped port, credentials from env). JetBrains' sample compose files annotate each service with its JDBC URL as a YAML **comment** — a human convention, not parsed by the IDE. No discovery of DB containers. [Observed]
- `intellij.database.docker` module only contributes the CLI Docker run target (§3.10). [Observed]

### 3.14 Data-source sharing, levels and storage

[Observed: help `managing-data-sources`; bundle; `dgimpl/META-INF/plugin.xml`; doc 01 §3 for XML shapes]
- **Levels**: *project* (default; stored in `.idea/dataSources.xml` (shareable via VCS) + `.idea/dataSources.local.xml` (per machine: introspection scope, database-info, user-specific bits) — doc 01/04) and *global* (IDE config, visible in all projects). Toggle with **Make Global** in Data Sources and Drivers. Pickers group items under *Project Data Sources*, *Global Data Sources*, *DDL Data Sources*, *Imported Data Sources*, *Recent*.
- **Copy Data Source** (Ctrl+C or Copy/Paste › Copy Data Source; "Data source is copied to clipboard" / "{n} data sources are copied…") puts **XML** on the clipboard that **excludes passwords** unless the password is part of a URL-only JDBC URL. **Import from Clipboard** (New menu) / **Paste Data Source** (Ctrl+V) recreates them. **Duplicate** copies within the project. Cut/paste between folders in one project moves without re-entering the password; other paste cases require it; cut is undoable (Ctrl+Z).
- Folders: F6 *Move to Folder*; `/` in a name creates nesting (`QA/Quick`); *Root folder* removes.
- **settings.zip** (File › Manage IDE Settings › Export Settings) contains only **global** data sources; components *Database: drivers*, *Database: settings*, *Database: global data sources* appear only if non-empty; import requires restart and re-entering credentials; local file paths (SQLite) must be checked.
- Passwords live in the OS keychain (macOS/Linux) or KeePass (Windows default); tip: switch to a KeePass `.kdbx` inside the project folder and copy the folder to share **with** passwords.
- IntelliJ IDEA's *Import Data Sources…* ("Import data sources from project files", e.g. framework configs) is **overridden with an empty action in DataGrip** — not available. [Observed: `dgimpl` plugin.xml]

### 3.15 Settings Sync and settings export

[Observed: help `sharing-your-ide-settings`; bundled `settingsSync` plugin dir]
- **Settings Sync** (bundled since 2022.3): JetBrains Account login; Enable Settings Sync → choose categories (UI, Code, Tools — incl. *Tools | Database* and *Tools | CSV Formats* — System settings, Registry keys, plugins); *Push Settings to Account* / *Get Settings from Account*; per-category Configure exclusions; scope "this IDE product" or "All JetBrains IDE products"; disable locally or *Remove data from JB account and disable for all IDEs*; plugin install/enable propagation rules. Whether **global data sources** (and never passwords) are part of *Tools | Database*: [Unknown → doc 13 SET-xx].
- **Settings Repository** (Git-backed, not bundled since 2022.3; mutually exclusive with Settings Sync).
- **Manual**: Export Settings / Import Settings (ZIP, component selection, restart).

### 3.16 Action inventory (this document's scope)

| Action id | Label | Where |
|---|---|---|
| `Console.History.Browse` | Browse Query History (Ctrl+Alt+E) | console toolbar |
| `DatabaseView.FullTextSearch` | Full-Text Search… (Ctrl+Alt+Shift+F) | explorer › Tools |
| `DatabaseView.Diagrams` group | Diagrams › Show Diagram / Popup | explorer |
| `DatabaseView.ImportExport` group | Import/Export | explorer |
| `DatabaseView.Import`, `DatabaseView.ImportNew` | Import Data from File(s)… | explorer |
| `DatabaseView.ImportFromSql` | Run SQL Script… | explorer › SQL Scripts |
| `DatabaseView.Dump.Native`, `DatabaseView.Restore.Native` | Native Export… / Native Restore… | explorer |
| `Console.TableResult.ShowDumpDialogAction` / `…Grid` | Export Data… / Export Data to File(s) / Execute to File(s) | grid, explorer, console |
| `Console.TableResult.ExportToClipboard` | Export Table to Clipboard | grid |
| `Console.TableResult.ImportTable` | Copy to Database… | grid |
| `FileEditor.ImportToDatabase` | Import to Database… | CSV editor |
| `DatabaseView.DataSourceCopyAction` / `DataSourcePasteAction` / `CreateDataSourceFromSettings` / `CopyAction` | Copy Data Source / Paste Data Source / Import from Clipboard / Duplicate | explorer |
| `DatabaseView.DumpToSqlDataSource`, `DumpToConfiguredSqlDataSource` | Dump to (associated) DDL Data Source | explorer (doc 09) |
| `DatabaseView.ImportDataSources` | Import Data Sources… | **disabled in DataGrip** |
| `AttachDirectory` | Attach Directory to Project… | Files |
| `DatabaseView.RunExtensionScriptGroup` | Database Scripts | explorer (extension scripts) |

### 3.17 On-disk layout (macOS paths; same structure elsewhere)

[Observed: names only in the user's dirs; help]
| Location | Content |
|---|---|
| `<config>/consoles/db/<ds-uuid>/console*.sql` | query consoles per data source |
| `<config>/consoles/.history/db/<ds-uuid>.sql` | query history list per data source (§3.1) |
| `<config>/extensions/…` | extension scripts (extractors, aggregators, loaders — doc 08) |
| `<config>/jdbc-drivers/` | downloaded drivers (doc 02) |
| `<config>/settingsSync/` | Settings Sync state |
| `<system>/LocalHistory/changes.storageData`, `.storageRecordIndex` | Local History |
| `<system>/grape/` | Groovy `@Grab` dependency cache (loaders) |
| `<system>/data-source/` | per-data-source caches (doc 04) |
| `<project>/.idea/dataSources.xml`, `dataSources.local.xml` | project data sources (doc 01) |
| `<project>/.idea/runConfigurations/*.xml` or `workspace.xml` | run configurations |

### 3.18 Proposed additions to the behavioral corpus (doc 13)

These rows close the [Unknown]s above; IDs continue the existing families.

| ID | DBMS | Setup / action | Observe | Expected (hypothesis) |
|---|---|---|---|---|
| HIST-05 | PG | run a statement from an attached `.sql` file | appears in Query History of that data source? | yes (same data-source list) |
| HIST-06 | PG | edit a grid cell and submit | generated UPDATE in history? | no |
| HIST-07 | PG | run a *Database Script* configuration | statements in history? | no |
| HIST-08 | PG | run `SELECT 1` then `select 1` then `SELECT 1` | order and count of entries | 2 entries, `SELECT 1` newest |
| HIST-09 | PG | two consoles on the same data source, run different statements | each console's history dialog | shared list |
| HIST-10 | PG | set "Console commands history size" to 5, run 7 distinct | entries kept | last 5 |
| LH-01 | — | edit console, Local History › Show History | revisions present, labels | auto revisions |
| LH-02 | — | set retention 0, edit file | history disabled | no new revisions |
| DUMP-06 | PG | pg_dump Statements=Auto vs Copy vs Insert | generated command line | Copy adds nothing / Insert adds `--inserts` |
| DUMP-07 | PG | Format=Directory with existing out path | validation | "non-existing directory" error |
| DUMP-08 | PG | tick Add "IF EXISTS" without Clean | validation | requires `-c/--clean` |
| DUMP-09 | PG | edit command line to add `--clean` | checkbox state | Clean ticked |
| DUMP-10 | MySQL | tick all 13 checkboxes | command line | flags per §3.10 table |
| DUMP-11 | PG | Out path `{data_source}-{timestamp}` with data source in folder `A/B` | resulting file name | sanitized |
| DUMP-12 | MSSQL | bcp export of `db.dbo.t` | command line, format flags | `out` + `-c`? |
| DUMP-13 | PG | Restore dialog with a `-Fc` file in psql tab | error/behavior | psql fails on binary |
| RUNFILE-02 | PG | 2 files × 2 schema targets | execution order | target-major or file-major? |
| RUNFILE-03 | PG | file containing `SET search_path` against schema target | warning icon, confirmation | ⚠ + "Run Script Anyway?" |
| RUNFILE-04 | PG | file with 1,500 INSERTs | all executed? message | batch-mode limit message |
| RUNFILE-05 | PG | error in statement 3 of 5 | continue or stop | [Unknown] |
| FTS-01 | PG | Contains `50%` | generated predicate | `%` escaped |
| FTS-02 | MySQL | Only FTS-indexed columns | generated SQL | `MATCH … AGAINST … NATURAL LANGUAGE MODE` |
| DIAG-01 | PG | export schema to Mermaid/PlantUML/DOT/GraphML | outputs | per §3.7 rules |
| DIAG-02 | PG | table name `class`, column with space | Mermaid identifiers | synthetic `nodeN` |
| DS-01 | PG | Copy Data Source with password saved | clipboard XML | no password |
| DS-02 | PG | URL-only data source with `password=` in URL | clipboard XML | password present (leak) |
| DS-03 | — | Export Settings with only project data sources | component list | no "global data sources" item |

---

## 4. Algorithm specifications

### ALG-TOOL-01: Import pipeline
- **Problem:** Load rows from files (CSV/DSV, Excel, JSON, Parquet) or another database into a target table, creating it if needed, robustly and fast.
- **Inputs / outputs:** sources (file paths, tables, or queries), per-source format (sniffed via doc 08 ALG-GRID-02), target (existing table | new table spec), mapping, options {errors_file?, inconvertible_as_null, fast_mode, batch_size=2000, header_row, header_column, trim} → `ImportReport{rows_ok, rows_failed, errors_file, duration}`.
- **Algorithm:**
  1. **Open source** as a row stream with column names (header or `c1..cn`) and a sample (first N rows).
  2. **Infer types** for a new table (doc 08 ALG-GRID-03) → propose `CREATE TABLE` with dialect types (Integer→INTEGER/INT, BigInteger→BIGINT, Double→DOUBLE PRECISION or NUMERIC(p,s), Boolean→BOOLEAN/BIT/TINYINT(1), String→TEXT/VARCHAR(rounded max)/NVARCHAR, Date/Timestamp…); show editable DDL preview; user edits are re-parsed to update the column list.
  3. **Mapping** for an existing table: match by normalized name (case-insensitive, `_`/space-insensitive, Unicode-folded), then by position; unmapped target columns use DEFAULT; unmapped source columns ignored (warn "Columns are not mapped"); type compatibility per column (Full/Lossy/Incompatible) → warnings; changing an existing table's schema requires confirmation.
  4. **Prepare target**: create table (in a tx if DDL is transactional); if fast_mode: dialect-specific (PG: use `COPY`; `ALTER TABLE … DISABLE TRIGGER USER` only if requested; MySQL: `foreign_key_checks=0`, `unique_checks=0`, `DISABLE KEYS` for MyISAM; MSSQL: `TABLOCK`, disable non-clustered indexes); MSSQL `SET IDENTITY_INSERT ON` when an identity column is mapped.
  5. **Load**: PG `COPY … FROM STDIN (FORMAT binary|csv)`; MySQL multi-row INSERT batches (or `LOAD DATA LOCAL INFILE` if allowed); MSSQL TDS bulk load; SQLite prepared INSERT in one transaction; generic parameterized batches of `batch_size`.
  6. **Convert** each value; on failure: if inconvertible_as_null → NULL + warning; else row → errors file (original line number + raw text + error) and skip; on a DB error for a batch: bisect to isolate failing rows unless "stop on first error".
  7. **Finish**: re-enable indexes/triggers (rebuild), reset IDENTITY_INSERT, commit, optional ANALYZE, report "{n} imported to {table}" with link.
- **Data structures:** bounded channel source→converter→writer (backpressure); per-column converter closures.
- **Complexity:** O(rows) streaming; memory O(batch).
- **Edge cases:** encodings (UTF-8 BOM, UTF-16 LE/BE, Windows-1254, ISO-8859-9); CRLF/CR; quoted multi-line fields; header repeated mid-file; ragged rows; empty file; header-only file; very wide rows; NULL markers (`\N`, empty, `NULL`); Excel serial dates and the 1900 leap-year bug; Excel error cells (`#N/A`); merged cells; multiple sheets; JSON arrays of nested objects (flatten with dotted names or JSON column); Parquet nested/repeated fields; timestamps without zone; decimal commas; boolean spellings (`yes/no`, `evet/hayır`); duplicate column names; reserved words/Unicode as names (quote); target table locked; permission denied (Karagöz); cancellation (explicit choice: rollback vs keep committed batches); trigger side effects skipped in fast mode; identity gaps; target in another data source than the source.
- **Test cases:** doc 13 IMP-01..12; plus: CSV with BOM + Turkish header `şehir` → column `şehir` quoted where needed; 2,500 rows with batch 2,000 → 2 batches; row 1,001 violating UNIQUE with bisect → exactly 1 failed row logged.
- **Prior art:** [Observed: dialog options, loaders, registry]; DBeaver data transfer (Apache-2.0), pgloader (PostgreSQL license), DuckDB CSV sniffer (MIT).
- **Used by:** Hacivat (local); Karagöz (server-side bulk import, permission + audit).

### ALG-TOOL-02: Export pipeline and file naming
- **Problem:** Stream table/query data through an extractor to files or the clipboard with predictable names.
- **Inputs / outputs:** sources (tables, queries, result sets), extractor, options {column header, row header, transpose, include DDL, computed/generated columns, output file|dir, xlsx query sheet} → files + report.
- **Algorithm:**
  1. Resolve names: one source → chosen file; many → directory with `<object>.<ext>`; sanitize (`/ \ : * ? " < > |`, control chars, trailing dots/spaces, Windows reserved names `CON`, `NUL`…); on collision prefix schema (`schema.table.csv`) then add `_2`, `_3`; queries use result title (doc 07 ALG-EXEC-04) or `query_N`.
  2. For each source: server-side cursor without page limit; stream chunks to the extractor (doc 08) writing to a buffered writer; transposed export materializes (warn above N rows) or does a two-pass column-major write.
  3. Include DDL first when selected (doc 09 generator).
  4. XLSX: streaming writer; sheet per source; optional "Query" sheet; Excel limits (1,048,576 rows, 16,384 cols, 32,767 chars/cell) → split sheets or truncate with warning.
  5. Clipboard target: same pipeline into memory with a size guard (ask above e.g. 50 MB).
  6. Report rows, bytes/chars, duration, link to file/folder (message with plural-aware arguments, §6).
- **Edge cases:** overwrite confirmation; disk full; cancellation (delete partial or keep `.partial`); binary values (hex/base64 per extractor); line separators (OS vs `\n`); CSV formula injection (`= + - @` prefixes, optional escaping); BOM/encoding for Excel-CSV; time zones; two objects differing only by case on case-insensitive file systems (macOS/Windows); names with Turkish dotted/dotless i (no case folding in file names); MongoDB documents (JSON only).
- **Test cases:** export `public.Users` and `public.users` to one dir on macOS → `Users.csv` and `users_2.csv`; table named `CON` on Windows → `_CON.csv`; 0-row table → header-only file (if header on) and "0 rows saved".
- **Prior art:** [Observed: Export Data dialog fields, Execute to File].
- **Used by:** Hacivat; Karagöz (server-side export with policy check — export may be forbidden per role and must be enforced there).

### ALG-TOOL-03: Query history store, dedup and search
- **Problem:** Persist executed statements with context; search fast; avoid DataGrip's lossy text-list model while keeping its useful semantics (move-to-end, per data source, prefix recall).
- **Data model (SQLite, WAL):** `history(id, ts_start, duration_ms, data_source_id, database, schema, console_id, file_path?, origin {console|file|grid|mcp|run_config|import}, statement_text (redacted), text_hash (exact), fingerprint (normalized), params_json (masked), outcome {ok|error|cancelled}, rows_affected, rows_fetched, error_code, error_message, tx_mode, run_count, last_run_ts, pinned)`; FTS5 table on `statement_text, error_message` with `unicode61 remove_diacritics 2`; indexes on `(data_source_id, last_run_ts DESC)`, `text_hash`, `fingerprint`.
- **Algorithm:**
  1. On auditor `on_finish` (doc 07 ALG-EXEC-07) enqueue a record (async writer).
  2. **Redact** secrets before hashing/storing (patterns per dialect: `PASSWORD '…'`, `IDENTIFIED BY …`, `CREATE USER … WITH PASSWORD`, `ALTER LOGIN … WITH PASSWORD`, connection strings with `password=`); store `redacted=true`.
  3. **Dedup**: exact `text_hash` match within the same data source → update `run_count`, `last_run_ts`, latest outcome/metrics (DataGrip-compatible move-to-end); different whitespace/comments → new row but same `fingerprint` (normalize: strip comments, collapse whitespace, case-fold keywords, replace literals with `?`).
  4. **Retention**: keep pinned forever; else max rows per data source (default 10,000) and max age (default 90 days); "do not record" flag per data source and per console ("private console").
  5. **Search**: FTS query + filters (data source, outcome, origin, date range, min duration); rank by bm25 then recency; "similar" = same fingerprint.
  6. **Recall in editor**: Up/Down on last line cycles entries of the current data source whose text starts with the prefix before the caret (platform behavior), newest first.
  7. Actions: insert at caret / replace editor, run again, copy, delete, pin, export (JSON lines).
- **Complexity:** insert O(log n); search FTS O(matches); retention purge in background batches.
- **Edge cases:** huge statements (store first 64 KB + hash + compressed full text optional); multi-statement scripts (per statement + `script_run_id`); parameters with secret-like names masked; statements containing the platform separator line (irrelevant in SQLite); clock changes (monotonic ordering by id); concurrent consoles (single writer); corrupt DB (rotate and start new); import of DataGrip history files (split on separator, timestamps unknown → mark `imported`); Turkish search: `İSTANBUL` must match `istanbul` (FTS folding + Turkish-aware lowercase for the query).
- **Test cases:** run `SELECT 1` twice → one row, `run_count=2`; run `select 1` → new row, same fingerprint; `ALTER ROLE app PASSWORD 'x'` → stored `PASSWORD '***'` (doc 13 HIST-03); 301st distinct statement with limit 300 → oldest non-pinned evicted; search `musteri` finds `müşteri` (diacritics removed); prefix recall `SEL` + Up → newest entry starting with `SEL`.
- **Prior art:** [Observed: IC console history model; DataGrip per-data-source files]; pgcli history; Atuin (MIT).
- **Used by:** Hacivat; Karagöz keeps a separate **authoritative append-only audit log** — client history is never an audit trail.

### ALG-TOOL-04: Diagram model and text export
- **Problem:** Build a schema graph and export it to Mermaid/PlantUML/DOT/GraphML deterministically.
- **Inputs / outputs:** introspected objects in scope (tables, views, virtual views), FKs + virtual FKs, toggles {key columns, columns, virtual FKs, comments}, optional node positions → text in the chosen format.
- **Algorithm:**
  1. Nodes: tables/views in scope, sorted by name (stable output). Column order within a node: non-key columns in ordinal order, then PK columns in key order (matches DataGrip cards and exports); hide according to toggles; shorten item text beyond `N` chars (default 60) with "…".
  2. Edges: one per FK (composite FK → one edge); label = column list; if child and parent column names differ, label `child:parent` per pair joined by `, `; virtual FKs flagged (dashed). Direction child → parent.
  3. Identifier policy per format: Mermaid identifiers restricted to `[A-Za-z0-9_]` → otherwise synthetic `nodeN` with display label (Mermaid `class nodeN["label"]` / `erDiagram` quoted names); PlantUML alias + quoted display name; DOT quoted IDs with escaped `"`, `{`, `}`, `|`, `<`, `>` in record labels.
  4. Emit: Mermaid `classDiagram` (DataGrip-compatible) or `erDiagram` (Hacivat default; cardinality from nullability/uniqueness: nullable FK → `o|`, unique FK → one-to-one); PlantUML class or entity syntax; DOT records with three compartments; GraphML with geometry when positions exist; comments optionally appended to the display name.
  5. Positions: if a layout exists, include them in "with positions" variants.
- **Edge cases:** self-referencing FK (loop edge); multiple FKs between the same pair (keep all, distinct labels); cross-schema FK to an out-of-scope table (add a ghost node or omit with a note); tables without PK (no key compartment); names with spaces, quotes, Unicode, reserved words of the target syntax (`end`, `class`); very long enum/set types; empty scope (emit valid empty diagram); views without columns introspected; 500+ tables (warn; collapse to key columns).
- **Test cases:** Sakila subset → edge `film → language` twice with labels `language_id` and `original_language_id:language_id`; table `actor_1` with comment and Mermaid → synthetic id; `class` table name in Mermaid → aliased; DOT label escaping of `a|b` column.
- **Prior art:** [Observed: help samples and screenshots]; Mermaid, PlantUML, Graphviz docs.
- **Used by:** Hacivat (UI + MCP resource "schema as Mermaid").

### ALG-TOOL-05: Native tool invocation builder
- **Problem:** Build a safe, reproducible command for native tools from a data source, a selection and options.
- **Inputs / outputs:** tool (pg_dump, pg_dumpall, pg_restore, psql, mysqldump, mysql, bcp, sqlcmd, sqlite3, mongodump), data source (host, port, db, user, SSL, SSH tunnel), selection, option values (ALG-TOOL-07 model), run target (local | docker container | Karagöz) → `Command{program, args[], env{}, stdin?, stdout_file?, temp_files[]}`.
- **Algorithm:**
  1. Locate executable: user path → remembered path → PATH → known install dirs (`/Library/PostgreSQL/*/bin`, `/opt/homebrew/opt/libpq/bin`, `C:\Program Files\PostgreSQL\*\bin`, MySQL `bin`) → offer a Docker image matching the server major (`postgres:<major>`), because pg_dump must be ≥ server major.
  2. Map connection: if SSH-tunnelled, use the local tunnel endpoint (doc 02); SSL → `PGSSLMODE`/`PGSSLROOTCERT` or `--ssl-mode`/`--ssl-ca`.
  3. **Credentials never on argv**: PG → env `PGPASSWORD` *or* temp `.pgpass` (0600) via `PGPASSFILE`; MySQL → temp option file (0600) passed as the **first** argument `--defaults-extra-file=`; bcp/sqlcmd → `SQLCMDPASSWORD` env; delete temp files on exit (also on crash via guard); run target Docker → `docker exec -e` with value from stdin, not argv.
  4. Map selection: PG `-n`, `-t` (quoted patterns, escape `*?` in literal names), MySQL `--databases a b` or `db t1 t2`, bcp `db.schema.table`.
  5. Expand output-path placeholders (ALG-TOOL-10).
  6. Render args from the option model; if the user edited the raw command line, re-lex it (shell-like tokens, quotes, `=` forms) and sync back; unknown flags preserved verbatim.
  7. Validate (ALG-TOOL-07); run with live output, exit code check, cancellation (SIGINT → SIGTERM after 5 s → SIGKILL), redact passwords in any echoed command.
- **Edge cases:** Windows quoting (`CommandLineToArgvW` rules) and `cmd` metacharacters; WSL path translation (`//wsl$/…` ↔ `/…`); Docker path mapping "(in container)"; spaces/Unicode in paths, DB and table names; password prompt when no credentials (stdin closed → fail fast with clear message); `.pgpass` precedence over env; huge outputs to file only; restoring into a non-empty DB; version mismatch; tool locale (`LC_MESSAGES` untouched for display; parse only exit codes); Kerberos/IAM auth (no password); Unix socket hosts.
- **Test cases:** PG data source with password `p@ss w"rd` → argv has no password, `PGPASSWORD` set; `ps` during run shows no secret (doc 13 DUMP-02); MySQL with option file → file removed after exit (also after SIGKILL of the child); table `My "T"` → `-t '"My ""T"""'`-equivalent correct quoting.
- **Prior art:** [Observed: CLI framework names, dialogs, WSL support, Docker target]; libpq env vars; MySQL option files.
- **Used by:** Hacivat; **Karagöz** (preferred executor for remote databases; artifact stored server-side or streamed; audit).

### ALG-TOOL-06: Docker database discovery (Karagöz and local Hacivat)
- **Problem:** Find databases in Docker on a host and propose data sources with minimal typed credentials — absent in DataGrip.
- **Inputs / outputs:** Docker Engine API (unix socket / named pipe / Podman socket) → `Vec<DiscoveredDb{container_id, name, engine, version?, endpoints[], credentials_hint, labels, confidence}>`.
- **Algorithm:**
  1. List running containers, subscribe to events (start/stop/die/rename).
  2. Engine by precedence: (a) explicit labels (`hacivat.db.engine`, `hacivat.db.port`, `hacivat.db.name`, `hacivat.db.user`, `hacivat.db.password-secret`, `hacivat.expose=false`); (b) image heuristics (`postgres`, `postgis/postgis`, `bitnami/postgresql`, `timescale/*`, `mysql`, `mariadb`, `percona`, `mcr.microsoft.com/mssql/server`, `mongo`, `redis`, `clickhouse/clickhouse-server`, `gvenzl/oracle-free`); (c) exposed ports (5432, 3306, 1433, 27017, 6379, 8123/9000, 1521); (d) process list as tiebreaker; (e) compose metadata: `com.docker.compose.project/service` labels, and a JDBC-URL comment next to the service in the compose file if the file is readable (JetBrains sample convention) → parse host port/db/user only.
  3. Endpoints: container network IP:port (when Karagöz shares the network) and published host ports; reachability probe.
  4. Credential hints from env (`POSTGRES_USER/PASSWORD/DB`, `*_FILE` variants, `MYSQL_ROOT_PASSWORD`, `MYSQL_USER/PASSWORD/DATABASE`, `MSSQL_SA_PASSWORD`/`SA_PASSWORD`, `MONGO_INITDB_ROOT_*`) — kept **only on Karagöz**; preferably create a least-privilege role and store only that.
  5. Verify by handshake/`SELECT version()`; set confidence.
  6. Publish metadata (engine, version, name, host id, labels) to Perde — never credentials.
- **Edge cases:** rootless Docker/Podman socket paths; Docker Desktop VM networking (container IPs not routable on macOS → published ports); Swarm/Kubernetes (later); `trust` auth (warn); several DBs in one container; poolers (PgBouncer) marked as such; Docker secrets files (need exec — off by default); IP changes on restart (key by container id/name); compose comment containing a password (ignore the password part).
- **Test cases:** `postgres:16` with `POSTGRES_PASSWORD` and port 55000→5432 → engine postgres, endpoint `127.0.0.1:55000`, confidence high after probe; container labelled `hacivat.expose=false` → skipped; image `myorg/app` exposing 5432 only → engine postgres, confidence medium.
- **Prior art:** [Observed: DataGrip manual flow, CLI Docker target, compose comment convention]; Traefik/Portainer labels; Testcontainers wait strategies.
- **Used by:** Karagöz (primary), Hacivat (local), Perde (inventory display).

### ALG-TOOL-07: Native-tool option model and cross-option validation
- **Problem:** Represent tool options declaratively so UI, command line, persistence and validation derive from one table; reproduce DataGrip's validators and add missing ones.
- **Inputs / outputs:** `ToolSpec{options: [OptionSpec{id, label_key, kind: Bool|Enum(values)|Text|Path(PathKind)|Multi, flag_render, default, group, applies_to: [dump|restore]}], rules: [Rule]}` + values → `ValidationResult{errors[], warnings[]}`.
- **Algorithm:**
  1. Evaluate field rules: executable exists and is executable (`Path to executable is wrong`); required paths non-empty (`Path to file is empty`).
  2. Path-kind rules: `File` → parent dir exists, not a directory (`Path must point on file`); `NewDir` (pg_dump `-Fd`) → must not exist (`…non-existing directory`); `ExistingDir` (pg_restore of a directory dump) → must exist.
  3. Cross-option rules (declarative `Requires(a,b)`, `Excludes(a,b)`): pg `--if-exists` requires `--clean`; `--clean` excludes `--data-only`; `--create` with `--data-only` → warning (no-op); mysqldump `--all-databases` excludes `--databases`; `--no-data` with `--no-create-info` → error (empty dump); `--lock-all-tables` with `--single-transaction` → warning; bcp `-c` excludes `-w`; pg format `Plain` with `pg_restore` → error ("use psql"); Docker target requires container.
  4. Re-validate after raw command-line edits (lexed flags mapped back to ids; unknown flags only warn).
- **Data structures:** option table per tool version (flags differ, e.g. `--delayed-insert` removed after MySQL 5.5 → hide for newer clients by `--version` probe).
- **Edge cases:** short/long flag aliases (`-c`/`--clean`); `--opt=value` vs `--opt value`; repeated flags (`-n a -n b`); negated flags (`--skip-extended-insert`); defaults that equal tool defaults (omit from argv); client version older than option.
- **Test cases:** pg_dump {if_exists:true, clean:false} → error; {clean:true, data_only:true} → error; mysqldump {all_databases, databases:[a]} → error; bcp {-c,-w} → error; pg_dump format Directory with existing dir → error; raw edit adding `--clean` re-enables the checkbox.
- **Prior art:** [Observed: DataGrip validator messages and argument-group class names].
- **Used by:** Hacivat; Karagöz (same spec when executing remotely; server-side policy can forbid options like `--clean`).

### ALG-TOOL-08: Restore format detection and tool choice
- **Problem:** Given a dump artifact, choose psql / pg_restore / mysql / bcp automatically (DataGrip leaves it to the user and hides items when the URL is unparseable).
- **Inputs / outputs:** path (file or directory), target engine → `{format, tool, warnings}`.
- **Algorithm:**
  1. Directory: if it contains `toc.dat` → PG directory format → pg_restore (requires existing dir).
  2. File magic: starts with `PGDMP` → PG custom format → pg_restore; tar header (`ustar` at offset 257) containing `toc.dat` → PG tar → pg_restore; gzip magic `1F 8B` → decompress stream and re-detect (plain SQL in gzip → psql via stdin); zstd/lz4 magic → same.
  3. Text: scan the first 64 KB: `-- PostgreSQL database dump` / `SET statement_timeout` / `\connect` → PG plain → psql; `-- MySQL dump` / `/*!40101 SET` → MySQL → mysql client; `GO` lines + `USE [` → T-SQL script → sqlcmd/our runner; `PRAGMA foreign_keys` + `BEGIN TRANSACTION;` → SQLite `.dump` → sqlite3/our runner; otherwise generic SQL → Hacivat script runner (ALG-TOOL-12).
  4. Binary bcp data has no magic: choose bcp only if user picked a table target and file is not detected as text SQL.
  5. Cross-engine mismatch (MySQL dump into PG) → error with explanation.
- **Edge cases:** psql meta-commands in plain dumps (`\connect`, `\.` after `COPY … FROM stdin`) cannot be run by a generic SQL runner → require psql or implement COPY-stdin handling; dumps from newer pg_dump than local pg_restore (archive version error); `CREATE DATABASE` inside dump vs selected target DB; encoding declarations (`SET client_encoding`); BOM; truncated files.
- **Test cases:** `pg_dump -Fc` output → pg_restore; `pg_dump -Fd` dir → pg_restore; `mysqldump` output gzipped → mysql via stdin; plain PG dump with `COPY … FROM stdin;` run through Hacivat runner → handled by COPY-in, not as SQL.
- **Prior art:** [Observed: DataGrip's split pg_restore vs psql and URL-dependent availability]; PostgreSQL archive format docs.
- **Used by:** Hacivat; Karagöz.

### ALG-TOOL-09: Data-source export/import with secret stripping
- **Problem:** Share connection definitions (clipboard, file, team via Perde) without leaking secrets — DataGrip strips passwords except inside URL-only JDBC URLs, which is a leak path.
- **Inputs / outputs:** data sources (incl. SSH/SSL/proxy settings, driver properties, init SQL, folder, color, scope) → portable document `{format_version, sources[]}` (TOML/JSON) and back.
- **Algorithm (export):**
  1. Serialize non-secret fields; replace secrets by references: `password`, SSH password/passphrase, SSL key passphrase, proxy password, cloud tokens → omitted with `requires_secret=["password", …]`.
  2. **Scrub URLs and properties**: parse connection URLs (`postgres://user:pass@`, JDBC `password=`/`pwd=` query params, ODBC-style `Password=`), driver properties whose key matches `(?i)pass|pwd|secret|token|key` → remove values; keep keys with empty value marker.
  3. Absolute local paths (SQLite file, SSL certs, SSH keys) → mark `machine_local=true`; optionally relativize to project.
  4. Add stable `id` (UUID) and `origin` (machine, app version) for conflict handling.
- **Algorithm (import):** validate version; for each source: if `id` exists → ask Replace / Keep both (rename "name (2)") / Skip; for name collision without id → keep both; prompt for required secrets lazily on first connect; never auto-connect after import; mark untrusted init SQL / driver properties for review (could execute code on connect).
- **Edge cases:** passwords embedded in host field (`user:pw@host`); env-var references (`${PGPASSWORD}`) kept as references; Perde-managed sources (export only a pointer, not endpoint details); DDL data sources (paths); folders with `/` nesting; Unicode names; clipboard text that is a DataGrip XML (optional importer, names/URL/driver only).
- **Test cases:** URL `jdbc:postgresql://h/db?user=u&password=p` → exported without `password`; `postgres://u:p@h/db` → `postgres://u@h/db`; property `sslpassword=x` removed; re-import over existing id → prompt; init SQL `COPY … TO PROGRAM` flagged for review.
- **Prior art:** [Observed: DataGrip Copy Data Source semantics, settings.zip global-only rule].
- **Used by:** Hacivat (share/backup), Perde (team templates without secrets; Karagöz holds real credentials).

### ALG-TOOL-10: Output-path placeholder substitution
- **Problem:** Expand `{timestamp}`, `{database}`, `{data_source}` (DataGrip) plus Hacivat extras (`{schema}`, `{table}`, `{date}`, `{engine}`) safely.
- **Algorithm:** tokenize on `{name}` (allow `{{`/`}}` escapes); unknown names → validation error; values: timestamp `YYYY-MM-DD_HH-MM-SS` local time (no `:` for Windows); names sanitized for file systems (ALG-TOOL-02 rules) but not case-folded; join; if the result exists and the tool would overwrite → confirm; multi-object runs expand per object.
- **Edge cases:** placeholder in directory part creating new folders; empty `{database}` for server-level dumps → `server`; data-source names with `/` (folders) → `_`; Turkish characters preserved; very long names truncated with hash suffix to stay under 255 bytes (UTF-8 byte length, not chars).
- **Test cases:** `~/{data_source}-{timestamp}-dump.sql` with data source `Prod/EU` → `~/Prod_EU-2026-11-11_10-00-00-dump.sql`; `{unknown}` → error; `{{literal}}` → `{literal}`.
- **Prior art:** [Observed: pg_dump dialog hint and default path].
- **Used by:** Hacivat, Karagöz.

### ALG-TOOL-11: Diagram layout input and graph analysis
- **Problem:** Provide layout and analysis comparable to DataGrip's yFiles-based features with permissive libraries.
- **Inputs / outputs:** graph from ALG-TOOL-04 with node sizes (from rendered card height = header + items × line height) → positions, edge routes; analysis results (centrality per node, clusters, cycles, characteristics).
- **Algorithm:**
  1. Default layered (Sugiyama) layout with FK direction child→parent drawn bottom-to-top (DataGrip default "Hierarchic"); orthogonal edge routing; disconnected components packed in a grid sorted by size.
  2. Incremental relayout when nodes are added (keep existing positions as constraints; "light layouter" = only place new nodes).
  3. Analysis: degree (in/out/all), closeness = 1/Σd, graph centrality = 1/max d (eccentricity), betweenness (Brandes, O(VE)); normalize to 0..1 for labels; cycles via Tarjan SCC (directed) and cycle basis (undirected); clustering via biconnected components (Hopcroft–Tarjan) and Girvan–Newman edge betweenness (cap iterations); characteristics (density = E / (V(V−1)), acyclic, bipartite via BFS 2-coloring, planar via a library or skip > 200 nodes).
  4. Focus: paths between two nodes = union of simple paths up to length k (BFS both directions); neighbourhood = k-hop ball.
- **Edge cases:** self-loops (excluded from SCC size>1 tests but counted in characteristics); multi-edges; isolated nodes (closeness undefined → 0); disconnected graphs (centrality per component); 1,000+ nodes (async, progress, cancel); deterministic output (seeded).
- **Test cases:** chain a→b→c: betweenness(b)=1 (normalized), others 0; cycle a→b→a: SCC size 2 → reported; star graph: center graph centrality 1.0.
- **Prior art:** [Observed: DataGrip layout and analysis menus]; dagre (MIT), ELK (EPL — reference only), petgraph (MIT/Apache-2.0) for algorithms.
- **Used by:** Hacivat.

### ALG-TOOL-12: Run script files against multiple targets
- **Problem:** Execute one or more files (or inline text) against N targets with predictable ordering, errors and warnings (DataGrip *Database Script* configuration).
- **Inputs / outputs:** files[] | text, targets[] (data source + optional database/schema), options {stop_on_error (default true), parallel_targets (default false), confirm_schema_switch} → per-target report {statements, ok, failed, duration, first_error}.
- **Algorithm:**
  1. Pre-scan each file with the splitter (doc 06 ALG-SQL-01) streaming (files can exceed memory); detect schema/database switches (`USE`, `SET search_path`, `\connect`, `ALTER SESSION SET CURRENT_SCHEMA`) → warning icon; if any target is a schema and the file switches schema → require confirmation ("Run Script Anyway?").
  2. Run unsafe-query validators (doc 07 ALG-EXEC-05) per statement; collect warnings once per file.
  3. For each target (sequential by default): open a dedicated session, set the target schema, execute statements in order; batch consecutive INSERT/UPDATE/DELETE into driver batches (cap configurable — DataGrip's 1,000 limit is a mystery, Hacivat has no hard cap); on error: stop (default) or continue; record per-statement status.
  4. Stream output to a Run panel per target; SELECT results to result tabs up to the result-set limit (doc 07).
  5. Final summary per target and overall; non-zero "exit" status if any failed (for CLI/CI use).
- **Edge cases:** target data source missing/deleted ("missing data source"); duplicate targets; files with mixed dialects; client-side commands (DELIMITER, GO n) handled by the splitter; transactions spanning files; file encodings/BOM; extremely long single statements; cancellation mid-statement (cancel token) leaves earlier commits; Docker/SSH tunnels per target; "Disconnect data source before launch" semantics (close pooled sessions first).
- **Test cases:** 2 files × 2 schemas → 4 runs in order f1@s1, f2@s1, f1@s2, f2@s2 (target-major) — verify DataGrip order (doc 13 RUNFILE-02); file with `USE other;` against schema target → warning + confirmation; error in statement 3 of 5 with stop_on_error → 2 ok, 1 failed, 2 skipped.
- **Prior art:** [Observed: run configuration fields, warnings, limits].
- **Used by:** Hacivat (UI + `hacivat run` CLI); Karagöz (scheduled scripts later).

### ALG-TOOL-13: Local revision store for consoles and scripts
- **Problem:** Never lose console/script text (DataGrip relies on Local History with 5-working-day retention, cleared on upgrade).
- **Data model:** SQLite `revisions(file_id, ts, kind {auto|save|run|label|external}, label?, content_zstd, content_hash)`; dedup by hash; delta compression optional.
- **Algorithm:** snapshot on: save, every N seconds of idle after edits, before each execution (kind=run, links to history id), external file change, user label; retention: keep all labels and run snapshots for 30 days, thin auto snapshots (keep 1 per hour after 1 day, 1 per day after 7 days); restore whole file or a hunk via diff; search contents (FTS).
- **Edge cases:** large files (> 5 MB → store diffs only); deleted files (keep tombstone with last content); renamed files (track by id); clock skew; disk full (degrade, warn once); encrypted store optional.
- **Test cases:** edit + run + edit → restore run snapshot; delete console → recover from tombstone; app upgrade keeps store (versioned schema migration).
- **Prior art:** [Observed: IC Local History behavior and storage].
- **Used by:** Hacivat.

### ALG-TOOL-14: Full-text data search query generation
- **Problem:** Search a value across many tables/columns efficiently and safely.
- **Inputs / outputs:** scope (databases/schemas/tables), query string, mode {contains, starts, ends, exact, like_pattern}, match_case, column filter {text, fts_indexed, numeric, all}, limit per table → per-table SQL + results.
- **Algorithm:**
  1. Enumerate columns from the cached model (doc 04) by filter: text types per dialect; numeric (only if the query parses as a number, compare with `=`); FTS-indexed (PG `tsvector`/GIN expression indexes, MySQL FULLTEXT); all (cast to text: PG `col::text`, MySQL `CAST(col AS CHAR)`, MSSQL `CAST(col AS NVARCHAR(MAX))`).
  2. Build the predicate with **bound parameters**; escape `%`, `_` and the escape char in non-pattern modes (`ESCAPE '\'`); case-insensitive: PG `ILIKE`, MySQL default collation (or `COLLATE utf8mb4_0900_ai_ci`), MSSQL depends on collation (`COLLATE … _CI_`), SQLite `LIKE` ASCII-only → use `lower()` with ICU or client-side filter; Oracle `UPPER()` both sides.
  3. One `SELECT … WHERE c1 … OR c2 …` per table with `LIMIT N` (or `TOP`/`FETCH FIRST`); engines without OR (Cassandra) → one query per column.
  4. Run with bounded concurrency per data source; stream results; show preview SQL before running.
- **Edge cases:** huge tables (warn: sequential scans; allow per-table timeout); views with expensive definitions (opt-out); binary columns (skip); JSON (`::text`); Turkish case-insensitive search (`İ`/`ı`) where DB collation is not Turkish → offer client-side Unicode Turkish folding on fetched candidates; numeric query against text columns.
- **Test cases:** query `50%` in contains mode → predicate escapes `%`; Turkish `istanbul` with match_case off in PG `ILIKE` under `tr_TR` collation → matches `İSTANBUL`; MySQL FTS-only → `MATCH … AGAINST (? IN NATURAL LANGUAGE MODE)`.
- **Prior art:** [Observed: DataGrip options and generated predicates].
- **Used by:** Hacivat; Karagöz must apply row/column permissions to the same queries.

### ALG-TOOL-15: Settings export/import bundle
- **Problem:** Move Hacivat configuration between machines (DataGrip: settings.zip with component selection, restart required, global data sources only; or cloud Settings Sync) — offline-first, no account needed, no secrets.
- **Inputs / outputs:** selected components {ui, keymap, editor, formatter, csv_formats, extractors, data_sources (via ALG-TOOL-09), drivers, run_configs, templates} → `hacivat-settings-<date>.zip` containing `manifest.json {format_version, app_version, components[{id, schema_version, sha256}]}` + one file per component; import → preview + merge.
- **Algorithm:**
  1. Export: serialize each component with its own schema version; data sources through ALG-TOOL-09 (secret-free); compute hashes; write manifest last.
  2. Import: verify manifest and hashes; show component list with *new/changed/identical* status; per component choose Replace / Merge / Skip (merge keyed by stable ids: keymap actions, template names, data-source UUIDs); run schema migrations for older versions; apply **without restart** (hot reload of settings stores); write a backup of the replaced settings first.
  3. Secrets: never exported; on import list data sources that need credentials.
- **Edge cases:** newer format than the app (refuse with message); partial archives; unknown component ids (skip with warning); conflicting keybindings (report); OS-specific values (font names, paths) flagged machine-local; Turkish names in archive entries (UTF-8 flag in ZIP); zip-slip paths (`../`) rejected.
- **Test cases:** export → import on a clean profile reproduces settings byte-for-byte (except machine-local); archive with `../evil` entry → rejected; v1 archive into v2 app → migrated; data-source password never present in any entry (grep test).
- **Prior art:** [Observed: DataGrip Export/Import Settings and component names; Settings Sync categories].
- **Used by:** Hacivat; Perde (later: optional sync for logged-in users, still secret-free).

---

## 5. Implications for Hacivat / Karagöz / Perde

1. **History**: structured SQLite + FTS5 store (ALG-TOOL-03) fed by the execution auditor for consoles, files, grid DML, run configs and MCP calls; redaction; per-data-source and private-console opt-out; optional importer for DataGrip `consoles/.history/db/*.sql` files.
2. **Revisions**: ALG-TOOL-13 replaces Local History for consoles/scripts; survives upgrades.
3. **Import/Export**: streaming pipelines with native fast paths (PG `COPY` binary, MySQL batched INSERT, MSSQL bulk, SQLite single tx); compiled-in loaders (calamine (MIT) for xls/xlsx, arrow-rs/parquet (Apache-2.0), serde_json streaming) — no runtime `@Grab`-style downloads.
4. **Dump/restore**: declarative option specs (ALG-TOOL-07) + secure invocation (ALG-TOOL-05) + auto format detection (ALG-TOOL-08); add what DataGrip lacks: SQL Server `BACKUP/RESTORE` (server-side paths), SQLite `VACUUM INTO`, mongodump/mongorestore; when the DB is behind Karagöz, execute **on Karagöz** and stream/store the artifact.
5. **Run scripts**: Database-Script-like configurations with multiple targets and a CLI (`hacivat run --target … file.sql`) for CI (ALG-TOOL-12).
6. **Diagrams**: Mermaid `erDiagram` + DataGrip-compatible `classDiagram`, PlantUML, DOT, GraphML, SVG/PNG; MCP resource for agents; layered layout default.
7. **Sharing**: portable secret-free export format (ALG-TOOL-09); team sharing through Perde templates (Karagöz holds credentials); no "share with passwords via KeePass file" pattern.
8. **Docker discovery** (ALG-TOOL-06) shared crate for Karagöz and local Hacivat; honour compose labels and JDBC-URL comments.
9. **Settings**: export/import as a single versioned file; no cloud sync in v1 (offline-first); later optional sync via Perde for logged-in users, never including secrets.

## 6. i18n considerations

- DataGrip messages pack plurals and optional fragments into nested ChoiceFormat strings (e.g. the "rows saved to file" notification combines source presence, row plural and file-vs-directory in one pattern) — hard to translate and impossible to reorder per language. Hacivat uses Fluent messages with **named** arguments and selectors per variable (`$rows`, `$hasSource`, `$targetKind`), one message per sentence shape.
- Tool output (pg_dump, mysqldump) is shown verbatim in the tool's locale; Hacivat's own status lines ("{tool} finished successfully", "failed with error code {n}") are translated; never parse localized tool text.
- Placeholder names in paths (`{timestamp}`) stay English (they are syntax); timestamps in file names use a locale-independent format.
- Diagram exports keep identifiers verbatim; UI labels of categories/layouts are translated; Mermaid/PlantUML keywords are not.
- History search folds Turkish `İ/ı` correctly (ICU4X case mapping with `tr` locale when the UI locale is Turkish, plus diacritic removal for "musteri" → "müşteri").
- CSV/Excel export presets: "Excel (Turkey)" = UTF-8 BOM, `;` separator, decimal comma — explicit, never implicit from OS locale.
- Import sniffing must detect Windows-1254/ISO-8859-9 and show the detected encoding.

## 7. Risks, pitfalls, open questions

- Native tools bring version-compatibility and credential-leak risks; how DataGrip passes passwords to pg_dump/mysqldump is unverified (doc 13 DUMP-02).
- Meaning of pg_dump *Statements: Auto* vs *Copy*, the pg_dumpall entry point, and the exact mysql restore invocation — [Unknown]; verify with DUMP-01/03/04.
- "Maximum 1000 DML statements per SQL file" vs "batch mode maximum {0}" — semantics unclear (RUNFILE-01).
- Whether Query History records statements from files, run configurations, grid DML — [Unknown] (add HIST-05..07); history is text-only and silently deduplicated, so "how many times / when" is lost.
- Settings Sync coverage of global data sources — [Unknown].
- Docker plugin bundling: help says bundled, installation shows not bundled — treat installation as truth for 2024.3.5.
- Import "fast mode" can leave triggers/indexes disabled if interrupted — Hacivat must make it transactional or resumable with recovery guidance.
- `@Grab`-style dynamic dependency download is a supply-chain risk — Hacivat ships loaders compiled in.
- Secret redaction in history is heuristic — provide private consoles and per-data-source opt-out; Karagöz audit stores hashes plus masked text.
- Diagram rendering in DataGrip relies on a commercial layout library (yFiles); open-source layouts (dagre/ELK-like) may look different on large schemas — budget time for routing quality.
