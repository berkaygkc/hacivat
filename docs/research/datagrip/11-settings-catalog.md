# 11 — DataGrip 2024.3.5 Settings Catalog (database, SQL, grid, editor)

> Part of the Hacivat research library. Read `../README.md` first for conventions (evidence tags, clean-room rules, templates).
> Sibling documents: 06 (SQL language & editor), 07 (query execution UX), 08 (data grid), 09 (DDL generation & schema compare), 10 (tools, history, import/export), 12 (localization), 13 (behavioral test corpus). Documents 00–05 (connectivity, sessions, introspection, NoSQL) are written by another agent and are referenced, not duplicated.

---

## 1. Scope and sources

**Goal.** Enumerate every user-visible setting that DataGrip 2024.3.5 exposes for databases, SQL editing, query execution, the data grid, CSV handling and SQL code style; explain what each setting means; state its default when it can be established; and turn the result into a settings model for Hacivat.

**Primary source: searchable-options indexes.** IntelliJ-based IDEs ship, inside each plugin JAR, a pre-built index of every searchable label on every settings page (`*-searchableOptions.json`). Each line is a JSON object `{id, name, entries:[{hit, path}]}`, where `hit` is either raw text or a *bundle reference* of the form `|b|messages.<Bundle>|k|<key>|`. Resolving those references against the shipped English message bundles yields the exact label of every option. This is the most complete inventory available without decompiling. [Observed: format of `dbplugin/p-com.intellij.database-searchableOptions.json`]

| Source file (inside the app) | Pages it covers | Entries |
|---|---|---|
| `plugins/DatabaseTools/lib/database-plugin.jar` → `p-com.intellij.database-searchableOptions.json` | Keymap (DB actions), Breadcrumbs, Sticky Lines, Smart Keys › SQL, Table Diff colors, Database colors, SQL colors, Debugger › Stepping, Query Execution, Output and Results, User Parameters, Data Editor and Viewer, Other, Code Style › SQL (General + 11 dialect pages), SQL Dialects, SQL Resolution Scopes | 2,200+ |
| `database-plugin.jar` → `m-intellij.database.flameGraphs-searchableOptions.json` | Keymap only (flame-graph actions) | 3 |
| `plugins/grid-impl/lib/grid-impl.jar` → `p-intellij.grid.impl-searchableOptions.json` | Keymap (grid actions), Data Editor colors, CSV Formats, Data Editor and Viewer › Appearance | 150 |
| `plugins/datagrip-impl/lib/datagrip-impl.jar` → `p-com.intellij.database.ide-searchableOptions.json` | Keymap only (project/file actions) | 11 |
| `lib/product.jar` → `p-com.intellij-searchableOptions.json` | Platform pages; the DB-relevant ones are Code Completion › SQL, Editor Tabs › Database, Code Folding › SQL, Advanced Settings › Database, Passwords › Database | ~1,160 lines resolved |

**Secondary sources.**
- English message bundles extracted from the app: `DatabaseBundle.properties` (2,942 keys), `SqlBundle.properties` (785), `DataGridBundle.properties` (393), `GridCsvBundle.properties`, plus platform bundles (`IdeBundle`, `ApplicationBundle`, `CodeInsightBundle`, `LangBundle`, `OptionsBundle`, `KeyMapBundle`, `ActionsBundle`). [Observed]
- Plugin descriptors: `dbplugin/META-INF/DatabaseRegistryKeys.xml` (registry keys and one advanced setting), `dbplugin/META-INF/plugin.xml`, `gridimpl/META-INF/plugin.xml` (grid registry keys, advanced settings), `SqlPlugin.xml` (`predefinedCodeStyle`, `projectConfigurable`). [Observed]
- The **bundled offline help** (plugin `DataGripHelp`, `help-datagrip-assets.jar`, 602 non-redirect HTML topics, "Last modified" dates October 2024), which is the official documentation for this exact release. Topics used: `settings-tools-database*.html`, `query-execution.html`, `output-and-results.html`, `settings-smart-keys-sql.html`, `settings-code-completion.html`, `settings-languages-sql-dialects.html`, `settings-languages-sql-resolution-scopes.html`, `settings-code-style-sql.html`, `code-style-sql-*.html`, `language-and-region.html`. [Observed: offline help]
- IntelliJ Community (Apache-2.0) knowledge of `Configurable`, `SearchableConfigurable`, `PersistentStateComponent`, `CodeStyleSettings`/`CustomCodeStyleSettings`, Registry and Advanced Settings mechanisms. [Inferred: platform architecture]

**Licenses.** DataGrip: proprietary (JetBrains EULA) — only labels, keys and documented behavior are described; no code. IntelliJ Community: Apache-2.0. Help content: JetBrains documentation (quoted only as short labels).

**What is *not* available.** Defaults are mostly stored in Java/Kotlin classes (`*Settings` state objects), which we may not inspect. Defaults below come from (a) the help text, (b) labels that encode a default (e.g. "Default: `.`"), (c) registry/advanced-setting declarations (which carry explicit `defaultValue`), or (d) are marked **[Unknown]** and turned into test cases in document 13 (section "SET").

---

## 2. Executive summary

1. DataGrip's database settings live under **Settings › Database** with eight pages: Query Execution, Output and Results, User Parameters, Data Editor and Viewer, CSV Formats, SQL Dialects, SQL Resolution Scopes, Other. [Observed: help `settings-tools-database`]
2. SQL editing settings are spread across platform pages: **Code Completion › SQL** (17 options incl. JOIN, alias and qualification policy), **Smart Keys › SQL** (3), **Code Style › SQL** (≈160 options per dialect in 8 tabs), **Color Scheme › SQL / Database / Data Editor and Viewer / Table Diff**, **Code Folding › SQL**, **Breadcrumbs / Sticky Lines › SQL**. [Observed]
3. Code style is **hierarchical**: a "General" SQL style plus 11 dialect-family pages, each with an "Inherit general SQL style" switch and "Disable formatting". Only a handful of options are dialect-unique (PostgreSQL `$$` wrapping ×4, MS SQL "Align line comments"). [Observed: set difference of the resolved pages]
4. The SQL dialect is resolved by **path → project → global** inheritance, with Generic SQL as the fallback; consoles take the dialect of their data source. [Observed: help `settings-languages-sql-dialects`]
5. Name resolution context is set by **SQL Resolution Scopes**: project mapping + per-path mapping to data source/database/schema sets. [Observed]
6. Query execution policy is **per Execute action** (three Execute actions, each with its own shortcut and caret-inside / caret-outside / selection policy). [Observed: labels `settings.shortcut`, actions `Console.Jdbc.Execute`, `.Execute.2`, `.Execute.3`]
7. User-parameter detection is a **regex table** with per-pattern scope (in scripts / in literals / languages) plus three global switches; eight default patterns are documented. [Observed: help `settings-tools-database-user-parameters`]
8. The grid has 4 option groups (Limitations, Data Presentation, Sorting, Modification) plus Custom Number Formats and Custom Date/Time Formats, URL click settings and quick-actions toolbar options. [Observed]
9. CSV formats are **named, ordered, user-editable profiles** (defaults: CSV, TSV, Pipe-separated, Semicolon-separated) with value/row separators, null text, prefix/suffix, multiple quotation patterns with escape method, quote policy, trim, header row/column. [Observed: `GridCsvBundle`, `DataGridBundle`]
10. Advanced Settings › Database holds 4 switches (Related Rows behavior, open file as table, Search Everywhere database tab, goto-class). Registry holds ≈50 low-level knobs with explicit defaults (result-set limit 100, batch size 2,000, paging via LIMIT/OFFSET, CSV preview 10,000 chars, INSERT matrix-align limit 1,000 rows, grid column limit 2,000). [Observed: XML declarations]
11. The searchable index stores **bundle keys, not strings**, so search works in every UI language without rebuilding the index — a pattern Hacivat should copy. [Observed + Inferred]
12. Hacivat recommendation: one typed settings schema (Rust structs + serde) with **scopes** (default → global → project → path → data source → console), a generated searchable index keyed by i18n message IDs, and per-dialect code-style overlays with explicit "inherit" flags.

---

## 3. Detailed findings — the catalog

Conventions in the tables:
- **Label** is the exact English UI label (mnemonic `&` removed).
- **Default**: value if known; `[Unknown]` otherwise (see doc 13, case family `SET`).
- Evidence: SO = searchable-options index, H = offline help, B = message bundle, X = plugin XML.

### 3.1 Settings tree (where everything lives)

```
Settings
├─ Appearance & Behavior
│  ├─ System Settings › Language and Region      (UI language: English + bundled ja/ko/zh packs) → doc 12
│  └─ System Settings › Passwords                 ("Database:" credential storage hint)
├─ Keymap                                          (≈320 DB/grid actions, see 3.17)
├─ Editor
│  ├─ General
│  │  ├─ Code Completion › SQL                    (3.10)
│  │  ├─ Smart Keys › SQL                          (3.11)
│  │  ├─ Breadcrumbs › SQL (show for SQL)          (3.15)
│  │  ├─ Sticky Lines › SQL                        (3.15)
│  │  ├─ Code Folding › SQL                        (3.15)
│  │  └─ Editor Tabs › Database                    (3.15)
│  ├─ Color Scheme › SQL | Database | Data Editor and Viewer | Table Diff   (3.14)
│  ├─ Code Style › SQL › General + 11 dialect pages                          (3.12)
│  ├─ Inspections › SQL / dialect groups         → doc 06
│  ├─ Inlay Hints › SQL                           → doc 06
│  ├─ Live Templates › SQL, Postfix Completion › SQL → doc 06
│  └─ Language Injections                         → doc 06
├─ Build, Execution, Deployment › Debugger › Stepping › Oracle            (3.16)
├─ Database   (DataGrip puts this top-level; in other IDEs it is under Tools)
│  ├─ Query Execution                              (3.2)
│  │  ├─ Output and Results                        (3.3)
│  │  └─ User Parameters                           (3.4)
│  ├─ Data Editor and Viewer                       (3.5)
│  │  └─ CSV Formats                               (3.6)
│  ├─ SQL Dialects                                 (3.7)
│  ├─ SQL Resolution Scopes                        (3.8)
│  └─ Other                                        (3.9)
├─ Advanced Settings › Database                    (3.13)
└─ (Help › Find Action › Registry…)                (3.13, hidden)
```
[Observed: SO page ids `database.query.execution`, `database.output.results`, `database.sql.templating`, `database.data.views`, `database.data.csv.formats`, `database.other`, `SQL Dialects`, `SQL Resolution Scopes`; help `settings-tools-database` lists the eight Database pages in this order]

Note the page id `database.sql.templating` for **User Parameters** — internally the feature is called "SQL templating" (parameters are template holes). [Observed: SO id] Data Editor *appearance* options are also indexed under `database.data.views.appearance` with a bundle string "Appearance settings were moved to: Appearance & Behaviour | Data Editor and Viewer" — i.e. font/row-color options were relocated between releases. [Observed: B `settings.settings.were.moved`, `settings.data.views.appearance`]

### 3.2 Database › Query Execution (`database.query.execution`)

The page is **repeated per Execute action**. There are three execute actions — *Execute* (`Console.Jdbc.Execute`, Ctrl/Cmd+Enter), *Execute (2)* and *Execute (3)* — and the page shows a "Shortcut:" field for each so the user can bind different policies to different keys (e.g. Ctrl+Enter = smallest statement, Ctrl+Shift+Enter = whole script). [Observed: SO entries "Ctrl+Enter", "Shortcut:"; action ids in keymap index; B `settings.shortcut`]

| # | Label | Values | Meaning | Default | Ev. |
|---|---|---|---|---|---|
| QE-1 | When caret inside statement execute | Ask what to execute · Smallest statement · Smallest subquery or statement · Largest statement · Largest statement or batch · Whole script · Everything from caret | Which text range is run when the caret is inside a statement. "Ask" opens a chooser popup listing candidate ranges. "Largest statement or batch" = current T-SQL batch (between `GO`s) for SQL Server/Sybase, otherwise same as Largest statement. | *Ask what to execute* for the main Execute action [Observed: settings screenshot in bundled help, see doc 07] (corrected after verification) | SO, H `query-execution` |
| QE-2 | When caret outside statement execute | Nothing · Whole script · Everything below caret | Caret on blank line or comment. | [Unknown] (likely *Nothing*) | SO, H |
| QE-3 | For selection execute | Exactly as one statement · Exactly as separate statements · Smart expand to script | "One statement" sends the whole selection as a single command (lets a driver run BEGIN…COMMIT atomically); "separate" splits the selection; "Smart expand" grows the selection to statement borders if it crosses at least one border, else runs it verbatim. | [Unknown] | SO, H |
| QE-4 | Open results in new tab | checkbox | New result tab per executed SELECT vs reuse one tab (pinnable). | [Unknown] — conflict: bundled help screenshot shows it ticked, help prose says the same tab is reused by default; resolve via corpus case |  SO, H |
| QE-5 | Split a script for execution in Generic and ANSI SQL dialects | Into valid ANSI SQL statements or by separator · Into ANSI SQL statements · By statement separator | Only for SQL:2016 / Generic dialect files. First = parse; fall back to separator (`;`). | **Into valid ANSI SQL statements or by separator** ("This setting is default") | H |
| QE-6 | Review parameters before execution | checkbox | Show the parameter dialog pre-filled with last values before running a statement with parameters; off = run immediately with remembered values. | [Unknown] | SO, H |
| QE-7 | Show warning before running potentially unsafe queries | checkbox | DELETE/UPDATE without WHERE etc. prompts a confirmation ("Execute" button in warning). | [Unknown] (on [Inferred]) | SO, H |
| QE-8 | Track databases/schemas creation and deletion | checkbox | Present in bundle, absent from the 2024.3 index — likely removed or moved. | n/a | B only |

Cross-reference: algorithms behind QE-1..QE-3 are specified in doc 07 (ALG-EXEC-01/02) on top of the splitter in doc 06 (ALG-SQL-01).

### 3.3 Database › Query Execution › Output and Results (`database.output.results`)

| # | Label | Values | Meaning | Default | Ev. |
|---|---|---|---|---|---|
| OR-1 | Show timestamp for query output | checkbox | Prefix lines in the Output tab with timestamps. | **off** ("By default, timestamps are hidden") | H |
| OR-2 | Show results in editor | checkbox | In-editor ("notebook") results under each statement instead of the Services pane. Also toggled per console by action *In-Editor Results* (`Console.Toggle.Notebook.Mode`). | [Unknown] | SO, H, B `settings.database.notebook.mode` |
| OR-3 | Create title for results from comment before query | checkbox | Use the comment preceding a query as the result-tab title. | [Unknown] | SO, H |
| OR-4 | Treat text as title after | text field | Marker; text after it inside the preceding comment becomes the title. Empty = any text after `--` or `/*`. | **empty** ("By default, no combination is used") | H |
| OR-5 | Show Services tool window for query console output | For all output · For query output, errors, and result sets · For errors and result sets · For result sets · Never | When to pop the Services window. | [Unknown] | SO, H, B |
| OR-6 | Focus on Services tool window in window mode | checkbox | Move keyboard focus to Services if it is a floating window. | [Unknown] | SO, H |
| OR-7 | Open new Services tab for sessions | Enabled / Disabled | A separate Services tab per data-source session. | [Unknown] | SO, H |
| OR-8 | Activate Services output pane for selected query console only | checkbox | Do not jump to another console's output when its long-running query finishes. | [Unknown] | SO, H |

### 3.4 Database › Query Execution › User Parameters (`database.sql.templating`)

| # | Label | Values | Meaning | Default | Ev. |
|---|---|---|---|---|---|
| UP-1 | Enable in query consoles and SQL files | checkbox | Master switch for scripts. | [Unknown] (on [Inferred]) | SO, H |
| UP-2 | Enable in string literals with SQL injection | checkbox | Master switch for SQL injected into other languages' string literals. | [Unknown] | SO, H |
| UP-3 | Substitute inside SQL strings | checkbox | Also detect parameters inside SQL string literals (`'${actor_id}'`). | [Unknown] (off [Inferred from help example]) | SO, H |
| UP-4 | Parameter patterns | table of regex + scope | Each row: regex; *In scripts*; *In literals*; *All languages* (language subset). Reorderable (Alt+Up/Down). A capture group or a leading `:` or parentheses defines the parameter **name**. | 8 documented patterns (below) | H |

Default patterns as documented (described, regex notation is ours, semantics from the help):
1. `?` followed by digits (`?69` → name `69`).
2. `%` followed by word characters (`%xyz`).
3. `${…}` where the body contains no `$`, `{`, `}` (`${}`, `${value}`).
4. `$(…)` with any non-`)` body (`$(x)`).
5. `$name$` (word characters between two `$`).
6. `#name#`.
7. `$` + digits or identifier (dots allowed), optionally closed by `$`, only when preceded by a non-word char or start and followed by non-word or end — so Oracle `V$SESSION` is not a parameter but ` $ACCESS` is.
8. Same as 7 with `#`.
[Observed: H `settings-tools-database-user-parameters`] Note that plain `?` and `:name` (JDBC-style) are handled natively by the driver/statement layer rather than through this table [Inferred: they are absent from the documented list but DataGrip prompts for them — verify with doc 13 case family PARAM].

### 3.5 Database › Data Editor and Viewer (`database.data.views`, `database.data.views.appearance`)

**Limitations**

| # | Label | Meaning | Default | Ev. |
|---|---|---|---|---|
| DV-1 | Limit page size to: [n] | Rows per grid page; unchecked = unlimited. Also changeable from the paging toolbar ("Change Page Size"). | **500** [Observed: help `rows` and `compare-data`: "the number of returned rows is limited to 500"] | SO, H |
| DV-2 | Result set prefetch size | JDBC fetch size: rows per round-trip. | [Unknown] | SO, H |
| DV-3 | Filter history size | Remembered WHERE/ORDER BY conditions per table (Alt+Down opens history). | [Unknown] | SO, H |
| DV-4 | Maximum number of bytes loaded per value | LOB/large value truncation threshold; larger values show "{0} of {1}" and can be loaded fully on demand; tooltip "Values exceeding the limit will not be loaded fully". | help `tables-view-data` says "the default LOB size is limited to 1024 bytes" (for image preview) [Observed]; general default [Unknown] | SO, H, B; doc 08 ALG-GRID-09 |
| DV-5 | Show first N data rows in preview | Rows in the Quick Documentation (Ctrl+Q) table preview. | [Unknown] | SO, H |
| DV-6 | Enable paging in in-editor results by default | In-editor results get paging controls. | [Unknown] | SO |
| DV-7 | Position of the grid pagination control | Where the floating paging widget sits (left / center / right / editor toolbar — mirrored by actions `MoveGridFloatingPaging.*`). | [Unknown] | SO, keymap |

**Data Presentation**

| # | Label | Meaning | Default | Ev. |
|---|---|---|---|---|
| DV-8 | Use custom font: (Size, Line height) | Font for grid/results only, not editors. | off | SO, H |
| DV-9 | Alternate row colors | Zebra striping (color key "Data Grid//Alternating row color"). | [Unknown] | SO, H |
| DV-10 | Show boolean values as: Text · Checkboxes | Text mode supports single-letter typing: `f`,`t`,`d` (default),`n` (null),`g` (generated),`c` (computed); Space toggles. | [Unknown] | SO, H |
| DV-11 | Automatically transpose tables: Never · If a table has one record · Always | Applies to tables/views opened in the data editor, not to query results. | [Unknown] | SO, H |
| DV-12 | Automatically detect binary values: (Text / UUID / …) | Default display type for binary columns (hex, text, UUID). | [Unknown] | SO, H |
| DV-13 | Display temporal data in time zone | Display TZ for date/time values. | [Unknown] (system TZ [Inferred]) | SO, H |
| DV-14 | Show the quick actions popup toolbar for cells | Floating cell toolbar. | [Unknown] | SO |
| DV-15 | Enable customization of the quick actions popup toolbar | Allow editing that toolbar (actions `CustomizeGridFloatingToolbar`, `DisableGridFloatingToolbar`). | [Unknown] | SO, keymap |
| DV-16 | Enable immediate completion in grid text cells | Completion popup while typing in cells (e.g. FK values, enum labels). | [Unknown] | SO |
| DV-17 | Enable local filter by default | Excel-style local filter enabled when grids open. | [Unknown] | SO |
| DV-18 | Remember whether the filter is ON | (Listed on "Other", see 3.9.) | | |

**URL Click Settings** — "Allow opening:" Standard links (HTTP) · Secure links (HTTPS) · Local file links; "If no protocol is specified, assume HTTP for URLs"; the click binding uses a pattern rendered from `{0}{1,choice,1#|2#Middle-|3#Right-|4#Button 4 |5#Button 5 }Click` (e.g. "Ctrl+Click", "Middle-Click"). [Observed: SO] Defaults [Unknown].

**Custom Number Formats** — Decimal separator (**default `.`** [Observed: H]), Grouping separator, Infinity text, NaN text, Number pattern(s) (Java DecimalFormat-style patterns; "Number patterns" link opens a reference). [Observed: SO, H]

**Data Sorting** — Sort via ORDER BY (server-side sort on every reopen vs local) [Unknown default]; Sort tables by numeric primary key: Ascending · Descending (plus presumably "none") [Unknown]; Add columns to sorting: (mouse/keyboard gesture) **default Alt+Click** [Observed: H "The default is Alt and a click"].

**Data Modification** — Submit changes immediately (auto-submit each edit vs pending-changes batch). [Unknown default; off [Inferred]]

**Custom Date/Time Formats** — five patterns: Datetime/timestamp; Datetime/timestamp with time zone; Time; Time with time zone; Date; plus Time zone; "Date patterns" link. [Observed: SO, H] Defaults [Unknown] (ISO-like `yyyy-MM-dd HH:mm:ss` family [Inferred]).

**Grid colors** (Editor › Color Scheme › Data Editor and Viewer): Data Grid//Error data, Image data, Null data, Alternating row color. [Observed: SO grid-impl]

### 3.6 Database › Data Editor and Viewer › CSV Formats (`database.data.csv.formats`)

A list of **named formats** (add, remove, copy, reorder) shared by copy, paste, import, export and the CSV file editor. Preview shows 10 records. [Observed: H]

Predefined formats: **CSV**, **TSV**, **Pipe-separated**, **Semicolon-separated**; each exists "with header" / "without header" when chosen from menus. [Observed: B `csv.format.*.default.name`, `csv.format.with.header`]

| # | Label | Values | Meaning | Default (for "CSV") |
|---|---|---|---|---|
| CSV-1 | Value separator | Comma · Tab · Semicolon · Pipe · Space · custom | Field delimiter | Comma |
| CSV-2 | Row separator | Newline · custom | Record delimiter | Newline |
| CSV-3 | Null value text | Undefined · Empty string · `\N` · custom | Text written/read for NULL. "Undefined" = no special NULL text. | [Unknown] |
| CSV-4 | Add row prefix/suffix (Row prefix, Row suffix) | text | Extra markers around each row (e.g. `(`…`),` for SQL-ish output) | empty |
| CSV-5 | Quotation | list of (Left, Right, Escape) patterns | First pattern is used when writing; escape is `<duplicate>` (double the quote) or a custom char | `"` `"` duplicate [Inferred] |
| CSV-6 | Quote values | Never · When needed · Always | When needed = value contains value/row separator | When needed [Inferred] |
| CSV-7 | Trim whitespaces | checkbox | Strip surrounding whitespace | [Unknown] |
| CSV-8 | First row is header | checkbox | Header row | [Unknown] |
| CSV-9 | First column is header | checkbox | Header column (transposed data) | off |

Related: action "Change CSV Format of File…" (`FileEditor.ChangeFormat`), "First Row Is Header" toggle on a CSV grid, "Configure CSV Formats…" from Copy menu. [Observed: B, keymap]

### 3.7 Database › SQL Dialects (`SQL Dialects`)

| # | Label | Meaning | Default |
|---|---|---|---|
| DI-1 | Global SQL Dialect | Dialect for all `.sql`/`.ddl` files on the machine. | `<None>` → Generic SQL [Observed: H "If you do not set a dialect, the Generic dialect is applied"] |
| DI-2 | Project SQL Dialect | Overrides global for project files; `<None>` inherits. | `<None>` |
| DI-3 | Path / SQL Dialect table | Per file/directory override. Explicit entries black, inherited gray italic. | empty |

Consoles: the dialect is defined by the attached data source and can only be changed for Oracle consoles (SQL vs SQL*Plus flavour). Choosing **Generic SQL** disables syntax-error highlighting while keeping keyword/table/column completion. [Observed: H] 32 selectable dialects are documented (Amazon Redshift, Apache Cassandra, Apache Derby, Apache Hive, Apache Spark, Azure SQL Database, BigQuery, ClickHouse, CockroachDB, Couchbase Query, DynamoDB, Exasol, Generic SQL, Greenplum, H2, HSQLDB, IBM Db2 iSeries, IBM Db2 LUW, IBM Db2 z/OS, MariaDB, Microsoft SQL Server, MongoDB, MySQL, Oracle, Oracle SQL*Plus, PostgreSQL, Redis, Snowflake, SQLite, SQL2016, Sybase ASE, Vertica). [Observed: H] Dialect internals are in doc 06.

### 3.8 Database › SQL Resolution Scopes (`SQL Resolution Scopes`)

| # | Label | Meaning | Default |
|---|---|---|---|
| RS-1 | Project mapping | Set of data sources / databases / schemas used by every SQL file of the project to resolve unqualified names when no path mapping applies. | "All data sources" [Inferred] |
| RS-2 | Path → Resolution Scope table | Map a file/dir to one or more data sources (optionally narrowed to DB/schema). When adding a directory whose children already have mappings, a dialog asks whether children inherit or keep their scope. | empty |

[Observed: H `settings-languages-sql-resolution-scopes`]

### 3.9 Database › Other (`database.other`)

| # | Group | Label | Meaning | Default |
|---|---|---|---|---|
| OT-1 | Modify Object | Confirm cancellation for dialogs that modify schema | Confirm on Cancel/Esc in Create/Modify dialogs. | [Unknown] |
| OT-2 | Refactoring | Show preview of valid script when updating source text | Always show the Object Migration dialog when submitting edited routine/view source, even if valid. | [Unknown] (on — "By default, the Object Migration dialog is displayed") [Observed: H] |
| OT-3 | DDL Mappings | Suggest dumping DDL for new mappings | After adding a DDL mapping, offer to dump the data source to a DDL data source. | [Unknown] |
| OT-4 | Code Generation | Generate context templates: Append to existing console · In a new console | Where "Context Templates" (`DatabaseView.Context.Templates`) put generated SQL. | [Unknown] |
| OT-5 | Code Generation | Statement delimiter: | Delimiter used by generators (SQL Generator / scripts) between statements. | [Unknown] (`;`) |
| OT-6 | Virtual Foreign Keys | Column patterns pane | Rules that map a column name pattern to a target table/column (virtual FK); used by JOIN completion and Related Rows. Has a rule debugger. | empty |
| OT-7 | SQL Resolution | Default resolve mode for consoles: (Script / Playground) | Script: earlier statements affect later ones (DDL overlay); Playground: statements independent. | [Unknown] (Playground for consoles [Inferred]) |
| OT-8 | Database Explorer | Remember whether the filter is ON | Persist the Database Explorer filter toggle. | [Unknown] |
| OT-9 | Editor | (group heading; contains OT-5/OT-7 in some layouts) | | |

[Observed: SO `database.other`, H `settings-tools-database-other`]

### 3.10 Editor › General › Code Completion › SQL (from `lib/product.jar` index)

| # | Group | Label | Values | Meaning | Default |
|---|---|---|---|---|---|
| CC-1 | Basic completion | Suggest objects from: | The current search path only · The current scope · All available schemas | Breadth of object candidates. | [Unknown] |
| CC-2 | Qualify object with: | Database: | Always · On collisions · Never | Insert `db.` prefix | [Unknown] (On collisions [Inferred]) |
| CC-3 | Qualify object with: | Schema: | same | Insert `schema.` prefix | [Unknown] |
| CC-4 | Qualify object with: | Table/View: | same | Qualify columns with table name | [Unknown] (Never) |
| CC-5 | Qualify object with: | Table/view alias: | same | Qualify columns with alias | [Unknown] (Always when alias exists) |
| CC-6 | Qualify object in: | Basic completion · JOIN completion · Live templates · Refactoring · Drag-n-Drop | checkboxes | Contexts where CC-2..5 apply | [Unknown] |
| CC-7 | JOIN clauses | Use aliases in completion for JOIN | checkbox | Generated ON conditions use aliases | [Unknown] |
| CC-8 | JOIN clauses | Invert order of operands in auto-generated ON clause | checkbox | Off: FROM-side table first (`a.x = b.y`) | off [Observed: H "When the checkbox is cleared, the FROM table comes the first"] |
| CC-9 | JOIN clauses | Suggest non-strict foreign keys based on the name matching | checkbox | Fuzzy name-based JOIN suggestions (see virtual FK rules) | [Unknown] |
| CC-10 | Table aliases | Automatically add aliases when completing table names | checkbox | Insert generated alias after table | [Unknown] |
| CC-11 | Table aliases | Suggest alias names in completion after table names | checkbox | Offer alias names as completion items | [Unknown] |
| CC-12 | Table aliases | Custom aliases (table → alias) | table | User alias dictionary | empty |

Platform completion options that affect SQL: Match case (All letters / First letter only), Sort suggestions alphabetically, Show suggestions as you type, Insert selected suggestion by space/dot/context keys, Additional characters to accept completion, **Machine Learning-Assisted Completion** (Sort completion suggestions based on machine learning; Mark the most relevant item; Mark position changes) with **SQL** listed as an ML language, Parameter Info (Show the parameter info popup in N ms; Show full method signatures), Insert parentheses automatically, Show the documentation popup in N ms. [Observed: SO product.jar `editor.preferences.completion`, H `settings-code-completion`]

### 3.11 Editor › General › Smart Keys › SQL (`editor.preferences.sqlOptions`)

| # | Label | Meaning | Default |
|---|---|---|---|
| SK-1 | Insert string concatenation on Enter | Enter inside a string literal splits it into `'a' || 'b'` (dialect concat operator). Turn off for DBMSs with multi-line literals. | [Unknown] |
| SK-2 | Close code blocks on Enter | After `BEGIN`, `LOOP`, `BEGIN TRY`, … Enter inserts the matching `END`, `END LOOP`, `END TRY`. | [Unknown] |
| SK-3 | Qualify object on code completion | Always · On collisions · Never (help documents this here; the 2024.3 index shows the detailed version under Code Completion, CC-2..5) | [Unknown] |

[Observed: SO, H `settings-smart-keys-sql`] Intentions "Introduce/Inline table alias" re-qualify according to these settings. [Observed: intention descriptions]

### 3.12 Editor › Code Style › SQL

**Structure.** One **General** page plus pages for: SQL:2016/Generic · Apache Derby · Db2 · H2 · HSQLDB · MS SQL Server/MS Azure · MySQL/MariaDB · Oracle · PostgreSQL/Greenplum/Redshift · SQLite · Sybase ASE. Every dialect page has: *Scheme*, *Set from…*, *Disable* / *Disable formatting*, **Inherit general SQL style**; the General page additionally has *Preview dialect:* and lists the dialects. [Observed: SO `preferences.sourceCode.*`] Dialects not listed (ClickHouse, Snowflake, BigQuery, …) use the General style [Inferred: no page].

Option counts per page: General 192 labels; Derby/Db2 175; PostgreSQL 179; MS SQL 172; Sybase 171; Oracle 170; MySQL 169; H2 158; SQL:2016/HSQLDB/SQLite 157. Differences come from procedural-code groups (Code tab) that only exist for dialects with procedural languages. [Observed: computed from resolved index]

**Dialect-unique options.** PostgreSQL family: *Wrap opening $$*, *Wrap the content after opening $$*, *Wrap before closing $$*, *Wrap options after closing $$*. MS SQL: *Align line comments (MS SQL only)*. All others are shared. [Observed: set difference]

**Predefined schemes ("Set from… › Predefined Style")**: Modern, Joe Celko, Allman, Whitesmiths, Egypt, Old idea (six `predefinedCodeStyle` registrations in `SqlPlugin.xml`). [Observed: X; prior notes report-editor.md §4]

Below, every option label by tab. "Wrap/Place" options are typically tri-state or enum (e.g. *Do not change / Wrap always / Wrap if long / Chop down if long*), alignment options are checkboxes, and "Place X under" options are enums of anchor tokens [Inferred: platform formatter conventions; exact value lists in help `code-style-sql-queries`].

**Tab: Case** — Word Case: Keywords, Built-in types, Custom types, Built-in functions, Identifiers, Quoted identifiers, Aliases (each *To upper / To lower / Do not change*); Use original case (apply declaration case throughout file); Identifier quotation: Quotation (*Quote / Unquote / Do not change*), Quotation character. [Observed: SO, H `code-style-sql-case`]

**Tab: Queries** — Common: Align the first word of clause; Keep section elements under section header; Align section elements; Place clause elements on (…); Place comma; Align line comments at right of elements; Spaces within parentheses; Space after keyword; Collapse short statement; Use AS; Align AS; Align `=`; Align ASC/DESC; Place the opening parenthesis; Place the closing parenthesis; Keep elements on one line if ≤ N; New line after ALL, DISTINCT; Treat asterisk as a regular element. Sections: WITH clause; SELECT clause; FROM clause (Align joined tables; Align table aliases; Indent JOIN; Wrap the first JOIN; Wrap the next JOIN; Place JOIN in join-only queries under; Place ON/USING under; Wrap ON/USING); WHERE and HAVING clauses (Place top-level AND/OR); GROUP BY and ORDER BY clauses; Subquery (Place subquery; Wrap subqueries); INSERT statement and VALUES clause (Place INTO on the new line; Place INTO clause elements on; Place columns or values; Wrap columns or values; Place VALUES clause elements (rows) on; Align multi-row VALUES; Align multi-row VALUES with INSERT; Collapse short multi-row values; Align multi-row VALUES break threshold; …minimal increase threshold; …percentile); UPDATE statement; Place elements on; Wrap elements. [Observed: SO]

**Tab: DDL** — CREATE TABLE / CREATE or ALTER TABLE / ALTER TABLE / CREATE SCHEMA / CONSTRAINT / Views groups: Place the opening parenthesis; Place the closing parenthesis; Place elements; Indent content; Collapse when short; Align types; Align defaults; Align nullabilities; Align options; Indent options; Postfix options; Wrap first option; Wrap next option; Wrap CONSTRAINT; Wrap KEY/CHECK; Wrap REFERENCES; Wrap cascade and deferrability; Wrap altering instructions; Indent altering instructions; Align altering instructions; Wrap altered items; Indent altered items; Align altered items; Wrap AS; Wrap the beginning of the query; Indent query; Minimum / Maximum blank lines between declaration. [Observed: SO]

**Tab: Code** (procedural) — Imperative Commons: Keep blank lines in code; Place comma; Spaces within parentheses; Place elements; Place the opening/closing parenthesis; Wrap elements; Align types; Collapse when short. Declared variables: Wrap variables; Align types; Align expressions; Align assignments. Routine arguments; Routine statement (Wrap AS; Wrap section; Wrap inner code). IF … THEN … ELSE (Wrap THEN; Wrap ELSE; Indent THEN and ELSE; Indent END IF). Loops (Wrap LOOP; Indent LOOP; Indent END LOOP). Script: Wrap every statement; Wrap command-ending semicolon; Commands follow by blank lines. PostgreSQL `$$` group (dialect-unique, above). MS SQL: Align line comments. [Observed: SO]

**Tab: Expressions** — Cortege (row-value lists): Place comma to begin; Space before comma; Space after comma; Space within parentheses; Space within parenthesized sub-expressions. Function or procedure call: Add space before `(`. Binary expression: Use spaces around operators; Align operands in binary expressions. CASE clause: Collapse short clause; Wrap WHEN; Indent WHEN if wrapped; Wrap THEN; Align THEN; Align ELSE under THEN when THEN aligned; Keep new line after THEN, ELSE; Align END. [Observed: SO]

**Tab: Tabs and Indents** — Use tab character; Smart tabs; Tab size; Indent; Continuation indent; Keep indents on empty lines. [Observed: SO]

**Tab: Wrapping** — Keep when reformatting: Line breaks; Comment at first column. [Observed: SO]

**Tab: Code Generation** — name templates: Primary key; Foreign key; Index. Templates contain literal text and variables in braces, e.g. `{table}`, `{columns}`, conditional `{unique?u:}` ("if unique insert `u` else nothing"). Help example: `{table}_{columns}_{unique?u:}index` → `persons_FirstName_LastName_index`; `{table}_pk` → `actor_pk`. Ctrl+Q in a field lists variables. [Observed: H `code-style-sql-code-generation`] Exact defaults [Unknown] (doc 09 ALG-DDL-04; doc 13 case SET-CG).

**Formatter registry knobs**: `sql.formatter.consider.language.injection` = true (format injected languages inside SQL); `sql.formatter.insert.matrix.limit.rows` = 1000 (max rows for INSERT VALUES matrix alignment). [Observed: X `DatabaseRegistryKeys.xml`]

### 3.13 Advanced Settings › Database and Registry

**Advanced Settings** (user-visible, `advancedSetting` declarations):

| id | Label | Default | Ev. |
|---|---|---|---|
| `database.grid.navigate.to.related.rows.behavior` | Related Rows action behavior: Navigate and filter all · Navigate and select first | `NAVIGATE_AND_FILTER_ALL` | X gridimpl + SO |
| `database.import.open.detected.tables` | Open file as table if detected by scripted loader: All · None · Table-first formats | `TABLE_FIRST_FORMATS` | X gridimpl + SO |
| `database.show.search.tab` | Show Database tab in Search Everywhere | `false` | X DatabaseRegistryKeys |
| `database.goto.class.enabled` | (Show database objects in Go to Class — label "Show Classes tab…" nearby) | `false` | X plugin.xml |

**Registry keys** (hidden; Help › Find Action › "Registry…"). All defaults are declared in XML. [Observed: `DatabaseRegistryKeys.xml`, `gridimpl/META-INF/plugin.xml`]

| Key | Default | Purpose (from description) |
|---|---|---|
| database.scrollable.result.sets | true | Use scrollable JDBC result sets when the driver supports them |
| database.result.set.limit | 100 | Max result sets per query execution |
| database.generic_dialect.limit | true | Allow LIMIT clause in Generic dialect paging |
| database.debug.enable / .trace | true / false | SQL debugger on; trace in debugger window |
| database.grid.paging.offset | true | Load pages with LIMIT/OFFSET syntax |
| database.snapshots.auto | true | Inject pre-introspected system objects (minicat) |
| database.introspection.level.auto.select.strategy | "" | Oracle introspection level strategy (mini/simple) |
| database.datasource.registry.using.storages | false (restart) | List data sources from storage vs manager |
| database.datasource.registry.in.advance | true (restart) | Register data source when it gains a model |
| database.view.show.mini.cat | false (restart) | Show system objects in tree |
| database.view.copy.datasource | true | Copy/paste data sources with Ctrl/Cmd+C/V |
| database.view.levels.hide.L3 | false | Hide introspection-level pills |
| database.view.ds.bold | false | Bold data sources |
| database.view.filter.enabled | true | Filter tree during speed search |
| database.view.layers.* (4 keys) | false/true/true/false | Tree cache invalidation policy |
| database.scopeRefresh.listSchemasFromAllDatabases | true | Visit every DB when listing schemas (slow on PG/Azure) |
| sql.formatter.consider.language.injection | true | Format injected code |
| sql.formatter.insert.matrix.limit.rows | 1000 | INSERT matrix-align row cap |
| database.ssl.use.socket.factory | false | SSL via socket factory instead of key stores |
| database.csv.preview.char.count | 10000 | Characters sampled for CSV preview/detection |
| database.query.generation.omit.schema.qualifier | false | Omit schema in generated queries |
| database.storage.output.buffer | 10000000 | Schema write buffer (-1 infinite) |
| database.migration.source.revert.on.success | true | Reset editor to DB version after successful source submit |
| database.legacy.navigate.to.schema / .to.code.from.tree | false / false | Legacy navigation behaviors |
| database.batch.size | 2000 | Statement batch size (import, copy) |
| database.disable.escape.processing | true | Disable JDBC escape processing (`{fn …}`) |
| database.dump.model.useEditorColors | false | Colors in Dump Model dialog |
| database.legacy.schema.diff | false | Pre-2021.3 diff window |
| database.schema.diff.table.view | true | Table-tree diff view |
| database.schema.diff.show.root | true | Show root node in diff |
| database.automatically.load.related.data.in.cell.quick.doc | true | Related rows in cell Quick Doc |
| database.script.generator.show.dump.option | false | Show dump option in generator |
| database.script.generator.separate.disable | false | Inline disabled constraints in CREATE when possible |
| database.completion.experimental.names | false | Experimental name completions (e.g. constraint names) |
| database.track.async.tasks | false | Debug |
| database.enable.routine.call.hierarchy | false | Experimental routine call hierarchy |
| database.automatically.load.sources | true | Load routine/view sources on connect |
| custom.cli.run.target.providers | false | Dump/restore via Docker etc. (doc 10) |
| database.grid.render.cache | true | Cache rendered cell images |
| database.grid.render.cache.fast.scroll | true | Lower quality while fast-scrolling |
| database.grid.cache.factor | 3 | Render cache size factor |
| database.grid.show.chart.tab | true | "Show Chart" action |
| grid.tables.columns.limit | 2000 | Max displayed columns |

### 3.14 Color Scheme pages

| Page | Attributes |
|---|---|
| **SQL** | Bad token; Braces; Brackets; Column; Comma; Comment; Database object; Dot; External command; Keyword; Label; Number token; Outer query column; Parameter; Parentheses; Procedure or function; Schema; Semicolon; String token; Synthetic entity; Table or column alias; Table; Type; Variable (24) |
| **Database** | Console//Statement to execute (highlight of the range about to run); Parameters//Parameter usage; Parameters//Parameter |
| **Data Editor and Viewer** | Data Grid//Error data; Image data; Null data; Alternating row color |
| **Table Diff** | Excluded from diff; Fuzzy match – matched; Fuzzy match – mismatched |

[Observed: SO] "Outer query column" (correlated subquery references) and "Synthetic entity" (implicit objects such as `*`, NEW/OLD) are noteworthy semantic colors. Data-source colors (per data source tint for tabs/editor) are a separate per-data-source property (doc 00–05).

### 3.15 Other platform pages with SQL/database entries

| Page | Entry | Meaning |
|---|---|---|
| Editor Tabs | Always show qualified names for database objects in tab titles; Shorten datasource and object names in tab titles | Tab title policy for table editors/consoles (action *Shorten Tab Titles* also exists) |
| Code Folding | SQL group | SQL folding defaults (exact options not resolvable from index) [Unknown] |
| Breadcrumbs | SQL | Show breadcrumbs (statement/clause path) for SQL |
| Sticky Lines | SQL | Sticky statement headers for SQL |
| Passwords | Database: | Mentions database credentials storage (details doc 00–05) |
| Output Console | console buffer, soft wraps, history size | Applies to Output pane |

### 3.16 Debugger › Stepping › Oracle

Pause at begin: (checkbox); Stepping mode: Graceful · Native. [Observed: SO `debugger.stepping` from database plugin] Relevant only to the PL/SQL debugger (not a Hacivat v1 target).

### 3.17 Keymap: database and grid action inventory

The keymap index lists ≈320 distinct DB/grid actions and groups. Groups (keymap folders): *Database › General, Execution, Sessions, Console Toolbar, Data Editor, Results Toolbar, Value Editor and Aggregate View*. Representative actions per area (full list in doc 07/08 where behavior is described):

- **Execution**: Execute, Execute (2), Execute (3), Explain Plan / Explain Plan (Raw), Explain Analyse / (Raw), Debug, Cancel Running Statements, View Parameters, Attach Data Source, Set Current Schema, Switch session, Enable DBMS_OUTPUT, Browse Query History, In-Editor Results, Shorten Tab Titles.
- **Transactions**: Commit, Roll back, Revert and Roll Back, Tx Isolation, (Auto/Manual mode).
- **Results/grid**: Add/Clone/Delete Row; Add/Clone/Delete/Rename Column; Set NULL; Set DEFAULT; Generate UUID; Clear Field; Load File…; Save LOB…; Submit; Submit and Commit; Preview Pending Changes; Revert Selected; First/Previous/Next/Last Page; Reload Page; Change Page Size; Change Table Sampling; Count Rows; Edit Filter Criteria; Edit ORDER BY Clause; Filter by; Set Local Filter / Enable Local Filter / Local Filter By Cell Value; Sort Ascending/Descending/Add Ascending/Add Descending/Clear Sorting; Show/Hide Columns; Hide Column; Hide Other Columns; Show All Columns; Columns List; Change Display Type; Set Highlighting Language…; Transpose; View as Table/Tree/Text; Record View; Show Chart/Switch to Chart/Table; Show Aggregate View; Enable/Disable All Aggregators; Copy Aggregation Result; Change Data Extractor; Go to Scripts Directory (extractors / aggregators); Copy; Copy Column Name; Paste Format; Export Data…; Export Table to Clipboard; Copy to Database…; Compare Data; Compare Cells…; Related Rows; Go to Referenced/Referencing Result; Find on Current Page; Go To (row); Open URL; Open File URI; Show Value Editor / Open in Value Editor / Hide; Value editor: Change Type, Move to Bottom/Right, Toggle Formatting, Toggle Soft-Wrap, Change Layout; Skip Computed / Skip Generated Columns; View Query; Copy Query to Console; Rename Tab; Reset View; Pagination toggle; Customize/Disable floating toolbar.
- **Database Explorer**: New › Data Source/Driver/DDL objects; Manage Shown Schemas…; Set Color…; Duplicate; Generate DDL to Clipboard; Request and Copy Original DDL; Copy Table to…; Import from Clipboard; Create DDL Mapping…; Copy/Cut/Paste Data Source; Modify Object; Modify Indexes; Context Templates; Clear Stored Sources; Forget Cached Schemas.
- **DDL editor**: Regenerate definition.

[Observed: resolved keymap entries of the four indexes]

---

## 4. Algorithm specifications

### ALG-SET-01: Layered setting resolution (scopes)
- **Problem:** A value (dialect, resolution scope, code style, execution policy) must be resolved from several scopes with inheritance, the way DataGrip resolves dialect (path → project → global → Generic) and resolution scopes (path → project mapping).
- **Inputs / outputs:** `key`, `context {file_path?, project?, data_source?, console?, dialect?}` → `(value, origin_scope)`.
- **Algorithm:**
  1. Build the scope chain for the context, most specific first: `console` (session overrides, e.g. current schema, in-editor results toggle) → `data_source` (e.g. read-only, dialect of console) → `path` entries (longest matching path prefix first; a file entry beats its directory) → `project` → `global/user` → `built-in default`.
  2. For each scope in order, if it holds an *explicit* value for `key` (not `<None>`/inherit), return it with `origin_scope`.
  3. If none, return the built-in default (`origin_scope = Default`).
  4. For keys of kind *set-valued with inherit-or-replace* (resolution scopes), apply the user's choice recorded when the mapping was added: `Inherit` → union with parent; `Preserve` → stop.
- **Data structures:** `ScopeChain = Vec<ScopeRef>`; per scope a `BTreeMap<SettingKey, SettingValue>`; path scopes in a trie keyed by normalized path components.
- **Complexity:** O(depth × log n) per lookup; cache per (file, key) and invalidate on settings change events.
- **Edge cases:** file moved/renamed (path entries must follow — DataGrip shows explicit vs inherited values in black vs gray italic); symlinks (normalize to real path or not — choose and document); case-insensitive file systems (macOS/Windows: compare paths case-insensitively **without** locale-sensitive lowering — Turkish `I`); a console attached to no data source (falls to project/global); console dialect may be overridden only for Oracle-like flavours; removed data source referenced by a path mapping (show as broken, fall through); conflicting directory/file mapping; project-less mode (Hacivat offline single-window).
- **Test cases:** global=PG, project=Oracle, dir `mysql/`=MySQL → `mysql/a.sql`→MySQL, `b.sql`→Oracle, file outside project→PG; nothing set → Generic; console on SQLite source in a dir mapped to MySQL → SQLite (console wins).
- **Prior art and evidence:** [Observed: H `settings-languages-sql-dialects`, `settings-languages-sql-resolution-scopes`]; IntelliJ `PerFileMappings` (Apache-2.0) [Inferred].
- **Used by:** Hacivat.

### ALG-SET-02: Per-dialect code-style overlay
- **Problem:** Represent "General SQL style" plus dialect pages with *Inherit general SQL style*, *Disable formatting*, *Set from…* and predefined schemes.
- **Inputs / outputs:** `(scheme, dialect)` → effective `SqlStyle`.
- **Algorithm:**
  1. `base = scheme.general` (General page values; each scheme is a full copy, predefined schemes are read-only templates).
  2. `page = scheme.dialect_page(dialect_family(dialect))`; dialects without a page (ClickHouse, Snowflake, …) map to General.
  3. If `page.inherit_general` → effective = base overlaid with **only dialect-unique options** from `page` (e.g. `$$` wrapping); else effective = page values (full copy).
  4. If `page.disable_formatting` → formatter returns the input unchanged (but case conversion?) — [Unknown] whether case still applies; test in doc 13.
  5. *Set from…* copies only options applicable to the target language from another language/scheme; unrelated options untouched.
- **Data structures:** `SqlStyle` as a flat struct of typed options + `BTreeSet<OptionId>` "applicable to family"; scheme = `{general, pages: HashMap<Family, Page>}`.
- **Edge cases:** option added in new version (missing in stored scheme → default); option removed (ignore unknown keys but keep them for round-trip); importing a JetBrains code-style XML (optional compatibility); family aliasing (Greenplum/Redshift share PG page; MariaDB shares MySQL; Azure shares MS SQL).
- **Test cases:** General keywords=upper, PG page inherit=on → PG file keywords upper; PG inherit=off with keywords=lower → lower; ClickHouse file → General.
- **Prior art:** [Observed: SO page structure]; IntelliJ `CodeStyleSettings` + `CustomCodeStyleSettings` + `PredefinedCodeStyle` (Apache-2.0) [Inferred].
- **Used by:** Hacivat.

### ALG-SET-03: Key-based searchable settings index
- **Problem:** Settings search must work in any UI language without re-indexing, and must find options on not-yet-opened pages.
- **Inputs / outputs:** build time: settings schema → index `[{page_id, page_name_key, entries:[{label_key, path_key?}]}]`; run time: query string + active locale → ranked `(page, option)` hits.
- **Algorithm:**
  1. At build time, walk the declarative settings schema (each option declares `label_key`, optional `group_key`, `page_id`, synonyms keys) and emit the index with **message keys**, not text (exactly how JetBrains encodes `|b|bundle|k|key|`).
  2. At run time, resolve each key through the active locale **and** English (so English searches still work in Turkish UI); normalize both (Unicode NFKC, case-fold with locale-independent folding plus a Turkish-aware secondary fold so `i/İ/ı/I` all match), strip mnemonics.
  3. Tokenize query; score = prefix/word matches on label (×3), group (×2), page (×1); include synonyms.
  4. Navigate: open page, scroll to option, highlight.
- **Data structures:** prebuilt `Vec<IndexEntry>` + per-locale lazily built inverted index (`HashMap<token, Vec<entry_id>>`).
- **Edge cases:** labels with placeholders (`{0}` click patterns — expand with representative values or index the static parts); keymap entries (index action names too); options shown only for some dialects; duplicated labels on many pages (rank page relevance by current context, e.g. current file dialect).
- **Test cases:** TR UI, query "sayfa boyutu" → DV-1; EN query "page size" in TR UI → DV-1; query "İMLEÇ"/"imlec" → caret options.
- **Prior art:** [Observed: searchableOptions JSON format]; IntelliJ `SearchableOptionsRegistrar` [Inferred].
- **Used by:** Hacivat (settings UI), Perde (web settings could reuse).

### ALG-SET-04: Settings persistence, migration and sync
- **Problem:** Persist typed settings across versions (DataGrip relocates options between pages, e.g. appearance moved; renames enum values).
- **Algorithm:** store as versioned TOML/JSON per scope (`user.toml`, `project/.hacivat/settings.toml`, per data source in the data-source store); each file has `schema_version`; on load run ordered migrations (`vN→vN+1` functions: rename key, map enum value, move between scopes); unknown keys preserved verbatim; write atomically (temp + rename); emit change events with key granularity.
- **Edge cases:** downgrade (newer file, older app → read-only warning, keep unknown keys); concurrent writes from two windows (file lock + reload); secrets never stored here (OS keychain; doc 00–05); team-shared project settings vs personal (`.hacivat/settings.local.toml` git-ignored).
- **Test cases:** v1 file with `grid.page_size` migrates to `grid.limits.page_size`; unknown key survives round-trip.
- **Used by:** Hacivat; Perde (org-level policy overlays, see §5).

---

## 5. Implications for Hacivat / Karagöz / Perde

1. **Adopt the same page taxonomy, trimmed for v1.** Hacivat v1 pages: *Execution* (QE-1..7 per execute binding), *Results* (OR-2..4), *Parameters* (UP-1..4 with the same default regex table — but ship `?`, `:name`, `$1`, `@name` as dialect-native and the template patterns as opt-in), *Data Grid* (DV-1..17 + number/date formats), *CSV Formats*, *SQL Dialects*, *Resolution Scopes*, *Code Completion (SQL)*, *Smart Keys (SQL)*, *Code Style (SQL)* with General + family pages, *Colors*, *Advanced*. Drop Services-window options (no such tool window) and debugger stepping.
2. **One typed schema, many consumers.** Define settings once in Rust (`#[derive(Setting)]` with `label_key`, `help_key`, `default`, `scope`, `since_version`), generate: UI forms, the searchable index (ALG-SET-03), JSON Schema for docs, MCP-tool exposure (the local MCP server can expose read-only settings so agents know the active dialect/page size).
3. **Make defaults explicit and documented** — DataGrip hides most defaults in code; Hacivat should show "Default: X" and a reset-to-default per option.
4. **Per-execute-binding policies** (three Execute actions) are a strong UX idea: keep it, with defaults Ctrl/Cmd+Enter = smallest statement (with chooser on ambiguity), Ctrl/Cmd+Shift+Enter = whole script/batch.
5. **Expose "registry" as an Advanced page**, not a hidden dialog, with clear "experimental" labelling; keep the important numeric limits (result-set limit 100, batch size 2,000, CSV sample 10,000 chars, matrix-align 1,000 rows, column limit 2,000) as tunables.
6. **Perde policy overlay.** For org-managed connections, Perde can push *enforced* settings (e.g. read-only default, unsafe-query warning forced on, max page size, export disabled) as an extra scope above `global` in ALG-SET-01, marked locked in the UI. **Enforcement of anything security-relevant must also happen at Karagöz** (P2P traffic bypasses Perde): e.g. read-only and row-limit policies are re-checked by Karagöz on each statement (see doc 07 ALG-EXEC-05/06).
7. **Settings sync** across machines is optional and should go through Perde only if the user opts in; offline-first means local files are canonical.

---

## 6. i18n considerations

- Every label, group, page title, enum value and help text is a **message ID** (Fluent) — never a literal; the searchable index stores IDs (ALG-SET-03). JetBrains' index proves the approach scales to ~3,000 labels.
- Mnemonics: JetBrains encodes them inline (`&` / `_`) and the Japanese pack appends `(&F)` because the mnemonic letter is not in the translated word. Hacivat should store mnemonics separately (or not at all on macOS) so translators do not have to manage them (doc 12).
- Placeholders inside labels (`Keep elements on one line if ≤ {n}`, the click-pattern choice format) need named, typed arguments (`{ $count }`) and plural/select support, not Java `ChoiceFormat` (doc 12).
- Turkish-specific: number format defaults (decimal comma, dot grouping) must not change **CSV separators** or SQL literals — keep *display* formatting (DV number formats, locale-aware) strictly separate from *data* formatting (CSV, SQL generation: always invariant). Search folding must treat `i/İ/ı/I` correctly.
- Code-style option names in Turkish should keep SQL keywords untranslated (`JOIN`, `ON`, `VALUES`), as JetBrains' ja pack does [Observed: ja SqlBundle keeps keywords].

---

## 7. Risks, pitfalls, open questions

- **Defaults unknown** for ~60% of options (stored in class files). Mitigation: doc 13 family `SET` — open a fresh DataGrip profile (`Help › Change Memory/Config` or new config dir) and screenshot each page; record into this catalog.
- The searchable index can lag the UI (labels in bundles but not indexed, e.g. "Track databases/schemas creation and deletion"); some options may exist only in dialogs (Modify Object, Import, Export, Schema Compare) and are not in any index — those are catalogued in docs 08–10.
- Truncated index entries exist (`|b|messages.LangBundle|k|PerFileConfi...`) — harmless but shows the index is not a perfect source.
- Over-copying: DataGrip's ~190 code-style options per dialect are a maintenance burden; Hacivat should start with ~30 high-value options and presets, and grow only with demand.
- Open: exact enum value lists for "Place … on/under" options (help `code-style-sql-queries` documents them; verify against UI); whether "Disable formatting" still applies case conversion; whether User Parameters patterns apply to `:name`/`?` in addition to driver-native handling.
