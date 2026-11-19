# ADR 0008 — Metadata and introspection

- Status: Accepted
- Date: 2026-11-19
- Scope: M4 (HV-MODEL-01..03; HV-INTRO-01..05, -07, -08, -11..16; HV-DDL-07 view-only producers,
  -09, -10, -11 core subset; R-M4-1 to -50)

## Context

M4 gives Hacivat a real catalog: an explorer tree that appears at once from a local cache,
refreshes incrementally, knows the built-in objects of every supported server version offline
(system-catalog snapshots built by CI, L7), refreshes after the user's DDL, refuses to wipe
itself silently, and shows byte-stable DDL. Five streams meet only through seams fixed before
the work starts:

- `hacivat-meta` (new, Core): the arena model with stable identity, Arc-swapped snapshots, the
  tri-state `ModelView`, the existence-diff merge, the per-profile cache, fingerprints, search,
  overlays and the system-catalog layer;
- `sql-catalog` (new, `crates/sql`): the SQL-facing `CatalogView` M5's completion calls, with
  `Lookup<T>` and its reasons (one definition);
- `hacivat-introspect` (new, Core): query packs, scope and filters, the scheduler and
  `CatalogHub`, the PostgreSQL / MySQL-MariaDB / SQLite introspectors, freshness, refresh after
  DDL and the `minicat` generator;
- `hacivat-ddl` (new, Core, pure on its default features): the alg/03 IR, renderers and view-only
  producers;
- the UI (`hacivat-ui` tree widget; the app's explorer, DDL view, scope editor), and the snapshot
  assets with their CI jobs.

`sql-ident` (M3) gains the identifier key and quoting modules every stream shares (R-M4-28).

## Rule

Each M4 task that makes a choice in this area appends one section headed
`## m4-tNN — <topic>` (append-only; a security choice made in a fix round goes into the same
task's section before re-review). A section states the choice, the alternatives it rejected,
and the tests or measurements that hold it. The rulings below are the plan's
(`docs/superpowers/plans/2026-11-19-m4-metadata-and-explorer.md`, "Assumptions and rulings");
each lands in the gate's PO list. Where this ADR and alg/02 or alg/03 disagree, this ADR wins
(R-M4-8 amends alg/02's PostgreSQL change predicate).

## Rulings R-M4-1 to R-M4-50

| # | Ruling | Why (source) | Cost if wrong |
|---|---|---|---|
| R-M4-1 | **Names and places follow the spec, not the catalog**: crates `hacivat-meta`, `hacivat-introspect`, `hacivat-ddl` (spec §5.1); snapshot assets in `assets/system-catalogs/<dbms>-<ver>.hvsnap.zst` (spec §5.1 `assets/`), not `hacivat-introspect/assets/system/` (alg/02 HV-INTRO-16); the generator is `xtask minicat` (spec §6 M4), not `gen-system-snapshots` (alg/02). No `hacivat-core` facade is created (spec §8-4 leaves it to the builder). | The spec wins over catalogs on layout (spec header "How to read it"). | A rename later; no behaviour change. |
| R-M4-2 | **No persistent-map crate.** `CatalogSnapshot.namespaces` is `Arc<BTreeMap<NsKey, Arc<NamespaceSnapshot>>>`, cloned on publish (O(namespaces) `Arc` clones). Task 9's `publish_5000_namespaces_under_1ms` is the trigger: if it fails on the CI runner, the controller rules a two-level map (database → schemas) first, a crate (`imbl`, MPL-2.0) only after that. | alg/02 HV-MODEL-02 "pick by benchmark", edge 10 ("fall back to a two-level map if > 1 ms"); ~10³ namespaces typical. | One refactor of `snapshot.rs` behind the same API. |
| R-M4-3 | **List binds expand to `IN (…)` lists on every dialect, chunked at 1,000 items**; no PostgreSQL array binds. | alg/02 HV-INTRO-04 step 2 proposes `= ANY($n)` with an array bind; `hacivat_values::BindValue` has no array type (`crates/core/hacivat-driver/src/typed.rs:57`) and M3's binds are scalar. | Longer SQL text for fragment and `check_state` queries; no correctness change. |
| R-M4-4 | **Engines in M4: PostgreSQL, MySQL, MariaDB, SQLite.** SQL Server, ClickHouse, Oracle, Redis and Mongo introspectors and producers are not written; their `ObjectKind`s, `ServerId` variants and kind-schema slots exist so v1.x needs no model change (spec §5.7). | Spec §4 "Engines" and DDL row (renderers for PG, MySQL/MariaDB, SQLite). | None for v1. |
| R-M4-5 | **"Completion resolves built-in functions and system views offline" (§6 M4 exit) is proven at the `sql_catalog::CatalogView` layer** that M5's completion calls (created by Task 1a, implemented for `SnapshotView` by Task 26), and at `ModelView`: a per-version offline resolution matrix (Task 30) and an offline explorer test (Task 32). The completion popup itself is M5. **PO-Q4.** | Completion is M5 scope (§6 M5); M4 owns the catalog it reads; R-M5-2. | If the PO wants the popup in M4, M5's P2 work moves forward (large). |
| R-M4-6 | **Snapshot selection is exact**: PostgreSQL by major (14–18), MySQL by major.minor (8.0, 8.4), MariaDB by major.minor (10.6, 10.11, 11.4). Any other version (e.g. PG 13, MySQL 9.x) introspects its system schemas live at `Names` level (**PO-Q7**). SQLite never uses a snapshot (`function_list` is cheap, alg/02 HV-INTRO-16 matrix). | Spec §4 introspection row: "Unsupported versions fall back to live introspection of system schemas"; alg/02's "closest snapshot version" is overruled by the spec. Supported set: spec C10, L7. | A PG 19 user gets live system introspection until a snapshot ships. |
| R-M4-7 | **Introspection is trusted text on read-only metadata sessions** (Global Constraints): pack text only, as a typed `MetaQuery`; names as binds; server-enforced read-only per engine — PG read-only transaction, **MySQL/MariaDB read-only session plus read-only transaction**, SQLite `mode=ro`. It never passes through the exec gate or the classifier. Joined connections on Hacivat-owned `Job(DdlApply\|DriftCheck)` transactions are the one exception (R-M4-29), enforced by `MetaQuery` because the server enforces nothing there. | alg/02 HV-INTRO-01 step 4, HV-INTRO-04 edge 6, spec §3.4; ver/01 `#mysql-ro` rows 13–14 (DDL executes inside `START TRANSACTION READ ONLY`; `SET SESSION TRANSACTION READ ONLY` blocks it). | — (safety rule). |
| R-M4-8 | **PostgreSQL watermark uses `pg_current_snapshot()` everywhere** (one code path on 13–18, PG spike §3); `txid_current()`/`pg_current_xact_id()` are never used; the end-of-pass canary is `pg_current_xact_id_if_assigned() IS NULL`. **The change predicate is exact:** `age(x.xmin) BETWEEN 0 AND age($W::xid)`, both ages from the transaction's stable reference XID, with no client-side `txage` and no margin; the client only checks the 64-bit span `snap_xmax − W ≥ 2^31 − 2^20` → `Full(wraparound)`. | alg/02 HV-INTRO-05 decision; ver/01 `#pg-canary`; PG spike finding 6 (the draft's `txage = (snap_xmax − W) + 1 + 64` misses rows when > 64 XIDs are assigned between the probe and the change query). alg/02 is amended by ADR 0008. | None known; the spike passed 49 DDL steps on 13–18. |
| R-M4-9 | **Hot standby and replicas: `Full` per namespace** (no incremental). PgBouncer statement pooling: `Full`. | alg/02 HV-INTRO-05 matrix (standby incremental is an open research item, alg/02 §6 research list). | Slower refresh on replicas. |
| R-M4-10 | **MariaDB: `Full` per namespace in v1**; no `CREATE_TIME` marker. Task 17 records MariaDB's `CREATE_TIME` behaviour for the ver/01 ALTER list as a non-gating artifact for v1.x. **PO-Q12.** | Spec §4 introspection row; alg/02 HV-INTRO-07 matrix (`[Unknown]` for MariaDB); ver/01 did not test MariaDB (§1 method). | Slower MariaDB refresh until v1.x. |
| R-M4-11 | **Session overlays for uncommitted DDL** (a console sees its own uncommitted `CREATE TABLE` in completion) **are M5.** M4 keeps pending refresh targets per session and exposes them (`RefreshCoordinator::pending(session)`), so M5 only adds the overlay. | alg/02 HV-INTRO-12 decision (overlay is the SQL-tooling layer), HV-MODEL-02 "session overlays are layered above … by the SQL-tooling resolver". | None. |
| R-M4-12 | **Virtual foreign keys: storage, catalog layer and display in M4; the editor dialog is M7** (with Modify Object). Shipped system-catalog virtual FKs (e.g. `pg_class.relnamespace → pg_namespace.oid`) are hand-authored from vendor documentation in `assets/system-catalogs/virtual-fks.toml`, reviewed row by row. **PO-Q8.** | alg/02 HV-INTRO-16 overlays; spec §4 Explorer "Virtual FKs stored locally (D-057)"; no M4 consumer edits them (diagrams v1.x, join completion M5). | A user who needs to add a virtual FK waits until M7. |
| R-M4-13 | **"Open data" in M4** (context action and double-click on a table or view) opens a new console bound to the same profile and schema with `SELECT * FROM <qualified name>` and runs it through the normal Run path (gate, classifier, cursor, 500-row first page). It is the default handler of `explorer::actions::set_open_data_handler`, which M6 replaces with the table editor (keyset paging, HV-RES-06). Double-click on a container toggles it. **PO-Q3.** | Spec §4 Explorer row (double-click opens data; keyset paging is HV-RES-06, scheduled in §6 M6). | If the PO prefers "nothing until M6", the action is hidden behind a flag (one line). |
| R-M4-14 | **Default scope for a new profile**: PostgreSQL `@:*` (every user schema of the connected database; other databases are listed, greyed, with "Add to scope"); MySQL/MariaDB `*` (every database the login can see is listed; contents load on first expand, `Brief` then auto level); SQLite every attached schema. System schemas are never introspected live by default: they come from snapshots (R-M4-6) and appear in a collapsed "Built-in" group. **PO-Q5.** | alg/02 HV-INTRO-02 (empty pattern = `@:@`; `*:*` on PG needs one connection per database and prompts, edge 10), HV-INTRO-01 step 3 (other namespaces load on first expansion). | A PG user with many databases sees only the connected one until they widen the scope. |
| R-M4-15 | **Cache encryption**: a profile flag `catalog_sensitive` ("Encrypt cached metadata"); when on, rows are XChaCha20-Poly1305-encrypted (`RowCipher`, domain `hv-catalog-v1`) with a per-profile 32-byte data key kept in the secret store (slot `SecretSlot::CatalogKey`, retention Forever); when the keyring is unavailable the profile's cache is memory-only for that session with a notice (alg/02 HV-MODEL-03 edge 9). **Default on for production-tagged profiles, off otherwise** (**PO-Q2**). **Key timing:** at app start the key is fetched (blocking pool) only for encrypted profiles expanded in the restored workspace; every other encrypted profile shows its root row collapsed and makes no keyring call until the user expands or connects it (**PO-Q19**). | D-016 (plaintext 0600 unless sensitive or managed); alg/02 HV-MODEL-03 encryption decision; "production" is the product's sensitivity signal everywhere else (D-008, D-036); review I10 (the keychain may block or prompt on macOS). | Extra keyring item per production profile; a Linux user without a secret service gets memory-only caches for production profiles. |
| R-M4-16 | **Golden files are plain `.sql`** under `crates/core/hacivat-ddl/tests/golden/<dialect>/`, updated only with `UPDATE_GOLDEN=1` (the existing convention, `crates/core/hacivat-driver/tests/error_fluent_golden.rs:3`); no `insta`. CI fails if a golden file changes without the flag. | Consistency with the codebase; one fewer dependency. | None. |
| R-M4-17 | **Snapshot determinism**: payload = canonical JSON format 1 (the minicat spike's: byte-order sorted, no server ids, OIDs, `StateMark`s, watermarks, timestamps, hostnames or paths, names only for variables; PG objects with OID < 16384 read from database `postgres` with `search_path = pg_catalog`; GUC names containing a dot excluded), compressed with zstd level 19 single-threaded, no dictionary, checksum on, exact crate pin; `manifest.toml` per file: dbms, line, exact server version, image digest from `docker/m2a/compose.yml`, format number, JSON and zst byte counts, XXH3-64 digests, object counts, and **`excluded = [...]`** (every named exclusion rule, so `--check` shows what is deliberately absent). Volatile content found by a double run is excluded by a named rule, recorded in ADR 0008 and in `excluded`, never by loosening a comparison. amd64 CI is authoritative for the bytes. | Spec §6 M4 exit "snapshots regenerate reproducibly in CI"; minicat spike §1, §4; fresh-container stability of `performance_schema`, `sys` and PG `information_schema` is **[Unknown]** until the first `minicat-verify` run, which controller step C-1 dispatches before W1 so its answer becomes a T23 carry. | A kind excluded from snapshots is live-introspected or `Unknown` offline. |
| R-M4-18 | **The explorer list becomes the tree in place**: the M2a profile rows (groups by environment, filter field, row content, keyboard) become the tree's roots, gaining a chevron. Every approved shot that shows the explorer changes; the changed PNGs are listed one by one (PO item) and `baseline-m4` replaces them after sign-off. **PO-Q11.** | Spec §4 Explorer row; DESIGN.md Explorer Tree (chevron + icon + label); `crates/app/hacivat-app/src/connections/explorer.rs:1-20` (M2a T18 list). | One large re-baseline round at the gate. |
| R-M4-19 | **Explorer extension points for later milestones** (L14 and the reconciliation): `ExplorerContributor` may insert nodes directly under a database node, above its schemas, with lazy children, activation, a context menu, filter text and a generation; `ExplorerActionProvider` adds actions to the explorer menu and the DDL view toolbar by slot; the open-data handler is replaceable; `reveal` and `database_rows` serve M5 and M4b; `TreeKey` is profile-scoped and domain-separated. M4 registers no contributor and no provider. | Spec §6 M4b depends on M4; L14 "Scripts node directly under the database node and above the schemas"; M4b C-M4-1..8, M5 M4-9, M6 open data/export/import, M7 Modify/Edit source. | If M4b needs another anchor, a small seam change in M4b's Task 1. |
| R-M4-20 | **Defaults for background work**: periodic validation every 10 min for profiles tagged `local` or `dev`, off for every other tag and for untagged remote hosts; refresh after DDL **Always** (no prompt) for scripts the user runs, with Ask/Never in settings. **PO-Q6.** | alg/02 HV-INTRO-01 step 6 ("default off for remote/managed, 10 min for local dev"), HV-INTRO-12 (ASK/ALWAYS/NEVER, default not given — **[Unknown]**: no source states DataGrip's default). | A remote production catalog may stay stale until the user refreshes or connects. |
| R-M4-21 | **Depth thresholds** are D-038's 2,000 / 20,000 / 5,000 major objects (current namespace Sources / Structure; other namespaces Structure), constants in `LevelPolicy`, overridable per database/schema (pills). Task 29 records cold-introspection times at those thresholds on the PO's Mac for the D-038 "confirmed by benchmark" clause. | D-038 (decided, A); alg/02 HV-INTRO-01. | Re-tuned constants. |
| R-M4-22 | **Mass-deletion guard**: > 50 % of a namespace with ≥ 20 objects disappearing in one pass; user-triggered refresh → confirmation dialog; background pass → nothing removed, namespace marked stale with a badge, the result kept as `pending_confirmation` until the user refreshes. An explicit namespace drop (`ScopeRefresh` no longer lists it) is not guarded but shows a notice. | D-039 (decided, A); alg/02 HV-INTRO-11 decision and edge 2. | — |
| R-M4-23 | **Constraint placement default `InsideTable`**, qualification default `Auto` for display, generated text never reformatted in M4 (the formatter is M5). **PO-Q9** (low). | alg/03 HV-DDL-09 decision and §7-5; HV-DDL-11 defaults; spec §6 M5 (formatter). | Golden files change if the PO prefers `InsideColumn`. |
| R-M4-24 | **DDL view shows synthesized DDL for tables, indexes and sequences, and server text for views, materialized views, functions, procedures and triggers**; a "Definitions provided by server" toggle shows server text for tables too where the engine has it (MySQL `SHOW CREATE TABLE` through R-M4-37, SQLite `sqlite_schema.sql`; PostgreSQL has no table definition function, so the toggle is hidden there). MySQL server text for views, routines and triggers comes from `SHOW CREATE` (R-M4-37). MySQL executable comments in server text are shown verbatim and flagged (`ddl-warn-executable-comment`). SQLite CHECK constraints and generated-column expressions exist only in `sqlite_schema.sql` (no pragma reports them, F10): SQLite tables show server text by default. **PO-Q21.** | alg/03 HV-DDL-07; ver/01 `#mysql-comments`; MySQL/SQLite spike F10. | — |
| R-M4-25 | **Cache caps** 512 MB per profile and 4 GB global, LRU by last load, never evicting the current database's namespaces. | alg/02 HV-MODEL-03 step 7. | Settings change only. |
| R-M4-26 | **The model spike's choices are the model core**: hand-written versioned codec (`"HVNS" major minor compression checksum body`, every record and union value length-prefixed), `FORMAT_MAJOR`/`FORMAT_MINOR: u16`, `Name = Arc<str>` interned per namespace, XXH3-128 for `SourceRef`, the SQLite schema digest and fingerprints, XXH3-64 for row checksums; cache rows **uncompressed** in v1 (the compression byte is reserved); `ObjectKey` structural `Eq`/`Hash` plus `same_object()`; `Ref { kind, path, server_id }` resolved on demand; `ObjFlags(u32)` with `bitflags`; `ObjectKind` is `Copy` (67 kinds with stable `u16` cache codes), `Unknown` is a unit variant and the raw server kind lives in `Props::Generic`; `KindGroup { Relations, Routines, Procedures, Types, Triggers, Other }` plus `group() -> Option<KindGroup>` (`None` = parent-scoped; PG composite types in `Relations`); `NsKey { kind: NsKind { Server, Database, Schema }, path }`; module names `kind`, `name`, `ids`, `props`, `namespace`, `snapshot`, `codec`, `persist`, `fixture`. No `postcard`, `blake3`, `compact_str`. | `spike-model-report.md` (postcard is not self-describing, so HV-MODEL-03 "older minors read newer rows" needs per-record envelopes anyway; 36 ms load of 160k columns; server-id-first equality is not transitive). | A codec swap later costs a `FORMAT_MAJOR` bump (cache rebuilt, no data loss: the cache is not user data). |
| R-M4-27 | **M4 creates `crates/sql/sql-catalog`** with the SQL-facing `CatalogView` trait, `Lookup<T>`, `MissingReason`, `UnknownReason` and the payload types M5 lists (`plan-m5.md` Task 1a); `hacivat-meta` implements it (`SnapshotView`, Task 26) and re-exports `Lookup`. M4's own generic trait is renamed **`ModelView`**. | R-M5-2 "preferred" path; alg/10 §0.4; `SqlReachesCore` forbids the trait in a core crate; M4's exit (R-M4-5) must be proven at the layer completion calls. | M4 grows by one seam and one implementation file; M5 drops its adapter task. |
| R-M4-28 | **Identifier keys and quoting live in `sql-ident`** (M3's crate, named exceptions): `sql_ident::key::{IdentKey, IdentCtx, NameClass, FoldRule, match_key, ServerDependent}` (Task 3) and `sql_ident::quote::{QuoteCtx, needs_quoting, quote, quote_qualified, string_literal}` (Task 7). `hacivat-meta` and `hacivat-ddl` re-export or call them; M5 Task 6 extends them; M6 Task 6 consumes them. | One fold table and one quoting function for the model, the resolver, DDL, M6's `valfmt::literal` and M8's tools (alg/03 HV-DDL-07 "identifier quoting only through `needs_quoting`/`quote`"). | A module move. |
| R-M4-29 | **R-M4-7 widened by exactly one case**: joined metadata reads inside a Hacivat-owned `Job(DdlApply\|DriftCheck)` transaction (Task 5's `MetaConn::join`). | M7 R-M7-9/-25 drift check inside the apply transaction (`alg/03:614`). | — (safety rule; tested). |
| R-M4-30 | **Grants, dependencies and statistics are facets** (alg/02 HV-MODEL-01 "Facets … side tables with their own `LoadState`"), loaded on demand (`load_facet`) or with the table rows (`Statistics` at `Structure`); a facet the login cannot read completely is `Partial` and says so. | M7 (grants, dependencies), M5/M6/M8 (row estimates; HV-RES-10 part routed to M4 by the M3a Deferred table, `2026-11-14-m3a-run-a-query.md:293`); T19 already renders grants. | Extra catalog queries only when a facet is asked for. Whether the explorer shows row estimates is **PO-Q14**. |
| R-M4-31 | **`CatalogHub` is a core service** (`hacivat-introspect::hub`): one engine, catalog, cache and key per profile, reachable without the UI. The app's `CatalogHost` is a thin wrapper. | M8's broker (A2), M7's apply and M6 need catalogs outside the explorer. | One more core type; T24 shrinks. |
| R-M4-32 | **Query packs are multi-file per dialect** (`packs/<dialect>/NN-<topic>.hvq.sql`, merged in name order; a duplicate section is a parse error). Later milestones add files and never edit M4's. | M7 drift sections, M8's optional fingerprint section. | — |
| R-M4-33 | **System-catalog assets use the minicat spike's format and tooling** (canonical JSON format 1 + zstd 19, XXH3-64 manifest digests, the raw-file boundary between the Docker read and the xtask `collect`/`build`/`check`/`verify`). The Docker read moves into `hacivat-introspect/tests/minicat_gen.rs` and runs **the introspectors' own pack sections in system mode** (R-M4-44), through the Docker wrapper's lock. **No SQLite snapshot** (R-M4-6). Decoding lives in `hacivat_meta::system::decode` (`hacivat-meta` may not call `hacivat-introspect`). MySQL/MariaDB built-in **signatures** are M5's curated data; snapshots carry their names (`FnSig.args = None`). | `spike-minicat-report.md` §1–§7; `MetaBelowIntrospect`; review I4, I5. | — |
| R-M4-34 | **Explorer extension points are M4's**: `ExplorerContributor` (extended), `ExplorerActionProvider`, `set_open_data_handler`, `reveal`, `database_rows`; `TreeKey` is profile-scoped and domain-separated. | M4b C-M4-1..8, M5 M4-9, M6 open data/export/import, M7 Modify/Edit source. | — |
| R-M4-35 | **`DdlIsPure` holds for `hacivat-ddl`'s default features**; the non-default feature `apply` carries M7's session-bound code. | M7 places `apply/`, `store/`, `drift/` in `hacivat-ddl` (`plan-m7.md` T1a ownership); Karagöz links the pure part. | M7 might prefer a separate crate; the rule then names that crate instead. |
| R-M4-36 | **Names**: `generation` (not `epoch`), `DialectId`, `sql_ident::QualifiedName`, `ObjectKey`, `Props`, `fixture`, `PackCtx`; M4 keeps `CatalogDelta` (M5's statement-effect delta becomes `DdlDelta`); ADR numbers by milestone order: M4 **0008**, M4b 0009, M5 0010, M7 0011, M8 0012. | `reconciliation.md` §2 (N-1..N-17). | Renames only. |
| R-M4-37 | **MySQL `SHOW CREATE` text uses a quoted-identifier macro.** `SHOW CREATE TABLE\|VIEW\|PROCEDURE\|FUNCTION\|TRIGGER\|EVENT` takes no bind parameters, so a pack macro `#{ident:<bind-name>}` may appear **only** in sections marked `--- show-create ---` (Task 4 lint). It renders exclusively through `sql_ident::quote::quote(DialectId::MySql, …)` (backticks doubled), refuses a name containing NUL, and is security-reviewed in Tasks 4 and 17. Hostile names tested: `` a`b ``, `İ`, a name with a trailing space. Every `SHOW CREATE` form is [Inferred] to run on the typed path; `every_mysql_section_prepares_and_returns_its_declared_columns` pins it, and a form the server refuses there is reported NEEDS_CONTEXT, never spliced another way. **PO-Q21.** | Review I9; R-M4-24 needs MySQL server text; `INFORMATION_SCHEMA` text columns hold bodies, not the full `CREATE` statement (definer, characteristics). | If the security review rejects the macro: synthesize MySQL table DDL, show only the `INFORMATION_SCHEMA` bodies for views/routines/triggers, and hide the toggle on MySQL (R-M4-24 changes; PO-Q21 alternative). |
| R-M4-38 | **The SQLite metadata connection is a separate `mode=ro` connection without the user-facing read-only authorizer.** It opens the main file by URI `mode=ro` (attached files by `mode=ro` URIs, in attach order), runs only `MetaQuery` text, and reads inside a deferred `BEGIN` for a consistent pass. `hacivat-guard`'s `SQLITE_READ_PRAGMAS` stays unchanged. The `temp` schema is read on the user's own connection (idle, outside a transaction) with only the pragmas that authorizer allows (`table_xinfo`, `index_list`, `index_xinfo`, `foreign_key_list`, `database_list`, `function_list`, `collation_list`, `schema_version`), using the text scan for `WITHOUT ROWID`/`STRICT`. `PRAGMA data_version` is not used for change detection. | MySQL/SQLite spike F7 (the authorizer blocks `pragma_table_list`, `pragma_module_list`, `PRAGMA data_version` and `BEGIN`), F8 (`data_version` reports other connections' commits, not schema changes); the authorizer defends against user text, which never reaches this connection; `mode=ro` already refuses writes in the engine. | If the security review wants the authorizer on the metadata connection: add `table_list`, `module_list` to the allowlist in M3's guard crate (a security change) and replace `BEGIN` by the spike's `schema_version` before/after retry loop. |
| R-M4-39 | **MySQL incremental marker rule:** a table, trigger, routine or event is changed when **its marker differs from the stored marker, or is `≥ W − 1 s`**; every pass fetches `(name, kind, marker)` for every object of the kind (the existence query reads those rows anyway; 140 ms at 10k tables), with no `CREATE_TIME >= ?` filter in SQL. Table markers are `CREATE_TIME \| max(TRIGGERS.CREATED) \| trigger count` (the count catches a dropped trigger). `W = UTC_TIMESTAMP(6)` is read **inside** the consistent-snapshot transaction. | MySQL/SQLite spike F1 (MySQL stamps a long `ALTER` with its start time; the window alone missed it on mysql80/84), F2 (MySQL `INFORMATION_SCHEMA` is frozen inside the snapshot; later DDL is caught by the next pass through marker equality), spike difference 6. | A second long DDL starting in the same second as an already-stored stamp is still invisible to markers (spike F1, last paragraph); background validation and user refresh remain. |
| R-M4-40 | **Spike code is adopted by copy into the owning task's files**, never by rebase or cherry-pick (Global Constraints "Spike code"). Task 1a copies the type declarations of the model spike as the seam with stub bodies; Tasks 3, 9, 11 (model), 8 (tree), 16 (PG), 17, 18 (MySQL/SQLite) and 23 (minicat) copy their own bodies; Task 1b copies the tree spike's public API with stub bodies. | Review C3 offered two options. Cherry-picking `d86313b` into Task 1a (the review's recommendation) would land real `persist`/`snapshot` bodies that T1a must then disable to keep the fail-closed stub list true, and T3/T9/T11 would re-enable them: double work and a stub list that is false in between. Copy-per-task keeps the stub list true and gives each reviewer exactly the code their task owns. | A reviewer re-reviews code that was already exercised by the spike's tests (a cost the review asked for anyway). |
| R-M4-41 | **MySQL/MariaDB non-ASCII names are `ServerDependent` until the server rule is pinned.** Stored keys are ASCII-caseless (the spike's `FoldRule::AsciiCaseless`). A lookup whose probe text contains a non-ASCII **cased** letter answers `Found` only on an exact byte match of a stored name and otherwise `Unknown(ServerDependent)` — never `Missing` — for column names and for table names under `lower_case_table_names ≥ 1`. Task 17's probe pins the column rule on lctn = 0; Task 17 then adopts it in `sql-ident/src/key.rs` (fenced `// ## m4-t17-mysql-fold`), and ADR 0008 records an INV-3 exception if the server folds beyond ASCII. lctn ≥ 1 table names stay `ServerDependent` (no lctn ≥ 1 server in the matrix, reconciliation O-4). | Review I6 (MySQL documents column names as case-insensitive; an ASCII-only key would answer a wrong `Missing` for `äb` vs `Äb`, a F31 violation); reconciliation A-21. | Some completion lookups on non-ASCII MySQL names say "unknown" instead of "found" until the probe lands (W3). |
| R-M4-42 | **SQLite refresh-after-DDL timing follows the read-only connection, not alg/02's in-session rule:** DDL in autocommit refreshes immediately; DDL inside an explicit transaction is pending until `COMMIT` and discarded on `ROLLBACK`/`ROLLBACK TO` (uncommitted DDL is invisible to the `mode=ro` connection). No compensating refresh. | Review I8; R-M4-38; alg/02 HV-INTRO-12 matrix assumes the read runs on the user's connection, which Task 5 forbids for E8. | — |
| R-M4-43 | **Docker fixture changes for M4** (Task 1c files; effective only after controller step C-2 recreates the MySQL/MariaDB services and their volumes, **needs PO OK**, PO-Q18): `hv_test` gains `ALL` on databases matching `hvm4\_%`; the `hv_catalog` and `hv_fixture` accounts (Global Constraints "Docker"); `--log-bin-trust-function-creators=1` on `mysql80`, `mysql84` (and `mysql84-x509` if it shares the image line); the `pg17wrap` service (built and started by C-6, needs PO OK). PG volumes are never recreated for M4. **Fallback if the PO declines C-2:** tests use prefixed objects (`hvm4_…`) inside `hv_test` with a static mutex (the MySQL/SQLite spike's way), the Turkish-collation test uses a table-level collation, MySQL trigger and function tests run on MariaDB only (gap listed in ADR 0008), `SET GLOBAL` tests are skipped with a reported reason, and the committed MySQL/MariaDB snapshots come from the C-1 CI artifact. | MySQL/SQLite spike F4 (`hv_test` cannot `CREATE DATABASE`), F5 (error 1419 on `CREATE FUNCTION`/`CREATE TRIGGER` with binary logging on), §"Deliberate differences" 10 (`SET GLOBAL` needs SUPER/`SYSTEM_VARIABLES_ADMIN`); minicat spike §8 Q1 (`hv_catalog` exists only on fresh volumes); review C2. | Without C-2, weaker MySQL trigger/function coverage and a dependency on the CI artifact for E2. |
| R-M4-44 | **System-mode pack sections and the one generator-only probe.** Relations, columns, routines, types, operators, PG type categories and GUC/variable names with type and context come from Task 16/17 pack sections run in `system` mode (PG: `pg_catalog`/`information_schema`, OID < 16384, `search_path = pg_catalog`; MySQL/MariaDB: `information_schema`, `mysql`, `performance_schema`, `sys`), named and owned by those tasks (`packs/<dialect>/40-system.hvq.sql`). MySQL/MariaDB built-in function **names** come from a generator-only helper in `hacivat-introspect/tests/minicat_gen.rs` that `PREPARE`-probes the names in `sql-safety/data/mysql/manual-index.txt` (plus MariaDB `information_schema.SQL_FUNCTIONS`) from a database the reader has a grant on, with a sentinel name required absent. That helper is the **single exception to R-M4-7**: it lives only under `tests/`, never in the runtime crate, never runs on a `MetaConn`, and is recorded in ADR 0008. | Review I4 (the draft's packs had no operator, type-category or GUC-context section; building `PREPARE` text from a data file breaks "names only as binds"); minicat spike §2 (sentinel caught a 1044-vs-1305 false positive). | — |
| R-M4-45 | **CI evidence on `m4` comes from `workflow_dispatch`**, not "nightly": `minicat-verify` twice on the `m4` tip (each from a fresh CI stack) plus one PR-triggered run; `fuzz-meta`, `docker-full`, `bench` and the package job dispatched on the gate tip; run ids recorded in `m4.md`. | Review I11: `ci.yml` (lines 9–15) triggers `schedule` only on the default branch; M3 got "nightly" evidence the same way (ledger lines 89, 93). | — |
| R-M4-46 | **PostgreSQL passes pin `search_path`:** every standalone metadata snapshot runs `SET LOCAL search_path = pg_catalog, pg_temp` right after `BEGIN`, so `format_type` and `pg_get_*def` qualify names the same way in every session (user types then appear schema-qualified in raw type text; the model also stores `TypeRef { schema, name }`). Every catalog is written `pg_catalog.<name>` in the packs, so a hostile path cannot shadow one. A joined connection cannot pin the path (no `SET` there, R-M4-7): `joinable` sections select type schema and name columns, never `format_type` or `pg_get_*def` text. | PG spike §6 (commit `5a5ab08`): without the pin, snapshots differ between sessions; review M3. | — |
| R-M4-47 | **PostgreSQL 13 is exercised but not supported.** The PG introspector's Docker tests run on `pg13`–`pg18` in the `full` set (PG 13 needed no special case, PG spike §3) and gate on 14–18; PG 13 connects with the "not tested" notice of C10 and gets no snapshot (R-M4-6). Version gates the packs carry: `indnullsnotdistinct` (15+), domain `NOT NULL` in `pg_constraint` (`contype = 'n'`, 17+), column `NOT NULL` constraints and `conenforced` (18+), `attgenerated = 'v'` (18+). | Spec C10 (PG 13 reached EOL in Nov 2025); PG spike §3. | One more full-set target (seconds). |
| R-M4-48 | **PostgreSQL change detection reaches one dependency hop and the same relation.** A young `pg_class`, `pg_attribute`, `pg_type` or `pg_proc` row **anywhere in the database** marks the in-scope objects that depend on it through one `pg_depend` hop (`deptype NOT IN ('e','x','p')`) for a re-read (section `ChangedDependents`); a `columns` change of a relation also re-reads the relation row, and a relation-row change re-reads its indexes, constraints and triggers (section cascade). Removed comments are found by an existence query over comment keys (`ListExistentComments`). A young `pg_namespace` row of a renamed schema that in-scope objects reference triggers a `Full` pass of every in-scope namespace (two hops; rare; PG spike §7 Q3). An incremental pass whose changed parents exceed 25 % of the namespace's relations runs `Full` instead (PG spike §7 Q5; the threshold is recorded with Task 16's timings, not measured yet). A pass that fails with SQLSTATE `XX000` ("cache lookup failed", a concurrent drop) is retried once (finding 8). | PG spike findings 3, 4, 5, 8; §7 Q3, Q5. | Extra re-reads on renames (cheap: a few rows per changed object); a schema rename costs a full pass. |
| R-M4-49 | **Tree group headers take keyboard focus but never the selection** (ARIA practice): arrows stop on a kind-group header ("Tables"), → / ← / Enter expand or collapse it, actions and `SelectionChanged` ignore it, type-ahead skips it. | Tree spike concern 1: skipping headers (the draft's rule) left keyboard users unable to collapse a kind group; DESIGN.md Explorer Tree keeps them unselectable. | One more stop per group when arrowing (the alternative, host commands only, was rejected as less discoverable). |
| R-M4-50 | **Tree type-ahead and selection defaults:** the widget's type-ahead fold defaults to `Locale { turkic: false }`; the explorer sets `Fold::Tolerant` (i/ı/İ/I one letter) when the UI language or region is Turkish and ties it to the filter's Turkish-tolerant toggle (D-074: on for Turkish users, toggleable). M4's explorer uses **single selection**; multi-select (and whether hidden selected rows under a collapsed ancestor count) is decided by the first milestone that adds a bulk action. Type-ahead for IME scripts needs an input handler and is not in v1. | Tree spike concerns 2, 3, 6; D-074 (decided). The spike's default (Tolerant everywhere) is overruled by D-074. | A non-Turkish user with Turkish object names gets exact type-ahead until they enable the toggle. |

## m4-t01a — manifests, core types and layering rules

**Manifests (one table, exhaustive; Task 1a's first commit).** The only new third-party crate
is `zstd =0.13.3` (no default features; `zstd` MIT, `zstd-safe` BSD-3-Clause, `zstd-sys`
BSD-3-Clause AND BSD-2-Clause, all accepted by `deny.toml` unchanged), with `zstd-sys` built at
`opt-level = 2` in debug. The workspace entry of `hacivat-meta` has `default-features = false`,
so `hacivat-ddl` takes the model pure and every other user names the features it needs
(`hacivat-introspect`: `catalog`, `persist`, `system`; `hacivat-app`: those plus
`embedded-system`). `sql-ident` holds `icu_casemap` for Task 17 only (R-M4-41).

**Seams, fail closed.** Every type of the plan's Task 1a interface list is declared; every body
that would state a fact about a catalog is a stub that cannot be read as one:

| Seam | Stub answer until | Owner |
|---|---|---|
| `ModelView` on `SnapshotView` (`lookup`, `children`, `find_by_server_id`), `CatalogSnapshot::{resolve, fk_referencing}`, `sql_catalog::CatalogView` on `EmptyCatalog` and `SnapshotView` | `Unknown(NotLoaded)`; `virtual_fks` empty, `row_estimate` `None`; `freshness` `Unknown` | T9, T26 |
| `sql_ident::key::match_key` | `Err(ServerDependent)` (no key, so every lookup is `Unknown`) | T3 |
| `sql_ident::quote::{needs_quoting, quote}` | always quote, the dialect's quote character doubled; NUL refused; `string_literal` `Err(NotImplemented)` | T7 |
| `ObjectKind::{group, allows_child, exists_in}` | `None` / `false` / `false` | T3 |
| `NamespaceSnapshot::build`, `replace_subtree`, `upsert_child` | `Err(ModelError::NotImplemented)` (no arena exists) | T3 |
| `ObjectKey::same_object` | structural equality only (a rename is not recognised; never two objects as one) | T3 |
| `name::has_invisible_chars` | flags C0/C1 controls, Unicode `Default_Ignorable_Code_Point` (16.0) and a named filler list (U+2800, U+FFF9–U+FFFB, Arabic/Syriac/Kaithi format characters); the list is documented in `name.rs` and claims no more (fix round 1) | T3 |
| `Publisher::publish`, `merge` | `Err(NotImplemented)`; nothing is published | T9, T10 |
| `CacheStore::open` | `Err(CacheError::Disabled)` (memory-only; nothing is created) | T11 |
| `codec::{encode_namespace, decode_namespace}`, `payload_version` | `Err(NotImplemented)`, `None` | T11 |
| `persist::crypt::RowCipher::{seal, open}` | `seal` returns an empty vector (never the plaintext); `open` `Err(NotImplemented)` | T11 |
| `fingerprint::subtree` | `None` (never a constant two objects could share) | T11 |
| `SystemCatalogs::for_server`, `system::{decode, normalize}`, `SystemCatalogs::open` | `None`, `Err(NotImplemented)` | T23, T26 |
| `search::search`, `SearchPath::for_*` | no hits, empty path (not wired to any user action) | T9 |
| every `hacivat-ddl` renderer; `produce::create_script` | `RenderError::Unsupported`; `DdlError::NotImplemented` | T7, T19 |
| `xtask minicat` | exit 2, except `check`, which passes only an empty snapshot set | T23 |
| `hacivat-app` example `catalog_cold` | prints `RESULT: n/a`, exit 2 | T29 |

Choices beside the plan's text, each small and reversible by its owner:

- **`VirtualTable` takes cache code 14.** The model spike (`d86313b`) declares 66 kinds; the
  plan's list adds `VirtualTable` (SQLite virtual tables), which takes the free relation code
  next to `ForeignTable` (13). Every spike code is unchanged; `seams.rs` lists all 67.
- **Results where the plan wrote bare values, so a stub cannot answer a fact:**
  `encode_namespace -> Result<Vec<u8>, DecodeError>`, `fingerprint::subtree -> Option<Fingerprint>`,
  `CatalogSnapshot::resolve -> Lookup<ObjHandle>`.
- **`Flavor = DbmsKind`** (the precise engine: MySQL vs MariaDB, PostgreSQL vs its wire forks);
  `DdlTarget::new` derives it from the dialect.
- **`Qualify` and `Placement` live in `hacivat_meta::userdata`** (the DDL view remembers them per
  profile) and `hacivat_ddl::options` re-exports them: one definition.
- **`ObjectKey`/`NsKey` serialize through plain-`String` mirrors** (serde's `rc` feature is off in
  the workspace); a kind id this build does not know reads back as `Unknown`. `ProfileUserData`
  carries a `version` and writes `ObjectKey`-keyed maps as lists of pairs (TOML and JSON maps take
  string keys only).
- **`TypeProps` is a struct** (`owner`, `comment`, `body: TypeBody::{Enum, Domain, Composite,
  Range, Base}`), so the owner the plan asks for on types has a place.
- **`IntroErrorCode`** (a closed, locale-free code set) lives in `hacivat-meta`: `LoadState`
  needs it and `hacivat-meta` may not see `hacivat-introspect`'s error type.
- **`RowKey`** has a redacted `Debug`, no `PartialEq`, and is `Zeroize` + `ZeroizeOnDrop`
  through `zeroize` (fix round 1, m4, controller ruling: `zeroize` 1.9.1, already in the lock,
  joins `hacivat-meta`'s `persist` feature; no new crate in the lock).

**Layering (`cargo xtask check-deps`).**

- `DdlIsPure` walks `hacivat-ddl`'s graph **feature-resolved from the manifests**, not the
  workspace resolve: the unified resolve keeps `hacivat-meta -> tokio` because
  `hacivat-introspect` turns `catalog` on, so a resolve walk would refuse the pure crate. The walk
  follows normal edges, activates optional dependencies through `dep:x`, `x` and `x/f`, and adds
  `default` unless the edge turns it off. Fix round 1 (I2): it starts from `default` **and every
  feature but `apply`** (a feature that only names `apply` does not turn it on), so a second
  feature cannot carry a runtime. It checks every activated dependency **by name**, so an
  optional edge that no member turns on (no edge in the cargo resolve) is still seen. Refused
  by rule (m6): any `tokio*` or `futures`/`futures-*` crate, `async-std`, `smol`,
  `async-executor`, `async-global-executor`, `async-io`, `mio`, `hacivat-session`,
  `hacivat-driver*`, `hacivat-introspect`, `hacivat-l10n`.
- `DdlApplyOnlyFromApp` and `EmbeddedSystemOnlyFromApp` (fix round 1, I1) run the same
  feature walk from each member with **all its features**, dev and build edges included, and
  not through `hacivat-app`; they refuse the member when the owner (`hacivat-ddl`,
  `hacivat-meta`) ends with the feature on. That covers a renamed dependency (`package = …`),
  `dep:`/weak forms, the member's own feature chains through `default`, and an alias feature the
  owner declares (`snapshots = ["embedded-system"]`), including through the owner's `default`.
  The owner itself and `hacivat-app` are exempt; the `hacivat` binary reaches them only through
  the app.
- `MetaBelowIntrospect` (fix round 1, I2) refuses any **declared** dependency of
  `hacivat-meta` (any kind, optional or not) that is forbidden, and the all-features graph
  (every edge kind out of `hacivat-meta`, normal edges beyond it).
- `IntrospectNotExec` refuses `hacivat-exec` at any depth and `hacivat-results` as a direct edge
  of any kind. `hacivat-results` is not refused transitively: `hacivat-session` (which
  introspection needs) keeps its result store on it, and that edge is not introspection's.
- `MetaQueryOnlyFromPacks` and `HoldCountsTestOnly` (the M3c carry, P-M4-17) are source rules
  over every `crates/<layer>/<crate>/`, `fuzz/` and `xtask/` (`target/` skipped). Fix round 1
  (I3): they read **tokens** — comments, whitespace, char literals, lifetimes and string contents
  dropped — so spacing, line splits and comments do not hide a call, and strings never match.
  `MetaQueryOnlyFromPacks` refuses `MetaQuery::new`, `<…MetaQuery>::new`, `MetaQuery as X`,
  `type X = …MetaQuery…;` and `impl MetaQuery` outside `hacivat-introspect/src/pack/**`,
  `hacivat-session/src/meta.rs` and a crate's `tests/`, counting the code a file pulls in with
  `#[path]` or a literal `include!` (followed four levels deep); it also refuses a `#[path]` in a
  `src/` file that leaves the crate's `src/`, and an `include!` of a file that is not `.rs`.
  `HoldCountsTestOnly` refuses any `hold_counts` identifier (call, alias, function pointer) in a
  `src/` file but its definition. `include!(concat!(env!("OUT_DIR"), …))` (generated code) is not
  followed. Carry to T1c (reviewer): `MetaQuery`'s fields private, no `Default`/`From`, `new` the
  only constructor, so the token rules cover every way to build one.
- `PRODUCT_CRATES` gains the four M4 crates; `sql-catalog` falls under the SQL rules by
  directory.

**Localization.** `model-*` (kind labels, cache notices) and `ddl-*` (generator options and
notices) are new ID prefixes of `hacivat-core.ftl` (named exceptions in `hacivat-l10n-schema`
and xtask's `l10n` rules, as M3b's `safety-*`). `model-kind-*` is a dynamic family (labels are
addressed by `ObjectKind::id()`). Because `model` is now an ID module, xtask's code scan no longer
reads a `model::CONST` path that follows `::` (`hacivat_ui_grid::model::MAX_COPY_CELLS`) as a
message ID.

**Fix round 1 (review `task-m4-01a-review.md`).** I1–I3 and m1, m3, m4, m6, m8 above and in
`check_deps.rs`, `name.rs`, `persist/mod.rs`, `minicat.rs` and the FTL block
(`model-cache-rebuilt` gains `[wrong-key]` for `RebuildReason::WrongKey`). Recorded here too
(m5): only `zstd` is pinned exactly; `zstd-sys`, which carries the libzstd version the committed
snapshot bytes depend on, is pinned by `Cargo.lock` (every build is `--locked`, and `cargo
update` is never run plain); `minicat check` re-encodes, so a different libzstd is noticed.

**Fix round 2 (CI 44514966689; re-review of fix round 1).**

- *Fuzz packages.* The M4 targets live in their own cargo-fuzz package `fuzz/meta` (own
  workspace, own `Cargo.lock` generated from the root lock, the root `[patch]` copies), so
  `fuzz/` keeps the SQL targets and their graph. The SQL package's lock differs from its pre-M4
  state only by `sql-ident`'s planned `icu_casemap` closure (R-M4-41). Under the pinned nightly
  and the sanitizer flags, `russh` (through `hacivat-session`) failed to compile (E0391); neither
  package compiles it now. cargo-fuzz picks `fuzz/` from the repository root, so every command
  for the M4 package passes `--fuzz-dir fuzz/meta`.
- *`hacivat-introspect` feature `session` (default).* It gates the optional dependencies
  `hacivat-session`, `hacivat-secrets` and `hacivat-settings` and the modules `assemble`,
  `ddl_effects`, `engine`, `fragment`, `freshness`, `hub`, `minicat`, `mysql`, `pg`,
  `refresh_coord`, `sqlite`, `testing` and `ticket`. Always built (the pure grammars): `pack`,
  `scope`, `filter`, `glob`. **For Task 1c Part B and Task 4:** the part of `pack` that builds a
  `MetaQuery` (the renderer) goes in a `#[cfg(feature = "session")]` submodule of `pack`; the
  parser, sections, macros and lint stay session-free; the `Introspector` trait and anything else
  that names a session type go in a gated module. `fuzz/meta` takes the crate with
  `default-features = false`; CI checks `cargo check -p hacivat-introspect --no-default-features`
  in `build-test`, and an xtask test proves that graph reaches no session, network, SSH,
  secrets, settings or keyring crate.
- *I-R1.* `DdlIsPure` refuses a `hacivat-ddl` whose own `default` closure contains `apply`
  (`apply` is left out of the walk, so `default` must not turn it on).
- *I-R2.* `MetaQueryOnlyFromPacks` also refuses any `impl` header naming `MetaQuery` (a local
  trait whose methods call `Self::new`), `MetaQuery` inside a macro invocation's brackets
  (`mk!(MetaQuery)` with `$t::new` in the macro), and the raw identifier `MetaQuery::r#new`
  (the tokenizer reads `r#ident` as `ident`, and `br#"…"#`/`cr"…"` as strings).

**Fix round 3 (re-review 2).**

- *I-R4.* `MetaQueryOnlyFromPacks` refuses **any associated-item access on `MetaQuery`** outside
  `hacivat-session/src/meta.rs`, `hacivat-introspect/src/pack/**` and a crate's `tests/`: the
  segment `MetaQuery`, then optional generic arguments (one balanced `<…>`, with or without a
  leading `::`), then optionally the `>` that closes a qualified path (a `<` that follows no path
  segment), then `::` and any item. `MetaQuery::<'_>::new`, the spaced turbofish,
  `MetaQuery::<>::new`, `<MetaQuery<'_>>::new` and `MetaQuery::CONST` are refused; type
  positions (`&MetaQuery`, `Vec<MetaQuery>`, `Vec::<MetaQuery>::new()`, `q: MetaQuery`,
  `-> impl Iterator<Item = &MetaQuery>`, `impl Into<MetaQuery>`) stay legal.
- *m-R2-2.* `impl` counts as an impl-block header only after none, `}`, `;`, `]`, `{`, `pub`,
  `unsafe` or `default`; `impl Trait` in return or argument position is a type.
- *I-R3.* `path = "…"` inside `#[cfg_attr(…)]` / `#![cfg_attr(…)]`, at any nesting, is a
  `#[path]` target (followed, and refused when it leaves `src/`).
- *m-R2-1.* `IntrospectNotExec` also refuses any declared `hacivat-results` or `hacivat-exec`
  dependency of `hacivat-introspect` (any kind, optional or not).

**Fix round 4 (re-review 3, I-R5).** `MetaQueryOnlyFromPacks` also refuses `MetaQuery` anywhere
inside the `<…>` of a qualified path that is followed by `::`, whatever wraps it there
(parentheses, tuples, `&`/`&mut`, `dyn`, its own generic arguments): `<(MetaQuery)>::new`,
`<((MetaQuery<'_>))>::new`, `<(MetaQuery,)>::default`, `<&MetaQuery>::clone`, also after `=>`.
A `<` opens a qualified path when it follows no identifier, no `::` and no closing `>` of
generic arguments (after `->`/`=>` it is qualified). In doubt it refuses: `<Vec<MetaQuery>>::new`
is refused too, and `Vec::<MetaQuery>::new()` is the legal spelling. The runtime guard (T5's
`join` allowlist) stays the primary defence; this rule is defence in depth.

## m4-t01c — introspection seams

M4 Task 1c Part B declares `hacivat-introspect`'s seams with fail-closed stubs (Global
Constraints, "No silent weakening"). Session-side seams are in ADR 0004 `m4-t01c`.

- **Feature split kept.** A 14th `session`-gated module, `introspector`, holds the
  `Introspector` trait, `introspector_for`, `IntroError`, `Staleness`, `IntroCaps` and
  `FacetRows` (every one names `MetaConn` or `MetaError`). `pack`, `scope`, `filter` and `glob`
  stay ungated; `pack::Mode` and `scope::NsInfo` live there so both builds share them. The
  renderer that calls `MetaQuery::new` is the gated `pack::render`
  (`RenderedQuery::as_meta_query`).
- **What each stub answers.** Every introspector method (the four engines and
  `testing::FakeIntrospector`) answers `IntroError::NotImplemented`; `Pack::parse_all`
  answers `PackErrorKind::NotImplemented` (the pack files are compiled in, header only);
  `Pack::render` answers `RenderError::NotImplemented`; the scope selects only `@:@`; the
  filter matches everything and pushes nothing down; `freshness::check_state` answers
  `Staleness::Unknown`; `plan_refresh` widens any statement with a catalog effect to
  `Target::Namespace(current)` (pending until `COMMIT` where DDL is transactional);
  `CatalogHub::ensure` answers an empty catalog with a detached `IntroHandle`, whose every
  request resolves to `NotImplemented` and publishes nothing; `fragment::on_session`,
  `minicat::run_system_sections`, `{pg,mysql}::facets` and `testing::big_catalog` answer
  `NotImplemented`.
- **`CatalogHub::new` takes a `SessionLookup`**, not a single `SessionManager` (the plan's
  wording): a hub serves every profile, so it looks each profile's sessions up;
  `ConnectionManager` implements it, and tests pass a double. Settings arrive as
  `HubSettings { paths, metadata }`.
- **`MetaConn::offline(dbms)`** (`#[doc(hidden)]`, ADR 0004 `m4-t01c`): an inert connection
  for tests that must hand an introspector a `MetaConn`; Task 5 keeps it inert.
- **Localization.** `intro-*` is a new ID prefix of `hacivat-core.ftl` (named exceptions in
  `hacivat-l10n-schema` and xtask's `l10n` rules, beside T1a's `model`/`ddl`); the block
  `## m4-t01c-core` holds the 27 introspection messages alg/02 names (HV-INTRO-01..03, -05,
  -11..13, -15, -16) plus `intro-catalog-race-retried`. `model-cache-disabled-by-policy` and
  `ddl-warn-executable-comment` are T1a's already.

## m4-t07 — DDL IR, renderers and quoting

M4 Task 7 fills `sql_ident::quote` (R-M4-28) and `hacivat-ddl`'s renderers, placement,
qualification and option applicability (alg/03 HV-DDL-07 renderer layer, -09, -10, -11 v1
subset). The bodies are copied from the DDL spike (`da358e4`, R-M4-40, U-1) onto Task 1a's
seams and reviewed as new code.

- **Quoting lives in `sql-ident`, once.** `needs_quoting`/`quote` follow each server's own rule
  (PostgreSQL `quote_identifier`: bare only `[a-z_][a-z0-9_]*` and not a non-unreserved
  keyword; MySQL/MariaDB/SQLite bare only `[A-Za-z_][A-Za-z0-9_]*` and not a reserved word;
  every other dialect always quoted). With no server version in the signature, a word that is
  reserved on **any** line of the keyword tables is quoted (probing the newest and the oldest
  rows covers every one-sided gate). MariaDB's table has no reserved flag, so every MariaDB
  keyword is quoted (`id`, `name`, `status`): safe and noisy, as the spike reported. Renderers
  call only `RenderCtx::ident`/`qname`, which call `quote`; there is no other quoting function.
- **MySQL and MariaDB always backtick** (T1a review m2). `QuoteCtx::ansi_quotes` is carried but
  never trusted: backticks quote identifiers in every `sql_mode`, while `"` under a wrong
  `ANSI_QUOTES` would make a name a string literal.
- **`QuoteCtx` holds `Option<bool>`** (seam change, T1a review m2): it mirrors `ServerFacts`,
  and `Default` is "unknown". `string_literal` picks a form that reads the same under either
  value of an unknown setting: PostgreSQL with a backslash and `standard_conforming_strings`
  not known on → a dollar-quoted string (`$hv$…$hv$`, the tag chosen so `$tag` does not occur
  in the text); `E'…'` is never produced. MySQL/MariaDB with `NO_BACKSLASH_ESCAPES` not known
  on → the backslash doubled: exact under the server default and, otherwise, an extra
  backslash that can never end the literal early. NUL is refused everywhere.
  `QuoteError::NotImplemented` is kept (never produced) so callers' matches stay valid.
- **Qualification is the producer's.** IR names are `QualifiedName`s printed with exactly the
  parts they hold; `produce::qualify::qualify(name, &QualifyCtx) -> QualifiedRef` decides
  which parts a producer keeps. `Auto` is per name: bare when its schema is the current schema
  (the target's current schema, else the first `search_path` entry) and the name is not
  ambiguous in the scope; SQLite keeps `main` bare. The plan's sentence "unqualified only when
  every object is in one schema" is read through its own test (`app.t1` + `sales.t2`, current
  `app` → `t1`, `sales.t2`), which is per name. Server text is never re-qualified.
- **Placement is the renderer's within a table.** `ConstraintDef` has no position:
  `produce::placement::place(constraints, mode, dialect, &PlacePlan) -> Vec<Placed>` decides
  `InColumn`/`InTable`/`After` under `GenOptions::placement` with the dialect overrides (MySQL
  FK never inline, an inline MySQL unique key only when named after its column, an
  `AUTO_INCREMENT` key stays in the table, SQLite always inside with
  `ddl-sqlite-placement-forced`, PG `NOT VALID` after). A renderer places a table's own
  constraints (`After` → `ALTER TABLE … ADD` statements of the same op); a constraint
  postponed past *later* tables (`PlacePlan::later`, cycles) is the producer's separate
  `AlterTable { AddConstraint }` op, which M4 renders (only that action; any other is
  `Unsupported`).
- **Options the renderers honour.** `OR REPLACE` and `IF NOT EXISTS` come from the op's flag
  or `GenOptions`, each only where `options::or_replace_supported`/`if_not_exists_supported`
  say the dialect has it for the kind (`OR REPLACE` wins); `skip_definer` and `or_replace` edit
  only the head of MySQL/MariaDB `SHOW CREATE` text, token by token
  (`render::mysql::edit_head`, moved here from the spike's producer so T7's test can pin it);
  a head of any other shape stays verbatim (producers notice `ddl-head-not-edited`). PG 14+
  turns a plain `CREATE TRIGGER` into `CREATE OR REPLACE TRIGGER`. `ServerDefinitions` applies
  to MySQL/MariaDB **and SQLite** (R-M4-24), not only MySQL as in the spike.
- **IR additions** (`ir.rs` is T7's): `sig: Option<Name>` on `DdlOp::Owner` and `GrantDef`
  (a routine is never named without its argument types: `None` on a routine is
  `RenderError::Invalid`, never an ambiguous `ALTER FUNCTION f`), and
  `ConstraintBody::single_column`. `RenderError::Invalid(&'static str)` is new for malformed
  ops (an index part with neither column nor expression, `AUTOINCREMENT` without its key).
- **Keyword slots are never free text**: privileges (`[A-Z_ ]`), `WITH LOCAL|CASCADED CHECK
  OPTION`, MySQL `ROW_FORMAT` and index methods (`HASH` stays an unquoted keyword) are
  validated; MySQL grantees must be the server's `'user'@'host'` form. Expressions, types,
  defaults (verbatim per flavor, F9) and server text stay raw, as the IR defines them.
- **Output.** Never from Fluent or the process locale (a child-process test renders under
  `en_US` and `tr_TR` and compares bytes); `\n` only; four-space indentation; `render_text`
  ends each statement with `;\n` and separates statements by one empty line. IR goldens:
  `tests/golden/ir/{pg,mysql,mariadb,sqlite}/create_table.sql` (`UPDATE_GOLDEN=1`, R-M4-16).
- **Not in M4's IR (for T19 and M7):** `ALTER SEQUENCE … OWNED BY` after the table (a serial's
  sequence must exist before the table's `DEFAULT nextval(…)`; `SeqDef::owned_by` renders the
  clause inside `CREATE SEQUENCE` only); MySQL secondary indexes as `KEY` table elements (the
  spike's round-trip choice; M4 renders `CREATE INDEX`); a standalone comment on a trigger or
  constraint (`CommentTarget` has no table; the trigger def renders its own).
- **Fix round 1 (review of `abb67e4`).**
  - *I1:* `quote` and `string_literal` answer `QuoteError::NotImplemented` for every dialect
    without a verified rule (SQL Server, ClickHouse, Oracle, and any later one); only PG and
    PG-wire, MySQL/MariaDB, SQLite and Generic (ANSI) produce text. ClickHouse reads `\` as an
    escape in every quote kind, and SQL Server's `"` depends on `QUOTED_IDENTIFIER`, so a plain
    `"…"`/`'…'` there could break out.
  - *M1:* on MySQL/MariaDB a name starting with `_` is always quoted (a bare `_<charset>` is a
    charset introducer to MySQL's lexer). This also covers T4's `SHOW CREATE` macro.
  - *M2:* SQLite `Auto` also qualifies an ambiguous `main` name (the plan's ambiguity rule;
    SQLite resolves a bare name in `temp` first).
  - *M4:* `render::terminator` puts `;` on its own line when the last line holds `--` or `#`.
    Producers (T19 `Script::text`) end statements through it too.
  - *M5:* PG refuses `GRANT … ON EXTENSION` and `ALTER EXTENSION … OWNER TO` (they do not
    exist); `COMMENT ON EXTENSION` stays.
  - The reviewer's lexer round-trip property is
    `crates/sql/sql-safety/tests/quote_roundtrip.rs`: the only crate that reaches both
    `sql-ident` and `sql-lexer` without a manifest edit.

## m4-t03 — model and identifier keys

M4 Task 3 fills the arena, the kind schema and the identifier keys (HV-MODEL-01, HV-INTRO-14).
The bodies of `kind`, `name`, `ids`, `props`, `namespace` and `fixture` come from the model
spike (`d86313b`), adapted to the T1a seam.

- **Arena.** The layout is breadth-first: index 0 is the root, a parent precedes its children,
  and children are contiguous and in insertion order. It is computed in O(n) by a counting
  sort. Every build re-validates the root, the layout and the kind schema. The crate-private
  `from_arena` (for Task 11's decoder) refuses a parent after its child, non-contiguous
  children, a root with a parent and an empty arena. A `NodeId` of another subtree is a bad
  layout, never a panic. `INVISIBLE_CHARS` is derived on every build; a NUL name gets
  `LOSSY_NAME`. `Obj::add_flags` adds flags (it never replaces them).
- **Identity.** `ObjectKey` keeps a structural `Eq`/`Hash`/`Ord`; `same_object` compares the
  server id first. `ObjectKey` gains `raw_kind: Option<Name>` (T1a review m10): an unknown
  kind's raw id survives a serde round trip, so two newer kinds stay two user-data keys. The
  raw id is written as the kind id. A raw text that equals a known id is written as
  `"unknown"` plus a `raw_kind` field. `find` matches a server id only within the
  namespace's database (PostgreSQL OIDs are per database). Otherwise it walks the exact path,
  taking the first step through the name indexes.
- **Kind schema.** `exists_in` is exact for PostgreSQL, MySQL, MariaDB and SQLite (alg/02
  matrix rows 1, 2 and 4); later engines keep the spike's permissive rule until v1.x.
  PostgreSQL composite types sit in `Relations` (they share `pg_class`). MariaDB sequences
  are tables. SQLite `VirtualTable` (code 14) has columns only.
- **Identifier keys (`sql_ident::key`).** `match_key` builds keys by these rules:
  - PostgreSQL, and the forks that run PostgreSQL's parser (Timescale, Yugabyte YSQL): folds
    unquoted ASCII and cuts at 63 bytes. With a non-UTF-8 or unknown `server_encoding`,
    unquoted non-ASCII input is `ServerDependent` (rule 3), and so is a quoted non-ASCII name
    that the cut might reach in the server's encoding, counted at 4 bytes per non-ASCII
    character (the longest of any server encoding, `EUC_TW` and `MULE_INTERNAL`).
  - Redshift, CockroachDB and unnamed PostgreSQL forks: `ServerDependent` (fix round 1, I1).
    AWS "Names and identifiers" says Redshift folds ASCII letters of delimited identifiers to
    lowercase and allows 127 bytes; CockroachDB lowercases unquoted names with Unicode rules
    (Inferred from its `lexbase.NormalizeName`). `sql_ident::fold`/`ident_eq` (M3) still treat
    every PG-wire fork like PostgreSQL; that is M3's code, carried to the next owner.
  - MySQL/MariaDB:
    - database, schema and table (view) names are exact under `lower_case_table_names = 0`,
      ASCII-caseless under 1 and 2, and `ServerDependent` for any other value;
    - trigger names (`NameClass::Trigger`) are exact under 0 and `ServerDependent` under 1 and
      2, where only a byte-exact stored name is `Found` (fix round 1, I3);
    - account names, users and roles (`NameClass::Account`, `KindGroup::Accounts`), are
      always exact (fix round 1, I2): MySQL 8.4 §8.2.3 says comparisons of `User` values are
      case-sensitive, and roles are accounts;
    - every other class is ASCII-caseless, and a caseless text with a non-ASCII **cased**
      letter (`Lowercase`, `Uppercase`, `Lt`) is `ServerDependent`.
  - SQLite: ASCII-caseless (accounts included; SQLite has none).
  - Other dialects: always `ServerDependent`.

  `FoldRule::for_names` gives the stored rule, and `FoldRule::key` keys a stored name; a test
  shows the two agree with `match_key` on every text it keys. No Unicode or locale case
  mapping runs anywhere. The reviewer's whole-Unicode test
  (`hacivat-meta/tests/review_hostile.rs`, ignored by default, about 50 s in debug) gives the
  same key digest under `LC_ALL=C` and `LC_ALL=tr_TR.UTF-8`.
- **MySQL/MariaDB trigger names, the sources.**
  - MySQL 8.4 §11.2.3: "the case sensitivity of the underlying operating system plays a part
    in the case sensitivity of database, table, and trigger names".
  - The same section, on `lower_case_table_names`: "This variable does not affect case
    sensitivity of trigger identifiers."
  - MariaDB "Identifier Case-sensitivity": "Database, table, table aliases, and trigger names
    are affected by the system's case sensitivity, while index, column, column aliases, stored
    routine, and event names are never case sensitive."
  - Hence: exact on the M4 matrix (Linux, `lower_case_table_names = 0`); under 1 and 2 the
    file system decides, which Hacivat does not know, so the rule stays unpinned (Inferred).
    The plan's "triggers ASCII-caseless" is superseded.
- **Deviations from the plan text, recorded** (the review accepted 2–5):
  - `ServerDependent` covers every caseless MySQL class (routines, events, indexes as well as
    columns). In addition, a namespace whose stored names in a class contain a non-ASCII cased
    letter answers `Unknown(ServerDependent)` instead of `Missing` for any miss: the server
    may fold `İD` or the Kelvin sign to ASCII.
  - Only where the server compares case-insensitively (MySQL/MariaDB) does an exact stored
    name turn a `ServerDependent` probe into `Found`. For PostgreSQL the server may fold the
    probe itself, so the answer is `Unknown`.
  - A probe whose `IdentCtx` disagrees with the namespace (another engine family, another
    `lower_case_table_names`, or an unpinned fork) is `Unknown`.
- **Kind schema, fix round 1.** MySQL/MariaDB views take no `Trigger` (or `Rule`) children.
  PostgreSQL domain constraints and defaults stay in `TypeBody::Domain` (`checks`,
  `default_raw`, `not_null_constraint`): alg/02's matrix lists no domain child kinds, so
  `Domain` has none; T16 asks here if it needs them as objects.
- **Arena, fix round 1.** A duplicated `(kind, server id)` in one namespace is counted
  (`NamespaceSnapshot::duplicate_server_ids`) and finds nothing by id; each object is still
  found by its path, which checks the id. `Obj::with_flags` is renamed `Obj::add_flags` (it
  adds). `ObjectKey::unknown` builds a key with a raw kind; `ObjectKey::is_well_formed`
  (asserted on serialization in debug builds) says a known kind carries none.
- **Memory (ruling R-W1-2, fix round 1).** `ColumnProps.ext` became `Option<Arc<ColumnExt>>`,
  and the rare fields (the named `NOT NULL` constraint, identity, generated, collation, enum
  labels) moved into one `rare: Option<Arc<ColumnRare>>`, before Task 11's codec freezes the
  cache format. `ColumnProps` went from 216 B to 104 B; the 160k-column fixture measures
  **224 B per column** (a 104 B `Obj`, 104 B `ColumnProps`, a 16 B `Arc` header) and 414 B per
  table, within alg/02's 250 B and 600 B. `model_perf` asserts both, and
  `a_column_stays_within_the_memory_target` checks the sizes on every 64-bit test run.

## m4-t04 — query packs

M4 Task 4 fills `hacivat-introspect::pack` (`mod`, `parse`, `render`, `lint`; alg/02
HV-INTRO-04, R-M4-3, -7, -29, -32, -37). The packs themselves stay header-only until Tasks
16–18.

- **Grammar.** Line-oriented, as alg/02 HV-INTRO-04: `--- name: <Section> ---`,
  `--- returns: <col type, …> ---` (required; types `text bool i16 i32 i64 u32 u64 f64 bytes
  timestamp`), the flags `--- joinable ---` and `--- show-create ---` (before the first SQL
  line), `--# if/elif/else/end` nested at most 4 deep (a parse error, not only a lint, so the
  tree a fuzzer builds stays shallow). Conditions are a closed language over `version` (an
  integer: no `12.0` form), `mode` (`full incremental brief system`), `level`, `flavor` (a
  `DbmsKind` id) and `flag(mariadb|joined)` with `and`/`or`/`not`/parentheses (at most 8 deep).
  Files merge in byte order of their names, whatever order the caller passes.
- **Fail-closed lexing** (as first written; the review found four gaps, closed in fix round
  1 below). Literals, quoted identifiers and block comments close on their own line; a backslash in a
  literal, a nested `/*`, a raw placeholder (`$` on PostgreSQL and SQLite, `?` on MySQL and
  SQLite, `@` and `[` on SQLite), a control character other than the tab (a bare `\r` ends a
  PostgreSQL `--` comment), `;`, a MySQL `#` comment, a MySQL `--` not followed by a
  space, and `#{` inside a literal or comment are parse errors. `::` is never a bind.
- **List binds** (R-M4-3) render `(…)` after `IN`, written `<operand> IN :[name]` on one line;
  `NOT IN :[name]` is refused (a chunked `NOT IN` matches what another chunk excludes), as is a
  second list name in one section. Chunks hold at most `MAX_IN_LIST` = 1,000 items; an empty
  list renders no query, but every scalar bind is still checked. Rows of chunks are
  concatenated. (Superseded by fix round 1, I3, below: the placement is now enforced.)
- **Macros** are `Macro::{MaxInList, Ident}`. The plan's "catalog prefix" macro is **not**
  written: no M4 dialect has a privilege-dependent catalog prefix (that is Oracle's
  `ALL_`/`DBA_`), and PG packs must spell `pg_catalog.<name>` literally for the lint. It lands
  with the first pack that needs one. `#{ident:<bind>}` (R-M4-37) renders only in a
  `show-create` section, which only a MySQL/MariaDB pack may declare (elsewhere it is a lint
  finding and the renderer refuses it), through `sql_ident::quote::quote(DialectId::MySql, …)`
  with `QuoteCtx::default()` (MySQL identifiers are always backticked, T7); the renderer re-checks that the result is one backtick-quoted
  identifier with every inner backtick doubled, and refuses NULL, empty and NUL names. A
  `show-create` section holds no literal, comment, quoted identifier or bind, and each macro is
  bounded by whitespace, `.`, `(`, `)` or `,` (two adjacent quoted names would read as one).
- **No silent "no rows".** A context for which every SQL line is gated away answers
  `RenderError::NoVariant`, never an empty `Vec` (alg/02 edge 8; F31).
- **Joined connections** (R-M4-29): `PackFlags::JOINED` in the context refuses every section not
  marked `joinable` (`RenderError::NotJoinable`); the `joinable` flag travels into
  `RenderedQuery` and `MetaQuery`. T5's statement allowlist stays the primary runtime check.
- **Feature split, refined.** Rendering is session-free (the `fuzz/meta` target `pack_parse`
  parses, lints and renders without `hacivat-session`); only
  `RenderedQuery::as_meta_query`, the single `MetaQuery::new` call outside `meta.rs`, is
  `#[cfg(feature = "session")]` inside `pack::render`. `pack` re-exports `DialectId`,
  `BindValue`, `LogicalType` and `Flavor` so the fuzz package needs no manifest change.
- **Lint** (`lint(pack, dialect)`, every branch read): HV-INTRO-14 canonical spelling (PG
  `pg_catalog`, `information_schema` and `pg_*` lower case and `pg_*` always qualified by
  `pg_catalog.`; MySQL `INFORMATION_SCHEMA.<UPPER>` and lower-case `mysql.`,
  `performance_schema.`, `sys.`; SQLite `sqlite_schema`, never `sqlite_master`); folding
  (`lower(`, `upper(`, `lcase(`, `ucase(`, `ILIKE`, `COLLATE`, `~*`, `!~*`, `~~*`, `!~~*`
  anywhere in code, not only on a name column: packs compare bound exact names); MySQL
  `UPDATE_TIME`, `GROUP_CONCAT` and, added here, executable comments `/*!`, `/*M!`; PG
  ordering comparisons on `age(…)` (R-M4-8); `txid_current` and `pg_current_xact_id` in every
  dialect; `#{ident:…}` outside `show-create`; `--#`/`---` inside a literal.
- **Variants.** `Pack::reachable_variants(section)` answers one context per distinct path
  through the section's blocks (versions at every threshold and its neighbours, every mode and
  level a condition names, the dialect's flavor plus every named one plus one unnamed family
  member, `flag(mariadb)` tied to the flavor, `flag(joined)` only on joinable sections). A
  property test checks it against a brute-force sweep.
- **Fix round 1 (security review: 0 Critical, 4 Important).**
  - **I1, lexer vs server.** The claim "they never disagree" was false in four places; each is
    now a parse error, not only a lint finding: a backtick in a PostgreSQL pack (an operator
    character there); `/*` anywhere inside a block comment *including its closing `*/`*
    (`/* a /*/` opens a nested PostgreSQL comment); SQLite `#name`, and `:` followed by a digit
    or a non-ASCII byte on every dialect (SQLite parameters); MySQL `/*!` and MariaDB `/*M!`
    executable comments. What is enforced now: every form the review or this task found where
    the server would read code that the lexer reads as a literal or comment (or the reverse),
    on the servers' default settings, is refused; `tests/hostile_t04_review.rs` H1–H4 pin them.
    That is a list of closed gaps, not a proof: a server setting that changes lexing
    (PostgreSQL `standard_conforming_strings = off` is covered by the backslash rule; MySQL
    `ANSI_QUOTES` changes only what `"` means, not where it ends) is the residual risk.
  - **I2, provenance (controller ruling).** A renderable `Pack` comes only from `&'static`
    text: `Pack::parse(dialect, &'static str)`, `Pack::parse_all(&[PackFile])`,
    `Pack::builtin`. Runtime text parses into a `DraftPack` (same parser, lint and renderer),
    whose `DraftQuery` has no `as_meta_query` (two `compile_fail` doctests in `pack::render`).
    `Box::leak` still yields `&'static`, so check-deps gains **`PackParseOnlyFromPacks`**
    (named exception: one rule plus tests in `xtask/src/check_deps.rs`, `## m4-t04-packs`,
    shaped like `MetaQueryOnlyFromPacks`): `Pack::parse`, `Pack::parse_all` and any other
    associated item of `Pack` but `builtin`, an alias of `Pack`, an `impl` header or a macro
    invocation naming it, are refused outside `hacivat-introspect/src/{pack,pg,mysql,sqlite}/**`,
    a crate's `tests/` and `fuzz/meta/`.
  - **I3, list binds (controller ruling, fixed now).** A list bind is legal only as a
    top-level `AND` conjunct of the `WHERE` clause written `<column> IN :[name]` (column
    optionally qualified), at parenthesis depth 0, preceded by `WHERE` or a conjunction `AND`
    (not the `AND` of `BETWEEN`), followed by nothing, `AND` or `ORDER`; the nearest clause
    keyword before it is `WHERE` (so not an `ON` clause); used once per section (the section
    text, every branch counted); and the statement holds no `OR`, `XOR`, MySQL `||`, aggregate
    call, `GROUP BY`, `HAVING`, `LIMIT`, `OFFSET`, `FETCH`, `DISTINCT`, `OVER`, `WINDOW` or
    set operation at depth 0. It is checked in every `--# if` variant (at most 4,096 candidate
    contexts, else `ListBindTooManyVariants`). Items are deduplicated (first seen kept) before
    chunking. An empty list renders no query, which is right because no negated form parses
    (tested both ways). Aggregates in correlated subqueries (depth ≥ 1) stay legal.
  - **I4, `age()`.** The lint now requires every `age(` call to be one side of
    `age(x) BETWEEN 0 AND age(y)` (casts after the lower operand allowed); a comparison,
    parentheses, arithmetic or `IS` around it is a finding.
  - **Minor.** PG 18 `casefold(`/`initcap(` are folds. PG catalog functions without a `pg_`
    prefix that packs use (`format_type`, `obj_description`, `has_*_privilege`, `age`,
    `unnest`, …) need `pg_catalog.` in every section, and in a `joinable` section every
    function call does (R-M4-46: no pinned `search_path` there). `#{ident:…}` outside
    `show-create` is a parse error (so the lint finding and the `ExecutableComment` lint
    finding are gone: the parser refuses both first). The identifier macro refuses a bind
    whose hint is not text. The fuzz target re-lexes every render (no `;`, placeholders equal
    binds, `$1..$n` in order, `show-create` quotes closed), checks parse and render are
    idempotent, and parses a two-file arm. `tests/pack.proptest-regressions` is kept as a
    named addition (repo practice: `sql-split`, `hacivat-sql-editor`).
  - **T5 carry, enforced at parse time.** T5's joined `MetaConn` and the session's read
    check refuse text holding `INSERT`, `UPDATE`, `DELETE`, `MERGE` or `INTO` anywhere, and
    `PRAGMA` statements. So a `joinable` section may not hold those words anywhere in its text
    (literals and comments included; `PackErrorKind::JoinableWriteWord`; a value that needs one
    is a bind), and no pack may hold a `PRAGMA` word in code (`PragmaStatement`): SQLite
    sections read through the `pragma_*` table-valued functions (`pragma_table_info(?)`).
    Tasks 16–18 write their sections under these rules.
  - **Carry (controller → T5/T17).** The `show-create` quoting assumes a UTF-8 connection
    charset; under GBK-family charsets a name can break out. `SET NAMES utf8mb4` at connect
    and after every reconnect, assert `@@character_set_client`/`_connection`, refuse
    otherwise.
- **Fix round 2 (re-review: 1 Important, I5; minors m7–m10).**
  - **I5, a general rule (controller ruling).** In a list-bind query, expression depth counts
    `(…)`, `[…]` and `CASE … END`, and the top-level `AND` conjunct of the `WHERE` clause that
    holds the list must be **exactly** `<column-ref> IN :[name]` (`<column-ref>` one or more
    `.`-joined identifiers): so `CASE WHEN a AND x IN :[ids] … END = 0` and
    `ARRAY[… x IN :[ids] …] = …` are refused. A separate *query level* counts only subquery
    parentheses (`(SELECT`, `(WITH`, `(VALUES`), so a scalar subquery, `EXISTS` or CTE body
    may aggregate freely while the list's own query may not.
  - **Function calls: an allowlist, not an aggregate denylist (the choice asked for).** At the
    list's query level (inside expressions too, so `coalesce(max(x), 0)` is caught) every
    function call must be a row-by-row function on `ROW_FUNCTIONS` (PostgreSQL accessors and
    snapshot readers, shared string and number functions, MySQL `IF`/`IFNULL`/…, SQLite
    `iif`/`substr`/…, every SQLite `pragma_*` table-valued function) or a PostgreSQL deparser
    (`pg_get_*`, `format_type`, `*_description`, `has_*_privilege`, `pg_*_is_visible`,
    `pg_identify_object*`, `pg_describe_object`). `GROUP`, `HAVING`, `LIMIT`, `OFFSET`,
    `FETCH`, `DISTINCT` (but `IS [NOT] DISTINCT FROM`), `OVER`, `WINDOW`, `FILTER`, `WITHIN`
    and set operations are refused at that level; `OR`/`XOR`/MySQL `||` at its expression depth
    0. Workable for Tasks 16–18: their retrieve sections call deparsers and accessors row by
    row; a function they need that is not listed is added to `ROW_FUNCTIONS` in
    `pack/parse.rs` by a named exception, reviewed as such. Every aggregate family (`any_value`,
    `range_agg`, `corr`, `regr_*`, SQLite `total`, user aggregates, …) is refused without being
    named.
  - **m8, joinable sections: the same allowlist.** A `joinable` section may call only
    `ROW_FUNCTIONS` plus the read-only aggregates `count`, `max`, `min`, `sum`, `bool_and`,
    `bool_or`, `every` (`PackErrorKind::JoinableFunction`), anywhere in its text. So
    `set_config`, `nextval`/`setval`, `pg_advisory*`, `lo_*`, `dblink*`, `pg_notify`,
    `txid_current`/`pg_current_xact_id`, MySQL `GET_LOCK`/`RELEASE_LOCK`/`SLEEP`/`BENCHMARK`
    and the `search_path`-dependent deparsers (R-M4-46) are refused without a denylist. T5's
    runtime allowlist stays the primary check.
  - **m7.** `render` refuses a context whose `PackFlags::MARIADB` disagrees with
    `flavor == MariaDb` (`RenderError::FlavorFlagMismatch`), so no context reaches a branch the
    parse-time checks (which assume they agree) never read.
  - **m9, `age()`.** The lint accepts an `age(` call only inside
    `age(x) BETWEEN 0 AND age(y)` with a boolean boundary before the lower operand (`WHERE`,
    `AND`, `OR`, `NOT`, `ON`, `WHEN`, `THEN`, `ELSE`, `SELECT`, `(`, `,`) and one after the
    upper bound's arguments and casts (`AND`, `OR`, `)`, `,`, a clause word, the end): no
    arithmetic around the form, no lower operand other than an `age(` call, no lower bound but
    `0`.
  - **m10.** The `compile_fail` doctests carry `E0599` and `E0597` in their fence and in a
    comment; both codes were observed by compiling the snippets (each is the only error).
    Stable rustdoc does not check the fence code, so the comment and the positive controls
    in `tests/hostile_t04_review.rs` R9 are what pin it.
  - The re-review's probes R1–R9 are in `tests/hostile_t04_review.rs`, flipped to assert the
    fixes. **m11** (`Pack::parse` call sites in `pg/`, `mysql/`, `sqlite/` must take
    `include_str!` or a `const`) is carried to the T16–T18 reviews by the controller.

## m4-t09 — snapshots and lookups

M4 Task 9 fills the catalog, the tri-state `ModelView`, the resolver, the search paths, the
negative cache and search (HV-MODEL-02). The bodies of `Catalog`, `publish` and
`CatalogSnapshot` come from the model spike (`d86313b`), adapted to the T1a seam; its five
catalog-level tests (deferred by Task 3) are in `tests/snapshot.rs`.

- **Publish.** One writer (`Catalog::publisher` hands it out once). A publish clones the
  current generation shallowly (`Arc::make_mut` on the namespace and state maps, only when a
  change touches them), applies every change in order, stores once and then broadcasts. Any
  error publishes and announces nothing. Untouched namespaces stay `Arc::ptr_eq`; so does the
  states map when no state changes. An empty change list is a no-op that answers the current
  generation. `Put` stamps the arena with the new generation: a unique `Arc` is moved, a
  shared one is copied (by reference count for its objects) to be stamped, so the engine hands
  over unique arenas. `Remove` also drops every state of the namespace's containers, and
  accepts a namespace the catalog only lists by state. Publishes are serialized by a writer
  mutex (never taken by readers); the store is a compare-and-swap whose failure is
  `ModelError::NotTheWriter` (alg/02 edge 2's misuse guard).
- **Events.** `publish` announces `Published` with an empty delta plus one `NamespaceReplaced`
  per changed namespace; `publish_with_delta` announces the merge's delta (Task 10) alone.
  `announce_pending_confirmation` sends R-M4-22's `PendingConfirmation` without a publish.
  `pinned_generations` lists the generations still alive (alg/02 edge 1).
- **Completeness.** A miss is a fact only in a namespace whose state is `Loaded` at
  `Structure` or above with an arena. `Names` is never enough: which kinds a `Names` pass
  reads completely (PostgreSQL indexes share the relations name space) is pinned by Tasks
  16–18, so until then a miss at `Names` is `Unknown(NotLoaded)`. An arena without a state is
  treated as not loaded. `Loading`, `Failed` and `Partial` answer hits from a cached arena and
  `Unknown(Loading | Failed)` for misses. `OutOfScope` answers nothing, not even hits. Facet
  states never affect object existence.
- **Schema list.** `ContainerKey::schema_list(database)` (PostgreSQL: the database's schemas;
  elsewhere the server's) marks the list of schemas as complete. With it `Loaded`, a schema
  the catalog neither holds nor lists does not exist: a qualified name into it is `Missing`,
  and a search-path entry naming it is skipped, as the server does (`$user` most often).
  Without it both are `Unknown(NotLoaded)`. Task 15 sets it after each scope refresh.
- **Lookups.** An unqualified name walks the search path; the first namespace with a hit is
  `Found`, even when an earlier one is unknown (alg/02 step 5, "Found if any"). Until Task 26
  serves `pg_catalog` from the system snapshot, a hit in `public` may in principle be shadowed
  by a `pg_catalog` object of the same name; misses stay `Unknown(OutOfScope)` there. The
  unknown reason is the first one along the path. A PostgreSQL `database.schema.name` naming
  another database than the session's is `Unknown(NotLoaded)` (the server refuses
  cross-database references). An empty path (MySQL with no current database) is
  `Unknown(NotLoaded)`. Accounts are looked up in the server namespace.
- **Search paths.** `SearchPath` gains `database` and `PathEntry { ns,
  relations_and_types_only }`. `for_pg(database, setting, user, temp)` (the plan wrote
  `for_pg(path, temp)`; the setting keeps `$user` and `pg_temp` as tokens, as
  `SchemaCtx::search_path_setting` does): the temporary schema first unless the setting names
  `pg_temp`, for relations and types only; `pg_catalog` first unless named; duplicates keep
  their first place. `for_mysql(current)`, `for_sqlite(attached)` (`temp`, `main`, then the
  attached in order), and `exact(database, entries)`.
- **Hooks.** `LookupHooks` is shared by every view of a catalog. `on_unknown(ns)` fires once
  per namespace per generation, only for a namespace the catalog lists (an arena or a state)
  that is not loaded at a sufficient level; never for `Loading`, `Failed` or `OutOfScope`.
  `on_missing_probe(ns, name)` fires at most once per namespace per 30 s, and only when the
  lookup's answer is `Missing`. The callbacks run on the looking-up thread after the hooks'
  lock is released; the lock is never taken on a hit.
- **Scope filters.** `ScopeFilter::excludes(ns, kind, name)` lets the engine's adapter
  (Task 24) say what the scope's object filters left unread; that turns a `Missing` (or a
  complete children list) into `Unknown(OutOfScope)`. The view filter never reaches the view.
- **No allocation on a hit.** A probe whose key is its own text (pinned rule, ASCII, nothing
  to fold, within PostgreSQL's 63-byte cut) goes straight to the name index; the same family
  and table-rule check as `match_named` is applied first. Both are checked against
  `match_key`/`match_named` in debug builds and by a property test. `unsafe` is denied outside
  the UI platform layer, so the perf test counts slow-path steps per thread
  (`slow_lookups_on_this_thread`, `test-support`) instead of hooking the allocator.
- **Resolver and reverse edges.** References resolve on demand: the longest listed namespace
  prefix, then the kind's name index (or a walk of exact names for parent-scoped kinds), the
  server id preferred among same-named hits. `fk_referencing` builds the reverse edges once per
  generation; it needs the schema list and every in-scope schema at `Structure` (PostgreSQL:
  only the target's database, as FKs never cross databases). Out-of-scope schemas are not part
  of the answer (the plan's rule), and the snapshot cannot see object-level scope filters, so
  M7's drop flows must combine it with the view's scope. `find_by_server_id` searches the
  relation kinds of the database's schemas; `Missing` needs the schema list, every schema
  complete and no scope filter.
- **Generation-local ids.** `ObjId.ns` is the namespace's position in key order, from an
  index built once per generation. The per-generation caches are ignored on a clone whose
  public `namespaces` field was replaced.
- **Search.** ASCII-caseless and locale-free; exact matches first, then prefix, then
  substring; within each, namespace key order and arena order; at most `limit` hits kept while
  scanning.
- **Named exception.** `tests/seams.rs` `stub_merge_publishes_nothing` (T1a): its publish
  assertion now uses a refused change list (an empty list is a no-op), fenced
  `// ## m4-t09-snapshots`.
- **Perf** (Apple silicon, release): a publish of one prebuilt namespace into 5,000 has a p95
  of 0.10 ms (max 0.9 ms) over 200 publishes, so R-M4-2's trigger does not fire; a lookup hit
  takes about 0.18 µs and leaves the allocation-free path zero times in 200,000 lookups.

### m4-t09 fix round 1 (review: 1 Critical, 5 Important)

- **C1. A miss is judged in the target's own schema.** `resolve`, `children` and
  `fk_referencing` look in the longest *listed* prefix of a path, which can be the database
  or server namespace when the target's schema is not listed. A miss there is now `Missing`
  only when that namespace is the path's own schema namespace (PostgreSQL `[db, schema]`,
  elsewhere `[schema]`), or when the schema list is `Loaded` and does not hold the schema.
  Otherwise it is `Unknown(NotLoaded)`. A PostgreSQL path that only the server namespace
  holds (another database) is never a fact. So an FK into a schema not yet cached (the state
  every connect passes through, cached namespaces being published before the scope refresh)
  never shows as broken.
- **R-W2-6 (controller ruling; amends alg/02 HV-MODEL-02 step 5 "Found if any").** A walk
  answers `Found` from a namespace only when every earlier entry for the group was a certain
  miss. The first entry that cannot answer (out of scope, not listed while the list is
  unknown, not loaded at `Structure`, loading, failed, a server-dependent probe, an unnamed
  entry) stops the walk with its `Unknown` reason: the server could resolve the name there,
  and a later hit may be the wrong object (then a wrong "unknown column" one level down).
  **Cost:** more `Unknown` answers until the scopes load. On PostgreSQL every unqualified
  lookup is `Unknown` until `pg_catalog` is complete, which Task 26's system layer provides
  (or "Introspect live"). Qualified names are unaffected. Unqualified lookups have no M4
  consumer.
- **I2. Unknown session facts stay unknown.** `PathEntry.ns` is an `Option`; `None` is an
  entry the session could not name, and a walk reaching it answers `Unknown(NotLoaded)`.
  `for_pg` makes one for `$user` when the user is unknown, and for an empty setting
  (`SchemaCtx::search_path_setting` is empty when unknown). A mapper that knows nothing
  passes `SearchPath::default()`.
- **I3. What exists is never skipped.** `PathEntry.must_exist` is set for `pg_catalog` (every
  server has it) and for a temporary schema the caller reports. Unlisted, such an entry is
  `Unknown(NotLoaded)` whatever the schema list says, so system lookups stay `Unknown` until
  Task 26 (F31) without relying on Task 15 listing system schemas.
- **I4. R-M4-41 at schema level.** Under MySQL/MariaDB `lower_case_table_names` 1 or 2, a
  qualifier with no caseless match among the listed schemas is `Unknown(ServerDependent)`
  when any listed sibling's name holds a non-ASCII cased letter. This is the schema-level
  twin of the table-level `uncertain` rule.
- **I5. The schema list is its own container.** `ContainerKey::SchemaList { ns }` replaces the
  `Children { kind: Schema }` key `schema_list` used to build, so "the list is loaded" never
  reads as "this database has no schema children". `children(database, Schema)` (MySQL and
  SQLite: `children(root, Schema)`) answers from the listed schema namespaces, in key order:
  `Found` only when the list is loaded and every listed schema has an arena to hand out (its
  root), else `Unknown(NotLoaded)`.
- **M1. `layers`.** `SnapshotView` gains the plan's `layers` field (`Arc<Overlays>`, the
  generation's own unless `with_layers` sets others) and `layers()`. No lookup reads it
  before Task 26, which fills it in `view.rs`.
- **Minor.**
  - The publish perf line now times the real shape, `Put` plus `SetState(Loaded)`, so both
    5,000-entry maps are copied. Its p95 is 0.24 ms (max 0.73 ms), still under R-M4-2's
    1 ms.
  - The allocation test documents its limit and names the functions on the hit path that
    must stay allocation-free.
  - `publish` documents that a shared `Arc` in `Put` copies the arena.
  - `search` answers nothing for empty text and skips namespaces whose state is
    `OutOfScope`.
- **Carries.**
  - Task 15: set the schema list after each scope refresh; hand over unique arenas; call
    `announce_pending_confirmation`.
  - Task 24 and M5: map an unknown user or setting to an unnamed entry or
    `SearchPath::default()`.
  - Task 26: the `layers` seam, then re-check R-W2-6's cost for `pg_catalog`.
  - M5: the probe TTL is per namespace, so a second missing name in the same namespace within
    30 s is not probed (the plan's rule).
  - M7: `fk_referencing` answers within the scope.
  - Task 18 and M5: SQLite `temp` is per connection, so the metadata connection never marks
    a console's `temp` loaded; the session overlay owns it.
