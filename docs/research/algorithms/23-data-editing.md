# 23 — Data Editing: Canonical Algorithm Catalog (HV-EDIT)

> Status: canonical, implementation-ready. Date: 2026-11-11. Scope: editing data in grids and turning edits into safe database changes: row identity detection and editability, the pending-changes overlay, invalid-input retention, DML generation per dialect, optimistic concurrency, affected-row assertions, the submit pipeline and transactions, reconciliation after submit (no full reload), DML preview and the literal renderer, undo/redo, and document/key-value store edits.
>
> Synthesis document; conventions in `../README.md`. Companions: `20-query-execution.md` (session actor, transactions, gate), `21-result-fetching-and-store.md` (column origins, store patches), `22-data-grid-component.md` (editor lifecycle, parsing), `24-clipboard-import-export.md` (paste produces pending changes).
>
> **Where Hacivat must beat DataGrip and DBeaver:** neither has optimistic concurrency (DataGrip: none evidenced; DBeaver: key-only WHERE, silent overwrite). DBeaver does not detect multi-row updates and turns explicit NULLs into DEFAULTs. DBeaver discards pending edits on local sort and reloads from row 0. This catalog fixes all of these by design.

## Sources and licenses

| Source document | Specs consumed | Underlying license |
|---|---|---|
| `datagrip/08-data-grid.md` | ALG-GRID-04 (overlay), -05 (DML generation), -06 (affected rows, re-read), §3.5 (row identity), §3.8–3.10 (editors, special values, validation, submit, tx) | DataGrip proprietary (behaviour only) |
| `clients/dbeaver-04-results-viewer-data-editor.md` | ALG-DBV-GRID-01 (identity detection), -02 (DML generation and save), §3.9–3.10, §3.16 (virtual model), defects Q5–Q7, Q12; `DBDResultSetDataUpdaterTest` vectors | Apache-2.0 @ `f78ac45` |
| `clients/oss-clients-beekeeper-and-tui.md` | ALG-EDIT-01 (change set with optimistic concurrency and preview) | Beekeeper GPL-3.0 (behaviour only) |
| `clients/oss-clients-redisinsight-compass.md` | ALG-MONGO-02 step 7 (Mongo edits with `$set`/`$unset`), ALG-REDIS-02 step 9 (Redis edits with WATCH); Compass "update unless changed in background" | SSPL (behaviour only) |
| `datagrip/07-query-execution-ux.md` | ALG-EXEC-15 (tx state), §3.13 (Submit and Commit, Roll Back reverts edits) | DataGrip proprietary |
| `datagrip/13-behavioral-test-corpus.md` | DML-01…31 | own corpus |
| `i18n/02-turkish-and-db-specific-pitfalls.md` | §3.8.3 (canonical values sent to DB), ALG-I18N-07 | — |
| `verification/01-db-behavior-tests.md` | #26 (PG freezing keeps the raw `xmin`), #42 (SQL Server Turkish identifier case) | — |

Clean-room: own pseudocode; DBeaver test names are cited as behaviour vectors only.

---

## 0. Dependency map and priorities

```
 HV-RES-09 column origin ──► HV-EDIT-01 row identity + editability
                                   │
 HV-GRID-13 editor ──► HV-GRID-11 parse ──► HV-EDIT-03 invalid input ──► HV-EDIT-02 pending overlay ◄── HV-XFER-03 paste
                                                                            │       ▲
                                                                            │       └── HV-EDIT-10 undo/redo
                                                                            ▼
                                     HV-EDIT-04 DML generation ◄── HV-EDIT-05 optimistic concurrency
                                            │            │
                                            ▼            ▼
                                  HV-EDIT-09 preview   HV-EDIT-07 submit + transactions (HV-EXEC-05/10)
                                                         │
                                            ┌────────────┴────────────┐
                                            ▼                         ▼
                                  HV-EDIT-06 affected-row     HV-EDIT-08 reconcile (RETURNING/OUTPUT,
                                  assertion                    re-select, store patches)
 HV-EDIT-11 document / key-value edits (Mongo, Redis) — v1.x
```

| ID | Name | Priority | Depends on | Consumed by |
|---|---|---|---|---|
| HV-EDIT-01 | Row identity detection and editability | **v1** | HV-RES-09, catalog | 02, 04, grid |
| HV-EDIT-02 | Pending-changes overlay | **v1** | 01 | 03, 04, 07, 09, 10, grid |
| HV-EDIT-03 | Invalid-input retention and pre-submit validation | **v1** | HV-GRID-11, 02 | 07 |
| HV-EDIT-04 | DML generation per dialect | **v1** (PG, MySQL/MariaDB, SQLite, SQL Server) | 01, 02, 05 | 07, 09 |
| HV-EDIT-05 | Optimistic concurrency | **v1** | 01, 02, HV-RES-04 | 04, 07 |
| HV-EDIT-06 | Affected-row assertion | **v1** | 04 | 07 |
| HV-EDIT-07 | Submit pipeline and transactions | **v1** | 03–06, HV-EXEC-05/10 | grid, MCP |
| HV-EDIT-08 | Reconcile after submit | **v1** | 07, HV-RES-02 | grid |
| HV-EDIT-09 | DML preview and literal renderer | **v1** | 04 | grid, HV-RES-12, HV-XFER-06 |
| HV-EDIT-10 | Undo/redo of edits | **v1** | 02 | grid |
| HV-EDIT-11 | Document and key-value store edits | **v1.x** | 02, 05 (pattern) | Mongo/Redis views |

## 1. Cross-cutting decisions for this catalog

1. **Edits are an overlay keyed by row identity**, never by view index, and survive paging, local sort, local filter, refresh and reconnect (until submitted, reverted or explicitly discarded).
2. **Typed values everywhere.** The overlay stores `Value` (HV-RES-01); binds are typed; preview literals come from one dialect renderer; nothing goes through display strings.
3. **NULL, DEFAULT, empty string and "not set" are four different things.** "Not set" in an inserted row means "omit the column" (server default applies). Explicit `NULL` is inserted as NULL. `Default` emits the `DEFAULT` keyword. The empty string is a value.
4. **One explicit transaction per submit** (all-or-nothing), even in auto-commit mode; a savepoint inside a user's manual transaction. **Exactly-one-row assertions** for every UPDATE/DELETE. **Optimistic concurrency on by default.**
5. **Karagöz receives the structured intent** (table, key, changed columns, old/new values) along with the SQL, so that it can enforce column-level write permissions, forbid all-column keys under policy, and audit precisely.
6. **Rust placement:** `hacivat-core::edit` (identity, overlay, validation, DML model, concurrency, submit, reconcile, preview), `hacivat-core::sql_render` (dialect identifier quoting and literal rendering, shared with filters and extractors), `hacivat-core::virtual_model` (per-connection virtual keys/FKs file). Karagöz links `hacivat-core::edit::dml` to validate intents.
7. **Crate versions to check:** `indexmap 2.14.2` (ordered overlay maps), `smallvec`, `blake3` (row fingerprints), plus the driver crates of doc 21.

---

## 2. Specifications

### HV-EDIT-01: Row identity detection and editability

- **Priority / depends on:** v1. Depends on HV-RES-09 (column origins), the catalog cache (keys, indexes, nullability, generated columns), the virtual model.
- **Sources merged:** dbeaver-04 ALG-DBV-GRID-01 (identity order, good-referrer rule, nullable unique rejected #424, partial indexes, views, self-joins, pseudo columns, all-columns fallback, read-only reasons, async detection), datagrip/08 §3.5 (PK/unique only else read-only; recommended fallback order), DML-03/23/27/28 (doc 13).
- **Problem:** Decide whether each result column is editable and, per source range variable, which columns identify a row uniquely so that UPDATE/DELETE touch exactly that row.
- **Inputs / outputs:** result columns with `ColumnOrigin`s, catalog metadata per table (PK, unique constraints/indexes with nullability, partial/expression flags, `NULLS NOT DISTINCT`, table kind, generated/identity/rowversion columns), virtual keys, connection and policy flags → per range variable `Identity{kind, key_cols} | Incomplete{missing} | None{reason}` and per column `Editability`.
- **Decision:**
  - **Strict default** (DataGrip-like): (1) primary key; (2) a unique constraint or unique index that is **not partial, not on expressions**, whose columns are all NOT NULL — or nullable but declared `NULLS NOT DISTINCT` (PG ≥ 15) / an SQL Server unique index (NULLs collide there); prefer the narrowest (fewest columns, then integer/uuid types). Otherwise read-only with a reason.
  - **Opt-in fallbacks, persisted per table in the virtual model** (one click from the read-only banner): (a) **virtual key** (user picks columns); (b) **physical row id** where it is stable for the edit window: SQLite `rowid` (not for `WITHOUT ROWID` tables), Oracle `ROWID` (later); PostgreSQL `ctid` **only together with `xmin` equality** and only for the row as last read (a ctid changes on every update); (c) **all columns** (excluding float, LOB/truncated, json, xml, geometry and array columns), always with the exactly-one-row assertion (HV-EDIT-06).
  - **Identity per range variable** (alias), not per table — self-joins get two identities.
  - **Detection is asynchronous**: rows paint first; editing turns on when detection completes (≤ 50 ms typical).
  - **Single-source editing in v1** (all edited columns come from one range variable); per-range-variable UPDATEs for join results are v1.x; INSERT and DELETE always need a single source.
  - Table editors add missing key columns as **hidden** columns to their SELECT (and PG `xmin`, MSSQL `rowversion` for HV-EDIT-05). Console results cannot be rewritten → if key columns are missing: read-only with "Key column `id` is not in the result" and an action "Open table editor filtered to these rows" (via HV-RES-12).
- **Algorithm:**
  1. Group result columns by origin range variable (HV-RES-09); columns without origin → `ReadOnly(NoOrigin | Expression | Aggregate …)`.
  2. For each range variable `rv` with table `T`:
     1. Connection/policy checks: read-only connection, no UPDATE privilege known from the catalog (`has_table_privilege` / grants cache), Karagöz policy → `ReadOnly(...)` for all columns of `rv`.
     2. `T` is a view: editable only if "edit through view" is enabled for it (PD-8) and it has a virtual key; else `ReadOnly(ViewNotEditable)`.
     3. Candidates = [PK] + qualifying unique constraints/indexes (rules above), ordered PK first, then fewest columns, then integer/uuid columns.
     4. The first candidate whose columns are all present in the result (or hidden-added) → `Identity{Pk | Unique, cols}`.
     5. Else the virtual-model choice if persisted → `Identity{Virtual | PhysicalRowId | AllColumns, cols}`.
     6. Else `None(NoIdentity)` with the missing-key detail.
  3. Per column of `rv`: generated/computed/rowversion → `ReadOnly(Generated)`; identity column → editable only in INSERT (UPDATE of identity columns refused: `ReadOnly(IdentityColumn)` for updates); LOB truncated → editable after full load; key columns → editable (WHERE uses the original value).
  4. Duplicate identity values within the loaded rows (possible with virtual keys/all-columns) → editing of those rows disabled with `ReadOnly(DuplicateKey)`.
  5. Re-validate at submit time (schema may have changed).
- **Data structures and Rust placement** (`hacivat-core::edit::identity`):
  ```rust
  pub enum IdentityKind { Pk, UniqueConstraint, UniqueIndex, Virtual, PhysicalRowId, AllColumns }
  pub struct RowIdentity { pub range_var: Arc<str>, pub table: TableId, pub kind: IdentityKind, pub key_cols: SmallVec<[ResultCol; 2]>,
                           pub version_col: Option<ResultCol> /* xmin, rowversion, user version column */ }
  pub enum Editability { Editable, InsertOnly, ReadOnly(ReadOnlyReason) }
  pub enum ReadOnlyReason { NotConnected, ConnectionReadOnly, NoPrivilege, PolicyDenied, NoOrigin, Expression, Aggregate,
      NoIdentity { missing: Vec<Arc<str>> }, ViewNotEditable, MultiSource, Generated, IdentityColumn, DuplicateKey,
      EngineNotSupported, ResultNotRequeryable }
  pub struct RowKey(Arc<[Value]>);   // identity values in key_cols order (original values, never edited ones)
  ```
- **Per-DBMS matrix:**

  | Engine | Default identity | Nullable unique accepted? | Physical row id (opt-in) | Version token for HV-EDIT-05 | Notes |
  |---|---|---|---|---|---|
  | PostgreSQL | PK / unique NOT NULL (non-partial, non-expression) | only `NULLS NOT DISTINCT` (PG 15+) | `ctid` + `xmin` equality | `xmin` (hidden column in table editors) | `oid` tables are gone since PG 12 |
  | MySQL/MariaDB | PK / unique NOT NULL | no (NULLs distinct) | none | none native → original values | InnoDB clustered PK |
  | SQL Server | PK / unique NOT NULL | unique **index** with NULL: yes (NULLs collide; one NULL allowed) | `%%physloc%%` — not used (unstable) | `rowversion` column if present | quote catalog spelling (Turkish collations) |
  | SQLite | PK / unique NOT NULL / `INTEGER PRIMARY KEY` | no | `rowid` (rowid tables only) | none → original values | `WITHOUT ROWID` → PK required |
  | ClickHouse | — | — | — | — | **read-only in v1** (`EngineNotSupported`): no row identity guarantee; mutations are async |
- **Complexity / performance budget:** O(columns + Σ constraints) per result with cached catalog data; ≤ 50 ms after `ResultSetStart`; never blocks first paint.
- **Edge cases (exhaustive list):** PK exists but not all PK columns in a console result (read-only with the missing names; table editors add them hidden); nullable unique (#424) rejected; `NULLS NOT DISTINCT` accepted; partial unique index (`WHERE active`) rejected; expression index rejected; deferrable unique constraint (accepted); views (opt-in + virtual key); self-join (two identities by alias); joins (multi-source: v1 read-only for columns of non-primary sources, v1.x per-source UPDATE); aliased columns (origin from protocol); the same column selected twice (both cells edit the same value; the overlay keeps them in sync); expressions/aggregates/DISTINCT/GROUP BY/window/UNION (no origin); PG `ctid` (only with `xmin`); SQLite `WITHOUT ROWID`; all-columns with duplicate rows (DuplicateKey); all-columns with float/LOB/json columns (excluded from the key; if nothing remains → NoIdentity); NULL key values (only possible with virtual keys → `IS NULL` predicate, HV-EDIT-04); catalog unreadable (permissions → NoIdentity with "cannot read table metadata"); case-mismatch between protocol names and catalog (normalize by dialect folding rules; never by locale case mapping — ALG-I18N-05); results from procedures, `RETURNING`, DML CTEs (`ResultNotRequeryable` unless identity columns are present → editable through identity); connection read-only (all read-only); schema changed after fetch (re-validated at submit → error "table definition changed; reload"); temporary tables (editable within the same session only — submits go to the main session, HV-EXEC-05); foreign tables (editable; server decides); partitioned tables (PK includes the partition key; fine).
- **Test cases (input → expected):** `t(id PK, name)`, `SELECT * FROM t` → `Pk(id)`, editable; `SELECT name FROM t` → `NoIdentity{missing: [id]}`; `t(a INT NULL UNIQUE, b)` → NoIdentity; `t(a INT NOT NULL UNIQUE, b)` → `UniqueConstraint(a)`; PG 15 `UNIQUE NULLS NOT DISTINCT (a)` with nullable `a` → accepted; view `v` → `ViewNotEditable`; `SELECT a.id, b.id FROM t a JOIN t b ON …` → identities for `a` and `b`; SQLite rowid table without PK + opt-in → `PhysicalRowId(rowid)`; ClickHouse table → `EngineNotSupported`; DML-03 (PG no PK) → read-only "Table has no unique key"; DML-28 (SQLite without explicit PK) → read-only until the rowid opt-in.
- **i18n notes:** every `ReadOnlyReason` maps to a Fluent id (`ro-reason-no-identity = This result has no unique key. Missing: { $columns }.`, `ro-reason-view`, `ro-reason-generated`, `ro-reason-policy = Editing is not allowed by your organization's policy.`, …). Column names in messages are data (verbatim, isolated).
- **Verification status:** DBeaver identity logic [Observed: `DBExecUtils.java:584-712`, `DBUtils.java:1437-1484`]; DataGrip fallbacks [Unknown → DML-03/23/27/28]; [Observed: verification/01 #42] SQL Server Turkish collation makes identifier case significant → emit catalog spelling.
- **Prior art and evidence:** as merged.
- **Used by:** Hacivat; Karagöz (policy: forbid AllColumns/PhysicalRowId keys on protected databases).

### HV-EDIT-02: Pending-changes overlay

- **Priority / depends on:** v1. Depends on HV-EDIT-01.
- **Sources merged:** datagrip/08 ALG-GRID-04 (data structure, rules, render layers, orphans, submit-enabled iff no Unparsed, immediate-submit, leave guard), dbeaver-04 §3.9 (original captured at first edit, LOB cloning, ADDED/REMOVED states, restore-original removes the change), Beekeeper ALG-EDIT-01 step 2 (edit back to the original removes it; editing a pending insert edits the insert; deleting a pending insert drops it; delete replaces updates; un-delete restores updates), DBeaver Q12 (never drop edits on sort).
- **Problem:** Hold user edits without touching loaded data; render, revert, preview and submit them; keep them consistent across paging, sorting, filtering and reloads.
- **Inputs / outputs:** edit operations from the grid, record view, paste, MCP → overlay state, change events, decorations for the grid (HV-GRID-03).
- **Decision:** DataGrip's structure plus Beekeeper's rules, with the "un-delete restores the edits" choice (DataGrip leaves this open; we choose to restore). Pending inserted rows carry an anchor (the row they were added after) for display (HV-GRID-06). Overlays are **per grid** (per result tab); after a submit, other grids showing the same table get a "Data changed — Reload" hint (no automatic reload).
- **Algorithm:**
  1. `set_cell(row_ref, col, new: NewValue)`:
     - `row_ref = Pending(temp)` → update `inserts[temp].cells[col] = new` (a value equal to "not set" removes the entry).
     - `row_ref = Existing(key)`: if `deletes ∋ key` → reject (`edit-row-deleted`); capture `original` for the row on its first edit (cells as loaded, including the hidden version column); if `typed_eq(new, original[col])` → remove the cell entry (and the row entry if empty); else upsert. LOB originals are full-value handles when loaded, else `Truncated` (key columns are never LOBs).
     - A key column edit keeps `RowKey` (original key) for the WHERE and records the new key value; after submit the reconciled row uses the new key.
  2. `delete_rows(refs)`: pending inserts → removed; existing → `deletes.insert(key)` and their cell updates are **parked** (kept in `parked_updates`), so that un-delete restores them.
  3. `undelete(refs)` → remove from `deletes`, restore parked updates.
  4. `insert_row(anchor, template: Empty | Clone(row))` → `inserts.push(NewRow{temp, anchor, cells})`; clone copies values except identity/generated/rowversion columns; truncated cells are loaded in full first (or the clone is refused when the source has no identity — DataGrip refusal).
  5. `revert(selection)` → remove cell updates, inserts and deletes inside the selection; an empty selection reverts everything (with confirmation when > 100 changes).
  6. Counters: `modified_rows`, `inserted_rows`, `deleted_rows`, `unparsed_cells` (Submit enabled iff `unparsed_cells == 0` and there is at least one change).
  7. Reload/refresh/paging: the overlay persists; rows are re-matched by `RowKey`; keys that are no longer present in the loaded rows become **orphans** (still submitted — they target rows by key — and flagged in the preview).
  8. Re-running a *different* statement in the result tab, closing the tab, disconnecting, or switching data source with a non-empty overlay → guard dialog "Changes are not submitted. Submit / Discard / Cancel" (DataGrip only offers continue/cancel).
  9. Immediate-submit mode (setting, default off): each confirmed edit triggers HV-EDIT-07 for that row; the overlay never holds more than one row; deletes ask for confirmation in this mode.
  10. Rendering: the grid reads decorations from the overlay through the adapter (`RowState`, `CellFlags::MODIFIED/UNPARSED`) and displays new values.
- **Data structures and Rust placement** (`hacivat-core::edit::overlay`):
  ```rust
  pub enum NewValue { Value(Value), Null, Default, Unparsed { text: Arc<str>, err: ParseErrorKind } }
  pub struct RowUpdate { pub original: Arc<[Value]>, pub cells: BTreeMap<ColId, NewValue>, pub new_key: Option<RowKey> }
  pub struct NewRow { pub temp: TempId, pub anchor: Option<RowAnchor>, pub cells: BTreeMap<ColId, NewValue> }
  pub struct PendingChanges {
      pub updates: IndexMap<RowKey, RowUpdate>,     // insertion order = user order
      pub inserts: IndexMap<TempId, NewRow>,
      pub deletes: IndexSet<RowKey>,
      parked_updates: HashMap<RowKey, RowUpdate>,
      pub counters: Counters, version: u64,
  }
  ```
- **Complexity / performance budget:** O(1) amortized per edit; O(changes) for revert-all; memory ≈ originals of edited rows only; 100k pasted edits ≤ 200 ms to apply.
- **Edge cases (exhaustive list):** editing a key column (WHERE keeps the original); duplicate keys in results (editing disabled for those rows); rows without identity (no overlay entries possible); transposed edits (same overlay); the same table in two grids (independent overlays; post-submit hint); truncated LOB cells (full load first; full replacement only); edit → delete → undelete (edit restored); clone of an inserted row (copies its pending cells); insert then edit its key (fine: insert uses the final values); `Default` on a column without a default (allowed; server inserts NULL or errors — preview warns "no default defined"); undo/redo interplay (HV-EDIT-10); generation change by reload (re-match by key; orphans); local sort/filter (overlay untouched; view mapping keeps pending rows at anchors); the anchor row deleted (pending insert re-anchored to the previous visible row); 1M-row paste (bounded by the paste limit, HV-XFER-03); edits on a result whose session reconnected (overlay kept; submit uses the new connection); MCP edits (same overlay API with origin `mcp`).
- **Test cases (input → expected):** edit A→B→A ⇒ no pending changes; insert a row, then delete it ⇒ empty; edit row 1, delete row 1, undelete row 1 ⇒ row 1 still edited; edit on page 2, scroll back to page 1 ⇒ counter 1; local sort after editing ⇒ edits kept (DBeaver Q12 regression); reload after the row was deleted on the server ⇒ orphan flagged in the preview; DML-15 (delete SKU-3 and re-add it) ⇒ one delete + one insert, deletes first (HV-EDIT-07).
- **i18n notes:** `pending-summary = { $updated } updated, { $inserted } inserted, { $deleted } deleted`, `pending-guard-title`, `pending-guard-body = { $count -> [one] One change is *[other] { $count } changes are } not submitted.`, `edit-row-deleted`, `pending-orphan = Row is no longer in the loaded result.`
- **Verification status:** DataGrip storage [Observed: `MutationsStorageImpl`, `MutationsColorLayer`, dialogs]; DBeaver model [Observed: `ResultSetModel.java:500-716`]; Beekeeper rules [Observed: behaviour].
- **Prior art and evidence:** as merged.
- **Used by:** Hacivat grid/record view/paste; MCP `stage_edit`.

### HV-EDIT-03: Invalid-input retention and pre-submit validation

- **Priority / depends on:** v1. Depends on HV-GRID-11, HV-EDIT-02.
- **Sources merged:** datagrip/08 §3.8 (`UnparsedValue` kept with a typed error, blocks submit; boundary resolver "Value is out of range. Set …?"; typed error messages), datagrip/08 ALG-GRID-15 step 5, dbeaver-04 §3.18 ("DataGrip approach is better": keep bad input instead of an error dialog), DML-17 (invalid enum).
- **Problem:** Never lose what the user typed, never send invalid data, and catch obvious constraint violations before the round trip.
- **Inputs / outputs:** parse outcomes from HV-GRID-11, column constraints from the catalog → overlay entries `NewValue::Unparsed` or validated values + `Vec<ValidationIssue>` per change.
- **Decision:** invalid input is **kept** as `Unparsed{text, err}` (red border, tooltip with the error and a hint), never blocks the editor from closing, and blocks Submit (with a counter and "Go to next invalid value"). Client-side validation is **advisory except for parse errors**: length, precision/scale, NOT NULL, enum labels and check-like domain ranges produce warnings shown in the preview; the server stays the authority (constraints, triggers, RLS). Out-of-range values offer the boundary value (or ±infinity where the DB supports it) as a one-click fix.
- **Algorithm:**
  1. On commit from the editor/paste: `parse()` → `Ok(v)` → `NewValue::Value(v)`; `Ambiguous` → resolved by the editor or the paste column prompt; `Err(e)` → `NewValue::Unparsed{text, e}`.
  2. Validation per value (cheap, local): `Text{max_chars}` → length (in chars; for byte-length types, UTF-8 bytes) → warning `will_truncate` or `too_long` per engine strictness (MySQL strict mode errors); `Decimal{p, s}` → integer digits overflow → error-level warning, extra scale → `will_round` note with preview; `NOT NULL` without default and value NULL → warning; enum label unknown → `Unparsed` (it cannot be bound); `Int{bits}` range → `Unparsed(OutOfRange)` + boundary fix; temporal range (MSSQL `datetime` 1753–9999, MySQL TIMESTAMP 1970–2038) → `OutOfRange` + boundary fix.
  3. Inserts: required columns (NOT NULL, no default, not identity/generated) left "not set" → warning "will fail: { $column } is required" in the preview (not blocking; servers may have triggers that fill them).
  4. Submit gate: `unparsed_cells > 0` → Submit disabled; F8-style "next invalid" navigation; MCP gets a structured list.
- **Data structures and Rust placement:** `hacivat-core::edit::validate { validate(change, &ColumnMeta, &EngineMode) -> Vec<ValidationIssue> }`, `ValidationIssue { severity: Note | Warning | Error, kind, column, detail }`.
- **Complexity / performance budget:** O(1) per value; preview validation O(changes).
- **Edge cases (exhaustive list):** a value valid for one engine mode only (MySQL non-strict truncation vs strict error — read `sql_mode` at connect); text with trailing spaces in `CHAR(n)` (padding semantics; no warning); multi-byte chars vs byte-length limits (Oracle BYTE semantics later; MySQL `VARCHAR` counts chars); decimals with more scale (PG rounds on insert — preview shows the rounded value per server rules [Inferred: verify]); enum values added on the server after load (an unknown label may be valid → allow with a warning when the catalog is stale; refresh enum labels on submit failure); domain types with CHECK constraints (server-side only); `Default` value in a column without default (warning); Unparsed in a cell that later gets reverted (counter decrements); Unparsed in a deleted row (the delete drops it).
- **Test cases (input → expected):** `abc` into an int column → `Unparsed(InvalidNumber)`, Submit disabled, tooltip "Invalid number — use 1234.5 or 1.234,5"; `99999999999` into `int4` → `Unparsed(OutOfRange)` + "Set 2147483647?"; `foo` into an enum `status` (DML-17) → `Unparsed(UnknownEnumLabel)` with suggestions; a 300-character text into `varchar(255)` on MySQL strict → preview error-level warning; insert without the required `name` → preview warning, submit allowed; Decimal(10,2) `123.456` → note `will_round` preview `123.46`.
- **i18n notes:** error texts reuse `parse-*` ids from HV-GRID-11; `validate-too-long = { $column }: { $length } characters, maximum { $max }.`, `validate-required`, `validate-will-round`, `validate-out-of-range-fix = Set { $boundary }?`.
- **Verification status:** DataGrip behaviour [Observed: bundle, class names]; PG rounding mode [Inferred].
- **Prior art and evidence:** as merged.
- **Used by:** Hacivat; MCP (validation errors returned before submit).

### HV-EDIT-04: DML generation per dialect

- **Priority / depends on:** v1 for PostgreSQL, MySQL/MariaDB, SQLite, SQL Server. Depends on HV-EDIT-01, -02, -05; `hacivat-core::sql_render`.
- **Sources merged:** datagrip/08 ALG-GRID-05 (only changed columns, DEFAULT handling, generated/identity exclusion, batched DELETE IN for single-column keys, multi-row INSERT, DEFAULT VALUES / `() VALUES ()`, generated keys, binds, qualified quoted identifiers, casts, limits), dbeaver-04 ALG-DBV-GRID-02 (nested/composite values written as whole roots, original key for edited key columns, NULL keys `IS NULL`, NULL-pattern batching, value binders with casts, preview = same code path; defects Q6/Q7), Beekeeper ALG-EDIT-01 (order, parameter limits), DML-01…28 (doc 13).
- **Problem:** Turn the overlay into minimal, correct, parameterized DML for each dialect.
- **Inputs / outputs:** `PendingChanges`, identities, column metadata (native types, generated/identity flags, defaults), dialect capabilities, concurrency predicates (HV-EDIT-05) → `Vec<DmlStatement>` (typed) → rendered `(sql, binds)` per statement or batch.
- **Decision:**
  - **Typed statement model first, text last.** One generator produces `DmlStatement`s; the executor and the preview render from the same model (DBeaver's good property).
  - **UPDATE:** `SET` only changed columns; `Default` → `DEFAULT`; WHERE = original key values + concurrency predicate; never update generated/identity/rowversion columns; composite/array element edits write the whole column value.
  - **DELETE:** one statement per row (each carries its own concurrency predicate and is asserted individually). Bulk deletes of > 1,000 rows with single-column keys and **no** concurrency predicate requested ("fast delete" opt-in) may use `IN` lists chunked per dialect limits.
  - **INSERT:** only columns that were set (explicit NULL included — fixes DBeaver Q7); `Default` → keyword `DEFAULT` or omission; an all-unset row → `DEFAULT VALUES` (PG, SQLite, SQL Server) / `() VALUES ()` (MySQL); multi-row `VALUES` for rows with the **same set of columns**, chunked by the parameter limit; generated keys via `RETURNING`/`OUTPUT`/`LAST_INSERT_ID()` (HV-EDIT-08).
  - **Identity columns:** omitted unless the user typed a value → PG `GENERATED ALWAYS` needs `OVERRIDING SYSTEM VALUE`; SQL Server wraps with `SET IDENTITY_INSERT t ON … OFF`; MySQL `AUTO_INCREMENT` accepts explicit values.
  - **Binds with casts** where the protocol needs them: PG binds use the column's type OID (binary format) so that `jsonb`, `uuid`, enums, arrays, ranges, `interval`, `money` (`$1::numeric::money`) bind correctly; MySQL/SQLite bind text or native types; SQL Server binds `nvarchar` for Unicode text (never `varchar` with Turkish data).
  - **Identifiers:** fully qualified, quoted with the dialect's quote characters using the **catalog spelling** (verified necessity on SQL Server Turkish collations).
- **Algorithm:**
  1. Order: deletes → updates → inserts (DataGrip [Inferred] and Beekeeper; avoids unique collisions when a key is deleted and re-added). Within each group, user order (overlay insertion order).
  2. For each delete: `DELETE FROM q(T) WHERE k1 = $a AND k2 = $b [AND <concurrency>]`.
  3. For each update: `UPDATE q(T) SET c1 = $1, c2 = DEFAULT … WHERE <key on original values> [AND <concurrency>]`.
  4. Inserts: group consecutive rows by their column set; for each group, `INSERT INTO q(T) (cols) VALUES (…), (…) [RETURNING *]` chunked so that `rows × cols ≤ limit − reserve`.
  5. Key predicate rendering: `k = $n` for non-NULL; `k IS NULL` for NULL key values (virtual keys only).
  6. Attach to each statement: `expect: ExactlyOne` (update/delete) or `Exactly(n)` (insert of n rows), the overlay entries it covers, and the structured intent for Karagöz.
- **Data structures and Rust placement** (`hacivat-core::edit::dml`):
  ```rust
  pub enum DmlStatement {
      Delete { table: QualifiedName, key: Vec<(ColRef, Value)>, guard: Option<ConcurrencyGuard>, covers: RowKey },
      Update { table: QualifiedName, set: Vec<(ColRef, SetValue)>, key: Vec<(ColRef, Value)>, guard: Option<ConcurrencyGuard>, covers: RowKey },
      Insert { table: QualifiedName, cols: Vec<ColRef>, rows: Vec<Vec<SetValue>>, overriding: Option<Overriding>, returning: Returning, covers: Vec<TempId> },
  }
  pub enum SetValue { Value(Value), Null, Default }
  pub enum Expect { ExactlyOne, Exactly(u64) }
  pub trait DmlDialect: SqlDialect {
      fn render(&self, s: &DmlStatement, mode: RenderMode /* Bind | Literal */) -> Rendered { sql: String, binds: Vec<BindValue> };
      fn param_limit(&self) -> usize; fn supports_returning(&self, kind: DmlKind) -> bool; fn default_values(&self) -> DefaultValuesSyntax;
  }
  ```
- **Per-DBMS matrix:**

  | Concern | PostgreSQL | MySQL/MariaDB | SQL Server | SQLite |
  |---|---|---|---|---|
  | Identifier quote | `"x"` | `` `x` `` (or `"` with ANSI_QUOTES) | `[x]` | `"x"` |
  | Max bind parameters | 65,535 | 65,535 | 2,100 (keep ≤ 2,000) | 32,766 (SQLite ≥ 3.32) |
  | All-default insert | `DEFAULT VALUES` | `() VALUES ()` | `DEFAULT VALUES` | `DEFAULT VALUES` |
  | `DEFAULT` keyword in VALUES/SET | yes | yes | yes | **no** → omit the column (insert) / refuse (update: "SQLite cannot set DEFAULT in UPDATE") |
  | Returning | `RETURNING *` | MariaDB ≥ 10.5 `INSERT … RETURNING`, `DELETE … RETURNING`; MySQL none | `OUTPUT inserted.*` (into a table variable when triggers exist) | `RETURNING *` (≥ 3.35) |
  | Identity insert | `OVERRIDING SYSTEM VALUE` (ALWAYS) | explicit value OK | `SET IDENTITY_INSERT t ON/OFF` | explicit value OK |
  | Never written | generated columns | generated columns | computed, `rowversion` | generated columns |
  | Unicode text binds | text | utf8mb4 connection | `nvarchar` | text |
  | Casts | by OID (binary binds) | — | typed RPC params | — |
- **Complexity / performance budget:** generation O(changes); rendering ≤ 1 ms per 1,000 statements; multi-row INSERT of 10k rows × 10 columns → PG 2 statements, SQL Server 50 statements.
- **Edge cases (exhaustive list):** composite keys; float/timestamp keys (bind exact originals — HV-EDIT-01 avoids float keys); case-sensitive and reserved identifiers (always quoted); identifiers with quotes inside (`"a""b"`); PG json/array/enum/uuid/bytea/interval/range casts; MySQL `BIT` (bind as integer or `b'…'`); SQL Server `rowversion` (never written), `sql_variant`; computed columns; triggers affecting other rows (counts unaffected, HV-EDIT-06); time-zone conversions (`timestamptz` binds as UTC instants; MySQL `TIMESTAMP` in the session zone; naive timestamps verbatim); floats displayed rounded (bind exact stored value); updatable views (only when enabled); result aliases → base columns (from origins); batch splitting by parameter limits; Oracle 1,000-item IN limit (later); key column edited plus other columns (one UPDATE; WHERE on the original key; SET includes the key); two rows swapping unique values (fails with a unique violation; the error suggests "submit in two steps" — deferrable constraints are not assumed); empty string vs NULL on Oracle (later: Oracle treats `''` as NULL — warn); MySQL `NO_BACKSLASH_ESCAPES` (binds unaffected; only the literal renderer cares); MySQL `sql_safe_updates` (our UPDATEs always have key WHERE clauses → fine); SQLite `UPDATE … SET c = DEFAULT` unsupported.
- **Test cases (input → expected):** DML-01 PG single cell → `UPDATE "public"."customers" SET "city" = $1 WHERE "id" = $2 AND "xmin" = $3` (table editor with xmin guard, HV-EDIT-05); without a version token → `… WHERE "id" = $2 AND "city" IS NOT DISTINCT FROM $3`; DML-02 composite key → `WHERE "order_id" = $2 AND "line_no" = $3`; DML-05 Set NULL → `SET "city" = $1` with bind NULL; DML-06 Set DEFAULT → `SET "is_active" = DEFAULT`; DML-07 insert with only `customer_id` → `INSERT INTO "public"."orders" ("customer_id") VALUES ($1) RETURNING *`; MySQL all-default insert → ``INSERT INTO `t` () VALUES ()``; explicit NULL into a column with a default (DBeaver Q7 regression) → the column is listed with NULL; DML-16 PK edit → `UPDATE … SET "sku" = $1 WHERE "sku" = $2`; DML-24 MSSQL identity value typed → wrapped in `SET IDENTITY_INSERT [dbo].[t] ON; … ; SET IDENTITY_INSERT [dbo].[t] OFF;`; DML-25 MSSQL Turkish text → bound as `nvarchar`; 3,000 rows × 1 column insert on SQL Server → 2 statements (2,000 + 1,000 params).
- **i18n notes:** generated SQL is never localized; keyword case follows the user's SQL code style setting (uppercase default); error messages for unsupported combinations are Fluent ids (`dml-sqlite-no-default-update`, `dml-unique-swap-hint`).
- **Verification status:** DataGrip DML shapes [Unknown → DML family]; DBeaver shapes [Observed: `JDBCTable.java:445-617`, `ExecuteInsertBatchImpl.java:72-200`] and tests [Observed: `DBDResultSetDataUpdaterTest.java:45-176`]; parameter limits [Inferred: vendor docs]; MariaDB RETURNING versions [Inferred: verify]; MSSQL OUTPUT-with-triggers rule [Inferred: vendor docs — error 334].
- **Prior art and evidence:** as merged.
- **Used by:** Hacivat; Karagöz (re-validates the structured intent against the SQL; authorizes per table/column).

### HV-EDIT-05: Optimistic concurrency

- **Priority / depends on:** v1. Depends on HV-EDIT-01 (version column), -02 (originals), HV-RES-04 (hidden columns in table editors).
- **Sources merged:** Beekeeper ALG-EDIT-01 step 4 (original-value predicate per modified column, exclusions, `xmin`/rowversion alternatives) and Compass's "update unless changed in background" pattern (behaviour), dbeaver-04 ALG-DBV-GRID-02 recommendation and §3.18 (add it; DBeaver and DataGrip have none), datagrip/08 ALG-GRID-06 (optional optimistic check), DML-18/20 (doc 13), verification/01 #26.
- **Problem:** Detect that a row changed or disappeared on the server since it was read, instead of silently overwriting someone else's change (the "lost update").
- **Inputs / outputs:** identity with optional version column, original values, changed columns, column types → a `ConcurrencyGuard` per UPDATE/DELETE; conflict reports with server values.
- **Decision:**
  - **On by default for every connection** (DBeaver suggested production-only; we choose always, because the cost is one extra predicate and the benefit is correctness). Per-connection setting "Conflict detection: Version token / Changed columns / Off".
  - **Guard ladder (first available):**
    1. **Version token:** SQL Server `rowversion` column; PG `xmin` (hidden column added by table editors; also used when a console result selected `xmin` explicitly); a user-declared version column from the virtual model (`version`, `updated_at`; v1.x).
    2. **Changed-column originals** (UPDATE): for each changed column of a comparable type, `col IS NOT DISTINCT FROM $orig` (PG; MySQL `<=>`; SQLite `IS`; SQL Server `(col = @p OR (col IS NULL AND @p IS NULL))`).
    3. **Loaded-column originals** (DELETE): all loaded comparable columns (cap 32 columns, key columns excluded), so that a delete does not remove a row someone just changed.
  - **Non-comparable types are excluded** from 2 and 3: floating point, truncated/LOB values, PG `json` (no equality operator; `jsonb` is fine), `xml`, geometry, MSSQL `text/ntext/image`, arrays of these, `Opaque`. If nothing comparable remains → key-only predicate with a preview note "Concurrent changes to this row cannot be detected".
  - **On conflict (0 rows matched):** roll back the submit (HV-EDIT-07), re-select the row by key (side query), and show a conflict dialog per row: *Deleted on server* → Discard my change / Re-insert (for updates: convert to insert, opt-in); *Changed on server* → three-way view (original | mine | server) with **Reload row** (discard mine), **Keep mine** (re-submit with the guard rebuilt from the server's current values — an explicit, informed overwrite), **Cancel**.
  - Rejected: key-only WHERE (DBeaver/DataGrip status quo); rejected: compare all columns for UPDATE (false conflicts from triggers updating `updated_at` in unrelated columns are acceptable for DELETE but noisy for UPDATE).
- **Algorithm:**
  1. At first edit, the overlay captured the original row including hidden version columns (HV-EDIT-02).
  2. Build the guard per statement following the ladder; record which columns were compared (for the preview and the dialog).
  3. After execution, HV-EDIT-06 classifies 0-row results as conflicts when a guard was present (otherwise "row not found").
  4. Conflict resolution as above; "Keep mine" rebuilds the guard from the re-selected values and re-runs the whole submit (other changes included) inside a new transaction.
  5. PG `xmin` details: `xmin` changes on every UPDATE of the row (including no-op updates and HOT updates); VACUUM FREEZE keeps the raw `xmin` value ([Observed: verification/01 #26]), so equality checks remain valid across freezing; compare as `xid` text (`"xmin"::text = $n`) because `xid` has no `=` operator with bound `bigint` parameters in older versions [Inferred: verify the binding form].
- **Data structures and Rust placement** (`hacivat-core::edit::concurrency`):
  ```rust
  pub enum ConcurrencyGuard { Version { col: ColRef, value: Value }, Originals { cols: Vec<(ColRef, Value)> }, None { reason: NoGuardReason } }
  pub enum Conflict { Deleted { key: RowKey }, Changed { key: RowKey, original: Arc<[Value]>, mine: Arc<[Value]>, server: Arc<[Value]> } }
  ```
- **Per-DBMS matrix:**

  | Engine | Version token | Null-safe equality | Notes |
  |---|---|---|---|
  | PostgreSQL | `xmin` (system column; table editors add it hidden) | `IS NOT DISTINCT FROM` | `jsonb` comparable, `json` not |
  | MySQL/MariaDB | none (user version column, v1.x) | `<=>` | needs `CLIENT_FOUND_ROWS` (HV-EDIT-06) |
  | SQL Server | `rowversion`/`timestamp` column if present | expanded OR form | `text/ntext/image` not comparable |
  | SQLite | none | `IS` | dynamic typing: compare with the stored storage class (bind the original exactly as read) |
- **Complexity / performance budget:** one extra predicate per statement (the key predicate keeps it index-driven); conflict re-select ≤ 1 round trip per conflicting row.
- **Edge cases (exhaustive list):** triggers that modify the row on UPDATE (`updated_at`) → a second edit of the same row in a later submit uses the reconciled values (HV-EDIT-08), so no false conflict; NULL originals (null-safe equality); float columns (excluded); truncated values (excluded); time-zone conversions (originals bound as read, typed); text with trailing spaces in `CHAR(n)` (PG `bpchar` comparison ignores trailing spaces → fine); case-insensitive collations (`'a' = 'A'` true → a concurrent case-only change is not detected; documented); MySQL "found rows" vs "changed rows" (with `CLIENT_FOUND_ROWS` a matching row with identical new values counts as 1); RLS policies hiding the row after a concurrent change (shows as Deleted; message mentions RLS); a guard on a column the user also changes (compare the original, set the new value — correct); multi-row submit with one conflict (whole submit rolled back; other rows stay pending); PG `xmin` on a partitioned table row moved between partitions (UPDATE of partition key → the old ctid/xmin are gone → conflict Deleted on a second edit — re-select by key finds it); Karagöz-proxied sessions (guards travel with the structured intent).
- **Test cases (input → expected):** DML-20: another session changes `city`; the user edits `full_name` and submits → table editor with `xmin` guard → 0 rows → conflict "Changed on server" showing the new city; console result without `xmin` → guard on `full_name` original → 1 row updated (no conflict, because only `full_name` is compared — documented behaviour of the changed-columns rung); DML-18: row deleted by another session, user edits it → conflict "Deleted on server"; delete of a row whose `status` changed meanwhile → loaded-column guard → conflict; a row with a `json` column changed by someone else → not detected (json excluded) and the preview said so; "Keep mine" → re-submit succeeds with the new guard.
- **i18n notes:** `conflict-title`, `conflict-deleted = Row { $key } was deleted on the server.`, `conflict-changed = Row { $key } was changed on the server after you loaded it.`, column headers `conflict-original`, `conflict-mine`, `conflict-server`, actions `conflict-reload-row`, `conflict-keep-mine`, `conflict-cancel`, preview note `concurrency-not-detectable`.
- **Verification status:** [Observed: verification/01 #26] raw `xmin` preserved by freezing (equality remains valid; the age-based introspection use case was refuted, which does not affect equality); DataGrip and DBeaver lack the feature [Observed: dbeaver-04 Q6/§3.18; datagrip/08 §3.9 Unknown → DML-20]; `xmin` binding form [Inferred: verify in Docker].
- **Prior art and evidence:** Beekeeper (behaviour), Compass hadron-document conditional update (behaviour).
- **Used by:** Hacivat; MCP edits (guards mandatory); Karagöz (may require guards by policy).

### HV-EDIT-06: Affected-row assertion

- **Priority / depends on:** v1. Depends on HV-EDIT-04.
- **Sources merged:** datagrip/08 ALG-GRID-06 (require exactly 1; 0 → conflict; > 1 → rollback + error; MySQL found vs changed rows; `SET NOCOUNT ON`; ClickHouse async), dbeaver-04 Q6 (DBeaver ignores > 1 and assumes unknown batch counts are 1), DML-19 (trigger updates other rows).
- **Problem:** Guarantee that each row-level statement touched exactly the intended number of rows, and fail the whole submit otherwise.
- **Inputs / outputs:** executed statement + `Expect` + driver count → `Ok | Conflict | RowNotFound | TooMany(n) | CountUnknown`.
- **Decision:** exact counts are mandatory; when a driver cannot report a reliable count, we obtain one explicitly (SQL Server `SELECT @@ROWCOUNT` in the same batch); "unknown" is treated as a failure, never as 1.
- **Algorithm:**
  1. After each statement read its own affected count: PG `CommandComplete` tag; MySQL OK packet `affected_rows` with `CLIENT_FOUND_ROWS`; SQLite `changes()`; SQL Server: append `; SELECT @@ROWCOUNT AS hv_rc` to each DML batch and read that value (immune to `SET NOCOUNT ON` and to trigger `DONEINPROC` counts).
  2. Compare: `ExactlyOne`: 0 → `Conflict` (guard present) or `RowNotFound`; 1 → Ok; > 1 → `TooMany(n)`. Inserts `Exactly(n)`: mismatch → failure.
  3. Any failure → abort the submit (rollback, HV-EDIT-07) and mark the covered overlay entries with the error.
- **Data structures and Rust placement:** `hacivat-core::edit::assert { check(expect, count: Option<u64>) -> AssertOutcome }`.
- **Per-DBMS matrix:** listed in step 1; ClickHouse not editable in v1.
- **Complexity / performance budget:** zero extra round trips except on SQL Server (one extra result in the same batch).
- **Edge cases (exhaustive list):** MySQL without `CLIENT_FOUND_ROWS` (identical values → 0 → false conflict; the connection flag is mandatory); triggers changing other rows (PG/MySQL/SQLite counts cover the target statement only; SQL Server handled via `@@ROWCOUNT` right after the statement); `INSTEAD OF` triggers on views (counts may be 0 or arbitrary → views are opt-in and the preview warns); RLS (0 rows); FDW/foreign tables reporting -1 (CountUnknown → failure with a hint "the foreign data wrapper does not report row counts; disable the check for this table" (per-table opt-out)); rules (PG `CREATE RULE` rewriting the statement → counts of the rewritten statement); multi-row INSERT counts; `INSERT … ON CONFLICT` (not generated by us).
- **Test cases (input → expected):** DML-19: trigger updates 2 rows on update of one row → PG count 1 → Ok; virtual key matching 2 duplicates → `TooMany(2)` → rollback, error "2 rows matched; expected 1. The chosen key is not unique."; MySQL update to the same value with `CLIENT_FOUND_ROWS` → 1 → Ok; SQL Server with `SET NOCOUNT ON` in the session → `@@ROWCOUNT` = 1 → Ok.
- **i18n notes:** `assert-too-many = { $count } rows matched; expected 1. The key used for this row is not unique.`, `assert-not-found = The row was not found (it may have been deleted or hidden by a row-level security policy).`, `assert-count-unknown`.
- **Verification status:** DBeaver defect [Observed: `ResultSetPersister.java:576-616`, `ExecuteBatchImpl.java:311-330`]; DataGrip [Unknown → DML-18/19]; SQL Server `@@ROWCOUNT` approach [Inferred: vendor docs].
- **Prior art and evidence:** as merged.
- **Used by:** Hacivat; Karagöz ("max affected rows" policy).

### HV-EDIT-07: Submit pipeline and transactions

- **Priority / depends on:** v1. Depends on HV-EDIT-03…06, HV-EXEC-04 (gate: read-only, policy), HV-EXEC-05 (`ApplyDml` request), HV-EXEC-10 (tx state).
- **Sources merged:** datagrip/08 §3.9–3.10 (Submit Ctrl/⌘+Enter; Auto vs Manual tx; Submit and Commit; Roll Back reverts edits; immediate submit; leave guard; query errors dialog), dbeaver-04 ALG-DBV-GRID-02 steps 5 and 7 (savepoint in manual mode, stop at first error, smart commit; partial application under auto-commit — to avoid), dbeaver-04 §3.10 (save preview on production connections), Beekeeper ALG-EDIT-01 steps 6 and 8, DML-29/30.
- **Problem:** Apply the overlay atomically, in the right transaction context, with clear outcomes, without ever leaving half-applied changes in auto-commit mode or poisoning a user's open transaction.
- **Inputs / outputs:** overlay snapshot, session state → `SubmitOutcome { Applied { stats, reconciled } | Failed { at, error, rolled_back } | Conflicts(Vec<Conflict>) | Cancelled }`.
- **Decision:**
  - **Auto-commit mode:** `BEGIN` → all statements → `COMMIT`. All-or-nothing (DBeaver applies partially; DataGrip's auto mode behaviour is [Unknown]).
  - **Manual mode:** `SAVEPOINT hv_submit_<n>` → statements → `RELEASE SAVEPOINT`; on failure `ROLLBACK TO SAVEPOINT` (the user's transaction stays usable — essential on PG). No commit; the Tx badge counts the submit; "Submit and Commit" (Ctrl+Alt+Shift+Enter) commits after a successful submit; "Roll Back" rolls back the transaction **and** offers to restore the submitted changes as pending (DataGrip's `RevertAndRollback` reverts them).
  - **Preview first** when: the connection is production-tagged (mandatory), or the user enabled "Always preview", or the submit contains deletes of > 100 rows.
  - Statements execute through the session actor as one `ApplyDml` request (no interleaving with other requests); PG uses pipelining for many statements (send all, then read results), stopping at the first error.
  - Deadlock/serialization failures (`40P01`, `40001`, MySQL `1213`, SQL Server `1205`) → offer **Retry** (re-run the whole submit in a new transaction).
- **Algorithm:**
  1. Preconditions: overlay non-empty; `unparsed == 0`; identities re-validated; gate: connection read-only → deny; Karagöz policy per table/column (via the structured intent) → deny with the policy message.
  2. Generate statements (HV-EDIT-04) with guards (HV-EDIT-05); validation warnings (HV-EDIT-03).
  3. Preview if required (HV-EDIT-09); the user confirms.
  4. Freeze the overlay snapshot (edits made during submit go to a new overlay layer applied after completion).
  5. Release a suspended cursor on the session if any (HV-RES-05).
  6. Open the transaction context (BEGIN or SAVEPOINT). Execute statements in order; after each, HV-EDIT-06 assertion; collect RETURNING/OUTPUT rows.
  7. On the first failure: rollback (or rollback to savepoint); mark the failing overlay entry with the server error (verbatim, ALG-I18N-08) and keep **all** changes pending; conflicts → HV-EDIT-05 dialog.
  8. On success: COMMIT (auto mode) or RELEASE (manual); reconcile (HV-EDIT-08); clear the submitted entries; history (origin `grid`, a summary line plus the full statements in details); audit event; status line "N updated, M inserted, K deleted".
  9. Cancellation during submit → rollback; changes stay pending.
  10. Connection loss: before COMMIT was sent → the server rolled back; changes stay pending. After COMMIT was sent but before the acknowledgement → outcome **unknown**: keep the changes pending but mark them "outcome unknown", reconnect, re-select the affected keys and show what the server has; never replay automatically.
  11. Immediate-submit mode: steps 1–10 for each confirmed edit (one row), with a delete confirmation dialog.
- **Data structures and Rust placement:** `hacivat-core::edit::submit { SubmitPlan, SubmitOutcome, submit(plan, &SessionHandle) }`; the session-side execution in `hacivat-core::exec::session` as `Request::ApplyDml`.
- **Per-DBMS matrix:**

  | Engine | Auto mode wrapper | Manual mode wrapper | Notes |
  |---|---|---|---|
  | PostgreSQL | `BEGIN` / `COMMIT` | `SAVEPOINT` / `RELEASE` / `ROLLBACK TO` | after an error the tx is aborted until `ROLLBACK TO` |
  | MySQL/MariaDB | `START TRANSACTION` / `COMMIT` | `SAVEPOINT` … | DDL is never generated here (no implicit commits) |
  | SQL Server | `BEGIN TRAN` / `COMMIT` | `SAVE TRANSACTION hv_n` / `ROLLBACK TRANSACTION hv_n` | `XACT_ABORT` off for our batch; check `XACT_STATE()` after errors |
  | SQLite | `BEGIN IMMEDIATE` / `COMMIT` | `SAVEPOINT` … | `IMMEDIATE` avoids `SQLITE_BUSY` upgrade deadlocks |
- **Complexity / performance budget:** submit of 100 row edits on a LAN ≤ 200 ms (pipelined on PG); 10k inserts ≤ 2 s via multi-row VALUES; UI stays responsive (progress for > 500 ms).
- **Edge cases (exhaustive list):** auto-commit OFF without savepoint support (not among v1 engines); a user transaction in the aborted state before submit (PG: refuse with "roll back first"); submit while the session is busy (queued); grid DML while a console holds a suspended cursor (release first, HV-EXEC-05); deadlock/serialization (Retry); unique violations (error on the entry, constraint name from structured fields); FK violations (same; the hint suggests ordering inserts or submitting the parent first); NOT NULL violations; check constraints; triggers raising errors (`P0001` verbatim); permission denied (`42501`); read-only replica (`25006`); statement timeout; connection loss at each step (step 10); cancel; Karagöz policy denial; huge submits (≥ 100k statements → batch progress, cancellable); the same row edited in two grids and both submitted (the second hits a conflict via the guard — the intended behaviour).
- **Test cases (input → expected):** auto mode, 3 updates, the second violates a unique constraint → no change persists (all-or-nothing), the second entry shows the error, all 3 remain pending; manual mode, same → `ROLLBACK TO SAVEPOINT`, the user's earlier statements in the transaction are intact (PG), all 3 pending; DML-29 manual tx submit → invisible to another session until Commit; DML-30 immediate submit → each edit committed at once; production connection → preview shown before execution; connection killed after COMMIT sent (simulated) → "outcome unknown" banner and a re-select showing that the rows were committed.
- **i18n notes:** `submit-done = { $updated } updated, { $inserted } inserted, { $deleted } deleted`, `submit-failed = Changes were not applied: { $error }`, `submit-rolled-back`, `submit-outcome-unknown = The connection was lost while committing. Hacivat is checking what the server stored.`, `submit-retry-deadlock`, `tx-submit-and-commit`, `tx-rollback-restore-pending = Restore the rolled-back changes as pending edits?`.
- **Verification status:** DataGrip behaviours [Observed: help, bundle; auto-mode atomicity Unknown]; DBeaver savepoint logic [Observed: `DBDResultSetDataUpdater.java:316-475`]; DBeaver test `canceledExecutionLeavesStatementPendingWithoutError` [Observed] matches our cancel rule.
- **Prior art and evidence:** as merged.
- **Used by:** Hacivat; MCP `submit_edits` (always previewed in the tool result first; requires confirmation per MCP policy, doc mcp/02).

### HV-EDIT-08: Reconcile after submit (no full reload)

- **Priority / depends on:** v1. Depends on HV-EDIT-07, HV-RES-02 (patches/tombstones/local appends).
- **Sources merged:** datagrip/08 ALG-GRID-06 (`SelectUsingKeys` re-read after commit), dbeaver-04 ALG-DBV-GRID-02 step 7 and §3.10 (generated keys matched by name; per-row refresh; rows invisible after refresh), Beekeeper ALG-EDIT-01 step 5 (RETURNING/OUTPUT), ui-01 generation idea.
- **Problem:** Show exactly what the server stored (defaults, triggers, generated keys, rounding) right after a submit, without reloading the whole result or losing the scroll position.
- **Inputs / outputs:** submit results (RETURNING rows or keys) → store patches, tombstones, appended rows; overlay cleared.
- **Decision:** prefer `RETURNING *` / `OUTPUT inserted.*` in the same round trip; otherwise re-select by key in **one** batched query per table (`WHERE (k) IN (…)` chunked) inside the same transaction before COMMIT (manual mode: inside the savepoint) so that the values are consistent with what was written. Patch the store (HV-RES-02) instead of reloading: updated rows → `patches`; deleted rows → `tombstones`; inserted rows → `local_appends` placed at their anchors in the view (HV-GRID-06). The result's `generation` does not change; selection and scroll stay.
- **Algorithm:**
  1. Inserts: map RETURNING rows to temp ids in statement order (multi-row VALUES preserves order in PG/SQLite RETURNING [Inferred: PG returns rows in VALUES order for simple inserts — verify]; if order is not guaranteed, match by the inserted non-generated values, else re-select). MySQL: `LAST_INSERT_ID()` gives the first auto-increment value of a multi-row insert; consecutive ids are guaranteed only with `innodb_autoinc_lock_mode` 0/1 → re-select by the inserted values if mode 2 [Inferred: verify].
  2. Updates: RETURNING row → patch; no RETURNING → re-select by (new) key.
  3. Deletes → tombstones.
  4. A re-selected row missing (RLS, trigger moved it, a filter of the table editor no longer matches) → keep the last known values with a "not visible after save" flag (DBeaver idea) instead of removing it silently.
  5. Columns used by the current server ORDER BY that changed → the row stays where it is until the user re-sorts (hint "order may have changed").
  6. Other grids showing the same table → "Data changed — Reload" hint.
  7. History and audit events (HV-EXEC-06/12) record the submit.
- **Data structures and Rust placement:** `hacivat-core::edit::reconcile { reconcile(plan, results, &ResultStore) -> StorePatch }`.
- **Per-DBMS matrix:** PG `RETURNING *`; SQLite ≥ 3.35 `RETURNING *`; MariaDB ≥ 10.5 `INSERT/DELETE … RETURNING`, UPDATE without RETURNING → re-select; MySQL → re-select (`LAST_INSERT_ID()` for auto-increment); SQL Server `OUTPUT inserted.* INTO @t` (table variable, works with triggers) + `SELECT * FROM @t`.
- **Complexity / performance budget:** zero or one extra round trip per table; patch publication ≤ 1 ms; no full reload.
- **Edge cases (exhaustive list):** generated keys for multi-row inserts; triggers changing values (RETURNING reflects BEFORE-trigger changes; AFTER-trigger changes need the re-select → PG: use re-select when AFTER UPDATE triggers that modify the same row exist — rare; documented); RLS hiding the row; rows moving out of the table editor's filter; key changes (patched row gets the new key; the overlay re-keys); LOB columns (the re-select uses the LOB prefix rules); truncated values edited (store the new full value as written, or its prefix + length); console results whose statement is not a single-table query (patches still apply by identity); the result evicted from memory meanwhile (patches apply to whatever is loaded; evicted chunks reload later from the server anyway); SQL Server OUTPUT with computed columns (included); MySQL `ON UPDATE CURRENT_TIMESTAMP` columns (re-select shows them).
- **Test cases (input → expected):** DML-02 edit `qty` → `line_total` (generated column) is shown updated after submit without reload; DML-07 insert into `orders` → `id` and `status` default appear in the new row; delete of row 7 → row disappears, scroll position unchanged, row count −1; a submit on a 1M-row loaded result → no re-execution, selection preserved; PG update of a row hidden afterwards by RLS → row stays visible with "not visible after save".
- **i18n notes:** `reconcile-not-visible = This row is not visible to you after saving.`, `reconcile-order-changed`, `reconcile-other-grids = { $table } changed — reload?`.
- **Verification status:** DataGrip re-read [Observed: `SelectUsingKeys`]; DBeaver per-row refresh [Observed: `ResultSetPersister.java:714-827`]; RETURNING ordering and MySQL autoinc modes [Inferred: verify in Docker].
- **Prior art and evidence:** as merged.
- **Used by:** Hacivat.

### HV-EDIT-09: DML preview and the dialect literal renderer

- **Priority / depends on:** v1. Depends on HV-EDIT-04; the literal renderer is shared with HV-RES-12 (filter text), HV-XFER-06 (SQL INSERT/UPDATE extractors) and the parameter dialog's "insert as literal".
- **Sources merged:** datagrip/08 ALG-GRID-05 step 5 and §3.9 (Preview Pending Changes always available; literals only for preview), dbeaver-04 ALG-DBV-GRID-02 (preview = same code path; NATIVE literal rendering; inline values while skipping quoted regions; named parameters unsupported), DBeaver `JDBCDateTimeValueHandlerTest` (quote exactly once), DML-12/25/26 (literal forms), Beekeeper ALG-EDIT-01 step 7 (banner "parameters will be bound").
- **Problem:** Show users exactly what will run, readable and copyable as valid SQL, without the preview ever diverging from execution or becoming an injection vector.
- **Inputs / outputs:** `DmlStatement`s, dialect, session settings (MySQL `sql_mode`, PG `standard_conforming_strings`) → preview text (literal mode) and the exact parameterized form; per-statement annotations (guards, warnings, orphans).
- **Decision:**
  - The preview lists statements **in execution order** with transaction wrappers, rendered in **literal mode** by the same renderer that renders bind mode; a toggle shows the exact parameterized SQL plus the bind list. A banner states "Values are sent as bound parameters".
  - Long values are shortened **in the preview only** (`'…' /* 5.0 MB */`); "Copy to console" copies full literals (with a size confirmation above 10 MB).
  - **Literal renderer rules** (injection-safe by construction, fuzz-tested): strings `'…'` with `'` doubled; PG uses `E'…'` only when `standard_conforming_strings = off` (and then escapes `\`); MySQL escapes `\` as `\\` unless `NO_BACKSLASH_ESCAPES` is set; SQL Server `N'…'` for Unicode types; NUL characters → engine-specific (PG cannot store them → error; MySQL `\0`; SQL Server `CHAR(0)` concatenation); binary → PG `'\x…'::bytea`, MySQL `X'…'`, SQL Server `0x…`, SQLite `X'…'`; booleans → PG/MySQL `TRUE/FALSE`, SQL Server `1/0`, SQLite `1/0`; dates/times → typed literals (`DATE '…'`, `TIMESTAMP '…'`, `TIMESTAMPTZ '…'` for PG; MySQL `'…'`; SQL Server `CAST('…' AS datetime2(7))`, `datetimeoffset`); decimals and integers → canonical text (never locale-formatted); floats → shortest round-trip text, `'NaN'::float8`/`'Infinity'` for PG; UUID → `'…'::uuid` (PG), `'…'` elsewhere; JSON → `'…'::jsonb`/`::json` (PG), `CAST('…' AS JSON)` (MySQL); arrays → PG `ARRAY[…]` or `'{…}'::type[]`; enums → `'label'::enum_type` (PG); intervals → `INTERVAL '…'`; NULL → `NULL`; DEFAULT → `DEFAULT`.
  - Identifiers via the dialect quoting with the catalog spelling.
- **Algorithm:** render each `DmlStatement` twice (Bind, Literal) from the same AST; annotate guards ("conflict detection on xmin" / "changed columns: city" / "not detectable"), validation warnings, orphan flags; present grouped per table with counts; Execute / Cancel / Copy.
- **Data structures and Rust placement:** `hacivat-core::sql_render::{LiteralRenderer, quote_ident}`; `hacivat-core::edit::preview::Preview { statements: Vec<PreviewStatement { literal: String, parameterized: String, binds: Vec<BindValue>, notes } > }`.
- **Complexity / performance budget:** preview of 10k statements renders ≤ 100 ms and displays virtualized.
- **Edge cases (exhaustive list):** quotes, backslashes, newlines, NUL, emoji and RTL characters in strings (RTL text rendered with bidi isolation in the preview UI; the SQL text itself is untouched); MySQL `ANSI_QUOTES` (identifier quoting switches to `"`); PG `standard_conforming_strings = off` (legacy); SQL Server `QUOTED_IDENTIFIER OFF` (we always use brackets); values containing comment markers (`--`, `/*`) — inside quotes, harmless; very long values; binary 0 bytes; float specials; `-0`; dates beyond 9999 / BC (PG `'0044-03-15 BC'`); MySQL zero dates (`'0000-00-00'`); time zones (`TIMESTAMPTZ '2026-11-11 07:00:00+00'`); preview vs bind divergence (impossible by construction; property test: executing the literal form and the bind form against a scratch DB yields identical rows).
- **Test cases (input → expected):** `O'Brien` → PG `'O''Brien'`; MySQL `C:\temp` → `'C:\\temp'` (DML-26); SQL Server Turkish `Müşteri` → `N'Müşteri'` (DML-25); PG jsonb `{"a":1}` → `'{"a":1}'::jsonb` (DML-12); PG `text[]` `{a,b}` → `'{a,b}'::text[]` (DML-13); bytea `0xDEAD` → `'\xdead'::bytea`; timestamptz → `TIMESTAMPTZ '2026-11-11 07:00:00+00'`; fuzz: 1M random strings rendered as literals and executed in `SELECT <literal>` on each engine round-trip byte-identically.
- **i18n notes:** preview chrome strings `preview-title`, `preview-bound-banner`, `preview-toggle-parameterized`, `preview-guard-*`; the SQL itself is never translated.
- **Verification status:** DBeaver literal tests [Observed: `JDBCDateTimeValueHandlerTest.java:69-121`]; DML-12/25/26 open; fuzz test is a Hacivat requirement.
- **Prior art and evidence:** as merged.
- **Used by:** grid preview, filter "convert to text" (HV-RES-12), SQL extractors (HV-XFER-06), parameter dialog (HV-EXEC-03), MCP (preview text returned to agents).

### HV-EDIT-10: Undo/redo of edits

- **Priority / depends on:** v1. Depends on HV-EDIT-02.
- **Sources merged:** datagrip/08 §3.8 (undoable grid edits; "Cannot undo database modification" after submit), dbeaver-04 §3.9 (`ResultSetUndoRedoManager`, depth ≤ 10,000, cleared after save or reject).
- **Problem:** Let users step back through pending edits with familiar shortcuts, with paste and multi-cell edits as single steps.
- **Inputs / outputs:** overlay mutations → an undo stack of inverse diffs; undo/redo commands.
- **Decision:** every overlay mutation produces an `EditStep` (inverse diff); one user action = one step (a paste of 10k cells, a multi-cell typed value, "Set NULL" on a selection, delete rows, revert); depth 10,000 steps or 64 MiB of diffs; the stack is cleared by a successful submit (message "Changes were submitted; undo is not available — use the history or a new edit"), by discard, and by a generation change of a different statement.
- **Algorithm:** `apply(step)` pushes `inverse(step)` onto `undo`, clears `redo`; `undo()` pops, applies the inverse to the overlay and pushes the step onto `redo`; selection is restored to the cells of the step; steps referencing rows that became orphans still apply (they are keyed by `RowKey`).
- **Data structures and Rust placement:** `hacivat-core::edit::undo { EditStep { ops: Vec<OverlayOp> }, UndoStack { undo: VecDeque<EditStep>, redo: Vec<EditStep>, bytes: usize } }`.
- **Complexity / performance budget:** undo/redo O(cells in the step); ≤ 16 ms for 10k-cell steps.
- **Edge cases (exhaustive list):** undo after a failed submit (allowed; the overlay is unchanged); undo of a revert (restores the reverted changes); undo across local sort/filter (keys); undo of an insert whose row the user is editing (editor cancelled); memory cap reached (oldest steps dropped with a one-time notice); text-editor undo inside an open cell editor (belongs to the editor until commit).
- **Test cases (input → expected):** paste 3×3 → one undo restores all 9 cells; edit, edit, undo, redo → back to the second edit; submit → undo shows the "not available" message; revert-all then undo → all changes back.
- **i18n notes:** menu labels `edit-undo = Undo { $action }`, `edit-redo`, action names (`action-paste`, `action-set-null`, `action-delete-rows`, …); `undo-not-available-after-submit`.
- **Verification status:** DataGrip/DBeaver behaviours [Observed: bundle; `ResultSetPreferences.java` max undo].
- **Prior art and evidence:** as merged.
- **Used by:** Hacivat.

### HV-EDIT-11: Document and key-value store edits (MongoDB, Redis)

- **Priority / depends on:** v1.x. Depends on HV-EDIT-02 (overlay), HV-EDIT-05 pattern, HV-RES-14.
- **Sources merged:** ALG-MONGO-02 step 7 (`$set` on dotted paths, `$setField` for keys with `.`/`$`, `$unset`, type dropdown, original-value predicate), Compass conditional update (behaviour), ALG-REDIS-02 step 9 (per-type commands, `WATCH`/`MULTI`/`EXEC`, `LSET` verified with `LINDEX`, `KEEPTTL`).
- **Problem:** Apply grid edits to documents and Redis values with the same safety guarantees (exactly-one, conflict detection) as SQL.
- **Inputs / outputs:** overlay entries on document paths or Redis members → engine commands with guards; outcomes mapped to HV-EDIT-06/05 semantics.
- **Decision:**
  - **MongoDB:** `updateOne({_id: <id>, <path>: <original> for each changed path (or {$exists: false} for Missing)}, {$set: {...}, $unset: {...}})`; keys containing `.` or `$` use pipeline updates with `$setField`; inserts `insertOne`; deletes `deleteOne({_id, <guard on loaded fields>})`; `matchedCount == 1` is the assertion (0 → conflict); multi-document submits use a transaction when the deployment supports it (replica set / sharded), else per-document with a warning "not atomic".
  - **Redis:** per-type commands (`HSET/HDEL`, `SADD/SREM`, `ZADD/ZREM`, `LSET` + `LINDEX` check, `XADD/XDEL`, `SET … KEEPTTL`) inside `WATCH key … MULTI … EXEC`; `EXEC` returning nil → conflict (the key changed); TTL preserved unless the user edits it.
- **Algorithm:** build commands from the overlay, preview them in the engine's shell syntax (mongosh / redis-cli), execute, map outcomes, reconcile by re-reading the document or member.
- **Data structures and Rust placement:** `hacivat-core::edit::mongo`, `hacivat-core::edit::redis`.
- **Complexity / performance budget:** one round trip per document/key (+ WATCH); reconcile one read.
- **Edge cases (exhaustive list):** Missing vs NULL (editing a Missing cell creates the field; "remove field" → `$unset`); type changes (Int32 → String via the type dropdown); Int64/Decimal128 exactness (typed BSON values, never JS numbers); `_id` edits (refused: delete + insert explicit action); arrays edited by index (`$set: {"a.3": v}` with the original element guard); concurrent array reordering (guard detects); encrypted fields (read-only); time-series collections and views (read-only); Redis keys expiring mid-edit (EXEC nil or WRONGTYPE → conflict); Redis cluster (WATCH only within one slot: single-key edits are fine); Redis lists edited by index while being pushed to (LINDEX guard); binary-safe Redis values (bytes, display types).
- **Test cases (input → expected):** edit Missing `a.c` to 5 on doc 2 → `updateOne({_id: 2, "a.c": {$exists: false}}, {$set: {"a.c": 5}})`; concurrent change of `a.b` then edit of `a.b` → `matchedCount = 0` → conflict; Redis hash field edit while another client changes the field → EXEC nil → conflict dialog.
- **i18n notes:** shared conflict strings; `mongo-not-atomic-warning`, `redis-ttl-preserved`.
- **Verification status:** Compass/RedisInsight behaviours [Observed: SSPL code read for behaviour]; Mongo transaction availability rules [Inferred: docs].
- **Prior art and evidence:** as merged.
- **Used by:** Hacivat Mongo/Redis views; MCP.

---

## 3. Conflicts resolved in this catalog

| Topic | Source A | Source B | Decision |
|---|---|---|---|
| Identity fallbacks | DataGrip: PK/unique else read-only | DBeaver: ROWID, virtual key, all columns | Strict default; fallbacks opt-in per table, persisted |
| `ctid` | DBeaver avoids it | datagrip/08 suggests `ctid`+`xmin` opt-in | Opt-in only with `xmin` equality |
| Statement order | DataGrip del→upd→ins [Inferred]; Beekeeper same | DBeaver del→ins→upd | Deletes → updates → inserts |
| Explicit NULL on INSERT | DBeaver omits NULL (DEFAULT wins) | DataGrip distinguishes Set DEFAULT | NULL inserted; DEFAULT only when chosen |
| Concurrency | DataGrip/DBeaver: none | Beekeeper/Compass: original values | On by default; version token → changed columns → key-only |
| Multi-row match | DBeaver ignores > 1 | DataGrip ALG-GRID-06: > 1 → rollback | Exactly one; unknown counts are failures |
| Auto-commit submit | DBeaver: partial application | — | Always one explicit transaction |
| After submit | DBeaver per-row refresh or full refresh | DataGrip SelectUsingKeys | RETURNING/OUTPUT or one batched re-select; store patches, no reload |
| Local sort and edits | DBeaver drops edits | DataGrip keeps | Keep (HV-GRID-07) |
| Roll Back in manual mode | DataGrip reverts pending edits | — | Roll back and offer to restore them as pending |

## 4. Open questions that need a product decision

- **PD-14 (concurrency default).** On for all connections (recommended) vs only production-tagged ones (DBeaver suggestion). Also whether DELETE guards should compare all loaded comparable columns (recommended) or only the version token.
- **PD-15 (physical row-id fallbacks).** Offer SQLite `rowid` and PG `ctid + xmin` as opt-in (recommended) or never.
- **PD-16 (join-result editing).** Single-source only in v1 (recommended); per-range-variable UPDATEs for join results in v1.x.
- **PD-17 (preview policy).** Mandatory on production-tagged connections and for > 100 deletes (recommended); otherwise optional.
- **PD-18 (ClickHouse editing).** Read-only in v1 (recommended) vs lightweight `DELETE`/`ALTER … UPDATE` mutations with an "asynchronous" warning.
