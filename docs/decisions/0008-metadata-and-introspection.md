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

## m4-t10 — merge

M4 Task 10 fills the existence-diff merge, the catalog delta and the mass-deletion guard
(HV-INTRO-11, D-039, R-M4-22). The model spike has no merge, so nothing is copied.

- **Pure.** `merge(prev, result, policy)` reads no clock, does no I/O and never changes `prev`.
  `prev = None` is refused (`MergeError::NoBase`): a result carries no engine or fold rule, so
  a first load merges into `merge::base(key, dbms, fold)`, the namespace root alone.
- **Identity** is `ObjectKey::same_object`: the server id when both sides have one, else kind,
  exact path, overload signature and an unknown kind's raw kind. **R-W2-4 (fix round 1):** exact
  identity stays primary; on a miss, a folded fallback through `sql_ident::key::match_key`
  applies only when the name class is caseless for that engine and configuration (MySQL/MariaDB
  table and database names under `lower_case_table_names` 1/2 — the namespace's stored table
  rule — and their column, routine and event names; SQLite), the key is certain (never
  `ServerDependent`, so no non-ASCII cased letter on MySQL), and no server ids conflict. The
  server forbids case twins there, so the fallback only keeps an object whose spelling drifted;
  PostgreSQL never folds (`"Users"` and `users` are two objects). It runs in existence lookups,
  edge-10 admission and sibling matching; ancestors of a nested identity fold by the table rule.
  A `(kind, server id)` the cache holds twice (T3 m9) matches only by id **and** path, so a
  stale twin is dropped and never resurrected.
- **Deletions (C-1, I-1, R-W2-5).** Only a successful existence set of the object's own
  `ObjectKind`, for the scope it covers, deletes, at any depth and in full and incremental
  passes alike. `ExistSet::Ok(ids)` is namespace-complete for its kind; `ExistSet::Scoped {
  scope, ids }` decides a cached object only when its namespace-relative path, or an
  ancestor's, is in `scope` (a fragment of `t` reports `Table: Scoped { scope: [["t"]] }`; the
  columns of re-read `a` and `b` are `Column: Scoped { scope: [["a"], ["b"]] }`). A kind whose
  set is `Failed`, absent, or out of scope is never deleted: its cached objects are carried
  over, under re-read parents too, and a failed kind is in `MergeReport::stale_kinds`. Paths are
  those of the **next** namespace (a parent renamed by OID has its children looked up under the
  new name). Every deletion is in `delta.dropped`. **Contract for T15–T18:** a re-read that
  should remove a dropped child (column, index, constraint, trigger) reports a set of that child
  kind scoped to the re-read parents; without one, the cached children stay.
- **Changed objects** replace their cached counterpart's own properties; their children merge
  the same way: re-read children replace their cached twins (positionally when the pass returns
  them in cached order, else by identity), new ones are created, cached ones the pass did not
  return stay unless decided gone. Re-read parents keep the pass's child order, then the
  carried children; kept parents keep the cached order, then creations. Facets the pass did not
  re-read in its own props (a view's source) are invalidated; kept objects keep theirs, shared
  by reference count. An object identical but for its state mark is not altered; it takes the
  new mark. A changed child whose parent is cached is matched among that parent's children;
  one whose parent is neither cached nor re-read asks for a fragment of the parent (edge 9).
  `NamespaceResult::categories_to_reload` is not read by the merge.
- **Level (I-2).** No pass lowers a namespace's level: a full pass raises it to the pass's level
  (`max`), an incremental pass leaves it. A pass below the namespace's level erases nothing it
  did not read (its children are carried); the namespace keeps its level, and the top-level
  objects that pass created or re-read are listed in `MergeReport::below_level` (they hold only
  the pass's level; T15 elevates them, and lowering a namespace is T15's job).
- **Report.** `MergeOutcome` variants carry a `MergeReport`: stale kinds, fragments to read
  (edges 9 and 11, and a cached object the existence set lists under a new name that the pass
  did not re-read; an unknown kind keeps its raw kind), ignored changed objects (edge 10),
  duplicates (edge 12: the last wins; counted, not a debug assertion), the guard's counts
  (`checked`, `vanished`, `by_kind`), `suppressed`, `below_level`, and `steps`, a linearity
  witness.
- **Guard (I-4).** It counts **top-level** objects only (nested children never dilute it), per
  kind and over the whole top level: more than `guard_ratio_pct` % (50) of at least
  `guard_min_objects` (20) decided objects vanishing — in any one top-level kind or in all of
  them together — is `PendingConfirmation`, for both triggers; `vanished` is the top-level
  total. It is per pass, as D-039 says ("in one pass"); counting since a confirmed baseline is
  carried to T15/T22 (M-1).
- **Keep cached (I-5).** `MergePolicy::deletions` is `Apply` (default) or `Suppress`. On "Remove
  from cache" the engine publishes the pending `next`; on "Keep cached" it re-merges a clone of
  the same result with `Suppress`: the pass's creations and alterations apply, every object an
  existence set says is gone stays and is listed in `MergeReport::suppressed`, and the guard
  does not fire. The whole pass is never discarded.
- **Seam additions.** `NamespaceResult` gains `level` and `read_at_ms` (the merge reads no
  clock); both land in `NsMeta` with the new watermark. `Identity` gains `raw_kind` and
  `Identity::of`. `ExistSet::Scoped`, `MergePolicy::deletions`/`Deletions`, `KindCount`.
  `MergeError::NotImplemented` (the T1a stub variant) is removed.
- **Delta.** `created` and `dropped` name subtree roots; a child created, dropped or altered
  under an existing parent is listed itself and the parent is altered with `children`. Renames
  carry `Confidence { per_mille, identity }`: `Confidence::IDENTITY` for a rename that kept the
  object (server id, or the fold-aware fallback: a case-only rename on a caseless engine), so
  `CatalogDelta::is_hint(entry)` reads the merge's own decision instead of re-deriving it from
  the keys. Hints pair dropped and created top-level objects of one kind without server ids whose
  column signatures (name, raw type, nullability) are ≥ 90 % shared as multisets: identical
  lists are `Confidence::ONE`, others at most 999‰; above 4,096 candidate pairs per kind only
  identical lists are paired (a hash match), so a mass rename stays linear. A hint's pair is
  also in `dropped`/`created`. `CatalogDelta::combine` turns a drop and a create of the same
  kind and server id in two namespaces of one publish into one `moved` entry. A delta of more
  than 10,000 entries is coarse: `delta::event` gives `NamespaceReplaced`. `apply_delta`
  applies a delta to a namespace's top-level keys and refuses one that does not fit.
- **Cost.** O(|M| + |C| + |E|) with hash maps; no `find` per column. Two fast paths are dry
  walks of the same algorithm that fall back to the general walk when they do not apply: a full
  pass that leaves no cached object standing on its own reuses the changed arena (pass order);
  a pass with no changed object that deletes nothing reuses the cached arena with the new
  metadata (M-6). Measured on 15,000 tables / 160,000 columns in one schema (release, load ~6):
  full pass p50 28 ms / p95 31 ms, incremental (1 %) 24 / 26 ms, nothing changed 17 / 22 ms. The
  local gate is the median (R-W2-3); T33 records p95 on a quiet machine.
- **Fix round 2.**
  - **RR-1 (a):** a cached object that the same result re-reads under the same unambiguous
    server id is a rename (Present), whatever a path-scoped set says.
  - **RR-1 (b), the contract for T15b, T16 and T21:** on server-id engines a scoped set
    resolves its keys by server id, so a renamed object is reported under its current name. A
    scoped key that lists nothing is that key's drop.
  - **RR-m1:** a kept object that the existence set found under a new name, but that was not
    re-read, has its children looked up under the new name.
  - **RR-m2:** a re-read that only reorders children marks the parent `altered { children }`.
  - **RR-m3:** one existence identity vouches for one cached object. In a corrupt caseless
    cache the first fold twin takes the match and the others are dropped.
  - A dropped object is always named by its key in the previous generation, even under a
    parent renamed in the same delta.

## m4-t12 — scope and filters

M4 Task 12 fills `hacivat-introspect::{scope, filter, glob}` (alg/02 HV-INTRO-02, -03, -14;
R-M4-3, -7, -14, -41; R-W1-7). The three modules build without the `session` feature; the
`fuzz/meta` target `scope_pattern` fuzzes both grammars.

- **Scope grammar** as alg/02 HV-INTRO-02, with these readings:
  - a bare name may not hold `@` or `*` either (`app_*` is an error at the `*`, not a schema
    literally named `app_*`; quote it). The scope grammar has no globs.
  - `*` and `{regex}` never select a **system** namespace (`NsInfo::system`): the default
    PostgreSQL scope `@:*` (R-M4-14) must not introspect `pg_catalog` live (R-M4-6). A name
    or `@` selects one.
  - A PostgreSQL database-only clause is `db:@`: the schema named like the current one.
  - The implicit `@:@` is part of `select` only when the server lists it;
    `select_explicit` is the pattern alone (edge 9: resolvable, not shown).
  - Names compare through `match_key` and `FoldRule`; where `match_key` answers
    `ServerDependent` (R-M4-41) only the exact text matches. `CurrentCtx` gains
    `ident: Option<IdentCtx>` for `lower_case_table_names`.
  - A misplaced `{` that never closes is `UnclosedBrace` at the brace (`db{1:x` → 3).
    `ScopePattern` equality is dialect + syntax tree; parse warnings (edge 11
    `DbTermIgnored`) are data for Task 20's editor, which adds their copy.
- **Filter grammar** as alg/02 HV-INTRO-03. Kind tokens are `ObjectKind` ids (`_` for `-`)
  plus `routine`/`function`/`procedure` (functions, procedures, aggregates), `mview`, and
  `table` covering partitions; a token the dialect's kind schema lacks, or any `word:` prefix
  that is not a kind, is `UnknownKind`. Items are trimmed. `routine:sig:` items match a
  signature given to `matches_signature`; `matches` skips them. `excludes(kind, name)` is the
  `ScopeFilter` answer (unsure → `true`; a view filter → `false`); `applies_to` keeps scope
  filters off system namespaces (edge 10).
- **Case.** Literals, globs and regexes follow `FoldRule::for_names` for the kind's name class
  (`with_ident` sets `lower_case_table_names`): exact, or **ASCII**-caseless. Globs translate
  ASCII letters to `[aA]`; a user regex is rewritten the same way (`glob::ascii_caseless`:
  escapes, group heads, counted repetitions and ASCII classes untouched; class letters and
  plain ranges gain their other case in a nested class). The `regex` crate's own `(?i)` folds
  Unicode (`k` ↔ U+212A), which the server rule does not; it is only the fallback for text the
  rewrite does not follow (a verbose-mode comment holding `[`). Class escapes (`\p{Lu}`) stay
  case-sensitive (documented limit).
- **Regexes** are compiled once, at parse, with 1 MiB size limits, after compiling the text
  alone (balanced groups), as `\A(?:…)\z`; a verbose-mode comment that swallows the suffix is
  retried with a newline.
- **Pushdown: binds only.** `SqlPredicate.sql_fragment` is the fixed pack text
  `LIKE :name_like ESCAPE :like_escape`; `binds` holds one `LIKE` pattern per distinct
  positive glob. Task 4's lexer refuses a backslash inside a literal (and MySQL would read
  `'\'` as an open literal), so the escape character `\` travels as the `like_escape` bind
  (`SqlPredicate::escape()`). Positive items are OR'ed, which no pack construct expresses
  (a list bind is `<col> IN :[x]` only, R-W1-7), so a section runs **once per pattern**
  (`SqlPredicate::runs()`), unioned like list-bind chunks; the fragment sits beside a list
  bind as another top-level `AND` conjunct (tested through `DraftPack`). Eligibility:
  scope role only; PostgreSQL exact classes (a non-ASCII pattern only on a UTF-8 server);
  MySQL/MariaDB table and database classes, no character outside the BMP; SQLite ASCII
  patterns; nothing on another dialect. None when any applicable positive item is a regex, a
  signature item or `*`, when more than 200 apply, or when a pattern holds NUL.
- **Carried.** Task 17: confirm `LIKE ? ESCAPE ?` prepares on MySQL 8.0/8.4 and MariaDB
  (Inferred; no Docker in W2) and that `INFORMATION_SCHEMA` table names compare as this rule
  assumes under `lower_case_table_names` 0/1/2. Task 16: `relname LIKE $n ESCAPE $m` resolves
  in the `C` collation. Task 15/24: the `ScopeFilter` adapter keyed by namespace, warnings
  shown by Task 20.
- **Fix round 1 (security review: 3 Important, 8 Minor).** The rule behind each fix: **the
  server never returns fewer objects than the client filter keeps.**
  - **I1:** on PostgreSQL, unless `IdentCtx.encoding` is known to be UTF-8 (`SQL_ASCII`,
    single-byte or unknown), `?` and `[…]` push as `%`, not `_` (`Glob::like_pattern_with`):
    a byte-wise server's `_` is one byte, and a UTF-8 name stored there can take several.
  - **I2:** `CompiledFilter::pushdown_kinds(&[ObjectKind], …)` returns the union of the
    patterns only when **every** kind in the set has its own pushable positive item.
    `pushdown(kind)` is the one-kind case. **A catalog section whose rows can be of several
    kinds (PostgreSQL `pg_class`: tables, partitions, views, materialized views, sequences,
    foreign tables; MySQL `TABLES`: tables and views) must call `pushdown_kinds` with all of
    them** (carry to T16–T18). The 200-pattern cap counts the union.
  - **I3:** `@` and the implicit `@:@` compare with the listed names through `FoldRule` (MySQL
    and MariaDB `lower_case_table_names` 1 and 2, SQLite), exactly where no rule is pinned, and
    the **listed** key is selected, never the session text. T17 confirms on Docker what
    `DATABASE()` returns under lctn 2.
  - **M1:** untyped items never apply to `Root`, `Database` or `Schema`; MySQL pushdown covers
    the table name class only.
  - **M2:** pushdown refuses an `ident` whose fold rule for the kind differs from the filter's
    own (`with_ident`).
  - **M3:** a pattern over `MAX_PUSHDOWN_PATTERN_BYTES` (4,096; SQLite's limit is 50,000) is not
    pushed.
  - **M4:** in a negative clause `*` and a regex reach system namespaces (removing is safe).
  - **M5:** the 200-run cap is per call (one section, one namespace); the total per profile is
    the engine's (T15) to bound.
  - **M6 (carry to T18):** the SQLite reader must not run pushed sections on a connection with
    `PRAGMA case_sensitive_like` on.
  - **M7:** under MySQL/MariaDB `lower_case_table_names = 1` the pattern is sent
    ASCII-lowercased (names are stored lower-case).
  - **M8:** an explicit `(?-i)` / `(?-i:…)` in a user regex keeps its letters exact on
    caseless engines (`ascii_caseless` follows the flag per group). The seams test is renamed
    `scope_selects_the_current_and_every_user_schema`. The reviewer's hostile tests are
    `tests/scope_hostile.rs`.

## m4-t19 — DDL producers

M4 Task 19 fills `hacivat-ddl`'s view-only producers (alg/03 HV-DDL-07 producers) and the
script text form. The bodies are adapted from the DDL spike (`da358e4`, R-M4-40, U-1) to the
merged model, `ModelView` and the T7 renderers, and reviewed as new code.

- **A sources seam beside the view.** The model keeps only `SourceRef`s; the text lives in the
  cache's content-addressed store, and grants are a facet (R-M4-30). Neither is reachable
  through `ModelView`, so the producer takes a `DdlSources` (`text(SourceRef)`, the `Sources`
  facet `server_text(ObjectKey)`, the `Grants` facet `grants(ObjectKey) -> Facet<GrantsFacet>`,
  each defaulting to "not loaded"). `create_script` keeps Task 1a's signature and runs with
  `NoSources`; `create_script_with` is the DDL view's entry point (Task 25).
- **Only what the view vouches for.** An object is located through `ModelView::children`
  (the schema list for a schema) and used only when the answer is `Found`, i.e. from a
  namespace loaded at `Structure` or above. Everything else, a missing definition text, a
  `LOSSY_NAME`, or props of the wrong shape, is `DdlError::NotLoaded`, never an empty or
  partial `CREATE`. `ObjectKind::Unknown` named directly is `DdlError::UnsupportedKind`; inside
  an expanded schema it is left out with `ddl-unsupported-kind`. A NUL anywhere in the output
  is refused. A dialect with no renderer (SQL Server, ClickHouse, Oracle, the PG-wire forks)
  is `UnsupportedKind`.
- **Server text, verbatim** (R-M4-24): views, materialized views, routines and triggers; a
  PostgreSQL index with an expression, a predicate or `INCLUDE` when the `Sources` facet holds
  `pg_get_indexdef` (else synthesized; operator classes and collations are T13 fields);
  MySQL/MariaDB tables under `server_definitions` (`SHOW CREATE TABLE`, `NotLoaded` without
  it). **SQLite tables, indexes, views, triggers and virtual tables are always their
  `sqlite_schema.sql` text** (T7 carry M8 settled per dialect: `server_definitions` is on for
  SQLite whatever `GenOptions` says; placement other than `InsideTable` raises
  `ddl-sqlite-placement-forced`).
- **IR gaps handled in the producer** (T7 carries; the IR is Task 1a/7's):
  `ALTER SEQUENCE … OWNED BY` is a producer-rendered statement after the owning table; MySQL
  secondary indexes are `CREATE INDEX` after the table (no `KEY` element in the IR), with an
  FK moved to `ALTER TABLE … ADD` after an explicit index of the same name (MySQL would create
  the FK's own index under that name); a MySQL unique key on column prefixes is
  `CREATE UNIQUE INDEX` from its backing index (T13 m4); a MySQL `INVISIBLE` (MariaDB
  `IGNORED`) index gets the option word after the rendered statement; a MySQL `INVISIBLE`
  column carries the attribute after its type. The script's MySQL file form uses the custom
  delimiter the renderer chose (`DelimStyle::MySqlDelimiter`, T7 carry M3).
- **Names.** A server-generated constraint name (`SERVER_GENERATED_NAME`) is omitted unless
  `surrogate_names`; MySQL's `PRIMARY` never. `Auto` qualification asks the view's schema list
  whether a name exists in more than one schema of its database; when the list is not known
  the name is qualified (the safe side). A PostgreSQL temporary table is never qualified.
- **Order** is deterministic (BTree maps, arena order, name bytes): schemas, types (enum,
  range, domain, composite), sequences, routines, tables in FK and partition-parent order
  (a cycle broken at the smallest remaining table, its FK postponed past both tables), views,
  materialized views, standalone indexes, triggers and constraints.
- **Gaps recorded, not hidden:** an `INHERITS` child is `NotLoaded` (the model holds no parent
  list, and dropping inherited columns without `INHERITS` would describe another table);
  MySQL partitioning (`CREATE_OPTIONS partitioned`) raises `ddl-unsupported-kind`; (NOT
  ENFORCED: see fix round 1 below) PG 18 named NOT NULL constraints, MySQL table charset and per-part
  `NULLS FIRST`/operator class/collation and exclusion deferral (T13's R-W2-8 fields) are not
  written yet; `with_options` is unsupported (T13 carry).
- **Fix round 1 (opus review: a script never states a wrong fact silently; it is exact, carries
  a notice, or answers `NotLoaded`):**
  - **I1:** a MySQL/MariaDB unique key whose backing index has prefixes, descending parts,
    `INVISIBLE`, a comment or a method other than `BTREE` is `CREATE UNIQUE INDEX` from that
    index; such a primary key is `ALTER TABLE … ADD PRIMARY KEY (…) [USING HASH] [COMMENT …]`
    after the table. If an `AUTO_INCREMENT` column's only key needs that route, the script is
    refused (`Render(Invalid)`), since MySQL wants the key at `CREATE TABLE`.
  - **I2:** `KeyProps`/`CheckProps` `Default` is now **enforced** (named exception in
    `hacivat-meta/src/props.rs`, fenced `m4-t19-produce`; field name and codec unchanged, T11
    needs nothing; T13 adds `rare: None` to the two impls). A constraint the server does not
    enforce is `ALTER TABLE … ADD … NOT ENFORCED` where the target has it (MySQL 8.0.16+
    checks; PostgreSQL 18+ checks and foreign keys), else left out with
    `ddl-warn-not-enforced`. Introspector contract (T16/T17): `enforced` is `false` only for
    a constraint the server reports as not enforced.
  - **I3:** a disabled PostgreSQL trigger (`TriggerProps.enabled = false`) is followed by
    `ALTER TABLE … DISABLE TRIGGER …`, on PostgreSQL only. T16 must fill `enabled` from
    `tgenabled` (the derived default is `false`). `ENABLE REPLICA`/`ALWAYS` have no field yet.
  - **I4:** `ColumnDef.invisible` (named exception in T7's `ir.rs`, `render/{mysql,pg,sqlite}.rs`
    and `tests/render_ir.rs`, fenced): MySQL writes `INVISIBLE` after the type, any generated
    clause, `ON UPDATE` and `AUTO_INCREMENT`; PostgreSQL and SQLite refuse it. `on_update_of`
    keeps one token, so a trailing `INVISIBLE` in `EXTRA` is not swallowed.
  - **I5:** `ddl-warn-unknown-type` (named exception in m4-t01a's FTL DDL block, fenced
    `## m4-t19-produce`, TR `# Review: PO`): a PostgreSQL column whose `type_ref` is outside
    `pg_catalog`/`information_schema` and is neither a type nor a relation the view holds.
  - **Minors:** a `sqlite_autoindex_*` name is implicit on SQLite only; a PG/MySQL table with
    `Props::None`, or a partitioned parent without its key, is `NotLoaded`; MariaDB has no
    functional key parts (such an index is left out with `ddl-unsupported-kind`).
  - **Recorded gaps (carries):** identity sequence options and name, PG column storage and
    compression, view `security_invoker`/`security_barrier` (M7); dependency order for
    whole-schema scripts (T31, M7); `server_text` has no load state (T25, T22); revoked
    default privileges (M7); per-part opclass/collation/NULLS FIRST (T13).

## m4-t11 — catalog cache

M4 Task 11 fills `codec`, `fingerprint` and `persist/**` (HV-MODEL-03, R-M4-15, R-M4-25,
R-M4-26, D-016). The codec and the store's open, rotation and bad-row logic come from the model
spike (`d86313b`); encryption, the identity check, compare-and-set, debounce and eviction are
new.

- **Codec.** The spike's envelope (`"HVNS" major minor compression checksum body`, XXH3-64
  over the body, every record and union value length-prefixed, per-namespace string table),
  extended to the M4 types. Choices beyond the spike:
  - The namespace header lists the machine id of every kind code it uses. A code this build
    does not know decodes as `Unknown` with that id as its raw kind, so it matches the
    `ObjectKey::raw_kind` a newer build wrote into user data (T3 review m6). Our own `Unknown`
    objects keep their server text.
  - **Strictness:** a payload of this build's minor or an older one must use every byte of
    every record and of the body; a newer minor's trailing fields, tags and sections are
    skipped.
  - A namespace read from a **newer minor** loads with `Watermark::None` (its next pass is
    full, never incremental on top of fields this build dropped), is reported
    (`Decoded::from_newer_minor`, `LoadAll::from_newer_minor`) and is never saved back:
    `NamespaceSnapshot` has no flag for it (T3's file), so the store remembers the metadata it
    read and refuses a save of a namespace with exactly that metadata. A fresh introspection
    has new metadata and replaces the row with our minor (alg/02 edge 4).
  - Untrusted input: no recursion on input depth, and every count is capped by the bytes left
    divided by the smallest encoding of one element, so a corrupt count cannot allocate more
    than a small constant times the input. Equal empty strings share one allocation.
  - **Decode CPU (fix round 1, B-I3).** A name (object name, namespace or reference path
    element) is at most `MAX_NAME_BYTES` = 4 KiB: the engines' identifier limits are 63 bytes
    (PostgreSQL), 64 characters (MySQL/MariaDB; 256 bytes in utf8mb4) and 128 characters
    (SQL Server, Oracle; 512 bytes), so 4 KiB is headroom for any engine in any encoding;
    SQLite has no limit, and a SQLite name over the cap is simply not cached. The string bytes
    a payload expands to through its table are capped at 8 × body + 1 MiB (a real catalog
    expands to under 1×). The encoder enforces both caps, so whatever it writes can be read.
  - `ColumnProps::ext`: `Some(ColumnExt::None)` has its own tag, so the round trip is exact.
  - `reseal_checksum` is public: tests and the fuzz target frame arbitrary bodies with it. The
    checksum guards against accidents only; the header's version bytes are outside it by
    format (a flipped minor reads as a newer minor and is never rewritten).
  - `codec::testing` (feature `test-support`) encodes and decodes as another minor would, so
    the forward and backward rules are tested while this build is at 1.0.
- **Store.** One SQLite file per profile and major. Tables `header`, `namespace` (tag, key,
  minor, generation, writer, epoch, watermark, payload), `usage` (the LRU clock, kept apart so
  a load never rewrites a payload row) and `source` (tag, text).
  - **The schema is untrusted (fix round 1, B-I1).** Every connection sets
    `SQLITE_DBCONFIG_DEFENSIVE` and `trusted_schema = OFF`. Before any other read or write
    (after `quick_check`), `sqlite_master` must equal exactly this build's objects: the four
    tables with their exact `CREATE` text and the four primary-key autoindexes, and no
    trigger, view or other index; each table's `pragma_table_info` must match too. The header
    table is checked first and its `format_major` must be an integer: another major is
    `MajorChanged`, a missing or wrongly typed value `Corrupt`. Any other difference is
    `Corrupt`: the file is rotated and rebuilt, never read further. A planted trigger cannot
    pin a row or hang a load, and a wrong-shaped table cannot disable the cache.
  - **And it stays checked (fix round 2, N1).** The check runs inside one read transaction
    that records `PRAGMA schema_version`. Every later transaction of the store — reads in a
    read transaction, writes under `BEGIN IMMEDIATE` — reads the cookie as its first
    statement, before any DML or `SELECT`; within that transaction the snapshot (or the
    write lock) holds it. A changed cookie (a trigger, view or table altered while the store
    is open) abandons the transaction: the file is rotated and rebuilt and the call answers
    `CacheError::Corrupt { renamed_to }`, so a planted trigger never runs. The SQLite
    authorizer (`rusqlite` feature `hooks`) was not needed: the cookie check closes R1 (pinned
    row) and R2 (hang) deterministically.
  - **Read limits (B-M3; fix round 2, N2, r-m1).** A load reads rows most recently used
    first, at most the per-profile cap (`CacheStore::set_caps`, default 512 MiB) in all and
    no row over `MAX_ROW_BYTES` (256 MiB); the rest are dropped as `RowError::TooLarge`. Every
    cell that can be large is selected through `CASE WHEN length(x) <= cap THEN x END`
    (`load_all` and `load_namespace` payloads; keys and tags under `MAX_KEY_BYTES` = 128 KiB;
    watermarks, header values and source tags under 64 KiB), so an oversized cell is never
    materialized; rows are deleted by `rowid`. In a plaintext store the key cell must equal
    its tag byte for byte before it is decoded, and `decode_ns_key` takes at most
    `MAX_NS_PATH` = 16 path elements.
  - **Files (A-m3/B-M4, B-M5).** The leaf directory is tightened to 0700. Opening, preparing
    or scrubbing a file compares its device and inode before and after the open and requires
    one link; a hard-linked cache file is refused at open, and a hard-linked name is only
    unlinked when scrubbed, never written through.
  - WAL, `synchronous=NORMAL`, `temp_store=MEMORY` (no temp file holds plaintext),
    `secure_delete=ON` (a replaced or evicted row leaves zeros) and
    `auto_vacuum=INCREMENTAL` (an eviction returns pages).
  - Directory 0700 (the rule of `fsio::create_dir_all_private`, which this crate cannot call),
    file and sidecars 0600.
  - Symbolic links: a cache path, `-wal` or `-shm` that is a link is refused; a rotation
    renames the link itself; a deletion removes the link only. Not `SQLITE_OPEN_NOFOLLOW`:
    it refuses a link in any path component, and data directories sit behind one (macOS
    `/var`).
  - A file that is not a database, or fails `quick_check`, is rotated to
    `*.corrupt-<secs>[-n]` (`Corrupt`). A file of another major is read for its major
    **before** anything is changed and is rotated byte for byte (`MajorChanged`; FTL
    `incompatible`). A plaintext file opened with a key, or one the key's check value does
    not open, is overwritten with zeros, deleted and rebuilt with its rotations (`WrongKey`).
    An encrypted file opened without a key is refused (see Encryption below).
  - A row that does not decode, authenticate or agree with itself (key, tag, minor) is deleted
    and reported (`LoadAll::dropped`, `CacheError::Row`); only that namespace is
    re-introspected.
- **Encryption (R-M4-15).** `RowCipher`: XChaCha20-Poly1305, a fresh 24-byte nonce from the
  OS RNG per seal (an RNG failure is an error, never a fallback nonce), associated data
  `len(domain) as u16le ‖ domain ‖ ad`. The profile key seals nothing itself. (Fix round 1
  as below; the first version derived subkeys from XChaCha20 keystream and tagged
  `PRF(XXH3-128(input))`.)
  - **Ruling R-W2-7 (fix round 1): HMAC-SHA256.** Subkeys are derived HKDF-expand style,
    `HMAC-SHA256(profile key, "hv-rowcipher-v1" ‖ 0 ‖ purpose ‖ len(domain) u16le ‖ domain ‖
    1)`, one for sealing and one for lookup tags. Tags are
    `HMAC-SHA256(lookup subkey, input)[..16]`, so crafted names cannot collide. A derivation
    failure is an error (`RowCipher::new` returns a `Result`), never a fallback key.
    `hmac` 0.12.1 and `sha2` 0.10.9 join `hacivat-meta`'s `persist` feature by a
    controller-granted named exception to "only Task 1a edits manifests"; the root
    `Cargo.lock` gains only dependency edges. The HMAC crate's transient inner state is not
    zeroized on drop.
  - `RowKey::from_bytes` takes `&[u8; 32]`, so no by-value copy of the key is left on the
    caller's stack.
  - Sealed: payloads (`ad = 1 ‖ minor ‖ generation ‖ writer ‖ epoch ‖ ns_key`), namespace
    keys (`2 ‖ tag`), watermarks (`3 ‖ generation ‖ ns_key`), source texts (`4 ‖ hash ‖
    len`), the key check (`5`) and the server fingerprint with its epoch (`6`). A row cannot
    move between namespaces, row types or stores, and its bookkeeping columns cannot be
    edited.
  - **Padding (fix round 1, A-m2/B-I2).** Before sealing, every plaintext is padded
    (`data ‖ 0x80 ‖ 0…`) to a Padmé size with a floor: namespace keys 256 bytes (so every
    key of an ordinary name seals to one length), watermarks 128, sources 256, payloads
    4096. The clear `source.len` column is gone. A ciphertext length shows only its bucket
    (Padmé leaks O(log log n) bits, at most about 12 % overhead); two profiles over the same
    database still have the same bucketed length profile, since their content is the same.
  - In the clear: the minor, generation, writer id, epoch, the LRU counter, the row count
    and the bucketed lengths. They hold no name; the bucketed lengths and counts still show
    the rough size of the catalog and how often it is written and used (this line replaces
    the first version's "they name nothing", which overlooked the exact name lengths).
  - **Staleness is an epoch, bound into each row (A-m4/B-M1).** The header holds the
    server fingerprint and an epoch, sealed together. `check_identity` starts a new epoch
    when the server changes; a row is stale when its epoch differs from the header's. The
    epoch is in the row's associated data, so a stale row cannot be made fresh by editing a
    column (the first version's clear `stale` flag is gone).
  - **Rollback and replay (accepted, documented; R5).** Someone who can write the file can
    still put back a complete older row of the same namespace (every column, as it was
    written), an older sealed `server` header (an older fingerprint and epoch), or an older
    copy of the whole file; each authenticates and loads. The file alone cannot tell old
    from current: preventing that needs state outside the file. It is mitigated by every
    cache load being published `Loaded { stale: true }` and validated in the background
    (HV-INTRO-15) before anything trusts it as current; a replayed `server` header is caught
    by the next `check_identity` against the live server, which starts an epoch past every
    row's and the header's, so no row is made fresh by it (review A, H10). A partial rollback
    (the sealed columns without their bookkeeping) fails authentication.
  - **Downgrade (fix round 1, A-I1).** An encrypted file opened without a key is refused
    (`CacheError::Disabled`, memory-only) and left untouched; only a plaintext file opened
    with a key, a wrong key, or a file of unrecorded mode with rows, is scrubbed and rebuilt,
    together with its `.corrupt-*` rotations (A-m6/B-M6). Turning encryption off deletes the
    files first (Task 20).
  - **Key.** `SecretStore::catalog_key(profile, create)` (named lines in
    `hacivat-secrets/src/store.rs`, block `m4-t11-catalog-key`): reads the slot; with
    `create`, a missing or damaged key is replaced by 32 random bytes, stored hex-encoded in a
    `SecretString` with retention `Forever`. A put the backend refuses (a fallback to the
    process) is undone and returned as an error: a key that dies with the process would make
    the next start's cache unreadable, so the hub runs that profile memory-only (edge 9).
    Every buffer that holds the key is `Zeroizing`. `hacivat-meta` never calls it.
- **Identity (step 2).** The first `check_identity` records the fingerprint (epoch 0).
  Another server starts a new epoch, so every row saved before is stale; a stale row loads
  with `Watermark::None` (its next pass is full), the rows stay for offline use, and a fresh
  save (under the new epoch) is current again.
- **Compare-and-set (step 5).** A save may replace another writer's row only with a watermark
  that is not older for the same server: PostgreSQL by `system_identifier` then `w`, MySQL
  by `server_uuid` then time, SQLite by file identity then `schema_version`. A watermark of
  another server is not comparable and wins; a save without a watermark never replaces one
  that has it (`CacheError::Superseded`). A writer always replaces its own row.
- **Debounce (step 4).** `SaveDebouncer`: a namespace is due one second after its **first**
  pending offer, with the latest snapshot offered by then, so a namespace that keeps changing
  is still saved once a second. Pure logic over caller instants; the hub drives it.
- **Eviction (R-M4-25).** `CacheStore::evict(cap, protect)`: by logical row bytes, unreadable
  rows first, then least recently loaded; `protect` (the current database) is never evicted.
  `evict_global`: whole caches of other profiles by newest file modification time (a load
  writes the `usage` table, so it touches the `-wal`), never a profile in `keep`.
  `CacheCaps::from_mib` clamps the total up to the per-profile cap (T1c concern 5).
- **Disk full (edge 2).** `CacheError::Full { warn }`: `warn` is true for a store's first
  such failure only; the previous row stays. The notice string `model-cache-write-failed`
  (alg/02) is not in the FTL yet; the hub's task adds it.
- **Fingerprints.** XXH3-128 Merkle hashes: a node's canonical record (the codec's encoding
  with strings in place, no server ids — also inside references — and no state marks) plus
  the sorted hashes of its children. The order a server lists children in does not matter;
  a column's ordinal does. Statistics live in a facet, never in an `Obj`. Iterative, linear
  in the subtree. The value is locale-free and pinned in `tests/fingerprint.rs`.
- **Faults.** `Fault::AbortBeforeCommit` (feature `test-support`) aborts the process after
  the row is written and before `COMMIT` returns; the test runs it in a child process. The
  workspace's `rusqlite` has no `hooks` feature, so a hook inside `COMMIT` itself is not
  available; in WAL mode the uncommitted frames are ignored either way. A full disk is
  simulated with `PRAGMA max_page_count` (SQLite's own `SQLITE_FULL` path).
- **Seam changes (T1a stubs filled).** `DecodeError::NotImplemented` is gone and
  `DecodeError::Model` added; `CacheError` gains `Sqlite`, `Superseded`, `Full`, `Row`,
  `Crypt`, `Encode`; `CryptError::NotImplemented` becomes `Rng` and `TooLong`;
  `RowCipher::seal` returns a `Result`; `Opened` gains `moved_to`; `LoadAll::dropped` holds
  `(Option<NsKey>, RowError)` (an encrypted row's key may be unreadable) and `LoadAll` gains
  `from_newer_minor` and `into_changes` (each namespace put with `Loaded { stale: true }`);
  `load_namespace`/`load_all` take `&mut self` (they move the LRU clock and delete bad rows).
  Named exceptions: `tests/model.rs` (`props_round_trip…` becomes the round trip) and
  `tests/seams.rs` (`stub_cache_is_memory_only` now checks that an open that cannot create
  its file writes nothing).
- **Not in this task.** Garbage collection of other majors' files after 30 days (alg/02
  "Versioning") is not in the plan's list; cache rows stay uncompressed (the compression byte
  is reserved, R-M4-26).
- **Carried by the controller (fix round 1).** XXH3-128 fingerprints and `SourceRef`s are not
  collision-resistant (A-m8/B-M2) → M7 pre-flight. Two first-time creators of a profile's
  `CatalogKey` can race (A-m9/B-M7) → Task 15 creates the key once, under the hub. Task 15
  never opens a sensitive profile with `Encryption::None` (now also refused by the store).
  The `model-cache-write-failed` message → Task 24.

## m4-t13 — assembly

- **Seam.** `hacivat_introspect::assemble` gains the row and result types: `TableId` (server id
  and name), `ColRef` (an `(ordinal, name)` column reference), `ColumnIndex` (the namespace's
  columns per table, built by the introspector; `resolve`, `resolve_table`, `set_complete`),
  `KeyObj` (the owning table and the model `Obj`), and one adapter per dialect,
  `assemble::{pg, mysql, sqlite}::assemble_keys(rows, &ColumnIndex) -> Vec<KeyObj>`. Assembly
  cannot fail, so the T1c stub's `AssembleError` (referenced nowhere) is removed: a part that
  cannot be resolved flags the object `BROKEN` instead.
- **Column references** resolve by ordinal, then by name: exactly, then by a certain
  ASCII-caseless key where the engine folds the class (MySQL/MariaDB columns, SQLite;
  `match_key`, so a non-ASCII cased name stays exact, R-M4-41). An ordinal whose column has
  another name than the row says is an inconsistent read and does not resolve. Key order is the
  row order, never ordinal or name order. An unresolved reference keeps the row's text and the
  object is kept with `ObjFlags::BROKEN` (alg/02 HV-INTRO-13 step 3).
- **FK targets (fix round 1, I3, m6).** A target that resolves in this namespace is named by the
  table it resolved to (stored spelling and server id; `ColumnIndex::resolve_table`), never by
  the `REFERENCES` spelling: SQLite `REFERENCES P` against table `p`, MySQL under
  `lower_case_table_names` ≥ 1. `locate_ref` and `fk_referencing` match names exactly, so the
  spelling would read as a dangling FK. A target outside the namespace keeps the row's names and
  is resolved on demand. A target inside the namespace that the index does not hold is broken
  when the index is complete (`set_complete(true)`, a full pass) and only unknown otherwise (a
  fragment).
- **Expression parts** are `KeyColumn { ordinal: 0, name: None }` slots in `KeyProps::columns`
  (PostgreSQL's `indkey` convention); their raw text is in `KeyProps::expressions` in slot
  order: PG `pg_get_indexdef(oid, k, true)`, MySQL `EXPRESSION`, SQLite the part's text from
  `sqlite_schema.sql` (a quote-, bracket- and comment-aware scan; the part's `COLLATE` and
  `ASC`/`DESC` are not part of it, as in PostgreSQL's per-column form). The scan is used only
  when its part count equals the index's key count (m2). A SQLite partial index's predicate
  comes from the same scan; a partial index whose text yields none is broken.
- **R-W2-8: rare parts (named exception in `hacivat-meta/src/props.rs`, fenced
  `// ## m4-t13-assemble`).** `KeyProps.rare: Option<Arc<KeyRare>>` with per-part vectors
  parallel to `desc`, each empty when no part has the option: `nulls_first` (PG `indoption`
  bit 1, raw), `opclass` (PG `indclass` when not the type's default, schema-qualified outside
  `pg_catalog`, with parameters), `collation` (PG `indcollation` when it differs from the
  column's; SQLite `index_xinfo.coll` when it differs from the column's declared one, the
  comparison Task 18's). `FkProps.rare: Option<Arc<FkRare>>` with `fk_ref_index` (PG `conindid`,
  MySQL `UNIQUE_CONSTRAINT_NAME`, an `Index` ref on the referenced table). `CheckProps.rare:
  Option<Arc<CheckRare>>` with `no_inherit` (PG `connoinherit`), `excl_ops` (PG `conexclop`),
  `excl_deferrable` and `excl_initially_deferred`. The rare part is `None` when nothing is set.
  Accessors: `KeyProps::{nulls_first, opclass, collation}`, `FkProps::fk_ref_index`,
  `CheckProps::{no_inherit, excl_ops}`. `with_options` (index `reloptions`, MySQL `WITH PARSER`)
  is deferred; T19 lists it as not reproduced. The four `FkProps` literals of other tasks
  (`fixture.rs`, `tests/model.rs` ×2, `tests/snapshot.rs`) gain `rare: None` (named
  exceptions). The cache codec encodes the rare parts after Task 11 merges (format minor bump,
  a named exception in `codec.rs`).
- **Implicit indexes (A-11).** The index behind a constraint is `ObjFlags::IMPLICIT` with
  `IndexProps.key.implicit_for` pointing at it (kind, path `[…, table, constraint]`, server id),
  and the constraint's `backing_index` names the index, also when the names agree (as the
  T3/T9 fixture does): PostgreSQL PK/UNIQUE/EXCLUDE through `conindid` (an FK's `conindid`
  names the referenced table's index and becomes `fk_ref_index`, never an implicit link);
  MySQL/MariaDB `PRIMARY` and every unique index (`TABLE_CONSTRAINTS` lists each as a `UNIQUE`
  constraint of the same name, so both a key and its implicit index are emitted); SQLite
  autoindexes of `origin` `pk` and `u`. The InnoDB FK-created index is implicit only under
  `mysql::FkIndexRule::NamedAfterConstraint` (a non-unique index named like an FK of its table
  whose leading columns are the FK's); the default `None` marks nothing, so DDL shows an
  explicit index MySQL accepts. Task 17's probe picks the rule.
- **PostgreSQL keys** take `INCLUDE`, `indnullsnotdistinct` and the per-part options from their
  backing index. A PK or UNIQUE whose backing index row is not in the pass is incomplete and
  `BROKEN` (m1): Task 16 reads a constraint's index in the same pass. `NOT VALID` constraints
  and invalid indexes get `ObjFlags::INVALID`. A constraint that is not local (`conislocal`) or
  was cloned from a partitioned parent (`conparentid`) gets `ObjFlags::INHERITED` (DDL does not
  repeat it). `contype` `t` and `n` are not assembled. Carry to T16: the attached partition
  index (`pg_inherits` of an index) has no row field yet.
- **PostgreSQL CHECK and EXCLUDE text (fix round 1, I2).** `CheckProps.expr` is the bare
  expression the DDL producer wraps (`CHECK (expr)`, `EXCLUDE expr`): a CHECK stores
  `pg_get_expr(conbin, conrelid, true)` (`ConstraintRow.check_expr`; none → broken, empty
  text); an EXCLUDE stores the `pg_get_constraintdef` text after `EXCLUDE `, without the
  `DEFERRABLE` / `INITIALLY DEFERRED` words the server appends after the last `)`, its deferral
  and operators in `CheckRare`; an operator count that differs from the key's, or a text that
  does not start with `EXCLUDE `, is broken. Carry to T19: `ir.rs`'s `ConstraintBody::Exclusion
  { raw }` comment should say "the text after `EXCLUDE`".
- **MySQL/MariaDB.** Key parts arrive one row each and are grouped by `(table, index)` in
  `SEQ_IN_INDEX` order; a gap or repeat is a missing part (broken). `COLLATION = 'D'` is
  descending, as the server reports it (F9). FK columns group by `(table, constraint)` in
  `ORDINAL_POSITION` order, referenced columns by position. CHECK clauses join by name on MySQL
  (unique per schema) and by `(TABLE_NAME, CONSTRAINT_NAME)` on MariaDB; a check with no single
  matching clause is kept, broken, with an empty expression. `PRIMARY` is flagged
  `SERVER_GENERATED_NAME` (the server's own name). Carry to T19 (m4): `KeyProps` has no prefix
  lengths, so a MySQL key renders its parts through its backing index
  (`IndexProps::prefix_len`); a `UNIQUE KEY (body(32))` from the key alone is invalid DDL.
- **SQLite names (fix round 1, I1).** The primary key is `<table>_pk`. Each foreign key's
  display name is derived from its **content**, never its position, so dropping or adding one FK
  (a table rebuild) leaves the other FKs' identities alone:
  `<table>_<from columns>_<parent>_fkey`, the parent being the stored table it resolved to; when
  two FKs of the table share that name (same columns and parent), both take
  `<table>_<from columns>_<parent>_<to columns, or pk>_fkey`; a name still taken (exact
  duplicates, which SQLite allows) gets `_2`, `_3`, … in declaration order (SQLite numbers `id`
  from the last declared FK; checked on SQLite 3.51); a name over 128 bytes is
  `<table>_fk_<xxh3-64 of from, parent and to, 16 hex>`. All are `SERVER_GENERATED_NAME`, so DDL
  never emits them. Residual: adding a second FK with the same columns and parent renames the
  first (it gains its `to` columns). Deviation from the plan's "FKs with no name": siblings with
  one empty name share an identity, and a re-read matched sibling by sibling keeps only the last
  (merge edge 12). A `to` of NULL references the parent's primary key and leaves
  `target_columns` empty (not broken). FK deferral (`FkRow.deferrable`, `initially_deferred`)
  is not in `foreign_key_list`: Task 18 parses it from `sqlite_schema.sql` (m3).
- **SQLite autoindex names are positional (m5).** `sqlite_autoindex_<table>_<N>` is the server's
  own name, numbered by position, so a rebuild that drops an earlier UNIQUE renames the later
  autoindexes and their unique keys. It is kept as the server reports it.
- **Existence and order.** An object exists when its own catalog row does; part rows without
  one are ignored (the next pass reads both). Output is sorted by table, then name, kind and
  server id, whatever the row order.
- **Cache format minor 1 (codec round, after T11; named exception in `codec.rs`).**
  `FORMAT_MINOR` is 1. The rare parts are one optional record appended at the end of the key,
  foreign-key and check records, written **only when set**, so an object without one encodes to
  the same bytes, and the same fingerprint, as in minor 0. A record that ends before the field
  (a minor-0 payload) reads `rare: None`; no migration step is needed. The new names keep
  T11's caps: operator classes, collations and exclusion operators are names (at most
  `MAX_NAME_BYTES`, refused by the encoder and the decoder); every string counts against the
  expansion budget; `fk_ref_index` is an ordinary reference. The encoder knows the minor it
  writes (`Enc::minor`), so the test hook writes a genuine minor-0 payload. Tests:
  `tests/codec_rare.rs` (round trip, an older-minor payload, unchanged bytes without rare
  parts, an older build reading minor 1, the name cap). `persist_crypt`'s "minor column edited"
  probe now writes `FORMAT_MINOR + 1` (it wrote `1`, which became the current minor). The
  `catalog_row_decode` fuzz seed carries a key and an FK rare part; built and run locally for
  90 s (2.5M runs, no crash).
- **DDL (named exceptions in T7's IR and T19's producer, renderers and goldens).**
  `IndexPartDef` gains `opclass` (raw server text) and `collation` (a name, quoted on output);
  the PG part renders as `column COLLATE "C" opclass [DESC] [NULLS FIRST|LAST]`, a collation
  is refused on MySQL and an operator class outside PostgreSQL. `ConstraintBody::Check` gains
  `no_inherit` (`CHECK (…) NO INHERIT` on PostgreSQL, refused elsewhere). The producer fills
  `nulls_first` (only when the model has the vector), `opclass` and `collation` per part on
  PostgreSQL, `no_inherit` on PostgreSQL checks, and an exclusion's deferral from `CheckRare`
  (rendered after the text, which no longer carries it). Golden `pg/index_part_options`.

## m4-t16 — PostgreSQL introspection

Task 16 (HV-INTRO-05; the PostgreSQL parts of HV-INTRO-13 and -15), spike origin `5a5ab08`.
The packs `packs/pg/{10-core,30-state,40-system}.hvq.sql` are written from the PostgreSQL
documentation; `pg/**` runs them on Task 5's `MetaConn` (one `SnapshotTxn` per pass, pinned
`search_path`), decodes them, and builds the model (keys through Task 13's assembly).

- **Change detection** as R-M4-8 and R-M4-48: `age(x.xmin) BETWEEN 0 AND age($W::xid)` with
  `W mod 2^32` as a bind, the 64-bit span check on the client, `ListExistent` (one tagged
  `UNION ALL`, namespace-wide existence sets for every kind but columns), `ListExistentComments`
  (removed comments, finding 3), `ChangedParents`, `ChangedDependents` (one `pg_depend` hop
  from young rows of the whole database, finding 4) and the facet cascade (finding 5). A
  relation is always re-emitted with what was read (its cached properties when its own row is
  not young), so a re-read child replaces its twin and an unread child stays; re-read columns
  report a `Scoped` column set.
- **Full instead of incremental:** `no-watermark`, `restart`, `identity` (also a watermark
  ahead of the server's next XID), `version`, `standby`, `wraparound`, `schema-rename`,
  `mass-change`, plus `level` (the requested level is above the cached one) and `requested`.
  **Deviation:** the mass-change rule (> 25 % of the relations) applies only from 20
  relations (`MASS_CHANGE_MIN_RELATIONS`): below that a few re-reads are cheaper, and the
  narrow-read guarantees of HV-INTRO-05 tests 1–7 hold on small schemas. Task 28 records the
  threshold with timings.
- **The watermark's identity word.** `Watermark::PgXid { w, identity }` has room for the
  system identifier and the start time only; `system_identifier` stores one 64-bit word: the
  high 48 bits an XXH3 of `system_identifier | timeline | database oid` (each `-` when the
  role may not read it), the low 16 bits the major version. A different word's high bits is
  `identity`, its low bits `version`; the start time is `restart`.
- **The canary** reads the backend's own `pg_stat_activity.backend_xid IS NULL` (deviation
  from the plan's `pg_current_xact_id_if_assigned()`): Task 5's token guard refuses every
  `pg_current_xact_id*` and `txid_current*` name by prefix, the read-only forms included. A
  pass whose canary is false fails (`Protocol`), and nothing is published.
- **Joined text.** A `joinable` section is checked by the guard of every lexer (the joined
  engine is unknown), and `::` does not lex everywhere: the `StateOf<Kind>` sections cast with
  `CAST(x AS text)`. No section comment holds a `;` (the guard refuses it in raw text).
  `tests/pg_rows.rs` runs every rendered variant through `check_own_text` and every joinable
  one through `check_read_text`.
- **Composite types are types**, one model object (`CompositeType`, identity = the `pg_type`
  oid, attributes from `RetrieveTypeAttributes`), never also a relation. Base types
  (`typtype b`) have no model kind: `Unknown` with the raw kind `pg-base-type`; system types
  (`Mode::System`) `pg-type`.
- **State marks** (`stamps`): per object an XXH3-64 of the sorted `(source, xmin)` rows of
  its `StateOf<Kind>` section (own row, comments, children, one dependency hop); `None` when
  its own row is gone; `Some(StateMark::None)` for a key without an oid (a column).
  `check_state` answers `Dropped` for a gone object and `Unknown` otherwise: comparing marks
  is Task 22's. `check_namespace` answers the existence count and digest and
  `StateMark::Xmin` of the cached watermark when nothing changed since it (else of the
  current snapshot xmin), running only `Identity`, `ListExistent` and the two change queries.
- **Fragments** report `Scoped` sets resolved by oid (RR-1) and `Watermark::None`: a fragment
  reads a few objects, so the engine must keep the namespace's watermark (carry to T15; with
  `None` the next pass is full, which is correct).
- **Server facts** come from `Identity`: version text and number, encoding,
  `standard_conforming_strings`, recovery, the database's default collation
  (`PassReport.facts`). `list_namespaces` lists the connectable databases
  (`NsKind::Database`) and the connected database's schemas.
- **Scope filter:** applied on the client (objects it excludes are not built, and their
  existence identities are left out). Server pushdown waits for the trait's filter
  parameter (ruling R-W3-2); `PassOpts.filter` carries it.
- **`hv_m4_big`** is a schema of database `hv_test` on `pg17` (deviation from "database":
  no second connection per test, no `CREATE DATABASE`), built and dropped in 250-table `DO`
  blocks of `IF [NOT] EXISTS` statements (`testing::big_catalog`).
- **Named exceptions:** `hacivat-meta/src/props.rs` re-exports `ArgMode`, `FnArg`, `FnKind`,
  `Volatility` (one line: an introspector builds `RoutineProps` without depending on
  `sql-catalog`); the T1c/T4 stub tests (`introspector.rs`, `pack/mod.rs`, `tests/seams.rs`)
  now expect the real PostgreSQL introspector; `hacivat-introspect` gains the dev-dependencies
  `hacivat-net` and `hacivat-driver` (the Docker tests build `ConnectDeps`; the hacivat-exec
  precedent; `Cargo.lock` gains edges only). `hacivat-net` is the controller's R-W3-4; `hacivat-driver` still needs a controller
  ruling.
- **Fix round 1 (security review, I-1..I-6):**
  - *Schema rename (I-1).* The `schema-rename` branch of `ChangedDependents` now sees every
    depender class: relations, routines and types, constraints (`connamespace`), triggers,
    column defaults and view rules (through their relation); and referenced relations of the
    renamed schema as well as its types and routines. A hit runs a full pass of the namespace
    (the fail-safe: a renamed schema is two hops away and rare). Enum value, collation,
    operator and operator-class renames are sources of the dependency hop too (m-1).
  - *Stamps (I-2).* Every dependency hop of the `StateOf*` sections reads the referenced
    column's `pg_attribute` row (a column rename touches nothing else) and the referenced
    type's `pg_type` row.
  - *Validation (I-3).* `check_namespace`'s digest covers each object's `sub` (a domain's
    constraint count) and the comment keys (`ListExistentComments`).
  - *Search-path immunity (I-4).* The joinable `StateOf*` text writes every operator as
    `OPERATOR(pg_catalog.op)`, every cast to a `pg_catalog` type, every function qualified;
    `tests/pg_rows.rs` refuses any bare operator, `::`, unqualified cast or function in a
    joinable section (named exception: T4's `SYNTAX` words gain `operator`). The list bind's
    `IN` stays (the pack grammar fixes it; oid against oid parameters): a joined read must
    refuse a session whose `search_path` does not start with `pg_catalog` (carry to T15b
    and M7). `hv_m4_big` as a schema of `hv_test` is ruling R-W3-8.
- **Seams adopted after T15 (R-W3-2, R-W3-3).** `introspect` and `fragment` take the scope
  filter. `ListExistent`, `RetrieveRelations` and `ChangedParents` (its `rels` set and its
  relation branch) push it through `engine::bounded_pushdown` over every relation kind
  (`Table`, `View`, `MaterializedView`, `ForeignTable`: `pushdown_kinds`) as
  `(:name_like IS NULL OR c.relname LIKE :name_like ESCAPE :like_escape)`, one run per
  pattern (NULL when nothing is pushed), rows deduplicated by oid; the client filter still
  decides. `identity` is an XXH3-128 of the system identifier, the major version and the
  database oid from one `Identity` probe, `Unknown` when the role may not read the system
  identifier. Each section reports its engine `Phase` and row count to the pass's progress
  sink (`PassOpts.progress`); the trait has no sink yet, so the engine's own counts stay
  per pass until a seam carries one (carry to T15's owner).

## m4-t20 — scope editor, encrypted cache option, level pills

M4 Task 20 (gpui) fills `connections/dialog/schemas.rs`, `explorer/level_pill.rs` and
`explorer/userdata.rs` (mocks 6 and 7; R-M4-14, R-M4-15, R-M4-21).

- **One scope, two views.** The tree never evaluates a pattern itself: it asks Task 12's
  `ScopePattern::select_explicit` over the names the profile's last catalog lists, plus a
  sentinel schema named NUL in each database (no engine accepts NUL in an identifier). The
  sentinel tells "every schema of this database, also future ones" (`db:*`, `db:{…}`) from a
  list of the names shown. The tree writes `@` for the current database, `*` for a whole
  database and quotes a name unless a bare term selects exactly it (no separator, no space,
  no ASCII capital a server would fold). Typed text is never rewritten.
- **Errors at character positions** (`ScopeError.column`, scalar values). A pattern that does
  not parse disables Save; the save path refuses it too, so it is never written.
- **All databases** asks once per dialog, before the first change that reaches another
  database (the box, a tree box, a typed pattern, a PostgreSQL database-only clause). Without
  a listed catalog there is no count to show and the dialog does not ask; the engine's
  `intro-scope-all-databases-cost` stays the guard at connect (Task 15).
- **Encrypt cached metadata.** A new profile follows its environment (on for production) until
  the box is clicked; a saved profile keeps its flag, and changing it asks in place. The
  cache files are deleted at Save, before the profile with the new flag is written; a
  deletion that fails stops the save (`SaveError::Io`), so a profile never says "plaintext"
  while its encrypted cache is on disk (Task 11 refuses that downgrade). Task 24 must close
  the profile's open `CacheStore` (stop the hub's catalog) before the deletion runs.
- **Level pills.** Set here (blue) or inherited from the database (grey) come from
  `ProfileUserData.level_overrides` (keys: the namespace root's `ObjectKey` by path, no server
  id). Automatic (grey) shows the level a schema was last read at and its major-object
  count; a schema not read yet, a database without an override, and every row of a profile
  whose data file was not read yet show no pill (fail-closed: the automatic level is the
  engine's `LevelPolicy`, Task 15). The commands write the data file; Task 24 forwards them
  to `IntroHandle::set_level_override`.
- **The profile data file** is read and written only by `explorer::userdata`, on the core
  runtime, through one writer task that applies requests in order (each a read-modify-write
  of the `[metadata]` table). A file that is not TOML, mixes line endings or holds a
  `[metadata]` table of another shape is never rewritten; the profile's data stays unknown.
- **Fix round 1.**
  - **Ruling R-W3-1: sensitivity is derived unless set.** `Profile.catalog_sensitive_set:
    Option<bool>` (T1c's field, named exception; the TOML key `catalog_sensitive` is
    unchanged: a missing key reads `None`, `true`/`false` read `Some`, `None` is never
    written). `Profile::catalog_sensitive()` is the effective value: the choice, else on for
    production. Profiles from before M4 and imports are `None`, so production ones encrypt.
    The dialog writes `Some` only when the user chooses. An environment change that flips a
    saved profile's derived value asks first (it deletes the cache) and Save waits; Keep pins
    the old value. **Task 15 (and every later reader) uses `Profile::catalog_sensitive()`.**
  - **Save** reads the stored profile then: without a choice in this dialog the stored choice
    stands. The cache is deleted when the stored and the new *effective* values differ. Each
    failed step has its own message (cache not deleted, data file unreadable, scope not
    written); a deterministic fault seam (`schemas::inject_save_fault`, feature
    `test-support`) proves each one leaves flag and files consistent in both directions.
  - **The data file:** a failed read is final (`Unreadable`, shown in the dialog); only a
    failed write is followed by one read. No loop.
  - **The tree keeps what it cannot show.** When writing the pattern again from the checked
    rows would not give the same syntax tree, a click adds or removes one clause for its row
    (`!@:"s"`, `@:"s"`, `!db:*`, …), verified through `select_explicit`; a click that cannot be
    made that way asks before writing the pattern again.

## m4-t18 — SQLite introspection

Task 18 (HV-INTRO-08; the SQLite parts of HV-INTRO-13 and -15). Spike origin `f690f45`
(`sqlite.rs`): the bulk joins, their per-relation fallback, `table_options`, the digest and
the refresh decision, rebuilt on the pack and the model.

- **Connections (R-M4-38).** `main` and attached schemas are read on the separate `mode=ro`
  reader, inside its deferred `BEGIN` (one consistent snapshot per pass). `temp` is read on
  the user's own idle connection (`MetaConnSpec { database: Some("temp") }`) with only the
  pragmas its read-only authorizer allows: `pragma_table_list` and `pragma_module_list` are
  never run there, so `WITHOUT ROWID`/`STRICT` come from the text scan and FTS5 shadow tables
  are not recognised in `temp`. A joined read (R-M4-29) uses the same text (every SQLite
  section is `joinable`); T5 refuses to join SQLite's single session until M7 adds the writer
  seam, so the joined path is proven only for an in-memory profile, whose metadata connection
  is the writer.
- **No `PRAGMA` statement, no spliced schema.** Every pragma is its table-valued function
  with the schema as a bind. `sqlite_schema` itself needs the schema as a qualifier, so the
  sections that read it come in three copies: `…Main`, `…Temp` and `…Attached`, the last
  reading the fixed alias **`hacivat_attached`**. To read an attached schema the introspector
  attaches the file the connection already lists under that alias, read-only, through
  `MetaConn::attach_read_only` (the alias is reused on the same reader; a different file
  under it, or an attached schema on the user's connection, answers `Meta(Unavailable)`).
  The alias is never listed as a namespace. A schema the connection lacks is
  `Meta(Unavailable)`, never an empty namespace. No section uses `LIKE`, `GLOB` or a folding
  function, so `case_sensitive_like` on the user's connection changes nothing (T12 M6).
- **Change detection.** `main`'s `schema_version` (the table-valued form reads `main` only;
  `temp` and attached schemas have version 0 and rely on the digest), the main or attached
  file's `(dev, inode)` (Unix; unknown elsewhere), and an XXH3-128 digest of the **sorted**
  `(type, name, tbl_name, sql)` rows, each field length-prefixed with NULL distinct from
  empty. The digest runs on a connection's first pass (`PassConn::Fresh`; the engine opens
  one reader per task, so every trait pass is one), on an identity change or an unknown
  identity, without a version, and on every pass with `digest_every_pass` (network shares,
  edge 5). A different version or digest re-reads the schema whole; otherwise the pass returns
  no change and the fresh watermark. `data_version` is not used (F8). A fragment re-reads its
  relations from a whole read of the schema (local; the spike's 10k-table Structure read is
  245 ms) and returns `Watermark::None`, so the next incremental pass reads the schema again
  instead of trusting a watermark the fragment did not check.
- **Existence (R-W2-5).** Relation kinds are `Ok` on a full read and `Scoped` to the asked
  relations on a fragment; every child kind is `Scoped` to the relations whose children were
  read. A relation whose columns fail (a broken view, a virtual table with a missing module)
  is kept `BROKEN` with no columns and stays out of the column and key scopes, so cached
  children are never decided gone and a column is never "missing" from it. A failed index or
  FK pragma is `Failed`. CHECKs are not modelled (F10, R-M4-24). The engine's message of a
  broken relation has no field in the model; it is kept in the read (`SchemaRows::broken`)
  only.
- **Shapes.** `hidden` 1 = a hidden virtual-table column, 2 = virtual and 3 = stored
  generated (expression from the column's definition text); the declared collation from the
  text, and an index part's collation only where it differs from the column's (`BINARY`
  without one, and for expressions and the rowid; ASCII-caseless); the rowid alias is the
  single primary-key column declared exactly `INTEGER` on a rowid table with no `pk`
  autoindex (`AUTOINCREMENT` → `Identity::AutoIncrement`); FK deferral from the text, clause
  `k` of `n` being id `n − 1 − k`, unknown (not deferrable) when the counts disagree; triggers
  under their table or view with timing, event and `WHEN` from the text (`BROKEN` when the
  text has another shape); `sqlite_*` names `SYSTEM`; `shadow` tables `IMPLICIT`.
- **State (HV-INTRO-15)** is schema-level: every object carries `Hash(XXH3-64(schema_version
  ‖ digest))`; `stamps` answers that mark for a key that exists and `None` for one that does
  not; `check_state` answers `Dropped` or `Unknown` (it has no stored mark; Task 22 compares
  `stamps`); `check_namespace` the relation count, the mark and an existence digest that
  `sqlite::ns_check_of` computes the same way from a cached namespace.
- **Locks.** `SQLITE_BUSY`/`SQLITE_LOCKED` after the driver's 5 s busy timeout: a pass is tried
  twice, then fails softly (`Failed`, nothing decided).
- **Built-ins** (`function_list`, `module_list`, `collation_list`) are read live per
  connection (`SqliteIntrospector::builtins`, R-M4-6); none enters a namespace (the explorer
  shows no Built-in group for SQLite). **Facets:** `Sources` (`sqlite_schema.sql` of tables,
  views, indexes and triggers); `Grants` not applicable (empty); `Statistics` without a row
  estimate; `Dependencies` unsupported (`NotImplemented`). Capabilities: incremental (the
  version/digest skip), transactional DDL, facets `Sources`, `Grants`, `Statistics`.
- **File identity (R-W3-3, prepared).** `SqliteIntrospector::file_fingerprint`: XXH3-128 of
  `main`'s `application_id` and the file's canonical path (stable across DDL and `VACUUM`;
  the inode is left out so a save-by-rename keeps the database); unknown for an in-memory
  database.
- **After T15 (resume).** `introspect`/`fragment` take R-W3-2's scope filter and apply it on
  the client (a relation it does not keep is neither built nor listed as existing; nothing is
  pushed down: the read is local). `identity` (R-W3-3) is `file_fingerprint`.
  `SqliteIntrospector::with_progress` reports `Tables`, `Columns`, `Indexes` and `Constraints`
  counts per pass (the seam has no progress parameter, so the engine hands the sink to the
  introspector it builds). Every rendered section, in every copy, passes T5's guards
  (R-W3-9, `every_rendered_sqlite_section_passes_the_metadata_guard`).
- **Fix round 1 (opus review).**
  - **`temp` is read whole on every pass** (I-1): a temp view or trigger may depend on `main`
    or an attached schema, which leaves `temp.sqlite_schema` and its digest unchanged.
  - **`temp` is never decided off its connection** (I-2): `check_state` answers `Unknown` for
    `temp` keys, `stamps` and `check_namespace` answer `Meta(Unavailable)` when asked about
    `temp`. The engine-side split of a state check by connection is T22's.
  - **One reader per attached schema** (I-3, ruling R-W3-10): with
    `SqliteIntrospector::with_readers(source)` (the engine's `MetaSource`), each attached
    schema is read on a reader of its own (pool slot keyed by the schema's name), which
    attaches exactly that file under the alias and is closed, and reported released, as soon
    as the read ends. A call over several attached schemas works, and a user schema named
    `hacivat_attached` is an ordinary namespace. Without a source the connection handed in
    holds the alias for one file (fail-closed `Unavailable` for a second). No T5 detach seam.
  - **FK deferral** follows SQLite's `sqlite3DeferForeignKey` (m1): a `DEFERRABLE` clause
    applies to the table's most recent foreign key, also when it stands alone in a later
    column. A trigger's `WHEN` ends at the body's `BEGIN` only: a bare top-level word followed
    by a statement keyword, never a `begin` identifier or name part (m2).
  - **Bounded fallback** (m3): after `MAX_FALLBACK_FAILURES` (64) relations of one kind fail
    one by one, the rest of that kind is kept `BROKEN` unread (children undecided), so a
    crafted file of views at SQLite's reference cap cannot keep the reader busy for minutes.
  - **`check_namespace` counts what the cache could hold** (m5): its relation count and
    existence digest cover the server relations the cached namespace has (already cut by the
    scope filter); a newly kept relation moves the mark instead.

## m4-t15 — engine and hub

- **One actor per profile** (`engine/scheduler.rs`), spawned on the runtime the config names
  (the `CoreRuntime`), is the catalog's only writer: it takes the `Publisher`, so a second
  writer cannot exist. Workers (`engine/work.rs`) do the I/O and the merge (on the blocking
  pool) and send the answer back; only the actor publishes. Every `IntroHandle` method sends a
  message (or reads the level policy's lock, which no I/O holds) and answers a
  `RefreshTicket`: nothing of the engine runs on a GPUI thread.
- **Tasks and keys.** `ScopeRefresh`; namespace passes (`Brief`, `Incremental`, `Full`; an
  `Incremental` without a watermark or capability runs `Full`); `Fragment` (also `Elevate` and
  the per-object children read); `CheckState`. One task per namespace at a time (and one scope
  refresh), at most `EngineLimits::workers` (2) at once. Queued tasks of one namespace
  coalesce: one pass with the stronger mode and the higher level, one fragment with the union
  of the keys (fragments wait 50 ms for company, so a burst publishes once). A **load**
  (expand, children, elevate, a lookup's request) joins a running task that already reads
  what it wants; a **refresh** never does (it queues a new pass after the running one, so a
  DDL that happened during the running pass is seen). Priorities: user > completion >
  background > periodic.
- **Tickets.** A ticket resolves with the generation of the publish that holds its result, or
  the current generation when the catalog already held it (a satisfied load, nothing pending
  to confirm). Dropping a ticket never cancels the task; `cancel(ns)` and `shutdown` do, and
  then nothing of the task is published. An expansion of an out-of-scope namespace, and a
  result for a namespace that left the scope meanwhile, answer `Cancelled`. A `check_state`
  re-reads what changed or was dropped and resolves when those fragments are published
  (recording freshness itself is Task 22's).
- **Atomic publish and states.** A pass is merged whole and published with
  `publish_with_delta` in one step: the arena (a unique `Arc`, moved, never copied), the
  namespace's state, the `Children` states. A kind whose existence query failed makes the
  namespace `Partial { reason }` (a lookup then answers `Unknown(Failed)` for a miss, never
  `Missing`) and its `Children { root, kind }` container `Failed`; a kind that reads again is
  marked `Loaded`. A failed task publishes nothing, except `Failed` for a namespace with no
  previous state (no arena; `NotLoaded` or `Failed` before). A fragment never sets the
  namespace's state, and keeps the namespace's watermark and load time (the merge takes both
  from the result; a fragment read some objects, not the namespace).
- **Levels.** `LevelPolicy` holds D-038's thresholds, the overrides (a pill on a database
  reaches its schemas; a schema's own wins) and the back-pressure lowering: the second
  timeout of a namespace lowers its auto level one step (`EngineEvent::LevelLowered`, the
  `intro-hint-level-lowered` hint); an override is never lowered. The merge never lowers a
  level, so the published `LoadState::Loaded.level` is the pass's level when the merge reports
  objects re-read below the namespace's (`below_level`). `IntroHandle::level_for` gives the
  pills (T20) the effective level and its source; a pill change re-reads every loaded
  namespace below it that is now under its level.
- **Bulk policy.** `load_children(key)` answers at once when the namespace holds `key` at
  `Structure`; else one fragment of `key` at `Structure`, or one bulk pass (`Full` at
  `max(Structure, auto)`) when the namespace has fewer than 2,000 relations or ≥ 3 distinct
  objects were asked within 500 ms; the bulk pass absorbs a queued per-object read.
- **On connect.** The cached namespaces are published stale when the engine starts (before
  `ensure` answers). `on_connect` runs the scope refresh, then the current namespace's
  `Incremental` pass if it is cached, else `Brief` then `Full` at its auto level (skipped when
  the auto level is `Names`); other namespaces load on first expansion (`expand`: names first,
  then the auto level; a stale cached one is validated incrementally).
- **Scope refresh.** Every listed namespace in scope without a state gets `NotLoaded` (so a
  lookup knows it exists), every one out of scope is removed and marked `OutOfScope`, a
  namespace the server no longer lists is removed (`EngineEvent::NamespaceDropped`, not
  guarded, R-M4-22), and `ContainerKey::schema_list` is set `Loaded` (PostgreSQL: per database
  with listed schemas; elsewhere the server's) after each refresh (T9 carry). The current
  namespace is what the empty pattern (`@:@`) selects.
- **Follow-ups.** The merge's `report.fragments` (a child whose parent was not placed, edge 4;
  an object only the existence set lists) and the parent of every identity rename (T10 M-2: a
  half re-read name swap heals there; a top-level rename re-reads the namespace) are re-read
  at background priority. A follow-up never schedules another.
- **Retries.** A lost connection restarts the task once from the beginning (edge 5); a
  `CatalogRace` is retried once (`EngineEvent::CatalogRaceRetried`, log only). Each attempt
  takes its own `MetaConn` and drops it before the merge (it holds a budget slot; T5 concern
  1). `Busy` (a single-connection profile in a user transaction) is asked again every
  `busy_retry` until idle or cancelled (edge 10).
- **Mass-deletion guard.** A guarded merge publishes nothing of the pass: the namespace is
  marked stale (`Loaded { stale: true }`) and `Publisher::announce_pending_confirmation` is
  called, for a user or a background trigger alike (the UI asks for a user refresh, shows the
  badge for a background one). `confirm_pending(ns, true)` publishes the held merge; `false`
  re-merges the kept result with `Deletions::Suppress`. A held merge whose namespace changed
  since is dropped. The result is kept (cloned) only when the cached namespace holds at least
  `guard_min_objects` top-level objects, the only case the guard can fire.
- **Persistence.** Every namespace a publish puts is offered to Task 11's `SaveDebouncer`; due
  saves and removals run on the blocking pool; `shutdown` writes the pending ones before
  `closed()` resolves. A full disk raises `EngineEvent::CacheWriteFailed` once per store.
- **Periodic validation** (R-M4-20): every `validate_every`, an incremental pass of each loaded
  in-scope namespace at periodic priority, skipped while `MetaSource::user_in_transaction`
  (a single-connection profile whose user holds a transaction). Task 22 replaces the body
  (named lines `// ## m4-t15-engine (validation tasks)` in `engine/scheduler.rs`).
- **Filters** (T12 carries). The introspector seam carries no filter, so the engine applies the
  scope filter to every result on the client before the merge (changed objects and existence
  identities alike, so the merge neither stores a filtered object nor asks for a fragment of
  it); never inside a system namespace. `ScopeFilterAdapter` is the lookups' `ScopeFilter`.
  `bounded_pushdown` caps a pushed predicate at 16 runs per section (one query per run): the
  per-profile total of T12's per-call cap. The engine itself pushes nothing in M4.
- **The hub.** `ensure` makes a profile's catalog once (a per-profile lock: concurrent callers
  share one making). `Profile::catalog_sensitive()` (T20: the explicit choice, else production; never the raw field): `SecretStore::catalog_key(profile, true)` on the
  blocking pool, here and only here (so two first-time creators never race, T11 carry); a
  refused or unavailable keyring makes the profile memory-only (`CacheMode::MemoryOnly`,
  `KeyringUnavailable`), never `Encryption::None`. Any cache open error is memory-only too.
  `get` and `row_key` read a map only (no file, no keyring). Metadata connections come only
  from `SessionManager::meta_conn` with a spec naming the hub's own profile, which `meta_conn`
  refuses on another profile's manager (T1c review m-B2). Defaults until the app forwards the
  profile's own: the R-M4-14 scope, the profile's database as current (PostgreSQL's current
  schema comes from the session through `set_current`), D-038's levels, R-M4-20's interval.
- **Seam changes.** `IntroHandle` gains `ready`, `load_children`, `set_current`, `level_for`,
  `events`, `lookup_hooks`, `cancel`, `closed`; `CatalogHub` gains `new_on`, `with_limits`,
  `system`; `ProfileCatalog` gains `cache: CacheMode`; `HubError` gains `NoRuntime` (a hub
  made outside a runtime). `CatalogHub::new` keeps its signature and takes the current
  runtime. `IntroHandle::detached` stays (a catalog with no engine, every request
  `NotImplemented`). Named exception: `tests/seams.rs` (T1c), whose hub stubs expected
  `NotImplemented` from requests that now reach the engine (fenced `// ## m4-t15-engine`).
- **Test clock.** The workspace's `tokio` dev-dependency of this crate has no `test-util`
  feature (manifests are Task 1a's), so the plan's `start_paused` tests wait on conditions
  instead: counters of the fake metadata source (connection attempts, validation rounds) and
  millisecond intervals.

### m4-t15 fix round 1 (review: 0 Critical, 5 Important; rulings R-W3-2, R-W3-3)

- **Filter seam (R-W3-2).** `Introspector::introspect` and `fragment` take
  `filter: &CompiledFilter` after `level`. The engine passes `Filters::for_pass(ns, system)`:
  the scope filter where it applies, a match-all filter (an empty `Introspect` filter) for a
  system namespace, for no filter and for a view filter. A section pushes only through
  `bounded_pushdown` (≤ 16 runs; the client filter still decides, a pushed predicate must
  return a superset). `IntroHandle::set_filters`: what a narrower filter no longer keeps is
  removed from every loaded namespace in one publish, never guarded, listed in the delta's new
  `out_of_scope` (named exception in T10's `delta.rs`: a `#[serde(default)]` field, counted by
  `is_empty`/`len`, carried by `combine`, removed by `apply_delta`), with
  `EngineEvent::ScopeNarrowed`; a filter that may keep more (any change from a non-empty
  filter) re-reads every loaded namespace in scope. The hub still starts with no filter: the
  app forwards the profile's saved one (T24).
- **Identity seam (R-W3-3).** `Introspector::identity(conn) -> ServerIdentity`
  (`Known(ServerFingerprint)` / `Unknown`), default `Unknown`. `on_connect`'s scope refresh
  reads it on the same connection and, with a cache, calls `CacheStore::check_identity` on the
  blocking pool (without one, it compares with the last connect). Another server, or an
  unknown one (fail-closed), re-publishes every namespace that has a watermark without it
  (stale), before step 3, so the passes that follow are full. T22 owns the behaviour beyond
  this hook; T16–T18 implement `identity`.
- **Other PostgreSQL databases (I-2).** The scope refresh lists, on a connection to each, the
  schemas of every listed database (a `NsKind::Database` namespace) whose schemas the first
  listing lacks and that a positive scope clause may select (`ScopePattern::
  may_select_database`, named exception in T12's `scope.rs`). A schema of a database the
  scope no longer selects becomes `OutOfScope`, not "dropped". T16 must list the database-level
  namespaces.
- **Held merges (I-3).** A fragment keeps a held (guarded) merge; a later pass of the
  namespace replaces it and raises `EngineEvent::PendingWithdrawn`. `confirm_pending` applies
  the held merge when the namespace is unchanged; otherwise it re-plans a full pass that
  carries the decision (accept: guard off; keep: `Deletions::Suppress`) and resolves with it.
  With nothing held it answers `Cancelled` (nothing applied), never `Ok`.
- **Minor.** `closed()` waits on a token the actor cancels on exit (it holds no sender); a
  task cancelled after its read publishes nothing; `shutdown` takes the profile's lock and the
  locks are pruned; the fold rule is read per task from the current server facts; the hub's
  `user_in_transaction` asks only a manager `meta_conn` accepted (else "yes": validation
  waits); a current namespace learned after `on_connect` gets step 3; a missing-name probe runs
  at most once per namespace per `probe_interval` (30 s); a failed engine-initiated publish
  raises `EngineEvent::PublishFailed`; `cancel(ns)` also stops state checks of `ns`'s objects;
  a pass's progress counts the batch's tasks; `IntroHandle::level_of(ns)` derives the role and
  count (a database row: its override, else its schemas' objects); a pill change resets the
  back-pressure lowering it covers; only a namespace that was loaded is announced as dropped.
- **Carries.** A progress sink inside one pass needs the introspector seam → T16–T18. The
  catalog's `ServerFacts` cannot be replaced (MySQL `lower_case_table_names` arrives with the
  connection) → T17/T24 seam. `hold_pending` on a `Partial` namespace sets no stale mark (it
  has none to set). The fakes stay compiled under `session` (gating them needs a manifest
  feature, T1a's). `tests/assemble*.rs` do not build with `--no-default-features` (T13).

### m4-t15 fix round 2 (verification RR-1..RR-3)

- **A guard decision survives coalescing** (RR-1): a confirmation's re-planned pass that
  coalesces with a queued pass of the namespace carries the decision onto it (the latest
  answer wins).
- **Unlistable databases** (RR-2): a database the scope refresh cannot connect to or list does
  not fail the refresh. It is reported as unlisted (`EngineEvent::DatabaseUnlisted`), its
  known schemas keep their states, it gets no schema list, and its schemas are not in scope
  (so never `Missing`, and an expansion answers `Cancelled`) until a later refresh lists it.
  Only a failure of the profile's own listing fails the refresh.
- **One connection at a time** (RR-3): the scope refresh closes its own connection before it
  lists the other databases, each on its own connection, one after the other; no budget
  slot is held idle. `MetaSource::released` (default no-op) is called after each close.
- **Filters changed under a running pass** (m-rr1): its result is read again under the
  current filters, so a filter-excluded object is never published, then dropped as a
  server deletion.
- **Carried:** the merges of `set_filters` and of the identity step still run on the engine's
  task, not the blocking pool (user actions and connects, O(arena size)) → T22 (which owns
  the identity behaviour) or a later engine touch; the `Partial` stale mark (m12) stays as
  noted; gating the fakes is R-W3-7 (T33).
