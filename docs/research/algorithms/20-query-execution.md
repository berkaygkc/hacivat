# 20 — Query Execution: Canonical Algorithm Catalog (HV-EXEC)

> Status: canonical, implementation-ready. Date: 2026-11-11. Scope: everything between "the user presses Execute" (or an MCP agent calls an execute tool) and "results, messages and history are recorded": caret→range resolution, statement chooser, parameters, the pre-execution safety gate, the per-session execution queue, the auditor chain, multiple result sets, script error policy, cancellation, transaction tracking, the output log, the history store, explain plans and flame graphs, script runs, in-editor results.
>
> This is a **synthesis** document. It merges and decides between the algorithm sketches in the research library; it does not add new broad research. Conventions, evidence tags and clean-room rules: `../README.md`. Companion catalogs: `21-result-fetching-and-store.md` (HV-RES), `22-data-grid-component.md` (HV-GRID), `23-data-editing.md` (HV-EDIT), `24-clipboard-import-export.md` (HV-XFER).

## Sources and licenses

| Source document | Specs consumed | Underlying project license |
|---|---|---|
| `datagrip/07-query-execution-ux.md` | ALG-EXEC-01…17 | DataGrip: proprietary (behaviour only, no code, no shipped text) |
| `datagrip/10-tools-history-import-export.md` | ALG-TOOL-03, -12, -13 | DataGrip proprietary; IntelliJ Community Apache-2.0 |
| `datagrip/11-settings-catalog.md` | §3.2–3.4 (QE-*, OR-*, UP-*) | DataGrip proprietary |
| `datagrip/13-behavioral-test-corpus.md` | families CARET, SEL, CHOOSE, PARAM, TITLE, UNSAFE, RO, TX, ERR, HIST | own corpus |
| `clients/oss-clients-beekeeper-and-tui.md` | ALG-EXEC-01 (risk classification), ALG-EXEC-02 (cancellation) | Beekeeper GPL-3.0 (behaviour only); rainfrog MIT; harlequin MIT |
| `clients/oss-clients-redisinsight-compass.md` | ALG-MONGO-04 (explain → flame graph) | Compass SSPL (behaviour only) |
| `clients/sql-tooling-02-lexing-and-splitting-by-dialect.md` | ALG-SQL-SPLIT-01/02, ALG-SQL-PARAM-01 (only where execution depends on them) | pgls MIT/Apache; libpg_query BSD-3 |
| `clients/ui-01-stack-decision.md` | ALG-UI-02 (error ids, versioning principle) | — |
| `i18n/02-turkish-and-db-specific-pitfalls.md` | ALG-I18N-06 (search key, used by history search), ALG-I18N-08 (server messages) | ICU4X Unicode-3.0 |
| `decisions/0001-native-ui-on-gpui.md` | UI is in-process GPUI; ALG-UI-01 transport obsolete | gpui Apache-2.0 |
| `verification/01-db-behavior-tests.md`, `verification/02-external-fact-check.md` | override contradicted claims (see "Verification status" per spec) | — |

Clean-room note: every algorithm below is written in our own words and pseudocode. No code was copied from GPL/AGPL/SSPL/proprietary sources (Beekeeper, Compass, RedisInsight, DataGrip); permissive sources (rainfrog MIT, pgls MIT, gpui Apache-2.0) are cited by file only.

---

## 0. Dependency map and priorities

```
                       hacivat-sql (lexer, splitter, params; sql-tooling-02)
                                    │
     ┌───────────────┬──────────────┼──────────────────┬───────────────────┐
     ▼               ▼              ▼                  ▼                   ▼
 HV-EXEC-01 ──► HV-EXEC-02     HV-EXEC-03         HV-EXEC-04          HV-EXEC-15
 caret→units    chooser        parameters         safety gate         script runner
     └───────────────┴──────────────┴──────────────────┘                   │
                                    ▼                                      │
                         HV-EXEC-05 session actor + queue ◄────────────────┘
                          │        │           │          │
                          ▼        ▼           ▼          ▼
                   HV-EXEC-06  HV-EXEC-09  HV-EXEC-10  HV-RES-04/05 (fetch, cursors; doc 21)
                   auditors    cancel      tx state
                     │   │   │
          ┌──────────┘   │   └───────────────┐
          ▼              ▼                   ▼
   HV-EXEC-11       HV-EXEC-12          HV-EXEC-07 ──► HV-EXEC-08
   output log       history store       multi result   script error policy
                                         sets, titles
                                              │
                                              ▼
                                   HV-EXEC-13 explain model ──► HV-EXEC-14 flame graph
                                   HV-EXEC-16 in-editor results (anchors)
```

| ID | Name | Priority | Depends on | Consumed by |
|---|---|---|---|---|
| HV-EXEC-01 | Caret / selection → execution units | **v1** | sql-tooling-02 SPLIT-01/02 | 02, 05, MCP `execute_at_cursor` |
| HV-EXEC-02 | Statement chooser | **v1** | 01 | UI |
| HV-EXEC-03 | Parameter detection, prompt and binding | **v1** (bind); v1.x (template patterns) | sql-tooling-02 PARAM-01 | 04, 05, 12 |
| HV-EXEC-04 | Pre-execution safety gate | **v1** | 01, 03, sql-tooling-05 classifier | 05; Karagöz |
| HV-EXEC-05 | Session actor and execution queue | **v1** | driver trait (doc 21 HV-RES-04) | everything below |
| HV-EXEC-06 | Auditor chain | **v1** | 05 | 07, 10, 11, 12; Karagöz audit |
| HV-EXEC-07 | Multiple result sets, outcomes, result tabs | **v1** | 05, 06, HV-RES-02 | grid, MCP |
| HV-EXEC-08 | Script error policy | **v1** | 05, 07, 10 | UI, run configs |
| HV-EXEC-09 | Cancellation (client + server, escalation) | **v1** | 05 | UI, MCP, Karagöz |
| HV-EXEC-10 | Transaction state tracking | **v1** | 05, 06 | UI, HV-EDIT-07, HV-RES-05 |
| HV-EXEC-11 | Output log and message formatting | **v1** | 06, i18n ALG-I18N-08 | UI, logs, MCP |
| HV-EXEC-12 | Query history store | **v1** | 06 | UI, MCP |
| HV-EXEC-13 | Explain plan model and normalizers | **v1** (PG, MySQL 8, SQLite, raw for others); v1.x (MSSQL, MariaDB, ClickHouse, Mongo) | 05 | 14, UI |
| HV-EXEC-14 | Flame graph / icicle layout | **v1.x** | 13 | UI |
| HV-EXEC-15 | Script runner (large files, multi-target) | **v1.x** | 01, 05, sql-tooling-02 | CLI, run configs |
| HV-EXEC-16 | In-editor result anchoring | **later** | 07, editor anchors | UI |

## 1. Cross-cutting decisions for this catalog

1. **Two-layer split.** All algorithms in this file live in UI-free core crates. The UI (`hacivat-app`, `hacivat-ui`) only renders state and sends commands. Karagöz links the same `hacivat-sql` and `hacivat-core::exec::audit` code for server-side enforcement.
2. **Rust placement (crates).**
   - `hacivat-sql` = the `sql-lexer` + `sql-split` crates proposed in sql-tooling-02 (lexer, splitter, client-command pre-pass, placeholders). No I/O.
   - `hacivat-core::exec` — units, policies, gate, session actor, queue, auditors, cancel, tx state, explain.
   - `hacivat-core::history` — history store (SQLite).
   - `hacivat-core::driver` — the driver trait and per-DBMS implementations (`driver::{pg, mysql, mssql, sqlite, clickhouse, redis, mongo}`), owned by doc 21.
   - `hacivat-app::exec_ui` — GPUI glue: gutter markers, chooser popup, dialogs. `hacivat-ui` stays DB-agnostic.
3. **Runtimes.** Drivers run on a Tokio runtime owned by the core. The GPUI side uses `gpui_tokio` (Apache-2.0, `zed@912159c` `crates/gpui_tokio`) or plain channels to hop between executors. The UI thread never awaits DB, network or file I/O (DBeaver issue lesson: #42803, #9699, #35261).
4. **Errors are Fluent ids plus arguments**, never English text (ALG-UI-02 principle, still valid with native UI). Server text is shown verbatim (ALG-I18N-08).
5. **Karagöz re-enforces.** Every client-side check in this file (classification, unsafe warnings, read-only) is advisory. Karagöz re-splits, re-classifies and audits server-side, because P2P traffic bypasses Perde (README hard requirement).
6. **Never use editor-mode (heuristic) split output for execution** (sql-tooling-02 §7). Caret lookup may use the editor index; the text sent always comes from execution-mode splitting, and an `Ambiguous` boundary stops and asks.
7. **Crate versions to check** (last seen in the local Cargo cache or `.refs` on 2026-11-11; re-check at implementation): `tokio` (current), `tokio-postgres` / `postgres-protocol` (not cached locally — check), `mysql_async` (check), `tiberius` (check), `rusqlite 0.32.1` (cached; newer likely), `regex 1.12.2`, `memchr 2.8.3`, `indexmap 2.14.2`, `parking_lot 0.12.5`, `crossbeam-channel 0.5.15`, `zstd 0.13.3`, `fluent-bundle 0.16.0` (`.refs/fluent-rs`), ICU4X `2.4.0` components (`icu_casemap`, `icu_collator`, `icu_segmenter`), `jiff 0.2.17` (durations/timestamps; alternative `time 0.3.x`), `gpui 0.2.2` pinned by git commit.

---

## 2. Specifications

### HV-EXEC-01: Caret / selection → execution units

- **Priority / depends on:** v1. Depends on sql-tooling-02 ALG-SQL-SPLIT-01 (execution mode), ALG-SQL-SPLIT-02 (client-command pre-pass), the editor CST (for subqueries).
- **Sources merged:** datagrip/07 ALG-EXEC-01 (policy triple, Smallest-statement resolution), datagrip/11 QE-1…QE-5, sql-tooling-02 SPLIT-01/02 (execution vs editor mode, GO batches, DELIMITER), datagrip/13 CARET/SEL families.
- **Problem:** Map (caret positions or selections, policy) to the exact, ordered list of statements or batches to send, consistently across dialects, without sending heuristic boundaries.
- **Inputs / outputs:**
  - In: `DocSnapshot` (text + version), dialect + client flavor, `Vec<CaretOrSelection>`, `ExecPolicy { inside, outside, selection }`, `split_mode` for Generic dialect (parse-or-separator default | grammar only | separator only).
  - Out: `Resolution::Units(Vec<ExecUnit>) | Ask(Vec<Candidate>) | NothingToRun | Ambiguous(Vec<AmbiguousBoundary>)`.
- **Decision:**
  - Three execute actions (Execute, Execute 2, Execute 3), each with its own policy triple (DataGrip model).
  - Default policy for action 1: inside = **Ask, with a one-candidate shortcut** (if the chooser would have exactly one distinct candidate, run it directly); outside = Nothing; selection = Exactly as separate statements. Action 2 default: inside = Largest statement or batch. Action 3 default: Whole script.
  - Labels are unambiguous: "Smallest subquery or statement" runs the innermost runnable subquery; "Smallest statement" runs the enclosing statement. DataGrip's help prose swaps them; we follow its screenshots, labels and DBE-6418.
  - Rejected: DBeaver's "current query = text between blank lines" heuristic as an execution source (it can merge or split statements silently). Rejected: sending editor-mode `Heuristic` boundaries.
- **Algorithm:**
  1. Run the client pre-pass (SPLIT-02) and the execution-mode splitter (SPLIT-01) over the whole document. Cache them by `(doc version, dialect, client flavor)`. Build `StatementIndex`, an interval tree over *smart ranges*. A smart range is the statement plus the comments attached to it (no blank line in between) plus a trailing delimiter on the same line.
  2. If a selection is non-empty:
     - `AsOne`: one unit = the selected text verbatim. Strip one trailing `;` only if the dialect's driver rejects it (Oracle SQL statements; PL/SQL units keep `END;`).
     - `AsSeparate`: split the selection with lexer state carried in from the document start (active `DELIMITER`, inside-batch status, open dollar quote) and return the units.
     - `SmartExpand`: if any statement boundary lies strictly inside the selection, expand the start and end to the enclosing statements, then split. Otherwise one unit = the selection verbatim.
     - A selection made only of whitespace or comments returns `NothingToRun`.
  3. If there is no selection, find the top-level statement `S` whose smart range contains the caret.
  4. If there is none (the caret is outside every statement), apply the outside policy: `Nothing` → `NothingToRun` (UI hint `exec-nothing-to-run`); `WholeScript` → all; `EverythingBelow` → statements whose start is after the caret.
  5. If `S` exists, apply the inside policy:
     - `SmallestSubqueryOrStatement`: walk the CST upward from the caret leaf. The first query expression that is a subquery (derived table, scalar/`IN`/`EXISTS` subquery, parenthesised set-operation operand, CTE body) and that can run on its own is the unit. A node runs on its own when it has no outer (correlated) references; a CTE body qualifies only if it references no sibling CTE, otherwise prepend the needed `WITH` items. The unit text is the node without its enclosing parentheses. If none qualifies, fall through to `SmallestStatement`.
     - `SmallestStatement`: the innermost statement node that the driver can run alone. T-SQL inner statements of `BEGIN…END` qualify. A PL/pgSQL body inside `$$` is a string and does not. Oracle PL/SQL inner statements do not (local variables). Otherwise `S`.
     - `LargestStatement` → `S`. `LargestStatementOrBatch` → for T-SQL/Sybase, the whole GO batch as one unit; otherwise `S`.
     - `WholeScript` → all units. `EverythingFromCaret` → `S` and every following unit.
     - `Ask` → build candidates (HV-EXEC-02). One distinct candidate → run it directly.
  6. Expand `GO n` into a unit with `repeat = n`. Drop client-only commands (`DELIMITER`, `GO`, psql `\…`, SQL*Plus `SET/PROMPT`, sqlite3 `.…`) after applying their effect. Client commands that we do not emulate yield an `UnsupportedClientCommand` diagnostic. The run stops with a hint; nothing is sent.
  7. If any produced unit touches an `Ambiguous` boundary (heuristic split, unterminated block), return `Ambiguous` and let the UI offer "run as one statement" / "edit". For PG, the libpg_query split may serve as an oracle (SPLIT-01 step 9).
  8. With several carets (multi-cursor), compute units per caret, take the union by range, keep document order and remove duplicates.
  9. Attach to each unit an `OffsetMap` from unit text to document offsets. Later rewrites (parameter binding, auto-LIMIT, EXPLAIN prefix) append to the map so that server error positions map back exactly (ALG-I18N-08 step 5).
- **Data structures and Rust placement** (`hacivat-core::exec::units`):
  ```rust
  pub enum InsidePolicy { Ask, SmallestSubqueryOrStatement, SmallestStatement,
                          LargestStatement, LargestStatementOrBatch, WholeScript, EverythingFromCaret }
  pub enum OutsidePolicy { Nothing, WholeScript, EverythingBelow }
  pub enum SelectionPolicy { AsOne, AsSeparate, SmartExpand }
  pub struct ExecPolicy { pub inside: InsidePolicy, pub outside: OutsidePolicy, pub selection: SelectionPolicy }

  pub struct ExecUnit {
      pub text: Arc<str>,              // exactly what is sent (before param rewriting)
      pub doc_range: Range<usize>,     // byte offsets in the document
      pub batch_id: Option<u32>,       // T-SQL GO batch
      pub repeat: u32,                 // GO n
      pub kind: UnitKind,              // Statement | Batch | PlSqlUnit | Subquery
      pub offsets: OffsetMap,          // unit text offset -> doc offset (piecewise linear)
  }
  pub enum Resolution { Units(Vec<ExecUnit>), Ask(Vec<Candidate>), NothingToRun, Ambiguous(Vec<AmbiguousBoundary>) }
  pub fn resolve(doc: &DocSnapshot, at: &[CaretOrSelection], p: &ExecPolicy, d: &DialectCtx) -> Resolution;
  ```
  `StatementIndex` = sorted `Vec<(Range, ItemId)>` with binary search (the items do not overlap at top level), plus a per-item lazily built CST handle.
- **Per-DBMS matrix:**

  | DBMS | Unit boundary for execution | Batch concept | Notes |
  |---|---|---|---|
  | PostgreSQL | `;` outside quotes, dollar quotes, `BEGIN ATOMIC` blocks | none | `COPY … FROM STDIN` + data block is one unit (pass-through) |
  | MySQL/MariaDB | active `DELIMITER` (client), stored-program blocks | none | `DELIMITER` consumed client-side |
  | SQL Server | `GO` lines (client) delimit batches; statements inside a batch are not split for execution | GO batch, `GO n` | `LargestStatementOrBatch` = batch |
  | SQLite | `;` with trigger `BEGIN…END` | none | |
  | Oracle | `;` for SQL; `/` line for PL/SQL units | none | SQL statements are sent without `;` |
  | ClickHouse | `;` | none | `FORMAT` data blocks pass through |
  | Generic/ANSI | parse, else separator (`split_mode`) | none | |
- **Complexity / performance budget:** index build O(tokens), cached by version; resolution is O(log n) lookup plus O(depth) CST walk. Budget: ≤ 2 ms p95 on a 50k-line file with a warm cache; ≤ 30 ms on a cold cache (background re-split on edit, never on keypress-to-run).
- **Edge cases (exhaustive list):** caret exactly at a statement's start or end, right after `;`, in trailing whitespace on the same line, or on the next line; caret inside an attached leading comment (inside) vs a comment separated by a blank line (outside); caret on the empty statement `;;`; unterminated last statement; caret inside a string literal or quoted identifier (still inside `S`); caret inside a dollar-quoted body (Smallest statement = the whole `CREATE FUNCTION`); correlated subquery (not standalone → statement); subquery in a `VALUES` row; `INSERT … SELECT` source; `UNION` branch without parentheses (not a subquery → statement); `EXISTS(SELECT …)`; `LATERAL` subquery that references earlier FROM items (correlated); CTE referencing a previous CTE (include it); recursive CTE body (never standalone); `EXPLAIN SELECT` with the caret in the SELECT (Smallest statement = the whole EXPLAIN); `CREATE VIEW … AS SELECT` (whole CREATE); selection spanning half a string literal or comment (AsSeparate splits from the carried lexer state; AsOne sends verbatim and the server reports the error); whitespace-only selection; selection across a `GO` with AsOne (send as is and warn: the server rejects GO); `DELIMITER //` active with AsSeparate; T-SQL statements without `;` inside a batch; Oracle `/` after a statement that already ended with `;` (ignore the duplicate); multi-caret on the same statement (dedupe); document edited while a run is in progress (units are snapshots; gutter markers use anchors); file larger than the editor limit (HV-EXEC-15 path); BOM at file start (not part of the unit); CRLF line endings (offsets in bytes, unaffected); psql `\i` / sqlcmd `:r` include (unsupported → diagnostic); read-only and unsafe checks apply per produced unit (HV-EXEC-04).
- **Test cases (input → expected):**
  1. FX-CARET-01 (doc 13): caret inside `(SELECT * FROM film_actor WHERE film_id > 100)`. SmallestSubqueryOrStatement → `SELECT * FROM film_actor WHERE film_id > 100`; SmallestStatement → statement 1; EverythingFromCaret → statements 1 and 3; WholeScript → 0, 1, 3.
  2. `SELECT a FROM t WHERE x IN (SELECT y FROM u WHERE u.k = t.k)`, caret in the inner query, SmallestSubquery → the whole statement (correlated).
  3. T-SQL `SELECT 1\nGO\nSELECT 3; SELECT 4;\nGO`, caret on `SELECT 4`. LargestStatementOrBatch → one batch unit `SELECT 3; SELECT 4;`. LargestStatement → `SELECT 4`.
  4. Caret on a blank line, outside = Nothing → `NothingToRun`.
  5. Selection `SELECT 1; SEL`, SmartExpand → `SELECT 1;` and the whole second statement.
  6. PG `select * from t\n\nselect 1` (no `;`), WholeScript → `Ambiguous` (the boundary is heuristic); "run as one" sends the text verbatim.
  7. MySQL `DELIMITER //\nCREATE PROCEDURE p() BEGIN SELECT 1; END//\nDELIMITER ;\nCALL p();` WholeScript → 2 units: `CREATE PROCEDURE … END` and `CALL p()`.
  8. Oracle `BEGIN NULL; END;\n/\nSELECT 1 FROM dual;` → 2 units: a PL/SQL unit (text includes `END;`) and `SELECT 1 FROM dual` (no `;`).
  9. T-SQL `SELECT 1\nGO 3` → 1 unit, repeat 3.
  10. Two carets in statements 2 and 5 with LargestStatement → [2, 5] in document order.
- **i18n notes:** chooser and hint messages: `exec-nothing-to-run`, `exec-ambiguous-boundary = The statement boundary near line { $line } is not certain. Run as one statement or add a delimiter.`, `exec-client-command-unsupported = { $command } is a { $client } client command and is not supported.` SQL text is never translated or case-mapped. Line and column numbers are shown with locale digits only in UI chrome, never in copied text.
- **Verification status:** DataGrip's swapped help prose is resolved by screenshots and DBE-6418 [Observed: datagrip/07 §3.3]. Splitter test vectors come from sql-tooling-02 and are not yet run against live servers (sql-tooling-02 §7 lists the conformance matrix). The DataGrip default for QE-2/QE-3 is still [Unknown] (doc 11). This spec chooses its own defaults.
- **Prior art and evidence:** [Observed: datagrip/07 §3.2–3.5, settings screenshot]; [Observed: pgls `pgls_statement_splitter`, MIT]; [Observed: libpg_query `pg_query_split.c`, BSD-3].
- **Used by:** Hacivat (editor, run actions); MCP tool `execute_at_cursor { offset, policy }`; Karagöz (execution-mode split only, requires `Exact` and exactly one statement per request).

### HV-EXEC-02: Statement chooser

- **Priority / depends on:** v1. Depends on HV-EXEC-01.
- **Sources merged:** datagrip/07 ALG-EXEC-02, datagrip/13 CHOOSE family.
- **Problem:** When the policy is Ask, present an unambiguous, keyboard-first list of candidate ranges.
- **Inputs / outputs:** `Vec<Candidate{kind, range, unit_count}>` from HV-EXEC-01 → the chosen candidate, or cancel.
- **Decision:** DataGrip's candidate order and inclusion rules, plus three additions. (1) If there is only one distinct candidate, run it without a popup. (2) Number keys 1–9 pick quickly. (3) The last chosen *kind* is remembered per file and preselected. The default preselection is the statement candidate (as in DataGrip's screenshot). Rejected: always showing the popup (DataGrip shows it even with one item), because it costs a keystroke for nothing.
- **Algorithm:**
  1. Candidates in order: innermost runnable subquery (if any) → smallest statement (if its range differs from the previous one) → largest statement or batch (if different) → everything from the caret (only if it adds units) → whole script (always present when there are ≥ 2 units).
  2. Remove duplicate ranges, keeping the earliest (most specific) label.
  3. Label = the range text with runs of whitespace collapsed to one space. Leading comments are kept (as DataGrip does). Control characters become spaces, bidi controls become visible tokens (see i18n notes). Truncate at the popup width with `…`, measured on grapheme clusters. Multi-unit candidates get a suffix such as "· 3 statements".
  4. Preselect: the remembered kind for this file if present, else the statement candidate.
  5. When the selection changes, highlight the range in the editor (theme token `editor.exec-candidate`). Enter runs; Esc closes; 1–9 picks; "Customize…" opens the Query Execution settings.
- **Data structures and Rust placement:** candidate building lives in `hacivat-core::exec::units::candidates()` (pure). The popup is `hacivat-app::exec_ui::Chooser`, built on a `hacivat-ui` list popover. The per-file memory (`chooser_last_kind: HashMap<FileId, CandidateKind>`) is stored with the workspace state.
- **Complexity / performance budget:** building candidates is O(depth); the popup opens ≤ 16 ms after the keypress (one frame).
- **Edge cases (exhaustive list):** a single candidate (runs directly); a one-statement script (statement = whole script → one candidate → runs directly); a very long single-line statement (label truncated; full text in a tooltip); labels with RTL or bidi text (rendered with isolation; Trojan Source controls shown visibly); labels containing tabs or newlines (collapsed); caret outside a statement (the chooser is never shown for outside policies); a document edited while the popup is open (close the popup and re-resolve on the next Execute); a T-SQL batch candidate equal to the statement (deduplicated); a subquery candidate equal to the statement (`SELECT` in parentheses at top level — deduplicated).
- **Test cases (input → expected):** FX-CARET-01 → 3 items [subquery, statement, whole script], #2 preselected; a single-statement script → no popup, runs; the T-SQL batch fixture → 4 items including the batch; pressing `3` with 3 items → runs the whole script; after choosing "whole script" once in file A, the next Ask in file A preselects "whole script" while file B still preselects "statement".
- **i18n notes:** Fluent `exec-chooser-title`, `exec-chooser-kind-subquery`, `-statement`, `-batch`, `-from-caret`, `-script`, `exec-chooser-unit-count = { $count -> [one] { $count } statement *[other] { $count } statements }` (Turkish: `{ $count } ifade` for every count, still routed through plural rules). Number-key shortcuts use physical key codes (they work on Turkish Q/F layouts). SQL in labels is never translated.
- **Verification status:** order and labels [Observed: DataGrip screenshot `db_query_exec_ask_what_to_execute.png`]; the Hacivat additions are our own decisions.
- **Prior art and evidence:** datagrip/07 §3.5.
- **Used by:** Hacivat.

### HV-EXEC-03: Parameter detection, prompt and binding

- **Priority / depends on:** v1 for native placeholders bound as parameters and for the parameter dialog; v1.x for the DataGrip-compatible text-template patterns. Depends on sql-tooling-02 ALG-SQL-PARAM-01 and the lexer token kinds.
- **Sources merged:** datagrip/07 ALG-EXEC-03 and ALG-EXEC-09, datagrip/11 UP-1…UP-4, sql-tooling-02 ALG-SQL-PARAM-01, datagrip/13 PARAM family.
- **Problem:** Find placeholders and template variables, collect values (remembering previous ones), and turn each unit into `(final_text, binds)` safely. Text substitution is opt-in and clearly labelled.
- **Inputs / outputs:** a unit's tokens, dialect, client flavor, settings `{ native_placeholders, question_mark_in_pg, template_patterns: Vec<Pattern>, substitute_in_strings, review_before_run }`, remembered values → `PreparedUnit { final_text, binds: Vec<BindValue>, param_spans, text_substitutions }`, or `NeedsValues(Vec<ParamPrompt>)`.
- **Decision:**
  - **Bind by default.** Native placeholders (PG `$n` and `:name`; MySQL `?`; SQLite `?NNN`, `:a`, `@a`, `$a`; T-SQL undeclared `@name`; ClickHouse `{name:Type}`; Oracle `:name`) are bound through the driver's extended protocol. `:name` is rewritten to the dialect's positional form (`$1…` for PG, repeated `?` for MySQL).
  - PG `?` is **off** by default (it clashes with the jsonb operators `?`, `?|`, `?&`). It is enabled per data source for JDBC-style scripts.
  - The 8 DataGrip-compatible template patterns (`?N`, `%name`, `${…}`, `$(…)`, `$name$`, `#name#`, boundary-guarded `$name`, boundary-guarded `#name`) ship in v1.x **disabled by default**, behind a settings switch. When enabled, they substitute *text* and the dialog shows a "text substitution" badge. Rejected: DataGrip-style on-by-default textual templates (an injection footgun); rejected: no template support at all (DataGrip migrants rely on `${var}`).
  - MCP tools are bind-only. Karagöz never substitutes client variables; it receives the final text plus binds.
- **Algorithm:**
  1. Regions = code tokens outside comments, strings, dollar quotes and quoted identifiers (from the lexer). If `substitute_in_strings` is on (template patterns only), string interiors are added.
  2. Native placeholders: classify per dialect using ALG-SQL-PARAM-01 (operand-position rule for `?`, exclude `::`, `:=`, `[a:b]` slices, T-SQL locally declared `@x`, `@@x`, Oracle trigger `:new`/`:old`).
  3. Template patterns (if enabled): scan each region left to right; at each position try the patterns in table order; the first match wins and the scan advances past it, so matches never overlap. Reject a match whose start token belongs to a native construct (dollar-quote tag, `::` cast, JSON operator). The name is capture group 1 if present; else, for a match starting with `:`, the remainder; else the whole match. The regex engine is linear-time (`regex` crate). The look-behind boundary of patterns 7 and 8 is implemented as an explicit check that the previous char is not a word char (no `fancy-regex`).
  4. Group by name (case-sensitive; `MüşteriId` ≠ `müşteriıd`). The same name used in several units gets one value.
  5. If any value is unknown, or `review_before_run` is on, return `NeedsValues`. The dialog pre-fills remembered values keyed by `(data_source_id, name)`. Each parameter offers a type (inferred from the resolved column type when the parameter is compared with a column, else `text`), an explicit NULL toggle (distinct from the empty string), and, for templates, an "insert as quoted literal" toggle that quotes and escapes per dialect.
  6. Build `final_text`: rewrite native placeholders to the positional form and record the offset changes in the unit's `OffsetMap`; apply template substitutions in reverse order and record them in the map as well.
  7. Secrets: names matching `(?i)(pass(word)?|secret|token|api_?key|pwd)` are masked in history, logs and audit, and are not remembered across restarts unless the user opts in.
  8. Record in the auditor event: `final_text` (masked) and `binds` (masked) for HV-EXEC-06.
- **Data structures and Rust placement** (`hacivat-core::exec::params`; detection in `hacivat-sql::params`):
  ```rust
  pub enum PlaceholderKind { Positional(u32), Question(u32), Named(Arc<str>), Template { pattern: u8, name: Arc<str> } }
  pub struct ParamSpan { pub range: Range<usize>, pub kind: PlaceholderKind }
  pub enum BindValue { Null, Typed { type_hint: LogicalType, text: Arc<str> } }   // canonical text, parsed by the driver
  pub struct PreparedUnit { pub final_text: Arc<str>, pub binds: Vec<BindValue>, pub offsets: OffsetMap, pub masked_for_log: Arc<str> }
  pub trait ParamValueStore { fn get(&self, ds: DataSourceId, name: &str) -> Option<BindValue>; fn put(&mut self, ds: DataSourceId, name: &str, v: BindValue); }
  ```
- **Per-DBMS matrix:**

  | DBMS | Native placeholder forms | Rewritten as | Not a placeholder |
  |---|---|---|---|
  | PostgreSQL | `$n`, `:name` (on), `?` (off by default) | `$1…$n` | `?`, `?\|`, `?&` (jsonb), `::`, `a[1:2]`, `$tag$` |
  | MySQL/MariaDB | `?`, `:name` (on) | `?` repeated | `@var`, `@@var`, `:=` |
  | SQL Server | `@name` not declared in the batch | `@P1…` (tiberius) | `@@x`, declared `@x`, procedure parameters in the CREATE header |
  | SQLite | `?`, `?NNN`, `:a`, `@a`, `$a` | native | — |
  | ClickHouse | `{name:Type}` | server-side parameters | map literals `{'a':1}` |
  | Oracle | `:name`, `:1` | native | `:new`, `:old`, `:=` |
- **Complexity / performance budget:** O(tokens + regions × patterns). Budget ≤ 1 ms per unit of 10 KB; ≤ 20 ms for a 1 MB script.
- **Edge cases (exhaustive list):** jsonb `?`/`?|`/`?&`; `::type` casts after `:x` (`:x::int` → variable then cast); `a[:2]`, `a[1:]`, `a[:]`; MySQL `@var`, `SELECT @x := 1`; `$1` inside a PL/pgSQL body (inside a dollar quote → not a placeholder); `#temp` tables vs the `#name#` and `#name` patterns; `$tag$` vs pattern 5; `${}` (empty name → shown as "(unnamed)"); parameters inside comments (ignored); one name across units (one value); empty string vs NULL (explicit toggle); a value containing quotes in text mode (literal toggle escapes it); Unicode names (`:müşteri`, word-char class covers all scripts); Turkish dotted/dotless I in names (compared case-sensitively, never case-folded); a pattern that matches the empty string (rejected when settings are saved); catastrophic regexes (impossible with the linear engine); T-SQL `DECLARE @x INT = @y` (`@y` is a placeholder); `EXEC sp @p = @v`; placeholders in client commands (ignored); a placeholder in a `LIMIT` (bound as an integer); PG with more than 65,535 binds (error before sending); a value too large for the dialog (multi-line editor); parameter in an `IN (…)` list (bound as a single value; an array mode exists for PG `= ANY($1)`, and template text mode exists for lists).
- **Test cases (input → expected):**
  1. PG `select * from t where a = :a and b::int = :b and c ? 'k'` → placeholders `[:a, :b]`; final `… a = $1 and b::int = $2 and c ? 'k'`.
  2. PG with `question_mark_in_pg = true`: `where a = ? and j ? 'k'` → only the first `?`.
  3. T-SQL `DECLARE @x int = 1; SELECT * FROM t WHERE id = @x AND y = @y` → `[@y]`.
  4. SQLite `select ?2, :a, @a, $a` → 4 placeholders (3 distinct names plus a positional).
  5. Templates on: `SELECT ${col} FROM t WHERE id='${id}'`, substitute-in-strings off → `{col}`; on → `{col, id}`.
  6. Templates on: ` $ACCESS ` → `ACCESS` (pattern 7); `V$ACCESS` → none; `$x1$` → `x1` (pattern 5 wins by order); `?69` → `69`.
  7. `ALTER ROLE app PASSWORD :pwd` with value `s3cr3t` → history stores `PASSWORD $1` with binds `["***"]`.
  8. PG `select arr[1:2]` → none.
- **i18n notes:** dialog `exec-params-title`, `exec-params-null-toggle`, `exec-params-text-substitution-badge = Text substitution — the value becomes part of the SQL.`, `exec-params-type-label`. Values are data and are never localized. Typed values in the dialog use the HV-GRID-11 parser (machine format first, then the user's locale; Turkish `1.234` is ambiguous and is flagged).
- **Verification status:** the DataGrip pattern list is [Observed: help]. Whether DataGrip binds native `?` or substitutes it is still [Unknown] (PARAM-01/17 in doc 13). This spec does not depend on the answer.
- **Prior art and evidence:** datagrip/07 §3.7; pgls `crates/pgls_lexer/src/params.rs` (MIT) [Observed].
- **Used by:** Hacivat; MCP (bind only); Karagöz (fingerprinting; audits masked text).

### HV-EXEC-04: Pre-execution safety gate

- **Priority / depends on:** v1. Depends on HV-EXEC-01 and -03, and on the statement classifier specified in `clients/sql-tooling-05-safety-classification-lint-format.md` (outside this catalog; consumed here as a black box `classify(unit) -> Classification`).
- **Sources merged:** datagrip/07 ALG-EXEC-05 (unsafe validator, dialog buttons) and ALG-EXEC-06 (read-only classification), Beekeeper/TUI doc ALG-EXEC-01 (risk classes, connection safety tags, confirm-in-transaction), datagrip/13 UNSAFE and RO families.
- **Problem:** Decide per unit whether to run it, warn, confirm, run in a transaction and confirm, or deny, using a fast offline check. This happens before anything reaches the server.
- **Inputs / outputs:** `PreparedUnit`s, connection settings (`safety_level: Dev | Staging | Production`, `read_only: bool`, per-connection overrides), user settings (unsafe warnings on/off, skip temp tables), Karagöz policy snapshot (if remote) → per unit `GateDecision { Run | Warn(reasons) | Confirm(reasons) | RunInTxThenConfirm(reasons) | Deny(reason) }` and a combined dialog plan.
- **Decision:**
  - The gate merges both sources. Classification comes from sql-tooling-05: Read, Lock, Dml, Ddl, Routine, Tx, Session, Unknown. Unknown is treated as a write.
  - Unsafe patterns (DataGrip): UPDATE/DELETE without WHERE, WHERE always true, DELETE with a JOIN that never references the target table. Optional checks (off by default): DROP/TRUNCATE of non-temporary tables, `ALTER … DROP COLUMN`, MySQL `UPDATE … LIMIT` without ORDER BY.
  - Production connections add confirm-in-transaction for DML (Beekeeper/rainfrog idea) on engines with transactional DML. DDL on engines where it auto-commits (MySQL, Oracle) gets a confirm *before* running. A missing WHERE on production needs the user to type the table name.
  - Read-only connections run statements inside a database-level read-only transaction or session (doc 21 HV-RES-04 matrix) in addition to the client check, because driver flags alone are not enough (DataGrip help admits this).
  - Rejected: relying only on client-side classification (Karagöz re-enforces). Rejected: blocking `Unknown` on dev connections (too noisy; it warns instead).
- **Algorithm:**
  1. For each unit: `c = classify(unit.final_text, dialect, server_version)`. A multi-statement unit (AsOne or a T-SQL batch) takes the maximum severity of its parts.
  2. Read-only connection and `c ∉ {Read}` → `Deny(exec-deny-read-only)`. The message names the class (data modification, DDL, routine with possible side effects, statement with possible side effects).
  3. Run the unsafe checks after parameter substitution, so that `WHERE :p = :p` with a template value folds to "always true". Constant folding covers `1=1`, `TRUE`, `'a'='a'`, `NULL IS NULL`, `NOT FALSE`, `x = x` on the same column reference, `1 IN (1)`. `WHERE CURRENT OF` is safe.
  4. Base decision by safety level:

     | Class | Dev | Staging | Production |
     |---|---|---|---|
     | Read | Run | Run | Run |
     | Lock (`FOR UPDATE`) | Run | Run | Warn |
     | Dml with WHERE | Run | Run | RunInTxThenConfirm (transactional engines), else Confirm |
     | Dml unsafe (no/always-true WHERE) | Warn | Confirm | Confirm + type table name |
     | Ddl destructive (DROP/TRUNCATE/ALTER … DROP) | Warn (if enabled) | Confirm | Confirm (+ type name for DROP DATABASE/SCHEMA) |
     | Ddl other | Run | Run | Confirm |
     | Routine / Unknown | Run | Warn | Confirm |
     | Session (SET/USE) | Run | Run | Run (logged) |
     | `EXPLAIN ANALYZE <x>` | class of x | class of x | class of x (HV-EXEC-13 wraps DML in a rolled-back tx) |
  5. Karagöz policy (remote sessions): apply `Deny` and `Confirm` rules from the snapshot. A client cannot downgrade them. Karagöz checks again on receipt.
  6. Collect decisions for all units. If any unit is Warn or Confirm, show **one** dialog listing them with the buttons **Execute**, **Execute All**, **Execute and Suppress**, **Skip**, **Skip All**, **Cancel**. Skip removes the unit from the queue. Suppress records `(file_id | console_id, fingerprint)` for the session's lifetime.
  7. `RunInTxThenConfirm`: the session actor (HV-EXEC-05) runs `BEGIN` (or a `SAVEPOINT` if a transaction is already open), executes, captures the affected row count and up to 100 sample rows via `RETURNING` when the dialect supports it, and then shows "Commit / Roll back". A countdown auto-rolls back after `confirm_timeout` (default 5 min) because the locks are held meanwhile. If the connection drops, the server has rolled back; tell the user.
- **Data structures and Rust placement** (`hacivat-core::exec::gate`):
  ```rust
  pub enum SafetyLevel { Dev, Staging, Production }
  pub enum GateDecision { Run, Warn(Vec<Reason>), Confirm { reasons: Vec<Reason>, type_to_confirm: Option<String> },
                          RunInTxThenConfirm(Vec<Reason>), Deny(Reason) }
  pub struct Reason { pub fluent_id: &'static str, pub args: FluentArgsOwned, pub unit: UnitId }
  pub fn gate(units: &[PreparedUnit], cls: &dyn Classifier, conn: &ConnSafety, policy: Option<&RemotePolicy>) -> Vec<GateDecision>;
  ```
- **Per-DBMS matrix (confirm-in-transaction feasibility):**

  | DBMS | Transactional DML | Transactional DDL | RunInTxThenConfirm for DML | DDL handling |
  |---|---|---|---|---|
  | PostgreSQL | yes | yes | yes | can also preview in a tx (opt-in) |
  | MySQL/MariaDB | yes (InnoDB) | no (implicit commit) | yes | Confirm before |
  | SQL Server | yes | mostly yes | yes | Confirm before (default) |
  | SQLite | yes | yes | yes | Confirm before |
  | Oracle | yes | no | yes | Confirm before |
  | ClickHouse | no (mutations are async) | no | no → Confirm | Confirm |
- **Complexity / performance budget:** classification ≤ 1 ms per unit of ≤ 10 KB; the gate adds no network round trip except in tx-confirm mode.
- **Edge cases (exhaustive list):** DML inside routine bodies at creation time (no warning); CTE-wrapped DML (`WITH d AS (DELETE …) SELECT …` → Dml); `WHERE id IN (SELECT …)` (safe); MERGE; DML generated by the grid (it has its own preview, HV-EDIT-09; the gate is skipped for submits that the user previewed, but the read-only and Karagöz checks still apply); `UPDATE t SET a=a WHERE …`; parameters in WHERE (`WHERE id = $1` is safe); a constant-true WHERE created by a template; temporary tables (skippable); `SELECT … INTO new_table` (Dml/Ddl); `nextval`/`setval`/`pg_terminate_backend` in SELECT (Routine); `LISTEN/NOTIFY` (Routine); `VACUUM`/`ANALYZE` (maintenance → Ddl level); `COPY … TO PROGRAM` (Routine with high severity; verified to escape read-only transactions); a user already in a manual transaction (tx-confirm uses a savepoint); a mixed batch (one dialog that summarises it); MySQL `DROP TABLE` inside an executable comment `/*!… */` (classified as code: verified to execute); ClickHouse `ALTER … DELETE` (async mutation, not transactional); a type-to-confirm name with Turkish letters (compared exactly, with no case-folding); a confirm dialog open while another console wants the same session (it waits in the queue; the dialog shows the waiting count).
- **Test cases (input → expected):**
  1. Production PG, `DELETE FROM t WHERE id=1` → RunInTxThenConfirm; the dialog shows "1 row affected"; Roll back → nothing persists.
  2. `DELETE FROM t` on production → Confirm with type-to-confirm `t`.
  3. `EXPLAIN (ANALYZE off) DELETE FROM t` → Run. `EXPLAIN (ANALYZE) DELETE FROM t` → class Dml (HV-EXEC-13 wraps it in a rolled-back tx).
  4. `WITH x AS (UPDATE t SET a=1 RETURNING *) SELECT * FROM x` on production → RunInTxThenConfirm.
  5. MySQL production `DROP TABLE t` → Confirm before running (no tx preview).
  6. Read-only connection, `INSERT …` → Deny `exec-deny-read-only` with class "data modification".
  7. `UPDATE t SET a=1 WHERE 1=1` on dev → Warn (AlwaysTrue).
  8. MySQL `/*!50000 DROP TABLE t */` on a read-only connection → Deny (executable comment treated as code).
- **i18n notes:** `exec-unsafe-title`, `exec-unsafe-no-where = { $statement } has no WHERE clause and changes every row of { $table }.`, `exec-unsafe-always-true`, `exec-unsafe-join-without-on`, `exec-confirm-commit = { $count -> [one] 1 row was changed. *[other] { $count } rows were changed. } Commit or roll back?`, `exec-deny-read-only`, `exec-type-to-confirm`. Buttons `exec-btn-execute`, `-execute-all`, `-execute-suppress`, `-skip`, `-skip-all` need per-locale access keys that do not collide on Turkish layouts (doc 12 / i18n/02 §3.11). Table names in messages are data: shown verbatim, isolated with FSI/PDI.
- **Verification status:** [Observed in Docker, verification/01]: PG read-only transactions do **not** stop `LOCK TABLE`, advisory locks, `pg_terminate_backend`, `NOTIFY`, `SET`, `COPY … TO PROGRAM`/file, `dblink_exec`, `lo_export`, physical replication slots. MySQL executable comments run whole statements. MySQL DDL inside `START TRANSACTION READ ONLY` executes after an implicit commit. So read-only enforcement must combine classification **and** a DB-level read-only mode, and some classes (Routine) must be confirmed or denied by policy. The PG `txid_current_if_assigned()` canary misses `dblink_exec` and `COPY TO PROGRAM` and is not used as a guarantee.
- **Prior art and evidence:** datagrip/07 §3.8–3.9; rainfrog classification (MIT) [Observed: `mod.rs:264-340`]; Beekeeper (behaviour only).
- **Used by:** Hacivat; **Karagöz** (authoritative: org policy "block UPDATE/DELETE without WHERE on production", Deny rules).

### HV-EXEC-05: Session actor and execution queue

- **Priority / depends on:** v1. Depends on the driver trait (doc 21 HV-RES-04); it hosts HV-EXEC-06, -08, -09, -10 and HV-RES-05.
- **Sources merged:** datagrip/07 ALG-EXEC-07 (typed requests, per-session queue, result-set limit), datagrip/07 §3.10 (request kinds, isolated requests), dbeaver-04 §3.4 (one read in flight per viewer, newest request wins), Beekeeper/TUI doc §5 (never block the render loop; start/poll model).
- **Problem:** Serialise all work on one database session (connection) in a predictable order, while keeping the UI responsive, supporting cancellation, cursor fetches, grid DML, transactions and side requests.
- **Inputs / outputs:** requests from consoles, grids and MCP → events to auditors (HV-EXEC-06), result stores (doc 21), UI state (queued / running / done, per unit).
- **Decision:**
  - **One actor per session** (Tokio task owning the connection). A session may be shared by several clients (single-session mode = one transaction across consoles, as in DataGrip). The queue is per session, not per console.
  - **Request kinds:** `Execute(units)`, `FetchMore(result_id, n)`, `CloseCursor(result_id)`, `ApplyDml(batch)` (HV-EDIT-07), `Tx(TxCommand)`, `SwitchSchema`, `Ping`. Cancel is **out of band** (HV-EXEC-09) and never queued.
  - **Coalescing:** queued `FetchMore` requests for the same result merge (sum of n). A queued grid `Reload(result_id)` replaces an older queued `Reload` for the same result (newest wins, DBeaver rule) — but never an `Execute` or `ApplyDml`.
  - **Side connections:** introspection, `COUNT(*)`, related-row lookups, LOB lazy loads and full-value fetches run on a *side* pool of the same data source when the main session is not in an open transaction. If the main session holds an open transaction (manual mode, or a suspended cursor in a transaction), lookups that must see uncommitted rows (LOB load for a just-inserted row, re-select after submit) are queued on the main session. Rejected: always using the main session (blocks behind long queries); always using side connections (cannot see your own uncommitted data).
  - **Cursor ownership:** a suspended server cursor (doc 21 HV-RES-05) belongs to the session. Before any request that needs the connection for something else, the actor runs the cursor-release protocol (drain up to budget, or close and mark truncated). This is the rule that lets us keep cursors open without re-reading from row 0.
- **Algorithm:**
  1. `submit(req)` → assign a `RequestId`, push to the queue, and emit `Queued{position}`. The UI gutter shows "queued (n)".
  2. Actor loop: `loop { req = queue.pop_front().await; if session.suspended_cursor.is_some() && req.needs_connection_exclusively() { release_cursor(policy) } ; run(req) }`.
  3. `run(Execute(units))`: for each unit (respecting `repeat`):
     1. Gate decisions were taken before enqueueing; tx-confirm units run as in HV-EXEC-04 step 7.
     2. `auditors.on_unit_start(unit)`. Start the timers (execution, then fetch).
     3. Call `driver.execute(final_text, binds, ExecOptions{ max_result_sets: 100, fetch_first: page, cancel: token })`. This returns a stream of `ExecEvent`: `ResultSetStart(cols)`, `Rows(chunk)`, `ResultSetEnd{complete}`, `UpdateCount(n)`, `Notice(msg)`, `Error(e)`, `TxStatus(s)`, `Done`.
     4. Route events: rows go to the result store (doc 21 HV-RES-02); metadata goes to auditors; after `max_result_sets`, skip further result sets with a notice (HV-EXEC-07).
     5. After the first `fetch_first` rows of a result set, the fetch policy (HV-RES-05) decides whether to keep reading ahead, suspend the cursor, or drain.
     6. On error → script error policy (HV-EXEC-08).
     7. `auditors.on_unit_end(outcome)`.
  4. After the request: `auditors.on_request_end(summary)`. Output summary line for multi-unit runs (HV-EXEC-11).
  5. Connection loss during a request: mark the session `Broken`, fail the running unit with `exec-connection-lost`, keep the queue paused, and offer Reconnect. Statements are never replayed automatically. If a manual transaction was open, report "transaction state unknown; changes were rolled back by the server unless committed".
  6. The UI side subscribes to a `watch` channel of `SessionState { running: Option<RunningInfo>, queued: u32, tx: TxState, cursor: Option<CursorInfo> }` and redraws from it. It never blocks.
- **Data structures and Rust placement** (`hacivat-core::exec::session`):
  ```rust
  pub struct SessionHandle { tx: mpsc::Sender<QueuedRequest>, state: watch::Receiver<SessionState>, cancel: CancelHandle }
  pub enum Request { Execute(ExecRequest), FetchMore { result: ResultId, rows: u64 }, CloseCursor(ResultId),
                     Reload(ResultId), ApplyDml(DmlBatch), Tx(TxCommand), SwitchSchema(SchemaRef), Ping }
  pub struct SessionState { pub running: Option<RunningInfo>, pub queued: u32, pub tx: TxState,
                            pub cursor: Option<CursorInfo>, pub health: Health }
  #[async_trait] pub trait DriverSession: Send {
      async fn execute(&mut self, sql: &str, binds: &[BindValue], opts: ExecOptions) -> Result<ExecStream, DriverError>;
      fn cancel_handle(&self) -> Box<dyn ServerCancel>;    // usable from another task (HV-EXEC-09)
      fn tx_status(&self) -> TxStatus;
      fn capabilities(&self) -> &Capabilities;
  }
  ```
  The queue is a `VecDeque` behind the actor (not a bare channel) so that it can coalesce and reorder. `tokio::sync::{mpsc, watch}` carry messages.
- **Per-DBMS matrix:** see doc 21 HV-RES-04 for protocol details (multi-statement, cursors, cancel). Session concurrency: PG, MySQL, MSSQL, ClickHouse native = one in-flight statement per connection (pipelining is not used for user SQL). SQLite = one connection per session plus WAL readers on side connections. Redis and Mongo sessions follow the same actor model with their own request kinds.
- **Complexity / performance budget:** enqueue O(1); coalescing O(queue length) (the queue is short). Budget: keypress → request on the wire ≤ 10 ms when idle (excluding dialogs); UI state update ≤ 1 frame after each event; memory per idle session < 64 KB plus driver buffers.
- **Edge cases (exhaustive list):** cancel during connect; a statement that produces rows and update counts; endless result sets (streaming, capped by HV-RES-07); notice floods (rate-limited to 100 per second into the log, the rest counted); connection lost mid-transaction; the user runs another statement while busy (queued with its position shown); a session shared by several clients (single-session mode); `FetchMore` arriving after the result was closed (ignored); `Reload` while an `ApplyDml` is queued for the same result (the reload runs after the DML); a grid DML submit while the console holds a suspended cursor (the cursor-release protocol runs first and says so); the app quitting with queued work (cancel the running unit, drop the queue, prompt for open transactions, HV-EXEC-10); system sleep or resume (health ping before the next request); a driver that panics (catch it at the actor boundary, mark the session Broken); a Karagöz-proxied session (requests go over the P2P channel; the actor model is the same and cancel is forwarded).
- **Test cases (input → expected):**
  1. Cancel `SELECT pg_sleep(60)` within 1 s → "Query canceled"; the session is reusable; the queue is cleared and the dropped requests are marked cancelled.
  2. 3 statements, error in #2, policy Ignore → "2 of 3 statements executed, 1 failed in …".
  3. Two `FetchMore(r, 500)` queued → one driver fetch of 1,000 rows.
  4. Three `Reload(r)` clicks while busy → one reload executes.
  5. Console has a suspended PG cursor at row 10,000 of 1M (manual transaction). The user runs `SELECT 1` → the cursor is closed first; the result banner reads "Cursor closed after 10,000 rows (another statement needed the session)"; `SELECT 1` runs inside the same transaction.
  6. Kill the server mid-query → unit failed with `exec-connection-lost`, session Broken, no automatic replay.
- **i18n notes:** `exec-queued = { $position -> [one] Queued (next) *[other] Queued ({ $position }) }`, `exec-connection-lost`, `exec-cursor-closed-for-request`. Durations are formatted with ICU4X units (HV-EXEC-11).
- **Verification status:** DataGrip request names and registry defaults [Observed: class names, `DatabaseRegistryKeys.xml`]; the DataGrip queue semantics are [Inferred]. The DBeaver newest-wins rule [Observed: `ResultSetViewer.java:4642-4710`]. The cursor-release protocol is our own decision; its MySQL and MSSQL behaviour (KILL QUERY / attention, then drain) is [Inferred] and needs a Docker test (doc 21 HV-RES-04).
- **Prior art and evidence:** rainfrog start/poll and the driver `cancel()` comment (MIT) [Observed: `mod.rs:101-160`, `127-129`]; harlequin exclusive workers (MIT).
- **Used by:** Hacivat; Karagöz (same actor model server-side for proxied sessions).

### HV-EXEC-06: Auditor chain

- **Priority / depends on:** v1. Depends on HV-EXEC-05.
- **Sources merged:** datagrip/07 ALG-EXEC-07 and §3.10 (`DataAuditor`, Timing/Logging/TxMarker/CancelProgress auditors), datagrip/10 ALG-TOOL-03 step 1 (history fed by `on_finish`), Beekeeper/TUI §6 (Karagöz audit events with classification).
- **Problem:** Let many independent observers (timing, output log, transaction badge, gutter, history, Karagöz audit, MCP) see every execution event, in order, without slowing the session actor or each other.
- **Inputs / outputs:** `ExecEvent`s from the actor → side effects per auditor.
- **Decision:**
  - A fixed, ordered chain of **synchronous, O(1)** auditors runs on the actor task: Timing → TxMarker → CancelProgress → GutterState. These update in-memory state only.
  - **Asynchronous** auditors receive a cloned, masked event through a bounded channel each: OutputLog (capacity 10k), History (capacity 10k), McpObserver, and RemoteAudit (Karagöz forwarder, local Karagöz only).
  - **Drop policy:** if an async channel is full, OutputLog and History drop the event and increment a counter. The next event that gets through carries `dropped = n`, and the log shows "n events not shown". RemoteAudit never drops; it applies back-pressure. Karagöz-side audit is authoritative anyway; client history is never an audit trail.
  - Rejected: a single `Vec<Box<dyn Auditor>>` called synchronously (DataGrip-like) — a slow SQLite history write would stall query execution.
- **Algorithm:**
  1. Event model (all carry `session_id`, `request_id`, `unit_id`, a monotonic timestamp and a wall-clock timestamp): `RequestStart`, `UnitStart{masked_sql, masked_binds, classification, origin}`, `ResultSetStart{result_id, columns}`, `RowsFetched{result_id, n, total}`, `ResultSetEnd{result_id, rows, complete, truncated_reason}`, `UpdateCount{n | unknown}`, `Notice{severity, code, text}`, `Error{db_error}`, `TxChange{from, to}`, `CursorState{result_id, state}`, `CancelState{state}`, `UnitEnd{outcome, exec_ms, fetch_ms}`, `RequestEnd{summary}`.
  2. The actor calls `chain.emit(&ev)`. The sync auditors run in order. Then, for each async sink, `try_send(ev.clone_masked())`.
  3. The Timing auditor computes `exec_ms` (statement sent → first response) and `fetch_ms` (first row → last row fetched so far).
  4. The TxMarker auditor updates `TxState` from `TxChange` and from typed `BEGIN`/`COMMIT`/`ROLLBACK` units (HV-EXEC-10).
  5. Masking happens once, at `UnitStart`, using HV-EXEC-03 rules plus the redaction patterns of HV-EXEC-12. Unmasked text never leaves the actor except to the driver.
- **Data structures and Rust placement** (`hacivat-core::exec::audit`, shared with Karagöz):
  ```rust
  pub trait SyncAuditor: Send { fn on_event(&mut self, ev: &ExecEvent, st: &mut SessionScratch); }
  pub struct AsyncSink { name: &'static str, tx: mpsc::Sender<Arc<ExecEvent>>, policy: DropPolicy, dropped: AtomicU64 }
  pub enum DropPolicy { DropAndCount, BackPressure }
  pub struct AuditorChain { sync: SmallVec<[Box<dyn SyncAuditor>; 4]>, sinks: Vec<AsyncSink> }
  ```
- **Complexity / performance budget:** sync chain ≤ 5 µs per event; event clone is an `Arc` bump; budget ≤ 1% of statement time for 1-row statements at 1,000 statements/s (script runs).
- **Edge cases (exhaustive list):** event floods (`RowsFetched` is coalesced to at most 1 per 50 ms per result); notice floods (rate-limited in the actor before the chain); an auditor panic (caught; the auditor is disabled for the session and an internal error is logged); app shutdown with events still in channels (history flushes with a 2 s deadline, then drops); history DB locked or corrupt (sink reports degraded; execution continues); Karagöz unreachable (remote audit back-pressure would stall: the actor fails the request with `exec-audit-unavailable` instead of running unaudited — fail closed); masked SQL longer than 64 KB (truncated in events, the hash is kept).
- **Test cases (input → expected):** 1) 10,000 statements in a script with the history sink artificially slowed to 10 ms per write → execution finishes at full speed; the log shows "n history events not recorded". 2) Event order for `SELECT 1` = RequestStart, UnitStart, ResultSetStart, RowsFetched, ResultSetEnd, UnitEnd, RequestEnd. 3) A remote session with Karagöz audit unreachable → request fails closed with `exec-audit-unavailable`.
- **i18n notes:** events carry no UI text. Only OutputLog renders them through Fluent (HV-EXEC-11). The machine sinks (MCP, Karagöz) get structured, non-localized fields (ALG-I18N-08 step 8).
- **Verification status:** DataGrip auditor names [Observed: class names]; behaviour [Inferred]. The drop and back-pressure policy is a new decision.
- **Prior art and evidence:** datagrip/07 §3.10.
- **Used by:** Hacivat (all observers); Karagöz (the same trait server-side writes the append-only audit log).

### HV-EXEC-07: Multiple result sets, statement outcomes and result tabs

- **Priority / depends on:** v1. Depends on HV-EXEC-05/06 and doc 21 HV-RES-02.
- **Sources merged:** datagrip/07 ALG-EXEC-04 (result-tab titles), ALG-EXEC-07 step 2 (result-set limit 100 and messages), ALG-EXEC-13 (auto-show policy), §3.14 (tabs, pinning, reuse vs new tab); datagrip/11 OR-1…OR-8, QE-4.
- **Problem:** One unit can yield any mix of result sets, update counts, notices and errors (T-SQL batches, MySQL procedures, PG simple-protocol multi-statements). Each must be captured, named, shown and kept or replaced predictably.
- **Inputs / outputs:** the event stream of a unit → `UnitOutcome { results: Vec<ResultHandle>, update_counts: Vec<UpdateCount>, notices, error }` and tab operations.
- **Decision:**
  - A cap of **100 result sets per unit** (DataGrip `database.result.set.limit`), configurable. Beyond it, skip the extra result sets (still drain them so the protocol stays in sync) and log "only 100 of N result sets retrieved".
  - One result tab per result set. A tab is **reused** by default for the same console + unit position unless it is pinned. "Open results in new tab" is a per-execute-action setting (default off; DataGrip's own default is contradictory between help and screenshot).
  - Titles: from the preceding comment if enabled (default **on**, marker empty); else the qualified source table (single-table SELECT); else `Result N`. Further result sets of one unit get the suffix `(2)`, `(3)`.
  - Auto-show of the results panel follows DataGrip's 5 levels; default "For errors and result sets".
- **Algorithm:**
  1. Per unit, keep an ordered `Vec<OutcomeItem>`: `ResultSet(result_id)`, `UpdateCount(n | Unknown)`, `Notice`, `Error`.
  2. For each `ResultSetStart`: if `count == max_result_sets`, mark `skip = true` (consume rows without storing them) and log once `exec-result-set-limit`; else create a result store (doc 21) and a tab request.
  3. Tab placement: key = `(client_id, unit_ordinal, result_ordinal)`. If an unpinned tab with that key exists and "new tab" is off → reuse it: bump the generation, keep the column widths when the column names are unchanged, and preserve the scroll position when the row count allows. Else open a new tab.
  4. Title:
     1. If title-from-comment is on: take the comment block that directly precedes the statement inside its smart range (no blank line). With a non-empty marker, the title is the text after the marker's first occurrence up to the end of the line or comment; if the marker is absent, use no comment title. With an empty marker, the title is the first non-empty line of the comment text, with `--`, `/*`, `*/` stripped and whitespace collapsed. Optimizer hints `/*+ … */` never count as titles.
     2. Else, or if empty: single-table SELECT → qualified name, shortened per "shorten" settings; else `Result { $n }` (Fluent).
     3. Suffix `(k)` for result set k ≥ 2. Truncate the display at 40 graphemes; the full title is in the tooltip. Duplicate titles are allowed.
  5. Statements without result sets write only to the output log (HV-EXEC-11).
  6. Auto-show: show the results panel iff `level ≥ required(kind)` with `ResultSet` → "For result sets", `Error` → "For errors and result sets", `QueryOutput` → "For query output, errors and result sets", any output → "For all output". Switch the panel's selected client to the originating console only if "stay on selected console" is off or the origin is the selected console. Move keyboard focus only if the panel is floating and focus-on-show is on.
  7. In-editor results mode (HV-EXEC-16) redirects result sets inline; the panel then shows only errors and output.
- **Data structures and Rust placement** (`hacivat-core::exec::outcome` + `hacivat-app::results_tabs`):
  ```rust
  pub enum OutcomeItem { ResultSet(ResultId), UpdateCount(Option<u64>), Notice(DbNotice), Error(DbError) }
  pub struct UnitOutcome { pub unit: UnitId, pub items: Vec<OutcomeItem>, pub skipped_result_sets: u32 }
  pub struct TabKey { client: ClientId, unit_ordinal: u32, result_ordinal: u32 }
  pub fn title_from_comment(stmt_leading_trivia: &[Token], marker: &str) -> Option<String>;
  ```
- **Per-DBMS matrix (how multiple outcomes arise):**

  | DBMS | Multiple result sets from one unit | Update counts | Notices |
  |---|---|---|---|
  | PostgreSQL | simple-protocol multi-statement text (no binds); a function returning refcursors (opened on demand) | `CommandComplete` tag per statement | `NoticeResponse` (async, interleaved) |
  | MySQL/MariaDB | `CLIENT_MULTI_STATEMENTS`, `CALL` (procedure results + final OK) | OK packet affected rows (needs `CLIENT_FOUND_ROWS` for edits, doc 23) | `SHOW WARNINGS` after the statement if warning count > 0 |
  | SQL Server | every SELECT in the batch; `DONE`/`DONEPROC` row counts unless `SET NOCOUNT ON` | `DONE` tokens | `INFO` tokens (class ≤ 10), `PRINT`, `RAISERROR` severity ≤ 10 |
  | SQLite | one per statement (we split) | `changes()` | none |
  | ClickHouse | one per query | `written_rows` from progress/summary | server logs (optional) |
  | Oracle (later) | implicit results (12c+), refcursors | rows processed | `DBMS_OUTPUT` polled after the statement when enabled |
- **Complexity / performance budget:** title computation O(comment length); tab creation ≤ 1 frame. Skipped result sets are drained at network speed.
- **Edge cases (exhaustive list):** a unit with 0 result sets and 0 counts (DDL → "completed in"); a result set with 0 columns (some drivers report one for DDL → treat as none); a result set with 0 rows (header-only grid); T-SQL procedures interleaving result sets and errors (errors with severity ≥ 11 stop the batch only for some severities — record in order); MySQL procedure multi-results plus the trailing OK; PG `RETURNING` (a result set from DML); `EXPLAIN` (a result set routed to the plan view, HV-EXEC-13); 101+ result sets (cap); a reused tab where the new result has different columns (reset widths, keep pinned state); a pinned tab (never reused); a comment title containing a marker twice (first wins); a comment on the previous statement's line (belongs to the previous statement); a doc comment `/** */`; titles with Turkish characters (no case change); two consoles finishing at the same time (auto-show selects per OR-8); a result arriving for a console that was closed (dropped; the result is freed).
- **Test cases (input → expected):** TITLE-01…06 (doc 13); `-- Top customers\nSELECT …` → title "Top customers"; marker `@title` with `/* run @title Monthly */` → "Monthly"; T-SQL `SELECT 1; SELECT 2; UPDATE t SET a=1 WHERE 0=1;` → tabs "Result 1", "Result 1 (2)", log "0 rows affected"; 150 result sets → 100 tabs plus the log line "The query yielded 150 result sets, only 100 were retrieved"; level "For result sets" and the statement errors → the panel is not shown; the error is highlighted in the editor.
- **i18n notes:** `results-tab-default = Result { $n }`, `exec-result-set-limit = The query yielded { $total } result sets; only { $limit } were retrieved.` Titles from comments are user text: never translated or case-mapped.
- **Verification status:** limit 100 [Observed: registry]. Title rules [Observed: help]; the default of "Open results in new tab" is contradictory in DataGrip [Unknown → SET family]; Hacivat chooses off.
- **Prior art and evidence:** datagrip/07 §3.14, §3.15; datagrip/11 §3.3.
- **Used by:** Hacivat; MCP (results returned as typed pages per result set).

### HV-EXEC-08: Script error policy

- **Priority / depends on:** v1. Depends on HV-EXEC-05, -07, -10.
- **Sources merged:** datagrip/07 ALG-EXEC-12 and §3.11 (Retry / Ignore / Ignore All / Stop, "(aborted!)"), dbeaver-05 ALG-DBV-XFER-04 (the PG "transaction aborted" class of bug), datagrip/13 ERR family.
- **Problem:** When one unit of a multi-unit run fails, decide what happens to the rest, correctly for databases that abort the whole transaction after an error.
- **Inputs / outputs:** the failed unit, its error, the remaining queue, the run policy, `TxState` → next action and summary flags.
- **Decision:**
  - Policies: `Ask` (default for interactive multi-unit runs), `StopOnError` (default for run configurations, CLI and MCP scripts), `IgnoreAll`.
  - Ask shows an **inline bar** (not a modal) with Retry, Ignore, Ignore All, Stop.
  - PG-specific rule: inside an explicit transaction, after an error the transaction is in the aborted state, so "Ignore" would make everything after it fail. Offer **"Roll back and continue"** and **"Roll back to savepoint and continue"**. The second appears when Hacivat runs each unit under an automatic savepoint, which is opt-in (default off: it costs a round trip per statement).
  - Single-unit runs never prompt.
- **Algorithm:**
  1. On `Error` for unit k: if the run has 1 unit → finish with the error. If the policy is IgnoreAll → mark failed, continue. If StopOnError → mark failed, drop the queue, summary `aborted = true`.
  2. If Ask: pause the queue and show the bar.
     - Retry: re-send unit k. The user may have edited it in the meantime; resolve units again from the new document and re-send only k's new text if its range still exists, else Stop.
     - Ignore: continue with k+1. Ignore All: set the policy to IgnoreAll and continue. Stop: drop the queue, `aborted = true`.
  3. PG in an explicit transaction (`TxState.failed = true` after the error): replace Ignore by "Roll back and continue" (`ROLLBACK`, then continue in autocommit or in a new transaction according to the mode) and, when automatic savepoints are on, "Roll back to savepoint and continue".
  4. `GO n` repeated batch: an error stops the remaining repeats of that batch, then the policy applies.
  5. Errors that are warnings (MySQL warnings, T-SQL severity ≤ 10) never trigger the policy.
  6. Connection loss: Retry is offered only after a successful reconnect; the policy otherwise behaves like Stop.
  7. Summary (HV-EXEC-11): `executed`, `total`, `failed`, `aborted`.
- **Data structures and Rust placement** (`hacivat-core::exec::errors`): `pub enum ErrorPolicy { Ask, StopOnError, IgnoreAll }`; `pub enum ErrorChoice { Retry, Ignore, IgnoreAll, Stop, RollbackAndContinue, RollbackToSavepointAndContinue }`.
- **Per-DBMS matrix:**

  | DBMS | Transaction after an error inside an explicit tx | Ignore safe? |
  |---|---|---|
  | PostgreSQL | aborted (`25P02` for later statements) | no → roll back or use a savepoint |
  | MySQL/MariaDB | statement rolled back, tx continues (deadlock rolls back the whole tx) | yes (except deadlock `1213`) |
  | SQL Server | depends on `XACT_ABORT` and severity; may become uncommittable (`XACT_STATE() = -1`) | check `@@TRANCOUNT` / XACT_STATE after the error |
  | SQLite | statement rolled back; tx continues (some errors such as `SQLITE_FULL` roll back the tx) | mostly yes |
- **Complexity / performance budget:** the bar appears within one frame of the error event.
- **Edge cases (exhaustive list):** an error inside an AsOne multi-statement text (partial server-side execution: report the update counts already received); an error in the last unit (no prompt; summary); deadlock or serialization failure (`40P01`, `40001`, MySQL `1213`) → also offer "Retry transaction" when the whole run is one transaction; the user edits the document while the bar is shown; cancel pressed while the bar is shown (= Stop); an error in a confirm-in-tx unit (the tx is rolled back; continue per the policy); MCP-originated runs (no UI: StopOnError); an error flood in IgnoreAll (each logged; the summary counts them).
- **Test cases (input → expected):** 3 statements, #2 fails, Ignore → "2 of 3 statements executed, 1 failed"; Stop → "1 of 3 statements executed, 1 failed (aborted)"; PG `BEGIN; INSERT ok; INSERT dup; INSERT ok2;` with Ask → the bar offers "Roll back and continue" instead of "Ignore"; choosing it → ok2 runs in autocommit and the summary shows that the tx was rolled back; MySQL in the same scenario → Ignore is offered and ok2 commits with the tx.
- **i18n notes:** `exec-error-bar = Statement { $index } of { $total } failed.`, buttons `exec-error-retry`, `-ignore`, `-ignore-all`, `-stop`, `-rollback-continue`, `-rollback-savepoint-continue`. The server error text is verbatim (ALG-I18N-08).
- **Verification status:** button labels [Observed: bundle]; the DataGrip flow is [Inferred] (ERR family not yet run). The PG aborted-transaction behaviour is standard PG semantics [Inferred: PG docs]; the DBeaver bug #41291 shows the same class.
- **Prior art and evidence:** datagrip/07 §3.11.
- **Used by:** Hacivat; Karagöz (unattended scripts use StopOnError).

### HV-EXEC-09: Cancellation (client + server, with escalation)

- **Priority / depends on:** v1. Depends on HV-EXEC-05 and the driver `ServerCancel` capability.
- **Sources merged:** datagrip/07 ALG-EXEC-11 and §3.12 (three-stage escalation: cancel → slow-cancel balloon after ~10 s → Deactivate dialog), Beekeeper/TUI ALG-EXEC-02 (record the server session id first, cancel via a separate channel, discard a connection in an unknown state, PID-reuse race), dbeaver-04 §3.4 (detach a stuck read job after 5 s so that the UI never blocks).
- **Problem:** Stop running and queued work promptly, free server resources, and keep the session usable. Escalate clearly when the server does not respond. Never block the UI.
- **Inputs / outputs:** a cancel command (UI, Esc, MCP, Karagöz) → `CancelOutcome { Cancelled | AlreadyFinished | Escalated(stage) | Failed(reason) }` and the session's health.
- **Decision:** DataGrip's escalation plus a real server-side **terminate** (DataGrip only disconnects; its own dialog warns that the statement may keep running).
  - States: `Idle → Running → Cancelling(t0) → SlowCancel → (Terminating | Deactivating) → Idle`.
  - Protocol-level cancel first (it carries the session's secret key and avoids the PID-reuse race). Kill/terminate statements use a side connection.
  - Rejected: client-only abort (leaves the server busy); PID-only `pg_cancel_backend` as the first step (PID reuse race).
- **Algorithm:**
  1. Before executing, the driver records its cancel credentials: PG `CancelToken` (backend PID + secret key), MySQL `CONNECTION_ID()`, MSSQL SPID (and the TDS attention capability), ClickHouse `query_id` (set by us, a UUID per unit), MongoDB `comment` tag / opId, Redis client id (for `CLIENT UNBLOCK`), SQLite interrupt handle.
  2. Cancel pressed in Running (stage 1):
     1. Mark queued requests of this session as cancelled and drop them (`exec-cancelled`).
     2. Send the server cancel from a separate task: PG CancelRequest; MySQL `KILL QUERY <id>` on a side connection; MSSQL attention signal; ClickHouse `KILL QUERY WHERE query_id = …`; Mongo `killOp`; SQLite `interrupt()`; Redis `CLIENT UNBLOCK <id> ERROR` for blocking commands.
     3. Stop consuming the client stream for the running result; rows already received stay browsable (the result is marked `cancelled`).
     4. State `Cancelling(t0)`; gutter shows a red square; UI shows "Cancelling…".
  3. If the driver reports cancellation or completion → "Query canceled" or "finished before cancel" → Idle. If the protocol is mid-stream in an unknown state after T = 2 s (for example the cancel arrived between messages), discard the connection and reconnect transparently, unless the session holds an open transaction (then mark it Broken and tell the user that the transaction is lost).
  4. At `now − t0 ≥ 10 s` → a non-modal notification "Cancelling takes too long" with actions [Terminate on server] (when supported and permitted) and [Deactivate]. State `SlowCancel`.
  5. A second Cancel press in Cancelling or SlowCancel → dialog "The statement does not respond to cancel" with **Keep Waiting**, **Terminate on server** (PG `pg_terminate_backend(pid)`, MySQL `KILL <id>`, MSSQL `KILL <spid>` via a side connection; requires privileges; failures show the server's error), **Deactivate** (close all connections of the data source; warn that the statement may continue server-side).
  6. Cancel during Connecting → abort the handshake → "Connection canceled".
  7. In a manual transaction on PG, a cancelled statement aborts the transaction: show "Transaction aborted — roll back" (HV-EXEC-10).
  8. Remote (Karagöz) sessions: the cancel is forwarded to Karagöz, which owns the DB connection and performs steps 2–5 server-side. Karagöz accepts cancel and terminate only from the session owner or an admin.
- **Data structures and Rust placement** (`hacivat-core::exec::cancel`):
  ```rust
  pub trait ServerCancel: Send + Sync {
      fn cancel(&self) -> BoxFuture<'static, Result<(), DriverError>>;      // protocol-level / KILL QUERY
      fn terminate(&self) -> Option<BoxFuture<'static, Result<(), DriverError>>>; // kill session (side conn)
  }
  pub enum CancelState { Idle, Cancelling { since: Instant }, SlowCancel { since: Instant }, Terminating, Deactivating }
  ```
  `CancelHandle` is cloneable and `Send`. The UI holds one and never awaits it on the UI thread (it spawns a task).
- **Per-DBMS matrix:**

  | DBMS | Cancel | Terminate | Session id source | Notes |
  |---|---|---|---|---|
  | PostgreSQL | CancelRequest (tokio-postgres `CancelToken`) | `pg_terminate_backend(pid)` | BackendKeyData | same-role cancel allowed; terminating other roles needs `pg_signal_backend` |
  | MySQL/MariaDB | `KILL QUERY id` (side conn) | `KILL id` | `CONNECTION_ID()` | requires `CONNECTION_ADMIN`/`PROCESS` for other users' threads |
  | SQL Server | TDS attention | `KILL spid` | `@@SPID` | attention then drain to `DONE` with the attention ack |
  | SQLite | `sqlite3_interrupt` (rusqlite `InterruptHandle`) | close the connection | n/a | interrupts long C loops |
  | ClickHouse | `KILL QUERY WHERE query_id=…` | same with `SYNC` | our `query_id` | HTTP: closing the request also cancels (setting dependent) |
  | MongoDB | `killOp` (opId found via `$currentOp` filtered by our comment) | `killSessions` | comment tag | needs `killop` privilege for other users |
  | Redis | `CLIENT UNBLOCK id ERROR` (blocking commands only) | `CLIENT KILL ID id` | `CLIENT ID` | long Lua scripts: `SCRIPT KILL` (only if no writes yet) |
- **Complexity / performance budget:** cancel request sent ≤ 50 ms after the keypress; UI shows "Cancelling…" within one frame; typical PG `pg_sleep` cancel completes ≤ 1 s; never block the UI thread (target 0 ms of UI-thread waiting).
- **Edge cases (exhaustive list):** the query finished between the click and the cancel (AlreadyFinished); cancel during result streaming (client-side stop plus server cancel for an unbuffered stream); a cancel that reaches the server for the *next* query (protocol keys avoid it; KILL QUERY by id has a race window — use KILL QUERY only while our own state is Running and re-check `CONNECTION_ID` mapping); a network partition (the cancel channel is unreachable → Deactivate is the only option); a driver without cancel (close the connection); a cancel arriving after completion (ignored); a cancel in manual tx mode (PG tx aborted); several consoles sharing a session (cancel affects all queued work of that session — the dialog says so); missing privileges for terminate (show the server error verbatim); long DDL that cannot be interrupted (server-side; escalation explains this); MySQL streaming result being drained after KILL QUERY (fast, server stopped sending); MSSQL attention while no request is active (ignored); SQLite interrupt while the statement is between steps (the next step returns `SQLITE_INTERRUPT`); cancel of a Karagöz-proxied query while the P2P link is down (falls back to the relay; if both are down, Deactivate locally and Karagöz's own idle-statement timeout applies); app quit with running statements (cancel all, 2 s grace, then close).
- **Test cases (input → expected):** PG `SELECT pg_sleep(60)` cancelled at 1 s → done ≤ 2 s, session usable, "Query canceled"; MySQL `SELECT SLEEP(60)` → `KILL QUERY` sent on a side connection, session usable; a lock wait that ignores cancel (simulated by blocking the cancel path) → after 10 s the notification appears; the second press opens the dialog; Terminate on PG with privileges → backend gone, session reconnects, "Session terminated on server"; cancel after completion → AlreadyFinished and no error; SQLite long recursive CTE → interrupted, connection preserved.
- **i18n notes:** `exec-cancelling`, `exec-cancelled = Query canceled`, `exec-cancel-slow = Cancelling takes longer than { $seconds } seconds.`, `exec-cancel-dialog-title`, `exec-cancel-keep-waiting`, `exec-cancel-terminate`, `exec-cancel-deactivate`, `exec-cancel-deactivate-warning = The statement may continue to run on the server.`, `exec-connection-canceled`.
- **Verification status:** DataGrip escalation stages and messages [Observed: help, bundle]; DataGrip does not issue server-side kill [Inferred from dialog text]. rainfrog's cancel test [Observed: `cancellation_interrupts_native_work_and_preserves_the_session`]. Per-DBMS privilege notes are [Inferred: vendor docs]; a Docker test is required before v1 (PG, MySQL 8.0/8.4, MSSQL 2022, ClickHouse 24.8 — the same images as verification/01).
- **Prior art and evidence:** datagrip/07 §3.12; rainfrog (MIT) `pg_cancel_backend` and DuckDB interrupt.
- **Used by:** Hacivat; MCP `cancel_query`; Karagöz (server side).

### HV-EXEC-10: Transaction state tracking

- **Priority / depends on:** v1. Depends on HV-EXEC-05/06.
- **Sources merged:** datagrip/07 ALG-EXEC-15 and §3.13 (Auto/Manual, isolation, badge, close-with-open-transaction prompt, single-session mode), datagrip/08 §3.10 (grid Submit and Commit, Roll Back reverts pending edits), datagrip/07 §3.24 (protocol tx status per driver).
- **Problem:** Always know, per session, whether a transaction is open, aborted or idle — including transactions opened by typed `BEGIN` — and drive the Tx UI and the safety prompts from that.
- **Inputs / outputs:** protocol status after every round trip, typed tx statements, mode switches → `TxState` (watched by the UI).
- **Decision:** `TxState` is derived from the **protocol status** after each request (authoritative), not from parsing alone; the parse of typed statements only gives early hints. Manual mode is per session. A badge is shown even in Auto mode when a typed `BEGIN` opened a transaction. Grid edits never auto-commit when the session is in Manual mode.
- **Algorithm:**
  1. `TxState { mode: Auto | Manual, isolation: Default | RU | RC | RR | S | Snapshot, open: bool, failed: bool, statements: u32, since: Option<Instant>, savepoints: Vec<Name> }`.
  2. Switching mode: if `open` → refuse with `tx-change-in-progress` ("Commit or roll back first"); else set autocommit on the connection (MySQL `SET autocommit`; MSSQL `SET IMPLICIT_TRANSACTIONS`; PG and SQLite emulate Manual mode by sending `BEGIN` before the first statement after each commit or rollback).
  3. After every request: read the protocol status (PG ReadyForQuery `I/T/E`; MySQL `SERVER_STATUS_IN_TRANS`; MSSQL `@@TRANCOUNT` via ENVCHANGE tokens; SQLite `get_autocommit()`). Update `open` and `failed` (`E`). Emit `TxChange`.
  4. Typed statements: `BEGIN/START TRANSACTION/SAVEPOINT/RELEASE/ROLLBACK TO/COMMIT/ROLLBACK` update the savepoint list. MySQL/Oracle implicit commits from DDL are seen through the status in step 3.
  5. UI: toolbar label `Tx: Auto` / `Tx: Manual`; badge with the statement count and age when `open`; Commit and Roll back visible in Manual or when `open`; "Transaction aborted" warning when `failed`.
  6. Closing a console or tab, disconnecting, switching data source, or quitting with `open` → prompt Commit / Roll back / Cancel. Unattended (MCP, CLI) → roll back and report.
  7. Idle-in-transaction reminder: after `tx_idle_warning` (default 10 min) with `open` and no activity → a non-modal reminder ("Transaction open for 10 min; it holds locks"). Karagöz can enforce an org idle-in-transaction timeout.
  8. Isolation change while `open` → refuse.
- **Data structures and Rust placement** (`hacivat-core::exec::tx`): `TxState` as above; `pub enum TxCommand { Begin, Commit, Rollback, Savepoint(Name), RollbackTo(Name), Release(Name), SetMode(TxMode), SetIsolation(Isolation) }`.
- **Per-DBMS matrix:**

  | DBMS | Status source | Manual mode implementation | DDL ends tx? | After error |
  |---|---|---|---|---|
  | PostgreSQL | ReadyForQuery I/T/E | `BEGIN` before the first statement | no | aborted (E) |
  | MySQL/MariaDB | `SERVER_STATUS_IN_TRANS` | `SET autocommit=0` | yes (implicit commit) | statement-level rollback |
  | SQL Server | `@@TRANCOUNT` / ENVCHANGE begin/commit/rollback | `SET IMPLICIT_TRANSACTIONS ON` | no | depends on `XACT_ABORT` |
  | SQLite | `sqlite3_get_autocommit` | `BEGIN` (DEFERRED; IMMEDIATE when writing is expected) | no | mostly statement-level |
  | ClickHouse | none (no interactive tx) | Manual not offered | — | — |
- **Complexity / performance budget:** O(1) per request; no extra round trips (MSSQL reads the status from tokens already on the wire).
- **Edge cases (exhaustive list):** `BEGIN` typed in Auto mode (badge shown, prompt on close); MySQL DDL implicit commit (`open` becomes false; log "implicitly committed by DDL"); PG aborted state; isolation change mid-tx; savepoints and `ROLLBACK TO`; a session shared across consoles (the badge shows on all clients); connection lost with an open tx (state unknown → "rolled back by the server unless it was committed"); a server-side `idle_in_transaction_session_timeout` kills the session (connection-lost flow with a specific message when the error code says so); XA/2PC transactions (shown as open; commit and rollback refused with a hint); an MSSQL nested `BEGIN TRAN` (`@@TRANCOUNT` > 1; Commit decrements); a grid submit in Manual mode (no commit); a suspended cursor inside a transaction (closing the tx closes the cursor: HV-RES-05).
- **Test cases (input → expected):** TX family (doc 13); switch to Manual while `open` → refused; PG `BEGIN; SELECT 1/0;` → `failed = true`, warning; MySQL `BEGIN; INSERT …; CREATE TABLE x(i int);` → `open = false` after the DDL, log line "implicitly committed"; close a console with `open` → 3-button prompt; MCP session ends with `open` → rolled back, reported in the tool result.
- **i18n notes:** `tx-label = Tx: { $mode }`, `tx-mode-auto`, `tx-mode-manual`, `tx-isolation-*`, `tx-change-in-progress`, `tx-close-prompt`, `tx-aborted`, `tx-idle-reminder = Transaction open for { $minutes } min; it may hold locks.`, `tx-implicit-commit`.
- **Verification status:** DataGrip labels [Observed: bundle]. MySQL DDL implicit commit inside `START TRANSACTION READ ONLY` [Observed: verification/01 #13] — confirms that the status must be read from the protocol. PG aborted state [Inferred: PG docs].
- **Prior art and evidence:** datagrip/07 §3.13, §3.24.
- **Used by:** Hacivat (console, grid, MCP); HV-EDIT-07; Karagöz (idle-in-transaction policy).

### HV-EXEC-11: Output log and message formatting

- **Priority / depends on:** v1. Depends on HV-EXEC-06 and i18n ALG-I18N-08.
- **Sources merged:** datagrip/07 ALG-EXEC-10 and §3.15 (event formats: rows retrieved, rows affected, summary, aborted), i18n/02 ALG-I18N-08 (server message presentation), i18n/01 (Fluent), datagrip/10 §6 (ChoiceFormat anti-pattern).
- **Problem:** Render every execution event as a readable, translatable log line with correct plurals and optional parts, without English-centric concatenation, while keeping the server's own text verbatim and copied logs locale-invariant.
- **Inputs / outputs:** async auditor events → `LogLine { ts, severity, fluent_id, args, server_text: Option<String>, source_range: Option<DocRange> }` rendered in the UI; plain-text copy.
- **Decision:** One Fluent message per sentence shape with named arguments and selectors (`$rows`, `$hasStart`, `$failed`, `$aborted`). No string concatenation of fragments. Durations via ICU4X unit formatting in the UI; **copy/export of the log uses invariant formats** (ISO timestamps, ASCII digits, `ms`). Server errors follow ALG-I18N-08: localized explanation + code chip + verbatim server text + editor position.
- **Algorithm:**
  1. Map events: `ResultSetEnd` → `exec-rows-retrieved { $rows, $first, $duration, $execMs, $fetchMs }` (`$first` omitted when 1; a variant for `$rows = 0`); `UpdateCount(Some n)` → `exec-rows-affected { $rows, $duration }`; `UpdateCount(None)` → `exec-completed { $duration }`; `TxChange` → `exec-tx-committed` / `-rolled-back`; `RequestEnd` multi-unit → `exec-summary { $executed, $total, $failed, $aborted, $duration }`; script runs add `$symbols`.
  2. Duration: < 1 s → "N ms"; < 60 s → "N s M ms"; else "N min S s" (ICU4X duration/unit formatting; fallback Fluent `NUMBER()` with unit messages).
  3. Timestamps prefix (setting, default off): UI uses the locale's medium time; copy uses `yyyy-MM-dd HH:mm:ss`.
  4. Errors: build `DbErrorView` per ALG-I18N-08 (title from the SQLSTATE/errno family, code chip, verbatim text sanitized, detail/hint/context, `lang` hint). Map the server position through the unit's `OffsetMap` to the document and add an editor squiggle.
  5. Notices: severity-colored, verbatim, with a code chip; rate-limited (HV-EXEC-05).
  6. Log storage: ring buffer per session (default 10,000 lines); "Clear", "Copy", "Soft wrap", "Scroll to end", "Pause output".
- **Data structures and Rust placement:** `hacivat-core::exec::log` (model, event→message mapping, invariant formatter); `hacivat-app::output_view` (GPUI list using `uniform_list`).
- **Complexity / performance budget:** formatting ≤ 20 µs per line; the log view renders 10k lines virtualized at 60 fps.
- **Edge cases (exhaustive list):** 0 rows (no "starting from"); paging start > 1 ("starting from 501"); counts ≥ 2^31 and ≥ 2^53 (u64 formatting, never via f64); unknown update count (driver -1 → "completed in"); Turkish: no singular/plural difference after numerals ("1 satır", "5 satır") but messages still go through plural selectors; durations under 1 ms ("<1 ms"); clock adjustments (durations from a monotonic clock); server text with bidi controls (shown visibly), ANSI escapes (stripped), invalid UTF-8 (U+FFFD plus raw hex in details); very long server messages (capped at 16 KiB with "show more"); PG positions beyond the query length (clamped); PG position in a unit rewritten with binds (mapped through the `OffsetMap`); a server message language different from the UI (explanation in the UI language, server text marked `lang`).
- **Test cases (input → expected):** (1 row, start 1, 131 ms, en) → "1 row retrieved starting from 1 in 131 ms"; (executed 2, total 3, failed 1, 1,020 ms) → "2 of 3 statements executed, 1 failed in 1 s 20 ms"; (3, 3, 0, aborted) → "3 statements executed in … (aborted)"; tr (5 rows) → "5 satır alındı …" (wording illustrative); PG `23505` with Turkish server text and an English UI → English title "Unique constraint violated: users_email_key", chip `SQLSTATE 23505`, Turkish server text marked `lang="tr"`; copy of a line with the timestamp setting on → ISO timestamp regardless of locale.
- **i18n notes:** the message ids listed above live in `hacivat-core.ftl` (core strings) because MCP and CLI reuse them in the human-readable channel; error explanation ids `db-pg-sqlstate-*`, `db-mysql-errno-*`, `db-mssql-error-*`, `db-sqlite-code-*` follow ALG-I18N-08 naming, and CI checks that every curated code has an English message.
- **Verification status:** DataGrip formats [Observed: bundle]; ALG-I18N-08 facts on PG `V`/`C` fields and `lc_messages` privileges [Observed: PG 18 docs]; fluent-rs `NUMBER()` is locale-insensitive [Observed: verification/02 P31] → numbers in messages must be pre-formatted by ICU4X and passed as strings, or a custom Fluent function must be registered.
- **Prior art and evidence:** datagrip/07 §3.15; i18n/02 ALG-I18N-08.
- **Used by:** Hacivat; Karagöz audit summaries (non-localized fields); MCP (structured, non-localized).

### HV-EXEC-12: Query history store

- **Priority / depends on:** v1. Depends on HV-EXEC-06 (async History sink).
- **Sources merged:** datagrip/10 ALG-TOOL-03 (SQLite + FTS5 model, dedup, redaction, retention, prefix recall), datagrip/10 §3.1 (DataGrip's lossy text-list model: exact-text move-to-end dedup, 300 entries per data source), ALG-TOOL-13 (link to revision snapshots), i18n/02 ALG-I18N-06 (Turkish-tolerant search), Beekeeper/TUI §6 (history and favourites).
- **Problem:** Record every executed statement with context and outcome; find it again quickly, including with Turkish and accent-insensitive search; never store secrets; never become an "audit log".
- **Inputs / outputs:** `UnitStart`/`UnitEnd` events (masked) → rows in `history.sqlite`; queries from the history dialog, editor recall and MCP.
- **Decision:** ALG-TOOL-03 as specified, with these choices:
  - **Separate SQLite file** per Hacivat profile (`history.sqlite`, WAL), a single writer task; retention 10,000 entries per data source and 90 days by default; pinned entries never expire.
  - Exact-text dedup per data source (DataGrip-compatible "move to end") **plus** a `fingerprint` for "similar" grouping.
  - Origins recorded: console, file, grid DML, MCP, run configuration, import. Grid DML is recorded only as a summary (`UPDATE x SET … (3 rows)`) with the full SQL kept in the entry details.
  - FTS5 with `unicode61 remove_diacritics 2` plus our own query normalization (ALG-I18N-06 Turkish-tolerant key) applied to both the indexed shadow column and the query. FTS5 alone does not fold `İ/ı` to `i` the way we need.
  - Rejected: DataGrip's text-file list (loses time, outcome, duration). Rejected: storing raw (unmasked) text.
- **Algorithm:**
  1. On `UnitEnd`, the sink builds `HistoryRecord { ts_start, duration_ms, data_source_id, database, schema, client_id, file_path?, origin, statement_text (masked), text_hash (blake3 of the masked text), fingerprint, params_json (masked), outcome, rows_affected, rows_fetched, error_code, error_message (≤ 4 KiB), tx_mode, script_run_id? }`.
  2. Redaction before hashing: dialect patterns `PASSWORD '…'`, `IDENTIFIED BY …`, `WITH PASSWORD = '…'`, connection strings with `password=` / `pwd=`, `CREATE USER … PASSWORD`, `ALTER LOGIN … WITH PASSWORD`, `SET PASSWORD`, and bind values masked by HV-EXEC-03. Set `redacted = true`.
  3. Upsert: if `(data_source_id, text_hash)` exists → `run_count += 1`, update `last_run_ts` and the latest outcome and metrics; else insert.
  4. Fingerprint: strip comments, collapse whitespace, ASCII-case-fold keywords only (identifiers keep their case), replace literals and placeholders with `?`, hash.
  5. Retention: a background job every 10 min deletes the oldest unpinned rows per data source beyond the cap and rows older than the max age, in batches of 1,000.
  6. Search: FTS5 `MATCH` on the normalized shadow column (`search_key`) + filters (data source, outcome, origin, date range, minimum duration); rank by bm25, then recency; "Similar" = same fingerprint.
  7. Editor recall: Up/Down on the last line cycles the entries of the current data source whose text starts with the text before the caret, newest first.
  8. Private consoles and per-data-source "do not record" → the sink drops the event.
  9. Import of DataGrip history (`consoles/.history/db/<uuid>.sql`, entries separated by the platform separator line): split, mark `imported`, timestamps unknown (null).
- **Data structures and Rust placement** (`hacivat-core::history`, `rusqlite` with the bundled SQLite and FTS5 enabled):
  ```sql
  CREATE TABLE history(id INTEGER PRIMARY KEY, ts_start INTEGER, last_run_ts INTEGER, duration_ms INTEGER,
    data_source_id TEXT, database TEXT, schema TEXT, client_id TEXT, file_path TEXT, origin TEXT,
    statement_text TEXT, search_key TEXT, text_hash BLOB, fingerprint BLOB, params_json TEXT,
    outcome TEXT, rows_affected INTEGER, rows_fetched INTEGER, error_code TEXT, error_message TEXT,
    tx_mode TEXT, run_count INTEGER DEFAULT 1, pinned INTEGER DEFAULT 0, redacted INTEGER, script_run_id TEXT,
    UNIQUE(data_source_id, text_hash));
  CREATE VIRTUAL TABLE history_fts USING fts5(search_key, error_message, content='history', content_rowid='id',
    tokenize='unicode61 remove_diacritics 2');
  CREATE INDEX history_ds_recent ON history(data_source_id, last_run_ts DESC);
  CREATE INDEX history_fp ON history(fingerprint);
  ```
- **Complexity / performance budget:** insert O(log n), ≤ 1 ms on the writer task (never on the actor); search ≤ 50 ms p95 over 100k rows; recall ≤ 5 ms; history DB size ≤ 50 MB at the default caps (statements are capped at 64 KB each, with a zstd-compressed full text optional for longer ones).
- **Edge cases (exhaustive list):** huge statements (store the first 64 KB + hash, optional compressed full text); multi-statement scripts (one row per unit + `script_run_id`); secret-named parameters; clock changes (ordering by id and monotonic `ts_start` within a session); concurrent consoles (single writer); corrupt DB (rotate to `history.corrupt-<ts>` and start a new one; tell the user once); disk full (degrade, warn once); Turkish search (`İSTANBUL` matches `istanbul`, `musteri` matches `müşteri`); exact duplicates differing only by trailing whitespace (different hash, same fingerprint); statements from MCP agents (origin `mcp`, filterable); a history entry inserted into an editor attached to another data source (allowed; the dialect may differ — warn); retention purge while searching (WAL readers unaffected); profile copied to another machine (works; data source ids are stable UUIDs).
- **Test cases (input → expected):** run `SELECT 1` twice → one row with `run_count = 2`; `select 1` → a new row with the same fingerprint; `ALTER ROLE app PASSWORD 'x'` → stored `PASSWORD '***'`, `redacted = 1` (HIST-03); the 10,001st distinct statement → the oldest unpinned row is evicted; search `musteri` finds `müşteri`; search `istanbul` finds `İSTANBUL`; prefix `SEL` + Up → the newest entry starting with `SEL`; a private console → nothing stored.
- **i18n notes:** dialog strings `history-title`, `history-filter-*`, `history-outcome-*`, `history-origin-*`; dates in the list follow the locale with an ISO tooltip; search folding uses ALG-I18N-06 regardless of UI language (it is tolerant by design).
- **Verification status:** DataGrip model [Observed: IntelliJ Community sources, own config dir]. The FTS5 `remove_diacritics 2` behaviour with `İ` is [Inferred]; our own normalized shadow column removes the dependency.
- **Prior art and evidence:** datagrip/10 §3.1; pgcli history; Atuin (MIT) design ideas.
- **Used by:** Hacivat; MCP `search_history`; **not** Karagöz (its audit log is separate, authoritative and append-only).

### HV-EXEC-13: Explain plan model and normalizers

- **Priority / depends on:** v1 for PostgreSQL (JSON), MySQL 8 (JSON / tree), SQLite (EQP) and Raw for every other dialect; v1.x for SQL Server showplan XML, MariaDB JSON, ClickHouse, MongoDB `executionStats`. Depends on HV-EXEC-05.
- **Sources merged:** datagrip/07 ALG-EXEC-08 and §3.22 (providers per dialect, Raw vs tree, metrics, ANALYZE executes, Hacivat rolls back DML), Compass doc ALG-MONGO-04 steps 1–2 (child normalization, per-kind time combination), verification/01 #10 (plain EXPLAIN can run functions at plan time).
- **Problem:** Produce one normalized plan tree for all engines, with self and total metrics that can feed a tree table, a diagram and a flame graph, and run EXPLAIN ANALYZE safely.
- **Inputs / outputs:** unit, dialect, mode (`Plan | Analyze`), raw flag → `Plan { root: PlanNode, metrics_available: MetricSet, raw: RawPlan, warnings }`.
- **Decision:**
  - **ANALYZE of DML/DDL runs inside a transaction that is always rolled back** (`BEGIN; EXPLAIN ANALYZE …; ROLLBACK`, or a savepoint inside an open transaction). The user can opt in to "execute for real" per run. DataGrip executes for real; we deliberately differ. On engines without transactional DDL, ANALYZE of DDL is refused with an explanation.
  - **Plain EXPLAIN is not treated as side-effect-free**: verification showed that PG evaluates a mislabelled IMMUTABLE function at plan time, and a dblink write committed. The gate (HV-EXEC-04) classifies EXPLAIN like a read, but read-only connections run it inside the read-only transaction. The UI notes that functions may execute during planning.
  - Child time combination is **per node kind** (parallel: shard merges, PG Gather/Gather Merge workers; sequential otherwise), not Compass's "max for all children".
  - Rejected: parsing text plans when a structured format exists.
- **Algorithm:**
  1. Build the command per dialect (matrix below). Bind parameters as in the original unit; for PG with unbound `$n`, offer `EXPLAIN (GENERIC_PLAN)` (PG ≥ 16).
  2. Execute via the session actor (gated). Collect the raw output (JSON text, XML, rows).
  3. Parse into `PlanNode`s: JSON walker (PG `Plan`/`Plans`, MySQL `query_block` / `nested_loop` arrays / `inputs`, MariaDB `query_block`, Mongo `inputStage(s)`, `innerStage`, `outerStage`, `thenStage`, `elseStage`, `shards[]`, `executionStages`, aggregation `stages[]` chained so that stage k is the parent of stage k−1), XML walker (MSSQL `RelOp`), row-tree (SQLite `id`/`parent`/`detail`, Oracle `PLAN_TABLE`), indented text (MySQL `EXPLAIN ANALYZE` tree format, ClickHouse `EXPLAIN PIPELINE`).
  4. Metrics per node: `est_rows, act_rows, loops, startup_cost, total_cost, act_startup_ms, act_total_ms, buffers{hit, read, dirtied, written}, workers, extra: Map`.
  5. Totals: for PG `act_total = actual_total_time × loops` (per-loop times). For other engines use the cumulative time when present, else the combined child time.
  6. Self value = node metric − combine(children metric), clamped ≥ 0. `combine = max` for parallel kinds, `sum` otherwise. Record `scaled/inconsistent` flags when children exceed the parent (instrumentation overhead, millisecond granularity).
  7. Annotations: estimate-vs-actual ratio (flag ≥ 10× either way), `docsExamined/nReturned` and `keysExamined/nReturned` (Mongo), sorts spilling to disk, nested loops with many loops, sequential scans on large relations (when stats are available).
  8. Views: Raw (native output, monospace), Tree table (columns: operation, object, est rows, actual rows, loops, cost/time, self %), Diagram (later), Flame graph (HV-EXEC-14). Metric selector: Total Cost, Startup Cost; with Analyze also Actual Total Time, Actual Startup Time.
  9. Huge plans (≥ 2,000 nodes, PG partition Append): group identical siblings ("Append × 1,000 partitions") with expansion on demand; the tree table is virtualized.
- **Data structures and Rust placement** (`hacivat-core::exec::explain::{model, pg, mysql, sqlite, mssql, mongo, clickhouse}`):
  ```rust
  pub struct PlanNode { pub id: u32, pub kind: Arc<str>, pub label: Arc<str>, pub object: Option<Arc<str>>, pub index: Option<Arc<str>>,
      pub est_rows: Option<f64>, pub act_rows: Option<f64>, pub loops: Option<u64>,
      pub startup_cost: Option<f64>, pub total_cost: Option<f64>, pub act_startup_ms: Option<f64>, pub act_total_ms: Option<f64>,
      pub self_: MetricVec, pub total: MetricVec, pub flags: PlanFlags, pub extra: Vec<(Arc<str>, Arc<str>)>, pub children: Vec<u32> }
  pub enum CombineKind { Parallel, Sequential }
  pub trait PlanProvider { fn command(&self, sql: &str, mode: ExplainMode, v: &ServerVersion) -> ExplainCommand;
                           fn parse(&self, raw: &RawPlan) -> Result<Plan, PlanParseError>; }
  ```
- **Per-DBMS matrix:**

  | DBMS | Plan | Analyze | Format parsed | Priority |
  |---|---|---|---|---|
  | PostgreSQL | `EXPLAIN (FORMAT JSON, VERBOSE, SETTINGS)` | `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON, VERBOSE)` in a rolled-back tx | JSON | v1 |
  | MySQL 8.0.18+ | `EXPLAIN FORMAT=JSON` | `EXPLAIN ANALYZE` (tree text; `FORMAT=JSON` for ANALYZE on 8.4+ [Inferred: verify]) | JSON / tree text | v1 |
  | MariaDB | `EXPLAIN FORMAT=JSON` | `ANALYZE FORMAT=JSON` | JSON | v1.x |
  | SQLite | `EXPLAIN QUERY PLAN` | n/a (no actuals) | id/parent/detail rows | v1 |
  | SQL Server | `SET SHOWPLAN_XML ON` | `SET STATISTICS XML ON` | XML | v1.x |
  | ClickHouse | `EXPLAIN PLAN indexes=1` / `EXPLAIN PIPELINE` | n/a (use `system.query_log`) | text tree | v1.x |
  | MongoDB | `explain("queryPlanner")` | `explain("executionStats")` | JSON | v1.x |
  | Oracle | `EXPLAIN PLAN FOR` + `DBMS_XPLAN`/`PLAN_TABLE` | `GATHER_PLAN_STATISTICS` + `DBMS_XPLAN.DISPLAY_CURSOR` | rows | later |
  | Others | Raw only | Raw only | — | — |
- **Complexity / performance budget:** parse + normalize ≤ 20 ms for a 1,000-node plan; tree table first paint ≤ 1 frame after parse.
- **Edge cases (exhaustive list):** InitPlan/SubPlan/CTE scans referenced several times (shown as children with a "referenced" mark, counted once in totals); parallel workers (`Workers Planned/Launched`); `never executed` nodes (act = None, rendered dimmed); MySQL `nested_loop` arrays; MSSQL multiple statements per showplan (one plan per statement); Oracle `PLAN_TABLE` missing or no privilege (Raw error); plans for parameterized statements (GENERIC_PLAN on PG 16+, else ask for values); huge plans (grouping, virtualization); negative self cost from rounding (clamp, flag); zero total (flame minimum width); EXPLAIN of a statement that is itself EXPLAIN (refuse); EXPLAIN ANALYZE of `COPY … TO PROGRAM` or `dblink_exec` (verified to escape rollback → refuse ANALYZE for Routine-class statements with high severity unless "execute for real" is chosen); EXPLAIN on a read-only connection (inside the read-only tx; ANALYZE of DML refused); MySQL `EXPLAIN ANALYZE` actually executes SELECTs (no side effects, but long-running → cancellable); a JSON plan larger than 50 MB (stream-parse; truncate with a warning); ClickHouse without timing.
- **Test cases (input → expected):** PG JSON plan with a Hash Join over two Seq Scans → Σ self time = root total (± rounding); PG Nested Loop (actual 5.0 ms, loops 1) with an inner Index Scan 0.01 ms × 1,000 loops → inner total 10 ms > parent → parent self clamped to 0, flag "timing inconsistency"; SQLite EQP rows `(2,0,"SCAN t")`, `(3,2,"SEARCH u USING INDEX …")` → tree with 2 nodes; Derby → Raw only, flame graph disabled; `EXPLAIN ANALYZE DELETE FROM t` on PG → rows still exist afterwards; Mongo FETCH(total 10) → IXSCAN(total 7) → FETCH self 3, IXSCAN self 7; SHARD_MERGE with shards 40/25, node 42 → childTime 40 (parallel), self 2.
- **i18n notes:** operation names (`Seq Scan`, `IXSCAN`) are engine terms and are not translated; column headers, flags and tooltips are translated (`plan-col-operation`, `plan-flag-estimate-off = Estimated { $est } rows, actual { $act }`, `plan-analyze-rollback-note = The statement was executed and then rolled back.`, `plan-functions-may-run = Planning may execute functions.`). Numbers in the tree table use the grid number formatter (raw by default).
- **Verification status:** [Observed: verification/01 #10] plain `EXPLAIN` evaluated a mislabelled IMMUTABLE function and committed a dblink write → reflected in the Decision. [Observed: verification/01 #3] `COPY TO PROGRAM` and `dblink_exec` are not stopped by read-only transactions or rollback of the local tx (dblink commits remotely) → ANALYZE refusal rule. DataGrip provider list [Observed: class names]; the raw-only dialects are [Inferred].
- **Prior art and evidence:** datagrip/07 §3.22; Compass explain-plan helper (SSPL, behaviour only); PEV2 (PostgreSQL license) per-node exclusive times [Inferred: general knowledge].
- **Used by:** Hacivat; MCP `explain_query` (returns the normalized tree, never runs ANALYZE without explicit permission); Karagöz treats EXPLAIN ANALYZE as the underlying statement for permission checks.

### HV-EXEC-14: Flame graph / icicle layout

- **Priority / depends on:** v1.x. Depends on HV-EXEC-13.
- **Sources merged:** Compass doc ALG-MONGO-04 steps 3–8 (layout, zero-width minimum, colours, highlights, self-time table, zoom), datagrip/07 §3.22 (metric selector, "Flame Chart: {metric}").
- **Problem:** Show where time or cost goes in a plan at a glance, for any engine, accessible without colour.
- **Inputs / outputs:** a normalized `Plan`, a metric (TotalCost, StartupCost, ActualTotalTime, ActualStartupTime, or a work metric such as docsExamined), a zoom root → `Vec<Rect{node, x, w, depth, label, self, total, pct, category}>` plus a self-time table.
- **Decision:** Icicle orientation (root on top) by default, flame (root at the bottom) as a toggle. **Fit mode** (children scaled into `parent.width − parent.self` when they overflow, flagged `scaled`) is the default; **parallel lanes** mode draws parallel children at full width in separate lanes. When all times are 0 (granularity), fall back to node count or a work metric, with a banner.
- **Algorithm:**
  1. `W = root.total(metric)`; if `W == 0` → use uniform widths per leaf (banner `plan-no-timing`).
  2. Pre-order: `x(child_0) = x(parent)`; `w(child) = total(child) × scale`, where `scale = 1` unless `Σ total(children) > total(parent) − self(parent)`, in which case `scale = (total(parent) − self(parent)) / Σ total(children)` and the rects are flagged `scaled`. `depth(child) = depth(parent) + 1`.
  3. Pixel mapping happens in the UI: `px = x / W_zoom × view_width`. Rects narrower than 2 px are still drawn at 2 px when their node is selected or hovered; others narrower than 1 px are merged into a "…" sliver per parent.
  4. Colour by category (scan, index, join, sort/blocking, aggregate, network/shard, other) with hatch patterns for accessibility; text label = kind + object, truncated by grapheme clusters.
  5. Self-time table: aggregate self by kind and by (kind, object), sorted descending.
  6. Interaction: click to zoom (the node becomes the root of the x-scale); breadcrumb to zoom out; hover tooltip with all metrics; arrow keys traverse the tree (Up = parent, Down = first child, Left/Right = siblings); the screen-reader alternative is the tree table of HV-EXEC-13.
- **Data structures and Rust placement:** layout in `hacivat-core::exec::explain::flame` (pure, unit-tested); rendering in `hacivat-app::plan_view` using GPUI `canvas` + `paint_quad` + shaped text (same text cache as the grid, HV-GRID-03).
- **Complexity / performance budget:** layout O(nodes) ≤ 5 ms for 10k nodes; 60 fps zoom animations; paint ≤ 4 ms per frame.
- **Edge cases (exhaustive list):** no timing (count-based layout); rejected plans (Mongo; shown separately); sharded explains with different winning plans; `$lookup`/`$unionWith` sub-pipelines; PG `loops > 1`; never-executed nodes (width 0, shown as a sliver on hover); negative self (clamped); 10k-wide Append (grouped); right-to-left UI (the x axis is not mirrored: time flows left to right in every locale); very small windows (labels hidden below 24 px width); dark/light themes (category colours from theme tokens).
- **Test cases (input → expected):** Mongo FETCH(10) → IXSCAN(7) → widths 10 and 7, FETCH self 3; SHARD_MERGE(42) with shards 40/25 in fit mode → child widths 24.6 and 15.4, `scaled = true`; parallel lanes mode → widths 40 and 25 in separate lanes; all-zero times → uniform widths and the banner; zoom on a node with total 7 → that node spans the full width.
- **i18n notes:** `plan-flame-title = Flame chart: { $metric }`, metric names, `plan-no-timing`, `plan-scaled-note`. Percentages formatted with the locale's percent pattern (Turkish `%25` prefix form) via ICU4X.
- **Verification status:** Compass combination rule "max for all children" [Observed: `explain-tree-data.ts:36-42`] is replaced by the per-kind table (our decision). DataGrip metrics [Observed: bundle].
- **Prior art and evidence:** Brendan Gregg flame graphs [Inferred: general knowledge]; Compass (behaviour only).
- **Used by:** Hacivat.

### HV-EXEC-15: Script runner (large files, multi-target runs)

- **Priority / depends on:** v1.x. Depends on HV-EXEC-01 (execution-mode split), -05, -08, sql-tooling-02 SPLIT-01/02.
- **Sources merged:** datagrip/07 ALG-EXEC-14 (run configuration multi-target), ALG-EXEC-16 (large-file path: > 20 MB opens a 2.5 MB preview), datagrip/10 ALG-TOOL-12 (run script files against multiple targets), dbeaver-05 §3.13 (#42570: an 800 MB script OOM; stream from disk).
- **Problem:** Execute scripts of any size (GBs of INSERTs, dumps) against one or many targets with constant memory, a progress indicator and a clear error policy, without opening them in the editor.
- **Inputs / outputs:** targets `[DataSource(+db/schema)]`, script text and/or files, encoding, error policy, before-run tasks → per-target `RunReport { units_total, executed, failed, aborted, duration, bytes }`, streamed to a Run panel and the CLI.
- **Decision:** A streaming splitter over a 64 KiB rolling buffer that preserves lexer state across chunk boundaries (strings, comments, dollar quotes, `DELIMITER`, `GO`). Consecutive DML statements are pipelined, not concatenated: a batch of up to 2,000 statements or 4 MiB inside one transaction when "batch mode" is on. **No 1,000-statement cap** (DataGrip help mentions one). Targets run sequentially by default; parallel is opt-in with a concurrency cap (default 4). A warning before start if the script changes schema or database (`USE`, `SET search_path`, `\c`).
- **Algorithm:**
  1. Validate: code present, targets present.
  2. Detect encoding: BOM → UTF-8 strict on the first 64 KiB → user choice (Windows-1254 and ISO-8859-9 offered prominently for a Turkish locale); show the detected encoding.
  3. For each target (sequential unless parallel is on): open a dedicated session (not the user's console session), set the context schema, run the before-run tasks.
  4. Stream the file through the decoder and the incremental splitter. For each unit: run through the session actor with the error policy (StopOnError default for run configurations).
  5. A unit that grows beyond the buffer (a huge dollar-quoted body or a long INSERT) grows the buffer up to `max_unit_bytes` (default 256 MiB), then fails with `script-unit-too-large`.
  6. PG `COPY … FROM STDIN` blocks: switch to pass-through mode and stream the data lines to the `CopyIn` sink until `\.` (doc 24 HV-XFER-10).
  7. psql meta-commands (`\set`, `\i`, `\c`) and other client commands: emulate a safe subset (`\set` → template variables, `\c` → refuse with a hint), else stop with `script-client-command-unsupported` and suggest native restore (pg_restore/psql).
  8. Progress = bytes consumed / file size; the Run panel shows units/s, errors, the current line. No per-statement gutter.
  9. Report per target; history records one row per unit with `script_run_id` (HV-EXEC-12), sampled for scripts beyond 100k units (every unit with an error is recorded; successes are summarised).
- **Data structures and Rust placement:** `hacivat-core::exec::script { StreamingSplitter, ScriptRun, RunReport }`; CLI `hacivat run --target <ds> [--schema s] file.sql` in `hacivat-cli`.
- **Complexity / performance budget:** memory O(buffer + largest unit) — target ≤ 64 MiB resident for a 10 GB INSERT script; throughput ≥ 20k simple INSERT statements/s to local PG in batch mode (pipelined); splitter ≥ 200 MB/s.
- **Edge cases (exhaustive list):** a statement spanning a chunk boundary; a dollar-quoted body larger than the buffer; a multibyte UTF-8 sequence split at a chunk boundary; BOM; mixed CRLF/LF; `COPY FROM STDIN` data (pass-through); psql meta-commands; `GO n` batches; `DELIMITER` changes mid-file; an unreachable target (skip or stop per setting); the same file twice; `USE`/`search_path` changes that persist to the following files on the same target; output paused; cancel (stops after the current unit; reports the position); a file modified while running (the run uses an open file handle; warn); gzip-compressed `.sql.gz` (transparent decompression, v1.x); very long single-line dumps (MySQL extended inserts of 100 MB: max unit size).
- **Test cases (input → expected):** a 100 MB file of INSERTs → constant memory; a statement crossing the 64 KiB boundary splits correctly; a PG dump with `COPY … FROM stdin;` + data + `\.` → rows loaded; 2 targets × 2 files → 4 runs logged in order; a schema-switch statement present → warning dialog before start; error in unit 5,000 with StopOnError → report "4,999 of N executed, 1 failed (aborted at line L)".
- **i18n notes:** `script-run-title`, `script-progress = { $percent } · { $units } statements · { $errors -> [0] no errors *[other] { $errors } errors }`, `script-schema-switch-warning`, `script-encoding-detected = Encoding: { $encoding }`. Byte sizes formatted with ICU4X units.
- **Verification status:** DataGrip thresholds (20 MB / 2.5 MB) [Observed: help]; the "1000 DML statements" cap semantics are [Unknown] (RUNFILE-01) and not adopted. DBeaver #42570 [Observed: verification/02 S27].
- **Prior art and evidence:** datagrip/07 §3.20–3.21; datagrip/10 §3.11.
- **Used by:** Hacivat (run configurations, "Run SQL file"); `hacivat-cli` (CI); Karagöz (scheduled scripts, later).

### HV-EXEC-16: In-editor result anchoring

- **Priority / depends on:** later. Depends on HV-EXEC-07 and the editor's anchor/marker API in `hacivat-ui` (SQL editor component).
- **Sources merged:** datagrip/07 ALG-EXEC-17 and §3.14 (In-Editor Results, stacked result sets, collapse, optional paging).
- **Problem:** Keep inline result blocks attached to their statements while the user edits.
- **Inputs / outputs:** a unit's `doc_range`, its outcome → a `ResultBlock` anchored in the editor.
- **Decision:** Anchor to the statement's end offset with a sticky-right anchor. If the statement text changes, mark the block **stale** (dimmed) instead of removing it. If the statement is deleted, remove the block (restorable through undo within the undo window). Blocks show at most 50 rows inline with "Open in results panel"; paging off by default.
- **Algorithm:**
  1. After a unit with result sets completes in in-editor mode, insert a block decoration after the line containing `doc_range.end`; store `(anchor, text_hash_of_statement)`.
  2. On each document change: if the anchor was deleted → remove the block; else if the hash of the current statement text at the anchored range ≠ the stored hash → mark stale.
  3. Re-executing the statement replaces the block's content (same block id). Several result sets stack as tabs inside one block. The collapse state is per block.
  4. Large results: cap the visible rows, keep the full result in the store, link to the panel.
- **Data structures and Rust placement:** `hacivat-app::inline_results` (UI glue); anchors from the `hacivat-ui` editor (a sum-tree/rope anchor, Apache-2.0 `sum_tree` usable; never Zed's GPL `editor`/`text`).
- **Complexity / performance budget:** anchor update O(log n) per edit; block layout ≤ 1 frame; a 50-row inline grid uses the same grid component (HV-GRID-02) in a fixed-height viewport.
- **Edge cases (exhaustive list):** re-execution replaces the content; undo restoring a deleted statement (restore the block within the undo window); statements merged or split by an edit (the block stays with the anchor; stale); very large results (cap); a block for a statement whose unit was a batch (anchored to the batch end); folded code regions containing the block (hidden with the fold); soft wrap changes (layout only); the file closed and reopened (blocks are not persisted in v1).
- **Test cases (input → expected):** edit inside the statement → block stale; delete the statement → block removed; undo → block restored; re-run → content replaced, block not duplicated.
- **i18n notes:** `inline-results-stale = Results are from an earlier version of this statement.`, `inline-results-open-panel`.
- **Verification status:** DataGrip behaviour [Observed: help, bundle]; the stale rule is our own decision.
- **Prior art and evidence:** datagrip/07 §3.14.
- **Used by:** Hacivat.

---

## 3. Conflicts resolved in this catalog

| Topic | Source A | Source B | Decision |
|---|---|---|---|
| "Smallest statement" meaning | DataGrip help prose | DataGrip screenshots, labels, DBE-6418 | Follow the screenshots (HV-EXEC-01) |
| Default for caret inside | DataGrip screenshot: Ask | — | Ask with a one-candidate shortcut (product decision PD-1) |
| Native `?` in PG | DataGrip JDBC habit | sql-tooling-02: off by default | Off by default, per-data-source switch |
| Template parameters | DataGrip: on, textual | Hacivat safety goal | v1.x, disabled by default, labelled "text substitution" |
| Cancel | DataGrip: disconnect only | rainfrog/Beekeeper: server cancel | Protocol cancel → terminate → deactivate |
| EXPLAIN ANALYZE of DML | DataGrip executes for real | Hacivat safety | Rolled-back tx by default |
| Plain EXPLAIN side effects | datagrip/07: "Read" | verification/01 #10 | Read class, but run inside the RO tx on RO connections and warn |
| Flame child combination | Compass: max for all | PG/Mongo semantics | Per-kind table |
| History model | DataGrip text list, 300 entries | ALG-TOOL-03 | SQLite + FTS5, 10k/data source, 90 days |
| Script DML cap | DataGrip help: 1,000 | — | No cap; streaming |
| Auditor execution | DataGrip synchronous auditors [Inferred] | DBeaver freeze lessons | Sync O(1) chain + bounded async sinks |

## 4. Open questions that need a product decision

- **PD-1 (caret-inside default).** Ask-with-shortcut (recommended, matches DataGrip's shipped screenshot) vs Smallest statement (fewer popups, matches DBeaver/Beekeeper habits).
- **PD-2 (template parameters).** Ship DataGrip-compatible textual patterns disabled (recommended) or enabled by default for migrants.
- **PD-3 (confirm-in-transaction on production).** Default on for production-tagged connections (recommended) — it holds locks while the dialog is open; the auto-rollback timeout (5 min) needs product sign-off.
- **PD-4 (history retention).** 10,000 entries per data source and 90 days (recommended) vs unlimited; also whether MCP-originated statements are recorded by default (recommended: yes, origin-filterable).
- **PD-5 (EXPLAIN ANALYZE).** Rolled-back by default (recommended) — confirm that users accept that "Explain Analyse" of an INSERT shows the plan but leaves no rows.
