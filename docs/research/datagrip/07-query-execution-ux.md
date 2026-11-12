# 07 — Query Execution UX (DataGrip 2024.3.5)

> Part of the Hacivat research library. Conventions: `../README.md`. Depends on doc 06 (ALG-SQL-01 statement splitter, resolve modes). Related: 03 (sessions, connection lifecycle, transactions at protocol level), 08 (result grid, pending changes, submit), 10 (query history storage, Execute to file/export, run configurations output, native tools), 11 (settings QE/OR/UP and Data Editor pages), 13 (families CARET, SEL, CHOOSE, PARAM, UNSAFE, RO, TITLE, TX, ERR). Sessions, drivers, read-only flag storage and introspection are in docs 00–05 and only referenced here.

---

## 1. Scope and sources

**Scope.** Everything between "user presses Execute" and "results/messages are shown": the three Execute actions and their range policies, the statement chooser, Generic/ANSI script splitting, user parameters (every default pattern, scope and switch), safety validators and read-only enforcement, the execution engine (requests, queue, auditors), script error handling, cancellation and its escalation, transaction UI, results presentation (Services tool window, sessions tree, result tabs, pinning, titles, in-editor results), the Output log and its message formats, notifications, server output (DBMS_OUTPUT), Execute to File, routine execution, run configurations (Database Script) and script running, large files, explain plan / flame graphs per DBMS, and the (Oracle-only) debugger.

**Sources (no decompilation; nothing pasted from shipped scripts).**

| Source | Used for | License |
|---|---|---|
| Offline help (DataGripHelp plugin, `datagrip-help.jar`, Oct 2024; text extracted to `.refs/scratch-extracts/help/txt`): `run-a-query`, `query-execution`, `output-and-results`, `settings-tools-database-user-parameters`, `viewing-query-results`, `query-consoles`, `managing-connection-sessions`, `services-tool-window`, `run-sql-files`, `run-debug-configuration-database-script`, `query-execution-plan`, `debugging-code`, `data-sources-and-drivers-dialog`, `oracle`, `ibm-db2-luw`, `submitting-and-reverting-changes`, `advanced-settings`, `database-explorer` | documented behavior | JetBrains docs |
| Help screenshots (`help-datagrip-assets.jar`: `images/db_query_exec_*.png`, `db_settings_database_query_execution.png`) — viewed, hashed | resolving the Smallest-statement contradiction; chooser layout; settings page layout | JetBrains docs |
| Online help `jetbrains.com/help/datagrip/query-execution.html` (2026.3 edition, fetched 2026-11) | confirms prose unchanged in later versions | JetBrains docs |
| Public YouTrack **DBE-6418** (REST API fetch), DataGrip 2022.2.3 release blog | origin of "Smallest subquery or statement" | public tracker |
| `DatabaseBundle`, `SqlBundle`, `DataGridBundle` (en; `.refs/scratch-extracts/bundles/en`) | exact labels and message formats (summary lines, tx modes, chooser header, cancel dialogs, error-dialog buttons) | Proprietary (short labels quoted, formats described) |
| Plugin XML (`.refs/datagrip-extract/dbplugin/META-INF`): `DatabaseRegistryKeys.xml` (all keys parsed), notification groups, `database.queryValidator` registrations, EPs (`database.consoleRunContextParametersTuner`, `database.parameterPatternProvider`, `sql.executionFlowAnalyzerProvider`), `intellij.database.flameGraphs.xml` | extension architecture, defaults, notification behavior | Proprietary |
| Class-name inventories (`.refs/datagrip-extract/classes/*.txt`) | mechanism names: `DataRequest$*`, `run/audit/*Auditor`, `dialects/*/plan/*ExplainPlanProvider`, `*PlanModelBuilder` | names only |
| Searchable-options index (`.refs/scratch-extracts/so_resolved.txt`) | settings page membership | Proprietary |
| IntelliJ Community (Apache-2.0): `FileUtilRt` large-file thresholds, `idea.max.content.load.filesize` / `idea.max.intellisense.filesize` system properties, run-configuration framework, Services view | platform behavior | Apache-2.0 |

---

## 2. Executive summary

1. **Three Execute actions** (Execute = Ctrl/Cmd+Enter, Execute (2), Execute (3)), each with its own shortcut and an independent policy triple: *caret inside*, *caret outside*, *selection* (doc 11 QE-1..3). [Observed: help, settings screenshot]
2. **Caret-inside options (7):** Ask what to execute · Smallest subquery or statement · Smallest statement · Largest statement · Largest statement or batch · Whole script · Everything from caret. **Caret-outside (3):** Nothing · Whole script · Everything below caret. **Selection (3):** Exactly as one statement · Exactly as separate statements · Smart expand to script. [Observed: bundle `settings.*`]
3. **Resolved:** the help *prose* for "Smallest statement" vs "Smallest subquery or statement" is swapped. The help *screenshots*, the YouTrack history (DBE-6418) and the labels agree: **Smallest subquery or statement runs the innermost subquery under the caret; Smallest statement runs the enclosing top-level statement** (identical to Largest statement except inside nested statements such as routine bodies). See §3.3. [Observed]
4. The **statement chooser** ("Statements" popup) lists innermost subquery → enclosing statement → all statements; labels are the collapsed single-line text including leading comments, truncated with "…"; hovering highlights the range in the editor; a "Customize" link opens Query Execution settings; the "all statements" item is always present. [Observed: help + screenshot]
5. **Generic / SQL:2016 split mode** has three choices (parse-or-separator default, grammar only, separator only). [Observed]
6. **User parameters**: `?` per vendor plus **8 default regex patterns** (`?N`, `%name`, `${…}`, `$(…)`, `$name$`, `#name#`, boundary-guarded `$name`, boundary-guarded `#name`); per-pattern scope (in scripts / in literals) and language subset; three master switches. Substitution is **textual** (arrays/fragments allowed). Values are remembered; "Review parameters before execution" re-shows them. [Observed]
7. **Safety validators** are inspections reused at execution time (`SqlWithoutWhereInspection`, `SqlJoinWithoutOnInspection`, `SqlTypeInspection`); warning "Unsafe query: {0}" with Execute / Execute All / Execute and Suppress / Skip / Skip All buttons. [Observed: registrations, bundle; button-to-dialog mapping Inferred]
8. **Read-only data sources** rely on the driver's read-only flag; help warns that consoles may still modify data if the driver ignores it; IDE-side messages cover DML / DDL / routine / generic side effects. [Observed]
9. **Engine**: typed requests per session queue (`DataRequest$*`) observed by auditors (timing, logging, tx markers, cancel progress). Max **100 result sets per query** (registry), batch size 2000, escape processing disabled, scrollable result sets on. [Observed]
10. **Script errors** surface an inline choice (Retry / Ignore / Ignore All / Stop) and the summary marks "(aborted!)" when stopped. [Observed: bundle labels; flow Inferred]
11. **Cancel** is a three-stage escalation: cancel request ("Cancelling…", red square in gutter) → after ~10 s a balloon "Cancelling of running statements takes too long… Deactivate" → a second press opens "Deactivate the Data Source?" with **Keep Waiting / Deactivate** (closes all its connections; statement may keep running server-side). Cancel also aborts connection establishment ("Connection canceled"). [Observed]
12. **Transactions**: Auto / Manual + five isolation choices via the **Tx** menu (labels `Tx: {0}`, `Tx Mode ({0})`, `Tx Isolation ({0})`), Commit / Roll back buttons in manual mode, close-with-open-transaction prompt, "Locked by transaction: {0}", Single session mode shares one transaction across consoles. [Observed]
13. **Results**: Services tool window (data source → sessions → clients), one result tab per result set (title = qualified table name, or comment before the query), reuse vs new tab, Pin Tab, In-Editor Results with optional paging, compare result sets with tolerance. [Observed]
14. **Output log** lines are ChoiceFormat messages: "N rows retrieved starting from S in T", "N rows affected in T", "transaction committed/rolled back", "Summary: K of M statements executed, F failed in T (S symbols in file) (aborted!)". [Observed]
15. **Run configurations "Database Script"**: targets (data sources/schemas, many), script text and/or files, schema-switch warning, Before-launch tasks incl. *Disconnect Data Source*; output in Run tool window; help states a cap of **1000 DML statements per file** and a batch-mode message. [Observed]
16. **Large files**: >20 MB opens only the first 2.5 MB; the IDE nudges to run large scripts via run configurations. [Observed: help, bundle]
17. **Explain**: Explain Plan / Explain Analyse (+ Raw) for **21 dialect packages** (providers), tree/table "Plan" tab, diagram (Ctrl+Alt+Shift+U), flame graph by Total Cost / Startup Cost (+ Actual Total Time / Actual Startup Time). Five dialects have a provider but no model builder (Derby, H2, Hive, HSQLDB, Spark) → likely raw-only. [Observed: class names; Inferred: raw-only]
18. **Debugger** in 2024.3 is **Oracle-only** (DBMS_DEBUG based) — correcting earlier notes that listed MySQL. [Observed: help `debugging-code`]
19. Hacivat: implement ALG-EXEC-01..17; enforce read-only / unsafe / permission policies **again at Karagöz** because client-side validators are advisory and P2P traffic bypasses Perde.

---

## 3. Detailed findings

### 3.1 Entry points

| Entry | Behavior | Evidence |
|---|---|---|
| Execute (Ctrl/Cmd+Enter), toolbar ▶, context menu *Execute* | Policy triple of action 1 | help, keymap `Console.Jdbc.Execute` |
| Execute (2), Execute (3) | Same settings page, separate policies; no default shortcut | settings screenshot, keymap |
| *Run 'console [ds]'* (Ctrl+Shift+F10 in a console) | Executes **all** statements of the console | help `query-consoles` context menu |
| Context menu **Execute to File** (label pluralizes: "Execute to File/Files") | Export dialog, then streams results to file(s) without grid | help, bundle `ShowDumpDialogAction.ConsoleText` choice |
| Run Function / Run Procedure / Run Routine… (Database Explorer) | "Execute Routine" dialog: parameter values; optional *Run from* file/console; for Oracle also generates an anonymous block usable for debugging | help `run-a-query`, `debugging-code` |
| Run configuration **Database Script** (Run widget, Files tool window *Run '<file>'* Alt+Shift+R, Ctrl+Shift+F10 on selected files, Explorer *SQL Scripts › Run SQL Script*, drag file onto data source/schema, Search Everywhere + Ctrl+Shift+F10, *Current File* runner) | Runs files/text against one or many targets; output in Run tool window | help `run-sql-files` |
| Intention "Run query in console" (SQL injected in host-language string) | Opens console and executes | intention description |
| Explain Plan / Explain Analyse (+Raw) | Plan tab | help, keymap |
| Debug (Oracle) | PL/SQL debugger via DBMS_DEBUG (registry `database.debug.enable=true`) | help `debugging-code` |
| Preview DELETE/UPDATE rows intention | Runs derived SELECT | doc 06 |
| Edit as Table (selected INSERTs) | Opens INSERT values as grid, rewrites text (no execution) | help `query-consoles` |

Sessions: since 2024.1 the IDE manages sessions automatically; consoles/files attach to a data source + database/schema (top-right selectors); "Switch Session", "Attach Session", "Detach Session", and *Single session mode* exist. [Observed: help] Details: doc 03.

### 3.2 Execute actions and the policy triple

Settings › Database › Query Execution shows a list *Execute / Execute (2) / Execute (3)* on the left; for the selected action: *Shortcut*, *When caret inside statement execute*, *When caret outside statement execute*, *For selection execute*, and *Open results in new tab*. Below, page-wide: *Split a script for execution in Generic and ANSI SQL dialects*, *Review parameters before execution*, *Show warning before running potentially unsafe queries*. [Observed: screenshot `db_settings_database_query_execution.png`]

The screenshot shows (for *Execute*): Ask what to execute · Nothing · Exactly as separate statements · Open results in new tab ✔ · Into valid ANSI SQL statements or by separator · Review ✔ · Warning ✔. These are *probably* defaults (doc writer's fresh profile) but the help text says "By default, DataGrip updates the same tab", contradicting the ✔ on *Open results in new tab* — record as **[Unknown — doc 13 SET family]**.

| Policy | Option | Semantics (help + resolution in §3.3) |
|---|---|---|
| inside | Ask what to execute | Show the Statements chooser |
| inside | Smallest subquery or statement | Innermost subquery containing the caret; if none, the statement |
| inside | Smallest statement | Smallest *statement* containing the caret (a subquery is not a statement) — usually the top-level statement; inside routine bodies/blocks the innermost nested statement |
| inside | Largest statement | Outermost statement containing the caret |
| inside | Largest statement or batch | T-SQL (SQL Server, Sybase): the current batch (between `GO`s); other dialects: as Largest statement |
| inside | Whole script | All statements |
| inside | Everything from caret | Current statement and all following |
| outside | Nothing | Stop (nothing runs) |
| outside | Whole script | All statements |
| outside | Everything below caret | Statements starting after the caret |
| selection | Exactly as one statement | Selected text sent as one command (help example: `BEGIN; UPDATE…; UPDATE…; COMMIT;` succeeds/fails together) |
| selection | Exactly as separate statements | Selected text split into statements, each executed separately |
| selection | Smart expand to script | If ≥1 statement border inside the selection, expand to whole statements and run that sequence; else run the selection verbatim |

"Outside" = blank line or comment (help wording). Highlighting of the range-to-run uses color key *Console//Statement to execute*. [Observed]

### 3.3 Resolution: "Smallest statement" vs "Smallest subquery or statement"

**Help prose (both offline 2024.3 and online 2026.3):** "Smallest subquery or statement … when the caret is inside a subquery, the whole statement is executed, including a subquery" and "Smallest statement … when the caret is inside a subquery, the subquery is executed." [Observed: `query-execution.txt` lines 23/25; raw HTML; online 2026.3 fetch]

**Evidence that the prose is swapped:**
1. *Screenshots on the same page.* All four inside-caret screenshots use one fixture: statement 0 `SELECT * FROM actor`; statement 1 a `SELECT … FROM actor a JOIN (SELECT * FROM film_actor WHERE film_id > 100) fa ON … GROUP BY …` with the caret inside the parenthesized subquery; statement 3 a trailing `SELECT COUNT(*)…`. In `db_query_exec_smallest_subquery_or_statement.png` the execution frame surrounds **only the inner subquery**. In `db_query_exec_smallest_statement.png` the frame surrounds **the whole outer statement 1**. [Observed: viewed images]
2. `db_query_exec_smallest_statement.png` and `db_query_exec_largest_statement.png` are **byte-identical** (same size 41,519 B, same MD5 `5a04b117…`). For this fixture Smallest and Largest statement coincide because there is only one statement level. [Observed: md5]
3. **DBE-6418** ("Execute smallest statement not working for MSSQL dialect…", 2018): the reporter expected *Smallest statement* with the caret on an inner `SELECT` to run only the inner select; it ran the outer statement. Resolution: Fixed, available in **2022.2.3**, by **adding the new option "Smallest subquery or statement"** (release blog: "We've added the Smallest subquery or statement option…"). I.e. the old option intentionally keeps running statements; the new option is the one that can pick subqueries. [Observed: YouTrack REST, blog]
4. Labels: a subquery is not a statement in DataGrip's model (statements come from `ScriptModel`; subqueries are query expressions within them), so "smallest statement" cannot select a subquery; "subquery or statement" explicitly can. [Inferred from labels + doc 06 §3.3]
5. The chooser screenshot (Ask) lists the inner subquery first, then the outer statement, then the whole script — the same two granularities. [Observed]

**Conclusion (what is known):** labels are correct, prose is swapped. *Smallest subquery or statement* → innermost subquery (falls back to statement). *Smallest statement* → smallest enclosing statement, which differs from *Largest statement* only when statements nest (PL/pgSQL `DO`/function bodies, T-SQL blocks, Oracle PL/SQL, MySQL compound statements). **Still unknown:** (a) which subquery kinds qualify (derived table, scalar subquery, `EXISTS`, CTE body, set-operation branch, `INSERT … SELECT` source) and whether correlated subqueries are skipped; (b) whether Smallest statement inside a `CREATE FUNCTION … $$ … $$` body picks the inner statement (bodies are dollar-quoted strings in PG, so probably not; doc 06). These remain doc 13 CARET-05/06/14/21/22/25..30. **Doc 13 CARET-02/03 expectations should be updated**: CARET-02 (Smallest statement, caret in innermost subquery) → whole statement; CARET-03 (Smallest subquery or statement) → the innermost subquery.

### 3.4 Generic / ANSI split mode

Only for files/consoles in the *Generic* or *SQL:2016* dialects (unsupported databases). [Observed: help]
- **Into valid ANSI SQL statements or by separator** (default): parse with the SQL:2016 grammar; where parsing fails, fall back to separator splitting.
- **Into ANSI SQL statements**: grammar only.
- **By statement separator**: split on the separator (`;` for Generic).
Generic differs from SQL:2016 in error highlighting (Generic suppresses most errors). Related: Settings › Database › Other › *Statement delimiter* (code generation) and *Default resolve mode for consoles*. [Observed: SO index page `database.other`] Splitter algorithm: doc 06 ALG-SQL-01.

### 3.5 Statement chooser

- Popup titled **Statements**; one row per candidate; bottom link **Customize** (opens Query Execution settings; help: "define whether you want to see the chooser or always run the statement under the caret"). [Observed: bundle `statement.chooser.header`, `statement.chooser.settings.link.text`, screenshot]
- Candidates for the FX above: (1) inner subquery `SELECT * FROM film_actor WHERE film_id > 100` (no parentheses), (2) the outer statement, (3) the whole script. The whole-script label is the **raw script text collapsed to one line**, starting with the leading comment (`-- Statement 0: Whole script SELECT * FROM actor; …`). Labels are truncated with "…" at popup width (not at a fixed char count). [Observed: screenshot]
- Moving the selection highlights the candidate's range with a full-width block highlight in the editor; Enter runs it; Esc cancels. "The suggestion list always contains an item for running all the statements." [Observed: help]
- In the screenshot the **second** item (statement) is preselected, not the first (subquery) — preselection appears to be the *statement* granularity. [Observed: one screenshot; Inferred as rule]
- Whether the last choice is remembered: [Unknown — doc 13 CHOOSE-07].

### 3.6 Attachment, context and resolve mode (what runs *where*)

- Console = SQL file in `Scratches and Consoles/Database Consoles/<ds>/` (stored under the IDE config `consoles` dir) attached to a data source; each console has its own session unless *Single session mode*. Files attach via the top-right data-source list; *Switch Data Source*, *Detach Data Source*. [Observed]
- `<schema>` selector sets the current database/schema; for PostgreSQL/Redshift/Greenplum it forms the **search path** (multi-select). Data source option *Switch schema*: Automatic (restore search path/current schema across restarts) / Manual / Disable. [Observed]
- Resolve mode Playground (consoles) vs Script (files) changes name resolution only, not execution (doc 06). [Observed]
- *Startup script* (data source option) runs on every new connection; IDE macros insertable. *Before connection* tasks; *Session Templates* override Authentication / Read-only / Startup script / Driver. [Observed]

### 3.7 Parameters / placeholders

**Detection.** "Depending on the database vendor, `?` is treated as a parameter." Further syntaxes come from regex patterns on Settings › Database › Query Execution › User Parameters. [Observed]

**Master switches** (doc 11 UP-1..3):
- *Enable in query consoles and SQL files* — off disables all patterns in scripts regardless of per-pattern scope.
- *Enable in string literals with SQL injection* — same for SQL injected into other languages' strings.
- *Substitute inside SQL strings* — also find parameters **inside SQL string literals** (`WHERE actor_id='${actor_id}'` → param only when on).

**Default patterns** (described in our words; order = priority) [Observed: help]:

| # | Shape | Name source | Example | Note |
|---|---|---|---|---|
| 1 | `?` followed by digits | the digits (capture group) | `?69` → "69" | numbered placeholders |
| 2 | `%` + word chars | whole match | `%xyz` | no capture group |
| 3 | `${` … `}` with no `$`,`{`,`}` inside | inner text | `${value}`, also `${}` (empty name) | template style |
| 4 | `$(` … `)` | inner text | `$(x)` | |
| 5 | `$` word `$` | the word | `$x1$` | collides with PG dollar-quote tags `$tag$` |
| 6 | `#` word `#` | the word | `#field_3#` | |
| 7 | `$` + (digits or identifier with dots), optional trailing `$`, **preceded by non-word or start**, followed by non-word or end | the identifier | `$ACCESS` yes, `V$ACCESS` no | protects Oracle `V$` views; overlaps PG `$1` |
| 8 | same as 7 with `#` | identifier | `#name` | overlaps T-SQL `#temp` tables |

Naming rule: "Values that start with a colon (`:`) or located in parentheses are treated as parameter names" — i.e. the first capture group is the name; for patterns without a group, the whole match. Custom example from help: a pattern for `%%winnerpl_2`-style tokens (two `%` then `word_digits`), which also shows patterns apply **inside string literals** when that switch is on. [Observed]

**Per-pattern scope** controls: *In scripts* (SQL files/consoles), *In literals* (injected SQL), *All languages* link (subset of languages). The table renders the scope as *everywhere / in scripts / in literals / disabled* and languages as *All* or a list. Rows are reordered with Alt+Up/Down, added/removed with Alt+Insert/Alt+Delete. [Observed: help, bundle `user.parameter.place.*`]

**Collecting values.**
- On execute, if parameters exist, DataGrip prompts: a **Parameters** dialog/pane with columns *Parameter* / *Value*, button **Execute** (mnemonic x), empty state "No parameters present". *View Parameters* (toolbar) toggles the pane (`Console.Jdbc.ToggleParameters`, also `ToggleParametersPanelAction`); a **Parameters** tab exists beside Output. [Observed: bundle]
- Values are kept **in memory**; with *Review parameters before execution* on, the dialog re-opens pre-filled; off → runs immediately with remembered values. [Observed: help]
- *Edit Parameter* action edits the parameter at caret. Color keys *Parameters//Parameter*, *Parameters//Parameter usage*. SQL completion/inlays know "Query Parameter" (`SqlQueryParametersProvider`). [Observed]
- **Substitution is textual**: help example replaces one `?` with `206, 'John', 'Smith'` inside `VALUES (?, '…')` — impossible with JDBC binding, so user-entered values are pasted into SQL text. [Observed: help; Inferred: mechanism] Whether native `?` is bound or substituted is not documented [Unknown — doc 13 PARAM-01/17].
- EPs: `database.parameterPatternProvider` (default pattern source), `database.consoleRunContextParametersTuner`; iteration via `ScriptModel$ParamIt`. [Observed: names]

### 3.8 Safety validators

- Registered as `database.queryValidator`: `SqlTypeInspection`, `SqlWithoutWhereInspection`, `SqlJoinWithoutOnInspection`; run right before sending when *Show warning before running potentially unsafe queries* is on. While typing, the same inspections highlight in the editor ("Notification when you type DELETE and UPDATE without WHERE"). [Observed]
- Messages [Observed: SqlBundle]: Delete without where "clears all data in the table"; Update without where "updates all table rows at once"; both "…because 'where' condition is always true"; join: "Unsafe 'join' clause inside 'delete': either 'on' or 'where' should reference the left table". Dialog text template: "Unsafe query: {0}".
- Help: "If you omitted the WHERE or WHERE TRUE clause intentionally, you can execute … by clicking Execute in the warning." The phrase suggests `WHERE TRUE` may count as an *intentional* marker, while the bundle has an "always true" message — whether `WHERE TRUE` is warned is **[Unknown — doc 13 UNSAFE-02/03]**.
- Dialog buttons available in the bundle as mnemonic HTML labels: **Execute**, **Execute All**, **Execute and Suppress**, **Skip**, **Skip All**. Most likely mapping: per-unsafe-statement confirmation when a script contains several — Execute (this one), Execute All (all remaining unsafe ones), Execute and Suppress (and stop asking — scope [Unknown]), Skip (do not run this one), Skip All. [Observed: labels; Inferred: mapping]
- "Run Script Anyway?" title — for run configurations whose scripts switch schema. [Observed: bundle]
- `sql.executionFlowAnalyzerProvider` supplies control/data-flow for constant-condition detection. [Observed]

### 3.9 Read-only enforcement

- Data source *Read-only* checkbox: "protect the data source from accidental data modifications… you cannot modify the data in the Data editor. **Data modifications might be possible in the query console if the driver does not support the read-only status.**" → primary mechanism is the JDBC read-only flag; grid editing is blocked by the IDE. [Observed: help]
- IDE-side checks with messages "%s to execute a statement that leads to modification of data during a read-only connection" (DML), "…a DDL statement…", "…a routine that may cause side effects…", "…a statement that may cause side effects…" — `SqlSideEffectsInspection` classifies; the `%s` prefix is a verb phrase (e.g. "Unable"/"Attempt") [Inferred]. MongoDB has a "Disable read-only mode" quick fix. [Observed]
- Session templates can set Read-only per session (e.g. a read-only template for production). [Observed]

### 3.10 Execution engine

[Observed: class names; Inferred: pipeline behavior]
- **Requests** (`DataRequest$…`): `QueryRequest`, `RawQueryRequest`, `ConditionQueryRequest` (grid WHERE/ORDER BY), `IsolatedQueryRequest` (separate connection), `UpdateRequest`/`InsertRequest`/`DeleteRequest`/`DmlRequest` (grid edits, doc 08), `CallRequest` (routines), `TxRequest`/`TxCommand` (commit/rollback/savepoint — "Failed to roll back to a savepoint"), `SchemaSwitchRequest`, `Cancel`, `Disconnect`, `CompositeRequest`. Context: `StatementContext`, `TxContext`, `Constraints`, `OutputHandler`, `CoupledWithEditor$ErrorNavigator`.
- **Auditors** (`DataAuditor`, `TxEvent`; `DataAuditors$Adapter/BusyCounter/PrintingAdapter`): `CancelProgressAuditor`, `MeasuringDataAuditor` (execution vs fetching time), `SessionLogger` (Output tab), `TxIsolationAuditor`, `TxMarkerAuditor`.
- **Remote JDBC**: drivers in a separate process (`RemoteJdbcServer`). Connection-level failure messages: "Unrecoverable error received, the connection is spoiled", "Connection is closed", "Unable to acquire connection", "{0}. Increasing memory via -Xmx may help. Aborting." [Observed: bundle]
- **Tracking while editing**: `TrackingDocScript` keeps ranges valid while typing during a long run; gutter shows per-statement status (spinner / success / error + duration; red square while cancelling). [Observed: help (gutter), names]

**Registry keys relevant to execution** [Observed: `DatabaseRegistryKeys.xml`]:

| Key | Default | Meaning |
|---|---|---|
| `database.result.set.limit` | 100 | Max result sets per query; beyond → "The maximum number of result sets per query is reached ({0})", "Skipping result set", "The query yielded {0} result sets, only {1} were retrieved." |
| `database.scrollable.result.sets` | true | Use scrollable cursors if the driver supports them (paging back) |
| `database.grid.paging.offset` | true | Page loading via LIMIT/OFFSET |
| `database.generic_dialect.limit` | true | Allow LIMIT on Generic dialect |
| `database.batch.size` | 2000 | JDBC batch size for statements |
| `database.disable.escape.processing` | true | JDBC `{fn …}` escapes disabled |
| `database.debug.enable` / `.trace` | true / false | SQL debugger / trace |
| `custom.cli.run.target.providers` | false | Dump/restore tools in Docker etc. (doc 10) |

Data Editor settings that affect execution: *Result set prefetch size*, *Limit page size to*, *Enable paging in in-editor results by default*, *Submit changes immediately* (doc 08/11).

### 3.11 Script error handling

- The bundle carries a button set **Retry / Ignore / Ignore All / Stop** (HTML mnemonic labels) alongside the Execute/Skip set. These match DataGrip's inline error prompt when one statement of a multi-statement run fails: *Retry* re-sends the failed statement, *Ignore* continues with the next, *Ignore All* continues and stops asking for this run, *Stop* aborts the rest. [Observed: labels; Inferred: flow, doc 13 ERR]
- Summary marks aborted runs: "…statements executed, F failed in T (aborted!)". "Execution cancelled" / "Cancelled" for user cancels. [Observed]
- Errors navigate to the offending editor offset (`ErrorNavigator`); error text is the server's message verbatim.

### 3.12 Cancellation (three-stage escalation)

[Observed: help `run-a-query`, bundle]
1. **Cancel Running Statements** (Ctrl+F2; "Cancel running and queued statements"): sends a cancel for the running statement and drops queued ones. Status "Cancelling…" / "Cancelling query…" → "Query canceled"; progress title "Cancel running statements in {ds} [{session}]"; gutter shows a red square.
2. **Slow cancel**: after ~10 s of idle cancelling, a balloon (group *Database cancelling running statements*, BALLOON) says cancelling takes too long; you may keep waiting or **Deactivate** the data source with all connections (link).
3. **Second press** of Cancel: dialog "[Cancelling of Running Statements Failed.] Deactivate the Data Source?" — "You can keep waiting or deactivate the data source with all the connections. Note: the current running statement might continue its execution on the server side." Buttons **Deactivate** / **Keep Waiting**.
- The earlier note "terminate the remote process" (help wording) = deactivating = closing the client connections; DataGrip does **not** issue `pg_terminate_backend`/`KILL` itself. [Observed: dialog text; Inferred: no server kill]
- *Deactivate* (Explorer/Services, Ctrl+F2 there) closes connections; "the query will continue its execution in the database". Cancel during connect → "Connection canceled". [Observed]
- Run configurations are stopped from the Run tool window (Stop, or Ctrl+F2 → choose process). [Observed]

### 3.13 Transactions

- **Tx** menu (console, grid and Services toolbars): *Transaction Mode* Auto ("Changes submitted to the database are auto-committed") / Manual ("Manually commit changes submitted to the database"); *Transaction Isolation*: Database Default ("Isolation level of the current database transaction"), Read Uncommitted ("may detect uncommitted changes"), Read Committed ("detects only committed changes"), Repeatable Read ("never detects concurrent changes"), Serializable ("Concurrent execution has the same effect as serial"). Button labels: `Tx: {0}`, `Tx Mode ({0})`, `Tx Isolation ({0})`. [Observed]
- Default from data source option *Transaction control* (mode + isolation). [Observed]
- Manual mode adds **Commit** and **Roll back** (console/Services) or **Submit and Commit** (Ctrl+Alt+Shift+Enter) and **Roll Back** (`RevertAndRollback`: also reverts pending grid edits) in grids. "Roll back the current transaction to undo changes.", "All changes will be rolled back." [Observed]
- Output/notifications: "{0} transaction mode ON", "transaction mode can not be changed", "transaction isolation level switched to {0}", "…can not be switched", "transaction committed|rolled back{suffix}", "Failed to {action}. The transaction is in progress "{name}". Commit or roll back it first and try again" (actions include *import data*, *perform introspection*, *export data*, *execute request*), "Locked by transaction: {0}". [Observed]
- Closing a console/connection with an open transaction: "Current connection will be closed. Commit current transaction?" (title *Current Transaction*). [Observed]
- **Single session mode**: all consoles and the data source share one session → one transaction across consoles; temporary objects visible in the tree. [Observed]
- Typed `BEGIN`/`COMMIT` in Auto mode: tracked by `TxMarkerAuditor` [Inferred] — doc 13 TX.

### 3.14 Results presentation

**Services tool window** [Observed: help]
- Tree: data source → sessions (green dot = connected; survive restart as empty nodes) → clients (consoles, files, data editors). Per client: Output tab, Result tabs, Parameters tab, Plan tab.
- Session actions: Rename Session (Shift+F6), Close Session, Close All Sessions (data source), Switch Session, Attach Session / Detach Session (confirmations "Detach '{0}' from '{1}' session?", "Close {n} sessions?" with *Keep Open*), Open in New Tab / Open Each in New Tab / Open Each Type in New Tab, Jump to Source (F4), Jump to Query Console (Ctrl+Shift+F10), Deactivate (Ctrl+F2), Restore Default Layout. Toolbar: Expand/Collapse All, Group By (Service Type / Service Groups), Filter, Add Service.
- Output-and-Results settings (doc 11 OR-1..8): show timestamps (off by default); show results in editor; title from comment + marker; when to pop Services (*For all output* / *For query output, errors, and result sets* / *For errors and result sets* / *For result sets* / *Never*); focus Services in window mode; new Services tab per session; stay on selected console's output pane.

**Result tabs**
- One tab per result set; default title = **qualified table name** of the source (help) or comment-derived title; "Always show qualified names…" and "Shorten datasource and object names in tab titles" settings plus *Shorten Tab Titles* toggle. Non-row statements write to Output only. [Observed]
- Reuse vs new tab (*Open results in new tab*, per Execute action); **Pin Tab** (right-click) keeps a reused tab. [Observed]
- Result toolbar (details in doc 08): paging (First/Prev Ctrl+Alt+Up/Next Ctrl+Alt+Down/Last, Change page size), Reload Page Ctrl+F5, Update Interval (auto-refresh), Cancel, Add/Delete Row, Revert Selected, Preview Pending Changes, Submit, Tx, Submit and Commit, Roll Back, Pin Tab, Data Extractors, Export Data, Copy to Database, Compare Data, View as (Table/Tree/Text + Transpose), options menu (Geo Viewer, Value Editor, Aggregate View, Paste Format, Reset View, **View Query**, **Copy Query to Console**, Open Data View Settings). [Observed]
- **Compare Data** between two result sets with **Tolerance** = max number of differing columns for two rows to still count as equal. [Observed]

**In-Editor Results** [Observed: help, bundle]
- Toggle per file (toolbar *In-Editor Results*, "Switch the results place") or globally (*Show results in editor*). Result sets render below the statement, stacked when several; each block can be collapsed with *Show Results / Hide Results*. Paging inside in-editor results is off unless *Enable paging in in-editor results by default*.

### 3.15 Output tab and message formats

Toolbar: Soft-Wrap, Scroll to End, Print, Clear All. Context menu: Copy, Copy Path/Reference, Compare with Clipboard, Search with Google, Fold Lines Like This, Pause Output, Clear All. Content: user and internal queries, errors, timestamps (optional), affected rows, durations, autocommit mode changes, session changes ("session changed from '{0}' to '{1}'"), connection lines ("Connecting to {0}… ({1})", "Connected to {0}", "Disconnected"). [Observed]

Message formats (ChoiceFormat in English; decoded) [Observed: bundle]:

| Event | Format (decoded) |
|---|---|
| No rows | `0 rows retrieved in {time}` |
| Rows | `{1 row \| N rows} retrieved starting from {first} in {time}` (+ optional "(execution: a, fetching: b)" in the help example) |
| Update count | `{1 row \| N rows} affected in {time}`; batch variant `N row(s) affected in {time}` |
| Tx | `transaction {committed \| rolled back}{suffix}` |
| Generic | `completed in {time}` |
| Run summary | `{prefix}: K[ of M] statements executed[, F failed] in T[ (aborted!)]` — "of M" only when K≠M; ", F failed" only when F>0 |
| Script summary | `Summary: K[ of M] statements executed[, F failed] in T[ (S symbols in file)][ (aborted!)]` |
| Process | `{name} process canceled` / `process finished` |

These are plural/optional fragments encoded with `{n,choice,…}` — a pattern Hacivat must express with real plural rules (doc 12, §6 here).

### 3.16 Notifications (relevant groups)

[Observed: `notificationGroup` declarations]

| Group | Display | Use |
|---|---|---|
| Database execution log | NONE (log only) | execution messages mirrored to Event Log |
| Database cancelling running statements | BALLOON | slow-cancel → Deactivate offer |
| Database queries that took much time | BALLOON | long-running query finished (likely when the IDE/console is not focused) [Inferred] |
| Database view / view log / refactoring log | BALLOON / NONE | explorer, refactorings |
| Update Table | BALLOON | grid/table updates |

### 3.17 Server output

- **Enable DBMS_OUTPUT** (Oracle, IBM Db2 LUW): data source option "by default for new sessions" plus console toggle `Console.DbmsOutput`; captured lines appear in Output. [Observed]
- PostgreSQL notices (`RAISE NOTICE`), SQL Server `PRINT`/low-severity `RAISERROR` — appear in Output as "query output" (help: "Errors and Messages in PostgreSQL"). [Observed: help wording; mechanism Inferred]

### 3.18 Execute to File

Right-click a SELECT → *Execute to File(s)* → choose format → **Export Data** dialog (extractor, output file/directory, add column/row header, add DDL, transpose, computed/generated columns, Excel "Output query to separate sheet", preview). Results stream to file without a grid. Full dialog: docs 08/10. [Observed]

### 3.19 Routine execution

*Run Routine… / Run Procedure… / Run Function…* opens **Execute Routine**: generated call text with a field per parameter; *Run from* checkbox chooses an existing file/console to place the call into. In Oracle this generates an anonymous block used for debugging (§3.22). [Observed]

### 3.20 Run configurations and running scripts

**Database Script configuration** [Observed: help]
- *Target data source / schema*: list of data sources/databases/schemas (dialect-dependent); a data-source target shows the default schema it will use. *Recent* button for recently used targets.
- *Script text*: inline SQL; *Script files*: list of files. Schema-switching statements inside a file show a warning icon.
- Common: *Name*, *Allow multiple instances* (default off → starting again offers to stop the running one), *Store as project file* (`.idea/runConfigurations`; else `workspace.xml`).
- *Before launch*: Run External tool, Run Another Configuration, Launch Web Browser, **Disconnect Data Source**; *Show this page*; *Activate tool window*.
- Errors: "Missing code to execute", "Missing files to execute", "No configured target data sources", "Missing target data sources", "Failed to perform before task: {0}". Tab title "Run '{0}'…". [Observed: bundle]
- Temporary configurations: created by Run on a file/drag-drop; limit 5 (Advanced Settings › Run/debug). [Observed]
- Output in **Run** tool window; *Pause Output* suspends display only. [Observed]
- Help: "maximum number of DML statements that you can run from an SQL file is 1000"; bundle: "Using batch mode, maximum number of INSERT/UPDATE/DELETE statements is {0}". Likely: consecutive DML are grouped into JDBC batches with a per-batch cap; the help sentence probably over-generalizes [Inferred — doc 13 RUNFILE-01].
- Multi-target: selecting several files and several targets runs every file against every target. [Observed: help "Run an SQL file for several data sources"]

### 3.21 Large files

- Files >20 MB open showing only the first 2.5 MB (help). These are the IntelliJ platform thresholds `idea.max.content.load.filesize` (20 000 KB) and `idea.max.intellisense.filesize` (2 500 KB): above the latter, code insight is disabled; above the former, the file is opened in a read-only/partial viewer. [Observed: help; IntelliJ Community `FileUtilRt`/`PersistentFSConstants`, Apache-2.0]
- Hint: "You can execute large scripts using run/debug configurations" — Database Script runs files streaming from disk without the editor. [Observed: bundle]

### 3.22 Explain plan, flame graphs, debugger

- Actions (group *Explain Plan*): **Explain Plan** (tree+table on a *Plan* tab), **Explain Plan (Raw)** (table of native output; "Technically, EXPLAIN <CURRENT_STATEMENT> or similar statement is executed"), **Explain Analyse**, **Explain Analyse (Raw)**. With ANALYZE "the statement is actually executed". Costs are "relative cost units"; startup vs total explained. [Observed]
- Plan tab: *Show Diagram* (Ctrl+Alt+Shift+U; "Graphical Explain Plan") and **Flame Graph** (`DbPlanFlameGraphsGroup`, "Build flame graph by property"; tab title "Flame Chart: {metric}"). Metrics: Total Cost, Startup Cost; for Analyse also Actual Total Time, Actual Startup Time (ms). Module `intellij.database.flameGraphs` depends on `intellij.profiler.common`. [Observed]

Per-dialect support [Observed: class names; Inferred: native command and raw-only status]:

| Dialect package | Provider | Model builder | Likely native source |
|---|---|---|---|
| postgres / postgresbase / greenplum / redshift | ✓ | ✓ (Pg, PgBase, GPlum, Rs) | `EXPLAIN (FORMAT …)` / `EXPLAIN ANALYZE` |
| mysql / mysqlbase | ✓ | ✓ (MysqlTree, MysqlBase) | `EXPLAIN` tabular / `EXPLAIN ANALYZE` tree (8.0.18+) |
| maria | ✓ | ✓ JSON | `EXPLAIN FORMAT=JSON` / `ANALYZE FORMAT=JSON` |
| mssql / sybase (ASE) | ✓ | ✓ XML (Ms), Ase | showplan XML / statistics XML; ASE showplan text |
| oracle | ✓ | ✓ | `EXPLAIN PLAN FOR` + `PLAN_TABLE` |
| db2 | ✓ | ✓ | explain tables |
| sqlite | ✓ | ✓ | `EXPLAIN QUERY PLAN` (id/parent/detail) |
| clickhouse | ✓ | ✓ | `EXPLAIN` variants |
| snowflake | ✓ | ✓ | `EXPLAIN USING TABULAR/JSON` |
| vertica | ✓ | ✓ | `EXPLAIN` text |
| derby, h2, hive, hsql, spark | ✓ | — | raw only (no tree) |

- **Debugger**: "Debug functionality is only supported for Oracle databases" (2024.3). Requires DEBUG CONNECT SESSION, DEBUG ANY PROCEDURE, access to `V$SESSION` and `DBMS_DEBUG`; Oracle 9+. Units: anonymous blocks, packages, procedures, functions, triggers. Workflow: compile with *Recompile › With DEBUG option*; debug via anonymous block (from Execute Routine) or a DML that fires the trigger; breakpoints, step in/out, watches, evaluate. Graceful vs non-graceful stepping mode. [Observed]

### 3.23 Execution action inventory

[Observed: keymap index (doc 11 §3.17), `DatabasePlugin.xml`, bundle]

| Action id | Label | Notes |
|---|---|---|
| `Console.Jdbc.Execute` | Execute | "Execute queries in console"; `RunQueryAction$Alt1`; reuses the shortcut of `Console.Execute.Multiline` (Ctrl/Cmd+Enter), which grid *Submit* also reuses (`use-shortcut-of`) → Ctrl+Enter is context-dependent |
| `Console.Jdbc.Execute.2` / `.3` | Execute (2) / (3) | "alternative" / "extra"; `RunQueryAction$Alt2/$Alt3`; no default shortcut |
| `Console.Jdbc.Cancel` | Cancel Running Statements | Ctrl+F2 |
| `Console.Jdbc.Debug` | Debug | Oracle only |
| `Console.Jdbc.ExplainPlan`, `.ExplainPlan.Raw`, `.ExplainAnalyse`, `.ExplainAnalyse.Raw` | Explain … | group `Console.Jdbc.ExplainGroup` |
| `Console.Jdbc.ToggleParameters`, `Sql.EditParameter` | View Parameters / Edit Parameter | |
| `Console.Jdbc.ChooseDataSource`, `.ChooseSchema`, `.ChooseSession` | Attach Data Source / Set Current Schema / Switch session | |
| `Console.DbmsOutput` | Enable DBMS_OUTPUT | |
| `Console.History.Browse` | Browse Query History (Ctrl+Alt+E) | doc 10 |
| `Console.Toggle.Notebook.Mode` | In-Editor Results | |
| `Console.Toggle.Shorten.Tab.Titles` | Shorten Tab Titles | |
| `Console.Transaction.Commit`, `.Rollback`, `.RevertAndRollback` | Commit / Roll back / Roll Back | last also reverts grid edits |
| `Console.TableResult.SubmitAndCommit` | Submit and Commit | Ctrl+Alt+Shift+Enter |
| `Console.TableResult.ChangeIsolation` | Tx Isolation | |
| `Console.TableResult.ShowDumpDialogAction` | Execute to File(s) | |
| `QueryExecution.Settings` | Open Query Execution Settings… | console toolbar |
| `DatabaseView.ExecuteRoutine` | Run Routine / Procedure / Function… | |
| `DatabaseView.RunExtensionScriptGroup` | Database Scripts | extension scripts (doc 09/10) |
| `Jdbc.OpenConsole.New`, `.Any`, `Jdbc.OpenEditor.Console` | New Query Console (Ctrl+Shift+Q) / Jump to Query Console (Ctrl+Shift+F10) / Open Query Console (F4) | |
| keymap groups | Database › Execution, Sessions, Console Toolbar; menu groups *Transactions*, *Execution* | |

### 3.24 Per-DBMS execution mechanics Hacivat must implement

[Inferred: protocol knowledge; DataGrip's behavior here is JDBC-mediated]

| Concern | PostgreSQL (tokio-postgres) | MySQL/MariaDB (mysql_async) | SQL Server (tiberius) | SQLite (rusqlite) |
|---|---|---|---|---|
| Multi-statement "as one" | simple query protocol (no params) | `CLIENT_MULTI_STATEMENTS` | batch text natively | `execute_batch` (no results) |
| Cancel running | CancelRequest via `cancel_token()` | `KILL QUERY <id>` on side connection | TDS attention | `InterruptHandle::interrupt()` |
| Terminate | `pg_terminate_backend(pid)` | `KILL <id>` | `KILL <spid>` | close connection |
| Tx state | ReadyForQuery status I/T/E | `SERVER_STATUS_IN_TRANS` | `@@TRANCOUNT` / ENVCHANGE | `sqlite3_get_autocommit()` |
| Read-only session | `default_transaction_read_only` / `SET SESSION CHARACTERISTICS … READ ONLY` | `SET SESSION TRANSACTION READ ONLY` | none (permissions; app-level) | `PRAGMA query_only=1` / open read-only |
| Server messages | NoticeResponse | warnings (`SHOW WARNINGS`) | INFO tokens | none |
| Paging | portal + `Execute(max_rows)` | streaming | streaming tokens | step iterator |
| Multiple result sets | yes (simple) | yes | yes | n/a |
| Error position | `position` (1-based char) | message text | line number | message |
| Explain | `EXPLAIN (FORMAT JSON[, ANALYZE, BUFFERS])` | `EXPLAIN FORMAT=JSON` / `EXPLAIN ANALYZE` | `SET SHOWPLAN_XML ON` / `SET STATISTICS XML ON` | `EXPLAIN QUERY PLAN` |

---

## 4. Algorithm specifications

### ALG-EXEC-01: Caret/selection → execution range(s)
- **Problem:** Map (caret | selection, policy) to the exact list of statements to send.
- **Inputs / outputs:** document; statements from ALG-SQL-01 (with batch ids and nesting); CST (for subqueries); caret offset(s) or selection; policy `{inside, outside, selection}` → `Vec<ExecUnit{text, range, batch_id, repeat}>` or `AskUser(Vec<Candidate>)`.
- **Algorithm:**
  1. If selection non-empty:
     - `AsOne` → one unit = selected text verbatim (strip one trailing delimiter only if the driver rejects it).
     - `AsSeparate` → run ALG-SQL-01 on the selected text with lexer state carried in from document start (active MySQL `DELIMITER`, inside-batch status) → units.
     - `SmartExpand` → if any statement boundary lies strictly inside the selection, expand start to the start of the statement containing selection start and end to the end of the statement containing selection end, then split; else one unit = selection verbatim.
  2. Else find the top-level statement `S` whose **smart range** (statement plus attached leading comments and trailing delimiter on the same line) contains the caret.
  3. If none → `outside`: Nothing → `[]` + hint "Nothing to run"; WholeScript → all; BelowCaret → statements whose start > caret.
  4. If inside:
     - **SmallestSubqueryOrStatement** → walk from the CST leaf at caret upward; the first *query expression* node that is a subquery (derived table, scalar/`IN`/`EXISTS` subquery, CTE body, set-operation operand in parentheses) and is **independently executable** (no outer references; CTE body only if it references no sibling CTE — else include the needed WITH items) → unit = node text without enclosing parentheses; else fall through to SmallestStatement.
     - **SmallestStatement** → innermost *statement* node containing the caret that the driver can run alone: nested statements inside compound blocks are candidates only if the dialect allows executing them standalone (T-SQL `BEGIN…END` inner statements: yes; PL/pgSQL body inside `$$`: no — it is a string; Oracle PL/SQL block inner statement: no, variables) → otherwise `S`.
     - **LargestStatement** → `S`.
     - **LargestStatementOrBatch** → T-SQL/Sybase: all statements of the caret's batch as **one** batch unit; else `S`.
     - **WholeScript** → all; **EverythingFromCaret** → `S` and all following.
     - **Ask** → ALG-EXEC-02.
  5. Expand `GO n` repeats (unit repeated n times); drop client-only commands (`DELIMITER`, `GO`, SQL*Plus `SET`/`PROMPT`) after applying their effect.
  6. Multiple carets: compute per caret, union by range, keep document order, deduplicate. [Hacivat decision; DataGrip Unknown]
- **Data structures:** `StatementIndex` (interval tree over smart ranges), CST with node kinds `Statement | QueryExpr | Subquery | CteBody | Block`.
- **Complexity:** O(log n) statement lookup + O(depth) CST walk.
- **Edge cases (exhaustive):** caret exactly at statement start / end / after `;` / in trailing whitespace on the same line / on the next line; caret inside attached leading comment (inside) vs comment separated by blank line (outside); caret in empty statement `;;`; unterminated last statement; caret inside string literal or quoted identifier (still inside S); caret inside dollar-quoted body (Smallest statement = `CREATE FUNCTION`); correlated subquery (not standalone → statement); subquery in `VALUES` row; `INSERT … SELECT` source; `UNION` without parentheses (branch is not a subquery → statement); `EXISTS(SELECT …)`; lateral subquery referencing earlier FROM items (correlated → statement); CTE referencing previous CTE; recursive CTE body (never standalone); `EXPLAIN SELECT` with caret in SELECT (inner statement executable — choose whole EXPLAIN for Smallest statement); `CREATE VIEW … AS SELECT` (SELECT runnable standalone for SmallestSubquery? — it is a query expression, not a subquery: run whole CREATE); selection spanning half a string literal / comment; selection only whitespace (→ Nothing to run); selection across `GO` boundary with AsOne (send as-is; T-SQL rejects GO → warn); `DELIMITER //` active in AsSeparate; read-only/unsafe checks apply per produced unit.
- **Test cases:**
  - FX-CARET-01 caret inside `(SELECT * FROM film_actor WHERE film_id > 100)`: SmallestSubqueryOrStatement → `SELECT * FROM film_actor WHERE film_id > 100`; SmallestStatement → whole statement 1; Largest → whole statement 1; Everything from caret → statements 1 and 3; Whole → 0,1,3. (Matches help screenshots.)
  - `SELECT a FROM t WHERE x IN (SELECT y FROM u WHERE u.k = t.k)` caret in inner → SmallestSubquery → whole statement (correlated).
  - T-SQL `SELECT 1\nGO\nSELECT 3; SELECT 4;\nGO` caret on `SELECT 4` → LargestOrBatch → `SELECT 3; SELECT 4;` as one batch; Largest → `SELECT 4`.
  - Caret on blank line, outside=Nothing → `[]` + "Nothing to run".
  - Selection `SELECT 1; SEL` with SmartExpand → `SELECT 1;` and the full second statement.
- **Prior art:** [Observed: settings, help, screenshots, DBE-6418, `ScriptModel$SmartRange/StrictRange/ChosenRange`].
- **Used by:** Hacivat; MCP tool `execute_at_cursor` (agents pass offset + policy).

### ALG-EXEC-02: Statement chooser candidates and presentation
- **Problem:** Build an unambiguous candidate list when policy = Ask.
- **Algorithm:**
  1. Candidates in order: innermost executable subquery (if any, per ALG-EXEC-01 rules) → smallest statement (if ≠ previous range) → largest statement / batch (if ≠) → everything from caret (only if it adds statements) → whole script (always).
  2. Deduplicate identical ranges keeping the earliest (most specific) label.
  3. Label = range text with all whitespace runs collapsed to one space, leading comments **included** (DataGrip does), cut at popup width with "…"; screen readers get label + "N statements".
  4. Preselect the *statement* candidate (DataGrip screenshot) — Hacivat: configurable, default statement.
  5. On selection change, highlight the range (Statement-to-execute color). Enter runs; Esc closes; "Customize" opens settings. Number keys 1..9 quick-pick [Hacivat addition]. Remember last kind per file for preselection [Hacivat addition].
- **Edge cases:** single candidate (only whole script when caret is outside? — chooser is not shown for outside policies); caret in statement of a one-statement script (statement == whole script → one item; still show? DataGrip always shows "all statements" → show 1 item or execute directly — Hacivat: execute directly); very long single-line SQL; RTL/Unicode text in labels; label containing control chars (replace with space).
- **Test cases:** FX-CARET-01 → 3 items [subquery, statement, script], preselected #2; single statement script → no popup; T-SQL batch fixture → 4 items incl. batch.
- **Prior art:** [Observed: help + `db_query_exec_ask_what_to_execute.png`].
- **Used by:** Hacivat.

### ALG-EXEC-03: Parameter detection and substitution
- **Problem:** Find user parameters in units, collect values, and produce executable text/bindings.
- **Inputs / outputs:** units, dialect, pattern table (ALG-EXEC-09), master switches, remembered values → `(final_text, binds)` per unit.
- **Algorithm:**
  1. Tokenize (ALG-SQL-02). Regions = code tokens outside comments; plus string-literal interiors iff *Substitute inside SQL strings*; never quoted identifiers.
  2. Native placeholders per dialect → **bind** parameters: `?` (except PG JSON operators `?`, `?|`, `?&` when followed by an operand context), PG `$n` outside dollar-quoted bodies, `:name` (Oracle/SQLite named), never T-SQL/MySQL `@var`.
  3. Apply user patterns (ALG-EXEC-09) over remaining regions.
  4. Group by name; show the Parameters dialog if any value unknown or Review is on; prefill remembered values keyed `(data_source, name)`.
  5. User-pattern parameters: **text substitution**; native: bind (typed by resolved column type, else text). Hacivat: offer per-parameter "as literal" toggle that quotes/escapes.
  6. Record final text in history/log; mask values of parameters named like secrets.
- **Edge cases:** `?|`/`?&`/`?` JSONB operators; `::type` casts and `a[1:2]` slices vs `:name`; `@var`; `$1` in function bodies; `#temp` vs `#name` pattern; `$tag$` dollar quotes vs pattern 5; `${}` empty name; parameters in comments; same name across units (one value); empty string vs NULL (explicit NULL toggle); value with quotes in text mode; Unicode names (`:müşteri`) with Unicode `\w`; Turkish dotted/dotless i in names compared case-sensitively.
- **Test cases:** doc 13 PARAM-01..20; `SELECT ${col} FROM t WHERE id='${id}'` switch off → {col}; on → {col,id}; `V$SESSION … $ACCESS` → {ACCESS}; `'{"a":1}'::jsonb ? 'a'` → {}.
- **Prior art:** [Observed: help user parameters & examples; `ScriptModel$ParamIt`].
- **Used by:** Hacivat; Karagöz receives final SQL + binds and audits final text (masked).

### ALG-EXEC-04: Result-tab title
- **Problem:** Name result tabs from preceding comments or the source object.
- **Algorithm:** (1) If *Create title from comment* on: take the comment block immediately preceding the statement within its smart range (no blank line). If marker non-empty, find marker; title = text after it to end of line/comment; if marker absent → no comment title. If marker empty, title = comment text after `--` / `/*` (strip `*/`, trim, collapse whitespace, first non-empty line). (2) Else / empty → qualified source table name (single-table SELECT), shortened if *Shorten* settings; else "Result N". (3) Multiple result sets from one statement → suffix `(2)`, `(3)`. (4) Truncate display, full title in tooltip.
- **Edge cases:** comment inside the statement; trailing comment on previous statement's line (belongs to previous); hints `/*+ … */`; doc comments `/** */`; marker appearing twice (first); Unicode/Turkish (no case change); comment longer than 1 line; title collision with pinned tab names (allow duplicates).
- **Test cases:** doc 13 TITLE-01..06.
- **Used by:** Hacivat.

### ALG-EXEC-05: Unsafe-query validator (pre-execution)
- **Problem:** Warn before destructive statements, fast and offline.
- **Algorithm:**
  1. UPDATE/DELETE (incl. MySQL multi-table, T-SQL `DELETE … FROM … JOIN`, PG `USING`): no WHERE → `NoWhere`; WHERE constant-true by folding over literals (`1=1`, `TRUE`, `'a'='a'`, `NULL IS NULL`, `NOT FALSE`) → `AlwaysTrue`; `WHERE CURRENT OF` → OK.
  2. DELETE with JOIN whose ON/WHERE never references the target table → `JoinWithoutOn`.
  3. Optional (off by default): `DROP`/`TRUNCATE` non-temp; `ALTER … DROP COLUMN`; MySQL `UPDATE … LIMIT` without ORDER BY.
  4. Skip temp tables when configured.
  5. Collect warnings for all units; if any and the setting is on → dialog per unsafe unit: [Execute] [Execute All] [Execute and Suppress] [Skip] [Skip All] [Cancel]. Skip removes the unit from the queue; Suppress records a suppression for the session/file.
- **Edge cases:** DML in routine bodies at creation (no warn); CTE-wrapped DML; `WHERE id IN (SELECT …)`; MERGE; grid-generated DML; `UPDATE t SET a=a` with WHERE; parameters in WHERE (`WHERE id = ?` → OK); WHERE referencing only a parameter that is constant (`WHERE :p = :p`) → AlwaysTrue after substitution? (evaluate after ALG-EXEC-03 text substitution).
- **Test cases:** doc 13 UNSAFE-01..10.
- **Used by:** Hacivat (UX); **Karagöz** (org policy "block UPDATE/DELETE without WHERE on production", independent of client settings).

### ALG-EXEC-06: Read-only side-effect classification
- **Problem:** Decide whether a unit may modify state.
- **Algorithm:** classify: `Read` (SELECT, SHOW, EXPLAIN w/o ANALYZE, DESCRIBE, VALUES, `COPY … TO STDOUT`), `Lock` (`FOR UPDATE/SHARE`), `Dml` (INSERT/UPDATE/DELETE/MERGE/COPY FROM/SELECT INTO/LOAD DATA/`EXPLAIN ANALYZE <dml>`, data-modifying CTE), `Ddl` (CREATE/ALTER/DROP/TRUNCATE/GRANT/REVOKE/COMMENT/RENAME), `Routine` (CALL/EXEC/DO, `SELECT f()` with non-immutable f per catalog volatility), `Tx`, `Session` (SET/USE), `Unknown` (parse failure, dynamic SQL → treat as write). Multi-statement unit → max severity. Message map: Dml→"modification of data", Ddl→"DDL statement", Routine→"routine that may cause side effects", Unknown/Lock→"statement that may cause side effects".
- **Edge cases:** `nextval`, `pg_terminate_backend`, `setval` in SELECT; `SELECT … INTO #tmp` (temp); `SET @x := (SELECT …)`; comments/hints only; Oracle `SELECT … FOR UPDATE`; `VACUUM`/`ANALYZE` (maintenance → Ddl-level); `LISTEN/NOTIFY` (Routine).
- **Test cases:** doc 13 RO-01..
- **Used by:** Hacivat (pre-check + DB-level read-only transaction), **Karagöz** (authoritative).

### ALG-EXEC-07: Execution queue, cancellation and auditor pipeline
- **Data structures:** `Session { conn, queue: VecDeque<Request>, running: Option<Running{cancel, started_at, unit_id}>, tx: TxState }`; `trait Auditor { on_start, on_result_set, on_rows, on_update_count, on_notice, on_error, on_tx, on_finish }`.
- **Algorithm:**
  1. Enqueue units on the client's session; gutter "queued".
  2. Worker: pop; `on_start`; execute with cancel token; for each result set read up to prefetch/page size, hand to grid (doc 08), keep cursor for paging; stop after `result_set_limit` (100) with "maximum number … reached" and "yielded N, only M retrieved"; collect update counts and notices (poll DBMS_OUTPUT after statement when enabled).
  3. Error → ALG-EXEC-12.
  4. Cancel → ALG-EXEC-11.
  5. Tx tracking → ALG-EXEC-15.
  6. `on_finish`: Output line (ALG-EXEC-10), history record (doc 10), gutter status + duration; after multi-statement runs, summary.
- **Edge cases:** cancel during connect; statement producing rows + update counts; infinite result sets; notice floods (rate-limit); connection lost mid-tx; user runs another statement while busy (enqueue, show position); session shared by several clients (single session mode — queue is per session, not per console).
- **Test cases:** cancel `SELECT pg_sleep(60)` within 1 s → "Query canceled", session reusable; 3 statements, error in #2, Ignore → "2 of 3 statements executed, 1 failed in …".
- **Used by:** Hacivat; **Karagöz** (same auditor trait server-side, audit per statement).

### ALG-EXEC-08: Explain plan normalization and flame graph
- **Inputs / outputs:** statement, dialect, analyze flag → `PlanNode{op, object?, index?, est_rows, act_rows?, startup_cost, total_cost, act_startup_ms?, act_total_ms?, loops?, extra, children}`.
- **Algorithm:**
  1. Provider builds the EXPLAIN command (table §3.24); dialects without a builder → Raw only.
  2. Parse native output (JSON walker, XML walker, id/parent row-tree, indented-text tree).
  3. Self value = node metric − Σ children metric (clamp ≥0); PG actual time × loops.
  4. Views: Raw table; tree table; diagram; flame graph with metric selector (Total Cost, Startup Cost, Actual Total Time, Actual Startup Time — the latter two only for Analyse).
  5. ANALYZE of DML: Hacivat wraps in a transaction and rolls back by default (DataGrip executes for real).
- **Edge cases:** InitPlan/SubPlan/CTE scans referenced multiple times; parallel workers; MySQL `nested_loop` arrays; MSSQL multiple statements per showplan; Oracle PLAN_TABLE missing/privileges; plans for parameterized statements (PG16+ `GENERIC_PLAN`); huge plans (virtualize); negative self cost from rounding; zero total (flame width min 1px).
- **Test cases:** PG JSON with Hash Join → Σ self = root total; SQLite EQP rows → tree; Derby → Raw only, flame graph disabled.
- **Prior art:** [Observed: providers/builders, flame graph module, help]; pev2 (PostgreSQL license).
- **Used by:** Hacivat; Karagöz treats EXPLAIN ANALYZE as the underlying statement for permission checks.

### ALG-EXEC-09: User-parameter pattern table and name extraction
- **Problem:** Apply an ordered list of user regexes with scopes and languages, without double-matching.
- **Inputs / outputs:** text region list (with kind `code | sql_string | injected_literal`), host language, patterns `[{regex, in_scripts, in_literals, languages: All|Set}]`, switches → `Vec<ParamOccurrence{range, name, pattern_idx}>`.
- **Algorithm:**
  1. Filter patterns: if region is script code/string → requires switch *Enable in consoles and files* and `in_scripts`; if injected literal → requires *Enable in string literals* and `in_literals`; language must be in `languages`. A pattern with neither flag is *disabled*.
  2. For each region, scan left to right; at each position try patterns in table order; the first match wins; advance past it (no overlaps). Use Unicode-aware regex (`\w` includes letters of all scripts).
  3. Name = capture group 1 if the regex has a group (and it participated); else if the match starts with `:`, the rest; else the whole match.
  4. Reject matches fully inside a native construct (dollar-quote tag, `::` cast, JSON operator) by checking lexer token kinds at the match start.
- **Edge cases:** empty name (`${}`) → parameter with empty name (DataGrip allows; Hacivat shows "(unnamed)"); pattern that matches empty string (reject pattern at settings time); catastrophic backtracking (use a linear-time engine — Rust `regex` lacks look-behind: compile look-behind patterns 7/8 to an equivalent manual boundary check, or use `fancy-regex` with a time budget); patterns 5 vs 7 on `$x$` (5 wins by order); overlapping `%{2}…` custom pattern with default `%\w+` (order decides); same name with different patterns (one value); names differing only by case (distinct).
- **Test cases:** `$x1$` → name `x1` via pattern 5; ` $ACCESS ` → `ACCESS` via 7; `V$ACCESS` → none; `#field_3#` → 6; `?69` → `69`; `${}` → empty; custom `%{2}(\w+_\d+)` before default on `'%%winnerpl_2'` with strings on → `winnerpl_2`.
- **Prior art:** [Observed: help pattern list].
- **Used by:** Hacivat.

### ALG-EXEC-10: Output log message formatting
- **Problem:** Produce the Output lines/summaries with correct plurals and optional fragments in any locale.
- **Algorithm:** each event → message id + named args (`rows`, `first`, `duration`, `exec_ms`, `fetch_ms`, `executed`, `total`, `failed`, `symbols`, `aborted`). Fluent selectors: `{ $rows -> [one] … *[other] … }`; optional fragments as separate terms selected by booleans (`total != executed`, `failed > 0`, `aborted`). Durations: < 1 s → "N ms"; < 60 s → "N s M ms"; else "N m S s" (localized units via ICU4X). Timestamp prefix `[yyyy-MM-dd HH:mm:ss]` when OR-1 on (invariant format in copied logs).
- **Edge cases:** 0 rows ("0 rows retrieved in …", no "starting from"); paging start > 1 ("starting from 501"); counts ≥ 2^31; negative/unknown update counts (driver -1 → "completed in"); Turkish has no singular/plural distinction for counts after numerals ("1 satır", "5 satır") — still route through plural rules.
- **Test cases:** (1, en) → "1 row retrieved starting from 1 in 131 ms"; (executed 2, total 3, failed 1) → "2 of 3 statements executed, 1 failed in 1 s 20 ms"; (3,3,0,aborted) → "3 statements executed in … (aborted!)".
- **Used by:** Hacivat; Karagöz audit summaries.

### ALG-EXEC-11: Cancel escalation state machine
- **States:** `Idle → Running → Cancelling(t0) → SlowCancel → Deactivating → Idle`.
- **Algorithm:**
  1. Cancel pressed in Running: send driver cancel; clear queue (mark cancelled); state Cancelling(t0); gutter red square.
  2. If finished/errored with cancellation → "Query canceled"; Idle.
  3. If now − t0 ≥ 10 s → balloon "Cancelling takes too long… [Deactivate]"; state SlowCancel.
  4. Cancel pressed again in Cancelling/SlowCancel → modal "Deactivate the data source?" [Keep Waiting] [Deactivate] (+ Hacivat: [Terminate on server] where supported: PG `pg_terminate_backend`, MySQL `KILL`, MSSQL `KILL` — needs privileges, uses a side connection).
  5. Deactivate → close all connections of the data source; mark running statements "Disconnected"; warn they may continue server-side.
  6. Cancel during Connecting → abort handshake → "Connection canceled".
- **Edge cases:** driver without cancel (SQLite busy in C call → interrupt; else close); cancel arrives after completion (ignore); cancel in manual-tx mode (tx remains open, may be aborted state on PG → show "transaction aborted, roll back"); multiple consoles sharing a session (cancel affects all queued from that session); network partition (cancel socket can't reach → only Deactivate).
- **Test cases:** pg_sleep(60) cancel → canceled < 1 s; MySQL `SELECT SLEEP(60)` → KILL QUERY; blocked lock wait with cancel ignored → after 10 s balloon; second press → dialog.
- **Prior art:** [Observed: bundle messages, help].
- **Used by:** Hacivat; Karagöz must accept cancel/terminate commands only from the session owner (or admins).

### ALG-EXEC-12: Script error handling policy
- **Problem:** Decide what happens to remaining units when one fails.
- **Algorithm:** policy per run: `Ask` (default for interactive multi-unit runs), `StopOnError` (default for run configurations [Inferred]), `IgnoreAll`. On error with Ask: show inline prompt [Retry][Ignore][Ignore All][Stop]; Retry re-sends the same unit (after optional edit); Ignore → next; Ignore All → set policy IgnoreAll; Stop → drop queue, summary "(aborted!)". In manual-tx mode on PG after an error, the tx is aborted: Ignore would make all following fail — offer "Roll back and continue". Single-unit runs never prompt.
- **Edge cases:** error in `GO n` repeated batch (stop remaining repeats); error inside AsOne multi-statement text (server-side partial execution — report driver's update counts); errors as warnings (MySQL warnings, T-SQL severity ≤10 → not errors); connection loss (no Retry unless reconnect).
- **Test cases:** 3 statements, #2 fails, Ignore → summary "2 of 3 … 1 failed"; Stop → "1 of 3 … 1 failed … (aborted!)".
- **Used by:** Hacivat; Karagöz (policy for unattended scripts).

### ALG-EXEC-13: Services/results auto-show policy
- **Inputs:** OR-5 level, OR-6 focus, OR-8 stay-on-selected, event kind (`ResultSet | Error | QueryOutput | IdeLog`), originating console, currently selected console.
- **Algorithm:** show panel iff level ≥ required(kind) where `ResultSet` needs ≥ *For result sets*, `Error` ≥ *For errors and result sets*, `QueryOutput` ≥ *For query output, errors, and result sets*, `IdeLog` = *For all output*; *Never* shows nothing. Switch the panel's selected client to the originating console only if OR-8 off or origin == selected. Move keyboard focus only if panel is floating/windowed and OR-6 on.
- **Edge cases:** in-editor results mode (results go inline, panel only for errors/output); background run-configuration output (Run tool window instead); two consoles finishing simultaneously.
- **Test cases:** level=*For result sets*, statement errors → panel not shown, error highlighted in editor; OR-8 on, console_1 finishes while console_2 selected → selection stays.
- **Used by:** Hacivat.

### ALG-EXEC-14: Run configuration multi-target execution
- **Inputs:** targets `[DataSource(+db/schema)]`, script text, files, before-launch tasks, allow-multiple flag.
- **Algorithm:** validate (no code → "Missing code to execute"; no targets → "No configured target data sources"); run before-launch tasks in order (Disconnect Data Source closes the target's connections); for each target (sequentially) open a dedicated session, set context schema, stream each file through the splitter (doc 06) without loading into an editor, execute with batching of consecutive DML (batch size 2000, cap per batch), apply ALG-EXEC-12 policy, log to Run tool window; warn before start if a file contains schema-switching statements ("Run Script Anyway?").
- **Edge cases:** target unreachable (skip/stop?); same file twice; huge files (streaming; memory bound); file encoding/BOM; `USE`/`search_path` changes persist to following files on the same target; parallel instance when not allowed (offer stop & restart); output paused.
- **Test cases:** 2 targets × 2 files → 4 runs logged in order; schema switch present → warning dialog.
- **Used by:** Hacivat; Karagöz (scheduled scripts, later).

### ALG-EXEC-15: Transaction state tracking and Tx UI
- **Algorithm:** `TxState{mode: Auto|Manual, isolation: Default|RU|RC|RR|S, open: bool, statements: u32, failed: bool}`. Mode switch: apply on the connection (`autocommit` flag); if a tx is open → "Failed to change… The transaction is in progress. Commit or roll back it first". After every request update `open` from protocol status (table §3.24). UI: toolbar Tx label `Tx: Auto`/`Tx: Manual`; badge with pending statement count; Commit/Roll back visible only in Manual; on close/disconnect/switch data source with open tx → prompt Commit / Roll back / Cancel. Log "transaction committed/rolled back".
- **Edge cases:** user types `BEGIN` in Auto (tx open in Auto mode → show badge anyway); DDL auto-commit in MySQL/Oracle (tx closes implicitly); PG aborted tx state; isolation change mid-tx (refuse); shared session (badge on all clients); savepoints.
- **Test cases:** doc 13 TX family.
- **Used by:** Hacivat; Karagöz (idle-in-transaction timeout policy).

### ALG-EXEC-16: Large-file execution path
- **Algorithm:** if file size > `max_editor_bytes` (Hacivat default 20 MB): open read-only preview of first 2.5 MB with banner "Run as script"; execution uses ALG-EXEC-14 streaming splitter with a 64 KB rolling buffer that preserves lexer state across chunk boundaries; progress = bytes consumed / file size; no per-statement gutter.
- **Edge cases:** statement spanning chunk boundary; dollar-quoted body > buffer (grow buffer); multibyte UTF-8 split at chunk boundary; COPY FROM STDIN data blocks (PG dump files) — pass-through mode; `\` meta-commands of psql (unsupported → error with hint to use native restore, doc 10).
- **Test cases:** 100 MB file of INSERTs → constant memory; statement crossing boundary splits correctly.
- **Used by:** Hacivat.

### ALG-EXEC-17: In-editor result anchoring
- **Problem:** Keep inline result blocks attached to their statements while the user edits.
- **Algorithm:** anchor each block to the statement's end offset via a range marker that moves with edits; if the statement text changes, mark block "stale" (dim) but keep; if the statement is deleted, remove block; multiple result sets stacked in one block with tabs; collapse state per block; paging off unless setting.
- **Edge cases:** re-execution replaces block content; undo restoring statement (restore block if in undo window); statements merged/split by an edit; very large results (cap visible rows, link "open in Services").
- **Test cases:** edit inside statement → block stale; delete statement → block removed.
- **Used by:** Hacivat.

---

## 5. Implications for Hacivat / Karagöz / Perde

1. **Execution core crate** `db-exec`: per-session actor (tokio task) with queue + auditor trait (ALG-EXEC-07); drivers tokio-postgres, mysql_async, tiberius, rusqlite. Avoid sqlx for console execution (hides multi-result sets, notices, cancellation).
2. **Policies**: three execute bindings, each with the policy triple; **name options unambiguously** and ship screenshots/examples in our docs that match behavior (DataGrip's swapped help shows how easily this goes wrong). Hacivat defaults (matching the DataGrip screenshot): inside=**Ask**, outside=Nothing, selection=Exactly as separate statements.
3. **Parameters**: bind mode default; text-template mode compatible with DataGrip's 8 patterns (ALG-EXEC-09); MCP tools bind-only.
4. **Cancel**: implement the escalation, but add real server-side terminate where supported (DataGrip only disconnects).
5. **Safety by default**: unsafe warnings on; read-only enforced with DB-level read-only transactions, not just driver flags (DataGrip admits consoles may modify data if the driver ignores read-only); **Karagöz re-enforces** policies for P2P connections.
6. **Audit**: Karagöz records `{user, db, final_sql (masked), params (masked), classification, rows, duration, outcome, cancel/terminate events}`; Hacivat local history uses the same schema (doc 10).
7. **Explain**: PG/MySQL/SQLite/MSSQL normalizers first; raw view for everything else; flame graph component.
8. **Run scripts**: streaming runner for large files and multi-target runs; no arbitrary 1000-DML cap.
9. **Notifications**: separate channels for execution log (silent), long-running completion (toast when window unfocused), cancel issues (toast with action).

## 6. i18n considerations

- All Output/summary lines are message IDs with named plural args (ALG-EXEC-10), never ChoiceFormat-style `{n,choice,…}` concatenation (DataGrip's English-centric optional fragments like "(aborted!)" and " of {2}" are hard to translate).
- Durations/numbers via ICU4X; copied/exported logs use invariant formats.
- Server error texts verbatim; wrappers translated.
- Result-tab titles from comments keep user text (no case mapping; Turkish İ/ı preserved).
- Chooser labels show SQL verbatim; counts translated ("3 ifade").
- Button mnemonics (Execute All, Skip All, Ignore All, Keep Waiting) need per-locale access keys; avoid letters that collide on Turkish keyboard layouts.
- Parameter names: Unicode-aware patterns; never upper/lower-case names with locale-sensitive functions.
- Shortcuts (Ctrl+Enter, Ctrl+F2, Ctrl+Alt+Shift+Enter) must be checked on Turkish Q/F layouts (doc 12).

## 7. Risks, pitfalls, open questions

- **Resolved:** Smallest statement vs Smallest subquery or statement (§3.3) — prose swapped; still open: which subquery kinds qualify and nested-statement behavior (doc 13 CARET-05/06/14/21/22/25..30). Doc 13 CARET-02/03 expectations need flipping.
- Default of *Open results in new tab*: screenshot ✔ vs help "by default updates the same tab" — verify (SET family).
- `WHERE TRUE` treated as intentional or as always-true — verify (UNSAFE-03).
- Mapping of Execute All / Execute and Suppress / Skip / Skip All buttons and scope of "Suppress" — verify.
- Retry/Ignore/Ignore All/Stop flow and default continue-on-error — verify (ERR).
- "1000 DML statements per file" vs batch mode cap — verify (RUNFILE-01).
- Native `?` bound vs text-substituted — verify (PARAM-01/17).
- Whether "Database queries that took much time" fires only when unfocused, and its threshold — verify.
- Text substitution is an injection footgun; Hacivat defaults to bind mode.
- EXPLAIN ANALYZE executes statements — Hacivat defaults to rollback-wrapped DML.
- Deactivate leaves server statements running; Hacivat should offer server-side terminate with clear privilege errors.
- Karagöz must not trust client classification; log divergences.
