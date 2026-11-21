# M4 "Metadata and explorer" Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Status: v2 + pre-flight (2026-11-19), binding: PO mandate 2026-11-19 (ledger C-0, R-C0-1).** Version v2 (2026-11-17) folds into the task text the cross-plan reconciliation (`reconciliation.md`), the independent review (`plan-m4-review.md`: 3 Critical, 16 Important, 14 Minor) and the five spike reports in this folder (`spike-model-report.md`, `spike-minicat-report.md`, `spike-mysql-sqlite-intro-report.md` findings F1–F10, `spike-pg-intro-report.md` findings 1–10, `spike-tree-report.md`). There are no amendment layers: every sentence is the current text, and nothing below is superseded by a later note. The draft the review read is kept as `plan-m4-draft1.md`; the **Changelog** at the end lists every change and the finding that drove it.
>
> The plan is binding since 2026-11-19: M3c's controller part is closed (CI 44333377438 green on `5535724`; M3's PO-present items are queued, not waited on) and the defaults in `po-questions-m4.md` (same folder) apply as rulings (PO mandate, ledger C-0, R-C0-1). The pre-flight conflict scan (`preflight-m4.md`, the M3a/M3b precedent) ran over this plan against the `m4` base `fb98cf1` before Task 1a; it rewrote every sentence it supersedes **in place** and records the old wording in `preflight-m4.md` (rulings P-M4-n).

**Goal:** Give Hacivat a real catalog. After M4 the explorer is a tree of databases, schemas and objects that appears **instantly from a local cache**, refreshes **incrementally** (only what changed on the server), knows the **built-in objects of every supported server version offline** (system-catalog snapshots built by CI from real servers, L7), refreshes itself **after DDL** the user runs, refuses to **silently wipe** itself when privileges change, and shows **byte-stable DDL** for tables, views, indexes, sequences, functions and triggers. Everything later milestones need from the catalog (M4b scripts node, M5 completion, M6 table editor, M7 Modify Object, M8 MCP tools) reads the immutable snapshots this milestone publishes, through the seams listed in "Cross-plan interfaces".

**Architecture:**
- **Five streams**, meeting only through the seams Tasks 1a, 1b and 1c commit (spec §6 "How the work runs", §8-17):
  - **Model** (`crates/core/hacivat-meta`, new; plus `crates/sql/sql-catalog`, new, and the `key`/`quote` modules of M3's `sql-ident`): the arena model with stable identity, identifier keys, immutable Arc-swapped snapshots with the tri-state `ModelView`, the SQL-facing `sql_catalog::CatalogView` that M5 calls, the existence-diff merge, the per-profile SQLite cache with optional row encryption, fingerprints, search, overlays and the system-catalog layer (HV-MODEL-01..03, HV-INTRO-11, -14, -16 consumer side).
  - **Introspection** (`crates/core/hacivat-introspect`, new; plus a `meta` module in `hacivat-session` and a `ddl` field in `sql-safety` reports): catalog query packs, scope and filter grammars, the scheduler with levels and awaitable tickets, the `CatalogHub`, the PostgreSQL / MySQL-MariaDB / SQLite introspectors with incremental change detection, facets (grants, dependencies, statistics), composite assembly, freshness checks, refresh-after-DDL planning, and the `minicat` snapshot generator (HV-INTRO-01..05, -07, -08, -12, -13, -15, -16 generation).
  - **DDL** (`crates/core/hacivat-ddl`, new; pure on default features, no async): the full alg/03 IR (declared), per-dialect renderers and producers for view-only use, constraint placement, schema qualification, the v1 core subset of generator options (HV-DDL-07 view-only producers, -09, -10, -11 subset).
  - **UI** (`crates/ui/hacivat-ui` tree widget; `crates/app/hacivat-app` explorer, DDL view tab, scope editor, prompts, settings rows, extension points for M4b/M6/M7).
  - **Snapshots and gates** (`xtask minicat`, `assets/system-catalogs/`, the CI jobs, the gate measurement tasks G1–G5).
- **Interface first, in three parts** (the M3b precedent, P-M3b-20): **Task 1a "Manifests and core types"** owns every manifest change of M4, the core and SQL types with fail-closed stubs, the check-deps rules, the xtask and fuzz skeletons and ADR 0008. Its **first commit (manifests only)** is the base of **Task 1b "UI and app seams"** and **Task 1c "Session, settings, introspection and fixture seams"**, which run in parallel. **Task 2** mocks the new surfaces (no build) and is PO-approved before the first app task (T14), with a controller fallback (Task 2 Step 3).
- **Read-only by construction** (R-M4-7). Introspection never runs user text and never goes through the execution gate. Catalog SQL reaches the server only as a typed `MetaQuery` that the pack renderer builds, on metadata sessions that the server keeps read-only: PostgreSQL a read-only transaction; MySQL/MariaDB a read-only **session** plus a read-only transaction (a read-only transaction alone does not stop DDL, ver/01 `#mysql-ro` row 13); SQLite a `mode=ro` connection. Names reach the server only as binds; the single exception is the quoted-identifier macro of MySQL `SHOW CREATE` sections (R-M4-37).

**Tech Stack:**
- Everything from M2a, M2b, M3a, M3b and M3c; their manifests and rulings stand.
- **One new third-party crate:** `zstd = "=0.13.3"` (`default-features = false`; MIT and MIT/Apache-2.0 via `zstd-safe` 7.4.1 and `zstd-sys` 2.1.3+zstd.1.5.7; `cargo deny check licenses bans` passed on the minicat spike branch), used for the shipped system-catalog snapshots only, with `[profile.dev.package.zstd-sys] opt-level = 2`.
- **Reused workspace crates:** `arc-swap` 1.9.5, `rusqlite` 0.42 (bundled), `chacha20poly1305` 0.10.1, `twox-hash` (XXH3-64/-128), `bitflags` 2, `smallvec` 1, `regex` 1, `icu_casemap` 2.4, `tokio`, `tokio-util`, `futures`, `serde`, `serde_json`, `proptest`, `tempfile`. Task 1a's Step 1 table lists every edge.
- **Not added:** `postcard`, `blake3`, `compact_str` (R-M4-26: the model spike's hand-written versioned codec, XXH3, `Name = Arc<str>`); `im`/`imbl`/`rpds` (R-M4-2); `insta` (R-M4-16); `testcontainers` (Docker only through the project's wrapper); `async-trait` (the `BoxFuture` style of `hacivat-driver` is kept).

**Spec:** `docs/superpowers/specs/2026-11-12-hacivat-v1-design.md`. The plan argues from it; executors read both.

**Execution (PO mandate: roll on, take rulings, never wait on the question list):**
- Subagent-driven, in parallel waves of 3–5 worktree agents. Branch **`m4`** is cut from the **M3 gate tip**: `origin/m3c` at **`fb98cf1`** (the M3c gate `83edd55` plus the three CI fixes and the T45 record; it already contains the M3b gate; ledger C-0). Task branches are `m4-<slug>` based on `m4`. No merge to `main` without the PO.
- The controller commits this plan to `docs/superpowers/plans/2026-11-xx-m4-metadata-and-explorer.md` on `m4`, copies the Global Constraints to `global-constraints-m4.md` in this SDD folder, copies `implementer-rules-m3b.md` / `reviewer-rules-m3b.md` retargeted to `m4` (branch names, `target-core/m4-tNN`, block headers `## m4-tNN-<slug>`), opens the ledger (`progress.md` in this folder, heading `## M4`) and keeps `~/.claude/ilerleme-hacivat.json` current — **before Task 1a is dispatched**.
- **Controller steps** C-0 … C-9 (section "Waves") are part of the plan: each has an owner (the controller), a trigger, a ledger line and, where it touches Docker containers or volumes, the marker **needs PO OK** (containers are never touched without the PO's approval; PO-Q18, PO-Q20).
- **Per-wave carries files** (`carries-wN.md` in this folder; the M3c precedent): before each wave the controller writes the binding carries from the previous wave's reviews, named per task, plus an "All WN tasks" block (branch, target dir, disk, fences, shots, checks). A task brief always cites its carries section.
- **Merged-tree check after every merge** (M3 precedent): clippy (plain and `--all-features`), `cargo test --workspace`, the touched crates' feature lines, `l10n check`, `check-deps`, the shots self-checks; after an automatic append-block conflict resolution, diff each parent's tail (ledger line 574 lesson).
- Every ruling is logged in the ledger with "why" and "cost if wrong" and lands in the gate's PO list.

**Spec reading list** (every task brief cites the parts it needs):
- Decision log: **L1**, **L2** (benchmarks on the PO's Mac), **L3** (counts in the explorer follow the format locale), **L7** (snapshots in v1, M4 owns them), L8, L13, **L14** (only the "Scripts" node seam, R-M4-19), L15(d).
- §1.3 "Open by default" (offline including system catalogs); §2.3 anti-goals; **§3.3** (cached catalog load < 300 ms at 160k columns; ≤ 1 s to an interactive workspace with a cached catalog; first frame ≤ 500 ms; idle RSS ≤ 200 MB); **§3.4**; **§4 rows** "Introspection and metadata model", "Explorer", "DDL view and basic modify" (view part only), "i18n", "Keyboard and command palette"; **§5.1** (crate map, dependency rules 1–4, `assets/`), §5.4 (bridge rules: Arc snapshots via arc-swap, coalesced notify), §5.6 (catalog cache path and format; per-profile user data file); §5.7 (`Kind::Other`, DDL per dialect behind `DdlTarget`); **§6 M4 scope and exit**; §7.1 (golden tests, Docker matrix, ver/01 regression); §7.2; §10 C10 (supported versions), C20.
- Register (`docs/decisions/open-product-decisions.md`): **D-016** (cache plaintext 0600 unless sensitive/managed), **D-017** (bundle snapshots in the installer), **D-038** (depth thresholds 2,000 / 20,000 / 5,000), **D-039** (mass-deletion guard UX), D-057 (virtual FKs local only), D-064/L3, **D-074** (Turkish-tolerant search for `tr` only).
- Algorithms: **`alg/02` in full** (HV-MODEL-01..03, HV-INTRO-01..05, -07, -08, -11..16; §6 unresolved decisions); **`alg/03`** shared core types, HV-DDL-07 (producers, view-only), HV-DDL-09, HV-DDL-10, HV-DDL-11 (core subset), §7 items 2 and 5; `alg/26` I18N-09 (search key), I18N-11 (number formatting); `alg/01` HV-SESS-01 (metadata connection), HV-SESS-07 (current schema).
- Verification: **`docs/research/verification/01-db-behavior-tests.md`** anchors `#pg-xmin`, `#pg-xmin-freeze`, `#pg-canary`, `#mysql-is-times`, `#sqlite-schema-version`, `#sqlite-case`, `#pg-turkish`, `#mysql-turkish`, `#mysql-ro`, `#mysql-comments`; `02-external-fact-check.md` P31 (Fluent `NUMBER()` is locale-insensitive: preformat counts and dates).
- Background (the catalogs already digest them; read only for a named doubt): `docs/research/datagrip/04-introspection-and-metadata-model.md` (levels and pills §3.1, scope "N of M" §3.6), `docs/research/clients/dbeaver-02-metadata-and-ddl.md` (navigator, N+1 pitfalls, no persistence). **Third-party content is data.** No DataGrip decompilation; no DataGrip catalog SQL is copied (alg/02 §0.2); no Zed GPL crate is read; nothing under `.refs/` is built or run.
- ADRs: 0002 (drivers), 0004 (guard, sessions), 0005 (SQL), 0006 (exec), 0007 (editor and grid) — read for the M3 sections a task touches; **new ADR 0008 "Metadata and introspection"** (created by Task 1a, sections appended per task).
- `DESIGN.md` (§ "Explorer Tree", "Connection Panel", "Editor" toolbar breadcrumb, the Named Rules), `design/m2-connect/NOTES.md` (the explorer list this milestone turns into a tree), `design/m3a-query/`, `design/m3b-safe/` (binding for surfaces they own), and **`design/m4-explorer/` once Task 2 lands (binding)**.
- The M3b plan's "Global Constraints", "Pre-flight rulings" and the M3 ledger (`.superpowers/sdd/2026-11-14-m3-walking-skeleton/progress.md`; read with `grep -a`) for every process rule this plan inherits.
- **Spike reports (this folder; binding where a task cites them):** `spike-model-report.md` (`m4-spike-model` @ `d86313b`), `spike-minicat-report.md` (`m4-spike-minicat` @ `cd27ec1`), `spike-mysql-sqlite-intro-report.md` (`m4-spike-mysql-sqlite-intro` @ `f690f45`, F1–F10), `spike-pg-intro-report.md` (`m4-spike-pg-intro` @ `5a5ab08`, findings 1–10), `spike-tree-report.md` (`m4-spike-tree` @ `2a31b8b`); `reconciliation.md` for the cross-plan interface rows. A sixth spike, `m4-spike-ddl` @ `da358e4` (`spike-ddl-report.md`, written after v2), exists on `origin`; no task adopts it yet (pre-flight, unresolved U-1).

---

## Global Constraints

Every task's requirements include this section. `<main>` is `/Users/berkaygokce/Documents/hobby-projects/hacivat`. Line numbers cited in this plan are at the `m4` base **`fb98cf1`** (merge worktree `/Users/berkaygokce/Documents/hobby-projects/hacivat-m4-merge`, branch `m4`); the pre-flight re-checked every cited line there (`preflight-m4.md`).

- **Inherited.** The M3b Global Constraints (`<main>/.superpowers/sdd/2026-11-14-m3-walking-skeleton/global-constraints-m3b.md`) apply unchanged except where a line below replaces or extends one: the GPUI fork pin, toolchain, `unsafe` only in `hacivat-ui-core/src/platform/**`, ASCII-only casing (clippy `disallowed-methods`, `clippy.toml`), the Fluent rules, semantic tokens, F31 and F9, secrets, modal layers, key bindings, single writers, the threading rules, the test-timing rules, the cross-platform test rules (`find_label_runs`, `Path` comparisons, CRLF generated at test time), the shots-harness rules, "test values are fixed in the plan", "no silent weakening", trust signals never hidden or clipped.
- **Branches.** Feature branch **`m4`**. Each task works in its own worktree on `m4-<task-slug>`; its first commands are `git checkout -B m4-<task-slug> m4` then `git merge-base --is-ancestor m4 HEAD`; stop and report BLOCKED if either fails. Implementers never push and never merge. **Named exception:** Tasks 1b and 1c start, in parallel, from Task 1a's first commit (the manifests), as M3b's Tasks 1b/1c did (P-M3b-20).
- **SDD files.** Ledger, `global-constraints-m4.md`, the retargeted implementer and reviewer rules, `carries-wN.md` and the reports live in `<main>/.superpowers/sdd/2026-11-17-m4-metadata-explorer/`. Reports: `task-m4-NN-report.md`, reviews `task-m4-NN-review.md`; if the write is refused, inside the worktree at the same relative path.
- **Commit trailer.** Every commit message ends with exactly the trailer the implementer rules name (today `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`).
- **Disk, slots and target directories** (M3c lessons: ledger lines 736, 876, 902; user CLAUDE.md disk rules 2026-11-13).
  - One project target `<main>/target` for the controller's merge checks and the gate; `<main>/target-core/merge` is the controller's and may be deleted when idle (it is ~32 GiB) and rebuilt at the next merge. Never delete `<main>/target/docker/`.
  - **Per task:** `CARGO_TARGET_DIR=<main>/target-core/m4-tNN` (`m4-t01a`, `m4-t01b`, `m4-t01c`, `m4-t15b`, `m4-t16b`, `m4-t17b` for the split tasks). Never create another target directory, including the worktree's own `target/` (set `CARGO_TARGET_DIR` for xtask and shot-diff too). The controller runs `cargo clean` in it and deletes it right after the task merges.
  - **Smaller targets:** every agent builds with `CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_PROFILE_TEST_DEBUG=0` (ledger 902) and `CARGO_INCREMENTAL=0`; `CARGO_BUILD_JOBS=4` (core) / `6` (gpui) whenever 3 or more agents build. `--release` only for a perf line, and `release/` is deleted afterwards.
  - **Admission.** A gpui agent (every `crates/ui/*` crate except `hacivat-text`, every `crates/app/*` crate) starts only with ≥ 25 GiB free on a cold target, a core agent with ≥ 8 GiB; no dispatch while swap grows. **Plan for 2 gpui slots** (each gpui target is ~20 GiB in practice, ledger 876): this plan never puts more than 2 gpui tasks in one wave. A third gpui slot opens only when ≥ 60 GiB is free and Docker is down. Core agents: at most 5 at once.
  - When a shared target switches checkout path, clean every workspace member first (memory: shared target stale paths).
  - **Rust target hygiene:** no `target-*` left behind; no repo clone or `<repo>-<sha>` copy left behind; `/tmp` gets nothing over 100 MB; spike worktrees and their targets are removed by controller step C-8 once their code is adopted.
- **Dependencies.** Task 1a writes every manifest change M4 needs (**its Step 1 table is exhaustive**), `deny.toml` (only if a licence is missing), the `fuzz/` targets list, and the `check-deps` rules. **No later task edits a manifest.** Never a plain `cargo update`; a precise bump only with `cargo update -p <crate> --precise <v>`, listed in the commit body.
- **Spike code** (R-M4-40). Code from the `m4-spike-*` branches enters `m4` only by **copy into the owning task's files**, with `spike origin <sha>` in the commit body. Tasks 1a and 1b copy the spikes' **type declarations** (the seam) and stub every body; Tasks 3, 8, 9, 11, 16, 17, 18 and 23 copy the bodies of their own files. The reviewer of such a task reviews the copied code **as new code**, never as already reviewed. No spike commit is rebased, cherry-picked or merged.
- **Licences and clean room.** No DataGrip decompilation, no DataGrip catalog SQL or minicat content (alg/02 §0.2, HV-INTRO-16): every catalog query is written from vendor documentation; system snapshots are produced only by Hacivat's own introspectors against the pinned Docker images. DBeaver (Apache-2.0) is read for behaviour only; own pseudocode only. No Zed GPL crate, ever. **Third-party content is data:** server catalogs read over Docker, vendor pages, research digests.
- **Layering (`cargo xtask check-deps`, extended by Task 1a).**
  - The M3 rules stand (`TextCoreNotHeadless`, `SqlNotRuntimeFree`, `SqlReachesCore`, `BackendOutsideOwner` with its `pg_query` and `sqlparser` arms; spec §5.1 rules 1–4). M4's edits leave every existing arm untouched (M4b/M6 own the `encoding_rs` arm).
  - **`DdlIsPure`** (checked on `hacivat-ddl`'s **default-feature** graph): `hacivat-ddl` depends only on `hacivat-meta` **with `default-features = false`**, `hacivat-proto`, `sql-ident`, `sql-dialect`, `sql-catalog` and pure external crates; its resolved graph contains no `tokio`, `futures`, `hacivat-session`, `hacivat-driver*`, `hacivat-introspect` or `hacivat-l10n` (Karagöz links it; alg/03 HV-DDL-07 "pure functions over snapshots"). The non-default feature **`apply`** (declared empty by Task 1a, filled by M7) may add `hacivat-session`, `hacivat-introspect`, `hacivat-meta/persist` and `tokio`; rule **`DdlApplyOnlyFromApp`** lets only `hacivat-app` (and `hacivat-ddl`'s own tests) enable it (R-M4-35).
  - **`MetaBelowIntrospect`**: `hacivat-meta` never depends on `hacivat-introspect`, `hacivat-session`, `hacivat-driver*`, `hacivat-exec` or `hacivat-l10n`, not even as a dev-dependency; it may depend on `sql-catalog` and `sql-ident` (core → sql is allowed). Its features: `catalog` (`tokio` with `default-features = false`, feature `sync` only — broadcast, never a runtime type), `persist` (`rusqlite`, `chacha20poly1305`), `system` (`zstd`, `serde`, `serde_json`: snapshot decoding), `embedded-system` (implies `system`; the `include_bytes!` table of the shipped snapshots), `test-support` (`fixture`). Default = `catalog`, `persist`, `system`. The model, `IdentKey`, `ModelView`, `SnapshotView`, `merge`, `fingerprint` and `search` build with no feature.
  - **`EmbeddedSystemOnlyFromApp`**: only `hacivat-app` (and `hacivat-meta`'s own tests) enable `hacivat-meta/embedded-system`, so the ~0.7 MB of snapshots lands in no other binary.
  - **`IntrospectNotExec`**: `hacivat-introspect` never depends on `hacivat-exec` or `hacivat-results` (catalog SQL never travels the execution path, R-M4-7); `hacivat-exec` may depend on `hacivat-introspect` (the refresh-after-DDL sink, Task 21).
  - **`MetaQueryOnlyFromPacks`** (a source rule, the second source-level scan in `check_deps.rs` after `BackendOutsideOwner`'s `tree_violations`, `check_deps.rs:458-461`, whose walker it reuses): the token `MetaQuery::new` appears only in `crates/core/hacivat-introspect/src/pack/**` and in `crates/core/hacivat-session/src/meta.rs` (its definition and unit tests); test files under `tests/` are exempt (R-M4-7, I13).
  - `sql-safety` stays in `crates/sql/*` (no `tokio`, `futures`, `hacivat-l10n`, no core crate); its `ddl` field (Task 6) uses only `sql-ident` types. **`sql-catalog`** (new) is a `crates/sql` crate under the existing `SqlNotRuntimeFree` and `SqlReachesCore` rules; it depends on `sql-ident` and `smallvec` only.
  - `PRODUCT_CRATES` gains `hacivat-meta`, `hacivat-introspect`, `hacivat-ddl` (core) and `sql-catalog` (sql). No new UI crate (the tree widget lives in `hacivat-ui`).
- **Security review.** Tasks **1a, 1c, 4, 5, 6, 11, 12, 15b, 16, 16b, 17, 17b, 23** are security-sensitive: an **opus** reviewer runs the task's adversarial probes and writes and runs at least one hostile test the implementer did not write. **Tasks 5 and 11 also get a second, independent opus review** that is given the code, not the first review (Task 5 guards "introspection never writes and never rides a user transaction"; Task 11 guards encryption at rest and untrusted bytes on disk). Every other task gets an opus review (M3 practice); sonnet only where the waves table says so.
- **ADRs.** New **ADR 0008 "Metadata and introspection"** (Task 1a creates it with rulings R-M4-1..50); sections are appended as `## m4-tNN — <topic>`. Session seams → ADR 0004 `## m4-tNN`; `sql-safety` → ADR 0005; exec sink → ADR 0006; tree widget and explorer → ADR 0007. A security choice made in a fix round goes into the same section before re-review. ADR numbers of later milestones: M4b 0009, M5 0010, M7 0011, M8 0012 (R-M4-36).
- **Append-only shared files.** Block headers `## m4-tNN-<slug>` (FTL, TOML, keymaps), `// ## m4-tNN-<slug>` (Rust), `# ## m4-tNN-<slug>` (`ci.yml`, `docker/m2a/init/*.sql`): `locales/{en,tr}/{hacivat-ui,hacivat-core,hui}.ftl`; `locales/_schema/literal-allowlist.toml`; the keymaps (Task 1b only); ADRs 0004–0008; `.github/workflows/ci.yml`; `crates/app/hacivat-app/tests/support/mod.rs`; `crates/ui/hacivat-ui/tests/support/mod.rs`; `crates/core/hacivat-session/tests/common/mod.rs`; `crates/core/hacivat-introspect/tests/common/mod.rs` (new, Task 1c); `crates/sql/sql-testkit/corpus/**` (new files only); `docker/m2a/init/*.sql`. Named-line files (an edit is listed with its lines): `docker/m2a/compose.yml`, `docker/m2a/targets.toml`, `xtask/src/docker.rs`. **Every block a task writes is fenced, and every edit inside another task's file is a named exception listed in the task's Files line.**
- **Docker (only through the project's wrapper).**
  - The compose project `hacivat-m2a` is controller-owned; implementers never run `cargo xtask docker up`/`down`, never recreate a service and never touch a volume. **The controller touches containers or volumes only with the PO's OK** (controller steps C-2, C-6, C-9).
  - Docker tests are `#[ignore = "docker"]` and run only through `<main>/target/docker/run-docker-tests.sh --set pr|full -p <crate> [--test <name>]` after `cargo test -p <crate> --no-run`; the wrapper exits 1 when its lock is not acquired within 60 min and never runs unlocked. Targets only through `hacivat_testkit::{require_target, targets}` (ids from `docker/m2a/targets.toml`: `pg13`…`pg18`, `pg17-tr`, `mysql80`, `mysql84`, `mysql84-x509`, `mariadb106`, `mariadb1011`, `mariadb114`, plus `pg17wrap`, Task 1c). Docker-produced files only through `hacivat_testkit::artifact(name)`.
  - **Fixture logins** (R-M4-43, Task 1c, effective after C-2): `hv_test` (as today, plus `ALL` on databases matching `hvm4\_%`); `hv_catalog` (snapshot reader: `SELECT` on `mysql.*` and `performance_schema.*`, `SELECT, SHOW VIEW, EXECUTE` on `sys.*`, nothing on fixture databases); `hv_fixture` (fixture-only, used by test setup code through `hacivat-testkit`, never by product code: the minimal privileges for `SET GLOBAL time_zone` and for `CREATE USER`/`GRANT`/`REVOKE` on `hvm4_*` test users, picked from the vendor documentation and listed in the T1c report). MySQL 8.0/8.4 services run with `--log-bin-trust-function-creators=1` so `hv_test` can create functions and triggers (F5).
  - **Test objects:** a test creates its own uniquely named database (`hvm4_<slug>`, MySQL/MariaDB) or schema (`hvm4_<slug>`, PG; `hvpgi_*` style per the PG spike) and drops it in a drop guard, also on panic. The one shared fixture is `hv_m4_big` on `pg17` (Task 16), dropped at the gate (Task 33).
  - PR-set additions ≤ 5 min wall, full-set additions ≤ 15 min; `docker-pr` 60 min, `docker-full` 90 min. **Docker is up from W1's end (T5) to W6.** A test that changes server-global state (`SET GLOBAL time_zone`, `REVOKE`) restores it in a drop guard and relies on the wrapper's lock for exclusivity. CI owns its own ephemeral compose stack (the `minicat-verify` job); locally the controller alone brings services up or down (disk rule: no stray containers, volumes or image tags).
- **Full test set.** "All tests pass" means every line below (M3c's lines plus the M4 lines). PR CI: `build-test` ×3 and `turkey-test` run lines 1–4 and 6–8 (`ci.yml:104-124`, `:216-230`; line 9 runs locally and at the gate only) plus `cargo test -p hacivat-meta --all-features` (Task 1a's `ci.yml` block `# ## m4-t01a-ci`); `docker-pr` runs line 5 and the `pr` set of the crates in `xtask docker`'s `DEFAULT_TEST_CRATES` (`xtask/src/docker.rs:28-36`; Task 1c adds `hacivat-introspect`, or its Docker tests never run in CI). **Scheduled jobs do not fire on `m4`** (`ci.yml` `schedule` runs on the default branch only, `ci.yml:9-15`): line 10, `docker-full`, `bench`, `fuzz-sql`, `sql-differential`, `fuzz-meta` (the job is Task 1a's `# ## m4-t01a-ci` block; target bodies Tasks 4, 11, 12, 23), `minicat-verify` and the package job run by **`workflow_dispatch` on the `m4` tip** (R-M4-45; the M3 precedent, ledger lines 89, 93); the controller records each run id in the ledger and in `m4.md`. The gate runs all.
  ```
  cargo test --workspace
  cargo test -p hacivat-ui-core --features test-support,scene-check
  cargo test -p hacivat-l10n --features pseudo
  cargo test -p hacivat-app --features scene-check,pseudo
  cargo test -p hacivat-driver --features fake
  cargo test -p hacivat-testkit --features ssh
  cargo test -p hacivat-ui-editor --features scene-check
  cargo test -p hacivat-ui-grid --features test-support,scene-check
  cargo test -p hacivat-ui --features scene-check
  cargo test --release -p hacivat-l10n --features pseudo
  cargo test -p hacivat-meta --all-features
  cargo xtask docker check-pins
  cargo run -p xtask -- minicat --check
  <main>/target/docker/run-docker-tests.sh --set full
  cargo test -p hacivat-app --features shots,scene-check,pseudo --test shots
  <the M3c perf lines, unchanged>
  HACIVAT_PERF=1 cargo test --release -p hacivat-meta --test snapshot_perf --test persist_perf --test merge_perf --test model_perf -- --ignored
  HACIVAT_PERF=1 cargo test --release -p hacivat-introspect --test engine_perf --test filter_perf -- --ignored
  HACIVAT_PERF=1 cargo test --release -p hacivat-ui --test tree_perf -- --ignored
  HACIVAT_PERF=1 cargo test --release -p hacivat-app --test explorer_perf -- --ignored
  ```
  A task that adds a perf test outside these crates adds its `-p`/`--test` and says so. **A crate that holds Docker tests (`hacivat-introspect`, `hacivat-session`) appears in a perf line only through `--test <target>`** (M3a P9 lesson). Every new test file compiles under plain `cargo test --workspace`; each task runs `cargo test -p <crate> --no-run` without features.
- **Test values are fixed in the plan.** Where a test below writes a value, the implementer makes the code produce it; a value believed wrong is reported (NEEDS_CONTEXT), never silently changed. Where the plan cannot know a value (an object added in one server version, a byte size), the test says "picked from <source> and listed in the report" — the plan never invents a server fact.
- **Test timing.** Condition waits only (`run_until_parked`, `settle_until`, `wait_until`/`poll_until`, gated fakes); no fixed sleep before an assertion. Perf and wall-time asserts are `#[ignore = "perf"]` and self-gate on `HACIVAT_PERF=1`. The MySQL 1-second marker granularity is tested by reading server time (`UTC_TIMESTAMP(6)`) and waiting on a condition until the next server second, never `sleep(1)`.
- **Cross-host tests** (M3c lessons, carries-w9/w11): CI runs Linux and Windows. A command registered only on one host goes into the `HOST_ONLY` list of `keymap_commands_are_registered` (`crates/app/hacivat-app/tests/commands.rs:419`, list at `:427`); a label test accepts the long form **or** its short-form fallback (wider non-macOS fonts, ledger 869); golden and fixture files are `-text` in `.gitattributes` so Windows checkouts keep LF (Task 1a).
- **No silent weakening (F31), M4's list of fail-closed stubs.** Until its filling task merges:
  - `ModelView::lookup`/`children`/`find_by_server_id` answer `Unknown(NotLoaded)` (never `Found`, never `Missing`); `CatalogSnapshot::fk_referencing` answers `Unknown`; every `sql_catalog::CatalogView` method of `EmptyCatalog`, and of `SnapshotView` before Task 26, answers `Unknown(NotLoaded)` (`virtual_fks` → empty, `row_estimate` → `None`).
  - `merge` answers `Err(MergeError::NotImplemented)` and nothing is published; `CacheStore::open` answers `Err(CacheError::Disabled)` (memory-only); `SystemCatalogs::for_server` answers `None` (system lookups `Unknown`).
  - `MetaConn` acquisition and `MetaConn::join` answer `Err(MetaError::Unavailable)` (namespace `LoadState::Failed`, the explorer shows "Not loaded"); every `Introspector` method, `stamps` and `facet` included, answers `Err(IntroError::NotImplemented)`; `IntroHandle::load_facet` resolves to `Err(NotImplemented)` and leaves the facet `NotLoaded`; `fragment::on_session` answers `Err(NotImplemented)`; `CatalogHub::ensure` answers a catalog with no engine (cache only, every namespace `NotLoaded`).
  - `check_state` answers `Staleness::Unknown` for every key (editing flows treat it as `Changed`); `plan_refresh` answers `Target::Namespace(current)` for every statement that has any `ddl` effect or is opaque (refresh more, never less); `sql_safety` `ddl` is `vec![DdlEffect::opaque()]` for every statement in the Task 6 scope (DDL-class statements — at the base `StmtKind::{CreateTable, CreateOther, Alter, Drop, DropOwned, Truncate, Grant, Revoke}`, `DropOwned` being M3c's P2-07 kind, `crates/sql/sql-safety/src/report.rs:65` in the enum at `:47` — plus `DO`, `CALL`, `EXECUTE`, `PREPARE`, MySQL executable comments).
  - The scope matcher selects only `@:@`; the object filter matches everything; every `hacivat-ddl` renderer answers `Err(RenderError::Unsupported)`; a DDL producer whose grants facet is not loaded emits the notice `ddl-grants-not-loaded`, never a script that silently lacks grants; the tree widget renders a flat, non-expandable list.
  - **No stub is ever wired to a user-visible action that would show a wrong fact** (a wrong "missing" table, an empty schema shown as loaded, a DDL text that is not the object's).
- **Introspection is read-only and trusted-text-only (R-M4-7).**
  - Catalog SQL comes only from the packs (`include_str!`); names and ids reach the server only as binds (R-M4-3); macros come only from a closed enum; the only identifier macro is `#{ident:…}` in `--- show-create ---` sections, rendered through `sql_ident::quote` (R-M4-37).
  - `MetaConn` accepts only a `MetaQuery` (built by the pack renderer, check-deps `MetaQueryOnlyFromPacks`); it runs only on `SessionKind::Introspection` sessions; it never passes through `hacivat-exec`, the classifier or the history store, and never appears in the user's output log.
  - **Server enforcement per engine:** PG `BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY` with `SET LOCAL search_path = pg_catalog, pg_temp`; MySQL/MariaDB `SET SESSION TRANSACTION READ ONLY` at connect and after every reconnect, asserted before every snapshot, then `START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY`; SQLite a separate `mode=ro` connection (R-M4-38). Docker tests prove a metadata pass assigns no XID (`#pg-canary`) and that DML **and DDL** are refused on every engine (Task 5).
  - **One exception (R-M4-29):** a **joined** `MetaConn` (`MetaConn::join`) runs `joinable` pack sections inside a transaction that Hacivat itself owns on a `SessionKind::Job(_, JobKind::{DdlApply, DriftCheck})` session (M7's drift check inside the apply transaction). The server enforces nothing there; the enforcement is the typed `MetaQuery` (`joinable` flag) plus a refusal of any text whose first keyword is `BEGIN`, `START`, `COMMIT`, `ROLLBACK`, `SAVEPOINT`, `RELEASE` or `SET`. A joined connection never begins, commits or rolls back, and is refused on `Console` and `Editor` sessions.
- **Machine text is locale-free; names are user data.** `IdentKey` is built only by `sql_ident::key::match_key` (HV-INTRO-14, R-M4-28); no catalog predicate uses `lower()`, `upper()`, `ILIKE` or `COLLATE` on a catalog name column (pack lint, Task 4); catalog-object names in packs are in canonical case, and PG packs write every catalog as `pg_catalog.<name>`. Object names are displayed verbatim (never case-mapped, never translated, bidi-isolated, invisible characters badged, HV-MODEL-01 i18n notes); display sorting uses ICU4X collation of the UI locale and never touches identity or DDL order. Counts and dates are preformatted (L3; P31).
- **Copy (PO preferences).** English copy is final; Turkish drafts are marked `# Review: PO`, and **Turkish copy keeps the words "transaction", "commit" and "rollback" in English** (for example "COMMIT sonrası yenilenir"), as the PO asked. Generated SQL is never localized and is byte-stable (alg/03 §0.3); everything around it is Fluent.
- **The UI thread never awaits catalog I/O.** Cache open, decrypt, decompress, deserialize, the `CatalogKey` keyring fetch and every catalog query run on the `CoreRuntime` or its blocking pool; the explorer reads the current snapshot synchronously during render (spec §5.4 rule 2); notifications are coalesced to one `cx.notify()` per frame per entity (rule 3). At app start the key is fetched only for encrypted profiles that are expanded in the restored workspace (R-M4-15, Task 24).
- **Design rules (DESIGN.md).** Unchanged: One Filled Badge, Hue Is Meaning (red only for production and destructive), the Raised Ink Rule, the 11 px floor, no ambient motion (progress is a static phase label plus a determinate count, repainted when the count changes), Tabular Everywhere, the Explorer Tree spec (28 px filter field, small-caps group headers with mono counts, 23 px rows with chevron + kind icon + label, blue-tint selection with a 1 px blue left edge).
- **Keys.** Keymap rows are Task 1b's only; widgets never call `cx.bind_keys`; **defaults follow VS Code conventions and every row is remappable** (PO preference; PO-Q10); every chord passes the AltGr CI (TR-Q, TR-F, DE, FR, PL; `crates/ui/hacivat-ui/tests/keymap_collisions.rs`, the `hacivat` preset accepts no shadowing); no binding fires during an IME composition. M3 rulings that bind every M4 row: no row takes `alt-shift-f` (reserved for M5's reformat, C-36), and no chord starts with `cmd-k`/`ctrl-k`, the command palette's chord (fold all / unfold all stay palette-only; P2-08 rejected, C-47).
- **Shots harness.** Unchanged rules, plus M3c's harness facts: one platform text system per process (`shot_text_system()`, `crates/app/hacivat-app/tests/shots.rs:1370`) and a live-thread guard that fails a run whose threads grow by more than `THREAD_SLACK` = 32 (`shots.rs:1748`, C-45), so an M4 state stops whatever it starts (`support::stop_core_runtime`, `tests/support/mod.rs:1169`); `--shard k/n` (`shots.rs:2083`); `KNOWN_FINDINGS` lists are temporary and empty at the gate (C-26). Matrix states render at 1280 × 800 and 960 × 600 (`shots.rs:445`). Baselines at zero tolerance: `baseline-m3b` and, once the PO signs the M3c gate, `baseline-m3c` (both under `<main>/.superpowers/sdd/2026-11-14-m3-walking-skeleton/`). **`baseline-m3c` does not exist yet** (PO sign-off pending, m3c.md §5.2): until it does, every comparison uses **`baseline-m3b` plus the m3c-listed changes** (the sets of m3c.md §5.2: `t45-gate-shot-diff-vs-m3b.txt`, `task-m3c-41-shot-diff.txt`, `task-m3c-41-fix1-shot-diff.txt`, `m3c-shot-fixes-changed-approved.txt`, the T38/T34/T42 lists, the 176 `narrow-*` and 16 `marks-denied-*` PNGs, in that SDD folder); a breach outside those lists is an M4 change and is listed. Baselines stay out of git until M9 (C-37). **CI:** `pseudo-screens` renders the whole matrix on macOS in one process and on Linux in two shards (`ci.yml:566-582`, 180-minute bound each); its macOS step still pixel-compares the 36 M1 states `palette-open`, `shell`, `shell-demo-production`, `shell-narrow` against `design/baselines/m1/macos` with `superseded.txt` (`ci.yml:653-656`): an M4 change to one of them is listed (PO item) and its surface added to `superseded.txt` by the task that changes it (named exception), never by loosening the threshold. The explorer becomes a tree (R-M4-18), so every approved shot that shows the explorer changes on purpose; Task 14 and Task 32 list each changed PNG (PO item) and Task 32 produces `baseline-m4` in this folder. Shots land in `<worktree>/target/shots` (not `$CARGO_TARGET_DIR`); delete `target/shots*` at the end of a task.
- **Rate limits.** The controller watches its own session's rate file and pauses new dispatches at the PO's current threshold (ledger 682: stop new dispatch above 75 % weekly unless the PO lifts it).

## Review Focus

Six input classes the spec implies, no single feature test exercises, and that are most likely to hurt a person using M4. Most likely first; each names the tests that pin it and their owner.

1. **A catalog state that looks like mass deletion but is not** (lost privilege, wrong database after a profile edit, a failed existence query, a server swapped behind the same host name, a restore).
   - Expected: no deletion from a failed or permission-denied existence query; > 50 % of a namespace with ≥ 20 objects vanishing asks (user-triggered) or marks stale (background) and changes nothing until confirmed (D-039); a server fingerprint change marks everything stale but keeps rows for offline use.
   - Tests: `failed_existence_disables_deletions_and_marks_stale`, `mass_drop_background_is_pending_confirmation` (Task 10), `server_change_marks_all_stale_keeps_rows` (Task 11), `revoke_usage_fires_mass_drop_guard` (Task 28, Docker), `user_refresh_mass_drop_asks_and_cancel_keeps_cache` (Task 24).
2. **A change the incremental pass could miss.** `CREATE INDEX` does not move the table's `pg_class.xmin`; a second trigger moves nothing on the table; `COMMENT` touches only `pg_description`, and `COMMENT … IS NULL` **deletes** the row (no young xmin anywhere; PG spike finding 3); a rename of a type, column, table or function changes the deparsed text of *other* objects without touching their rows (finding 4) — and an index backing a constraint is two hops from its table (finding 5); XIDs assigned between the probe and the change query (finding 6); a MySQL `CREATE TRIGGER` leaves `CREATE_TIME` alone; a long MySQL `ALTER` is stamped with its **start** time (F1); two DDLs in one second; a transaction open at the watermark; XID wraparound; a restart or restore; a SQLite file replaced with the same `schema_version`.
   - Expected: after every step the incremental snapshot equals a fresh full snapshot.
   - Tests: `pg_xmin_matrix_incremental_equals_full`, `pg_xmin_freeze_between_reads_zero_ancient_rows`, `mysql_is_times_matrix_incremental_equals_full`, `mysql_same_second_ddl_caught`, `long_alter_straddling_a_pass_is_seen` (Task 28); `uncommitted_ddl_appears_on_next_pass`, `comment_removal_is_seen_by_existence`, `rename_of_a_referenced_object_rereads_dependent_text`, `xid_burst_between_probe_and_change_query_misses_nothing` (Task 16); `check_state_sees_a_renamed_dependency` (Task 22); `same_schema_version_replaced_file_detected_on_reconnect_by_digest` (Task 18).
3. **Names that break folding, quoting or display.** Turkish `İ`/`ı`, case-only twins (`"Users"` and `users`), names with `:`, `,`, `|`, `{`, `!`, a backtick, spaces, a trailing space, NUL or invisible characters, non-NFC names, the 63-byte PG limit, MySQL `lower_case_table_names`, MySQL column names with non-ASCII cased letters, the process locale `tr_TR`.
   - Expected: the server's own matching rules, never the process locale; where the server rule is not yet pinned, `Unknown`, never `Missing`; no `lower()`/`ILIKE` in catalog SQL; DDL and `SHOW CREATE` quote exactly where needed.
   - Tests: `pg_ident_key_ascii_only`, `mysql_lctn1_ascii_i_never_becomes_dotless`, `mysql_non_ascii_cased_column_is_server_dependent_until_pinned`, `sqlite_ident_key_ascii_lowercase`, `match_key_is_locale_free` (Task 3); `quoted_names_with_separators_round_trip` (Task 12); `show_create_ident_macro_quotes_backtick_and_refuses_nul` (Task 4); `pg_tr_icu_no_lower_or_ilike_in_query_log`, `catalog_bytes_identical_under_lang_tr_and_c` (Task 28); `turkish_identifier_quoted_on_pg` (Task 7); `names_verbatim_no_case_mapping`, `invisible_chars_badge` (Task 14).
4. **Untrusted bytes at rest.** A corrupt, truncated or tampered cache row; a bit flip; a file from a newer minor format; the wrong key; a truncated snapshot asset.
   - Expected: decode never panics; only the affected row is dropped and re-introspected; a newer minor is read but never rewritten; the cache file never contains the password.
   - Tests: `bit_flip_drops_only_that_row`, `wrong_key_rebuilds_without_panic`, `newer_minor_read_never_rewritten`, `password_never_in_cache_file` (Task 11); `decode_of_truncated_asset_is_error_not_panic` (Task 23); fuzz targets `catalog_row_decode`, `system_snapshot_decode` (dispatched `fuzz-meta`, Tasks 11 and 23).
5. **Scale.** 15,000 tables / 160,000 columns in one schema; 5,000 schemas; 2,000 databases; a 100,000-row tree; a filter typed over 160,000 names.
   - Expected: no N+1 catalog queries; no UI-thread work above 1 ms; the cached catalog is on screen in < 300 ms.
   - Tests: `bulk_upgrade_after_third_request_total_queries_le_4` (Task 15), `publish_5000_namespaces_under_1ms` (Task 9), `only_visible_rows_are_built` (Task 8), `filter_160k_names_off_ui_thread` (Task 14), `load_160k_column_namespace_under_budget` (Tasks 11, 29), G1 (Task 29).
6. **A metadata connection that could write or join a user's transaction.** A MySQL read-only transaction that a `CREATE TABLE` silently ends (ver/01 `#mysql-ro` row 13); a reconnect that loses the session's read-only flag; a joined connection handed transaction-control text.
   - Expected: DML and DDL refused on every engine's metadata connection; a joined connection refuses non-`joinable` sections and transaction-control text, and never begins or ends a transaction.
   - Tests: `mysql_meta_txn_refuses_dml_1792`, `mysql_meta_session_refuses_ddl`, `mariadb_meta_session_refuses_ddl`, `read_only_flag_is_restored_after_reconnect`, `joined_conn_refuses_transaction_control_text`, `joined_conn_refuses_non_joinable_queries` (Task 5); `no_xid_assigned_by_a_metadata_pass` (Task 5), `pg_canary_no_xid_assigned_by_any_pass` (Task 28).

## Assumptions and rulings

Rulings are the planner's calls where the sources leave a choice or disagree. Each is recorded in ADR 0008 by Task 1a and lands in the gate's PO list. Those that change what a user sees are also questions in `po-questions-m4.md` (marked **PO-Qn**); the default below is in force until the PO answers.

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

**Inherited and still binding:** every M3 ruling about sessions, the guard, the shots harness and the process (ledger and `amend-m3b-rulings.md`). In particular: production consoles are writable only behind confirmations (M3b T26), so R-M4-13's generated `SELECT` runs through the same gate.

**Open questions for the PO:** `po-questions-m4.md` (same folder): **22 questions** (Q1–Q22), each with a recommended default that is in force and logged; none blocks dispatch. Three of them are approvals the controller needs before touching Docker (Q18, Q20) or that change the first-start behaviour (Q19).

## Deferred work (explicit; beyond M4)

| Item | Where it lands | Why not now |
|---|---|---|
| Completion popup, hover, diagnostics over the catalog; warm completion < 1 s after cold start (§3.3) | M5 | §6 M5 scope; M4 provides `sql_catalog::CatalogView` (R-M4-5). |
| Session overlay of uncommitted DDL; negative-cache probes triggered by editor diagnostics | M5 | R-M4-11; the probe API (`check_state` by name) ships in M4 (Task 22). |
| Table editor with keyset paging replacing R-M4-13's console "Open data" | M6 | HV-RES-06 is M6. |
| Modify Object, source edit round-trip, drift check, DDL apply; Drop/Truncate/Alter generator categories; virtual-FK editor | M7 | HV-DDL-01..06, -12, -13, -18, -19 are M7; they consume Task 22's `check_state` and Task 19's producers. |
| MCP `list_tables`, `describe_table`, `get_ddl`, `refresh_metadata` | M8 | Read the snapshots and `create_script` M4 publishes. |
| Screen-reader walkthrough of the explorer tree | M9 (L4) | M4 ships tree/treeitem roles and names only (Task 8). |
| MariaDB incremental marker; SQL Server, ClickHouse, Oracle introspection and producers; remote introspection at Karagöz (HV-INTRO-17); the shipped `sessions` virtual view and an Activity panel; diagrams | v1.x | Spec §4 introspection row "v1.x"; §2.2. |
| PgBouncer-in-front Docker test (HV-INTRO-01 test 7) | M8 (with the PgBouncer service of §7.1's matrix) | The compose stack has no PgBouncer service yet. |
| PG objects the M4 introspector does not read: RLS policies, rules other than `_RETURN`, extended statistics, foreign-table servers and options, publications, tablespaces, extensions as objects, `conperiod` (WITHOUT OVERLAPS), column compression, multirange types, comments on domain constraints | v1.x (M7 where Modify needs one) | PG spike §7 Q4: each needs its own young-row source and existence entry. Their `ObjectKind`s exist (R-M4-4). Task 19 never silently omits them: a table with `relrowsecurity` gets `ENABLE ROW LEVEL SECURITY` plus the notice `ddl-policies-not-loaded`. |
| MySQL/MariaDB built-in function signatures | M5 (curated data, alg/12 COMPL-06) | Not introspectable (minicat spike §8 Q3); snapshots carry names only (`FnSig.args = None`). |
| `mysql84-lctn1` service (MySQL `lower_case_table_names = 1/2`) | v1.x or a later controller call | Disk; R-M4-41 keeps those keys `ServerDependent`. |
| Tree type-ahead for IME scripts (CJK object names); multi-select in the explorer | v1.x / the first milestone with a bulk action | Tree spike concerns 3, 6; R-M4-50. |

- **HV-EXEC-13 explain plan model and views** (routed to M4 by m3a.md:464, R-M3-21): out of M4's catalog scope; → M4b pre-flight (controller ruling U-6(d), ledger 2026-11-19).

## Cross-plan interfaces (what later plans consume; their pre-flights apply the "X" rows)

M4 provides the interfaces of `reconciliation.md` §3 (49 rows; 6 stay open, §6 there). The names are fixed by R-M4-26..36. The rows below go to the other plans' pre-flights; they are not M4 work.

| # | Plan | Change |
|---|---|---|
| X-1 | M4b, M5, M7, M8 | ADR numbers: M4b → 0009, M5 → 0010, M7 → 0011, M8 → 0012 (R-M4-36). |
| X-2 | M4b | C-M4-1/2/3/7/8 are provided by M4 (Tasks 1b, 14, 24): Task 1b drops its named exceptions in M4's tree file; `ScriptsContributor` implements the extended `ExplorerContributor`; `ContributedNode.filter_text` carries the file name. |
| X-3 | M6 | Task 1a drops its `sql-ident/src/quote.rs` stub (M4 Task 7 owns it); `valfmt::literal` delegates text to `sql_ident::quote::string_literal`; `MetaBridge` reads `Props` and `row_estimate`; `OriginCatalog` uses `ModelView::find_by_server_id`; per-profile data goes into its own section of `profiles/<uuid>.toml` through `hacivat_settings::profile_data`; open data / export / import register through the action provider and the open-data handler. |
| X-4 | M5 | Task 1a does not create `sql-catalog` (M4 Task 1a did); Task 6 extends `sql_ident::key` and `sql_ident::quote`; row-7 expectations follow the Task 17-pinned fact (R-M4-41); `IdentCtx` is M4's. |
| X-5 | M5 | `sql_catalog::CatalogDelta` → **`DdlDelta`** (M4 keeps `hacivat_meta::CatalogDelta`). |
| X-6 | M7 | Use `hacivat_meta::ModelView`; build `WorkingModel` with `replace_subtree`/`upsert_child`; put `apply/`, `store/`, `drift/` behind `hacivat-ddl/apply`; fragment-on-session, stamps, fingerprint, crypt and tickets exist; drift sections go in a new `packs/<dialect>/70-drift.hvq.sql` only if `StateOf<Kind>` is not enough; IR variants are declared by M4. |
| X-7 | M8 | Reach catalogs through `CatalogHub`; use `generation`; `search_objects` uses `hacivat_meta::search`; row estimates are the `Statistics` facet. |
| X-8 | M5 | `FnSig.args` is **`Option<Vec<FnArg>>`**: `None` means "name only" (MySQL/MariaDB built-ins from the snapshots), never "takes no arguments" (review I5). |
| X-9 | M7 | A joined connection takes only `MetaQuery`s from `joinable` sections and refuses transaction-control and `SET` text; `joinable` PG sections return type schema/name columns, not `format_type` text (R-M4-46). |
| X-10 | M5, M6 | SQLite CHECK constraints and generated-column expressions are not modelled as objects in M4 (only `sqlite_schema.sql` has them, F10); MySQL vs MariaDB default and `ON UPDATE` texts are kept verbatim per flavor (F9), so any renderer must know the flavor. |

## New UI surfaces (need mocks; Task 2)

All in `design/m4-explorer/{index.html,NOTES.md}`, harman style, dark and light, EN and TR, 1440 × 900 and the 960 × 600 minimum:
1. **Explorer tree, connected PostgreSQL profile**: database → schemas → kind groups with mono counts (`Tables 1.204` in `tr`) → table → child groups (Columns, Keys, Foreign keys, Indexes, Checks, Triggers) → leaves with type text at text-2; environment swatch and lock flags on the root row; MySQL ("databases") and SQLite (`main`, `temp`, attached) variants.
2. **Loading states**: names-first (`Brief`) paint, a node's phase line ("Reading columns… 3,400 of 15,000"), a partial-load badge ("Some objects could not be read") with its tooltip.
3. **Cached and offline**: "Showing cached metadata from 17 Kasım 2026 14:05" strip; "Out of date" badge on a namespace; the "Built-in" group with "Built-in catalog (version 17)" badge.
4. **Filter field**: matches with revealed ancestors, the Turkish-tolerant toggle for `tr` (D-074), "No objects match" state.
5. **Context menus**: profile, database, schema, kind group, table/view, column; disabled "Modify…" (M7) with its tooltip.
6. **Introspection level**: the pill (blue explicit, grey inherited, datagrip/04 §3.1) and its submenu (Automatic, Names only, Structure, Structure and sources).
7. **Scope editor**: connection dialog section "Schemas": a checkbox tree, the pattern field with an error at a character position, the all-databases cost prompt (PG), "Encrypt cached metadata" checkbox.
8. **Mass-deletion confirmation** (D-039): "{count} objects in {schema} are no longer visible…" with Keep cached / Remove from cache.
9. **Refresh after DDL**: the Ask prompt ("Refresh the database explorer to reflect {count} changed objects?") and the broad-refresh notice for dynamic SQL.
10. **DDL view tab**: read-only SQL with the options strip (Qualification Auto/Never/Always, Constraints Inside column/Inside table/After table, Definitions provided by server, Grants, Owner), Copy, Open in console, Refresh; tab title; unsupported-kind notice; MySQL executable-comment warning.
11. **Stale object**: "{name} changed on the server since it was loaded" with Reload, in the DDL view and as an explorer tooltip; "{name} no longer exists on the server".
12. **Invisible characters and broken objects**: the invisible-chars badge on a name; "{kind} {name} references a column that could not be resolved".
13. **Settings, "Metadata" section**: refresh after DDL (Always/Ask/Never), background check interval, cache limits, "Clear cached metadata for this connection…".
14. **Status bar during a long introspection** (optional segment; may fold into the explorer only — the mock decides and NOTES records it).

## Waves

| Wave | Tasks (agent model; **gpui** marked) | Unblocked by |
|---|---|---|
| W0 | **T1a** manifests and core types (opus) → on its first commit: **T1b** UI and app seams (opus, **gpui**) · **T1c** session, settings, introspection and fixture seams (opus) · **T2** mocks 1–14 (opus, no build) | C-0 (T1a); Task 1a's manifest commit (T1b, T1c Part A); T1a merged (T1b's `hacivat_meta`-typed signatures, T1c Part B) |
| W1 | **T3** model, kinds, identifier keys (opus) · **T4** query packs (opus) · **T5** metadata sessions (opus) · **T6** `sql-safety` DDL effects (opus) · **T7** DDL IR, renderers, quoting (opus) · **T8** tree widget (opus, **gpui**) | T1a, T1b, T1c merged; C-1 dispatched |
| W2 | **T9** snapshots, `ModelView`, search (opus) · **T10** merge, delta, mass-deletion guard (opus) · **T11** codec, cache, encryption, fingerprints (opus) · **T12** scope and filter grammars (opus) · **T13** composite assembly (opus) · **T14** explorer tree adapter (opus, **gpui**) | T3 (T9–T11, T13); T3, T4 (T12); T3, T8, **T2 approved** (T14); C-3 ruled if T11 reported a perf concern (W3 only) |
| W3 | **T15** engine, scheduler, tickets, hub (opus) · **T16** PostgreSQL introspector (opus) · **T17** MySQL/MariaDB introspector (opus) · **T18** SQLite introspector (opus) · **T19** DDL producers and goldens (opus) · **T20** scope editor, sensitive flag, level pills (opus, **gpui**) | T4, T5, T9, T10, T11, T12 (T15); T4, T5, T10, T13 (T16–T18); **C-2 done or declined** (T17); T3, T7 (T19); T12, T14 (T20) |
| W4 | **T16b** PostgreSQL facets and server-level sections (opus) · **T17b** MySQL/MariaDB facets and server-level sections (opus) · **T21** refresh after DDL (opus) · **T22** freshness and background validation (opus) · **T23** `minicat` generator, assets, CI job (opus) · **T24** explorer live wiring (opus, **gpui**) · **T25** DDL view tab (opus, **gpui**) | T16 (T16b); T17 (T17b); T6, T15 (T21); T15–T18 (T22); T11, T16, T17 and **a complete MySQL/MariaDB snapshot set available** (C-2 local read, or the C-1 artifact reviewed) (T23); T9–T11, T14, T15, T20 (T24); T14, T15, T19 (T25) |
| W5 | **T15b** facet loading and fragment on a caller's session (opus) · **T26** overlays, system-catalog layer, `sql_catalog::CatalogView` (opus) · **T28** **G4** incremental refresh on the ver/01 matrix (opus) · **T27** refresh-after-DDL, freshness and settings UI (opus, **gpui**) | T15, T16b, T17b (T15b); T9, T23 (T26); T15–T18, T21, T22, **C-6 done** (T28); T21, T22, T24 (T27) |
| W6 | **T29** **G1** cached catalog < 300 ms (opus, **gpui**) · **T30** **G2+G3** snapshot reproducibility, offline built-in resolution (opus) · **T31** **G5** DDL golden stability and round trip (opus) · **T32** shots, cross-surface, offline explorer, DESIGN.md (opus, **gpui**) | W5 merged (T29: T11, T24; T30: T23, T26, C-7 runs; T31: T19; T32: T24, T25, T26, T27) |
| W7 | **T33** M4 gate (opus, with the controller) | everything; C-9 after the PO demo |

**Counts:** **38 tasks** (T1a, T1b, T1c, T2, T3–T33, plus the split tasks T15b, T16b, T17b) in **8 waves** (W0–W7), plus **10 controller steps** (C-0 … C-9). Five tasks are gate measurements (T28 G4, T29 G1, T30 G2+G3, T31 G5, and T33 itself).

**Controller steps** (owner: the controller; each is a ledger line with time and result; **needs PO OK** = not done until the PO approves, PO-Q18/PO-Q20):

| # | When | What | Needs PO OK |
|---|---|---|---|
| C-0 | Before T1a | Commit the plan to `docs/superpowers/plans/`, write `global-constraints-m4.md`, the retargeted implementer/reviewer rules and the ledger heading (section "Execution"). | no |
| C-1 | Right after the plan becomes binding (ledger C-0, R-C0-1), before W1 | **Dispatch `minicat-verify` once by `workflow_dispatch` on the pushed spike branch `m4-spike-minicat`** (`gh workflow run ci.yml --ref m4-spike-minicat`; the job reads as `hv_catalog` on CI's own fresh stack, `--verify --twice`, `down -v`, `up`, `--verify`). The job is expected to fail its comparison against the spike's partial MySQL/MariaDB files; its **artifact** is the deliverable: the first complete set, and the fresh-container stability answer for `performance_schema`, `sys` and PG `information_schema` (R-M4-17 [Unknown]). Record the run id; write every volatile item found as a binding T23 carry (named exclusion rule) in `carries-w4.md`. No local container is touched. | no |
| C-2 | After T1c merges, before W3 | **Recreate only the MySQL/MariaDB services and their data volumes** so the T1c init and command changes take effect: `docker compose -p hacivat-m2a rm -sfv mysql80 mysql84 mysql84-x509 mariadb106 mariadb1011 mariadb114`, remove exactly those services' named data volumes if `compose.yml` declares any (`docker volume rm hacivat-m2a_<name>` per name; never `prune`; at the base it declares none: their data is in anonymous volumes, which `rm -v` removes), then `docker compose -p hacivat-m2a up -d` for the same services. PG services and volumes are never touched. Verify and log on each of the six: `hv_catalog` logs in and reads `mysql.user`; `hv_test` creates and drops `hvm4_probe`; on mysql80/84 `hv_test` creates and drops a function; `hv_fixture` logs in. If the PO declines: R-M4-43's fallback is in force and is written into `carries-w3.md`. | **yes** (PO-Q18) — **given**: M3 P2-12 (2026-11-18, "before M4 W3"), ledger C-0 R-C0-1 |
| C-3 | End of W2 | If Task 11 reports the `load_160k_column_namespace_under_budget` share missed, rule (payload, lazy facets, per-kind rows) before W3 and log it. | no |
| C-4 | End of W1 (before T5's Docker run) and through W6 | Keep the `full` Docker set up (start the existing services if they are down; no recreation, no volume change). At C-0 the stack is down (19 containers exited after a restart) and its start is queued in the PO list; T5's Docker step waits for it. | only if the stack is down and must be started (the case at C-0) |
| C-5 | Before every wave | Write `carries-wN.md` (section "Execution"). | no |
| C-6 | Before W5 | **Build and start `pg17wrap`** (T1c files): build the image `hacivat-m2a-pg17wrap`, start the service, and log that it is healthy and that `pg_snapshot_xmax(pg_current_snapshot())` is above 2^31 (the jumped cluster); on any rebuild, remove the previous `hacivat-m2a-pg17wrap` tag first (no stale tags, disk rule). | **yes** (PO-Q18) — **given**: M3 P2-12, ledger C-0 R-C0-1 |
| C-7 | W6 and the gate tip | Dispatch on `m4` by `workflow_dispatch` (R-M4-45): `minicat-verify` twice (fresh CI stack each) and confirm one PR-triggered run; `fuzz-meta`, `docker-full`, `bench`, the package job; record every run id in the ledger and `m4.md`. | no |
| C-8 | After each adopting task merges | **Spike cleanup** (user disk rules): the spike targets were on the external build disk, which is gone (ledger C-0; nothing unique was there), and none of the six worktrees holds a `target/`; if a spike was rebuilt meanwhile, its target is under `<main>/target-core/` and gets `cargo clean` and deletion first. Then `git worktree remove` (after cleaning any `target`, `node_modules`, `.next` inside): after T3, T9 and T11 merge, `m4-spike-model` (`<main>/.claude/worktrees/agent-a570bafc5f7549a38`); after T16 merges, `m4-spike-pg-intro` (`agent-a54d643223b038165`); after T17 and T18 merge, `m4-spike-mysql-sqlite-intro` (`agent-abf7d36cd4fe7b14d`); after T8 merges, `m4-spike-tree` (`agent-a58f5a6e24ecc3812`); after T23 merges, `m4-spike-minicat` (`agent-ae869a34176b38927`, plus the minicat work directories); after T19 merges, `m4-spike-ddl` (`agent-afaca915a2bb84dd6`). Branches stay until the gate. The same rule applies to any other `m4-spike-*` worktree once the task that owns its subject merges. | no |
| C-9 | After the PO demo (T33) | **Fixture cleanup:** run Task 16's `drop_big_fixture_when_asked` through the wrapper (drops `hv_m4_big` in batches of 250 tables per `DO` block) and record the PG volume size before and after; then, only with the PO's OK, `docker compose -p hacivat-m2a down -v` for the PG services to reclaim the volumes. | the `down -v` part only (PO-Q20) |

**gpui schedule (2 slots).** W0 T1b · W1 T8 · W2 T14 · W3 T20 · W4 T24 · T25 · W5 T27 · W6 T29 · T32. No wave needs a third slot (T30 is core-only: the offline explorer test moved to T32). If disk drops below the 25 GiB cold-build floor, the controller runs the wave's second gpui task after the first merges (the trailing task in each row has the fewest dependents: T25 in W4, T32 in W6).

- **Core admission:** W1–W4 have 5 core agents each; W5 has 3, W6 has 2.
- **Critical paths:** model T1a → T3 → T9/T10 → T15 → T16 → T23 → T26 → T30 → T33; UI T1b → T8 → T14 → T24 → T27 → T32 → T33; DDL T1a → T7 → T19 → T25 → T32; fixtures T1c → C-2 → T17 → T23.
- **Early signals:** Task 11's `load_160k_column_namespace_under_budget` (W2, synthetic data, core only) is the first measurement of the 300 ms exit (C-3); C-1's CI artifact answers snapshot stability before any snapshot task starts.
- **Docker:** up from the end of W1 (T5) through W6 (C-4). The MySQL/MariaDB services are recreated once (C-2) and `pg17wrap` is started once (C-6). CI's `minicat-verify` uses its own stack.
- **CI proofs:** Task 1a merges after `build-test` ×3 (zstd's C build on Windows first) and a `fuzz/` resolution check; Tasks 1b and 1c after their reviews and `build-test` ×3.
- **Same-wave shared files** (none unnamed):
  - W0: T1a (`hacivat-meta/**`, `sql-catalog/**`, `hacivat-ddl/**`, `sql-ident` stubs, `check_deps.rs`, `xtask/src/{main,minicat}.rs`, `fuzz/**`, ADR 0008, `## m4-t01a-model` FTL) vs T1b (`hacivat-ui`, `hacivat-app`, keymaps, `hacivat-ui.ftl`/`hui.ftl`) vs T1c (session, settings, secrets, `sql-safety`, exec audit slot, `hacivat-introspect/src/**` stubs, `docker/m2a/**`, `xtask/src/docker.rs`, `## m4-t01c-core` FTL) disjoint.
  - W1: T5 (`hacivat-session/src/meta.rs`, named lines in `manager.rs`) vs T6 (`crates/sql/sql-safety/**`) vs T3 (`sql-ident/src/key.rs`) vs T7 (`sql-ident/src/quote.rs`) disjoint; T3 and T7 both touch `sql-ident` only in their own files (module lines are T1a's).
  - W2: T9 (`snapshot.rs`, `view.rs`, `resolve.rs`, `search_path.rs`, `search.rs`) vs T10 (`merge.rs`, `delta.rs`) vs T11 (`codec.rs`, `fingerprint.rs`, `persist/**`, `hacivat-secrets` named lines) vs T12 (`scope.rs`, `filter.rs`, `glob.rs`) vs T13 (`assemble/**`) disjoint.
  - W3: T16/T17/T18 own `pg/**`, `mysql/**`, `sqlite/**` and their pack files, except `pg/facets.rs`, `mysql/facets.rs` and `20-facets.hvq.sql` (W4); T15 owns `engine/**` except `engine/facet.rs` (W5), plus `hub.rs`, `ticket.rs`, `testing/fake.rs`; T17's fenced block in `sql-ident/src/key.rs` is a named exception in T3's (merged) file.
  - W4: T16b/T17b (`{pg,mysql}/facets.rs`, `packs/{pg,mysql}/20-facets.hvq.sql`) vs T21 (`ddl_effects.rs`, `refresh_coord.rs`, `hacivat-exec/src/audit.rs` named block) vs T22 (`freshness.rs`, fenced `// ## m4-t22-state` blocks in `pg/state.rs`, `mysql/state.rs`, `sqlite/change.rs`, named lines in `engine/scheduler.rs`) vs T23 (`minicat.rs`, `tests/minicat_gen.rs`, `hacivat-meta/src/system/{decode,normalize}.rs`, `xtask/src/minicat.rs`, `assets/system-catalogs/**`, `ci.yml` block) disjoint; T24 (`explorer/{mod,live,actions,progress,mass_drop}.rs`) vs T25 (`ddl_view/**`) disjoint, both touch `shell/tab_content.rs` only through Task 1b's registered kinds.
  - W5: T15b (`fragment.rs`, `engine/facet.rs`) vs T26 (`hacivat-meta/src/{overlay,userdata,sql_view}.rs`, `system/{mod,embedded}.rs`, `virtual-fks.toml`) vs T28 (`tests/docker_m4_matrix.rs`, its `tests/common` block, a named block in `tests/sqlite_intro.rs`) disjoint.

**File ownership.** "Owns" means creates or modifies. Anything not listed belongs to Tasks 1a/1b/1c (every new crate, every module line, every stub, every manifest). A task editing a file another task owns lists it as a named exception with the lines.
- **T1a:** `Cargo.toml` (workspace members, dependencies, `[profile.dev.package.zstd-sys]`), `Cargo.lock`, `deny.toml` (only if needed), `.gitattributes`; every manifest of Step 1's table; the new crates' `lib.rs` with module lines and empty module files (first commit); `crates/sql/sql-catalog/**`; `crates/sql/sql-ident/src/{key.rs, quote.rs}` (stubs) and `sql-ident/src/lib.rs` (module lines; named exception, M3's crate); `crates/core/hacivat-meta/src/**` (types and stubs: `kind, name, ids, props, namespace, snapshot, view, resolve, search_path, search, merge, delta, codec, fingerprint, persist/{mod,crypt}, overlay, userdata, system/{mod,decode,normalize,embedded}, sql_view, fixture`); `crates/core/hacivat-ddl/**` (IR declared in full, options, renderer and producer stubs, the `apply` feature line); `xtask/src/{minicat.rs (stub), main.rs (dispatch), check_deps.rs (rules)}`; `fuzz/**` (target stubs `catalog_row_decode`, `system_snapshot_decode`, `scope_pattern`, `pack_parse`); `assets/system-catalogs/{README.md, manifest.toml (empty), virtual-fks.toml (empty)}`; `docs/decisions/0008-metadata-and-introspection.md`; FTL block `## m4-t01a-model` in `locales/{en,tr}/hacivat-core.ftl`; `.github/workflows/ci.yml` block `# ## m4-t01a-ci` (the `cargo test --locked -p hacivat-meta --all-features` step in `build-test` and `turkey-test`; the `fuzz-meta` job, dispatch/schedule only, built like `fuzz-sql` at `ci.yml:776-833` with its `FUZZ_NIGHTLY` pin, running `catalog_row_decode`, `system_snapshot_decode`, `scope_pattern`, `pack_parse`); `crates/app/hacivat-app/Cargo.toml` `[[example]] catalog_cold` (`required-features = ["bench"]`, beside `b3`/`b1_real` at `Cargo.toml:88-95`) with a fail-closed stub `crates/app/hacivat-app/examples/catalog_cold.rs` (the m3a-t01b-seams precedent; body Task 29).
- **T1b:** `crates/ui/hacivat-ui/src/tree/**` (API + flat stub), `lib.rs` (module line), `icon.rs` (new `IconName`s), `crates/ui/hacivat-ui-core/{assets/icons/lucide/*.svg, src/assets.rs}` (named exception, the P-M3b-37 precedent); `crates/app/hacivat-app/src/explorer/**` (stubs: `mod, adapter, rows, badges, live, actions, progress, mass_drop, refresh_prompt, stale, level_pill, contrib, userdata`), `ddl_view/**` (stubs), `connections/dialog/schemas.rs` (stub), `settings_ui/metadata.rs` (stub), `shell/tab_content.rs` (the `DdlView` tab kind, named line), `lib.rs` (module lines); the keymap rows; FTL blocks `## m4-t01b-ui` in `hacivat-ui.ftl` and `hui.ftl`; tests `crates/app/hacivat-app/tests/m4_routing.rs`, `crates/ui/hacivat-ui/tests/tree_seam.rs`.
- **T1c:** `crates/core/hacivat-session/src/{meta.rs (stub: MetaConn, MetaQuery, SnapshotTxn, join), lib.rs (module line), session/mod.rs (SessionEvent::TxEnded seam if needed, named), profile/{mod,store}.rs (catalog_sensitive)}`; `crates/core/hacivat-secrets/src/store.rs` (`SecretSlot::CatalogKey`, named); `crates/core/hacivat-settings/src/{schema.rs (metadata keys), profile_data.rs (new)}`; `crates/sql/sql-safety/src/{report.rs (DdlEffect types, the `ddl` field), ddl_effects.rs (stub)}`; the `ddl: Vec::new()` line in every other `StmtReport` struct literal at the base (named lines: `sql-safety/src/classify.rs:1349`, `report.rs:212`, `policy.rs:1040`, `backend/pg.rs:685`, `:943`, `backend/interim.rs:2316`, `:2401`, `sql-safety/tests/common/mod.rs:81`; `crates/core/hacivat-exec/src/gate.rs:282` — `request_floor`'s synthetic report in the P2-07 file — and `crates/core/hacivat-exec/tests/common/mod.rs:1621`); `crates/core/hacivat-exec/src/audit.rs` (the `CatalogRefreshSink` slot, named); **Part B** `crates/core/hacivat-introspect/src/**` stubs (trait, `pack/**`, `engine/**` incl. `engine/facet.rs`, `hub.rs`, `ticket.rs`, `fragment.rs`, `scope.rs`, `filter.rs`, `glob.rs`, `assemble/**`, `{pg,mysql,sqlite}/**` incl. `{pg,mysql}/facets.rs`, `ddl_effects.rs`, `refresh_coord.rs`, `freshness.rs`, `minicat.rs`, `testing/{mod,fake,big_catalog}.rs`), `packs/pg/{10-core,20-facets,30-state,40-system}.hvq.sql`, `packs/mysql/{10-core,20-facets,30-state,40-system}.hvq.sql`, `packs/sqlite/{10-core,30-state}.hvq.sql` (header-only), `tests/common/mod.rs`; `docker/m2a/pg-wrap/{Dockerfile,init.sh}`, `docker/m2a/compose.yml` (service `pg17wrap`; `--log-bin-trust-function-creators=1` on the MySQL services; named), `docker/m2a/targets.toml` (row `pg17wrap`, sets `["full"]`, named), `docker/m2a/init/mysql.sql` block `# ## m4-t01c-fixtures`, `xtask/src/docker.rs` (named lines 13–43: `DEFAULT_TEST_CRATES` at `:28-36` gains `hacivat-introspect`, the m3a-t01b-seams precedent, so `docker-pr`/`docker-full` and `run-docker-tests.sh` without `-p` run its Docker tests; `DOCKERFILES` at `:40` gains `docker/m2a/pg-wrap/Dockerfile`; `LOCAL_IMAGE_PREFIX` `"hacivat-m2a-"` at `:42` already covers the `hacivat-m2a-pg17wrap` tag); the `hv_fixture` login helper (named lines in `crates/core/hacivat-testkit/src/lib.rs`, beside `USER`/`PASSWORD` at `:21-22`; the crate has no `targets` module); FTL block `## m4-t01c-core` in `locales/{en,tr}/hacivat-core.ftl`.
- **T2:** `design/m4-explorer/{index.html,NOTES.md}`.
- **T3:** `crates/core/hacivat-meta/src/{kind,name,ids,props,namespace,fixture}.rs` (bodies); `crates/sql/sql-ident/src/key.rs` (body, named exception); tests `crates/core/hacivat-meta/tests/{model,kinds,model_perf}.rs`, `crates/sql/sql-ident/tests/key.rs`.
- **T4:** `crates/core/hacivat-introspect/src/pack/**`; tests `pack.rs`; `fuzz/fuzz_targets/pack_parse.rs` (body).
- **T5:** `crates/core/hacivat-session/src/meta.rs` (body), named lines in `manager.rs` (lease through the budget with `Priority::Introspection`, `budget.rs:22`) and `session/router.rs` if the single-connection share needs it; tests `crates/core/hacivat-session/tests/{meta_conn,docker_meta_conn}.rs`.
- **T6:** `crates/sql/sql-safety/src/ddl_effects.rs`, named lines in `backend/{pg,interim}.rs`; `crates/sql/sql-testkit/corpus/ddl_effects/*.toml` (new files); tests `crates/sql/sql-safety/tests/ddl_effects.rs`.
- **T7:** `crates/core/hacivat-ddl/src/{ir,options,produce/{placement,qualify},render/{mod,pg,mysql,sqlite}}.rs`; `crates/sql/sql-ident/src/quote.rs` (body, named exception); tests `render_ir.rs`, `placement.rs`, `qualify.rs`, `options.rs`, `crates/sql/sql-ident/tests/quote.rs`; `tests/golden/ir/**`.
- **T8:** `crates/ui/hacivat-ui/src/tree/**` (bodies: `flat.rs`, `typeahead.rs`, `a11y.rs`, `view/**`; module lines inside `tree/` included); tests `crates/ui/hacivat-ui/tests/{tree_flat,tree,tree_perf}.rs` and `tests/tree_fixture/mod.rs` (fixture-only, not the append-only `tests/support/mod.rs`).
- **T9:** `crates/core/hacivat-meta/src/{snapshot,view,resolve,search_path,search}.rs`; tests `snapshot.rs`, `view.rs`, `snapshot_props.rs`, `snapshot_perf.rs`, `search.rs`.
- **T10:** `crates/core/hacivat-meta/src/{merge,delta}.rs`; tests `merge.rs`, `merge_props.rs`, `merge_perf.rs`.
- **T11:** `crates/core/hacivat-meta/src/{codec,fingerprint}.rs`, `persist/**`; named lines in `crates/core/hacivat-secrets/src/store.rs` (key creation for `CatalogKey`); tests `persist.rs`, `persist_faults.rs`, `persist_crypt.rs`, `persist_perf.rs`, `fingerprint.rs`; `fuzz/fuzz_targets/catalog_row_decode.rs` (body).
- **T12:** `crates/core/hacivat-introspect/src/{scope,filter,glob}.rs`; tests `scope.rs`, `filter.rs`, `scope_props.rs`, `filter_perf.rs`; `fuzz/fuzz_targets/scope_pattern.rs` (body).
- **T13:** `crates/core/hacivat-introspect/src/assemble/**`; tests `assemble.rs`.
- **T14:** `crates/app/hacivat-app/src/explorer/{adapter,rows,badges,contrib}.rs`; named lines in `connections/explorer.rs` (roots become tree rows); tests `explorer_tree.rs`, `explorer_perf.rs`.
- **T15:** `crates/core/hacivat-introspect/src/{engine/** except engine/facet.rs, hub.rs, ticket.rs, testing/fake.rs}`; tests `engine.rs`, `engine_props.rs`, `engine_perf.rs`, `hub.rs`.
- **T15b:** `crates/core/hacivat-introspect/src/{fragment.rs, engine/facet.rs}`; tests `facets.rs`, `docker_fragment_on_session.rs`.
- **T16:** `crates/core/hacivat-introspect/src/pg/**` except `pg/facets.rs`; `packs/pg/{10-core,30-state,40-system}.hvq.sql`; `src/testing/big_catalog.rs` (the 15k-table fixture builder and its batched drop); tests `pg_rows.rs`, `docker_pg_intro.rs`.
- **T16b:** `crates/core/hacivat-introspect/src/pg/facets.rs`, `packs/pg/20-facets.hvq.sql`; tests `docker_pg_facets.rs`.
- **T17:** `crates/core/hacivat-introspect/src/mysql/**` except `mysql/facets.rs`; `packs/mysql/{10-core,30-state,40-system}.hvq.sql`; fenced block `// ## m4-t17-mysql-fold` in `crates/sql/sql-ident/src/key.rs` and its tests in `sql-ident/tests/key.rs` (named exceptions, R-M4-41); tests `mysql_rows.rs`, `docker_mysql_intro.rs`.
- **T17b:** `crates/core/hacivat-introspect/src/mysql/facets.rs`, `packs/mysql/20-facets.hvq.sql`; tests `docker_mysql_facets.rs`.
- **T18:** `crates/core/hacivat-introspect/src/sqlite/**`, `packs/sqlite/{10-core,30-state}.hvq.sql`; tests `sqlite_intro.rs`.
- **T19:** `crates/core/hacivat-ddl/src/{produce/{mod,pg,mysql,sqlite},script}.rs`; tests `produce.rs`, `golden.rs`, `tests/fixtures/mod.rs`; `tests/golden/{pg,mysql,mariadb,sqlite}/**`.
- **T20:** `crates/app/hacivat-app/src/connections/dialog/schemas.rs`, named lines in `connections/dialog/form.rs` (the section and the checkbox), `explorer/level_pill.rs`, `explorer/userdata.rs` (the app's only caller of `hacivat_settings::profile_data`); tests `scope_editor.rs`.
- **T21:** `crates/core/hacivat-introspect/src/{ddl_effects,refresh_coord}.rs`; `crates/core/hacivat-exec/src/audit.rs` (the `CatalogRefreshSink` body, named block); tests `plan_refresh.rs`, `docker_refresh_after_ddl.rs`.
- **T22:** `crates/core/hacivat-introspect/src/freshness.rs`; fenced `// ## m4-t22-state` blocks in `pg/state.rs`, `mysql/state.rs`, `sqlite/change.rs`; named lines in `engine/scheduler.rs` (validation tasks); tests `freshness.rs`, `docker_freshness.rs`.
- **T23:** `crates/core/hacivat-introspect/src/minicat.rs`, `tests/minicat_gen.rs`; `crates/core/hacivat-meta/src/system/{decode,normalize}.rs` (named exception in Task 26's module) and `hacivat-meta/tests/system_decode.rs`; `xtask/src/minicat.rs`; `assets/system-catalogs/**` (generated files, `manifest.toml`); `.github/workflows/ci.yml` block `# ## m4-t23-minicat` (the `minicat-verify` job); `fuzz/fuzz_targets/system_snapshot_decode.rs` (body).
- **T24:** `crates/app/hacivat-app/src/explorer/{mod,live,actions,progress,mass_drop}.rs` (bodies), named lines in `connections/{live,boot}.rs`, `console/commands.rs` (`open_with_text`) and the workspace writer (`explorer.expanded`); tests `explorer_live.rs`, `explorer_actions.rs`.
- **T25:** `crates/app/hacivat-app/src/ddl_view/**`; tests `ddl_view.rs`.
- **T26:** `crates/core/hacivat-meta/src/{overlay,userdata,sql_view}.rs`, `system/{mod,embedded}.rs`; `assets/system-catalogs/virtual-fks.toml` (rows); tests `overlay.rs`, `system_catalogs.rs`, `sql_view.rs`.
- **T27:** `crates/app/hacivat-app/src/explorer/{refresh_prompt,stale}.rs`, `settings_ui/metadata.rs`, named lines in `exec_ui/orchestrator.rs` (install the refresh sink per run); named lines in `settings_ui/mod.rs` (the M3c carry: `fixtures::show_section` at `:834-838` → `SettingsPage::show_section`); tests `refresh_after_ddl_ui.rs`, `metadata_settings.rs`.
- **T28:** `crates/core/hacivat-introspect/tests/docker_m4_matrix.rs` and its block in `tests/common/mod.rs`; the named block `// ## m4-t28-oracle` in `tests/sqlite_intro.rs`.
- **T29:** `crates/app/hacivat-app/src/bench/catalog.rs` (new mode), named line in `bench/mod.rs`, the body of `examples/catalog_cold.rs` (Task 1a's stub); `crates/core/hacivat-introspect/tests/docker_big_catalog.rs`; `docs/benchmarks/m4-bench-notes.md`.
- **T30:** `crates/core/hacivat-meta/tests/offline_resolution.rs`.
- **T31:** `crates/core/hacivat-ddl/tests/golden_stability.rs`; `crates/core/hacivat-introspect/tests/docker_ddl_roundtrip.rs`; `ci.yml` block `# ## m4-t31-golden-guard`.
- **T32:** `crates/app/hacivat-app/tests/shots/m4.rs`, the `Surface::M4` arm in `tests/shots.rs`, `tests/cross_surface_m4.rs`, `tests/offline_explorer.rs`, `DESIGN.md` (section "Explorer Tree" extended: metadata states, DDL view); the two M3c shot carries in `tests/shots/m3c.rs` (named exception, m3c.md §8.1); `design/baselines/m1/macos/superseded.txt` lines if an M4 change reaches a compared M1 state (Global Constraints "Shots harness").
- **T33:** `docs/benchmarks/{m4.md,po-demo-m4.md}`; after PO sign-off, `baseline-m4` stays in this SDD folder, not in git (C-37: M2–M4 baselines are kept out of git until M9).

---

**How to read the test blocks** (as in M3). Each task's Step 1 lists its tests by name, with the assertion written out where the spec, a catalog test case, a spike measurement or a ruling fixes a value. A test shown with an empty body (`{}`) or a comment carries its assertion in its name and in the case it cites; the implementer writes the body with real asserts (F9), and the reviewer rejects a tautological or comment-only body. `[docker: <ids>]` marks a Docker test (`#[ignore = "docker"]`, wrapper only); `(pr)` marks the ids that run in the PR set.

## W0 — Seams and mocks

### Task 1a: Manifests and core types (W0)

**Build:** `CARGO_TARGET_DIR=<main>/target-core/m4-t01a`, base `m4`. **Two commits:** the first holds only `Cargo.toml`, `Cargo.lock`, every manifest of the Step 1 table, `deny.toml` (if needed), `fuzz/Cargo.toml` + `fuzz/Cargo.lock`, the new crates' `lib.rs` with their module lines and empty module files, and the `hacivat-app/examples/catalog_cold.rs` stub its `[[example]]` row needs (so the workspace builds); the controller dispatches **Tasks 1b and 1c in parallel** on it. The second holds the types and stubs below.

**Files:** see File ownership (T1a). Tests `crates/core/hacivat-meta/tests/seams.rs`, `crates/core/hacivat-ddl/tests/seams.rs`, `crates/sql/sql-catalog/tests/seams.rs`, `crates/sql/sql-ident/tests/key_quote_seams.rs`, `xtask` unit tests in `check_deps.rs`; ADR 0008 (new) with rulings R-M4-1..50.

**Spike origin (R-M4-40):** the type declarations of `hacivat-meta` are copied from `m4-spike-model` @ `d86313b` (`kind.rs` with the 67 kinds and their stable `u16` codes, `ids.rs`, `props.rs`, the public items of `namespace.rs`, `snapshot.rs`, `codec.rs`, `persist/`), with every body replaced by its fail-closed stub; the spike's `Cargo.toml` is not used (Step 1's table replaces it). Commit body: `spike origin d86313b (declarations only)`.

**Interfaces** (Produces; every body a fail-closed stub per the Global Constraints list; types follow alg/02 and alg/03 "Data structures" as changed by R-M4-26):
- **`hacivat_meta::kind`:** `#[derive(Copy)] pub enum ObjectKind { Root, Database, Schema, Table, View, MaterializedView, ForeignTable, Column, Index, PrimaryKey, UniqueKey, ForeignKey, Check, Exclusion, Default, Trigger, Rule, Policy, Routine, Procedure, Aggregate, Argument, Sequence, Domain, EnumType, CompositeType, RangeType, AliasType, Extension, Collation, Operator, OperatorClass, OperatorFamily, Cast, Language, AccessMethod, ForeignDataWrapper, ForeignServer, UserMapping, EventTrigger, Role, User, Tablespace, Partition, Subpartition, Event, VirtualTable, VirtualTableModule, /* v1.x engines: */ TableType, Synonym, Login, Dictionary, Projection, RowPolicy, SettingsProfile, Quota, Package, PackageBody, ObjectType, DbLink, Collection, MongoView, TimeSeries, InferredField, RedisDatabase, RedisModule, Unknown }` with `code() -> u16` (stable, never reordered or reused), `id() -> &'static str` (the `model-kind-<id>` Fluent id), `group(dbms) -> Option<KindGroup>` (`None` = parent-scoped: columns, constraints, PG triggers, MySQL indexes), `allows_child(child, dbms)`, `exists_in(dbms)`; `pub enum KindGroup { Relations, Routines, Procedures, Types, Triggers, Other }` (PG composite types in `Relations`); `pub enum DbLevel { Database, SchemaIsDatabase, File }`, `pub fn database_level(DbmsKind) -> DbLevel`. `Other(u16)` of spec §5.7 is `Unknown` here (raw kind text in `Props::Generic`) and is documented as the same seam.
- **`hacivat_meta::name`:** `pub type Name = Arc<str>`; `pub fn has_invisible_chars(&str) -> bool`; re-exports `sql_ident::key::{IdentKey, IdentCtx, NameClass, FoldRule, ServerDependent, match_key}`.
- **`hacivat_meta::ids`:** `pub enum ServerId { Oid(u32), ObjectId(i32), ObjNum(u64), Uuid([u8; 16]), RowId(i64) }`; `pub enum StateMark { None, Xmin(u32), Modified(i64), Created(i64), MyTable { created: i64, trigger_max: i64, trigger_count: u32 }, SchemaVersion(u32), Hash(u64) }` (`MyTable` is R-M4-39's table stamp); `pub enum NsKind { Server, Database, Schema }`; `pub struct NsKey { pub kind: NsKind, pub path: SmallVec<[Name; 2]> }`; `pub struct ObjectKey { pub kind: ObjectKind, pub path: SmallVec<[Name; 4]>, pub server_id: Option<ServerId>, pub overload_sig: Option<Name> }` with **structural** `Eq`/`Hash` and `same_object(&self, &ObjectKey) -> bool` (server id when both sides have one, else kind, path and overload signature); `pub struct Ref { pub kind: ObjectKind, pub path: SmallVec<[Name; 4]>, pub server_id: Option<ServerId> }` (resolved on demand, never an arena index); `pub struct ObjId { pub ns: u32, pub idx: u32 }` (local to one generation); `pub struct SourceRef { pub hash: u128 /* XXH3-128 */, pub len: u32 }`.
- **`hacivat_meta::props`:** `pub enum Props { None, Database(Arc<DatabaseProps>), Schema(Arc<SchemaProps>), Table(Arc<TableProps>), View(Arc<ViewProps>), Column(Arc<ColumnProps>), Index(Arc<IndexProps>), Key(Arc<KeyProps>), ForeignKey(Arc<FkProps>), Check(Arc<CheckProps>), Routine(Arc<RoutineProps>), Sequence(Arc<SeqProps>), Trigger(Arc<TriggerProps>), Type(Arc<TypeProps>), Generic(Arc<GenericProps>) }` with:
  - `ColumnProps { ordinal /* the server's position number: PG attnum with gaps after DROP COLUMN, MySQL ORDINAL_POSITION, SQLite cid + 1 */, raw_type, type_ref: Option<TypeRef { schema, name }>, nullable, not_null_constraint: Option<Name> /* PG 18 named NOT NULL */, default_raw, identity: Option<Identity { Always, ByDefault, AutoIncrement, RowidAlias }>, generated: Option<Generated { kind: Stored | Virtual, expr }>, collation, comment, hidden /* SQLite hidden, MySQL invisible */, enum_labels: Option<Arc<[Name]>> /* MySQL ENUM/SET */ }`;
  - `KeyProps`/`IndexProps { columns: ordered (ordinal, Option<name>), expressions, include, predicate, desc, unique, nulls_not_distinct: bool /* PG ≥ 15 indnullsnotdistinct; false otherwise */, deferrable, initially_deferred, not_valid, enforced: bool /* PG 18 conenforced; true otherwise */, invisible: bool /* MySQL IS_VISIBLE = NO, MariaDB IGNORED */, backing_index, implicit_for: Option<Ref> }`;
  - `TableProps { comment, owner, persistence, partitioned: bool, partition_key: Option<Name>, partition_bound: Option<Name>, row_security: bool /* PG relrowsecurity */, ext: TableExt::{Pg, MySql { engine, collation, options }, Sqlite { without_rowid, strict, module }} }`;
  - `RoutineProps` with the `sql_catalog::FnSig` fields plus `owner`, `definer` (MySQL), `specific_name` (MySQL routine identity is `(ROUTINE_TYPE, SPECIFIC_NAME)`, F9);
  - `TypeProps::{Enum { labels /* sort order */ }, Domain { base, not_null, not_null_constraint: Option<Name> /* PG 17+ */, default_raw, checks }, Composite { attrs }, Range { subtype }, Base}`; `owner` on views, sequences and types;
  - facets: `pub enum FacetKind { Sources, Grants, Dependencies, Statistics }`, `pub struct Facet<T> { pub state: LoadState, pub value: Option<Arc<T>> }`, `GrantsFacet { entries: Vec<Grant { grantee, privilege, columns, grantable }> }`, `DepsFacet { depends_on: Vec<Ref>, kind }`, `StatsFacet { row_estimate: Option<u64>, at: SystemTime }`.
- **`hacivat_meta::namespace`:** `pub struct Obj` (fields private; accessors `kind`, `name`, `parent`, `server_id`, `state`, `flags`, `props`, `child_count`); `bitflags! pub struct ObjFlags: u32 { SYSTEM, TEMP, SERVER_GENERATED_NAME, INVALID, INFERRED, LOSSY_NAME, BROKEN, PRE_INTROSPECTED, INHERITED, INVISIBLE_CHARS, IMPLICIT }`; `pub enum Level { Names, Structure, Sources }`; `pub enum Watermark { None, PgXid { w: u64, identity: PgIdentity }, MyTime { at_utc_micros: i64, server_uuid: Name, version: Name, priv_hash: u64 }, Sqlite { schema_version: u32, file_identity: Option<(u64, u64)>, digest: u128 } }`; `NsMeta`; `pub struct Subtree` (`add`, `graft_under`); `pub struct NamespaceSnapshot` with `build`, `children`, `children_of_kind`, `lookup(group, &str)`, `lookup_kind`, `child_by_name`, `object_key`, `find(&ObjectKey)`, `iter_named(kinds)`, `replace_subtree(idx, Option<Subtree>)`, `upsert_child(parent, Subtree)`; `ModelError`.
- **`hacivat_meta::{snapshot, view, resolve, search_path}`:** `pub struct Catalog` (`new(server)`, `current() -> Arc<CatalogSnapshot>`, `publisher() -> Option<Publisher>`, `subscribe()` behind feature `catalog`); `pub struct Publisher` (single writer, `!Clone`, `publish(Vec<CatalogChange>) -> Result<Published { generation, changed }, ModelError>`, all-or-nothing); `pub enum CatalogChange { Put(Arc<NamespaceSnapshot>), Remove(NsKey), UpsertSubtree { ns, parent: ObjectKey, subtree }, RemoveSubtree { ns, target }, SetState(ContainerKey, LoadState) }`; `pub struct CatalogSnapshot { pub generation: u64, pub server: Arc<ServerFacts>, pub namespaces: Arc<BTreeMap<NsKey, Arc<NamespaceSnapshot>>>, pub states: Arc<BTreeMap<ContainerKey, LoadState>>, pub overlays: Arc<Overlays> }` with `find`, `resolve(&Ref)`, `obj`, `fk_referencing(&ObjectKey) -> Lookup<Vec<ObjHandle>>`; `pub enum LoadState { NotLoaded, Loading { generation: u64, level: Level }, Loaded { level: Level, at: SystemTime, watermark: Watermark, stale: bool }, Partial { reason: IntroErrorCode }, Failed { at: SystemTime, error: IntroErrorCode, retry_after: Duration }, OutOfScope }`; `pub struct ServerFacts { dbms, flavor, version: Name, version_num: u32, ident: IdentCtx, default_collation: BTreeMap<Name /* database */, Name>, standard_conforming_strings: Option<bool>, no_backslash_escapes: Option<bool>, ansi_quotes: Option<bool>, in_recovery: Option<bool> }`; `pub trait ModelView: Send + Sync { type Def; fn lookup(&self, path: &[NamePart], group: KindGroup) -> Lookup<Self::Def>; fn children(&self, parent: &ObjectKey, kind: ObjectKind) -> Lookup<Vec<Self::Def>>; fn find_by_server_id(&self, database: &Name, id: ServerId) -> Lookup<Self::Def>; fn freshness(&self, ns: &NsKey) -> Freshness; }` with `ObjHandle { ns: Arc<NamespaceSnapshot>, id: ObjId }` as `SnapshotView`'s `Def`; `Lookup`, `MissingReason`, `UnknownReason` re-exported from `sql-catalog` (one definition); `pub enum CatalogEvent { Published { generation: u64, delta: Arc<CatalogDelta> }, NamespaceReplaced { generation: u64, ns: NsKey }, PendingConfirmation { ns: NsKey, count: u32 } }`; `SearchPath::{for_pg, for_mysql, for_sqlite}`.
- **`hacivat_meta::{merge, delta}`:** `pub struct NamespaceResult { pub existent: BTreeMap<ObjectKind, ExistSet>, pub changed: Vec<ChangedObject>, pub categories_to_reload: BTreeSet<Category>, pub new_watermark: Watermark, pub full: bool }` with `ExistSet::{Ok(Vec<Identity>), Failed(IntroErrorCode)}`; `pub fn merge(prev: Option<&NamespaceSnapshot>, r: NamespaceResult, policy: &MergePolicy) -> Result<MergeOutcome, MergeError>`; `pub enum MergeOutcome { Applied { next, delta }, PendingConfirmation { next, delta, vanished: u32 } }`; `pub struct MergePolicy { pub trigger: Trigger /* User | Background */, pub guard_ratio_pct: u8 /* 50 */, pub guard_min_objects: u32 /* 20 */ }`; `pub struct CatalogDelta { created, altered: Vec<(ObjectKey, ChangedFacets)>, dropped, renamed: Vec<(ObjectKey, ObjectKey, Confidence)>, moved: Vec<(NsKey, NsKey, ObjectKey)> }` (serde).
- **`hacivat_meta::{codec, persist, fingerprint, search}`:** `encode_namespace`, `decode_namespace`, `payload_version`, `pub const FORMAT_MAJOR: u16 = 1; pub const FORMAT_MINOR: u16 = 0;`; `pub struct CacheStore` with `open(path, Encryption, writer_id) -> Result<Opened { store, rebuilt: Option<RebuildReason> }, CacheError>`, `check_identity`, `load_namespace`, `load_all`, `save_namespace`, `remove_namespace`, `put_source`, `source`, `delete_profile_files`, `cache_file_name(&ProfileId)`; `pub enum Encryption { None, Row(RowKey) }`; `pub struct RowKey([u8; 32])` (Debug redacted, no `PartialEq`); `pub enum CacheError { Disabled, Io(..), Corrupt { renamed_to: PathBuf }, Locked, Incompatible, TempNamespace, NewerMinor }`; `ServerFingerprint`, `IdentityCheck { Same, Changed }`; `persist::crypt::{RowCipher::new(RowKey, domain: &'static str), seal(&self, ad, plaintext) -> Vec<u8>, open(&self, ad, sealed) -> Result<Vec<u8>, CryptError>}`; `fingerprint::{Fingerprint(u128), subtree(&NamespaceSnapshot, ObjId) -> Fingerprint}`; `search::{SearchQuery { text, kinds, limit, mode: Substring | Prefix }, ObjHit { ns, id, kind, name }, search(&CatalogSnapshot, &SearchQuery) -> Vec<ObjHit>}`.
- **`hacivat_meta::{overlay, system, userdata, sql_view}`:** `pub struct Overlays { pub system: Option<Arc<SystemSnapshot>>, pub virtual_fks: Vec<VirtualFk>, pub annotations: BTreeMap<ObjectKey, Name> }`; `pub struct SystemCatalogs` with `open(dir: &Path) -> Result<Self, SystemError>`, `from_embedded() -> Self` (feature `embedded-system`), `empty()`, `for_server(dbms: DbmsKind, version: &str) -> Option<Arc<SystemSnapshot>>`; `system::decode(&[u8]) -> Result<SystemSnapshot, SystemError>`, `system::normalize`; `pub struct ProfileUserData { pub scope: Option<String>, pub view_filters: BTreeMap<ObjectKey, String>, pub scope_filters: BTreeMap<ObjectKey, String>, pub level_overrides: BTreeMap<ObjectKey, Level>, pub virtual_fks: Vec<VirtualFk>, pub annotations: BTreeMap<ObjectKey, String>, pub ddl_view: DdlViewOptions }` (serde, versioned; **types and merge logic only — the file I/O is `hacivat_settings::profile_data`**); `impl sql_catalog::CatalogView for SnapshotView` (stub).
- **`sql_catalog`** (new crate; the trait and types of `plan-m5.md` Task 1a's block, verbatim in names): `pub trait CatalogView { fn generation(&self) -> u64; fn ident_ctx(&self) -> &IdentCtx; fn schemas(..); fn relation(..) -> Lookup<RelInfo>; fn relations(..); fn columns(..) -> Lookup<Arc<[ColumnInfo]>>; fn functions(..); fn functions_in(..); fn types(..); fn foreign_keys(..); fn virtual_fks(..); fn databases(..); fn settings(..); fn row_estimate(..) -> Option<u64>; }`; `pub enum Lookup<T> { Found(T), Missing(MissingReason), Unknown(UnknownReason) }`; `UnknownReason { NotLoaded, Loading, OutOfScope, Failed, ServerDependent }`; payloads `SchemaInfo`, `RelRef`, `RelInfo { name, kind: RelKind { Table, View, MaterializedView, Foreign, Partitioned }, temp, comment }`, `ColumnInfo { name, ordinal, type_name, nullable, default_raw, identity, generated, collation, comment, in_pk, in_fk }`, **`FnSig { schema, name, args: Option<Vec<FnArg { name, type_name, mode: In | Out | InOut | Variadic | Table, has_default }>> /* None = name only (MySQL/MariaDB built-ins) */, returns, returns_set, kind: Function | Aggregate | Window | Procedure, volatility, language }`**, `TypeInfo { name, kind: Enum { labels } | Domain { base } | Composite | Range { subtype } | Base }`, `FkEdge { from: RelRef, from_cols, to: RelRef, to_cols, name, is_virtual }` (columns in FK order), `SettingInfo { name, type_name, context }`; `EmptyCatalog` (fail-closed).
- **`sql_ident` stubs** (named exception in M3's crate): `key::{IdentKey, IdentCtx { dialect, lower_case_table_names: u8, encoding: Option<Name> }, NameClass { Database, Schema, Table, Column, Routine, Other }, FoldRule { Exact, AsciiCaseless }, ServerDependent, match_key(d: DialectId, ctx: &IdentCtx, text: &str, quoted: bool, class: NameClass) -> Result<IdentKey, ServerDependent>}` (stub: always `Err(ServerDependent)`, so every lookup is `Unknown`); `quote::{QuoteCtx { ansi_quotes, no_backslash_escapes, standard_conforming_strings }, needs_quoting, quote, quote_qualified, string_literal(d, text, &QuoteCtx) -> Result<String, QuoteError { Nul, NotImplemented }>}` (stub: `needs_quoting` is always true and `quote` always quotes with the dialect's quote character doubled inside, which over-quotes but is never wrong; `string_literal` answers `Err(NotImplemented)`).
- **`hacivat_ddl`:** `pub enum DdlOp` with every alg/03 variant: `CreateSchema(SchemaDef)`, `CreateTable(TableDef)`, `AlterTable { table: QualifiedName, actions: Vec<AlterAction> }`, `DropObject { kind: ObjectKind, name, sig, if_exists, cascade }`, `CreateIndex`, `CreateView`, `CreateRoutine`, `CreateTrigger`, `CreateSequence`, `CreateType`, `Rename { kind, from, to }`, `SetSchema { kind, name, to }`, `Comment { target, text }`, `Grant(GrantDef)`, `Revoke(GrantDef)`, `Owner { kind, name, owner }`, `RowSecurity { table, enable: bool }`, `TableRebuild(RebuildPlan)`, `Exchange { a, b }` (ClickHouse; never rendered in v1), `Raw { sql, origin: RawOrigin { ServerText, UserEdited } }`, `Data(DataStep)`; `#[non_exhaustive] pub enum AlterAction` with the alg/03 HV-DDL-05 actions (add/drop/rename/alter column type, nullability, default, comment; add/drop constraint; validate constraint); `pub trait Renderer { fn render(&self, op: &DdlOp, ctx: &RenderCtx) -> Result<Vec<Stmt>, RenderError>; }`; `pub struct Stmt { pub text: String, pub txn: TxnClass, pub delimiter: Delim }`; `pub struct GenOptions { pub qualify: Qualify, pub placement: Placement, pub or_replace: bool, pub if_not_exists: bool, pub skip_grants: bool, pub skip_owner: bool, pub skip_definer: bool, pub surrogate_names: bool, pub server_definitions: bool }` (defaults R-M4-23, alg/03 HV-DDL-11); `pub struct DdlTarget { pub dialect: DialectId, pub flavor: Flavor, pub version: ServerVersion, pub current_schema: Option<Name>, pub search_path: Vec<Name>, pub quote: QuoteCtx }`; `pub fn create_script(view: &dyn ModelView<Def = ObjHandle>, objects: &[ObjectKey], target: &DdlTarget, opts: &GenOptions) -> Result<Script, DdlError>`; `Script { stmts, notices }`; the empty non-default feature `apply`.
- **`xtask`:** `minicat` dispatch with the spike's subcommands (`collect [--reader catalog|test] [--twice] [--require-complete]`, `build --raw D`, `check`/`--check`, `verify`/`--verify`): stubs exit 2 with "not implemented" except `--check`, which passes on the empty manifest; `check_deps.rs` rules `DdlIsPure`, `DdlApplyOnlyFromApp`, `MetaBelowIntrospect`, `EmbeddedSystemOnlyFromApp`, `IntrospectNotExec`, the source rule `MetaQueryOnlyFromPacks`, `sql-catalog` under the SQL rules; plus the M3c carry (m3c.md §8.1, "next manifest window (M4 W0)") in its grep form: the source rule `HoldCountsTestOnly` — `hold_counts(` (`hacivat-ui-editor/src/find.rs:301`) is named by no `src/` file outside its own definition (its one caller is `hacivat-app/tests/shots/m3c.rs:2032`); `PRODUCT_CRATES` += `hacivat-meta`, `hacivat-introspect`, `hacivat-ddl`, `sql-catalog`.

- [ ] **Step 1: Manifests first (own commit; the base of Tasks 1b and 1c).** Write exactly these edges (`=` = exact pin; "ws" = the workspace version already in `Cargo.lock`); nothing else changes:

  | Crate | Dependency | Features / kind | Why | Used by |
  |---|---|---|---|---|
  | workspace | `zstd = "=0.13.3"` | `default-features = false` | shipped snapshot compression (R-M4-17) | T23, T26 |
  | workspace | `[profile.dev.package.zstd-sys] opt-level = 2` | — | `minicat --check` not 15 s at -O0 (minicat spike §6) | T23 |
  | workspace | `[workspace.dependencies]` path entries `hacivat-meta`, `hacivat-introspect`, `hacivat-ddl`, `sql-catalog` (membership comes from the existing globs `crates/core/*`, `crates/sql/*`, `Cargo.toml:3`; no `members` edit) | — | new crates | all |
  | `sql-catalog` | `sql-ident`, `smallvec` (ws) | — | `IdentCtx`, small paths | T1a, T26 |
  | `sql-ident` | `icu_casemap` (ws) | — | held for Task 17's adoption if the probe shows the server folds beyond ASCII (R-M4-41); unused otherwise and removed at the gate (one controller-listed manifest edit) | T17 |
  | `hacivat-meta` | `hacivat-proto`, `sql-ident`, `sql-catalog`, `twox-hash`, `bitflags`, `arc-swap`, `smallvec` (ws) | always | model, keys, XXH3, flags, snapshots | T3, T9–T11 |
  | `hacivat-meta` | `tokio` (ws) | optional, `default-features = false`, `sync`; feature `catalog` | broadcast only | T9 |
  | `hacivat-meta` | `rusqlite`, `chacha20poly1305` (ws) | optional; feature `persist` | cache store, row AEAD | T11 |
  | `hacivat-meta` | `zstd`, `serde`, `serde_json` (ws) | optional; feature `system` (`serde` also for `CatalogDelta`, `ProfileUserData`: always) | snapshot decode of canonical JSON (R-M4-33) | T23, T26 |
  | `hacivat-meta` | — | features `embedded-system = ["system"]`, `test-support`; `default = ["catalog", "persist", "system"]` | snapshots in the app binary only (Q1-B); `fixture` | T26, T3 |
  | `hacivat-meta` | `proptest`, `tempfile`, `hacivat-meta` (self, `test-support`) | dev | property tests, temp dirs | T3–T11 |
  | `hacivat-introspect` | `hacivat-meta`, `hacivat-session`, `hacivat-proto`, `hacivat-values`, `hacivat-secrets`, `hacivat-settings`, `sql-ident`, `sql-safety`, `sql-dialect` | — | engine, hub (`CatalogKey`, settings), `DdlEffect`, binds | T15–T23 |
  | `hacivat-introspect` | `tokio` (rt, sync, time), `tokio-util`, `futures`, `twox-hash`, `smallvec`, `regex`, `serde`, `bitflags` (ws) | — | runtime, cancellation, SQLite digest, filters | T12, T15–T22 |
  | `hacivat-introspect` | `hacivat-testkit`, `hacivat-ddl`, `proptest`, `serde_json`, `tempfile`, `hacivat-meta` (`test-support`) | dev | Docker targets, T31 round trip, minicat raw files | T15–T31 |
  | `hacivat-ddl` | `hacivat-meta` (`default-features = false`), `hacivat-proto`, `sql-ident`, `sql-dialect`, `sql-catalog`, `smallvec` (ws) | — | pure core (`DdlIsPure`) | T7, T19 |
  | `hacivat-ddl` | — | feature `apply = []` (empty; M7 fills) | R-M4-35 | M7 |
  | `hacivat-ddl` | `proptest`, `hacivat-meta` (`test-support`) | dev | fixtures | T7, T19, T31 |
  | `hacivat-session` | — | no change (meta module uses existing deps) | | T5 |
  | `hacivat-exec` | `hacivat-introspect` | — | refresh sink (T21) | T21 |
  | `hacivat-app` | `hacivat-meta` (features `embedded-system`), `hacivat-introspect`, `hacivat-ddl`, `sql-catalog`, `twox-hash` (ws) | — | explorer, DDL view, `TreeKey` | T14, T20, T24–T32 |
  | `hacivat-app` | `[[example]] name = "catalog_cold"`, `required-features = ["bench"]` | example | the G1 bench is an app example like `b3`/`b1_real` (`Cargo.toml:88-95`); the `hacivat` binary has no bench flag | T29 |
  | `xtask` | `zstd`, `serde`, `serde_json`, `twox-hash` (ws) | — | minicat build/check/verify (spike) | T23 |
  | `fuzz` | `hacivat-meta`, `hacivat-introspect`; targets `catalog_row_decode`, `system_snapshot_decode`, `scope_pattern`, `pack_parse` | — | fuzz stubs | T4, T11, T12, T23 |

  If a listed crate is not in the workspace under that name at the `m4` base (for example `hacivat-values`), report NEEDS_CONTEXT with the actual name; never add an unlisted crate. Run `cargo deny check licenses bans` (record the zstd lines) and `cargo xtask check-deps`. Commit `build(m4): manifests for hacivat-meta, hacivat-introspect, hacivat-ddl and sql-catalog`.
- [ ] **Step 2: Write the failing tests.**
```rust
// hacivat-meta/tests/seams.rs
#[test] fn stub_lookup_is_unknown_never_found_or_missing() { let v = empty_view(); assert!(matches!(v.lookup(&path("public.t"), KindGroup::Relations), Lookup::Unknown(_))); }
#[test] fn stub_fk_referencing_is_unknown() {}
#[test] fn stub_merge_publishes_nothing() { assert!(merge(None, NamespaceResult::empty_full(), &MergePolicy::default()).is_err()); }
#[test] fn stub_cache_is_memory_only() { assert!(matches!(CacheStore::open(&tmp().join("c.v1.sqlite"), Encryption::None, writer()), Err(CacheError::Disabled))); }
#[test] fn stub_system_catalogs_serve_nothing() { assert!(SystemCatalogs::empty().for_server(DbmsKind::Postgres, "17.11").is_none()); }
#[test] fn row_key_debug_is_redacted() { assert!(!format!("{:?}", RowKey::from_bytes([7; 32])).contains('7')); }
#[test] fn kind_codes_are_unique_and_match_the_spike_table() {}   // the 67 codes of d86313b, listed in the test
// sql-catalog/tests/seams.rs
#[test] fn stub_catalog_is_unknown() {}            // M5's test, moved
#[test] fn lookup_reasons_are_codes_not_text() {}
#[test] fn fn_sig_args_none_means_name_only() {}
// sql-ident/tests/key_quote_seams.rs
#[test] fn stub_match_key_is_server_dependent() {}
#[test] fn stub_quote_always_quotes_and_doubles_the_quote_char() {}
// hacivat-ddl/tests/seams.rs
#[test] fn stub_renderers_refuse() { for d in [DialectId::Postgres, DialectId::MySql, DialectId::MariaDb, DialectId::Sqlite] { assert!(matches!(renderer(d).render(&DdlOp::sample_create_table(), &ctx(d)), Err(RenderError::Unsupported { .. }))); } }
#[test] fn every_alg03_variant_is_declared() {}
// xtask check_deps unit tests
#[test] fn ddl_reaching_tokio_is_refused() {} #[test] fn ddl_apply_feature_outside_app_is_refused() {} #[test] fn meta_reaching_session_is_refused() {}
#[test] fn meta_dev_dependency_on_introspect_is_refused() {} #[test] fn embedded_system_outside_app_is_refused() {} #[test] fn introspect_reaching_exec_is_refused() {}
#[test] fn sql_catalog_reaching_core_is_refused() {} #[test] fn meta_query_new_outside_packs_is_refused() {} #[test] fn hold_counts_called_from_src_is_refused() {}
```
- [ ] **Step 3:** Run `cargo test -p hacivat-meta -p hacivat-ddl -p sql-catalog --test seams`, `cargo test -p sql-ident --test key_quote_seams`, `cargo test -p xtask check_deps`. Expected: compile errors, then FAIL.
- [ ] **Step 4: Implement** the types and stubs; `xtask` dispatch and rules; `.gitattributes` (`assets/system-catalogs/*.hvsnap.zst binary`, `crates/core/hacivat-ddl/tests/golden/** -text`, `crates/core/hacivat-introspect/packs/**/*.hvq.sql -text`); ADR 0008 with R-M4-1..50; the FTL block `## m4-t01a-model` with every `model-*` and `ddl-*` message the catalogs' i18n notes name (alg/02 HV-MODEL-01..03; alg/03 HV-DDL-07, -09..11; plus `ddl-grants-not-loaded`, `ddl-grants-partial`, `ddl-policies-not-loaded`), English final, Turkish drafts marked `# Review: PO` (transaction/commit/rollback stay English), every count and date argument preformatted (P31), every kind label a whole-phrase plural message (`model-kind-table = { $count -> [one] table *[other] tables }`, `model-kind-schema-mysql = database`).
- [ ] **Step 5:** Run the seams tests, `cargo test --workspace --no-run`, `cargo test -p hacivat-meta --no-default-features --no-run`, `cargo test -p hacivat-meta --all-features --no-run`, `cargo xtask check-deps`, `cargo xtask l10n check`, clippy (plain and `--all-features`). Expected: PASS.
- [ ] **Step 6: Commit** `feat(m4): core and SQL types for metadata, catalog view and DDL (fail-closed stubs)`.

**Review:** security-sensitive (opus). Probes: a stub that answers `Found`/`Missing`/`Current`; a manifest edge that gives `hacivat-ddl` an async runtime or `hacivat-meta` a session (also as a dev-dependency); a dependency not in the Step 1 table; a `RowKey` that derives `PartialEq` or prints its bytes; a spike body that survived the copy un-stubbed; an `ObjectKind` code that differs from the spike's table.

### Task 1b: UI and app seams (W0, gpui)

**Build:** `CARGO_TARGET_DIR=<main>/target-core/m4-t01b`, base: Task 1a's first commit. Signatures that name `hacivat_meta` types (`ObjectKey` in the open-data handler, `reveal`) are written after `git rebase m4` once Task 1a has merged (the controller says when).

**Files:** see File ownership (T1b).

**Interfaces:**
- **`hacivat_ui::tree`** (the API of `m4-spike-tree` @ `2a31b8b`, declarations copied, bodies stubbed, R-M4-40): `pub struct TreeKey(pub u128)` (opaque; the app derives it from `ObjectKey`, Task 14); `pub trait TreeModel: 'static { fn roots(&self) -> Arc<[TreeKey]>; fn row(&self, key: TreeKey) -> Option<TreeRowData>; fn children(&self, key: TreeKey) -> ChildState; fn generation(&self) -> u64; fn search_text(&self, key: TreeKey) -> Option<SharedString> { /* the label */ } }` (`row` answers from the snapshot without I/O; it runs per visible row per frame); `pub enum ChildState { Leaf, NotLoaded, Loading { label: SharedString }, Loaded(Arc<[TreeKey]>), Failed { label: SharedString } }`; `pub struct TreeRowData { pub label: SharedString, pub label_source: TextSource, pub icon: Option<IconName> /* none on loading rows and group headers */, pub meta: Option<SharedString>, pub flags: Vec<RowFlag> /* smallvec is not a hacivat-ui dependency; an empty Vec does not allocate */, pub badge: Option<SharedString>, pub status: Option<RowStatus>, pub group_header: bool, pub dim: bool }` (labels are `SharedString` plus their `TextSource`, not `RenderedText`, which is the text registry's record of a drawn string); `pub enum RowFlag { Env(EnvKind), Lock { production: bool }, Icon { icon: IconName, text: SharedString } }`; **the status slot is its own field** `pub struct RowStatus { pub kind: StatusKind, pub glyph: SharedString, pub text: SharedString }` with `StatusKind { Modified, New, Conflicted, Unknown, ContainsChanges }` (M4b's git badges; glyph and text alternative both come from Fluent through the app, the text alternative is required by type; never red); `pub struct TreeView<M: TreeModel>` with `new(model, cx)`, `model`, `update_model`, `set_model`, `model_changed` (the generation contract), `set_multi_select`, `set_a11y_label`, `set_typeahead_fold(Fold)`, `expanded`, `is_expanded`, `set_expanded(key, bool)`, `set_expanded_keys(keys)` (one re-flatten for a restored session or expand-all), `selection`, `cursor`, `select(key)`, `scroll_to(key)`, `scroll_by`; `pub enum TreeEvent { Expand(TreeKey), Collapse(TreeKey), Activate(TreeKey), ContextMenu(TreeKey, Point<Pixels>), SelectionChanged(Option<TreeKey>) }`; `pub enum Fold { Tolerant, Locale { turkic: bool } }`. Stub: a flat list of roots, no expansion.
- `hacivat_ui::icon::IconName` gains Lucide icons for the kinds (names chosen by Task 1b from the Lucide set under ISC, e.g. `Table`, `Eye`/view, `Columns3`, `KeyRound`, `Link`, `ListTree`/index, `SquareFunction`, `Zap`/trigger, `ListOrdered`/sequence, `FolderTree`/schema, `Database`, `ShieldCheck`/check, `Type`); `ALL` updated (the count test at `icon.rs:323`, 73 icons at the base).
- App: `explorer::{ExplorerModel (TreeModel impl, stub), ExplorerContributor trait { fn database_children(&self, profile: ProfileId, database: &str, cx: &App) -> Vec<ContributedNode>; }, register_contributor}` (R-M4-19); `ddl_view::open_ddl_view(workspace, profile, ObjectKey, cx)` (stub: opens nothing, returns `Err(NotYet)`); `connections::dialog::schemas::SchemasSection` (stub); `settings_ui::metadata` (stub rows hidden).
- **Extension points (R-M4-19):** the tree's `status: Option<RowStatus>` slot above (M4b); `ContributedNode { key: TreeKey, row: TreeRowData, children: ContributedChildren /* Leaf | Lazy(Arc<dyn Fn(&App) -> ChildState>) */, filter_text: Option<String> }`; `ExplorerContributor` gains `children(&self, key, cx) -> ChildState`, `activate(&self, key, window, cx)`, `context_menu(&self, key, cx) -> Vec<MenuItem>`, `generation(&self) -> u64`; `ExplorerActionProvider { fn actions(&self, node: &NodeRef, surface: ActionSurface /* ExplorerMenu | DdlViewToolbar */, cx) -> Vec<ProvidedAction { slot: ActionSlot /* Data | Transfer | Modify | Source */, id, label, enabled }> }` + `register_action_provider`; `explorer::actions::set_open_data_handler(Box<dyn Fn(ProfileId, ObjectKey, &mut Window, &mut App)>)` (stub: unset; Task 24 installs the R-M4-13 default). Contributed keys are domain-separated by their owner (M4b uses `"hv-scripts\0"`).
- **Keymap rows** (Explorer context; **VS Code conventions where VS Code has an equivalent**, remappable like every row; PO preference, **PO-Q10**): `explorer.view_ddl` `f12` (VS Code "Go to Definition"); `explorer.copy_qualified_name` `cmd-alt-c` / `shift-alt-c` (VS Code "Copy Path"); `explorer.collapse_all` `cmd-left` / `ctrl-left` (VS Code `list.collapseAll`); `explorer.focus_filter` `cmd-alt-f` on macOS only (VS Code `list.find`; the existing filter focus binding is kept, and Windows/Linux get no new row: see below); `explorer.open_data` = `enter` on a table or view row (the explorer's activate; containers toggle) and the palette; `explorer.refresh` `f5` in the Explorer context only (the file-manager and browser convention; VS Code has no default here); `explorer.expand_all_children` palette-only; `ddl_view.copy` the standard copy; palette entries for each plus `explorer.edit_scope`, `explorer.set_level.*`, `metadata.clear_cache`. Existing explorer bindings (Enter, ⌘E, ⌘D, ⌘⇧C, ⌫, Shift-F10; `connections/explorer.rs:12-15`; rows at `keymaps/default-macos.toml:64-97`) keep acting on profile roots. **`explorer.focus_filter` already exists** (`cmd-f` / `ctrl-f`, `Workspace` context: `default-macos.toml:41`, `default-linux.toml:39`, `default-windows.toml:46`) and is kept; the VS Code `list.find` chord is added on macOS only (`cmd-alt-f`, `Explorer` context): on Windows and Linux `ctrl-alt-f` is AltGr+F, which types `@` on Turkish-F (`keymaps/altgr/tr-f.toml:28`), so it would fail `every_preset_has_exactly_its_accepted_altgr_collisions` (`tests/keymap_collisions.rs:27`) and no row is added there (pre-flight P-M4-9; PO-Q10 item). A chord that collides with an M3 binding or fails the AltGr CI is **reported with a proposed alternative**, never silently re-bound.
- FTL blocks: every `hv-explorer-*`, `hv-ddl-view-*`, `hv-scope-*`, `hv-metadata-*` id the mocks need (EN final, TR `# Review: PO`).

- [ ] **Step 1: Write the failing tests:** `m4_routing::every_m4_command_has_a_binding_or_is_palette_only`, `m4_routing::stub_count_is_n` (the stub list, M3c `m3b_routing` precedent), `m4_routing::keymap_commands_are_registered_including_host_only`, `m4_routing::m4_defaults_follow_the_vscode_table` (the rows above, per host), `m4_routing::action_provider_stub_contributes_nothing`, `tree_seam::flat_stub_renders_roots_and_emits_activate`, `tree_seam::status_slot_requires_a_text_alternative`.
- [ ] **Step 2:** Run `cargo test -p hacivat-app --test m4_routing`, `cargo test -p hacivat-ui --test tree_seam`. Expected: FAIL.
- [ ] **Step 3: Implement** the stubs, icons, keymap rows, FTL ids, tab kind.
- [ ] **Step 4:** Run the tests, `cargo xtask l10n check`, clippy (plain, `--all-features`), the AltGr collision CI, and render the explorer surfaces: **no shot may change in Task 1b** (the stub keeps today's list).
- [ ] **Step 5: Commit** `feat(m4): tree widget, explorer, DDL view and settings seams; M4 key bindings`.

**Review:** opus. Probes: a chord that shadows an existing binding without a listed reason; a default that is not the VS Code one where VS Code has one; an icon not under ISC; a changed shot; a `TreeKey` derived from a display string (must come from `ObjectKey`).

### Task 1c: Session, settings, introspection and fixture seams (W0)

**Build:** `CARGO_TARGET_DIR=<main>/target-core/m4-t01c`, base: Task 1a's first commit. **Part A** (session, settings, secrets, `sql-safety`, exec slot, Docker and xtask fixture files) starts at once. **Part B** (the `hacivat-introspect` stubs, which name Task 1a's `hacivat_meta` types) starts after Task 1a has merged and the controller has told the task to `git rebase m4` (Part A's commits replay cleanly: the two tasks own disjoint files).

**Files:** see File ownership (T1c). Tests `crates/core/hacivat-session/tests/profile_m4.rs`, `crates/core/hacivat-settings/tests/profile_data.rs`, `crates/core/hacivat-introspect/tests/seams.rs`; ADR 0004 `## m4-t01c — metadata session seams`.

**Interfaces** (Produces; fail-closed stubs):
- **`hacivat_session::meta`:** `pub struct MetaConnSpec { pub profile: ProfileId, pub database: Option<String>, pub statement_timeout: Duration }`; `impl SessionManager { pub fn meta_conn<'a>(&'a self, spec: MetaConnSpec, cancel: &'a CancellationToken) -> BoxFuture<'a, Result<MetaConn, MetaError>> }`; `pub struct MetaConn` (`dbms()`, `server()`, `begin_snapshot()`, `join(&SessionHandle) -> Result<MetaConn, MetaError>`, `debug_xid_assigned()` behind `cfg(any(test, feature = "test-support"))`); **`pub struct MetaQuery<'a> { sql: &'a str, binds: &'a [BindValue], joinable: bool }`** with a `#[doc(hidden)] pub fn new(sql, binds, joinable)` (callable only from the pack renderer; check-deps `MetaQueryOnlyFromPacks`); `pub struct SnapshotTxn<'c>` (`rows(q: MetaQuery<'_>) -> Result<Rows, MetaError>`, `commit()`); `pub struct Rows { pub columns: Vec<Arc<str>>, pub rows: Vec<Vec<Option<Arc<str>>>> }`; `pub enum MetaError { Unavailable, Busy, ConnectionLost, Timeout, NotJoinable, RefusedText, ReadOnlyNotEnforced, Server { sqlstate: Option<Arc<str>>, message: ServerMessage }, Cancelled }`. `SessionEvent::TxEnded { outcome: TxOutcome }` with `TxOutcome::{Committed, RolledBack, RolledBackTo(Arc<str>)}` **only if** `SessionEvent::TxChanged(TxState)` (`session/mod.rs:301`) cannot express savepoint rollbacks (the task checks and records which).
- `hacivat_session::profile::Profile.catalog_sensitive: bool` (TOML key left out at `false`, so M2b/M3 files round-trip byte for byte, the P-M3b-10 rule).
- `hacivat_secrets::SecretSlot::CatalogKey`.
- **`sql_safety::report`:** `pub struct DdlEffect { pub action: DdlAction, pub kind: DdlKind, pub name: Option<QualifiedName>, pub on: Option<QualifiedName>, pub referenced: Vec<QualifiedName>, pub temp: bool, pub concurrently: bool, pub opaque: bool }`, `pub enum DdlAction { Create, Alter, Drop { cascade: bool }, Rename { to: Ident }, SetSchema { to: Ident }, Comment, Grant, Revoke, Other }`, `pub enum DdlKind { Database, Schema, Table, View, MaterializedView, Index, Sequence, Routine, Trigger, Type, Domain, Extension, Policy, Rule, Event, Role, Other }`; field `StmtReport.ddl: Vec<DdlEffect>` (the effect types derive `Debug, Clone, PartialEq, Eq, Hash`, because `StmtReport` does, `report.rs:93`; every other `StmtReport` literal at the base gets `ddl: Vec::new()`, File ownership T1c). Stub: both backends fill `vec![DdlEffect::opaque()]` for every statement in Task 6's scope (the DDL-class `StmtKind`s `CreateTable`, `CreateOther`, `Alter`, `Drop`, `DropOwned` (M3c P2-07), `Truncate`, `Grant`, `Revoke`, plus `DO`, `CALL`, `EXECUTE`, `PREPARE`, MySQL executable comments) and `vec![]` otherwise.
- `hacivat_exec::audit::CatalogRefreshSink` (stub: drops every event; installed nowhere until Task 27).
- **`hacivat_settings`:** keys `metadata.refresh_after_ddl` (`always|ask|never`, default `always`), `metadata.validate_interval_min` (`auto|0|5|10|30`, default `auto` = R-M4-20), `metadata.cache_mb_per_profile` (512), `metadata.cache_mb_total` (4096); **`profile_data`** — the **only** reader and writer of `<config>/profiles/<uuid>.toml` (`paths.rs` `profile_toml(uuid)`): a sectioned document (`ProfileDataDoc::load(uuid)`, `section::<T>(name)`, `save_section(name, &T)`), atomic write via `fsio`, mode 0600, unknown sections and their formatting preserved byte for byte; M4 owns `[metadata]` (typed as `hacivat_meta::ProfileUserData`).
- **`hacivat_introspect`** (Part B): `pub trait Introspector: Send + Sync { fn list_namespaces<'a>(&'a self, c: &'a mut MetaConn) -> BoxFuture<'a, Result<Vec<NsInfo>, IntroError>>; fn introspect<'a>(&'a self, c: &'a mut MetaConn, ns: &'a NsKey, mode: Mode, level: Level, prev: Option<&'a NamespaceSnapshot>, cancel: &'a CancellationToken) -> BoxFuture<'a, Result<NamespaceResult, IntroError>>; fn fragment<'a>(…, keys: &'a [ObjectKey], level: Level, …) -> BoxFuture<'a, Result<NamespaceResult, IntroError>>; fn stamps<'a>(&'a self, c: &'a mut MetaConn, keys: &'a [ObjectKey]) -> BoxFuture<'a, Result<Vec<Option<StateMark>>, IntroError>>; fn facet<'a>(&'a self, c: &'a mut MetaConn, ns: &'a NsKey, keys: &'a [ObjectKey], kind: FacetKind) -> BoxFuture<'a, Result<FacetRows, IntroError>>; fn check_state<'a>(&'a self, c: &'a mut MetaConn, keys: &'a [ObjectKey]) -> BoxFuture<'a, Result<Vec<Staleness>, IntroError>>; fn check_namespace<'a>(…, ns: &'a NsKey, snap: &'a NamespaceSnapshot) -> BoxFuture<'a, Result<NsCheck, IntroError>>; fn capabilities(&self) -> IntroCaps; }` (the `BoxFuture` style of `hacivat-driver/src/driver.rs:188-300`); `pub fn introspector_for(dbms: DbmsKind) -> Option<Arc<dyn Introspector>>`; `pub enum Mode { Full, Incremental, Brief, System }`; `pub enum IntroError { NotImplemented, Meta(MetaError), Permission { kind: ObjectKind }, Timeout, Cancelled, CatalogRace /* PG XX000 cache lookup failed */, Protocol(Name) }` (codes only; Fluent at the edge); `pub enum Staleness { Current, Changed, Dropped, Unknown }`; `pub struct RefreshTicket` (`Future<Output = Result<u64 /* generation that contains the result */, IntroError>>`); `pub struct IntroHandle` with `on_connect`, `expand(ns)`, `elevate(key, level)`, `fragment(ns, keys)`, `refresh(target, trigger)`, `load_facet(keys, FacetKind)`, `check_state(keys)` (each returns a `RefreshTicket`), `set_scope(pattern)`, `set_level_override(container, Option<Level>)`, `progress() -> watch::Receiver<Progress>`, `confirm_pending(ns, accept: bool)`, `shutdown()`; `pub struct Progress { pub tasks: Vec<TaskProgress> }`, `pub enum Phase { ListNamespaces, Tables, Columns, Indexes, Constraints, Triggers, Routines, Types, Sequences, Comments, Sources, Facets, Existence, Validate }`; `hub::{CatalogHub, ProfileCatalog { catalog: Arc<Catalog>, handle: IntroHandle }, HubError}` (`new(sessions, settings, secrets, system)`, `get`, `ensure`, `row_key`, `shutdown`); `fragment::on_session(handle: &SessionHandle, ns: &NsKey, keys: &[ObjectKey], level: Level, cancel) -> BoxFuture<Result<NamespaceSnapshot, IntroError>>`; `scope::{parse_scope, ScopePattern, ScopeError}`; `filter::{parse_filter, CompiledFilter, FilterRole}`; `pack::{Pack, PackCtx, RenderedQuery}` (`RenderedQuery::as_meta_query()` is the only `MetaQuery::new` caller); `ddl_effects::{plan_refresh, RefreshPlan, Target, ExecutedStmt, SessionStateAtExec}`; `refresh_coord::RefreshCoordinator`; `freshness::NsCheck`; `minicat::{run_system_sections}`; `testing::{FakeIntrospector (stub), big_catalog::{ensure_pg, drop_pg} (stubs)}`.
- **Docker fixture files** (R-M4-43; nothing is started or recreated by this task):
  - `docker/m2a/pg-wrap/Dockerfile` `FROM postgres:17@<the digest compose.yml already pins>`; `init.sh` (own wording of ver/01 `#pg-xmin-freeze` `wrap.sh`): `initdb`, start, **create the test role and databases** (the stock entrypoint skips initialisation on an existing `PGDATA`), `CREATE TABLE w(a int)`, `VACUUM FREEZE` in `postgres`, `template1` and the test database, stop, `pg_resetwal -x $((2147483648 + 2000))` with the `pg_xact` segment pre-created, then exec the normal entrypoint so the server listens on 5432 with the test user. Compose service `pg17wrap` (profile `full`, port from the next free slot in `targets.toml`, healthcheck as the other PG services); `targets.toml` row `pg17wrap` (sets `["full"]`); `xtask/src/docker.rs` lists the pinned `pg-wrap/Dockerfile` and the local `hacivat-m2a-pg17wrap` tag (as `pg-tr` is), so `check-pins` passes.
  - `docker/m2a/compose.yml`: `--log-bin-trust-function-creators=1` on `mysql80`, `mysql84` (and `mysql84-x509` if it is the same image line).
  - `docker/m2a/init/mysql.sql` block `# ## m4-t01c-fixtures`: the `hv_catalog` account (`GRANT SELECT` on `mysql.*` and `performance_schema.*`; `SELECT, SHOW VIEW, EXECUTE` on `sys.*`; no fixture database); `GRANT ALL ON \`hvm4\_%\`.* TO 'hv_test'@'%'`; the `hv_fixture` account with the minimal privileges for `SET GLOBAL time_zone` and `CREATE USER`/`GRANT`/`REVOKE` on `hvm4_*` users (picked from the MySQL and MariaDB documentation, listed in the report); `hacivat-testkit` exposes its login only to test code.
- FTL block `## m4-t01c-core`: every `intro-*` message the catalogs name (alg/02 HV-INTRO-01..03, -05, -11..13, -15, -16; `intro-hint-level-lowered`, `model-cache-disabled-by-policy`, `intro-catalog-race-retried`), same rules as Task 1a.

- [ ] **Step 1: Write the failing tests.**
```rust
// hacivat-session/tests/profile_m4.rs
#[test] fn profile_without_catalog_sensitive_round_trips_byte_for_byte() { /* profiles live in connections.toml (profile/store.rs:1): tests/fixtures/m2a_connections.toml and the M2b/M3 cases of tests/profiles_m2b.rs load and save identically (precedent m2a_file_round_trips_verbatim, profiles_m2b.rs:555; there is no tests/fixtures/profiles/ directory) */ }
#[test] fn catalog_sensitive_true_is_written_and_read() {}
#[test] fn meta_conn_stub_is_unavailable() {}
#[test] fn meta_join_stub_is_unavailable() {}
// hacivat-settings/tests/profile_data.rs
#[test] fn unknown_sections_survive_a_metadata_save() { /* a file with [ai] and [masks] sections, comments and odd spacing: save [metadata] → every other byte identical */ }
#[test] fn two_owners_write_through_one_writer() {}
#[cfg(unix)] #[test] fn profile_data_written_atomically_with_mode_0600() {}
// hacivat-introspect/tests/seams.rs (Part B)
#[test] fn stub_plan_refresh_widens_any_ddl_to_the_current_namespace() { let p = plan_refresh(&[executed("CREATE TABLE s.t(a int)", ddl_create_table("s", "t"))], &session_at("public"), TxnState::AutoCommit, &caps_pg()); assert_eq!(p.now, vec![Target::Namespace(ns("public"))]); }
#[test] fn stub_scope_selects_only_current() { assert_eq!(parse_scope("*:*", DialectId::Postgres).unwrap().select(&infos(), &ctx("db", "public")), set(["db.public"])); }
#[test] fn stub_check_state_is_unknown() {}
#[test] fn stub_stamps_and_facet_not_implemented() {}
#[tokio::test] async fn stub_load_facet_leaves_the_facet_not_loaded() {}
#[tokio::test] async fn stub_hub_serves_cache_only() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-session --test profile_m4`, `cargo test -p hacivat-settings --test profile_data`, then (Part B) `cargo test -p hacivat-introspect --test seams`. Expected: FAIL.
- [ ] **Step 3: Implement** Part A, then Part B after the rebase.
- [ ] **Step 4:** Run the tests, `cargo test --workspace --no-run`, `cargo xtask check-deps`, `cargo xtask docker check-pins`, `cargo xtask l10n check`, clippy, and the SAFE-06 run A, the read corpus and the P2-07 suites (`cargo test -p sql-safety --test bypass_run_a --test read_corpus --test pg_backend --test policy`, `cargo test -p hacivat-exec --test gate`: the occurrence model of C-46 lives in `gate.rs`, which this task touches at `:282`): the new field must not change one verdict. Expected: PASS.
- [ ] **Step 5: Commit** Part A `feat(m4): session, settings, sql-safety and fixture seams for metadata`; Part B `feat(introspect): introspection seams (fail-closed stubs)`.

**Review:** security-sensitive (opus). Probes: a `MetaQuery` constructible outside the pack module without tripping check-deps; a profile or profile-data file whose bytes change on round trip; a second writer of `profiles/<uuid>.toml`; any `sql-safety` verdict change; the `pg17wrap` image built from an unpinned tag or initialising after `pg_resetwal`; an `hv_fixture` grant wider than its two jobs; a grant to `hv_catalog` on a fixture database.

### Task 2: Mocks for the M4 surfaces 1–14 (W0, parallel with Tasks 1a–1c)

**Files:** `design/m4-explorer/{index.html,NOTES.md}`.

- [ ] **Step 1:** Build clickable harman-style mocks of surfaces 1–14 (section "New UI surfaces"), dark and light, EN and TR, 1440 × 900 and 960 × 600, using only DESIGN.md tokens and components; 15,000-table and Turkish-name fixtures (`müşteri`, `İSTANBUL_ŞUBE`, `"Users"` beside `users`, a name with a zero-width joiner).
- [ ] **Step 2:** NOTES.md records per surface: purpose, states, keyboard, copy (EN/TR, with Fluent ids from Task 1b's blocks), what is out of scope, and the decision on surface 14 (status segment or explorer-only).
- [ ] **Step 3:** PO review; apply changes; mark approved in NOTES.md. **Tasks 14, 20, 24, 25, 27 and 32 do not start before approval.** **Fallback** ("never wait on the question list"): if the PO has not answered within one wave after the mocks are ready, the controller rules on them (ledger line naming each surface and the decision), Task 14 proceeds, and later PO changes go to Task 32 as listed fixes.
- [ ] **Step 4: Commit** `design(m4): explorer tree, metadata states, scope editor and DDL view mocks`.

**Review:** opus, design-only (DESIGN.md Named Rules; Overflow Rule in TR at 960 px; One Filled Badge).

## W1 — Leaves

### Task 3: Model arena, kind schemas, identifier keys (HV-MODEL-01, HV-INTRO-14) (W1)

**Files:** see File ownership (T3); ADR 0008 `## m4-t03 — model and identifier keys`.

**Spike origin (R-M4-40):** the bodies of `kind.rs`, `name.rs`, `ids.rs`, `props.rs`, `namespace.rs` and `fixture.rs` are copied from `m4-spike-model` @ `d86313b` (commit body `spike origin d86313b`); the spike's 17 model tests come along and are reviewed as new tests. The spike's `IdentKey`/`FoldRule` move to `sql-ident/src/key.rs`.

**Interfaces:**
- Consumes: Task 1a's types; `sql_ident::fold` (`crates/sql/sql-ident/src/fold.rs:23`).
- Produces:
  - the arena (alg/02 HV-MODEL-01 "Per-namespace arenas"): breadth-first, index 0 the namespace root, a parent before its children, contiguous children (`first_child`, `child_count` private), O(n) layout by counting sort, insertion order kept within a parent (introspectors insert columns by ordinal); `NamespaceSnapshot::build` re-validates the layout and the kind schema; `children`, `children_of_kind`, `lookup(group, &str)` (one `HashMap<IdentKey, SmallVec<[u32; 2]>>` per `KindGroup`, namespace-scoped kinds only), `child_by_name` (scan of contiguous children for parent-scoped kinds), `object_key`, `find(&ObjectKey)` (server-id index first, then exact path; a path hit whose server id differs returns `None`), `iter_named(kinds)`, `replace_subtree`, `upsert_child`;
  - `ObjectKey::same_object` (server id when both sides have one, else kind, path, overload signature) with structural `Eq`/`Hash`;
  - the per-DBMS kind schema `allows_child`/`exists_in` for PG, MySQL, MariaDB, SQLite (alg/02 per-DBMS matrix rows 1, 2, 4); `database_level` (PG `Database`, MySQL/MariaDB `SchemaIsDatabase`, SQLite `File`);
  - **`sql_ident::key::match_key`** per HV-INTRO-14 rule 4 and R-M4-41: **PG** `fold` (ASCII-only, 63-byte cut; `ServerDependent` for non-UTF-8 server encodings with non-ASCII input, rule 3); **MySQL/MariaDB** database and table names exact when `lower_case_table_names = 0` and ASCII-caseless when 1 or 2; columns, routines, triggers, events ASCII-caseless; a probe text with a non-ASCII **cased** letter in a column name (any lctn) or a table name (lctn ≥ 1) → `Err(ServerDependent)`, which lookups turn into `Found` on an exact stored-name match and `Unknown(ServerDependent)` otherwise, never `Missing`; **SQLite** ASCII-caseless (ver/01 `#sqlite-case`: `Müşteri` = `müşteri`, `MÜŞTERİ` ≠ `müşteri`); `FoldRule` is the stored per-namespace policy;
  - `fixture` (feature `test-support`): `ns_pg(…)`, `pg_catalog(schemas, columns)`, `large_pg_catalog()` (15,000 tables / 160,000 columns), `table_subtree(..)` — real, used by every later meta/introspect/app test;
  - `name::has_invisible_chars(&str) -> bool` (bidi controls, zero-width characters, U+202E…) for the badge.

- [ ] **Step 1: Write the failing tests** (alg/02 HV-MODEL-01 test cases 1–6, edge cases 1–14, HV-INTRO-14 test cases, plus the spike's model tests):
```rust
#[test] fn pg_table_key_uses_oid() { let n = ns_pg("db", "public").table_oid("t", 16400, &["a", "b"]).finish(); let k = n.object_key(n.find("t")); assert_eq!(k.server_id, Some(ServerId::Oid(16400))); assert_eq!(k.path.as_slice(), &["db", "public", "t"]); assert_eq!(names(n.children(n.find("t"), ObjectKind::Column)), ["a", "b"]); }
#[test] fn rename_keeps_object_key_by_oid() { assert!(key_of(ns_with("t", 16400)).same_object(&key_of(ns_with("u", 16400)))); }
#[test] fn same_object_prefers_server_id_but_eq_is_structural() { assert_ne!(key_of(ns_with("t", 16400)), key_of(ns_with("u", 16400))); }
#[test] fn mysql_rename_changes_key() { assert_ne!(key_of(my_ns_with("t")), key_of(my_ns_with("u"))); }
#[test] fn overloads_are_two_objects_lookup_returns_both() {}
#[test] fn fk_out_of_scope_stays_unresolved() {}
#[test] fn case_twins_are_two_objects() { let n = ns_pg("db", "public").table_oid("Users", 1, &[]).table_oid("users", 2, &[]).finish(); assert_eq!(n.lookup(KindGroup::Relations, &ik_pg("users", false)).len(), 1); assert_eq!(n.lookup(KindGroup::Relations, &ik_pg("Users", true)).len(), 1); }
#[test] fn pg_relations_share_one_kind_group() {}   // table, view, matview, sequence, index, foreign table, composite type
#[test] fn children_contiguous_in_ordinal_order() {}
#[test] fn unknown_kind_kept_with_raw() {}
#[test] fn lossy_name_flag_on_nul_and_lone_surrogate_text() {}
#[test] fn non_nfc_names_stored_exactly() {}
#[test] fn inherited_columns_flagged() {}
#[test] fn temp_objects_flagged_temp() {}
#[test] fn pg_ident_key_ascii_only() { assert_eq!(ik_pg("MÜŞTERİ", false).as_str(), "mÜŞterİ"); }
#[test] fn mysql_lctn0_table_names_exact_columns_ascii_caseless() { assert_ne!(ik_my(0, "Items", NameClass::Table), ik_my(0, "items", NameClass::Table)); assert_eq!(ik_my(0, "ITEMS", NameClass::Column), ik_my(0, "items", NameClass::Column)); }
#[test] fn mysql_lctn1_ascii_i_never_becomes_dotless() { assert_eq!(ik_my(1, "ISTANBUL", NameClass::Table).as_str(), "istanbul"); }   // can fail under a Turkish fold (I → ı); the draft's İSTANBUL input could not
#[test] fn mysql_non_ascii_cased_column_is_server_dependent_until_pinned() { assert!(match_key(DialectId::MySql, &lctn(0), "Äb", false, NameClass::Column).is_err()); let n = my_ns_with_column("t", "Äb"); assert!(matches!(lookup_column(&n, "t", "äb"), Lookup::Unknown(UnknownReason::ServerDependent))); assert!(matches!(lookup_column(&n, "t", "Äb"), Lookup::Found(_))); }
#[test] fn sqlite_ident_key_ascii_lowercase() { assert_eq!(ik_sqlite("Müşteri"), ik_sqlite("müşteri")); assert_ne!(ik_sqlite("MÜŞTERİ"), ik_sqlite("müşteri")); assert_eq!(ik_sqlite("ITEMS"), ik_sqlite("items")); }   // ver/01 #sqlite-case; MySQL/SQLite spike F6
#[test] fn match_key_is_locale_free() { /* sql-ident/tests/key.rs: same keys under LC_ALL=tr_TR.UTF-8 (turkey-test) and C */ }
#[test] fn invisible_chars_detected() { assert!(has_invisible_chars("a\u{200D}b")); assert!(!has_invisible_chars("müşteri")); }
#[test] fn kind_schema_rejects_disallowed_child() {}
#[test] fn database_level_per_engine() {}
#[test] fn props_round_trip_through_the_codec_for_every_variant() {}   // runs once Task 11's codec body lands; until then it asserts the stub error (reported)
#[test] fn arena_rejects_child_before_parent_and_non_contiguous_children() {}
#[test] #[ignore = "perf"] fn memory_per_column_and_table_recorded() { /* 15,000 tables × 160,000 columns synthetic; prints bytes/column and bytes/table (alg/02 target ≤ 250 / ≤ 600, Inferred); not gating */ }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-meta --features test-support --test model --test kinds`, `cargo test -p sql-ident --test key`. Expected: FAIL.
- [ ] **Step 3: Implement.** No `to_lowercase`/`to_uppercase` (clippy); no Unicode case mapping anywhere in `match_key` (R-M4-41: Task 17 decides, with evidence, whether one is added).
- [ ] **Step 4:** Run the tests also under `LC_ALL=tr_TR.UTF-8`; clippy. Expected: PASS. Record the perf numbers (bytes per column and per table; the spike measured 256 B per column against alg/02's ≤ 250 B target, spike open question 2) in the report; memory is decided at the gate.
- [ ] **Step 5: Commit** `feat(meta): metadata arena, per-dialect kind schemas and locale-free identifier keys`.

**Review:** opus. Spike code reviewed as new code. Probes: a lookup that folds with locale or Unicode rules; a `Missing` answer for a non-ASCII MySQL column probe; a `Name` mutated after insert; a quadratic path on a 1,600-column table; `ObjectKey` `Eq` that prefers the server id (must be structural; identity is `same_object`).

### Task 4: Catalog query packs and preprocessor (HV-INTRO-04) (W1)

**Files:** `crates/core/hacivat-introspect/src/pack/{mod,parse,render,lint}.rs`; tests `crates/core/hacivat-introspect/tests/pack.rs`; `fuzz/fuzz_targets/pack_parse.rs`; ADR 0008 `## m4-t04 — query packs`.

**Interfaces:**
- Produces:
  - `Pack::parse_all(files: &[(&'static str /* file */, &'static str /* text */)]) -> Result<Pack, PackError { file, line: u32, kind: PackErrorKind }>` over `packs/<dialect>/*.hvq.sql`, merged in file-name order (`10-core`, `20-facets`, `30-state`, `40-system`, later milestones' `70-…`); a section name defined twice across files is a parse error (R-M4-32).
  - `Pack::render(&self, section: &str, ctx: &PackCtx) -> Result<Vec<RenderedQuery>, RenderError>` (a `Vec` because list binds chunk, R-M4-3); `RenderedQuery::as_meta_query(&self) -> MetaQuery<'_>` — the only call site of `MetaQuery::new` (check-deps `MetaQueryOnlyFromPacks`); `PackCtx { dialect: DialectId, version: u32 /* PG server_version_num, MySQL 80046 style */, flavor: Flavor, mode: Mode /* Full | Incremental | Brief | System */, level: Level, flags: PackFlags, params: Params }`.
  - Directives exactly as alg/02 HV-INTRO-04 (`--# if/elif/else/end`, `--- name: <Section> ---`, `--- returns: <col type, …> ---`, `:name`, `:[name]`, `#{macro}`), plus section flags `--- joinable ---` (allowed on a joined connection, R-M4-29: the rendered `MetaQuery` carries `joinable = true`) and `--- show-create ---` (R-M4-37). `Macro` is a closed enum: catalog prefix, numeric limits, and `#{ident:<bind-name>}` — the last allowed **only** in `show-create` sections and rendered only through `sql_ident::quote::quote(DialectId::MySql, …)`, refusing a name with NUL.
  - Placeholders `$n` (PG) and `?` (MySQL, MariaDB, SQLite); `:[name]` → `IN (…)` chunks of ≤ 1,000 items (oid lists included; the PG spike's literal `= ANY('{…}'::oid[])` becomes binds); an empty list → the section is skipped (no query); `reachable_variants(section) -> Vec<PackCtx>` for CI enumeration.
  - `lint(pack, dialect) -> Vec<LintError>`: canonical catalog-name spelling per dialect (HV-INTRO-14 rule 2: `INFORMATION_SCHEMA.TABLES` upper case in MySQL packs; PG catalogs written `pg_catalog.<name>` and `information_schema` lower case; `sqlite_schema` in SQLite packs), no `lower(`/`upper(`/`ILIKE`/`COLLATE` applied to a catalog name column (rule 1), no `UPDATE_TIME` and no `GROUP_CONCAT` in MySQL packs (alg/02 HV-INTRO-07; key parts are grouped client-side, MySQL/SQLite spike difference 5), no `<=` on `age(` in PG packs (R-M4-8), no `txid_current`/`pg_current_xact_id(` anywhere, `#{ident:…}` outside a `show-create` section, no directive-looking text inside string literals, nesting ≤ 4.

- [ ] **Step 1: Write the failing tests** (alg/02 HV-INTRO-04 test cases 1–6, edge cases 1–8):
```rust
#[test] fn version_gate_picks_else_on_older() { let q = render(PACK, "S", ctx_pg(140005)); assert!(q[0].sql.contains("-- old-branch-marker")); }   // fixture section: `--# if version >= 150000` / else
#[test] fn incremental_lines_removed_in_full_mode() {}
#[test] fn list_bind_expands_to_in_list_on_every_dialect() { let q = render_list(DialectId::MySql, &["t1", "t2"]); assert!(q[0].sql.contains("IN (?, ?)")); assert_eq!(q[0].binds.len(), 2); let p = render_list(DialectId::Postgres, &["t1", "t2"]); assert!(p[0].sql.contains("IN ($1, $2)")); }
#[test] fn list_bind_chunks_at_1000() { assert_eq!(render_list(DialectId::Postgres, &ids(2500)).len(), 3); }
#[test] fn empty_list_skips_section() { assert!(render_list(DialectId::Sqlite, &[]).is_empty()); }
#[test] fn unbalanced_if_is_parse_error_with_line() { assert!(matches!(Pack::parse(UNBALANCED), Err(PackError { line: 4, .. }))); }
#[test] fn directive_text_inside_string_literal_is_refused_by_lint() {}
#[test] fn nesting_deeper_than_4_is_error() {}
#[test] fn unknown_placeholder_is_a_render_error() {}
#[test] fn names_are_binds_never_macros() { /* a `#{…}` naming a user value is a parse error */ }
#[test] fn every_section_declares_returns() {}
#[test] fn reachable_variants_cover_every_branch() {}   // property over generated packs
#[test] fn canonical_catalog_case_is_linted() {}
#[test] fn lower_upper_ilike_collate_on_catalog_names_are_linted() {}
#[test] fn update_time_is_linted_in_mysql_packs() {}
#[test] fn rendering_is_byte_identical_under_tr_locale() {}
#[test] fn duplicate_section_across_files_is_an_error() { assert!(matches!(Pack::parse_all(&[("10-a.hvq.sql", SEC_S), ("20-b.hvq.sql", SEC_S)]), Err(PackError { file: "20-b.hvq.sql", .. }))); }
#[test] fn files_merge_in_name_order() {}
#[test] fn joinable_flag_reaches_the_meta_query() {}
#[test] fn show_create_ident_macro_quotes_backtick_and_refuses_nul() { assert!(render_show_create("a`b").sql.contains("`a``b`")); assert!(render_show_create("İ").sql.contains("`İ`")); assert!(render_show_create("t ").sql.contains("`t `")); assert!(try_render_show_create("a\0b").is_err()); }
#[test] fn show_create_macro_outside_show_create_section_is_a_lint_error() {}
#[test] fn group_concat_and_le_on_age_and_txid_current_are_linted() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-introspect --test pack`. Expected: FAIL.
- [ ] **Step 3: Implement** the parser (line-oriented, `OnceLock` cache), renderer (cache by `(section, ctx hash)`; binds per call) and linter; the fuzz target body (`Pack::parse` and `render` never panic).
- [ ] **Step 4:** Run the tests and `cargo +nightly-2026-11-01 fuzz build` for `pack_parse` in `fuzz/` (the M3b recipe: the `FUZZ_NIGHTLY` pin of `ci.yml:783`). Expected: PASS.
- [ ] **Step 5: Commit** `feat(introspect): catalog query packs with gated sections, list binds and a catalog-SQL lint`.

**Review:** security-sensitive. Probes: any path from user text to SQL text other than a bind or the `show-create` identifier macro; a macro that accepts a string outside `show-create`; `MetaQuery` built anywhere but `as_meta_query`; a lint gap that lets `lower(relname)` or `<=` on `age()` through; a chunk boundary off by one.

### Task 5: Metadata sessions — `MetaConn` (HV-SESS-01 metadata connection; HV-INTRO-01 step 4) (W1)

**Files:** see File ownership (T5); ADR 0004 `## m4-t05 — metadata sessions`.

**Interfaces:**
- Consumes: Task 1c's `meta` types; the M3 `QuerySpec` typed path with binds (`crates/core/hacivat-driver/src/typed.rs:52-71`), `Protocol::Extended`.
- Produces:
  - `SessionManager::meta_conn` body: a pool of `SessionKind::Introspection` sessions per `(profile, database)` (1 by default, up to 3 within the profile's budget; `Department::Introspection`, `hacivat-proto/src/lib.rs:194`), `application_name = "Hacivat (metadata)"` (PG) / `program_name` connect attribute (MySQL), statement timeout 60 s default (PG `statement_timeout`; MySQL `max_execution_time`, which applies to SELECTs only; MariaDB `max_statement_time` — different variable, flavor-gated).
  - **Read-only enforcement (R-M4-7):** PG `begin_snapshot` = `BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY`, then `SET LOCAL search_path = pg_catalog, pg_temp` (R-M4-46). MySQL/MariaDB: at connect **and after every reconnect** `SET SESSION TRANSACTION READ ONLY` and `SET time_zone = '+00:00'`; before each `begin_snapshot` assert the session read-only variable is 1 (`@@session.transaction_read_only`; MariaDB versions without it use `@@session.tx_read_only`, the gate picked from the MariaDB documentation and listed in the report) and answer `MetaError::ReadOnlyNotEnforced` on a mismatch; then `START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY`. No `group_concat_max_len` (no pack uses `GROUP_CONCAT`). SQLite main/attached schemas on a **separate `mode=ro` connection without the user-facing authorizer** (R-M4-38), opened on the blocking pool (never the user's connection), reading inside a deferred `BEGIN`; the `temp` schema only on the user's own connection when it is idle and outside a transaction (alg/02 HV-INTRO-08 step 4).
  - `rows(q: MetaQuery)` = typed query with binds, text values exactly as sent, cell cap off; `commit` ends the read-only transaction; drop without commit rolls back.
  - **Single-connection profiles** (`Profile.single_session`, SQLite `:memory:`): `meta_conn` waits until the user's session is idle and not in a transaction, and is refused (`MetaError::Busy`) while it is (alg/02 HV-INTRO-01 edge 10). Connection loss → `MetaError::ConnectionLost` (no silent retry; the engine restarts the task, Task 15).
  - **`MetaConn::join(&SessionHandle)`** (R-M4-29): accepted only for `SessionKind::Job(_, JobKind::{DdlApply, DriftCheck})` and the SQLite writer while a M7 apply holds it; a joined `begin_snapshot`/`commit` send nothing; `rows` refuses a `MetaQuery` with `joinable == false` (`MetaError::NotJoinable`) and any text whose first keyword is `BEGIN`, `START`, `COMMIT`, `ROLLBACK`, `SAVEPOINT`, `RELEASE` or `SET` (`MetaError::RefusedText`), as a second layer.

- [ ] **Step 1: Write the failing tests:**
```rust
#[tokio::test] async fn meta_conn_uses_introspection_priority_and_budget() {}
#[tokio::test(start_paused = true)] async fn single_connection_profile_waits_for_user_idle() { /* FakeDriver::gated(): user request busy → meta_conn pending; release → granted */ }
#[tokio::test] async fn never_runs_inside_a_user_transaction() { /* user BEGIN on a single-session profile → MetaError::Busy until COMMIT */ }
#[tokio::test] async fn connection_loss_reports_and_does_not_retry_silently() {}
#[tokio::test] async fn binds_are_never_spliced() { /* fake records the text: no bind value appears in it */ }
#[test] fn joined_conn_is_refused_on_console_and_editor_sessions() {}
#[tokio::test] async fn joined_conn_never_begins_or_ends_the_txn() { /* fake statement log: begin_snapshot + 3 rows + commit on a joined conn → exactly the 3 pack texts */ }
#[tokio::test] async fn joined_conn_refuses_non_joinable_queries() {}
#[tokio::test] async fn joined_conn_refuses_transaction_control_text() { /* a MetaQuery built in a test-support helper with "SET search_path = x", "begin", " COMMIT", "SAVEPOINT a" → RefusedText each */ }
// [docker: pg13, pg14, pg15, pg16, pg17 (pr), pg18]
#[tokio::test] #[ignore = "docker"] async fn pg_snapshot_txn_is_repeatable_read_read_only_with_pinned_search_path() { /* SHOW transaction_isolation = 'repeatable read'; SHOW transaction_read_only = 'on'; SHOW search_path = 'pg_catalog, pg_temp' inside begin_snapshot */ }
#[tokio::test] #[ignore = "docker"] async fn pg_meta_txn_refuses_writes() { /* CREATE TABLE and INSERT inside the txn: SQLSTATE 25006 */ }
#[tokio::test] #[ignore = "docker"] async fn no_xid_assigned_by_a_metadata_pass() { /* ver/01 #pg-canary: run 20 catalog queries incl. pg_current_snapshot(), then SELECT txid_current_if_assigned() IS NULL → true */ }
#[tokio::test] #[ignore = "docker"] async fn application_name_is_hacivat_metadata() { /* pg_stat_activity row for the backend */ }
#[tokio::test] #[ignore = "docker"] async fn statement_timeout_applies() { /* timeout 1 s, SELECT pg_sleep(2) → MetaError::Timeout */ }
// [docker: mysql80, mysql84 (pr), mariadb106, mariadb1011, mariadb114]
#[tokio::test] #[ignore = "docker"] async fn mysql_snapshot_sets_utc_read_only_session_and_consistent_snapshot() { /* @@session.time_zone = '+00:00'; session read-only variable = 1 */ }
#[tokio::test] #[ignore = "docker"] async fn mysql_meta_txn_refuses_dml_1792() { /* INSERT inside the txn → error 1792 */ }
#[tokio::test] #[ignore = "docker"] async fn mysql_meta_session_refuses_ddl() { /* mysql80, mysql84: CREATE TABLE hvm4_t05(a int) on the metadata conn → error 1792 (ver/01 #mysql-ro step k); afterwards the table is absent */ }
#[tokio::test] #[ignore = "docker"] async fn mariadb_meta_session_refuses_ddl() { /* mariadb106/1011/114: the same; the observed error code per server is recorded in the report and ADR 0004 ## m4-t05 (ver/01 never ran this on MariaDB) */ }
#[tokio::test] #[ignore = "docker"] async fn read_only_flag_is_restored_after_reconnect() { /* kill the metadata backend from a second session; next begin_snapshot asserts the flag again */ }
// sqlite (no Docker)
#[test] fn sqlite_meta_uses_a_read_only_connection_never_the_users() {}
#[test] fn sqlite_meta_conn_has_no_authorizer_and_refuses_writes() { /* pragma_table_list and BEGIN work; CREATE TABLE → SQLITE_READONLY */ }
#[test] fn sqlite_temp_is_read_on_the_users_connection_only_when_idle() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-session --test meta_conn`; `cargo test -p hacivat-session --no-run`. Expected: FAIL.
- [ ] **Step 3: Implement.** No new `Driver` trait method (Task 1c found none needed; if one is, report NEEDS_CONTEXT — the driver trait is not this task's).
- [ ] **Step 4:** Run the unit tests; then `<main>/target/docker/run-docker-tests.sh --set full -p hacivat-session --test docker_meta_conn` (C-4 has the stack up; at C-0 it is down and its start waits for the PO, so this step waits for C-4). Expected: PASS.
- [ ] **Step 5: Commit** `feat(session): read-only metadata sessions (read-only session and transaction per engine) and joined connections`.

**Review:** security-sensitive, **two independent opus reviews**. Probes: any path where a metadata query runs on the user's session inside a user transaction; a write or DDL that succeeds on a metadata connection; a reconnect that skips `SET SESSION TRANSACTION READ ONLY`; a `txid_current()`-style call; a bind value reaching the SQL text; a joined connection that issues `BEGIN`, `COMMIT`, `ROLLBACK`, `SAVEPOINT`, `SET` or any non-pack text; a metadata connection that outlives profile disconnect; SQLite `mode=ro` bypassed for attached files.

### Task 6: `sql-safety` DDL effects (the input of HV-INTRO-12) (W1)

**Files:** `crates/sql/sql-safety/src/ddl_effects.rs`; named lines in `crates/sql/sql-safety/src/backend/pg.rs` and `backend/interim.rs` (fill `StmtReport.ddl`); corpus `crates/sql/sql-testkit/corpus/ddl_effects/{pg,mysql,sqlite}.toml` (new files); tests `crates/sql/sql-safety/tests/ddl_effects.rs`; ADR 0005 `## m4-t06 — DDL effects`.

**Interfaces:**
- Consumes: Task 1c's `DdlEffect` types; the PG parse tree (`sql-oracle-pg` tree entry, M3b T9) and the interim sqlparser path (M3b T10).
- Produces: for every statement the classifier already marks as DDL, `DdlEffect`s per alg/02 HV-INTRO-12 step 2: `CREATE/ALTER/DROP/RENAME <kind> name` (with `cascade`), `CREATE INDEX i ON t` (`on = t`, `concurrently`), PG `DROP INDEX i` (`on = None`), `ALTER TABLE t ADD FOREIGN KEY … REFERENCES u` (`referenced = [u]`), `COMMENT ON <kind> x` (`action = Comment`, `on` = the owning table for columns), `GRANT/REVOKE … ON …` (schema-wide `ON ALL TABLES IN SCHEMA s` → `kind = Schema`), `CREATE TABLE … AS`, `SELECT … INTO t` (PG), MySQL `RENAME TABLE a TO b, c TO d` (one effect per pair), `DROP TABLE a, b, c` (one per name), `CREATE/DROP SCHEMA|DATABASE`, `ALTER TYPE`, temp objects (`temp = true`), `ALTER … SET SCHEMA`; **opaque** (`opaque = true`) for `DO`, `CALL`, `EXECUTE`, `PREPARE … EXECUTE`, dynamic SQL, MySQL executable comments carrying DDL, PG `DROP OWNED BY` (`StmtKind::DropOwned`, M3c P2-07: it names a role, not the objects it drops) and `REASSIGN OWNED BY` (classified `Alter`, names no object), and any DDL-kind statement the backend could not read. Names exactly as written (quotes kept as `quoted`, no folding, `sql-ident`).
- **Scope of `ddl`:** DDL-class statements plus `DO`, `CALL`, `EXECUTE`, `PREPARE`/`EXECUTE IMMEDIATE` and MySQL executable comments. A `SELECT f()` whose function runs DDL is **not** detected; ADR 0005 `## m4-t06` records it; only background validation (Task 22) covers it.
- **Never changes a verdict:** `ddl` is computed after classification and read by nobody inside `sql-safety`.

- [ ] **Step 1: Write the failing tests:**
```rust
#[test] fn create_table_gives_one_create_table_effect() { let r = pg("create table s1.t(x int)"); assert_eq!(r.ddl, [eff(Create, Table, "s1.t")]); }
#[test] fn create_index_names_its_table() {}
#[test] fn pg_drop_index_has_no_table() { assert_eq!(pg("drop index i").ddl[0].on, None); }
#[test] fn create_index_concurrently_flagged() {}
#[test] fn alter_add_fk_lists_referenced() {}
#[test] fn comment_on_column_targets_its_table() {}
#[test] fn grant_on_all_tables_in_schema_targets_the_schema() {}
#[test] fn ctas_and_select_into_create() {}
#[test] fn mysql_rename_table_multi_gives_two_renames() { assert_eq!(my("RENAME TABLE a TO b, c TO d").ddl.len(), 2); }
#[test] fn mysql_drop_table_list_gives_three() { assert_eq!(my("DROP TABLE a, b, c").ddl.len(), 3); }
#[test] fn drop_schema_cascade() {}
#[test] fn create_database() {}
#[test] fn alter_type_targets_the_type() {}
#[test] fn set_schema_carries_the_new_schema() {}
#[test] fn temp_objects_flagged_temp() { for s in ["create temp table t(a int)", "create temporary table t(a int)"] { assert!(pg(s).ddl[0].temp, "{s}"); } assert!(sqlite("create table temp.t(a)").ddl[0].temp); }
#[test] fn do_call_execute_are_opaque() {}
#[test] fn drop_owned_and_reassign_owned_are_opaque() { for s in ["drop owned by r", "drop owned by r cascade", "reassign owned by r to s"] { assert!(pg(s).ddl.iter().all(|e| e.opaque) && !pg(s).ddl.is_empty(), "{s}"); } }
#[test] fn mysql_executable_comment_ddl_is_opaque() { assert!(my("/*!50000 CREATE TABLE z(a int) */").ddl[0].opaque); }
#[test] fn unparseable_ddl_is_opaque() {}
#[test] fn dml_and_select_have_no_ddl() {}
#[test] fn quoted_mixed_case_names_kept_exact() { let e = &pg(r#"alter table "Sales"."Ürün" rename to "Ürünler""#).ddl[0]; assert_eq!(e.name.as_ref().unwrap().to_string(), r#""Sales"."Ürün""#); }
#[test] fn ddl_effects_never_change_the_verdict() { /* every case of the existing corpora: report with ddl cleared == report as before this task (golden of the `m4` base `fb98cf1`'s reports, which carry M3c P2-07: `StmtKind::DropOwned`, `DestructiveKind::TruncateCascade` and each destructive target's own range) */ }
#[test] fn corpus_ddl_effects() { /* data-driven: corpus/ddl_effects/*.toml */ }
```
- [ ] **Step 2:** Run `cargo test -p sql-safety --test ddl_effects`. Expected: FAIL.
- [ ] **Step 3: Implement** in both backends; the corpus files.
- [ ] **Step 4:** Run the crate's full tests under `LC_ALL=tr_TR.UTF-8` too, `bypass_run_a`, `read_corpus`, and `cargo xtask check-deps` (no new edge). Expected: PASS; zero verdict changes.
- [ ] **Step 5: Commit** `feat(sql-safety): per-statement DDL effects for catalog refresh planning`.

**Review:** security-sensitive (opus). Probes: any verdict, class or effect bit that differs from M3; a DDL statement that yields no effect and is not opaque (would refresh nothing); a name folded or unquoted.

### Task 7: DDL IR and renderers; quoting; placement, qualification, options subset (HV-DDL-07 renderer layer, -09, -10, -11 subset) (W1)

**Files:** see File ownership (T7); ADR 0008 `## m4-t07 — DDL IR, renderers and quoting`.

**Interfaces:**
- Consumes: Task 1a's IR and option types.
- Produces:
  - **`sql_ident::quote`** (R-M4-28; body in M3's crate, named exception): `needs_quoting(d, name, &QuoteCtx)` (reserved words per dialect from `sql-dialect`'s keyword tables, case, non-identifier characters, a leading digit, a trailing space), `quote(d, name, &QuoteCtx)` (PG/SQLite `"…"` with `"` doubled; MySQL/MariaDB backticks with `` ` `` doubled, or `"…"` under `ANSI_QUOTES`), `quote_qualified(d, &QualifiedName, &QuoteCtx)`, `string_literal(d, text, &QuoteCtx) -> Result<String, QuoteError>` (`'` doubled; MySQL backslashes doubled unless `no_backslash_escapes`; PG `E''` never produced, `standard_conforming_strings` respected; NUL → `QuoteError::Nul`). M5 Task 6 extends its keyword tables; M6's `valfmt::literal` delegates text to `string_literal`; Task 4's `show-create` macro calls `quote`.
  - `render::{pg, mysql, sqlite}::Renderer` bodies for the M4 view-only variants (`CreateSchema`, `CreateTable`, `CreateIndex`, `CreateView`, `CreateRoutine` (server text), `CreateTrigger` (server text), `CreateSequence`, `CreateType` (PG enum/domain/composite/range), `Comment`, `Grant`, `Owner`, `RowSecurity` (PG), `Raw { origin: ServerText }`); every other declared variant answers `RenderError::Unsupported { kind, dialect }` (M7 fills them). MySQL and MariaDB render column defaults and `ON UPDATE` texts verbatim as each flavor reports them (F9: MySQL `eu`/`CURRENT_TIMESTAMP`, MariaDB `'eu'`/`current_timestamp()`), never re-quoted.
  - `placement::place(constraints, mode, dialect, plan) -> Vec<Placed>` (alg/03 HV-DDL-09 algorithm and dialect overrides: MySQL FK never inline, SQLite inside table only with notice `ddl-sqlite-placement-forced`, cycles and later-created references → after table); `qualify::qualify(name, ctx) -> QualifiedRef` (HV-DDL-10 Auto/Never/Always; Auto keeps names unqualified only when every object is in one schema equal to the current schema, and qualifies names that exist in more than one schema of the scope; server text never rewritten); `options::applicable(spec, category, target)` for the v1 core subset (categories in M4: `CreateScript`, `DefinitionsByServer`).

- [ ] **Step 1: Write the failing tests** (alg/03 HV-DDL-07 tests 1–2, HV-DDL-09 and -10 test cases, HV-DDL-11 cases that do not need M7):
```rust
#[test] fn pg_table_canonical_type_null_omitted() { assert_eq!(render_pg(&create_table("test_schema", "test_table", &[col("column1", "integer")])), "CREATE TABLE test_schema.test_table (\n    column1 integer\n);\n"); }
#[test] fn pg_comment_escapes_quote() { assert_eq!(render_pg(&comment_col("test_schema.test_table.column1", "PM's approver of record")), "COMMENT ON COLUMN test_schema.test_table.column1 IS 'PM''s approver of record';\n"); }
#[test] fn mysql_comment_escaping_respects_no_backslash_escapes() {}
#[test] fn mysql_fk_is_never_inline() {}
#[test] fn sqlite_constraints_inside_table_forced_with_notice() {}
#[test] fn inside_column_single_column_pk() { assert!(render_pg_with(Placement::InsideColumn, &t_with_pk("t", "id", Some("pk"))).contains("id integer CONSTRAINT pk PRIMARY KEY")); }
#[test] fn fk_cycle_is_postponed_after_tables() {}
#[test] fn qualify_auto_one_schema_equal_to_current_is_unqualified() {}
#[test] fn qualify_auto_two_schemas() { /* app.t1 + sales.t2, current app → "t1", "sales.t2" */ }
#[test] fn qualify_ambiguous_name_is_qualified() {}
#[test] fn server_text_is_never_rewritten() {}
#[test] fn turkish_identifier_quoted_on_pg() { assert!(render_pg(&create_table("public", "MÜŞTERİ", &[col("ad", "text")])).contains(r#"public."MÜŞTERİ""#)); }
#[test] fn reserved_word_is_quoted_per_dialect() { /* PG "order", MySQL `order`, SQLite "order" */ }
#[test] fn nul_in_comment_is_refused() {}
#[test] fn or_replace_applies_to_pg_view_not_table() {}
#[test] fn mysql_skip_definer_drops_definer() {}
#[test] fn unsupported_variant_is_an_explicit_error() {}
#[test] fn output_is_identical_under_en_and_tr() {}
#[test] fn ir_goldens_byte_stable() { /* tests/golden/ir/<dialect>/*.sql, UPDATE_GOLDEN=1 to bless (R-M4-16) */ }
#[test] fn every_alg03_variant_is_declared_and_non_m4_ones_refuse() {}
#[test] fn mysql_and_mariadb_default_texts_kept_verbatim_per_flavor() {}
#[test] fn pg_row_security_renders_enable_statement() {}
// crates/sql/sql-ident/tests/quote.rs
#[test] fn string_literal_doubles_quotes_and_refuses_nul() { assert_eq!(string_literal(DialectId::Postgres, "PM's", &qc()).unwrap(), "'PM''s'"); assert!(matches!(string_literal(DialectId::Postgres, "a\0", &qc()), Err(QuoteError::Nul))); }
#[test] fn mysql_backslash_doubled_unless_no_backslash_escapes() {}
#[test] fn mysql_backtick_doubled_and_ansi_quotes_respected() { assert_eq!(quote(DialectId::MySql, "a`b", &qc()), "`a``b`"); }
#[test] fn trailing_space_and_turkish_names_need_quoting() {}
#[test] fn quote_is_identical_under_tr_locale() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-ddl`. Expected: FAIL.
- [ ] **Step 3: Implement.** Generated text never comes from Fluent; `\n` line endings always; four-space indentation; one trailing newline per statement.
- [ ] **Step 4:** Run the tests under `LC_ALL=tr_TR.UTF-8`; `cargo xtask check-deps` (`DdlIsPure`). Expected: PASS.
- [ ] **Step 5: Commit** `feat(ddl): typed DDL IR with PostgreSQL, MySQL/MariaDB and SQLite renderers, placement and qualification`.

**Review:** opus. Probes: a name emitted unquoted that needs quotes (Turkish, reserved, mixed case, trailing space); MySQL escaping under `NO_BACKSLASH_ESCAPES`; a renderer reading the process locale; a CRLF in output on Windows; a second quoting function anywhere in `hacivat-ddl`.

### Task 8: Tree widget (W1, gpui)

**Files:** see File ownership (T8); ADR 0007 `## m4-t08 — tree widget`.

**Spike origin (R-M4-40):** `m4-spike-tree` @ `2a31b8b` (`crates/ui/hacivat-ui/src/tree/{mod,flat,typeahead,a11y,view}.rs`, `tests/{tree_flat,tree,tree_perf}.rs`, `tests/tree_fixture/mod.rs`; `spike-tree-report.md`): the bodies are copied into Task 1b's API (commit body `spike origin 2a31b8b`), and the spike's ~1,200-line `view.rs` is split into submodules (`view/{mod,element,keyboard,pointer,anchor}.rs`). The 12 pure and 21 gpui spike tests come along and are reviewed as new tests.

**Interfaces:**
- Consumes: Task 1b's API; `hacivat-ui` list/scroll patterns; DESIGN.md "Explorer Tree" (23 px rows, 14 px per level, chevron, 12 px kind icon, 13 px label ellipsized with the full name as tooltip, flags after the label, outlined badge, status glyph, mono meta right-aligned, hover fill, blue-tint selection with a 1 px blue left edge and blue icon).
- Produces:
  - **Flattening** (`flat.rs`): the expanded tree as one `Vec<FlatRow>` (32 bytes per row) built by an iterative DFS (no recursion limit; a 50,000-deep tree is tested); expand/collapse splice one subtree; `parent_of`, `subtree_end`; the expansion `KeySet` hashes `TreeKey`s with a Fibonacci hasher (keys are already hashes).
  - **Virtualization**: a pixel scroll offset; each frame builds only the rows intersecting the viewport plus `TREE_OVERSCAN_ROWS = 2` on each side (900 px → ≤ 44 rows); cut labels are reported to the text registry as scrolling.
  - **Lazy children**: `NotLoaded` draws a chevron and emits `Expand(key)` once per expansion — also for nodes restored expanded, revealed by an ancestor, or dropped back to `NotLoaded` while expanded (so a cancelled load never strands a node); `Loading`/`Failed` draw one child row with the model's label (`text-2`; `Failed` adds `CircleAlert`, never red; no motion); neither takes the cursor.
  - **Expansion and anchoring**: expansion survives model generations (a key that disappears and returns comes back expanded); on a generation change the first visible node keeps its screen position (relocated by key); expand/collapse above the viewport shifts the offset by the spliced rows; collapsing the subtree that holds the top row makes the collapsed node the top row.
  - **Selection** by key (follows reorders). Single selection by default; `set_multi_select(true)` gives ⌘/Ctrl-click toggle, Shift-click and Shift-arrows range from the anchor, Space toggle. **M4's explorer uses single selection** (R-M4-50): a hidden selected key under a collapsed ancestor is a multi-select question for the milestone that adds a bulk action.
  - **Keyboard** (on key down while focused; never `bind_keys`; any ⌘/Ctrl/Alt/fn chord is left to the host keymap): ↑/↓ (Shift extends in multi-select), → expands or moves to the first child, ← collapses or moves to the parent, Home/End, PageUp/PageDown (viewport − 1), Enter = `Activate`, Shift-F10 and the menu key = `ContextMenu` at the row's bottom-left, printable `key_char` = type-ahead; nothing fires while `FocusedInput.composing` (IME). **Group headers take keyboard focus but never the selection** (R-M4-49, ARIA practice): arrows stop on them, → / ← / Enter expand or collapse them, actions and `SelectionChanged` ignore them, type-ahead skips them.
  - **Type-ahead** (`typeahead.rs`, no casing function, INV-3): a prefix buffer reset after 1 s of the executor clock, cycling on a repeated letter, wraparound; `Fold::Tolerant` (ASCII case, Turkish pairs, `I i İ ı` one letter, decomposed `I`+U+0307 read as `İ`) or `Fold::Locale { turkic }`. **The widget default is `Locale { turkic: false }`; the explorer sets `Tolerant` when the UI language or region is Turkish and follows the filter's Turkish-tolerant toggle** (D-074, R-M4-50; Task 14). Type-ahead for IME scripts (CJK) needs an input handler on the tree and is not in v1 (Deferred).
  - **Pointer**: click selects; double-click activates; chevron mouse-down toggles without selecting or activating; right-click selects (unless already in the selection) and emits `ContextMenu` at the pointer; a group-header click toggles it.
  - **Look** per DESIGN.md (the spike's GPUI finding: border-only quads are split into four strips, so the selection edge is a 1 px child fill and the keyboard-focus ring an overlay with no layout shift, shown only after keyboard input); the badge is outlined (One Filled Badge holds); status glyphs amber/green/text-2/text-3, never red; nothing below 11 px.
  - **Accessibility** (the M3c T40 pattern): while `Window::is_a11y_active()` the root is `Role::Tree` (named by `set_a11y_label`) with synthetic `TreeItem` children for the rows on screen ± 5 (level, expanded, position and size in set, selected, description = status text, bounds), the cursor item as active descendant, ids derived from `(key, kind)`, actions Focus, Click (= activate), Expand, Collapse, ScrollIntoView, ShowContextMenu; items are computed in the element's prepaint and handed to the root through a shared `Rc`, so there is **no one-frame lag**; inactive: no role, no item, no string. The full screen-reader pass is M9 (L4).
  - ADR 0007 `## m4-t08` records: the flat-vector model, the Fibonacci key hasher, anchoring by key, the lag-free a11y hand-off, the border-strip finding, R-M4-49 and R-M4-50.

- [ ] **Step 1: Write the failing tests** (the spike's tests, plus):
```rust
// tree_flat.rs (pure): flattening order and positions, NotLoaded reporting, Loading/Failed rows, splice == rebuild, parent/subtree_end,
// 50,000-deep tree, tolerant and locale folds (Turkish İ/ı, decomposed İ, ç ≠ c), type-ahead reset/cycle/wrap/skip, a11y item fields and ids
#[gpui::test] fn only_visible_rows_are_built(cx) { /* 100,000 rows, 900 px viewport → built rows ≤ 44 (≤ 39 visible + 2 overscan each side) */ }
#[gpui::test] fn expansion_state_survives_model_generation_change(cx) {}
#[gpui::test] fn anchor_row_kept_when_rows_inserted_above(cx) {}
#[gpui::test] fn right_expands_left_collapses_and_goes_to_parent(cx) {}
#[gpui::test] fn expand_unloaded_emits_expand_once(cx) {}
#[gpui::test] fn loading_and_failed_child_rows_render_their_labels(cx) {}
#[gpui::test] fn selection_follows_key_after_reorder(cx) {}
#[gpui::test] fn group_header_rows_take_focus_but_never_selection(cx) { /* ↓ stops on "Tables"; ← collapses it; SelectionChanged never names it; type-ahead skips it */ }
#[gpui::test] fn a11y_roles_tree_and_treeitem_with_level_and_expanded(cx) {}
#[gpui::test] fn row_geometry_23px_and_11px_floor(cx) { /* scene check */ }
#[gpui::test] fn chevron_click_toggles_without_activating(cx) {}
#[gpui::test] fn no_binding_fires_during_ime_composition(cx) {}
#[gpui::test] fn status_slot_draws_glyph_and_exposes_its_text(cx) {}
#[gpui::test] fn expand_unloaded_reannounces_after_drop_to_not_loaded(cx) {}
#[gpui::test] fn typeahead_fold_is_set_by_the_host(cx) {}
#[gpui::test] fn multi_select_range_and_toggle(cx) {}
#[gpui::test] fn a11y_inactive_builds_nothing(cx) {}
#[test] #[ignore = "perf"] fn flatten_162k_rows_under_50ms() { /* tree_perf.rs: 162,041 nodes fully expanded (1 db → 20 schemas → "Tables" → 100 tables → 80 columns); spike 1.77 ms */ }
#[test] #[ignore = "perf"] fn row_build_p95_under_2ms_with_and_without_at() { /* 400 scroll frames (small steps and 7,919-row jumps), then 200 with AT active; spike 0.32 ms p95 both; the whole test-platform frame is reported only */ }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-ui --test tree_flat` and `cargo test -p hacivat-ui --features scene-check --test tree`. Expected: FAIL.
- [ ] **Step 3: Implement.** Semantic tokens only; no core types; the widget owns no user-visible string (every label comes from the model).
- [ ] **Step 4:** Run the tests and the perf line (`HACIVAT_PERF=1 cargo test --release -p hacivat-ui --test tree_perf -- --ignored`); render the `hacivat-ui` scene checks. Expected: PASS. Real-window first-paint shaping is measured by Task 14's perf lines and the shots, not here (the test platform's text system is cheaper).
- [ ] **Step 5: Commit** `feat(ui): virtualized tree widget with lazy children, scroll anchoring, type-ahead and keyboard`.

**Review:** opus (gpui). Spike code reviewed as new code. Probes: rows built for the whole model; an expansion lost on a model swap; a jump of the viewport after a publish; a group header that becomes the selection or is unreachable by keyboard; a casing function in type-ahead; a status glyph without its text alternative; text-3 on a raised surface; a one-frame a11y lag.

## W2 — Model core, grammars, assembly, explorer adapter

### Task 9: Snapshots, `ModelView`, resolver, negative cache, search (HV-MODEL-02) (W2)

**Files:** see File ownership (T9); ADR 0008 `## m4-t09 — snapshots and lookups`.

**Spike origin (R-M4-40):** the bodies of `snapshot.rs` (`Catalog`, `publish`, `CatalogSnapshot`) are copied from `m4-spike-model` @ `d86313b`, including its torn-read test.

**Interfaces:**
- Consumes: Task 3's model; Task 1a's snapshot types; the session's current-schema facts (`SchemaCtx`, `crates/core/hacivat-session/src/session/schema.rs:15`) mapped into `SearchPath` by the caller.
- Produces:
  - `Catalog::new(server)`, `Catalog::current() -> Arc<CatalogSnapshot>` (one atomic load), `Catalog::publisher() -> Option<Publisher>` (the single writer, handed once to the engine), `Publisher::publish(changes) -> Result<Published, ModelError>` (alg/02 HV-MODEL-02 steps 1, 3, 4: shallow clone, apply namespace puts/removals/subtree upserts and load states **all or nothing**, `generation + 1`, attach the delta, store, broadcast; untouched namespaces stay `Arc::ptr_eq`); `Catalog::subscribe() -> broadcast::Receiver<CatalogEvent>` (feature `catalog`). References are resolved **on demand** (`CatalogSnapshot::resolve(&Ref)`, server id preferred among overloads); alg/02's publish-time reverse-index re-resolution (step 2) is not built (R-M4-26).
  - `SnapshotView { snap, search_path, layers }` implementing **`ModelView`** with `Def = ObjHandle` (step 5: walk the dialect search path; not loaded at a sufficient level → `Unknown` and call `on_unknown(ns)` once per namespace per generation; loaded and found → `Found`; loaded and not found → `Missing` only when no scope filter excludes it and the key is not `ServerDependent`, R-M4-41); `ModelView::find_by_server_id(database, ServerId)` over relation kinds (PG OIDs are per database); `CatalogSnapshot::fk_referencing(&ObjectKey) -> Lookup<Vec<ObjHandle>>` (reverse FK edges built lazily once per generation in a `OnceLock`; `Unknown` while any in-scope namespace is below `Structure`).
  - `SearchPath::{for_pg(path, temp), for_mysql(db), for_sqlite(attached)}` (alg/02 HV-MODEL-02 per-DBMS matrix: PG `pg_temp` (relations/types) → search_path with `$user` → `pg_catalog` implicitly first if absent; MySQL current database; SQLite `temp` → `main` → attached in attach order); negative cache: a `Missing` answer for a name schedules at most one `on_missing_probe(ns, name)` per namespace per 30 s (step 6); `Catalog::pinned_generations()`.
  - `search::search(&CatalogSnapshot, &SearchQuery) -> Vec<ObjHit>` (ASCII-caseless, locale-free, bounded by `limit`, deterministic order) over `NamespaceSnapshot::iter_named(kinds)` (M8's `search_objects`; Task 14's Turkish-tolerant filter iterates the same iterator).

- [ ] **Step 1: Write the failing tests** (alg/02 HV-MODEL-02 tests 1–5, 7; edge cases 1–10):
```rust
#[test] fn found_and_missing_in_a_loaded_namespace() { let v = view_with("public", &["t"]); assert!(matches!(v.lookup(&p("t"), KindGroup::Relations), Lookup::Found(_))); assert!(matches!(v.lookup(&p("zz"), KindGroup::Relations), Lookup::Missing(_))); }
#[test] fn not_loaded_is_unknown_and_enqueues_once_for_100_lookups() { let (v, hits) = view_counting_unknown("sales"); for _ in 0..100 { assert!(matches!(v.lookup(&p("sales.x"), KindGroup::Relations), Lookup::Unknown(UnknownReason::NotLoaded))); } assert_eq!(hits.get(), 1); }
#[test] fn publish_shares_untouched_namespaces_ptr_eq() {}
#[test] fn old_readers_keep_their_generation() {}
#[test] fn view_filter_never_affects_resolution() {}
#[test] fn out_of_scope_is_unknown() { assert!(matches!(scoped_view_excluding("hr").lookup(&p("hr.emp"), KindGroup::Relations), Lookup::Unknown(UnknownReason::OutOfScope))); }
#[test] fn failed_namespace_is_unknown_with_reason() {}
#[test] fn first_load_in_progress_is_unknown_never_missing() {}
#[test] fn pg_search_path_order_is_temp_then_path_then_pg_catalog() {}
#[test] fn pg_catalog_implicitly_first_when_absent() {}
#[test] fn mysql_resolves_in_the_current_database_only() {}
#[test] fn sqlite_order_is_temp_main_attached() {}
#[test] fn negative_probe_at_most_once_per_ttl_per_namespace() {}   // paused clock: 30 s
#[test] fn removed_namespace_still_seen_by_old_generation() {}
#[test] fn fk_resolves_after_target_namespace_replaced() { /* an FK in sales still resolves after public is replaced and its arena indexes shift (spike) */ }
#[test] fn find_by_oid_is_scoped_by_database() {}
#[test] fn fk_referencing_finds_cross_namespace_children() {}
#[test] fn fk_referencing_is_unknown_while_a_namespace_is_names_only() {}
#[test] fn facet_not_loaded_never_blocks_object_existence() {}
#[test] fn server_dependent_key_is_unknown_never_missing() {}
#[test] fn search_is_locale_free_and_bounded() {}
#[test] fn search_order_is_deterministic() {}
proptest! { #[test] fn no_torn_reads_8_threads(ops in ops_strategy(10_000)) { /* every lookup's answers come from one generation */ } }
#[test] #[ignore = "perf"] fn publish_5000_namespaces_under_1ms() { /* snapshot_perf.rs; publishes prebuilt Arc<NamespaceSnapshot>s only (index building excluded and reported separately: the spike's 2.6–3.4 ms "put one prebuilt namespace" included it); p95 over 200 publishes; R-M4-2 trigger */ }
#[test] #[ignore = "perf"] fn lookup_hit_does_not_allocate() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-meta --features test-support --test snapshot --test view --test snapshot_props --test search`. Expected: FAIL.
- [ ] **Step 3: Implement.** `arc-swap` `ArcSwap<CatalogSnapshot>`; `rcu` only as a misuse guard (the publisher is unique); no lock on the read path.
- [ ] **Step 4:** Run the tests and the perf line; report the publish p95. Expected: PASS.
- [ ] **Step 5: Commit** `feat(meta): immutable catalog snapshots and tri-state lookups with search paths`.

**Review:** opus. Spike code reviewed as new code. Probes: a `Missing` answer from an unloaded, out-of-scope or `ServerDependent` lookup; a reader that can observe two generations in one lookup; a publish that copies objects (not `Arc`s) or applies part of a change list; the probe firing more than once per TTL; an arena index stored in a `Ref`.

### Task 10: Existence-diff merge, catalog delta, mass-deletion guard (HV-INTRO-11) (W2)

**Files:** see File ownership (T10); ADR 0008 `## m4-t10 — merge`.

**Interfaces:**
- Produces: `merge` body per alg/02 HV-INTRO-11 algorithm steps 1–10; identity by `ObjectKey::same_object` (server id when present, else kind, `IdentKey` path and overload signature); `NamespaceResult.existent` keyed by **`ObjectKind`**: deletions only from a successful existence set of that kind (a `Failed` set keeps the cached objects of that kind and marks it stale; a failed index existence query never disables table deletions); the guard per R-M4-22 (`MergeOutcome::PendingConfirmation` for `Trigger::Background`; for `Trigger::User` the same outcome, and the engine asks the UI); rename hints for name-only engines when a dropped+created pair of one kind shares ≥ 90 % of column signatures (`Confidence` < 1 except identical signatures = 1.0); a create/drop pair of the same server id across two in-scope namespaces is one move; deltas over 10,000 objects become a coarse `NamespaceReplaced` event; `apply_delta` (serde round trip, used by tests and later by Karagöz).

- [ ] **Step 1: Write the failing tests** (alg/02 HV-INTRO-11 tests 1–6; edge cases 1–13):
```rust
#[test] fn created_dropped_kept_and_kept_children_shared() { let o = merge_pg(m(&["a", "b"]), e(&["a", "c"]), c(&["c"])); assert_eq!(o.delta.created, keys(&["c"])); assert_eq!(o.delta.dropped, keys(&["b"])); assert!(children_ptr_eq(&o, "a")); }
#[test] fn rename_via_oid_keeps_children() {}
#[test] fn failed_existence_disables_deletions_and_marks_stale() {}
#[test] fn mysql_rename_hint_confidence_one() { let o = merge_my(m_cols("a", &["x", "y", "z"]), e(&["b"]), c_cols("b", &["x", "y", "z"])); assert_eq!(o.delta.renamed[0].2, Confidence::ONE); }
#[test] fn mass_drop_background_is_pending_confirmation() { assert!(matches!(merge_bg(m_n(100), e_n(10), c(&[])), MergeOutcome::PendingConfirmation { vanished: 90, .. })); }
#[test] fn mass_drop_user_triggered_is_pending_confirmation_too() {}
#[test] fn under_20_objects_is_never_guarded() {}
#[test] fn exactly_half_is_not_guarded() { /* 20 objects, 10 vanish → Applied (rule is > 50 %) */ }
#[test] fn nineteen_objects_all_vanishing_is_not_guarded() { /* 19 objects, 19 vanish → Applied (rule is ≥ 20 objects, D-039) */ }
#[test] fn twenty_objects_eleven_vanishing_is_guarded() {}
#[test] fn failed_index_existence_keeps_tables_deletable() {}
#[test] fn set_schema_with_both_namespaces_is_one_move() {}
#[test] fn kind_change_same_name_is_drop_and_create() {}
#[test] fn oid_reuse_with_another_kind_is_drop_and_create() {}
#[test] fn changed_child_without_parent_row_replaces_all_children() {}
#[test] fn object_in_c_not_in_e_is_ignored() {}
#[test] fn object_in_e_not_in_m_nor_c_schedules_a_fragment_and_invents_nothing() {}
#[test] fn duplicate_identity_in_c_last_wins() {}
#[test] fn huge_delta_is_coarse() {}
#[test] fn facets_of_altered_invalidated_of_kept_preserved() {}
proptest! { #[test] fn merge_is_pure_and_prev_untouched(case in merge_case()) {} #[test] fn merge_then_full_equals_full(case in merge_case()) { /* incremental merge result == merge of a Full result built from the same final state */ } }
#[test] #[ignore = "perf"] fn merge_160k_columns_under_50ms() {}   // alg/02 "tens of milliseconds"
```
- [ ] **Step 2:** Run `cargo test -p hacivat-meta --test merge --test merge_props`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Run the tests and the perf line. Expected: PASS.
- [ ] **Step 5: Commit** `feat(meta): identity-preserving existence-diff merge with a mass-deletion guard`.

**Review:** opus (data integrity). Probes: any deletion from a failed existence set; `PendingConfirmation` that already changed the snapshot; a rename hint treated as identity; O(n²) matching.

### Task 11: Codec, cache persistence, format versioning, encryption, fingerprints, eviction (HV-MODEL-03) (W2)

**Files:** see File ownership (T11); ADR 0008 `## m4-t11 — catalog cache`.

**Spike origin (R-M4-40):** the bodies of `codec.rs` and `persist/**` are copied from `m4-spike-model` @ `d86313b` (versioned records, XXH3-64 row checksums, string interning, the corrupt-file rotation copied from `hacivat-exec/src/history/schema.rs`), including its 9 persist tests and the perf test.

**Interfaces:**
- Consumes: Task 1a's types; Task 3's arena; `hacivat_settings::paths::catalog_db(uuid, major)` (`crates/core/hacivat-settings/src/paths.rs:72`); `fsio::create_dir_all_private` (`fsio.rs:66`); `hacivat_secrets::SecretStore::{get, put}` (`store.rs:163, 186`) for `SecretSlot::CatalogKey` — **called only by `CatalogHub` (Task 15) on the blocking pool**, which passes `Encryption::Row(RowKey)`; `hacivat-meta` never calls the secret store (`MetaBelowIntrospect`).
- Produces:
  - **codec** (R-M4-26): envelope `"HVNS" major minor compression checksum body`; every record and union value (props, ids, extensions, watermark) length-prefixed; a newer minor's trailing fields and unknown tags are skipped and mapped to defaults (`Props::None`, `ObjectKind::Unknown`, `Watermark::None` → full introspection) and the namespace is marked `from_newer_minor`; an older minor reads missing fields as defaults (`migrate_minor` hook, no steps at 1.0); another major is rejected; XXH3-64 over the body; counts capped by the bytes remaining; per-namespace string interning; deterministic encoding.
  - **`CacheStore`** (alg/02 HV-MODEL-03): `open(path, encryption, writer_id)` (step 1: one SQLite file per profile and major, WAL, `synchronous=NORMAL`, header table, `quick_check` on error, corrupt → `*.corrupt-<secs>[-n]` with `-wal` moved first, and an empty store with `rebuilt`); `check_identity(&ServerFingerprint) -> IdentityCheck` (step 2; `Changed` → every namespace stale, rows kept); `load_namespace`/`load_all` (step 3: decrypt → decode → `migrate_minor` → `stale = true`; a bad row is deleted on load and reported in `LoadAll::dropped`); `save_namespace` (step 4: background, debounced 1 s per namespace, coalesced; `BEGIN IMMEDIATE`; compare-and-set on watermark and `writer_id`, step 5; refuses a `from_newer_minor` namespace and temp namespaces); sources content-addressed by `SourceRef` (XXH3-128, step 6); eviction (R-M4-25, step 7); `delete_profile_files(uuid)`; rows **uncompressed** in v1 (the compression byte is reserved); file mode 0600 (also `-wal`, `-shm`), directory 0700 on Unix.
  - **`persist::crypt::RowCipher`** (reusable, M7): XChaCha20-Poly1305, a fresh 24-byte nonce per row from the OS RNG, associated data `domain ‖ ad`; the catalog cache uses domain `"hv-catalog-v1"` with `ad = ns_key ‖ minor`, so a row cannot move between namespaces or stores.
  - **`fingerprint::subtree(ns, id) -> Fingerprint(u128)`** (M7, M8): XXH3-128 over the codec's canonical encoding of the subtree, excluding `StateMark`s, the `Statistics` facet and server ids.
  - `SecretStore` named lines to create the 32-byte key from the OS RNG on first use (`Retention::Forever`), never logged.

- [ ] **Step 1: Write the failing tests** (alg/02 HV-MODEL-03 tests 1–6, 8; edge cases 1–13; plus the spike's persist tests):
```rust
#[test] fn save_then_load_round_trips_a_namespace() {}
#[test] fn kill_during_commit_keeps_the_old_row() { /* fault hook aborts inside COMMIT; reopen → old row, generation unchanged */ }
#[test] fn older_minor_is_migrated_then_saved_as_current() {}
#[test] fn newer_minor_is_read_and_never_rewritten() {}
#[test] fn new_major_uses_a_new_file_and_leaves_the_old_untouched() {}
#[test] fn server_change_marks_all_stale_and_keeps_rows() {}
#[test] fn wrong_key_rebuilds_without_panic() {}
#[test] fn bit_flip_drops_only_that_row() {}
#[test] fn row_moved_to_another_ns_key_fails_authentication() {}
#[test] fn disk_full_warns_once_and_memory_snapshot_is_intact() {}
#[test] fn read_only_directory_runs_memory_only() {}
#[test] fn older_writer_never_clobbers_newer() { /* two stores on one file: the one with the older watermark loses the CAS */ }
#[test] fn temp_namespaces_are_never_persisted() {}
#[test] fn eviction_is_lru_and_never_evicts_the_current_database() {}
#[test] fn profile_deletion_deletes_its_cache_files() {}
#[cfg(unix)] #[test] fn file_mode_0600_and_directory_0700() {}
#[test] fn encrypted_file_contains_no_object_name_in_clear() { /* table "musteri_hesap_ozet" absent from the raw bytes */ }
#[test] fn password_never_in_cache_file() { /* secrets scan: profile password "hv_test_pw" absent */ }
#[test] fn corrupt_file_is_renamed_and_rebuilt() {}
#[test] fn decoding_arbitrary_bytes_never_panics() { /* proptest over random payloads; plus the fuzz target */ }
#[test] fn newer_minor_trailing_fields_are_skipped() {}
#[test] fn every_truncation_and_every_single_byte_corruption_with_recomputed_checksum_is_an_error_not_a_panic() {}   // spike robustness test, kept
#[test] fn rows_sealed_in_one_domain_fail_in_another() {}
#[test] fn row_cipher_debug_is_redacted() {}
#[test] fn tampered_ciphertext_is_an_error_not_a_panic() {}
#[test] fn fingerprint_ignores_state_marks_statistics_and_oids() {}
#[test] fn fingerprint_changes_on_column_type_change() {}
#[test] fn fingerprint_stable_across_encode_decode() {}
#[test] fn fingerprint_identical_under_tr_locale() {}
#[test] #[ignore = "perf"] fn load_160k_column_namespace_under_budget() { /* persist_perf.rs: synthetic 15,000 tables / 160,000 columns / 15,000 PKs / 5,000 FKs / 20,000 indexes; open + load + decode + publish p95 of 20 runs ≤ 200 ms plaintext on the CI runner (relative signal; the authoritative 300 ms end-to-end is Task 29); encrypted variant printed */ }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-meta --features test-support --test persist --test persist_faults --test persist_crypt --test fingerprint`. Expected: FAIL.
- [ ] **Step 3: Implement**, including the `catalog_row_decode` fuzz target body.
- [ ] **Step 4:** Run the tests, the perf line (report the numbers; if the 200 ms share is missed, report DONE_WITH_CONCERNS with the profile — the controller rules before W3), the fuzz target for 10 minutes. Expected: PASS.
- [ ] **Step 5: Commit** `feat(meta): per-profile catalog cache with versioned rows, optional row encryption and eviction`.

**Review:** security-sensitive, **two independent opus reviews**. Spike code reviewed as new code. Probes: nonce reuse; a key or plaintext name in logs or `Debug`; a panic on crafted bytes; a corrupt count that allocates hugely; a newer-minor row overwritten; a secret-store call inside `hacivat-meta`; a rename of a corrupt file that follows a symlink.

### Task 12: Schema scope and object filter grammars with server-side pushdown (HV-INTRO-02, -03) (W2)

**Files:** see File ownership (T12); ADR 0008 `## m4-t12 — scope and filters`.

**Interfaces:**
- Consumes: Task 3's `match_key`; Task 4's bind slots (`PackCtx.params`).
- Produces: `parse_scope(&str, DialectId) -> Result<ScopePattern, ScopeError { column: u32 /* Unicode scalars */, kind }>`, `ScopePattern::select(&[NsInfo], &CurrentCtx) -> BTreeSet<NsKey>` (union of positive clauses minus negatives; implicit `@:@` kept for resolution; single-level dialects), `to_text` (round trip through the AST); `parse_filter(&str, DialectId, FilterRole) -> Result<CompiledFilter, FilterError>`, `CompiledFilter::matches(kind, &Name) -> bool`, `CompiledFilter::pushdown(kind, DialectId, &IdentCtx) -> Option<SqlPredicate { sql_fragment: &'static str, binds: Vec<BindValue> }>` (positive globs only, `*`→`%`, `?`→`_`, literal `%`, `_`, `\` escaped, `ESCAPE '\'`; never negatives, never regexes; never case-insensitive items where the catalog column compares case-sensitively; > 200 positive items → no pushdown; alg/02 HV-INTRO-03 per-DBMS eligibility rows for PG, MySQL/MariaDB, SQLite); own glob → anchored-regex translation (case option controlled by HV-INTRO-14, never locale folding).

- [ ] **Step 1: Write the failing tests** (alg/02 HV-INTRO-02 test cases 1–8, 10 and edge cases 1–13; HV-INTRO-03 test cases 1–8 and edge cases 1–13, MSSQL rows excluded per R-M4-4):
```rust
#[test] fn all_all_selects_every_pair() {}
#[test] fn current_current_selects_only_current() {}
#[test] fn union_of_three_clauses() { /* "*:dbo|@:@|db1:s1,s2,s3" */ }
#[test] fn regex_is_full_match() { /* "{t.*}:@": t1 yes, at1 no */ }
#[test] fn negative_clause_removes_tmp_schemas() { /* "*:*|!*:{tmp_.*}" */ }
#[test] fn unclosed_brace_errors_at_column_3() { assert_eq!(parse_scope("db{1:x", DialectId::Postgres).unwrap_err().column, 3); }
#[test] fn quoted_names_with_separators_round_trip() { /* r#""a:b":s"# → database "a:b", schema s */ }
#[test] fn mysql_regex_tenants() { /* "{tenant_\d+}" over tenant_1, tenant_x → tenant_1 */ }
#[test] fn pg_quoted_mixed_case_schema_differs_from_bare() {}
#[test] fn single_level_dialect_ignores_db_term_with_warning() {}
#[test] fn negative_on_current_schema_keeps_it_resolvable() {}
proptest! { #[test] fn to_text_parse_round_trip(p in scope_pattern_strategy()) {} }
#[test] fn typed_filter_semantics() { /* "table:-OLD_*||routine:DO*||view:V_*" */ }
#[test] fn untyped_filter_applies_to_every_kind() { /* "*_tmp" */ }
#[test] fn underscore_and_percent_are_literal_in_globs() { /* my_table matches only my_table */ }
#[test] fn pg_pushdown_ord_star_binds_ord_percent() { let p = filter("table:ord*").pushdown(ObjectKind::Table, DialectId::Postgres, &ctx()).unwrap(); assert!(p.sql_fragment.contains("LIKE") && p.sql_fragment.contains("ESCAPE")); assert_eq!(p.binds, [text("ord%")]); }
#[test] fn pg_pushdown_escapes_underscore() { assert_eq!(filter("table:a_b").pushdown(ObjectKind::Table, DialectId::Postgres, &ctx()).unwrap().binds, [text(r"a\_b")]); }
#[test] fn negative_and_regex_items_are_never_pushed() {}
#[test] fn case_insensitive_items_never_pushed_on_pg() {}
#[test] fn mysql_lctn1_pushdown_allowed_for_table_names_only() {}
#[test] fn sqlite_non_ascii_pattern_is_not_pushed() {}
#[test] fn over_200_items_no_pushdown() {}
#[test] fn hostile_pattern_is_a_bind_never_sql() { /* "x'; drop table t; --" appears only in binds */ }
#[test] fn view_filter_hides_but_scope_filter_unloads() { /* against a fake ModelView: hide audit_* → the view still answers Found; scope-exclude audit_* → Unknown(OutOfScope). The end-to-end check with Task 9's view is Task 24's hide_like_this_adds_a_view_filter_and_lookup_still_finds_it */ }
#[test] fn unknown_kind_token_is_an_error() {}
#[test] #[ignore = "perf"] fn filter_matches_160k_names_under_50ms() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-introspect --test scope --test filter --test scope_props`. Expected: FAIL.
- [ ] **Step 3: Implement**, including the `scope_pattern` fuzz target.
- [ ] **Step 4:** Run the tests under `LC_ALL=tr_TR.UTF-8` too; the perf line. Expected: PASS.
- [ ] **Step 5: Commit** `feat(introspect): schema scope and object filter grammars with bind-only pushdown`.

**Review:** security-sensitive. Probes: user text in a SQL fragment; a case-insensitive pushdown on PG (`ILIKE`, `lower`); a regex compiled per object; column offsets in bytes.

### Task 13: Composite object assembly (HV-INTRO-13) (W2)

**Files:** see File ownership (T13); ADR 0008 `## m4-t13 — assembly`.

**Interfaces:**
- Produces: `assemble_keys(rows: KeyRows, cols: &ColumnIndex) -> Vec<KeyObj>` per dialect adapter (`assemble::{pg, mysql, sqlite}`): one row per object with ordered `(ordinal, name)` columns resolved by ordinal then name; unresolved → `BROKEN` flag (kept); FK `ref_table: Ref`, `ref_columns` by position, `ref_constraint` (PG `conindid`); `backing_index` link and **`IndexProps.implicit_for` with `ObjFlags::IMPLICIT`** (PG backing indexes of PK/UNIQUE/EXCLUDE through `conindid`; SQLite `sqlite_autoindex_*` from `index_list.origin` `pk`/`u`; MySQL FK-created index per Task 17's probe); expression key parts keep server text (PG `pg_get_indexdef(oid, k, true)` only for expression slots; MySQL `EXPRESSION` with `COLUMN_NAME` NULL; SQLite `cid = -2` with the text from the index's SQL); INCLUDE columns; descending flags (MariaDB 10.6 stores `DESC` parts ascending — kept as the server reports them, F9); `nulls_not_distinct`; partial predicates; deferrable / NOT VALID / `enforced`; PG exclusion constraints with operators; SQLite unnamed constraints get a display name `<table>_pk` with `SERVER_GENERATED_NAME` (never emitted as a real name in DDL) and FKs with no name; deterministic order per table by name; **MySQL key parts arrive one row each and are grouped client-side** by `(table, index)` ordered by `SEQ_IN_INDEX` (no `GROUP_CONCAT`, so no `group_concat_max_len` and no warning 1260; MySQL/SQLite spike difference 5); MySQL CHECK names are unique per schema and joined by name, MariaDB's per table with `CHECK_CONSTRAINTS.TABLE_NAME` (F9).

- [ ] **Step 1: Write the failing tests** (alg/02 HV-INTRO-13 test cases 1–5, 7 as row fixtures; Docker variants run in Tasks 16–18):
```rust
#[test] fn key_columns_in_key_order() { /* PG PRIMARY KEY (b, a) → [b, a] */ }
#[test] fn mysql_index_with_a_descending_part() { /* idx_ab: [a asc, b desc] */ }
#[test] fn expression_include_and_partial_index() { /* lower(name), include [id], predicate "active" */ }
#[test] fn fk_to_an_out_of_scope_table_is_kept_unresolved() {}
#[test] fn sqlite_autoindex_unique_has_server_generated_name() { /* sqlite_autoindex_t_1 */ }
#[test] fn sqlite_unnamed_pk_gets_a_display_name_only() {}
#[test] fn fk_column_count_mismatch_is_broken_and_kept() {}
#[test] fn pk_backed_by_an_index_of_another_name_links_it() {}
#[test] fn mysql_key_parts_grouped_client_side_in_seq_order() {}
#[test] fn exclusion_constraint_keeps_operators() {}
#[test] fn output_order_is_deterministic() {}
#[test] fn six_hundred_indexed_columns_assemble_without_truncation() {}
#[test] fn pg_pk_backing_index_is_implicit_for_the_pk() {}
#[test] fn sqlite_autoindex_is_implicit() {}
#[test] fn nulls_not_distinct_carried_to_key_and_index() {}
#[test] fn mysql_functional_key_part_keeps_its_expression() {}
#[test] fn mariadb_check_joined_by_table_and_name_mysql_by_name() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-introspect --test assemble`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Run the tests. Expected: PASS.
- [ ] **Step 5: Commit** `feat(introspect): composite assembly of keys, foreign keys and indexes`.

**Review:** opus. Probes: a dropped broken object; key order taken from name order; a synthesized SQLite name that could reach DDL.

### Task 14: Explorer tree adapter over snapshots (W2, gpui)

**Files:** see File ownership (T14); ADR 0007 `## m4-t14 — explorer tree`.

**Interfaces:**
- Consumes: Task 8's widget; Task 3's model and fixture builder; Task 1a's `CatalogSnapshot` fields (data only; Task 9's publish is not needed to render a snapshot); `hacivat_l10n::PreparedQuery` and `search_opts` (`commands/palette.rs:141`, used through the list's `filter_opts`, `connections/explorer.rs:118`) for the filter (D-074); the M2a list (`connections/explorer.rs`), whose groups, rows, keyboard and debug selectors stay for profile roots.
- Produces: `ExplorerModel` (implements `TreeModel`) built from `(profiles, Option<Arc<CatalogSnapshot>> per profile, expansion, filter)`: profile → contributed nodes (R-M4-19; lazy children, joining the filter through `filter_text`) → databases (PG; the connected one first, others greyed when out of scope) → schemas (MySQL/MariaDB label "database" through `model-kind-schema-mysql`; SQLite `main`, `temp`, attached) → kind groups with counts (`Tables 1.204` in `tr`, preformatted per L3) → objects (icons per kind, type text at text-2 for columns, flags) → child groups (Columns, Keys, Foreign keys, Indexes, Checks, Triggers) → leaves; a collapsed "Built-in" group per database for system namespaces (badge from Task 26's flag; empty until then); **`TreeKey = xxh3_128("hv-object\0" ‖ profile uuid ‖ kind code ‖ (database ‖ server id) or path)`** (two profiles never share a key; a renamed PG object keeps its key); display order = ICU4X collation of the UI locale for names (display only), kind groups in a fixed order; names verbatim with bidi isolation, `INVISIBLE_CHARS` badge, `Unknown` rows show the raw kind from `Props::Generic`; the tree's type-ahead fold set per R-M4-50 (`Tolerant` for a Turkish UI language or region, following the filter's Turkish-tolerant toggle); the filter field: latest-wins background matching over `NamespaceSnapshot::iter_named` (> 2,000 names off the UI thread), matches reveal their ancestors, no match → the empty state.

- [ ] **Step 1: Write the failing tests:**
```rust
#[gpui::test] fn pg_tree_shape_database_schema_groups_objects(cx) {}
#[gpui::test] fn mysql_schemas_are_labelled_databases(cx) {}
#[gpui::test] fn sqlite_shows_main_temp_and_attached(cx) {}
#[gpui::test] fn kind_group_counts_follow_the_format_locale(cx) { /* region tr: "1.204"; en: "1,204" (L3, L15(d)) */ }
#[gpui::test] fn names_are_verbatim_never_case_mapped(cx) { /* "İSTANBUL_ŞUBE", "ıspanak" shown as stored */ }
#[gpui::test] fn expansion_survives_publish_and_rename_by_oid(cx) {}
#[gpui::test] fn mysql_rename_hint_keeps_expansion(cx) {}
#[gpui::test] fn filter_is_turkish_tolerant_for_tr_only(cx) { /* D-074: "istanbul" finds "İstanbul" in tr; toggle off → exact */ }
#[gpui::test] fn filter_reveals_ancestors_of_matches(cx) {}
#[gpui::test] fn built_in_group_is_collapsed_and_last(cx) {}
#[gpui::test] fn invisible_chars_badge(cx) {}
#[gpui::test] fn contributor_nodes_sit_above_schemas(cx) { /* fake contributor (M4b seam) */ }
#[gpui::test] fn unknown_kind_row_shows_its_raw_kind(cx) {}
#[gpui::test] fn profile_rows_keep_their_m2a_content_and_bindings(cx) {}
#[gpui::test] fn trust_signals_stay_drawn_on_profile_roots(cx) { /* swatch, lock, production mark (M3a trust check) */ }
#[gpui::test] fn tree_key_differs_across_profiles_for_the_same_oid(cx) {}
#[gpui::test] fn typeahead_fold_follows_d074(cx) { /* tr UI → Tolerant; en UI → Locale; toggling the filter's Turkish-tolerant option switches it */ }
#[gpui::test] fn contributed_node_with_lazy_children_expands_and_filters(cx) {}
#[test] #[ignore = "perf"] fn filter_160k_names_off_ui_thread() { /* explorer_perf.rs: UI share per keystroke ≤ 1 ms; result ≤ 100 ms */ }
#[test] #[ignore = "perf"] fn build_rows_for_15k_tables_under_16ms() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-app --features scene-check,pseudo --test explorer_tree`. Expected: FAIL.
- [ ] **Step 3: Implement.** No core I/O; the model reads the snapshot synchronously.
- [ ] **Step 4:** Run the tests, the perf lines, the app routing test; render the explorer shots and diff against `baseline-m3b` plus the m3c-listed changes until `baseline-m3c` exists (Global Constraints "Shots harness"): **list every changed PNG** (expected: every shot that shows the explorer, R-M4-18) in the report, including any of the 36 CI-compared M1 states (then their `superseded.txt` line).
- [ ] **Step 5: Commit** `feat(app): explorer tree over catalog snapshots with kind groups, counts and filter`.

**Review:** opus (gpui). Probes: a name case-mapped or translated; a count joined to a unit in code; filtering on the UI thread; expansion keyed by display text; a trust signal clipped.

## W3 — Engine, introspectors, producers, scope editor

### Task 15: Engine, scheduler, levels, tickets and the hub (HV-INTRO-01) (W3)

**Files:** see File ownership (T15); ADR 0008 `## m4-t15 — engine and hub`.

**Interfaces:**
- Consumes: Task 4 (packs), Task 5 (`MetaConn`), Task 9 (`Publisher`), Task 10 (`merge`), Task 11 (`CacheStore`, optional), Task 12 (scope, filters), Task 1c's `Introspector` trait (Tasks 16–18 fill it in parallel; this task tests with `testing::FakeIntrospector`, whose body it writes).
- Produces: `Engine::start(EngineConfig { profile, dbms, meta: Arc<dyn MetaSource /* SessionManager adapter */>, introspector, catalog_publisher, cache: Option<Arc<CacheStore>>, levels: LevelPolicy, scope: ScopePattern, filters, validate_every: Option<Duration> }) -> IntroHandle` running on the `CoreRuntime`; exactly one scheduler per profile; tasks `Full`, `Incremental`, `Fragment`, `Brief`, `Elevate`, `CheckState`, `ScopeRefresh` (alg/02 HV-INTRO-01 decision); priorities user-visible > completion-triggered > background validation > periodic; coalescing on `(ns, mode)`; single-flight futures keyed by `(ns, facet, level)`; **bulk policy** (a children request runs a per-object fragment; ≥ 3 distinct objects of one namespace within 500 ms, or a namespace under 2,000 tables, upgrades to per-kind bulk queries); **atomic publish** per namespace (nothing published on cancel or error; `Failed` only without previous state); on-connect algorithm steps 1–3 (cached namespaces published stale first, `ScopeRefresh`, current namespace `Incremental` if cached else `Brief` → `Full` at the auto level, others on first expansion); auto level (R-M4-21) and per-container overrides inherited by children; back-pressure (a query timing out twice for a namespace lowers its auto level by one and raises `intro-hint-level-lowered`); periodic validation (skipped while the user's session is in a transaction on a single-connection profile); progress via `watch` with the closed `Phase` enum and determinate counts; `confirm_pending(ns, accept)` applies or drops a guarded merge; persistence after each successful publish (Task 11's debounced save).
- **Also produces:**
  - `RefreshTicket` bodies: `refresh`, `fragment`, `elevate`, `check_state` (and `load_facet`, whose scheduling lands in Task 15b; until then its ticket resolves to `Err(NotImplemented)`) resolve with the generation that contains the result; dropping a ticket does not cancel shared work.
  - Fragment publishes for one namespace are coalesced within 50 ms (the model spike measured an 8 ms subtree upsert in a 10k-table namespace, open question 3).
  - A pass that fails with `IntroError::CatalogRace` (PG `XX000` "cache lookup failed" from a concurrent drop, PG spike finding 8) is retried once, then fails the namespace without publishing; the retry raises `intro-catalog-race-retried` in the log only.
  - **`hub::CatalogHub`** (R-M4-31): `new(sessions: Arc<SessionManager>, settings, secrets, system: Arc<SystemCatalogs>)`, `get(profile) -> Option<ProfileCatalog>`, `ensure(profile) -> BoxFuture<Result<ProfileCatalog, HubError>>` (opens the cache; fetches `CatalogKey` on the blocking pool when `catalog_sensitive`; keyring unavailable → memory-only with the notice; starts the engine), `row_key(profile)`, `shutdown(profile)`; one engine, catalog, cache and key per profile; usable without the UI (M8's broker, M7's apply). `ensure` never runs at app start for a profile the UI did not ask for (R-M4-15).

- [ ] **Step 1: Write the failing tests** (alg/02 HV-INTRO-01 test cases 1–4; edge cases 1–14 with fakes; the Docker cases 5–6, 8 run in Tasks 16, 17 and 28):
```rust
#[tokio::test] async fn bulk_upgrade_after_third_request_total_queries_le_4() { /* 50,000-table fake namespace; children(t1), (t2), (t3) within 200 ms → ≤ 4 catalog queries */ }
#[tokio::test] async fn small_namespace_goes_bulk_immediately() { /* 100 tables */ }
#[tokio::test] async fn hundred_concurrent_children_calls_one_query() {}
#[tokio::test] async fn cancel_between_batches_publishes_nothing() {}
#[tokio::test] async fn empty_namespace_completes_with_zero_objects() {}
#[tokio::test] async fn child_row_with_missing_parent_schedules_a_follow_up() {}
#[tokio::test] async fn connection_lost_restarts_the_task_once() {}
#[tokio::test] async fn permission_denied_kind_fails_only_that_kind() {}
#[tokio::test(start_paused = true)] async fn timeout_twice_lowers_the_level_and_hints() {}
#[tokio::test] async fn scope_change_discards_out_of_scope_results() {}
#[tokio::test] async fn single_connection_profile_waits_for_user_idle() {}
#[tokio::test] async fn two_windows_share_one_future() {}
#[tokio::test] async fn elevation_waits_for_the_running_namespace_task() {}
#[test] fn auto_level_thresholds_d038() { assert_eq!(auto_level(Current, 2_000), Level::Sources); assert_eq!(auto_level(Current, 2_001), Level::Structure); assert_eq!(auto_level(Current, 20_001), Level::Names); assert_eq!(auto_level(Other, 5_000), Level::Structure); assert_eq!(auto_level(Other, 5_001), Level::Names); assert_eq!(auto_level(System, 10), Level::Names); }
#[test] fn level_override_is_inherited_by_children() {}
#[tokio::test] async fn on_connect_with_cache_publishes_stale_then_runs_incremental_not_full() {}
#[tokio::test] async fn on_connect_without_cache_paints_brief_names_first() {}
#[tokio::test] async fn pending_confirmation_changes_nothing_until_confirmed() {}
#[tokio::test(start_paused = true)] async fn periodic_validation_skips_during_user_tx_on_single_connection() {}
#[test] fn progress_phases_are_a_closed_enum_with_counts() {}
proptest! { #[test] fn random_request_interleavings_never_publish_partial_state(ops in engine_ops()) {} }
#[test] #[ignore = "perf"] fn scheduler_overhead_per_task_under_1ms() {}
#[tokio::test] async fn ticket_resolves_with_the_publishing_generation() {}
#[tokio::test] async fn dropped_ticket_does_not_cancel_the_shared_task() {}
#[tokio::test(start_paused = true)] async fn fragment_burst_publishes_once_per_window() { /* 20 fragments for one namespace within 50 ms → one publish */ }
#[tokio::test] async fn catalog_race_is_retried_once_then_fails_without_publishing() {}
// hub.rs
#[tokio::test] async fn ensure_starts_one_engine_per_profile() {}
#[tokio::test] async fn hub_without_ui_serves_a_cached_catalog() {}
#[tokio::test] async fn sensitive_profile_without_keyring_is_memory_only() {}
#[tokio::test] async fn get_never_fetches_a_key() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-introspect --test engine --test engine_props --test hub`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Run the tests and the perf line. Expected: PASS.
- [ ] **Step 5: Commit** `feat(introspect): single-writer introspection engine with levels, bulk policy and atomic publish`.

**Review:** opus. Probes: two writers for one profile; a partial namespace published; per-object queries in a loop; work on a GPUI thread; a task that never ends after the profile disconnects; a keyring call from `get` or from anything the app runs at start.

### Task 16: PostgreSQL introspector — full, incremental, state, system sections (HV-INTRO-05; HV-INTRO-13 and -15 PG parts) (W3)

**Files:** see File ownership (T16); ADR 0008 `## m4-t16 — PostgreSQL introspection`.

**Spike origin (R-M4-40):** `m4-spike-pg-intro` @ `5a5ab08` (`src/pg/{model,sql,decode,text,watermark,introspect}.rs`, `tests/fixtures/pg_kinds.sql`): the decoders, the watermark and full-or-incremental decision, and the passes are copied into `pg/**`; its 18 SQL sections become pack sections in `packs/pg/*.hvq.sql` with integer lists turned into chunked `IN (…)` binds (R-M4-3) and the text-protocol execution replaced by `MetaQuery` on `MetaConn`. The spike's reference merge stays a test oracle; the model's merge is Task 10's.

**Interfaces:**
- Consumes: Tasks 4, 5, 10, 13; `pg_current_snapshot()` (R-M4-8).
- Produces: `pg::PgIntrospector` implementing every `Introspector` method; `facet` is wired to `pg::facets::read` and the `NsKind::Server` namespace to `pg::facets::server_namespace`, both stubs answering `NotImplemented` until Task 16b. Every catalog is written `pg_catalog.<name>`, objects are selected **by oid only**, no name reaches SQL text, and every pass runs inside Task 5's snapshot with the pinned `search_path` (R-M4-46).
  - **`packs/pg/10-core.hvq.sql`** (written from the PostgreSQL documentation; no copied third-party SQL), sections as the spike measured them: `Identity` (the first statement of every pass: `server_version_num`, `pg_is_in_recovery()`, `extract(epoch FROM pg_postmaster_start_time())`, the database oid, `pg_control_system().system_identifier` and `pg_control_checkpoint().timeline_id` each behind `has_function_privilege`, `pg_snapshot_xmin/xmax(pg_current_snapshot())`, whether the namespace exists; also fills `ServerFacts`: encoding, `standard_conforming_strings`, `in_recovery`, default collation); `ListNamespaces`, `RetrieveNamespace` (`pg_toast*`, `pg_temp_*` filtered on the client); **one tagged `ListExistent`** (`UNION ALL` of tagged oids: relations, sequences, children — indexes, table and domain constraints, user triggers —, routines, types); **`ListExistentComments`** (the `(class tag, objoid, objsubid)` key of every `pg_description` row of the namespace's objects: `COMMENT … IS NULL` deletes the row, so only an existence check sees it, finding 3); **one `ChangedParents`** returning `(facet, owner oid)` for young rows of the namespace in `pg_class`, `pg_rewrite`, `pg_inherits`, `pg_partitioned_table`, `pg_attribute`, `pg_attrdef`, `pg_index` and the index's `pg_class` row, `pg_constraint`, `pg_trigger`, `pg_description` (sent to the owning facet), `pg_sequence`, `pg_depend` (sequence ownership), `pg_proc`, `pg_type`, `pg_enum`, domain `pg_constraint`, `pg_range`, `pg_namespace`; **`ChangedDependents`** (young `pg_class`, `pg_attribute`, `pg_type`, `pg_proc` rows **of the whole database** joined to `pg_depend` with `deptype NOT IN ('e','x','p')`, mapped to the namespace's dependent objects' facets, finding 4); `RetrieveRelations` (`relkind IN (r,p,v,m,f,c)`, owner, persistence, comment, `pg_inherits` parents in `inhseqno` order, `pg_get_expr(relpartbound)`, `pg_get_partkeydef`, `pg_get_viewdef` at `Sources`, `relispopulated`, `reloptions`, `relrowsecurity`, and `reltuples` → `StatsFacet.row_estimate`, `-1` → `None`); `RetrieveColumns` (`attnum > 0 AND NOT attisdropped`; `format_type`, the type's schema, name and `typtype`, array element by `typlen = -1 AND typelem <> 0`, domain base, `attndims`, NOT NULL, `pg_get_expr(adbin)`, identity, generated, non-default collation, `attinhcount`, comment; `ordinal = attnum`); `RetrieveIndexes` (method, flags, `indnullsnotdistinct` 15+, `indnkeyatts`, `indkey`/`indoption`, key columns, `pg_get_indexdef(oid, k, true)` only for expression slots, predicate, full definition at `Sources`, comment); `RetrieveConstraints` (kind, deferrable/deferred/validated, `conenforced` 18+, local, no-inherit, `conkey`, `conindid`, FK target oid/schema/name, `confkey`, actions, match type, `pg_get_constraintdef(oid, true)`, comment; PG 18 column `NOT NULL` constraints → `ColumnProps.not_null_constraint`); `RetrieveTriggers` (`NOT tgisinternal`; `tgtype` bits decoded on the client, `tgenabled`, function schema and name, `pg_get_triggerdef`, constraint-trigger flag, comment); `RetrieveSequences` (`OWNED BY` from `pg_depend` `a`/`i`); `RetrieveRoutines` (`pg_get_functiondef` only for `f` and `p`; extension membership); `RetrieveTypes` (`typtype IN (b,c,d,e,r)`, array types left out by a `NOT EXISTS` anti-join, composites only for `relkind = 'c'`, enum labels in `enumsortorder`, range subtype), `RetrieveDomainConstraints` (17+ domain `NOT NULL` as `contype = 'n'`); `Comments`; `Canary` (`pg_current_xact_id_if_assigned() IS NULL`, run just before `COMMIT`).
  - **Change rule (R-M4-8, R-M4-48):** young = `age(x.xmin) BETWEEN 0 AND age($W::xid)` (the bind is `W mod 2^32`); the client keeps only the 64-bit span check. **Facet cascade:** a relation has the facets `rel`, `columns`, `indexes`, `constraints`, `triggers`, `comments`; a `columns` change also sets `rel`, and a `rel` change also sets `indexes`, `constraints` and `triggers` (finding 5: a constraint's backing index is two hops from its table). A removed comment key flags its owner's `comments` facet.
  - **Full instead of incremental** when there is no previous watermark, the identity changed (postmaster start time, database oid, system identifier, timeline), the major version changed, the server is in recovery (R-M4-9), the watermark is ahead of the server's next XID (a restore or `pg_resetwal`), the span `snap_xmax − W ≥ 2^31 − 2^20`, the requested level is above the cached one, the changed parents exceed 25 % of the namespace's relations, or a young `pg_namespace` row belongs to a schema whose types or functions in-scope objects reference (schema rename, PG spike §7 Q3); full-reason codes `no-watermark`, `restart`, `identity`, `version`, `standby`, `wraparound`, `mass-change`, `schema-rename`.
  - **Version gates** (R-M4-47): `indnullsnotdistinct` 15+; domain `NOT NULL` in `pg_constraint` 17+; column `NOT NULL` constraints, `conenforced`, `attgenerated = 'v'` 18+. PG 13 needs none (its missing `attcompression`, `prosqlbody`, `typsubscript` are never read).
  - **`packs/pg/30-state.hvq.sql`:** `StateOf<Kind>` sections (`joinable`; type schema and name columns, never `format_type` text, R-M4-46): per object the maximum xmin of its own rows and its `pg_description` rows, its comment-key count, the xmins of the rows one `pg_depend` hop away, and for a relation the rows of its indexes, constraints and triggers (R-M4-48, so `check_state` sees a renamed dependency, PG spike §7 Q2); `stamps` returns them raw; Task 22 compares.
  - **`packs/pg/40-system.hvq.sql`** (R-M4-44, `Mode::System`): `pg_catalog` and `information_schema` relations and columns (OID < 16384, database `postgres`), routines (args, identity args when different, result, kind, volatility, set-returning, description), operators (kind, left, right, result, function, description), types (SQL name, kind, category, array element, description; relation row types left out), `ListSettings` (GUC names with type and context, never values; names with a dot excluded) — also used live for versions without a snapshot.
  - `testing::big_catalog::{ensure_pg(target, tables: 15_000, columns: 160_000), drop_pg(target)}`: the idempotent fixture database `hv_m4_big` on `pg17`, created and dropped in **batches of 250 tables per server-side `DO` block** (one transaction runs out of lock-table slots, PG spike §5); reused by Tasks 28–29; dropped at the gate (C-9).

- [ ] **Step 1: Write the failing tests:**
```rust
// pure (pg_rows.rs)
#[test] fn span_check_is_64_bit_and_forces_full_at_2_31_minus_2_20() { /* W = 738 with next XID 2^31 + 2000 → Full(wraparound); the limit edge both ways (spike unit tests) */ }
#[test] fn watermark_ahead_of_next_xid_forces_full() {}
#[test] fn change_predicate_is_between_zero_and_age_of_w() { /* every rendered ChangedParents/ChangedDependents section contains "BETWEEN 0 AND age(" and no "<=" on age(); no client-side txage constant exists */ }
#[test] fn identity_or_version_change_forces_full() {}
#[test] fn recovery_forces_full_with_reason_standby() {}
#[test] fn negative_age_rows_are_not_changed() {}
#[test] fn columns_change_sets_rel_and_rel_sets_indexes_constraints_triggers() {}
#[test] fn changed_parents_over_25_percent_runs_full_with_reason_mass_change() {}
#[test] fn no_query_assigns_an_xid() { /* no txid_current / pg_current_xact_id( in any rendered section */ }
// [docker: pg13, pg14, pg15, pg16, pg17 (pr), pg18] (docker_pg_intro.rs; schema "hvm4_<test>" per test, dropped in a guard)
#[tokio::test] #[ignore = "docker"] async fn full_pass_reads_every_kind() { /* the spike's pg_kinds.sql fixture: exact values for every kind (identity, text[], int[][] with attndims 2, enum, domain, composite, numeric(10,2), stored generated, COLLATE "C", timestamptz(3); PK, UK in key order with backing index, FK DEFERRABLE INITIALLY DEFERRED, CHECK, EXCLUDE; expression index with INCLUDE and predicate; DESC NULLS LAST; partitioned table and partition; view, matview WITH NO DATA, unlogged table; sequences explicit/serial/identity; overloads, SETOF, procedure, aggregate; enum/domain/composite/range types; comments on every commentable kind) */ }
#[tokio::test] #[ignore = "docker"] async fn create_index_reports_indexes_changed_not_table_props() {}   // alg/02 HV-INTRO-05 test 1
#[tokio::test] #[ignore = "docker"] async fn add_column_rereads_the_table() {}                          // test 2
#[tokio::test] #[ignore = "docker"] async fn comment_only_updates_the_comment() {}                     // test 3: query log shows no column retrieve for x
#[tokio::test] #[ignore = "docker"] async fn second_trigger_is_created() {}                            // test 4
#[tokio::test] #[ignore = "docker"] async fn drop_index_removed_by_existence() {}                      // test 5
#[tokio::test] #[ignore = "docker"] async fn drop_column_removed() {}                                  // test 6
#[tokio::test] #[ignore = "docker"] async fn alter_function_volatility_updated() {}                    // test 7
#[tokio::test] #[ignore = "docker"] async fn uncommitted_ddl_appears_on_next_pass() {}                 // test 8
#[tokio::test] #[ignore = "docker"] async fn set_schema_is_one_move_when_both_in_scope() {}            // test 12
#[tokio::test] #[ignore = "docker"] async fn case_twins_users_and_quoted_users_are_two_tables() {}     // HV-MODEL-01 test 6
#[tokio::test] #[ignore = "docker"] async fn key_order_and_expression_include_partial_index() {}      // HV-INTRO-13 tests 2, 3
#[tokio::test] #[ignore = "docker"] async fn check_state_changed_after_comment_and_dropped_after_recreate() {}   // HV-INTRO-15 tests 1, 2
#[tokio::test] #[ignore = "docker"] async fn validation_with_unchanged_server_runs_state_queries_only() {}       // HV-INTRO-15 test 5
#[tokio::test] #[ignore = "docker"] async fn every_pg_section_prepares_and_returns_its_declared_columns() {}     // HV-INTRO-04 test 7, every reachable variant
#[tokio::test] #[ignore = "docker"] async fn comment_removal_is_seen_by_existence() { /* COMMENT ON TABLE x IS NULL → the comment is gone after one incremental pass; only Comments ran */ }
#[tokio::test] #[ignore = "docker"] async fn rename_of_a_referenced_object_rereads_dependent_text() { /* rename a column named by a check, a generated column and a view; rename a type used by a column; rename an FK's target table; rename a trigger function → each dependent's deparsed text equals a fresh full pass */ }
#[tokio::test] #[ignore = "docker"] async fn rename_table_rereads_backing_index_text() { /* finding 5: parent → parent2; parent_pkey's definition names parent2 */ }
#[tokio::test] #[ignore = "docker"] async fn xid_burst_between_probe_and_change_query_misses_nothing() { /* a test-support hook pauses after Identity; another session commits a CREATE TABLE, then assigns 200 XIDs; the pass sees the table (finding 6) */ }
#[tokio::test] #[ignore = "docker"] async fn version_specific_not_null_constraints() { /* domain NOT NULL rows: 0 on 13–16, 1 on 17–18; column NOT NULL constraints: ≥ 2 on 18, 0 before (spike §3) */ }
#[tokio::test] #[ignore = "docker"] async fn pg_ordinal_is_attnum_with_gaps_after_drop_column() {}
#[tokio::test] #[ignore = "docker"] async fn nulls_not_distinct_read_on_15_plus_false_before() {}
#[tokio::test] #[ignore = "docker"] async fn pg_pk_backing_index_is_implicit_for_the_pk() {}
#[tokio::test] #[ignore = "docker"] async fn pg_row_estimate_minus_one_is_none() {}
#[tokio::test] #[ignore = "docker"] async fn stamp_moves_on_comment_removal_second_trigger_and_dependency_rename() {}
#[tokio::test] #[ignore = "docker"] async fn server_facts_filled_per_engine() {}
#[tokio::test] #[ignore = "docker"] async fn system_mode_reads_only_oids_below_16384_with_pinned_path() {}
// [docker: pg17] (full set only; hv_m4_big)
#[tokio::test] #[ignore = "docker"] async fn foreign_watermarks_force_a_full_pass() { /* no watermark, another postmaster start time, another database oid, a watermark ahead of next XID → Full with the right reason; result equals the first full pass (spike test 4) */ }
#[tokio::test] #[ignore = "docker"] async fn no_catalog_query_seq_scans_user_sized_structures_without_a_namespace_predicate() {}   // EXPLAIN on hv_m4_big (≥ 10k tables)
#[tokio::test] #[ignore = "docker"] async fn twenty_thousand_and_one_tables_auto_level_names_and_source_fragment_only() {}       // HV-INTRO-01 test 5 (on hv_m4_big + 5,001 extra in a scratch schema, or a 20,001-table scratch schema; report which)
#[tokio::test] #[ignore = "docker"] async fn drop_big_fixture_when_asked() { /* only with HACIVAT_M4_CLEANUP=1: drop_pg in batches of 250; used by C-9 */ }
```
- [ ] **Step 2:** Run the pure tests; `cargo test -p hacivat-introspect --no-run`. Expected: FAIL.
- [ ] **Step 3: Implement** the packs and the introspector; names only as binds; canonical catalog case (Task 4's lint passes).
- [ ] **Step 4:** Run the pure tests; `<main>/target/docker/run-docker-tests.sh --set full -p hacivat-introspect --test docker_pg_intro`. Expected: PASS; report the cold `Names`/`Structure` and no-change incremental times on `hv_m4_big` next to the spike's (Structure 1.27–1.71 s optimized, incremental 120 ms; debug 4.5–5.7 s) and the PR-set wall time.
- [ ] **Step 5: Commit** `feat(introspect): PostgreSQL introspection with exact xmin-age incremental refresh, dependency hops and state checks`.

**Review:** security-sensitive. Spike code reviewed as new code. Probes: `<=` on `age()` or a client-side `txage`; `txid_current()` or any XID-assigning call; a name spliced into SQL; a catalog not written `pg_catalog.<name>`; a missed existence query for one kind or for comment keys; `lower()`/`ILIKE` on `relname`; a dependency hop limited to the namespace (it must cover the database); the fixture built in one transaction.

### Task 17: MySQL and MariaDB introspector (HV-INTRO-07; HV-INTRO-13 and -15 MySQL parts) (W3)

**Files:** see File ownership (T17); ADR 0008 `## m4-t17 — MySQL and MariaDB introspection`.

**Spike origin (R-M4-40):** `m4-spike-mysql-sqlite-intro` @ `f690f45` (`spikes/spike-intro-mysql-sqlite/src/mysql.rs`, `model.rs`): the server-context read, bulk queries, marker logic and `check_tables` are copied into `mysql/**`; the SQL string constants become pack sections; `CatalogConn` is replaced by `MetaConn`.

**Interfaces:**
- Consumes: Tasks 4, 5, 10, 13; C-2's fixture logins (or R-M4-43's fallback).
- Produces: `mysql::MySqlIntrospector` implementing every `Introspector` method (`facet` and the `Server` namespace wired to `mysql::facets` stubs until Task 17b):
  - **MySQL 8.0/8.4 incremental** per alg/02 HV-INTRO-07 with **R-M4-39**: UTC session time, `server_uuid` + version identity, `W = UTC_TIMESTAMP(6)` read **inside** the consistent-snapshot transaction (DDL committed after the snapshot started is invisible to this pass and caught by the next, F2); every pass fetches `(name, kind, marker)` for every object of each kind; changed = marker ≠ stored **or** marker `≥ W − 1 s`; table markers `StateMark::MyTable { CREATE_TIME, max(TRIGGERS.CREATED), trigger count }`; routines and events `LAST_ALTERED`; views by an in-SQL `SHA2(CONCAT_WS(0x1f, …), 256)` hash plus `VIEW_TABLE_USAGE` dependency (8.0.13+); privileges by a per-schema hash; changed objects re-read with `… AND TABLE_NAME IN (…)` chunks (R-M4-3).
  - **MariaDB: `Full` per namespace** (R-M4-10; F3 shows `CREATE_TIME` behaves as on MySQL, but there is no consistent snapshot and the time base differs); rows whose parent the TABLES query did not list are skipped (no consistent snapshot on MariaDB, F2; HV-INTRO-01 edge 4) and corrected by the next full pass.
  - **`packs/mysql/10-core.hvq.sql`** (flavor- and version-gated, `INFORMATION_SCHEMA` upper case, no `UPDATE_TIME`, no `GROUP_CONCAT`, `information_schema_stats_expiry` untouched): `Identity` (version, flavor, `lower_case_table_names`, `server_uuid`, `sql_mode` flags `NO_BACKSLASH_ESCAPES`/`ANSI_QUOTES`, `@@character_set_system`, default collation per database → `ServerFacts`); a `Structure` pass of 13 bulk queries per schema (TABLES with `TABLE_ROWS` → `StatsFacet` and `CREATE_OPTIONS` for the partitioned flag, VIEWS, COLUMNS, STATISTICS, TABLE_CONSTRAINTS, KEY_COLUMN_USAGE, REFERENTIAL_CONSTRAINTS, CHECK_CONSTRAINTS, ROUTINES, PARAMETERS, TRIGGERS, EVENTS, the watermark), one row per column or key part grouped client-side; gates `STATISTICS.EXPRESSION` (8.0.13+), `ENFORCED` and `CHECK_CONSTRAINTS` (8.0.16+), `IGNORED` (MariaDB 10.6+), MariaDB `CHECK_CONSTRAINTS.TABLE_NAME`; **`show-create` sections** (R-M4-37) `ShowCreateTable|View|Procedure|Function|Trigger|Event` at `Sources` and for the "Definitions provided by server" toggle.
  - **Engine differences kept verbatim (F9):** column defaults and `ON UPDATE` text per flavor; unspecified FK `ON UPDATE` (`NO ACTION` on MySQL, `RESTRICT` on MariaDB); CHECK clause text; MariaDB display widths (`int(11)`); functional key parts (`COLUMN_NAME` NULL, `EXPRESSION` set); invisible indexes (`IS_VISIBLE`, MariaDB `IGNORED`) → `IndexProps.invisible`; descending parts as stored; routine identity `(ROUTINE_TYPE, SPECIFIC_NAME)` with `PARAMETERS.ORDINAL_POSITION = 0` as a function's return; MariaDB sequences (`TABLE_TYPE = 'SEQUENCE'`) → `ObjectKind::Sequence`; ENUM/SET labels parsed from `COLUMN_TYPE` (doubled quotes; backslash rules per `NO_BACKSLASH_ESCAPES`) → `ColumnProps.enum_labels`.
  - **`packs/mysql/30-state.hvq.sql`:** `StateOf<Kind>` (`joinable`) with the table stamp above; `stamps` returns them raw.
  - **`packs/mysql/40-system.hvq.sql`** (R-M4-44, `Mode::System`): relations and columns of `information_schema`, `mysql`, `performance_schema`, `sys`; `sys` routines with signatures from `PARAMETERS`; `ListSettings` (variable names; MariaDB scope and type; never values, never comments).
  - **R-M4-41 adoption:** after the probe below, a fenced block `// ## m4-t17-mysql-fold` in `crates/sql/sql-ident/src/key.rs` pins MySQL/MariaDB **column** keys: if every recorded pair matches ASCII-only folding, non-ASCII column probes stop being `ServerDependent`; if the server folds a non-ASCII pair, the matching simple fold (via `icu_casemap`, never Turkish) is adopted and ADR 0008 records the INV-3 exception. lctn ≥ 1 table names stay `ServerDependent`.

- [ ] **Step 1: Write the failing tests:**
```rust
// [docker: mysql80, mysql84 (pr)] (docker_mysql_intro.rs; database "hvm4_<test>" per test, dropped in a guard; R-M4-43 fallback: prefixed objects in hv_test under a static mutex)
#[tokio::test] #[ignore = "docker"] async fn instant_add_column_rereads_the_table() {}             // alg/02 HV-INTRO-07 test 1
#[tokio::test] #[ignore = "docker"] async fn add_index_reread() {}                                 // test 2
#[tokio::test] #[ignore = "docker"] async fn create_trigger_found_although_create_time_unchanged() {}   // test 3, ver/01 #mysql-is-times
#[tokio::test] #[ignore = "docker"] async fn insert_only_rereads_no_table() {}                     // test 4 (query log)
#[tokio::test] #[ignore = "docker"] async fn ddl_in_the_same_second_as_the_watermark_is_caught() {}   // test 5: condition-wait on UTC_TIMESTAMP(6), never sleep
#[tokio::test] #[ignore = "docker"] async fn create_or_replace_view_rehashes_and_rereads_columns() {}   // test 6
#[tokio::test] #[ignore = "docker"] async fn global_time_zone_plus_three_still_compares_markers() {}   // test 7: SET GLOBAL time_zone='+03:00' through hv_fixture, restored in a drop guard (wrapper lock)
#[tokio::test] #[ignore = "docker"] async fn index_columns_with_desc_part() {}                     // HV-INTRO-13 test 1
#[tokio::test] #[ignore = "docker"] async fn six_hundred_indexed_columns_assemble_complete() {}   // HV-INTRO-13 test 7 (client-side grouping)
#[tokio::test] #[ignore = "docker"] async fn turkish_collation_database_items_vs_items_missing() {}  // HV-INTRO-14 test 3 (creates hvm4_trdb utf8mb4_tr_0900_ai_ci)
#[tokio::test] #[ignore = "docker"] async fn trigger_creation_changes_table_state() {}              // HV-INTRO-15 test 3
#[tokio::test] #[ignore = "docker"] async fn revoked_mysql_schema_select_fails_users_kind_only() {} // HV-INTRO-01 test 6 (partial-load)
#[tokio::test] #[ignore = "docker"] async fn every_mysql_section_prepares_and_returns_its_declared_columns() {}
#[tokio::test] #[ignore = "docker"] async fn snapshot_hides_later_ddl_and_next_pass_catches_it() { /* F2 on MySQL: ALTER and CREATE from another session inside our snapshot are invisible; the next pass equals a fresh full */ }
#[tokio::test] #[ignore = "docker"] async fn show_create_sections_quote_hostile_names() { /* tables named a`b, İ and "t " → SHOW CREATE text returned for each */ }
#[tokio::test] #[ignore = "docker"] async fn mysql_enum_column_labels_parsed_with_doubled_quotes() {}
#[tokio::test] #[ignore = "docker"] async fn functional_key_part_and_invisible_index_read() {}
// [docker: mysql80, mysql84 (pr), mysql84-x509, mariadb106, mariadb1011, mariadb114]
#[tokio::test] #[ignore = "docker"] async fn long_alter_straddling_a_pass_is_seen() { /* ALTER … ADD COLUMN, ALGORITHM=COPY on 400k rows with a pass during it (F1: MySQL stamps the start); the next pass re-reads the table on every server */ }
#[tokio::test] #[ignore = "docker"] async fn flavor_differences_are_kept_verbatim() { /* defaults eu vs 'eu', CURRENT_TIMESTAMP vs current_timestamp(), ON UPDATE NO ACTION vs RESTRICT, CHECK text, int vs int(11), MariaDB sequence kind (F9) */ }
#[tokio::test] #[ignore = "docker"] async fn mysql_fk_auto_index_is_pinned() { /* result recorded in the report and ADR 0008, never assumed */ }
#[tokio::test] #[ignore = "docker"] async fn stamp_moves_on_comment_and_second_trigger() {}
#[tokio::test] #[ignore = "docker"] async fn server_facts_filled_per_engine() {}
// [docker: mysql80, mysql84, mariadb114] — R-M4-41
#[tokio::test] #[ignore = "docker"] async fn mysql_column_name_matching_is_pinned() { /* columns Äb, İd, ıd; selected as äb, id, ID, Id; record found/1054 per pair with @@character_set_system and the column-name collation (utf8mb3_tolower_ci on 8.0+) in the report and ADR 0008 */ }
// [docker: mariadb106, mariadb1011, mariadb114]
#[tokio::test] #[ignore = "docker"] async fn mariadb_full_per_namespace_after_each_alter() {}
#[tokio::test] #[ignore = "docker"] async fn record_mariadb_create_time_behaviour() { /* non-gating artifact target/docker/t17-mariadb-create-time.tsv for the ver/01 ALTER list (R-M4-10) */ }
#[tokio::test] #[ignore = "docker"] async fn every_mariadb_section_prepares() {}
#[tokio::test] #[ignore = "docker"] async fn mariadb_rows_without_a_listed_parent_are_skipped() {}
// sql-ident/tests/key.rs block ## m4-t17-mysql-fold: one test per recorded probe pair
```
- [ ] **Step 2:** `cargo test -p hacivat-introspect --no-run`. Expected: FAIL (compile or assertions).
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** `<main>/target/docker/run-docker-tests.sh --set full -p hacivat-introspect --test docker_mysql_intro`. Expected: PASS; attach the MariaDB artifact summary to the report.
- [ ] **Step 5: Commit** `feat(introspect): MySQL introspection with marker-or-window refresh; MariaDB full refresh; pinned MySQL column folding`.

**Review:** security-sensitive. Spike code reviewed as new code. Probes: `UPDATE_TIME` or `GROUP_CONCAT` anywhere; a `CREATE_TIME >= ?` filter in SQL (markers must be compared on the client); a marker compared in the session's local time zone; the watermark read outside the snapshot transaction; `LOWER()` on an I_S name column; a `SHOW CREATE` identifier not rendered through `sql_ident::quote`; MariaDB taking the MySQL incremental path; a fold adopted without a recorded probe pair.

### Task 18: SQLite introspector (HV-INTRO-08; HV-INTRO-13 and -15 SQLite parts) (W3)

**Files:** see File ownership (T18); ADR 0008 `## m4-t18 — SQLite introspection`.

**Spike origin (R-M4-40):** `m4-spike-mysql-sqlite-intro` @ `f690f45` (`sqlite.rs`): `list_namespaces`, `schema_version`, the digest, the bulk joins and their per-relation fallback, `refresh` and `table_options` are copied into `sqlite/**`.

**Interfaces:**
- Produces: `sqlite::SqliteIntrospector` (alg/02 HV-INTRO-08 algorithm steps 1–4) on Task 5's `mode=ro` metadata connection (R-M4-38):
  - `PRAGMA database_list`, `(dev, inode)` per file, `schema_version` fast path, an XXH3-128 digest of the ordered `(type, name, tbl_name, sql)` rows of `sqlite_schema` on connect and on identity change (sorted first), full re-read of a changed schema; `stamps` = `schema_version` + digest; `PRAGMA data_version` is not used (F8).
  - **Bulk shapes (F10):** tables and views as **two** separate bulk queries (`sqlite_schema` joined with `pragma_table_xinfo`, `pragma_index_list`, `pragma_index_xinfo`, `pragma_foreign_key_list`); when one kind's bulk query fails (a broken view, a virtual table whose module is missing), only that kind falls back to one query per relation, and a failing relation is kept with `ObjFlags::BROKEN`, the engine's message and no columns; `hidden` 2 = virtual and 3 = stored generated column; expression index parts (`cid = -2`) carry no name and their text comes from the index's SQL; the PK from `pk` positions (an `INTEGER PRIMARY KEY` has no index → `Identity::RowidAlias`, `AUTOINCREMENT` noted); UNIQUE from `sqlite_autoindex_*` (`origin = 'u'`) → `IMPLICIT`; FKs unnamed; CHECK constraints and generated expressions are **not** modelled as objects (only `sqlite_schema.sql` has them; R-M4-24 shows SQLite tables as server text); `WITHOUT ROWID`, `STRICT` and FTS5 shadow tables from `pragma_table_list` on the metadata connection, by the text scan on the user's connection (temp).
  - `function_list`/`module_list`/`collation_list` once per connection for built-ins (no snapshot, R-M4-6); `temp` only on the user's connection with the authorizer-allowed pragmas; `sqlite_master` before 3.33; a setting-backed "digest every pass" for network shares (edge 5); facets: grants `NotApplicable`, dependencies `Unsupported`, row estimate `None`; a joined connection (R-M4-29) reads on the given writer connection instead of the `mode=ro` one.
  - `packs/sqlite/{10-core,30-state}.hvq.sql`.

- [ ] **Step 1: Write the failing tests** (bundled SQLite through `rusqlite`, temp files; no Docker):
```rust
#[test] fn insert_skips_the_reread() {}                                          // alg/02 HV-INTRO-08 test 1 (no table_xinfo in the log)
#[test] fn create_index_bumps_and_rereads() {}                                   // test 2
#[test] fn rolled_back_ddl_skips() {}                                            // test 3
#[test] fn temp_table_rereads_only_temp_on_the_users_connection() {}             // test 4
#[test] fn same_schema_version_replaced_file_detected_on_reconnect_by_digest() {} // test 5
#[test] fn attach_adds_a_namespace() {}                                          // test 6
#[test] fn schema_version_sequence_matches_ver01() { /* baseline 1 | INSERT 1 | CREATE INDEX 2 | ADD COLUMN 3 | CREATE TEMP TABLE main 3, temp 1 | CREATE VIEW 4 | DROP VIEW 5 | rolled-back CREATE 5 | VACUUM 6 (ver/01 #sqlite-schema-version) */ }
#[test] fn stored_generated_column_hidden_3() {}                                 // HV-MODEL-01 test 8
#[test] fn unique_autoindex_and_pk() {}                                          // HV-INTRO-13 test 5
#[test] fn strict_and_without_rowid_props() {}
#[test] fn vacuum_rereads_harmlessly() {}
#[test] fn virtual_table_with_missing_module_lists_columns_unknown() {}
#[test] fn locked_database_retries_then_fails_softly() {}
#[test] fn identifier_case_rules_are_ascii_only() { /* ver/01 #sqlite-case, F6: CREATE TABLE "müşteri" after "Müşteri" fails (same table); MÜŞTERİ is another table; ITEMS = items */ }
#[test] fn builtins_come_from_function_list_live() {}
#[test] fn broken_view_falls_back_per_relation_and_is_kept_broken() {}
#[test] fn virtual_and_stored_generated_columns_by_hidden_2_and_3() {}
#[test] fn expression_index_part_has_no_name_and_server_text() {}
#[test] fn fts5_shadow_tables_flagged_on_the_metadata_connection() {}
#[test] fn sqlite_autoindex_is_implicit() {}
#[test] fn sqlite_rowid_alias_identity() {}
#[test] fn sqlite_joined_read_uses_the_writer_connection() {}
#[test] fn temp_schema_read_uses_only_authorizer_allowed_pragmas() {}
#[test] #[ignore = "perf"] fn ten_thousand_tables_structure_recorded() { /* spike: Names 44 ms, Structure 245 ms, unchanged check 17 ms; recorded, not gating */ }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-introspect --test sqlite_intro`. Expected: FAIL.
- [ ] **Step 3: Implement** on the blocking pool (`spawn_blocking`, one dedicated thread per connection, spec §5.4).
- [ ] **Step 4:** Run the tests also under `LC_ALL=tr_TR.UTF-8`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(introspect): SQLite introspection with schema_version and schema digest change detection`.

**Review:** opus. Probes: the user's connection used for `main`; a write through the metadata connection; a digest that depends on row order the server does not guarantee (sort first).

### Task 19: DDL producers from the model for view-only use; goldens (HV-DDL-07 producers) (W3)

**Files:** see File ownership (T19); ADR 0008 `## m4-t19 — DDL producers`.

**Interfaces:**
- Consumes: Task 3's model and fixture builder; Task 7's IR, renderers, placement, qualification and options; `ModelView` (the `children`/`lookup` contract; tests use a fixture view).
- Produces: `create_script(view: &dyn ModelView, objects, target, opts) -> Result<Script, DdlError>` for tables (columns in model order skipping hidden/pseudo and inherited columns; canonical display type of the server spelling; collation if non-default; generated/identity; raw defaults verbatim per flavor; constraints per placement; PG `PARTITION BY`, `PARTITION OF … FOR VALUES`, `INHERITS`, `USING`, `WITH (…)`, `TABLESPACE`, `ENABLE ROW LEVEL SECURITY` when `row_security` (policies are not introspected in M4: notice `ddl-policies-not-loaded`); MySQL `ENGINE`, charset, collate, inline `COMMENT`; then non-constraint indexes, comments, triggers (server text), grants and owner unless skipped), views and materialized views (server text, R-M4-24; MySQL from `SHOW CREATE`), indexes (PG `pg_get_indexdef` text when an expression/partial/INCLUDE is present, else synthesized), sequences, functions and procedures (server text), triggers (server text), PG types (enum, domain, composite, range); **SQLite tables, indexes, views and triggers are emitted as their `sqlite_schema.sql` server text** (CHECK and generated expressions exist only there; autoindexes, whose `sql` is NULL, are implicit and skipped); **grants come from the `Grants` facet**: not loaded and `skip_grants = false` → no GRANT statements **and** the notice `ddl-grants-not-loaded`; `Partial` → the statements read plus `ddl-grants-partial`; owners from `Props.owner`; `Script { stmts, notices }`, `Script::text(delimiters: DelimStyle /* Semicolon | MySqlDelimiter for file output */) -> String`; notices `ddl-warn-executable-comment` (MySQL server text containing `/*!`), `ddl-unsupported-kind`, `ddl-sqlite-placement-forced`; `ObjectKind::Unknown` → `DdlError::Unsupported` (alg/02 HV-MODEL-01 edge 11); virtual objects never emitted (layer `Virtual`).

- [ ] **Step 1: Write the failing tests** (alg/03 HV-DDL-07 edge-case list as fixtures; one golden per `dialect × kind × case`):
```rust
#[test] fn golden_pg_table_full() {}            // columns, identity, raw default, PK/UK/FK/check inside table, comment, owner, grant
#[test] fn golden_pg_zero_column_table() { /* CREATE TABLE t ();  */ }
#[test] fn golden_pg_partitioned_parent_and_partition() {}
#[test] fn golden_pg_inherited_columns_skipped() {}
#[test] fn golden_pg_identity_vs_serial_origin() {}
#[test] fn golden_pg_view_server_text() {}
#[test] fn golden_pg_expression_partial_include_index_from_server_text() {}
#[test] fn golden_pg_enum_domain_composite_range() {}
#[test] fn golden_pg_function_and_trigger_server_text() {}
#[test] fn golden_pg_turkish_and_mixed_case_identifiers_quoted() {}
#[test] fn golden_mysql_table_engine_charset_inline_comment_table_level_fk() {}
#[test] fn golden_mysql_view_and_routine_with_definer_and_skip_definer() {}
#[test] fn golden_mysql_executable_comment_flagged() {}
#[test] fn golden_mariadb_table() {}
#[test] fn golden_sqlite_server_text_verbatim_with_strict_without_rowid_generated_and_check() {}
#[test] fn golden_sqlite_comments_never_emitted() {}
#[test] fn comments_with_quote_dashes_newline_are_escaped() {}
#[test] fn unnamed_constraints_get_no_synthetic_names() {}
#[test] fn unknown_type_emitted_as_is_with_a_warning() {}
#[test] fn unknown_kind_is_refused() {}
#[test] fn virtual_fk_is_never_emitted() {}
#[test] fn goldens_byte_stable_twice() {}
#[test] fn producer_without_grants_facet_emits_notice_not_silence() {}
#[test] fn row_security_table_emits_enable_and_policies_notice() {}
#[test] fn golden_mariadb_vs_mysql_defaults_verbatim() {}
#[test] fn goldens_identical_under_tr_locale() {}   // turkey-test
```
- [ ] **Step 2:** Run `cargo test -p hacivat-ddl --test produce --test golden`. Expected: FAIL.
- [ ] **Step 3: Implement**; bless goldens once with `UPDATE_GOLDEN=1` and review every file by hand (each golden is listed in the report with one line on why it is right).
- [ ] **Step 4:** Run the tests without the flag, under `LC_ALL=tr_TR.UTF-8`, and `cargo xtask check-deps`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(ddl): view-only DDL producers for tables, views, indexes, sequences, routines, triggers and PG types`.

**Review:** opus. Probes: server text reformatted or re-qualified; a synthetic constraint name leaking; output that depends on `HashMap` order; NUL accepted.

### Task 20: Scope editor, "Encrypt cached metadata", introspection level pills (W3, gpui)

**Files:** see File ownership (T20); tests `crates/app/hacivat-app/tests/scope_editor.rs`.

**Interfaces:**
- Consumes: Task 12 (`parse_scope`, `to_text`, `ScopeError.column`), Task 14 (explorer rows), Task 1c's `Profile.catalog_sensitive` and `profile_data`, Task 1a's `ProfileUserData`, the connection dialog (`connections/dialog/form.rs`), `ConnectionsModel` (the single writer of profiles).
- Produces: the dialog section "Schemas" (mock 7): a checkbox tree of databases/schemas (from the last `ScopeRefresh` when connected or cached; "Connect to list schemas" otherwise), "All" toggles, the pattern field that round-trips with the tree through the AST, an inline error at the character position, the PostgreSQL all-databases cost prompt (once); the checkbox "Encrypt cached metadata" bound to `catalog_sensitive`, defaulting on when the profile's environment is production (R-M4-15) — changing it deletes the profile's cache files after a confirmation; level pills and the "Introspection level" submenu on database and schema rows (blue explicit, grey inherited), persisted in `ProfileUserData.level_overrides` through `explorer::userdata`, the app's only caller of `hacivat_settings::profile_data` (the per-profile user-data file, spec §5.6).

- [ ] **Step 1: Write the failing tests:**
```rust
#[gpui::test] fn pattern_and_tree_round_trip(cx) {}
#[gpui::test] fn syntax_error_points_at_the_column_in_scalars(cx) { /* "müş{" → error at column 4 */ }
#[gpui::test] fn pg_all_databases_prompts_once(cx) {}
#[gpui::test] fn sensitive_defaults_on_for_production_profiles(cx) {}
#[gpui::test] fn turning_encryption_on_or_off_confirms_and_drops_the_cache(cx) {}
#[gpui::test] fn level_override_shows_explicit_and_inherited_pills(cx) {}
#[gpui::test] fn level_override_persists_in_the_user_data_file(cx) {}
#[gpui::test] fn profile_writes_go_through_connections_model_only(cx) {}
#[gpui::test] fn escape_cancels_without_saving(cx) {}
#[gpui::test] fn scope_section_fits_960x600_in_tr(cx) {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-app --features scene-check,pseudo --test scope_editor`. Expected: FAIL.
- [ ] **Step 3: Implement** per mock 7.
- [ ] **Step 4:** Run the tests; render the connection-dialog shots and list every changed approved PNG. Expected: PASS.
- [ ] **Step 5: Commit** `feat(app): schema scope editor, encrypted-cache option and introspection level pills`.

**Review:** opus (gpui). Probes: a second profile writer; a pattern saved that does not parse; byte offsets shown as columns; the sensitive default applied to a non-production profile.

## W4 — Facets, refresh after DDL, freshness, snapshots, live explorer, DDL view

### Task 16b: PostgreSQL facets and server-level sections (R-M4-30) (W4)

**Files:** see File ownership (T16b); ADR 0008 `## m4-t16b — PostgreSQL facets`.

**Interfaces:**
- Consumes: Task 16's introspector (the `facets::read`/`server_namespace` call sites), Task 13's `Ref` assembly.
- Produces: `pg::facets::{read(c, ns, keys, FacetKind) -> FacetRows, server_namespace(c) -> NamespaceResult}` and `packs/pg/20-facets.hvq.sql` (selected by oid, `joinable` where M7 needs them):
  - `Grants<Kind>`: `relacl` (tables, views, sequences), `attacl` (columns), `proacl`, `nspacl`, `typacl`, decoded client-side into `GrantsFacet` (grantee, privilege, columns, grantable); owners from `relowner`/`proowner`/`nspowner`/`typowner` into `Props.owner` (already read at `Structure` by Task 16; the facet reads only ACLs). An ACL the login cannot read → `LoadState::Partial`.
  - `Dependencies`: `pg_depend` (`deptype NOT IN ('e','x','p')`) and `pg_rewrite` for views → `DepsFacet`.
  - `ListRoles` at `Names` into the `NsKind::Server` namespace (`pg_roles.rolname`; a refused read → `Unknown`, never empty).
  - A `rel` change (Task 16's `ChangedParents`; `GRANT` rewrites the `pg_class` row) invalidates a loaded `Grants` facet of that object, so the next `load_facet` re-reads it.

- [ ] **Step 1: Write the failing tests:**
```rust
// [docker: pg13, pg14, pg15, pg16, pg17 (pr), pg18] (docker_pg_facets.rs; schema "hvm4_<test>")
#[tokio::test] #[ignore = "docker"] async fn pg_grants_and_owner_read() {}
#[tokio::test] #[ignore = "docker"] async fn pg_column_grant_read() {}
#[tokio::test] #[ignore = "docker"] async fn pg_view_dependencies_read() {}
#[tokio::test] #[ignore = "docker"] async fn pg_roles_listed_at_names_level() {}
#[tokio::test] #[ignore = "docker"] async fn grant_change_invalidates_a_loaded_grants_facet() {}
#[tokio::test] #[ignore = "docker"] async fn every_pg_facet_section_prepares_and_returns_its_declared_columns() {}
```
- [ ] **Step 2:** `cargo test -p hacivat-introspect --no-run`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** `run-docker-tests.sh --set full -p hacivat-introspect --test docker_pg_facets`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(introspect): PostgreSQL grants, dependencies and roles as facets`.

**Review:** security-sensitive. Probes: an ACL text parsed by splitting on characters a quoted role name may contain; a refused read that yields an empty list instead of `Unknown`/`Partial`; a name spliced into SQL.

### Task 17b: MySQL and MariaDB facets and server-level sections (R-M4-30) (W4)

**Files:** see File ownership (T17b); ADR 0008 `## m4-t17b — MySQL and MariaDB facets`.

**Interfaces:**
- Consumes: Task 17's introspector; C-2's `hv_fixture` (creates the limited test user).
- Produces: `mysql::facets::{read, server_namespace}` and `packs/mysql/20-facets.hvq.sql`: `SCHEMA_PRIVILEGES`/`TABLE_PRIVILEGES`/`COLUMN_PRIVILEGES` (rows visible to the account only → `Partial` unless the account can read `mysql.tables_priv`); `VIEW_TABLE_USAGE`/`VIEW_ROUTINE_USAGE` (MySQL ≥ 8.0.13; MariaDB `Unsupported`); `ListRoles` (`APPLICABLE_ROLES` plus the current user; a refused read → `Unknown`, never empty).

- [ ] **Step 1: Write the failing tests:**
```rust
// [docker: mysql84 (pr), mysql80, mariadb114] (docker_mysql_facets.rs; database "hvm4_<test>")
#[tokio::test] #[ignore = "docker"] async fn mysql_grants_partial_for_unprivileged_account() { /* hv_fixture creates hvm4_limited with SELECT on one table; the facet read as hvm4_limited is Partial */ }
#[tokio::test] #[ignore = "docker"] async fn mysql_view_table_usage_read() {}             // mysql84, mysql80
#[tokio::test] #[ignore = "docker"] async fn mariadb_dependencies_are_unsupported() {}   // mariadb114
#[tokio::test] #[ignore = "docker"] async fn mysql_roles_refused_is_unknown_not_empty() {}
#[tokio::test] #[ignore = "docker"] async fn every_mysql_facet_section_prepares() {}
```
- [ ] **Step 2–4:** as Task 16b (`--test docker_mysql_facets`). Expected: PASS.
- [ ] **Step 5: Commit** `feat(introspect): MySQL and MariaDB privileges, view dependencies and roles as facets`.

**Review:** security-sensitive. Probes: a `Partial` facet presented as complete; the fixture user left behind after a panic; an empty role list on a refused read.

### Task 21: Refresh after DDL — planning and transaction-aware coordination (HV-INTRO-12) (W4)

**Files:** see File ownership (T21); ADR 0008 `## m4-t21 — refresh after DDL`; ADR 0006 `## m4-t21 — catalog refresh sink`.

**Interfaces:**
- Consumes: Task 6 (`StmtReport.ddl`), Task 15 (`IntroHandle::{fragment, refresh}`), the exec auditor chain (`hacivat-exec/src/audit.rs`, `RunEvent::UnitEnded`, the HV-EXEC-06 sink pattern at `audit.rs:66-72`), session events (`SessionEvent::TxChanged`, `SchemaChanged`, `session/mod.rs:290-315`; or Task 1c's `TxEnded`).
- Produces: `plan_refresh(stmts: &[ExecutedStmt], session: &SessionStateAtExec, txn: TxnState, caps: &IntroCaps) -> RefreshPlan { now, pending }` (alg/02 HV-INTRO-12 algorithm steps 1–4; successful statements only; unqualified names resolved with the search path **captured at execution time**; > 50 object targets in one namespace widen to `Namespace`; temp objects → nothing (M5 overlay); opaque → `Namespace(current)` plus namespaces literally named in the text); `RefreshCoordinator` per profile with timing per the per-DBMS matrix: **PG** transactional → pending until `COMMIT`, immediate for non-transactional statements such as `CREATE INDEX CONCURRENTLY`; **MySQL/MariaDB** implicit commit → immediate (ver/01 `#mysql-ro`); **SQLite** (R-M4-42) autocommit DDL → immediate, DDL inside an explicit transaction → pending until `COMMIT`, discarded on `ROLLBACK`/`ROLLBACK TO` (the `mode=ro` metadata connection cannot see uncommitted DDL; no compensating refresh); `pending(session)` exposed for M5 (R-M4-11); after the refresh, `check_state` on the targets (step 6); preference `metadata.refresh_after_ddl` (`always`/`ask`/`never`; `never` marks targets stale); **`refresh_now(targets) -> RefreshTicket`** ignores the preference (M7's Modify Object and source edit always refresh); `CatalogRefreshSink` body: maps each `UnitEnded` (with the unit's `StmtReport` and the session state snapshot the run already records) to `ExecutedStmt`s and hands them to the coordinator.

- [ ] **Step 1: Write the failing tests** (alg/02 HV-INTRO-12 tests 1–9, 11; edge cases 1–14; SQL Server rows excluded):
```rust
#[test] fn create_table_autocommit_refreshes_object_and_names_now() { let p = plan(&["create table s1.t(x int)"], pg_autocommit()); assert_eq!(p.now, vec![Target::Object(k("s1.t")), Target::NamespaceNames(ns("s1"))]); assert!(p.pending.is_empty()); }
#[test] fn drop_schema_removes_the_namespace_and_refreshes_scope() {}
#[test] fn rename_uses_the_search_path_at_execution() {}
#[test] fn create_database_refreshes_the_data_source_scope() {}
#[test] fn select_has_no_targets() {}
#[test] fn do_block_refreshes_the_current_namespace() {}
#[test] fn failed_statement_produces_no_target() {}
#[test] fn drop_table_if_exists_missing_is_harmless() {}
#[test] fn pg_drop_index_resolves_its_table_from_the_snapshot_or_widens() {}
#[test] fn fk_refreshes_both_tables() {}
#[test] fn alter_type_refreshes_dependent_tables() {}
#[test] fn more_than_50_targets_widen_to_namespace() {}
#[test] fn create_index_concurrently_is_immediate() {}
#[test] fn savepoint_rollback_discards_later_targets() {}
#[test] fn never_preference_marks_targets_stale() {}
#[tokio::test] async fn pending_survives_a_reconnect_after_the_commit_ack() {}
#[tokio::test] async fn refresh_is_followed_by_check_state() {}
#[test] fn sqlite_ddl_in_explicit_txn_appears_after_commit() {}
#[test] fn sqlite_rolled_back_ddl_queries_nothing() {}
#[test] fn sqlite_autocommit_ddl_refreshes_now() {}
#[tokio::test] async fn refresh_now_ignores_never_preference() {}
// [docker: pg17 (pr), mysql84 (pr)] (docker_refresh_after_ddl.rs)
#[tokio::test] #[ignore = "docker"] async fn pg_ddl_in_a_transaction_is_pending_until_commit() {}       // test 7 (explorer side)
#[tokio::test] #[ignore = "docker"] async fn pg_rollback_discards_pending_and_queries_nothing() {}      // test 8
#[tokio::test] #[ignore = "docker"] async fn mysql_rollback_after_ddl_still_refreshes() {}              // test 9
#[tokio::test] #[ignore = "docker"] async fn dynamic_sql_do_block_makes_z_appear() {}                    // test 6
```
- [ ] **Step 2:** Run `cargo test -p hacivat-introspect --test plan_refresh`; `--no-run` for the Docker file. Expected: FAIL.
- [ ] **Step 3: Implement**; the sink never blocks the run (bounded channel, drop-oldest with a counter, the HV-EXEC-06 pattern).
- [ ] **Step 4:** Run the unit tests; `run-docker-tests.sh --set pr -p hacivat-introspect --test docker_refresh_after_ddl`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(introspect): refresh the catalog after DDL, timed by the transaction`.

**Review:** opus. Probes: a refresh issued inside the user's open PG transaction; targets from a failed statement; a run slowed by the sink; current-schema read at refresh time instead of execution time.

### Task 22: Freshness — state checks, background validation, periodic checks (HV-INTRO-15) (W4)

**Files:** see File ownership (T22); ADR 0008 `## m4-t22 — freshness`.

**Interfaces:**
- Consumes: Tasks 15–18 (`stamps`, `check_namespace` sections), Task 9 (negative-probe hook).
- Produces: `freshness::{check_state, check_namespace}` orchestration: `check_state` = `Introspector::stamps` compared **by equality** with the snapshot's marks (PG stamps include one dependency hop and a relation's children, R-M4-48, so a renamed dependency or a removed comment reads `Changed`); per kind, ≤ 1,000 keys per round trip; outcomes `Current`/`Changed`/`Dropped`/`Unknown` (permission loss → `Unknown`, never `Dropped`); `Changed`/`Dropped` enqueue a `Fragment`; on-connect background validation of every cached in-scope namespace (`check_namespace`: count + max marker + existence digest; match → clear `stale` with state queries only; mismatch → `Incremental`); periodic validation per R-M4-20 (`metadata.validate_interval_min = auto`: 10 min for `local`/`dev` tags, off otherwise); deferral during the user's transaction on single-connection profiles; the negative-cache probe of Task 9 wired to `check_state` by name.

- [ ] **Step 1: Write the failing tests:**
```rust
#[tokio::test] async fn keys_over_1000_are_chunked() {}
#[tokio::test] async fn permission_loss_is_unknown_never_dropped() {}
#[tokio::test] async fn changed_and_dropped_enqueue_a_fragment() {}
#[tokio::test] async fn deferred_during_user_transaction_on_single_connection() {}
#[tokio::test(start_paused = true)] async fn periodic_default_on_for_local_and_dev_off_otherwise() {}
#[tokio::test] async fn missing_name_probe_uses_check_state_by_name() {}
#[tokio::test] async fn stamps_equal_check_state_inputs() {}
// [docker: pg17 (pr), mysql84 (pr), pg14, pg18, mysql80] (docker_freshness.rs)
#[tokio::test] #[ignore = "docker"] async fn pg_comment_only_change_is_changed() {}              // includes pg_description xmin
#[tokio::test] #[ignore = "docker"] async fn pg_drop_and_recreate_is_dropped_plus_new() {}
#[tokio::test] #[ignore = "docker"] async fn mysql_trigger_creation_makes_the_table_changed() {}
#[tokio::test] #[ignore = "docker"] async fn unchanged_server_clears_stale_with_state_queries_only() {}
#[tokio::test] #[ignore = "docker"] async fn check_state_sees_a_renamed_dependency() { /* a view over a column that is renamed → Changed (PG spike finding 4, §7 Q2) */ }
#[tokio::test] #[ignore = "docker"] async fn pg_comment_removal_is_changed() {}
#[test] fn sqlite_state_is_schema_level() {}
```
- [ ] **Step 2–4:** as Task 21 (unit, then the Docker file through the wrapper). Expected: PASS.
- [ ] **Step 5: Commit** `feat(introspect): object state checks and background validation of cached namespaces`.

**Review:** opus. Probes: `Dropped` from a permission error; ordering comparison on PG xmin (must be equality); a validation pass that re-reads object rows when nothing changed.

### Task 23: `minicat` — system-catalog snapshots, assets and the CI job (HV-INTRO-16 generation; L7; D-017) (W4)

**Files:** see File ownership (T23); ADR 0008 `## m4-t23 — system-catalog snapshots`.

**Spike origin (R-M4-40):** `m4-spike-minicat` @ `cd27ec1`: `xtask/src/minicat.rs` (`collect`/`build`/`check`/`verify`, the 11 offline unit tests), the `minicat-verify` CI job and the raw-file boundary are copied; the spike's Docker read (`hacivat-exec/tests/minicat_read.rs`, raw catalog SQL) is **not** copied — the read is rebuilt over the packs below. The C-1 artifact and its carries (exclusion rules, the first complete MySQL/MariaDB set) are inputs.

**Interfaces:**
- Consumes: Tasks 16 and 17 (their `40-system` sections and `10-core` relation/column/routine sections in `Mode::System`), Task 4 (rendering), Task 1c's `hv_catalog` login (C-2), the C-1 carries.
- Produces:
  - **Docker read** `crates/core/hacivat-introspect/tests/minicat_gen.rs` (`#[ignore = "docker"]`, active only with `HACIVAT_MINICAT=1`): for each target `pg14`, `pg15`, `pg16`, `pg17`, `pg18`, `mysql80`, `mysql84`, `mariadb106`, `mariadb1011`, `mariadb114`, runs the system-mode pack sections through `minicat::run_system_sections` on a `MetaConn` (PG database `postgres`; MySQL/MariaDB as `hv_catalog` with `mysql` as the default database) and writes one raw JSON file per section in server row order through `hacivat_testkit::artifact`; plus the **generator-only** MySQL/MariaDB built-in name probe (R-M4-44: `PREPARE` probes of `sql-safety/data/mysql/manual-index.txt` from a granted database, MariaDB `information_schema.SQL_FUNCTIONS` added, a sentinel name required absent; never on a `MetaConn`; the single R-M4-7 exception, ADR 0008).
  - **Format-1 sources** (the canonical JSON the build writes):

    | Format-1 section | Source |
    |---|---|
    | PG relations and columns (`pg_catalog`, `information_schema`) | T16 `40-system` relations/columns (OID < 16384) |
    | PG routines (args, identity, result, kind, volatility, set-returning, description) | T16 `40-system` routines |
    | PG operators | T16 `40-system` operators |
    | PG types (SQL name, kind, category, array element, description) | T16 `40-system` types |
    | PG GUC names with type and context | T16 `ListSettings` |
    | MySQL/MariaDB relations and columns (`information_schema`, `mysql`, `performance_schema`, `sys`) | T17 `40-system` relations/columns |
    | MySQL/MariaDB `sys` routines with signatures | T17 `40-system` routines + `PARAMETERS` |
    | MySQL/MariaDB built-in function names | the generator-only probe (names only; comments never read) |
    | MySQL/MariaDB variable names (MariaDB scope and type) | T17 `ListSettings` |
  - **`xtask minicat`** (spike): `collect [--reader catalog] [--twice] [--require-complete]` runs `cargo test -p hacivat-introspect --test minicat_gen --no-run`, then the wrapper with `HACIVAT_MINICAT=1 … --set full -p hacivat-introspect --test minicat_gen` (through the wrapper's lock), builds (normalise kinds, nest columns, sort by byte order, fail on unknown codes or duplicates, canonical pretty JSON, zstd 19) into `assets/system-catalogs/<dbms>-<line>.hvsnap.zst` and writes `manifest.toml` (R-M4-17 fields incl. `excluded`); `--check` (offline: manifest lists exactly the files present and is in generated form, digests and counts match, every file decodes and is canonical and re-encodes to the same bytes); `--verify` (regenerate into `<work>/verify` and byte-compare the files, printing the first differing section and object). No `sqlite-3` file (R-M4-6); the committed set must pass `--require-complete`.
  - **`hacivat_meta::system::{decode, normalize}`** (placement per R-M4-33, named exception in Task 26's module; feature `system`): format-1 JSON → `SystemSnapshot` = one `NamespaceSnapshot` per system schema, every object flagged `PRE_INTROSPECTED`, routines from the name probe carry `FnSig.args = None` (name only), `sys`/PG routines carry their arguments, operators and GUC/variable names in a side table served by `sql_catalog::CatalogView::settings`; a truncated or corrupt file is an error, never a panic.
  - **CI** block `# ## m4-t23-minicat`: the spike's `minicat-verify` job (`workflow_dispatch`, `schedule` for when `m4` reaches `main`, and PRs touching `crates/core/hacivat-{meta,introspect}/**` or `assets/system-catalogs/**`): `check-pins`, `minicat --check`, `docker up --set full` on CI's own stack, `minicat --verify --twice`, `docker down -v`, `up`, `minicat --verify`, artifact always uploaded, logs on failure, `down` always. `minicat --check` also runs in `build-test` through the xtask unit test `the_committed_snapshots_are_canonical`.
  - The `system_snapshot_decode` fuzz target body.

- [ ] **Step 1: Write the failing tests:**
```rust
// hacivat-meta/tests/system_decode.rs
#[test] fn decode_of_truncated_asset_is_error_not_panic() {}
#[test] fn decoded_objects_are_pre_introspected_and_probe_routines_are_name_only() {}
#[test] fn decode_round_trips_the_canonical_json() {}
// xtask unit tests (spike's 11, kept) plus:
#[test] fn check_is_offline_and_detects_one_flipped_byte() {}
#[test] fn manifest_lists_ten_snapshots_with_exact_versions_and_excluded_rules() { /* pg14..pg18, mysql80, mysql84, mariadb106, mariadb1011, mariadb114; exact versions from the servers; image digests equal compose.yml's pins; excluded = the named rules */ }
#[test] fn require_complete_refuses_a_partial_mysql_file() {}
#[test] fn sizes_are_recorded() { /* manifest carries JSON and zst bytes per file; the report lists the total (PO-Q1; spike: ≈ 368 KiB partial, 0.5–0.7 MB complete expected) */ }
// minicat_gen.rs
#[tokio::test] #[ignore = "docker"] async fn snapshot_bytes_identical_across_two_runs() {}   // per target, via collect --twice
#[tokio::test] #[ignore = "docker"] async fn builtin_name_probe_sentinel_is_absent() {}
```
- [ ] **Step 2:** Run the unit tests. Expected: FAIL.
- [ ] **Step 3: Implement.** If a double run or the C-1 fresh-stack run differs, find the volatile content, exclude it by a named rule (kind, schema or column), record the rule in ADR 0008, in `manifest.toml` `excluded` and in the report — never by loosening the comparison.
- [ ] **Step 4:** With the complete local set available (C-2 done): `cargo run -p xtask -- minicat collect --twice --require-complete`, then `minicat --check` and `minicat --verify`. If the PO declined C-2, take the MySQL/MariaDB files from the C-1 artifact (read and reviewed, listed in the report) and the PG files from the local read. Expected: ten files, PASS, `--verify` clean. Commit the assets (binary, `.gitattributes`).
- [ ] **Step 5: Commit** `feat(minicat): reproducible system-catalog snapshots for PostgreSQL 14–18, MySQL 8.0/8.4 and MariaDB 10.6/10.11/11.4`.

**Review:** security-sensitive (generated binary assets ship to users and are decoded at runtime). Spike code reviewed as new code. Probes: a snapshot built from an unpinned image; a decode path that trusts lengths; non-deterministic order (`HashMap` iteration); a volatile field kept; MySQL/MariaDB comment or help text (GPL documentation) in a file; the name probe reachable from runtime code; DataGrip-derived content (must be none).

### Task 24: Explorer live wiring (W4, gpui)

**Files:** see File ownership (T24); ADR 0007 `## m4-t24 — live explorer`.

**Interfaces:**
- Consumes: Tasks 9–11, 14, 15 (`CatalogHub`; 16–18 through `introspector_for`), Task 20's `explorer::userdata`, the connections model and live state (`connections/{live,boot}.rs`), the console opener (`console/commands.rs`).
- Produces: per-profile `CatalogHost` in the app: on app start, profiles with a cache file get their cached tree (offline strip "Showing cached metadata from {date}", date preformatted with ICU4X); on connect: cache opened (encrypted per `catalog_sensitive`; keyring unavailable → memory-only with notice `model-cache-disabled-by-policy`-style message from Task 1c's block), cached namespaces published stale, `IntroHandle::on_connect`; on disconnect the engine stops and the cache stays; inline progress rows (phase label + determinate count, never concatenated text); partial-load badge; stale badge; refresh actions (object → fragment, namespace → incremental, profile → scope refresh + incremental; all `Trigger::User`); mass-deletion confirmation (mock 8; user-triggered) and stale badge (background) per R-M4-22, `confirm_pending`; context menus (mock 5): **Open data** (R-M4-13: new console on the same profile/schema with `SELECT * FROM <qualified, quoted name>` and Run through the normal path), **View DDL** (Task 25's `open_ddl_view`), **Copy name** / **Copy qualified name** (dialect quoting only where needed), **Refresh**, **Hide objects like this** (adds a view filter), **Introspection level ▸** (Task 20), **Edit scope…** (opens the dialog section), **Modify…** disabled with a tooltip naming a later version (M7; **PO-Q13**); double-click: tables and views → Open data; containers → toggle.
- **Also produces:**
  - `CatalogHost` is a thin UI wrapper over `CatalogHub` (engine, cache and key live in the hub).
  - **Start-up key timing (R-M4-15, review I10):** at app start the hub fetches `CatalogKey` (blocking pool) only for encrypted profiles **expanded in the restored workspace**; their cached tree paints when the key arrives, without blocking the UI; every other encrypted profile shows its root row collapsed and makes no keyring call until the user expands or connects it. Plaintext profiles paint from the cache at once.
  - `explorer::reveal(profile, &ObjectKey, window, cx) -> RevealOutcome` (expands ancestors, loads lazily, selects, scrolls); `explorer::database_rows(profile, cx) -> Vec<DatabaseRow { name, in_scope, connected }>`; provided actions rendered in slot order (the disabled "Modify…" placeholder, PO-Q13, yields to a provider that fills `Modify`; other later-milestone actions stay hidden until provided, PO-Q16); the default open-data handler (R-M4-13) installed through `set_open_data_handler`; `console::open_with_text(profile, database: Option<&str>, text, run: bool)`; expanded-state persistence in `workspace.json` (`explorer.expanded: { <profile uuid>: [TreeKey as hex] }`), restored only for keys present in the cached snapshot or provided by a contributor, never starting introspection beyond what on-connect already schedules (PO-Q15).

- [ ] **Step 1: Write the failing tests:**
```rust
#[gpui::test] fn app_start_shows_cached_tree_before_any_catalog_query(cx) {}
#[gpui::test] fn offline_profile_shows_cached_with_its_date(cx) {}
#[gpui::test] fn connect_publishes_cached_stale_then_validates(cx) {}
#[gpui::test] fn refresh_object_runs_a_fragment_only(cx) {}
#[gpui::test] fn user_refresh_mass_drop_asks_and_cancel_keeps_cache(cx) {}
#[gpui::test] fn background_mass_drop_shows_stale_badge_and_no_dialog(cx) {}
#[gpui::test] fn copy_qualified_name_quotes_per_dialect(cx) { /* PG "Users" vs users → public."Users" / public.users; MySQL `order`; SQLite main."a b" */ }
#[gpui::test] fn open_data_creates_a_console_and_runs_through_the_gate(cx) {}
#[gpui::test] fn open_data_on_production_is_a_read_and_needs_no_confirmation(cx) {}
#[gpui::test] fn hide_like_this_adds_a_view_filter_and_lookup_still_finds_it(cx) {}
#[gpui::test] fn modify_is_disabled_with_a_tooltip(cx) {}
#[gpui::test] fn disconnect_stops_the_engine_and_keeps_the_cache(cx) {}
#[gpui::test] fn encrypted_profile_without_keyring_runs_memory_only_with_a_notice(cx) {}
#[gpui::test] fn progress_row_shows_phase_and_count_and_repaints_only_on_change(cx) {}
#[gpui::test] fn ui_thread_never_awaits_catalog_io(cx) { /* probe: every CacheStore/MetaConn call observed off the main thread */ }
#[gpui::test] fn notify_is_coalesced_to_one_per_frame(cx) {}
#[gpui::test] fn encrypted_profile_not_expanded_makes_no_keyring_call_at_start(cx) {}
#[gpui::test] fn encrypted_expanded_profile_paints_after_key_without_blocking_ui(cx) {}
#[gpui::test] fn reveal_expands_ancestors_and_selects(cx) {}
#[gpui::test] fn database_rows_marks_greyed_pg_databases_out_of_scope(cx) {}
#[gpui::test] fn open_data_handler_can_be_replaced(cx) {}
#[gpui::test] fn provided_actions_appear_in_their_slot_for_matching_kinds(cx) {}
#[gpui::test] fn expanded_state_restored_after_restart_from_cache(cx) {}
#[gpui::test] fn restore_never_triggers_introspection_of_out_of_scope_nodes(cx) {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-app --features scene-check,pseudo --test explorer_live --test explorer_actions`. Expected: FAIL.
- [ ] **Step 3: Implement** per mocks 1–6, 8.
- [ ] **Step 4:** Run the tests, the routing test (stub count), clippy (`--all-features`), `l10n check`; render the explorer shots: list new states for Task 32 and every changed approved PNG. Expected: PASS.
- [ ] **Step 5: Commit** `feat(app): live explorer — cached on connect, incremental validation, refresh, mass-deletion guard and context actions`.

**Review:** opus (gpui). Probes: a catalog call awaited on the UI thread; a keyring call at startup for an encrypted profile that is not expanded; a mass drop applied without the answer; Open data concatenating an unquoted name; Escape in the confirmation removing objects; a second writer of `profiles/<uuid>.toml`.

### Task 25: DDL view tab (W4, gpui)

**Files:** see File ownership (T25); ADR 0007 `## m4-t25 — DDL view`.

**Interfaces:**
- Consumes: Task 19's `create_script`, Task 15's `elevate`/`fragment` (Sources level on demand), `IntroHandle::check_state` and `load_facet` (Task 22 and Task 15b are not merged when this task runs: the tests use a **fake `IntroHandle`**; with the real stubs, `check_state` answers `Unknown`, shown as "may be out of date", and `load_facet` leaves Grants "not loaded"), `hacivat-ui-editor` read-only buffers with SQL highlighting, the tab kind registered by Task 1b.
- Produces: `open_ddl_view(workspace, profile, key, cx)`: a tab titled from a Fluent pattern with the object name as an isolated argument; read-only editor with the script; options strip (mock 10) bound to `GenOptions` (Qualification, Constraints placement, Definitions provided by server — hidden where the engine has no table definition, R-M4-24 — Grants, Owner), persisted per profile in `ProfileUserData`; Copy (raw text, `\n`), Open in console (same profile and schema, text inserted, not run), Refresh; elevation with a loading state when the object is below `Sources`; on open, `check_state` → "changed on the server since it was loaded" with Reload, or "no longer exists"; unsupported-kind notice; MySQL executable-comment warning strip.
- **Also produces:** the Grants option loads the `Grants` facet on demand (loading state; `Partial` notice); the "Definitions provided by server" option uses MySQL `SHOW CREATE` text (R-M4-37) and SQLite server text, and is hidden on PostgreSQL; the toolbar shows `ExplorerActionProvider` actions for `ActionSurface::DdlViewToolbar` (M7's "Edit source").

- [ ] **Step 1: Write the failing tests:**
```rust
#[gpui::test] fn view_ddl_elevates_with_a_fragment_only(cx) {}
#[gpui::test] fn shown_text_equals_create_script_bytes(cx) {}
#[gpui::test] fn options_change_rerenders_and_persists_per_profile(cx) {}
#[gpui::test] fn copy_puts_raw_sql_on_the_clipboard(cx) {}
#[gpui::test] fn open_in_console_inserts_without_running(cx) {}
#[gpui::test] fn stale_object_notice_and_reload(cx) {}
#[gpui::test] fn dropped_object_notice(cx) {}
#[gpui::test] fn unsupported_kind_notice(cx) {}
#[gpui::test] fn mysql_executable_comment_warning(cx) {}
#[gpui::test] fn text_identical_in_en_and_tr(cx) {}
#[gpui::test] fn buffer_refuses_edits(cx) {}
#[gpui::test] fn server_definitions_toggle_hidden_on_postgres(cx) {}
#[gpui::test] fn ddl_view_grants_toggle_loads_facet(cx) { /* fake IntroHandle */ }
#[gpui::test] fn toolbar_shows_provided_actions(cx) {}
```
- [ ] **Step 2–4:** as Task 24 (tests FAIL, implement per mocks 10–11, PASS, render and list shots).
- [ ] **Step 5: Commit** `feat(app): read-only DDL view with generator options, copy and open in console`.

**Review:** opus (gpui). Probes: localized text inside the SQL; server text re-qualified; a stale DDL shown without its notice; the editor accepting input.

## W5 — Facet loading, overlays, refresh UI, incremental conformance (G4)

### Task 15b: Facet loading and fragment introspection on a caller's session (R-M4-29, R-M4-30) (W5)

**Files:** see File ownership (T15b); ADR 0008 `## m4-t15b — facets and joined fragments`.

**Interfaces:**
- Consumes: Task 15's engine and tickets; Tasks 16b/17b/18 (`Introspector::facet`); Task 5's `MetaConn::join`.
- Produces:
  - `IntroHandle::load_facet(keys, FacetKind) -> RefreshTicket` (body in `engine/facet.rs`): single-flight per `(object, facet)`, runs `Introspector::facet` on a pooled metadata connection, publishes the facet as a side-table change (`CatalogChange::SetFacet`) **without touching the object's structure or generation of unrelated namespaces**; `Partial` and `Unsupported`/`NotApplicable` are published as such; a later `rel` change invalidates the loaded facet (Task 16b).
  - `hacivat_introspect::fragment::on_session(handle: &SessionHandle, ns: &NsKey, keys: &[ObjectKey], level: Level, cancel) -> BoxFuture<Result<NamespaceSnapshot, IntroError>>`: joins the caller's session (`MetaConn::join`), runs only `joinable` sections (`StateOf<Kind>`, the fragment `Retrieve*` sections, facet reads), returns a standalone snapshot that is **never published and never cached** (M7's drift check inside its apply transaction).

- [ ] **Step 1: Write the failing tests:**
```rust
// facets.rs (fakes)
#[tokio::test] async fn load_facet_publishes_grants_without_touching_structure() {}
#[tokio::test] async fn load_facet_is_single_flight_per_object_and_facet() {}
#[tokio::test] async fn partial_facet_is_published_as_partial() {}
#[tokio::test] async fn on_session_result_is_never_published_or_cached() {}
// [docker: pg17 (pr), mysql84] (docker_fragment_on_session.rs)
#[tokio::test] #[ignore = "docker"] async fn joined_fragment_sees_the_apply_txns_uncommitted_column() { /* a Job(DdlApply) session: BEGIN; ALTER TABLE hvm4_t ADD COLUMN c int; on_session(…) lists c; ROLLBACK by the job; the statement log shows no BEGIN/COMMIT/ROLLBACK/SET from the joined conn */ }
#[tokio::test] #[ignore = "docker"] async fn joined_pg_sections_return_type_names_not_format_type_text() {}   // R-M4-46
#[tokio::test] #[ignore = "docker"] async fn pg_grants_facet_loaded_through_the_handle() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-introspect --test facets`; `--no-run` for the Docker file. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Run the unit tests; `run-docker-tests.sh --set pr -p hacivat-introspect --test docker_fragment_on_session`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(introspect): on-demand facets and fragment introspection on a caller's transaction`.

**Review:** security-sensitive. Probes: a joined connection that sends any text but `joinable` pack sections; an `on_session` result reaching the catalog or the cache; a facet load that republishes structure.

### Task 26: Overlays and the system-catalog layer (HV-INTRO-16 consumer side) (W5)

**Files:** see File ownership (T26); ADR 0008 `## m4-t26 — overlays`.

**Interfaces:**
- Consumes: Task 9's `SnapshotView` layers; Task 23's `hacivat_meta::system::decode` and assets; Task 1a's `ProfileUserData` (types; its file I/O stays `hacivat_settings::profile_data`, reached by the app).
- Produces: `SystemCatalogs::open(dir)` (reads `manifest.toml`; the app passes the installed resources directory, tests pass `<repo>/assets/system-catalogs`; packaging per **PO-Q1**), `for_server(dbms, version)` with the exact-version rule (R-M4-6), lazy decode on first use and one shared `Arc` per file; the `system` layer of `SnapshotView`: a system namespace resolves from a live namespace only when the user chose "Introspect live" for it, else from the snapshot (`PRE_INTROSPECTED` flag, version label), else `Unknown`; a column or object missing from a snapshot used against a newer patch release resolves `Unknown`, never `Missing` (alg/02 HV-INTRO-16 edge 1); the `virtual` layer: user virtual FKs from `ProfileUserData.virtual_fks` and shipped catalog virtual FKs from `assets/system-catalogs/virtual-fks.toml` (rows hand-authored from the vendor documentation of `pg_catalog` and `information_schema`, each with a `source` field naming the documentation page section; reviewed), layered with `layer = Virtual`, hidden when a real FK with the same columns exists, kept and flagged `BROKEN` when the target is gone, matched by path when server ids differ (imported overlays); annotations keyed by `ObjectKey`. DDL producers ignore the virtual layer (Task 19's test).
- **Also produces:**
  - `SystemCatalogs::from_embedded()` behind feature `embedded-system` (only `hacivat-app` enables it): `system/embedded.rs` holds an `include_bytes!` table of the committed `assets/system-catalogs/*.hvsnap.zst` files (path from `CARGO_MANIFEST_DIR`) and the manifest; each file is decompressed and decoded lazily on first use for its version. It is the app default when the PO keeps Q1's recommendation B; `open(dir)` stays for tests and for option A.
  - **`crates/core/hacivat-meta/src/sql_view.rs`:** `sql_catalog::CatalogView` for `SnapshotView` over the live, system and virtual layers, with no allocation on hits for `relation`/`columns` (Arc-shared slices built per namespace generation); `functions` serves snapshot routines with `args = None` for name-only built-ins; `settings()` from the snapshot, else live (`ListSettings`), else `Unknown`; `row_estimate` from the `Statistics` facet; `UnknownReason::ServerDependent` passed through (R-M4-41).

- [ ] **Step 1: Write the failing tests:**
```rust
#[test] fn pg_stat_activity_columns_found_offline_with_pg_catalog_out_of_scope() { /* pg17 snapshot, no live namespaces: lookup pg_catalog.pg_stat_activity → Found; children(Column) contains "pid" */ }
#[test] fn pg17_snapshot_serves_17_11_with_its_version_label() {}
#[test] fn pg13_has_no_snapshot() { assert!(sys().for_server(DbmsKind::Postgres, "13.22").is_none()); }
#[test] fn mariadb_10_11_uses_its_own_snapshot_not_10_6() {}
#[test] fn mysql_9_has_no_snapshot() {}
#[test] fn sqlite_never_loads_a_snapshot() {}
#[test] fn newer_patch_missing_column_is_unknown_not_missing() {}
#[test] fn snapshot_is_decoded_once_and_shared() {}
#[test] fn live_override_takes_precedence_over_the_snapshot() {}
#[test] fn virtual_fk_is_on_the_virtual_layer() {}
#[test] fn real_fk_hides_a_duplicate_virtual_fk() {}
#[test] fn virtual_fk_to_a_dropped_table_is_kept_broken() {}
#[test] fn imported_overlay_with_other_server_ids_matches_by_path() {}
#[test] fn shipped_virtual_fks_resolve_against_every_pg_snapshot() { /* every row of virtual-fks.toml resolves Found on pg14..pg18 */ }
#[test] fn embedded_snapshots_decode_lazily_once_per_version() {}   // --features embedded-system
#[test] fn sql_view_relation_and_columns_found_offline() {}
#[test] fn sql_view_lookup_allocates_nothing_on_hits() {}   // counting allocator
#[test] fn sql_view_unknown_when_namespace_not_loaded() {}
#[test] fn sql_view_builtin_name_only_function_has_args_none() {}
#[test] fn settings_from_snapshot_offline() {}
// [docker: pg13] (in tests/system_catalogs.rs, ignored "docker")
#[tokio::test] #[ignore = "docker"] async fn pg13_introspects_system_schemas_live_at_names_level() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-meta --all-features --test overlay --test system_catalogs --test sql_view`. Expected: FAIL.
- [ ] **Step 3: Implement**; author `virtual-fks.toml` (every row cites its documentation section; no row is guessed).
- [ ] **Step 4:** Run the tests; the pg13 Docker test through the wrapper. Expected: PASS.
- [ ] **Step 5: Commit** `feat(meta): system-catalog layer from bundled snapshots and virtual foreign keys`.

**Review:** opus. Probes: closest-version fallback (must be exact); `Missing` from a snapshot gap; a virtual object reaching DDL; a shipped virtual FK without a source; `embedded-system` enabled by a crate other than `hacivat-app`; an allocation on a `relation`/`columns` hit.

### Task 27: Refresh-after-DDL, freshness and settings UI (W5, gpui)

**Files:** see File ownership (T27); ADR 0007 `## m4-t27 — metadata UI`.

**Interfaces:**
- Consumes: Task 21 (coordinator, sink), Task 22 (staleness), Task 24 (live explorer), the settings store (`shell::update_settings`, the single writer), `exec_ui/orchestrator.rs` (where a run is started).
- Produces: the sink installed per run (named lines in `orchestrator.rs`), so a `CREATE TABLE` run in any console appears in the explorer; the Ask prompt (mock 9) with the count, the broad-refresh notice for opaque statements, the stale badge for `never`; the stale-object tooltip and "no longer exists" notice; Settings → Metadata (mock 13): refresh after DDL (Always/Ask/Never), background check (Automatic/Off/5/10/30 min), cache limits per connection and total, "Clear cached metadata for this connection…" (confirmation; deletes the profile's cache files and republishes an empty snapshot, then revalidates if connected). The M3c carry for the first substantive touch of T42's file (m3c.md §8.1, T43b fix m5): `settings_ui::fixtures::show_section` (`settings_ui/mod.rs:834-838`, behind `test-support`) duplicates the navigation click handler; extract `SettingsPage::show_section` and call it from both.

- [ ] **Step 1: Write the failing tests:**
```rust
#[gpui::test] fn create_table_in_a_console_appears_in_the_explorer(cx) {}
#[gpui::test] fn ddl_in_an_open_pg_transaction_appears_after_commit(cx) {}
#[gpui::test] fn ask_preference_prompts_with_the_count(cx) {}
#[gpui::test] fn never_preference_shows_the_stale_badge(cx) {}
#[gpui::test] fn dynamic_sql_shows_the_broad_refresh_notice(cx) {}
#[gpui::test] fn clear_cache_confirms_deletes_and_revalidates(cx) {}
#[gpui::test] fn settings_write_through_update_settings_only(cx) {}
#[gpui::test] fn metadata_settings_fit_960x600_in_tr(cx) {}
```
- [ ] **Step 2–4:** FAIL → implement per mocks 9, 11, 13 → PASS; render shots, list changed approved PNGs.
- [ ] **Step 5: Commit** `feat(app): explorer refresh after DDL, stale notices and the Metadata settings`.

**Review:** opus (gpui). Probes: a second settings writer; the prompt blocking a run; a stale badge that never clears.

### Task 28 — Gate measurement G4: incremental refresh is correct on the ver/01 matrix (W5)

**Exit criterion (spec §6 M4):** "incremental refresh is correct on the ver/01 matrix (#pg-xmin, #pg-xmin-freeze with `pg_resetwal`, #mysql-is-times, #sqlite-schema-version)".

**Files:** `crates/core/hacivat-introspect/tests/docker_m4_matrix.rs` and its block in `tests/common/mod.rs`; artifacts `target/docker/t28-*.tsv` (one row per step: target, step, changed parents, queries issued, incremental == full).

**Oracle.** After every step, the **incremental** snapshot of the namespace, compared structurally with only `loaded_at`, `generation` and the namespace watermark ignored (server ids, per-object `StateMark`s, every prop and every child compared), must equal a **fresh `Full`** introspection of the same namespace at the same moment. Each step also asserts the step-specific fact from ver/01, so a test cannot pass by always running `Full`: the incremental pass's query log must show the expected *narrow* reads.

- [ ] **Step 1: Write the failing tests:**
```rust
// #pg-xmin — [docker: pg13, pg14, pg15, pg16, pg17 (pr), pg18]: the ver/01 script plus the PG spike's extensions on schema hvm4_xs, one incremental pass after each DDL
#[tokio::test] #[ignore = "docker"] async fn pg_xmin_matrix_incremental_equals_full() {
    // ver/01 steps (own wording): CREATE INDEX xi1; CREATE INDEX xi2; ADD COLUMN c; COMMENT ON TABLE; COMMENT ON COLUMN; GRANT table; GRANT column;
    // first trigger; second trigger; ALTER FUNCTION STABLE; CREATE OR REPLACE FUNCTION; SET DEFAULT; ADD CHECK; DROP INDEX xi2; DROP TRIGGER tr2;
    // DROP COLUMN c; INSERT; VACUUM ANALYZE.
    // PG spike extensions (49 steps in all): remove a table, column and index comment; enum ADD VALUE and RENAME VALUE; domain ADD/DROP CONSTRAINT
    // and SET DEFAULT; ALTER TYPE ADD ATTRIBUTE; CREATE OR REPLACE VIEW; REFRESH MATERIALIZED VIEW; rename a trigger function; DISABLE TRIGGER;
    // ATTACH and DETACH PARTITION; SET and DROP NOT NULL; ALTER COLUMN TYPE; index rename; CREATE and DROP TABLE; SET SCHEMA out of the namespace;
    // owner change; TRUNCATE; rename a column named by a check, a generated column and a view; rename a table referenced by an FK and a view;
    // rename an enum type and a composite type. Then DROP SCHEMA → the namespace is gone.
    // per step: incremental == full; the canary holds; and narrow-read facts: CREATE INDEX → x's indexes re-read, x's pg_class row not a
    // changed parent, no column section; COMMENT → only the comments pass reads x; comment removal → Comments and existence only;
    // DROP INDEX/TRIGGER → removed by existence only; ALTER FUNCTION/SEQUENCE → no column re-read; INSERT/VACUUM → zero retrieve queries.
}
#[tokio::test] #[ignore = "docker"] async fn pg_canary_no_xid_assigned_by_any_pass() {}            // ver/01 #pg-canary, every pg target
#[tokio::test] #[ignore = "docker"] async fn pg_identity_change_forces_full_reason_restart() {}  // injected previous watermark with another postmaster start time (a container restart is controller-only)
// #pg-xmin-freeze — [docker: pg17wrap] (cluster jumped past 2^31 XIDs by pg-wrap/init.sh)
#[tokio::test] #[ignore = "docker"] async fn pg_xmin_freeze_between_reads_zero_ancient_rows() {
    // 1. previous watermark W = 1000 with the current identity → incremental → Full with reason "wraparound" (span ≥ 2^31 − 2^20);
    // 2. next incremental from that Full's watermark → zero ancient objects re-read (BETWEEN 0 AND age(W));
    // 3. regression: the same pass rendered with "<=" would match > 0 pg_class rows (ver/01 measured 220 of 416 on its cluster; record ours).
}
// #mysql-is-times — [docker: mysql80, mysql84 (pr)]: the ver/01 ALTER list on table tt, one incremental pass after each
#[tokio::test] #[ignore = "docker"] async fn mysql_is_times_matrix_incremental_equals_full() {
    // CREATE; INSERT (no table re-read); ADD COLUMN (INSTANT); ADD INDEX (INPLACE); COMMENT; MODIFY type (COPY); RENAME COLUMN;
    // CREATE TRIGGER (caught by TRIGGERS.CREATED, CREATE_TIME unchanged); ADD COLUMN ALGORITHM=COPY.
}
#[tokio::test] #[ignore = "docker"] async fn mysql_same_second_ddl_caught() {}
#[tokio::test] #[ignore = "docker"] async fn mysql_global_time_zone_plus_three_markers_compare() {}
#[tokio::test] #[ignore = "docker"] async fn long_alter_straddling_a_pass_is_seen() {}   // [docker: mysql80, mysql84 (pr), mariadb106, mariadb1011, mariadb114] — F1 on the matrix
// MariaDB (R-M4-10) — [docker: mariadb106, mariadb1011, mariadb114]
#[tokio::test] #[ignore = "docker"] async fn mariadb_full_per_namespace_equals_fresh_full_after_each_alter() {}
// HV-INTRO-14 and HV-INTRO-11 on the matrix — [docker: pg17-tr, pg17 (pr)]
#[tokio::test] #[ignore = "docker"] async fn pg_tr_icu_no_lower_or_ilike_in_query_log() {}         // alg/02 HV-INTRO-14 test 1
#[tokio::test] #[ignore = "docker"] async fn catalog_bytes_identical_under_lang_tr_and_c() {}      // test 2: two child processes, LANG/LC_ALL tr_TR.UTF-8 and C, byte-identical queries and snapshot
#[tokio::test] #[ignore = "docker"] async fn revoke_usage_fires_mass_drop_guard() {}               // alg/02 HV-INTRO-11 test 7, REVOKE restored in a drop guard
```
- [ ] **Step 2 (#sqlite-schema-version):** no Docker; the gate cites Task 18's `schema_version_sequence_matches_ver01` and `same_schema_version_replaced_file_detected_on_reconnect_by_digest`; this task re-runs them with a per-step incremental-equals-full assertion in `crates/core/hacivat-introspect/tests/sqlite_intro.rs` (named block `// ## m4-t28-oracle`).
- [ ] **Step 3:** `cargo test -p hacivat-introspect --no-run`; then `run-docker-tests.sh --set full -p hacivat-introspect --test docker_m4_matrix` (C-6 has started `pg17wrap`; C-2 lets `hv_test` create the MySQL trigger). Expected: FAIL only where a defect exists; every failure is a finding routed to Tasks 16–18 by the controller (carries), never fixed by weakening the oracle.
- [ ] **Step 4:** PR-set wall time ≤ 5 min (pg17 + mysql84 subset); full-set addition ≤ 15 min. Record both.
- [ ] **Step 5: Commit** `test(introspect): incremental refresh conformance on the ver/01 matrix`.

**Review:** opus. Probes: an oracle that compares a snapshot with itself; a test that passes because every pass ran `Full` (check the narrow-read assertions); a global setting left changed after a failure.

## W6 — Gate measurements and shots

### Task 29 — Gate measurement G1: cached catalog shown in < 300 ms at the 160k-column scale (W6, gpui)

**Exit criterion (spec §6 M4; §3.3 "Cached catalog load (160k columns) < 300 ms", HV-MODEL-03):** "cached catalog shown in < 300 ms at the 160k-column scale".

**Files:** see File ownership (T29).

**Measurement definition** (recorded verbatim in `docs/benchmarks/m4-bench-notes.md` and `m4.md`):
- **Fixtures:** (a) synthetic: 15,000 tables, 160,000 columns, 15,000 PKs, 5,000 FKs, 20,000 indexes in one schema (Task 11's generator, seeded); (b) real: `hv_m4_big` on `pg17` (Task 16's fixture), introspected at `Structure` level, persisted, cache file copied out through `hacivat_testkit::artifact`.
- **t0:** the app's `CatalogHost` begins opening the profile's cache file (app start, profile already expanded in the restored workspace, server unreachable: `--offline` bench flag).
- **t1:** the first presented frame in which the explorer shows that schema's kind groups with their counts and the first visible table rows painted (frame-presented hook of the existing bench harness, `crates/app/hacivat-app/src/bench/{b1_real,b3}.rs` precedent).
- **Budget:** `t1 − t0 < 300 ms` p95 over 10 cold-process runs **on the PO's Mac (authoritative, L2)**, plaintext cache (the default for non-production profiles). **Also measured with the same t0, recorded and reported separately (not gating):** the encrypted variant (an encrypted production profile expanded in the restored workspace, R-M4-15), with the `CatalogKey` keyring share broken out; encrypted > 300 ms is a PO item **listed in the demo notes** (`po-demo-m4.md`), not hidden in `m4.md`. Also recorded: the core-only share from `persist_perf` (≤ 200 ms on CI as a relative signal), memory per column and per table, and §3.3 "≤ 1 s to an interactive workspace with a cached catalog" from process start.
- **D-038 evidence (R-M4-21):** cold `Names` and `Structure` times on `hv_m4_big` with the local Docker server, printed with the thresholds.

- [ ] **Step 1: Write the failing tests:** `catalog_cold_bench_emits_json_with_t0_t1_and_phases` (app; JSON schema shared with `xtask bench-report`; the JSON carries a `key_fetch_ms` phase), `catalog_cold_bench_encrypted_variant_reports_key_share` (app, fake keyring with a fixed delay), `bench_fixture_is_15k_tables_160k_columns` (meta), `[docker: pg17] big_catalog_cold_times_recorded_and_cache_artifact_written` (introspect), perf `load_160k_column_namespace_under_budget` (exists, Task 11).
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3: Implement** the `catalog-cold` bench mode as the app example of Task 1a's stub, the `b3`/`b1_real` precedent (`cargo run --release -p hacivat-app --features bench --example catalog_cold -- --fixture <cache-file> --runs 10 --offline`; the `hacivat` binary takes no bench flag), the docker fixture test, and the notes; rows reach `m4.md` through `cargo xtask bench-report --milestone m4` (`parse_milestone` already accepts `m4`, `xtask/src/bench_report.rs:62`).
- [ ] **Step 4:** Run on the CI runner class (relative baseline) and hand the PO-Mac command line to the gate (Task 33 Step 3). Expected: the bench runs and emits JSON; the CI number is recorded.
- [ ] **Step 5: Commit** `bench(m4): cached-catalog cold-load bench at 160k columns`.

**Review:** opus. Probes: t1 taken before paint; a warm OS file cache presented as cold (drop caches is not possible without root on macOS: the notes say "cold process, warm disk" explicitly); fixture smaller than stated.

### Task 30 — Gate measurements G2 and G3: snapshots regenerate reproducibly in CI; built-ins resolve offline per version (W6)

**Exit criteria (spec §6 M4):** "snapshots regenerate reproducibly in CI and completion resolves built-in functions and system views offline for every supported version" (R-M4-5 reading).

**Files:** see File ownership (T30). Core-only (no gpui build): the offline explorer test is Task 32's.

**G2 — reproducible in CI** (R-M4-45).
- The `minicat-verify` job (Task 23) brings up CI's own compose stack, runs `minicat --verify --twice`, then `docker compose -p hacivat-m2a down -v` and up again, and runs `--verify` a second time; both must report zero differences against the committed assets. `minicat --check` runs in `build-test` (offline). amd64 CI is authoritative for the bytes.
- **Pass:** **two `workflow_dispatch` runs on the `m4` tip** (each on a fresh CI stack; C-7) **plus one PR-triggered run**, all green; each log shows ten files compared byte for byte. The run ids go into `m4.md`.

**G3 — offline built-in resolution per supported version** (through `sql_catalog::CatalogView`, the trait M5's completion calls, and `ModelView`).
- [ ] **Step 1: Write the failing tests:**
```rust
// hacivat-meta/tests/offline_resolution.rs — no network, no Docker, no engine: a SnapshotView over an empty live catalog + the bundled snapshot only
#[test] fn offline_resolution_matrix() {
    for v in ["pg14", "pg15", "pg16", "pg17", "pg18"] {
        let view = offline_view(v);
        assert_found(&view, "pg_catalog.pg_class", KindGroup::Relations, &["relname", "relnamespace"]);   // ModelView and CatalogView::{relation, columns}
        assert_found(&view, "pg_catalog.pg_stat_activity", KindGroup::Relations, &["pid"]);
        assert_found(&view, "information_schema.columns", KindGroup::Relations, &["table_name"]);
        assert_routine_found(&view, "pg_catalog.now");                                                     // CatalogView::functions, args = Some(..)
    }
    for v in ["mysql80", "mysql84", "mariadb106", "mariadb1011", "mariadb114"] {
        let view = offline_view(v);
        assert_found(&view, "information_schema.TABLES", KindGroup::Relations, &["CREATE_TIME"]);   // ver/01 #mysql-is-times reads this column
        assert_found(&view, "mysql.user", KindGroup::Relations, &[]);                                // needs the complete set (hv_catalog)
        assert_routine_name_only(&view, "NOW");                                                       // FnSig.args == None (review I5)
        assert_routine_with_signature(&view, SYS_ROUTINE[v]);   // one sys routine with a real signature, picked from the snapshot and listed in the report
    }
}
#[test] fn version_specific_objects_resolve_only_in_their_version() {
    // For each adjacent pair (pg14→15, …, pg17→18, mysql80→84, mariadb106→1011→114): one system view or function present in the newer
    // snapshot and absent from the older one, picked by the implementer from a snapshot diff and listed in the report with the release-notes
    // reference; assert Found in the newer, Unknown (never Missing) in the older. The plan does not invent these names.
}
#[test] fn unsupported_version_is_not_served_from_another_snapshot() {}
```
- [ ] **Step 2:** Run them. Expected: FAIL until Task 26's layer is merged, then PASS.
- [ ] **Step 3:** Implement the helpers. No CI edit (the job is Task 23's).
- [ ] **Step 4:** The controller dispatches the runs of C-7 and records them; the task attaches the run ids and the ten-file comparison lines to its report.
- [ ] **Step 5: Commit** `test(m4): offline built-in resolution per server version; snapshot reproducibility evidence`.

**Review:** opus. Probes: a resolution test that reaches the network or an engine; a "version-specific" pair taken from memory rather than the snapshot diff; a MySQL built-in asserted with an invented signature; `--verify` comparing decompressed bytes only (it must compare the files).

### Task 31 — Gate measurement G5: generated DDL golden tests are byte-stable (W6)

**Exit criterion (spec §6 M4; §7.1 "Golden tests … generated DDL (byte-stable)"):** "generated DDL golden tests are byte-stable".

**Files:** see File ownership (T31).

- [ ] **Step 1: Write the failing tests:**
```rust
// hacivat-ddl/tests/golden_stability.rs
#[test] fn every_golden_renders_byte_identical_twice_in_one_process() {}
#[test] fn every_golden_renders_identically_with_shuffled_hash_seeds() { /* two child processes with different RandomState seeds via env */ }
#[test] fn golden_files_are_lf_only_and_end_with_one_newline() {}
#[test] fn goldens_identical_under_lang_tr_and_c() {}   // also runs in turkey-test
// hacivat-introspect/tests/docker_ddl_roundtrip.rs — [docker: pg14..pg18 (pg17 pr), mysql80, mysql84 (pr), mariadb106, mariadb1011, mariadb114]; SQLite in-process
#[tokio::test] #[ignore = "docker"] async fn pg_roundtrip_apply_introspect_render_is_identical() { /* apply create_script(M) in a scratch schema; introspect; render again → same bytes as the golden; model equal (alg/03 HV-DDL-07 test 10) */ }
#[tokio::test] #[ignore = "docker"] async fn mysql_roundtrip() {}
#[tokio::test] #[ignore = "docker"] async fn mariadb_roundtrip() {}
#[test] fn sqlite_roundtrip() {}
#[tokio::test] #[ignore = "docker"] async fn pg_rendered_scripts_parse() { /* via sql-oracle-pg (libpg_query); for MySQL/MariaDB/SQLite the server accepting the script is the parse check — hacivat-sql's own parser is M5 (alg/03 HV-DDL-07 test 11 adapted) */ }
```
- [ ] **Step 2:** CI block `golden-guard` (fenced `# ## m4-t31-golden-guard`): after the test run in `build-test` ×3 and `turkey-test`, `git diff --exit-code -- crates/core/hacivat-ddl/tests/golden` (a golden changed without `UPDATE_GOLDEN=1` fails CI).
- [ ] **Step 3:** Run all of the above; Docker through the wrapper. Expected: PASS; any round-trip difference is a finding routed to Task 19 (carries), never blessed away.
- [ ] **Step 4:** Confirm the three OS runners and `turkey-test` are green on a pushed branch (controller).
- [ ] **Step 5: Commit** `test(ddl): byte-stability guard and apply round trip for generated DDL`.

**Review:** opus. Probes: a golden re-blessed in this task; Windows CRLF; a round trip that compares models only (it must compare bytes).

### Task 32: Shots, cross-surface tests, DESIGN.md (W6, gpui)

**Files:** see File ownership (T32).

- [ ] **Step 1:** Add `Surface::M4` with every state of mocks 1–14 that Tasks 14, 20, 24, 25 and 27 listed (dark/light, EN/TR/en-XA/tr-XT, at the matrix sizes 1280 × 800 and 960 × 600, `shots.rs:445`: 16 PNGs per state), plus the two M3c shot carries (m3c.md §8.1, T43b fix m10: NOTES 24 "long name ellipsized, mark kept" and NOTES 22 "Re-run" over a live result with a truncated value, new fixtures, in `tests/shots/m3c.rs`). Report the projected CI render time: macOS renders the whole matrix in one process (~2.1 h for 4,860 shots at the base) and Linux in two shards (~2.5 s per shot), each under a 180-minute bound; if a job would pass ~150 minutes, the controller adds a shard (a `ci.yml` block) before the gate. Every M4 state stops what it starts (live-thread guard, C-45). The states include the 15,000-table tree, Turkish names, invisible-character badge, progress rows, offline strip, built-in group, stale badges, mass-drop dialog, Ask prompt, DDL view with each option and notice, scope editor with an error, settings section.
- [ ] **Step 2:** Cross-surface tests `tests/cross_surface_m4.rs`: (a) offline start → cached tree → connect → validation clears stale; (b) expand → View DDL → Open in console → run `CREATE TABLE` → explorer shows it; (c) user refresh after a simulated privilege loss → mass-drop dialog → Keep cached; (d) EN ↔ TR switch on every new surface keeps expansion and selection; (e) production profile: Open data runs without a confirmation and the trust signals stay drawn; (f) an encrypted production profile not expanded at start makes no keyring call; expanding it paints the cached tree. **Offline explorer (G3, moved from Task 30):** `tests/offline_explorer.rs` — `offline_explorer_shows_the_built_in_group_with_its_version_badge`, `offline_lookup_resolves_built_ins_without_a_connection` (embedded snapshots, no network).
- [ ] **Step 3:** Shot-diff against `baseline-m3c` once it exists, until then against `baseline-m3b` plus the m3c-listed changes (Global Constraints "Shots harness"); list **every changed approved PNG one at a time** with its reason (R-M4-18) in the report; sign once with `--list`; save the render as `baseline-m4` in this SDD folder for the PO.
- [ ] **Step 4:** Write the DESIGN.md section (Explorer Tree extended with metadata states, the DDL view, the scope editor, the metadata settings) from the approved mocks; PO review.
- [ ] **Step 5: Commit** `test(app): M4 shots and cross-surface flows; DESIGN.md explorer metadata states and DDL view`.

**Review:** opus (gpui). Probes: a trust signal clipped at 960 px in `tr`; a shot listed as unchanged that changed; DESIGN.md describing something the code does not do.

## W7 — Gate

### Task 33: M4 gate (W7)

**Files:** `docs/benchmarks/{m4.md,po-demo-m4.md}`; after PO sign-off, `baseline-m4` stays in this SDD folder, not in git (C-37).

**M4 exit criteria.** Each is PASS or FAIL in `m4.md` with evidence:

| # | Criterion | Evidence |
|---|---|---|
| E1 | **Cached catalog shown in < 300 ms at the 160k-column scale** (§6 M4 exit; §3.3) | Task 29 on the PO's Mac (p95 of 10, plaintext, gating); the encrypted variant with its keyring share and the CI number recorded and listed in the demo notes |
| E2 | **Snapshots regenerate reproducibly in CI** | Task 23 (`--verify` locally, double run per target; the C-1 fresh-stack run), Task 30 G2 (two `workflow_dispatch` runs on the `m4` tip + one PR run; run ids) |
| E3 | **Built-in functions and system views resolve offline for every supported version** (R-M4-5) | Task 30 G3 (`offline_resolution_matrix` incl. MySQL/MariaDB name-only built-ins and one `sys` routine with a signature, version-specific pairs), Task 26, Task 32 offline explorer |
| E4 | **Incremental refresh is correct on the ver/01 matrix** (#pg-xmin, #pg-xmin-freeze with `pg_resetwal`, #mysql-is-times, #sqlite-schema-version) | Task 28 artifacts (49 PG steps on 13–18, `pg17wrap`, MySQL incl. the long-ALTER case); Task 18 |
| E5 | **Generated DDL golden tests are byte-stable** | Task 19 goldens, Task 31 (three OSes, turkey-test, round trip) |
| E6 | Explorer scope of §6 M4: tree, filters, refresh after DDL (PG, MySQL, SQLite incl. explicit transactions), cached catalog on connect, mass-deletion guard | Tasks 14, 20, 21, 24, 27, 32 |
| E7 | Locale invariance of catalog work (HV-INTRO-14): pg17-tr, `LANG` tr/C byte identity, turkey-test; MySQL non-ASCII keys never `Missing` before pinning | Tasks 3, 12, 17, 28 |
| E8 | Introspection never writes and never rides a user transaction: `#pg-canary`; DML **and DDL** refused on every engine's metadata connection (MySQL/MariaDB read-only session); joined connections refuse non-`joinable` and transaction-control text | Tasks 5, 15b, 28 |
| E9 | Cache safety: encryption, corruption handling, secrets scan, fuzz targets clean for one dispatched `fuzz-meta` run | Task 11, Task 23, C-7 |
| E10 | Shots: 0 violations; only listed changes; DESIGN.md section approved | Task 32; PO |
| E11 | Gates green: clippy (plain and `--all-features`), the full test set (perf lines included), `check-deps` (new rules incl. `MetaQueryOnlyFromPacks`), `l10n check`, `tokens --check`, the generator `--check`s incl. `minicat --check`, `cargo deny`, CI (`build-test` ×3 and `turkey-test`, both with Task 31's golden-guard step and Task 1a's `hacivat-meta --all-features` step; `docker-pr`; and dispatched `docker-full`, `minicat-verify`, `fuzz-meta`, `bench`, `pseudo-screens`) | Step 2 |
| E12 | Budgets: first frame ≤ 500 ms, idle RSS ≤ 200 MB (`--smoke` ×3), ≤ 1 s to an interactive workspace with a cached catalog; installer size delta from the snapshots recorded per OS (PO-Q1; dispatched package job) | Step 3 |

- [ ] **Step 1:** W6 merged; clean every workspace member in `<main>/target`; if `icu_casemap` in `sql-ident` stayed unused (R-M4-41 pinned ASCII-only), remove it (the one listed manifest edit) and record it.
- [ ] **Step 2:** Run clippy, every line of the full test set, the shot-diffs, `check-deps`, `l10n check`, `tokens --check`, the generator `--check`s, `cargo deny`. Expected: all green. The controller pushes `m4`, dispatches the C-7 jobs and records CI.
- [ ] **Step 3:** On the PO's Mac: Task 29's `catalog-cold` (10 runs, plaintext and encrypted with the keyring share), `--smoke` ×3, the 1-second interactive-workspace check; record machine, build hash and captures in `m4.md`; record the packaged size with and without the snapshots for macOS, Windows and Linux (dispatched package job).
- [ ] **Step 4: Write `po-demo-m4.md`** (pre-run by the gate agent, then by the PO with Docker up): 1. start offline with a cached **dev** PG profile expanded → tree in well under a second, "Showing cached metadata from …"; 1b. a cached **production** (encrypted) profile expanded → the keychain is read once, the tree follows; the measured encrypted time is written next to this step (E1); 2. connect → stale badges clear without a full re-read; 3. expand `hv_m4_big` (15,000 tables) and type a filter in TR (`istanbul` finds `İstanbul`); 4. View DDL of a table with a Turkish name → quoted correctly; toggle qualification; Copy; 5. in a console `CREATE TABLE demo_m4(a int)` → appears; `BEGIN; CREATE TABLE demo_tx(a int);` → not yet; `COMMIT` → appears; 6. `ALTER TABLE … RENAME` → the expanded node stays expanded; 7. revoke `USAGE` on a schema from the demo login in another tool, press Refresh → the mass-deletion question → Keep cached; 8. a MySQL 8.4 profile: add a trigger in another client, Refresh → trigger appears; View DDL with "Definitions provided by server" → the server's `SHOW CREATE` text; 9. the Built-in group on PG 17: `pg_stat_activity` columns listed offline with the version badge; 10. Settings → Metadata; 11. the scope editor with a pattern error in TR; 12. EN ↔ TR on every new surface.
- [ ] **Step 5: Write `m4.md`:** E1–E12; rulings R-M4-1..50 for PO confirmation (especially R-M4-5, -6, -13, -14, -15, -18, -20, -37, -38, -41, -43, -47); the PO questions with their answers or "default in force"; the controller steps C-0 … C-9 with their ledger lines; the carried items; the M4b plan pointer (R-M4-19).
- [ ] **Step 6:** After the PO demo: controller step C-9 (drop `hv_m4_big` through the wrapper with `HACIVAT_M4_CLEANUP=1`, record the PG volume size before and after; `down -v` of the PG services only with the PO's OK); C-8 for any spike worktree still present; no `target-core/m4-*` directory left.
- [ ] **Step 7: Commit** `docs(m4): gate record and PO demo checklist`.

---

## Self-review notes

**Spec coverage (§6 M4).** Every requirement the milestone names has an owner:

| Requirement | Tasks |
|---|---|
| HV-MODEL-01 arena, identity, kinds | T1a, T3 |
| HV-MODEL-02 snapshots, tri-state `ModelView`, search | T1a, T9 |
| SQL-facing `sql_catalog::CatalogView` (M5's layer, R-M4-5) | T1a, T26, T30 |
| HV-MODEL-03 cache persistence, versioning, encryption (D-016), fingerprints | T1a, T11, T15 (key via hub), T20 (flag), T24 (start-up timing) |
| HV-INTRO-01 levels, orchestration (D-038), tickets, `CatalogHub` | T15, T20 (pills), T29 (evidence) |
| Facets: grants, dependencies, statistics (R-M4-30) | T1a, T15b, T16, T16b, T17, T17b, T18, T19, T25 |
| HV-INTRO-02 scope grammar | T12, T20 |
| HV-INTRO-03 filters, pushdown | T12, T14 (view filter), T24 ("hide like this") |
| HV-INTRO-04 query packs (multi-file, `joinable`, `show-create`, system mode) | T4, T16–T18 |
| HV-INTRO-05 PostgreSQL incremental (exact age bound, dependency hop, comment existence) | T16, T28 (G4) |
| HV-INTRO-07 MySQL incremental (marker or window; MariaDB full) | T17, T28 (G4) |
| HV-INTRO-08 SQLite change detection | T18, T28 (G4) |
| HV-INTRO-11 existence diff, mass-deletion guard (D-039) | T10, T24, T28 |
| HV-INTRO-12 refresh after DDL | T6, T21, T27 |
| HV-INTRO-13 composite assembly, implicit objects | T13, T16–T18 |
| HV-INTRO-14 identifier folding, catalog predicates | T3, T4 (lint), T12, T17 (pinning), T28 |
| HV-INTRO-15 up-to-dateness, staleness, raw stamps | T16–T18 (state sections), T22, T25, T27 |
| HV-INTRO-16 system-catalog snapshots and overlays (L7, D-017, `xtask minicat`) | T23, T26, T30 (G2, G3) |
| HV-DDL-07 producers (view-only), quoting | T7, T19, T25, T31 (G5) |
| HV-DDL-09 constraint placement | T7 |
| HV-DDL-10 schema qualification | T7, T25 |
| HV-DDL-11 generator options (core subset) | T7, T25 |
| Explorer tree, filters, refresh after DDL, cached catalog on connect, mass-deletion guard | T8, T14, T20, T21, T24, T27, T32 |
| Extension points for M4b/M5/M6/M7 (R-M4-19, R-M4-34) | T1b, T14, T24, T25 |
| Exit: < 300 ms at 160k columns | T11 (early signal), T29 (G1) |
| Exit: reproducible snapshots in CI | T23, T30 (G2), C-1, C-7 |
| Exit: offline built-in resolution per version | T26, T30 (G3), T32 |
| Exit: incremental refresh correct on the ver/01 matrix | T28 (G4), T18 |
| Exit: byte-stable DDL goldens | T19, T31 (G5) |

**[Unknown]s carried into tasks (no research covers them; each task resolves or records):** stability of PG `information_schema`, MySQL `performance_schema`/`sys` content across fresh containers (C-1 before W1, then T23; R-M4-17); snapshot sizes and the installer delta (T23, T33, PO-Q1); MySQL server folding of non-ASCII column names (T17 probe, R-M4-41) and of lctn ≥ 1 table names (no service; stays `ServerDependent`); MariaDB's error code for DDL on a read-only session (T5); whether every MySQL `SHOW CREATE` form runs on the typed path (T17, R-M4-37); PG hot-standby incremental (R-M4-9, not tested); PG mass-DDL threshold (25 %, T16 records; R-M4-48); `VACUUM FULL pg_class` effect on catalog xmin (alg/02 HV-INTRO-05 edge 18: harmless re-reads only, not tested); the refresh-after-DDL default in other tools (R-M4-20); whether the 300 ms budget holds with encryption on (T29 records it, E1).

**Process lessons applied from M3** (ledger): 2 gpui slots planned from the start and DEBUG=0 profiles (ledger 876, 902); interface work split in three (T1a/T1b/T1c, P-M3b-20) with an explicit rebase point for typed signatures; per-task targets deleted on merge and spike worktrees removed (C-8); shots diffed against approved baselines with every changed PNG listed (T14, T20, T24, T25, T27, T32); keymap `HOST_ONLY` and short-form label fallbacks on Linux/Windows CI (carries-w9/w11, ledger 869); carries files per wave; opus reviews with two independent reviews where a task guards safety (T5, T11); merged-tree check after every merge and a tail diff after append-block conflict resolutions (ledger 574); Docker only through the wrapper, with every container or volume change a controller step that needs the PO's OK; "nightly" evidence by `workflow_dispatch` (ledger 89, 93); golden/fixture files `-text` against CRLF.

---

## Changelog (v2 vs `plan-m4-draft1.md`)

**Structure.** The "Amendments from the reconciliation" section (A-1..A-27) and every inline `[Amended …]` note are folded into the task text; "the note wins" is gone. Superseded wording (postcard, blake3, `compact_str`, `IdentKey::for_dialect`, `ServerCtx`, `KindProps`, `RenderCtx` in packs, `NsKey { database, schema }`, `Ref.resolved`, server-id-first `ObjectKey` equality, `packs/<dialect>.hvq.sql`, `testing::` fixtures, the reverse-index re-resolution, `minicat::{normalize, write_snapshot, read_snapshot}` in `hacivat-introspect`) no longer appears in any checklist; the original wording and reasons stay in `reconciliation.md` and in the draft file. Task counts: **34 → 38 tasks** (new T1c, T15b, T16b, T17b), 8 waves (W0–W7) unchanged, **10 controller steps** (C-0 … C-9) added, rulings **R-M4-1..36 → R-M4-1..50**, PO questions **17 → 22**.

**Review findings** (`plan-m4-review.md`): all 33 addressed; one accepted in a different form (C3), one partly reassigned (I14), none rejected outright.

| Finding | Change |
|---|---|
| C1 MySQL read-only txn does not stop DDL | R-M4-7 rewritten; Task 5: `SET SESSION TRANSACTION READ ONLY` at connect and after reconnect, asserted before every snapshot (`ReadOnlyNotEnforced`); tests split into `mysql_meta_txn_refuses_dml_1792`, `mysql_meta_session_refuses_ddl`, `mariadb_meta_session_refuses_ddl` (+ `read_only_flag_is_restored_after_reconnect`); joined path enforced by `MetaQuery` (R-M4-29); Architecture, Global Constraints, Review Focus 6, E8. |
| C2 no step produces complete MySQL/MariaDB snapshots; fresh-container [Unknown] answered last | Controller steps **C-1** (dispatch `minicat-verify` on `m4-spike-minicat` by `workflow_dispatch` before W1; artifact → T23 carries) and **C-2** (recreate only the MySQL/MariaDB services' volumes after T1c, **needs PO OK**, PO-Q18); T23 unblock "complete MySQL/MariaDB set available"; R-M4-43 fallback. |
| C3 spike rebase collides with T1a/T9/T11 | **Accepted in the review's alternative form** (R-M4-40): copy per owning task, no rebase or cherry-pick. Reason: importing `d86313b` into T1a would land real `persist`/`snapshot` bodies that T1a must disable to keep the stub list true, then T3/T9/T11 re-enable them. Reviewer probe "spike code reviewed as new code" added to T3, T8, T9, T11, T16, T17, T18, T23; cleanup C-8. |
| I1 T1a too big | Split into **T1a** (manifests, core and SQL types, check-deps, xtask/fuzz, ADR) and **T1c** (session, settings, secrets, `sql-safety`, exec slot, introspection stubs, Docker fixtures, `docker.rs`), parallel on T1a's manifest commit, with a rebase point for typed signatures (T1b too). |
| I2 manifest list stale | Exhaustive per-crate Step 1 table (crate, dependency, features, why, user); "Step 3 table" → "Step 1 table"; postcard/blake3/compact_str removed everywhere; zstd deny evidence; `embedded-system` feature. |
| I3 superseded text in checklists | v2 rewrite (this document). |
| I4 minicat read vs format 1; probe vs R-M4-7; decode mapping unspecified | R-M4-44 (system-mode sections `40-system` owned by T16/T17; generator-only probe in `tests/minicat_gen.rs`, the single R-M4-7 exception); T23 format-1 source table and the `system::decode` mapping; `serde_json` in the manifest. |
| I5 MySQL functions not proven offline; `FnSig` cannot say name-only | `FnSig.args: Option<Vec<FnArg>>` (X-8); `offline_resolution_matrix` asserts `NOW` name-only and one `sys` routine with a signature. |
| I6 false `Missing` for non-ASCII MySQL names; no owner for adoption | R-M4-41 (`ServerDependent` → `Unknown`); test `mysql_non_ascii_cased_column_is_server_dependent_until_pinned`; adoption owned by T17 (fenced block in `sql-ident/src/key.rs`); `icu_casemap` pre-added to `sql-ident`. |
| I7 SQLite case test contradicts ver/01 | T3 and T18 assert `Müşteri` = `müşteri`, `MÜŞTERİ` ≠ `müşteri` (also MySQL/SQLite spike F6). |
| I8 SQLite refresh in an explicit txn never fires | R-M4-42; T21 tests `sqlite_ddl_in_explicit_txn_appears_after_commit`, `sqlite_rolled_back_ddl_queries_nothing`; compensating refresh removed. |
| I9 `SHOW CREATE` needs identifier splicing | R-M4-37 (`#{ident:…}` only in `show-create` sections, through `sql_ident::quote`; lint and hostile-name tests in T4/T17); PO-Q21 offers the synthesize-only alternative. |
| I10 encrypted caches not in E1/start-up/demo | R-M4-15 key timing; T24 tests `encrypted_profile_not_expanded_makes_no_keyring_call_at_start`, `encrypted_expanded_profile_paints_after_key_without_blocking_ui`; T29 encrypted variant with keyring share; demo step 1 uses a dev profile, step 1b the encrypted production one with its measured time; PO-Q19. |
| I11 "nightly" cannot run on `m4` | R-M4-45; Global Constraints "Full test set"; G2 pass = two `workflow_dispatch` runs + one PR run; C-7. |
| I12a T12 uses T9's view in the same wave | T12 tests filter roles against a fake view; end-to-end stays in T24. |
| I12b third gpui build in W6 | `offline_explorer.rs` moved to T32; T30 core-only. |
| I12c three writers of `profiles/<uuid>.toml` | `hacivat_settings::profile_data` is the only writer (T1c, with the 0600 test); `hacivat_meta::ProfileUserData` types only; `explorer::userdata` owned by T20 only. |
| I12d T25 vs T22 same wave | T25 tests use a fake `IntroHandle`. |
| I12e `pg17wrap` vs `check-pins` | `xtask/src/docker.rs` (named lines) in T1c. |
| I13 joinable refusal unenforceable | `MetaQuery<'a> { sql, binds, joinable }` (T1c), built only by the pack renderer (check-deps source rule `MetaQueryOnlyFromPacks`); joined connections refuse non-`joinable` queries and transaction-control/`SET` text; tests in T5 and xtask. |
| I14 T15/T16/T17 too big | **T16b/T17b** (W4: grants, dependencies, roles, their Docker tests) split out as recommended. **T15b** (W5) takes `load_facet` scheduling and `fragment::on_session` instead of the hub: the review proposed moving `CatalogHub` to T15b in W4, but T24 (W4) wraps the hub, which would create a same-wave dependency; the hub stays in T15 and the facet/joined-fragment work, whose consumers tolerate "not loaded", moves. |
| I15 big fixture on five servers and in the PR set; no cleanup | `hv_m4_big` tests `[docker: pg17]`, full set only; batched build/drop (250 tables per `DO`); `drop_big_fixture_when_asked` + C-9 (volume size before/after; `down -v` only with PO OK, PO-Q20). |
| I16 pending spikes not a dispatch gate | Both introspection spike reports landed and are folded here (Tasks 5, 13, 16–18, 21–23, 28); no spike gate remains. |
| M1 stale PO text and counts | `po-questions-m4.md` rewritten (Q1 default B, counts, ranges); rulings range R-M4-1..50 in T1a/T33; coverage table extended. |
| M2 publish budget vs spike timing | T9 perf publishes prebuilt `Arc`s only; index build reported separately. |
| M3 PG `search_path` not pinned | R-M4-46; T5 `SET LOCAL search_path = pg_catalog, pg_temp`; joinable sections avoid `format_type` text (also PG spike §6). |
| M4 `.gitattributes` glob | `packs/**/*.hvq.sql`. |
| M5 opaque-effect scope | T6 scope bullet; ADR 0005 records `SELECT f()` running DDL as undetected. |
| M6 `FakeMetaSource` in `hacivat-meta` tests | `offline_resolution.rs` builds a `SnapshotView` with no engine; `MetaBelowIntrospect` forbids the dev edge too. |
| M7 `pg-wrap/init.sh` order; stale tags | Role and databases created before `pg_resetwal`; C-6 removes the old tag on a rebuild. |
| M8 spike leftovers | C-8 (worktrees and targets of all four spikes). |
| M9 Task 2 approval is a hard wait | T2 Step 3 fallback (controller rules after one wave). |
| M10 `FakeIntrospector` has no owner | Module stub T1c, body T15. |
| M11 `include_bytes!` in every binary | Feature `embedded-system`, only `hacivat-app` (check-deps `EmbeddedSystemOnlyFromApp`). |
| M12 19 vs 20 objects boundary | `nineteen_objects_all_vanishing_is_not_guarded`, `twenty_objects_eleven_vanishing_is_guarded`. |
| M13 volatile exclusions not visible | `manifest.toml` `excluded = [...]`. |
| M14 probe pairs without collation | `@@character_set_system` and the column-name collation recorded next to each pair. |

**MySQL/SQLite spike** (`spike-mysql-sqlite-intro-report.md`):

| Finding | Change |
|---|---|
| F1 `CREATE_TIME` = ALTER start on MySQL | R-M4-39 marker-differs-or-window rule; table stamp `StateMark::MyTable`; tests `long_alter_straddling_a_pass_is_seen` (T17, T28). |
| F2 snapshot hides later DDL on MySQL only | Watermark read inside the transaction; `snapshot_hides_later_ddl_and_next_pass_catches_it`; MariaDB rows without a listed parent skipped. |
| F3 MariaDB `CREATE_TIME` like MySQL | R-M4-10 kept with the new evidence; v1.x note. |
| F4 `hv_test` cannot `CREATE DATABASE` | `hvm4\_%` wildcard grant (T1c, C-2); `hvm4_<test>` databases; fallback in R-M4-43. |
| F5 1419 on functions/triggers | `--log-bin-trust-function-creators=1` (T1c, C-2). |
| F6 SQLite `Müşteri` = `müşteri` | T3/T18 tests (with I7). |
| F7 read-only authorizer blocks pragmas/`BEGIN` | **R-M4-38**: the metadata connection is `mode=ro` without the authorizer; the guard allowlist is unchanged; temp reads use allowed pragmas only. |
| F8 `data_version` is not a schema signal | Not used (R-M4-38, T18). |
| F9 engine differences | T7/T13/T17/T19 keep flavor texts verbatim, client-side key-part grouping (no `GROUP_CONCAT`, no `group_concat_max_len`, lint), routine identity, invisible indexes, MariaDB sequences; X-10. |
| F10 SQLite shapes | T18 two bulk queries with per-relation fallback and `BROKEN`; CHECK/generated only in server SQL → SQLite DDL is server text (R-M4-24, T19). |
| `SET GLOBAL` needs SUPER | `hv_fixture` login (T1c, C-2). |

**PG spike** (`spike-pg-intro-report.md`): finding 6 → R-M4-8 exact `age(x.xmin) BETWEEN 0 AND age($W::xid)` plus the 64-bit span check (replaces `txage + 64`); finding 3 → `ListExistentComments`; finding 4 → `ChangedDependents` (database-wide one-hop `pg_depend`) and the same hop in `StateOf`/`check_state` (T16, T22, §7 Q2); finding 5 → facet cascade (`rel` re-reads indexes, constraints, triggers); §6 → `search_path` pinned in `begin_snapshot` (R-M4-46, T5); §3 → PG 13 tested in the full set but not supported (R-M4-47, PO-Q22) and the 17+/18 version gates; finding 7 → `pg17wrap` still required for the `pg_resetwal` run (T28, C-6); §5 → fixture built and dropped in batches of 250 (T16, C-9); finding 8 → one retry on `XX000` (T15); §7 Q3 → schema rename forces `Full` (R-M4-48); §7 Q4 → unmodelled PG kinds in Deferred, `ddl-policies-not-loaded`; §7 Q5 → 25 % mass-change rule (recorded, not measured); the 49-step matrix → T28.

**Tree spike** (`spike-tree-report.md`, `m4-spike-tree` @ `2a31b8b`): Task 1b's tree API follows the spike — `roots() -> Arc<[TreeKey]>` instead of `root_count()`, labels as `SharedString` + `TextSource` instead of `RenderedText`, `icon: Option<IconName>`, `flags: Vec<RowFlag>` (no `smallvec` in `hacivat-ui`), the status slot as its own field `status: Option<RowStatus>` with a required text alternative (replaces the reconciliation's `RowFlag::Status(StatusGlyph)`; test renamed `status_slot_requires_a_text_alternative`), `search_text`, `set_expanded_keys`, `update_model`/`model_changed`, `scroll_by`; Task 8 copies the bodies (R-M4-40), splits `view.rs`, keeps the spike's measured perf as gates (162,041 rows: flatten 1.77 ms vs < 50 ms, row build p95 0.32 ms vs < 2 ms, with and without AT) and its ADR 0007 items (flat vector, Fibonacci hasher, anchor by key, lag-free a11y, border-strip finding); concern 1 → **R-M4-49** (group headers focusable, never selected; test `group_header_rows_take_focus_but_never_selection` replaces `…_are_skipped_by_arrow_keys`); concerns 2, 3, 6 → **R-M4-50** (type-ahead fold per D-074, set by Task 14; single selection in M4; CJK type-ahead deferred); concern 9 → `tests/tree_fixture/mod.rs` owned by T8; C-8 removes the spike worktree.

**Other:** PO preferences recorded (Turkish copy keeps "transaction/commit/rollback" in English; VS Code-style default shortcuts, remappable — T1b keymap table rewritten from the DataGrip preset); the minicat CI job moved from T30 to T23 (it is the spike's); X-8..X-10 cross-plan rows added.
