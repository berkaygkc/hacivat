# M3a "Run a query" Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The first half of M3, the walking skeleton. A developer opens a console on a PostgreSQL, MySQL/MariaDB or SQLite connection, types SQL in a real editor (IME, undo, clipboard, lexical highlighting), sees the exact range Run will execute, runs it, and browses typed rows in a virtualized grid that holds 1M × 50 rows from PostgreSQL at B1 speed in the `tr-TR` format locale. Production-tagged consoles are read-only until M3b's confirmations land (R-M3-6 REVISED).

M3 is split like M2 (ledger "SPLIT DECISION"). This plan is **M3a**. M3b "Safe and complete" (params, classifier, confirmations, the rest of the grid and the transaction UI) is planned while the M3a gate is PO-pending. The "Deferred work" section lists everything that is not here, with its owner and reason.

**Architecture:**
- **Four streams, one per crate family** (spec §6 "How the work runs"), meeting only through the seams that Task 1b commits:
  - **SQL text** (`crates/sql/*`, Core layer): `sql-text`, `sql-dialect`, `sql-lexer`, `sql-split`, `sql-oracle-pg` and the dev-only `sql-testkit`. A total, lossless, table-driven lexer; a token splitter with editor and execution modes (execution never uses the heuristic split, D-06); `SqlDocument`; caret/selection → execution units with the D-011 chooser and its goldens. `sql-oracle-pg` wraps libpg_query (`pg_query =18.1.0`, vendored under `third_party/pg_query` without protobuf regeneration, R-M3-1), which M3a uses to resolve PostgreSQL ambiguous boundaries and to prove the C build on three OSes.
  - **Editor** (`crates/ui/hacivat-text` headless, `crates/ui/hacivat-ui-editor` element, `crates/app/hacivat-sql-editor` adapter): rope buffer with byte offsets canonical, change sets and anchors, grouped linear undo, single-cursor commands, IME through `EntityInputHandler`, clipboard, L1 lexical highlighting, gutter execution marks, the Run preview band, an accessibility baseline and two large-file tiers.
  - **Execution and results** (`crates/core/hacivat-values`, `hacivat-results`, `hacivat-exec`, typed surfaces in the three drivers and the session actor): columnar chunks decoded by the drivers, an `ArcSwap`-published result store with caps and console cursors, a typed read-only guard path, a vendored `tokio-postgres` for portals (R-M3-18), and a run service that turns editor requests into one session request.
  - **Grid and workspace** (`crates/ui/hacivat-ui-grid`, `hacivat-values::valfmt`, `crates/app/hacivat-app`): a database-agnostic grid element with fast-scroll, L3 number formatting, an adapter over the store, result tabs, raw-TSV copy, the results tile with the cursor strip and output log, consoles bound to connections, the production hairline, and the capsule and status bar that follow the focused console.
- **Interface first, in two halves** (pre-flight Part 4). **Task 1a "Proofs"** lands the GPUI fork scheduler patch with its rev bump, the vendored `pg_query` with the libpg_query build proof on macOS, Windows and Linux, the `deny.toml` and notices changes, the CI libclang action and the teardown stress test. **Task 1b "Seams"** commits every cross-stream type and trait with fail-closed stubs (F31), every other manifest change, the check-deps rules and the vendored `tokio-postgres` scaffold. Task 2 mocks every new surface in parallel.
- **Two M3a-specific structural calls** (rulings below): the SQL adapter lexes **off the UI thread** in M3a (R-M3-22), so B3 can pass in the real app before M3b's incremental relex; and a unit may keep a **suspended cursor** only when the lexer shows a read head, with a rollback when the cursor does not complete (R-M3-23), so cursors work before the classifier exists.

**Tech Stack:**
- Everything from M2a and M2b; their manifests and rulings stand.
- SQL text: `text-size` 1.1, `phf` 0.13 (in the lock) with `phf_codegen` in `xtask`, `memchr` 2, `bitflags` 2, `smallvec` 1, `twox-hash` 2.1.5 (`xxhash3_64`, `xxhash3_128`; R-M3-4; never `xxhash-rust`, BSL-1.0), `pg_query = "=18.1.0"` (PostgreSQL 18.6 grammar; needs libclang and a C compiler; vendored under `third_party/pg_query`, its pre-generated protobuf code always used; R-M3-1), `proptest` 1.12 (dev).
- Editor: `ropey` 1.6.1 with `default-features = false, features = ["cr_lines", "simd"]` (no `unicode_lines`: U+2028 and U+0085 are not line breaks), `unicode-segmentation` (in the workspace), `unicode-width` 0.2, `unicode-ident` 1, `lru` 0.18, `slotmap` 1 (Zlib, allowed).
- Execution: `tokio-postgres` 0.7.18 vendored under `third_party/tokio-postgres` with patch §1 (R-M3-18), `postgres-types` 0.2.14, `mysql_async` (the M2b vendored copy), `rusqlite` 0.42 with `column_decltype`, `bytes` 1, `arc-swap` (in the workspace).
- Grid: ICU4X 2.4 (`icu_decimal`, `fixed_decimal`, `icu_locale`, already in the workspace) and `jiff` (in the workspace) for display; no new grid crate dependency.
- Exact versions are what Task 1b's manifest table (with Task 1a's rows) pins; `Cargo.lock` is the record. Any other crate needs a ruling.

**Spec:** `docs/superpowers/specs/2026-11-12-hacivat-v1-design.md`. The plan argues from it; executors read both.

**Execution (PO mandate 2026-11-13 and 2026-11-14: roll on, take rulings, never wait on the question list):**
- Subagent-driven, in parallel waves of 3–5 worktree agents. Branch **`m3a`** is cut from the m2b tip **`5d5832f`** (the merge worktree; its PR CI run 43512432001 is green: 14 passed, 8 skipped by trigger). Task branches are `m3a-<slug>` based on `m3a`. No merge to `main` without the PO.
- The controller commits this plan into `docs/superpowers/plans/2026-11-14-m3a-run-a-query.md` on `m3a`, copies the M2b implementer and reviewer rules into the M3 SDD folder with the M3a paths **before Task 1a is dispatched** (P28), and keeps the ledger (`progress.md`) and the status-line file `~/.claude/ilerleme-hacivat.json` current.
- Every ruling the controller or a task takes is logged in the ledger with "why" and "cost if wrong", and lands in the gate's PO list.
- The pre-flight conflict scan (`preflight.md` in the SDD folder, findings P1–P28) ran over this plan (M2b precedent); its amendments are applied in this version, in one pass ("Pre-flight rulings P1–P28" below).
- **Before Task 1a is dispatched** the controller also copies the M2b render `<main>/target/shots-review-m2b/_render-m2b` (1,012 PNGs; inside `target/`, so a clean can delete it) to the git-ignored `<main>/.superpowers/sdd/2026-11-14-m3-walking-skeleton/baseline-m2b/`, records in the ledger the commit it was rendered from, and re-renders it at `5d5832f` if that commit differs (P17).

**Spec reading list** (every task brief cites the parts it needs):
- Decision log: **L1**, L2, **L3** (grid numbers by format locale), L4, L8, L13, **L15** (M2 split precedent; (d) format locale = OS region).
- §2.3 anti-goals (wrong-range Run, typing lag, toy feel), §3.1 (licensing, clean room), **§3.3** (budgets: B1, B3, open 2 MB, result memory), **§3.4**.
- §4 rows: "SQL editor component", "SQL language: lexer, splitter, parameters", "Execution", "Result store", "Grid", "Safety and read-only" with the "Safety model (normative summary)", "Production UX", "Accessibility", "i18n", "Keyboard and command palette".
- **§5.1** (layout; dependency rules 1–4), **§5.3**, **§5.4** (threading and bridge rules), **§5.5** (query → store → grid), §5.6 (consoles directory, history path), §5.7, §5.8.
- **§6 M3** (scope and exit), §7.1, §7.2; §10 C3, C10, C20.
- Register (`docs/decisions/open-product-decisions.md`): D-004, D-011 (and Appendix B #1), D-013, D-014, D-015, D-019, D-035, D-036, D-063, D-065, D-071, D-072, D-073, D-074.
- Algorithms: `alg/10` SQL-LEX-01..05, SQL-SPLIT-01..04 (with R-M3-5's errata); `alg/20` HV-EXEC-01, -02, -05, -07, -09, -11; `alg/21` HV-RES-01..05, -07, -08; `alg/22` HV-GRID-01..05, -10, -16 (role only); `alg/25` UI-EDIT-01..09, -11, -12, -14, -19, -21, -23, -24; `alg/26` INV-1..5, I18N-11; `alg/13` SQL-SAFE-05 (read-only pipeline only).
- `docs/research/datagrip/13-behavioral-test-corpus.md` §3.3 (fixtures), §3.5–§3.8 (SPLIT, CARET, SEL, CHOOSE) and ALG-TEST-01.
- ADR 0001 and its M0 addendum (B1, B3, IME numbers, findings 1, 6, 7), ADR 0002, ADR 0004.
- `DESIGN.md`, `.impeccable/design.json`, harman (`design/directions/harman/index.html`), `design/m2-connect/NOTES.md` (binding copy: "No extra confirmations" off production), `design/m2b-secure/NOTES.md`, and `design/m3a-query/` once Task 2 lands (binding).
- `docs/superpowers/plans/2026-11-13-m2b-secure-network-sessions.md`: its Global Constraints (inherited, restated below) and its appendix of pre-flight rulings Q1–Q46 (lessons inherited; see "Inherited pre-flight lessons").
- The M3 ledger `<main>/.superpowers/sdd/2026-11-14-m3-walking-skeleton/progress.md` (rulings R-M3-1..21, the SPLIT DECISION) and the four digests in its `inputs/` folder. **The digests are background; where this plan differs, the plan wins.** Read them by absolute path: `.superpowers/` is git-ignored.

## Global Constraints

Every task's requirements include this section. `<main>` is `/Users/berkaygokce/Documents/hobby-projects/hacivat`. Line numbers cited in this plan are at the m2b tip `5d5832f` (the merge worktree `/Users/berkaygokce/Documents/hobby-projects/hacivat-m2b-merge`; the pre-flight checked them at `ffa78ea`, and `5d5832f` changes only `docs/benchmarks/m2b.md`).

- **Inherited.** The M2b Global Constraints apply unchanged, except where a line below replaces or extends one. They carry the M1 and M2a rules: GPUI pin through the fork, toolchain 1.98.1, no `unsafe` outside `hacivat-ui-core/src/platform/**`, ASCII-only casing (INV-3, `clippy.toml` `disallowed-methods`), the text and Fluent rules, semantic tokens only, the named design rules, F31 (stubs fail closed) and F9 (no comment-only or tautological test bodies), secrets, machine text, engines and versions, modal layers (Q34), Escape and paste in fields, key bindings, writes through their single owner, and rate limits.
- **Branches.** The feature branch is `m3a`. Each task works in its own worktree on `m3a-<task-slug>`. Its first commands are `git checkout -B m3a-<task-slug> m3a` then `git merge-base --is-ancestor m3a HEAD`; stop and report BLOCKED if either fails. Never `m3a/<slug>`. Implementers never push and never merge; the controller does both, including the GPUI fork scheduler commit that Task 1a prepares (P13). **Named exception:** Task 1b starts from Task 1a's first commit, not from `m3a` (Task 1b's Build line).
- **SDD files.** The ledger, `global-constraints.md` (an exact copy of this section) and the implementer and reviewer rules live in `<main>/.superpowers/sdd/2026-11-14-m3-walking-skeleton/`. Briefs cite them by absolute path. Report files go there too; if the write is refused, inside the worktree at the same relative path.
- **Commit trailer.** Every commit message ends with exactly:
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01N4f31X8x4esgKaXTQESQ1E
  ```
- **Disk and target directories (Q1, updated by the 2026-11-13 disk-hygiene rule).**
  - **One project target directory, `<main>/target`.** Task 1a, the controller's wave verification and the gate build there.
  - **A per-task directory only when parallel builds need it:** a task that runs while another agent builds uses `CARGO_TARGET_DIR=<main>/target-core/m3a-tNN` (gpui tasks included; Task 1b: `target-core/m3a-t01b`). The controller runs `cargo clean` in it after the task merges and deletes it; nobody leaves a `target-*` directory behind. No two agents ever share a target directory (path crates collide across worktrees).
  - **Admission (R-M3-50 as amended by P14: RAM and CPU bind before disk).** Before each dispatch the controller checks `df -h /System/Volumes/Data` and memory (`memory_pressure`, `vm_stat`): a gpui agent (any task that builds `gpui`: every `crates/ui/*` crate except `hacivat-text`, every `crates/app/*` crate) starts only with **≥ 25 GiB free**, a core-only agent with **≥ 8 GiB**, and no agent starts while swap is growing. At most **4 gpui agents at once while the Docker matrix is down, 3 while it is up, 2 whenever less than 60 GiB is free**; a further one waits for one to finish.
  - **`CARGO_BUILD_JOBS`.** Whenever 3 or more agents build at once, every agent exports `CARGO_BUILD_JOBS=4` (core agent) or `CARGO_BUILD_JOBS=6` (gpui agent); the brief names the value. Fourteen cores do not carry four default-width gpui builds plus the Docker VM, and the timing tests (`…within_1s`, the 10 s teardown watchdog) turn flaky under that load.
  - Every agent exports **`CARGO_INCREMENTAL=0`**. Before its final run each task runs `cargo clean -p <every crate it touched>` in its own directory. Nobody but the controller runs `cargo test --workspace`.
  - **When a shared target directory switches checkout or worktree path, clean every workspace member first** (`cargo clean -p <each member>`): cargo reuses artifacts across paths and `env!(CARGO_MANIFEST_DIR)` stays baked to the old path (M2b merge-gate lesson).
  - **Never delete `<main>/target/docker/`** (Docker artifacts and the lock wrapper live there); clean only `target/debug` and `target/release`.
  - The libpg_query C build (from the vendored copy) adds about 3 minutes and a few hundred MB per cold target directory; budget it. **Nobody builds the registry `pg_query`:** every build reaches it through `[patch.crates-io]`, which Task 1a's first commit puts into every later base (P1).
- **Dependencies.**
  - Task 1 writes every manifest change M3a needs (Task 1a its proof rows, Task 1b the rest; Task 1b's Step 3 table is exhaustive), `deny.toml`, `[patch.crates-io]`, `[workspace] members` and `exclude`, and two vendored copies under `third_party/` (excluded from the workspace; the `mysql_async` pattern): **`pg_query/`** (Task 1a: 18.1.0 pristine plus `PATCHES.md` §1, the protobuf regeneration removed from `build.rs`; R-M3-1, P1) and **`tokio-postgres/`** (Task 1b, pristine). No later task edits a manifest. **Named exception:** Task 12 edits files inside `third_party/tokio-postgres/src/**` and its `PATCHES.md`, each change fenced by `// HACIVAT PATCH §N`.
  - **Never run a plain `cargo update`.** Use `cargo update -p <crate> --precise <v>` when a bump is unavoidable and list it in the commit body. `Cargo.lock` gets only additions, except entries listed in the commit body (the vendored `pg_query` and `tokio-postgres` losing `source`/`checksum`; the GPUI rev move, made by editing `rev` in `Cargo.toml`, never by `cargo update`, P13).
- **Licences and clean room.**
  - The `deny.toml` allow-list is unchanged, plus **one crate-scoped exception: `PostgreSQL` for `pg_query`**, with a `[[licenses.clarify]]` naming its real licence set (R-M3-1). `THIRD_PARTY_NOTICES.md` ships in every bundle.
  - **No BSL-1.0 crate** (it is not on the list): hashing uses `twox-hash` (R-M3-4).
  - **No Zed GPL crate, ever** (`editor`, `ui`, `workspace`, `rope`, `text` stay banned). In the fork checkout `~/.cargo/git/checkouts/zed-*/<rev>/` agents read only `crates/gpui/**` and `crates/scheduler/**` (both Apache-2.0). **Nothing under `<main>/.refs/` is read, built or run.** DataGrip artefacts are never opened; corpus expectations come from Hacivat's specs (ALG-TEST-01).
  - `aws-lc-rs`/`aws-lc-sys` stay banned; `cargo tree -i aws-lc-rs --workspace` stays empty.
- **Layering (`cargo xtask check-deps`, extended by Task 1b).**
  - Only `hacivat-ui-core` reaches `gpui` or a zed-sourced crate (F1's dev-only `gpui` with `test-support` stays allowed in Ui and App crates). Other UI crates reach GPUI only through `hacivat_ui_core::gpui_surface`.
  - Ui crates never reach Core (`crates/core`, `crates/sql`, `crates/ai`); Core never reaches Ui. Only `crates/app/*` and `crates/bin/*` meet both.
  - New rules: `TextCoreNotHeadless` (`hacivat-text` reaches no `hacivat-ui-core`, `gpui` or zed crate), `SqlNotRuntimeFree` (no `crates/sql/*` crate reaches `tokio`, `futures`, `salsa`, `hacivat-l10n`, `gpui`, `async-std` or `smol` through normal edges), `SqlReachesCore` (no `crates/sql/*` crate depends on a `crates/core/*` crate through any edge, dev included), `BackendOutsideOwner` (`pg_query` is a direct dependency of `sql-oracle-pg` only).
  - `PRODUCT_CRATES` (`check_deps.rs:20`) gains `hacivat-ui-editor`, `hacivat-ui-grid`, `hacivat-sql-editor`, `hacivat-text` and `hacivat-ui`, so a normal dependency of theirs on a `test-support` or `scene-check` feature is caught. `SQL_FORBIDDEN` keeps `salsa` and `sql-sema` (inherited from `GUARD_FORBIDDEN`) for every M3a crate; ADR 0005 names M5's exemptions (`sql-sema`, `sql-ide`) now (P22).
  - `hacivat-values` has no `hacivat-l10n` dependency (spec §5.1); its `valfmt` uses ICU4X directly.
- **Security review.** Tasks **1a, 1b, 8, 9, 12, 13, 14, 15, 18, 22, 23, 24, 31 and 33** are security-sensitive (Task 24 because a preview that execution would not run is the §2.3 anti-goal that R-M3-8 rests on, P28): an **opus** reviewer runs the adversarial probes in the task's Review line and writes and runs at least one hostile test the implementer did not write.
- **ADRs.** Task 1 creates three (Task 1a creates 0005 for its proof record; Task 1b creates 0006 and 0007 and adds its own section to all three), each with Status, Context and a Rule heading naming the per-task section format `## m3a-tNN — <topic>`:
  - `docs/decisions/0005-sql-text-and-safety.md` (M3a: text layer and libpg_query; M3b appends the classifier);
  - `docs/decisions/0006-execution-and-results.md`;
  - `docs/decisions/0007-editor-and-grid.md`.
  Driver tasks append ADR 0002 sections; session and guard tasks append ADR 0004 sections; Tasks 1a, 1b and 13 append ADR 0005 sections; Tasks 1b, 23 and 31 append ADR 0006 sections; **Tasks 19, 20, 24 and 28 append ADR 0007 sections** (the shape-cache key, the fast-scroll thresholds, the off-thread analysis of R-M3-22 and the preview rule). A security choice made during a fix round goes into the same task's section before re-review.
- **Append-only shared files.** Each task appends one block headed `## m3a-tNN-<slug>` (FTL, TOML, keymaps), `// ## m3a-tNN-<slug>` (Rust), `# ## m3a-tNN-<slug>` (`ci.yml`), and never edits another task's block. The files: `locales/{en,tr}/{hacivat-ui,hacivat-core,hui}.ftl`; `locales/_schema/literal-allowlist.toml`; `crates/ui/hacivat-ui/keymaps/default-{macos,windows,linux}.toml` and `presets/datagrip.toml` (Task 1b only); ADRs 0002 and 0004–0007; `.github/workflows/ci.yml`; `crates/app/hacivat-app/tests/support/mod.rs`; `crates/ui/hacivat-ui/tests/support/mod.rs`; `crates/core/hacivat-session/tests/common/mod.rs`; `crates/core/hacivat-exec/tests/common/mod.rs`; `docker/m2a/init/*.sql`. The controller resolves append conflicts by keeping both blocks. Named exceptions are listed in the task that takes them (Task 1a's one `libclang` step inside existing `ci.yml` jobs is one).
- **Docker.**
  - The compose project `hacivat-m2a` (`docker/m2a/`) is the matrix and is **controller-owned**: only the controller runs `cargo xtask docker up|down`.
  - Docker tests are `#[ignore = "docker"]` and run **only through the lock wrapper** `<main>/target/docker/run-docker-tests.sh --set pr|full -p <crate> [--test <name>]` from the worktree root (it serialises `cargo xtask docker test` across agents; shared fixtures; `--test` is Task 1b's pass-through, P9). It fails fast on an unhealthy service: report it, never restart anything.
  - **Pre-build first:** run `cargo test -p <crate> --no-run` before the wrapper, so its lock covers only the test run. The wrapper exits 1 when its lock wait (720 × 5 s) expires and never runs unlocked (the controller changes it before W1, P9). `hacivat-exec` joins `DEFAULT_TEST_CRATES` (Task 1b, P6), so `docker-pr` builds `pg_query`.
  - Targets come only from `hacivat_testkit::require_target(id)` or `targets(Some(engine))` (Q6); no literal target id outside those calls. Fixtures and roles are created idempotently by test helpers (Q7). Docker artifacts only through `hacivat_testkit::artifact(name)`. No test calls `std::env::set_var`/`remove_var`.
  - **Idle bounds in Docker tests are injected (2 s)**, never real minutes. 1M-row tests run on the `full` set only. PR-set additions stay ≤ 5 min wall, full-set additions ≤ 15 min (`docker-pr` times out at 60 min, `ci.yml:517`).
- **Full test set.** "All tests pass" means every line below. Each task runs the lines for the crates it touches. PR CI: `build-test` (macOS, Windows, Linux) and `turkey-test` (`tr_TR`) run lines 1–4 and 6–8 (Task 1b adds lines 6–8, which no job ran before, P10); `docker-pr` runs line 5 and the `pr` Docker set. Nightly: line 9 (`release-tests`), `docker-full` and the bench job. The gate runs all, the perf lines included.
  ```
  cargo test --workspace
  cargo test -p hacivat-ui-core --features test-support,scene-check
  cargo test -p hacivat-l10n --features pseudo
  cargo test -p hacivat-app --features scene-check,pseudo
  cargo test -p hacivat-driver --features fake
  cargo test -p hacivat-testkit --features ssh
  cargo test -p hacivat-ui-editor --features scene-check
  cargo test -p hacivat-ui-grid --features test-support,scene-check
  cargo test --release -p hacivat-l10n --features pseudo
  cargo xtask docker check-pins
  <main>/target/docker/run-docker-tests.sh --set full
  cargo test -p hacivat-app --features shots,scene-check,pseudo --test shots
  HACIVAT_PERF=1 cargo test --release -p sql-lexer -p sql-oracle-pg -p hacivat-text -p hacivat-values -p hacivat-results -p hacivat-sql-editor -p hacivat-ui-editor -- --ignored
  HACIVAT_PERF=1 cargo test --release -p hacivat-driver-sqlite --test typed -- --ignored
  HACIVAT_PERF=1 cargo test --release -p hacivat-app --test results_model -- --ignored
  ```
  The three perf lines name crates and test targets, not a name filter (perf test names share no substring). A task that adds a perf test elsewhere adds its crate or `--test` target to these lines and says so in its report.
  **Every new test file compiles under plain `cargo test --workspace`:** scene-check-only asserts are `#[cfg(feature = "scene-check")]`; Unix-only cases are `#[cfg(unix)]` with `#![cfg_attr(not(unix), allow(dead_code, unused_imports, unused_variables))]` at the file top (M2b CI ruling); each task also runs `cargo test -p <crate> --no-run` without features. Windows CI is the first to compile some tests; one-lint-per-round fixes are avoided by that attribute. A shared module that already has `#![allow(dead_code)]` (`hacivat-app/tests/support/mod.rs:7`, `hacivat-ui/tests/support/mod.rs:3`, the session's `tests/common/mod.rs`) adds only `#![cfg_attr(not(unix), allow(unused_imports, unused_variables))]` (clippy `duplicated_attributes`, P15).
- **Cross-platform test rules (M2b CI lessons, P15).** A test that finds a menu or button label by its drawn text uses the shared run-joining finder `find_label_runs` (Task 1b, in `hacivat-ui/tests/support` and `hacivat-app/tests/support`): on Linux and Windows a `PopupMenu` label is drawn as runs around its access-key letter. Path assertions compare `Path`s, never strings with `/` (the autosave path `<data>/consoles/<uuid>/<n>.sql`, the golden loader). CRLF fixtures are generated at test time (`.gitattributes` `* text=auto eol=lf` rewrites committed CRLF); TOML corpora spell `\r` as an escape.
- **Test timing (Q4, Q25).** Tests wait on a condition (`run_until_parked`, the app's `settle_until`, `wait_until`/`poll_until`), never on a fixed sleep or a tight wall budget before an assertion. `#[tokio::test(start_paused = true)]` only with FakeDriver or pure state, never with real sockets. Perf assertions, and every wall-time assert outside the Docker tests (whose `…within_1s` asserts stay, with margin), are `#[ignore = "perf"]` and begin with `if !perf_enabled() { return; }` (`hacivat_testkit::perf_enabled()` reads `HACIVAT_PERF=1`, Task 1b; crates that may not reach `crates/core`, the `crates/sql` family under `SqlReachesCore` and the Ui crates, inline the same `std::env::var_os("HACIVAT_PERF")` check), so `docker-pr`'s `-- --ignored` skips them in debug; the gate runs them in `--release` through the perf lines (P9). PR CI asserts complexity counters, never wall time (L2).
- **Threading (Q4 ruling, §5.4).**
  - The UI thread never awaits database, network or file I/O and never runs anything > 1 ms. Awaiting `tokio::sync` channels from GPUI foreground tasks is allowed (the `AppUiPort` pattern). Driver and session work runs on the core Tokio runtime; SQLite on its dedicated blocking thread; CPU work that UI starts (lexing a whole document, resolving units, copy over 10k cells, column measurement batches) runs on the GPUI background executor.
  - **Paint path:** one `ArcSwap` snapshot load per frame, no lock, no `await`, no I/O, no allocation per cell on a cache hit, at most one `cx.notify()` per frame per entity (§5.4 rules 2–3).
  - Every shared test helper that boots core calls `cx.executor().allow_parking()` and stops the core runtime before the context drops (`stop_core_runtime`, moved into `tests/support/mod.rs` by Task 1b with two thin entry points over one body: `stop_core_runtime(&mut HeadlessAppContext)` for the shots harness and `stop_core_runtime_test(&mut TestAppContext)` for `#[gpui::test]`s, P20).
- **No silent weakening (F31).** A stub for a layer that decides what runs or what is allowed fails closed. M3a's list: typed `RawConn` defaults answer `RawErrorKind::Unsupported`; on PostgreSQL a typed `query` with `suspendable: true` outside a transaction block answers `Unsupported` (P5); the actor answers `Request::Run`/`FetchMore`/`CloseCursor` with `ExecErrorKind::Internal` until Task 18; the typed read-only path answers `Internal` until Task 22; `resolve_units` answers `NothingToRun`, the splitter stub returns one `Heuristic` item and the lexer stub one `Unknown` token over the whole input; the preview stub answers `Uncertain` with no range; the app's Run stub sends nothing. **Production-tagged consoles are read-only (R-M3-6 REVISED)** from Task 1b on: `effective_read_only` ships working.
- **Design rules (DESIGN.md).** One Filled Badge (no new `cp_fill`); Hue Is Meaning (amber = pending/running, green = success/loaded, red = production/destructive only, blue = selection/focus/primary); the Raised Ink Rule (`text-3` never on raised surfaces; toolbars and strips use `text-2`); the 11 px floor; no ambient motion (static skeletons; countdowns and timers repaint at most once a second); Tabular Everywhere. Errors use ink glyphs, never a new hue. No red quad in a non-production editor.
- **Numbers and text.** Grid numbers and chrome quantities use the **format locale** (OS region, L3, L15(d)); ID-like columns stay ungrouped; dates stay ISO. Copy, sort keys and every SQL value use raw, locale-free text (I18N-11). NULL is the constant `NULL` (D-065), never a Fluent message. Header names and cell text are data (`TextSource::Catalog`/`Data`), never pseudo-localised. No number is joined to a unit in code: each count or duration is one Fluent message with the unit inside. Server text is shown verbatim in its own field, never inside a translated sentence.
- **Keys.** Every binding is keymap data (Task 1b writes the M3a rows); widgets never call `cx.bind_keys` (`menus::rebuild` owns GPUI bindings); row-level keys are element-level `on_key_down`. Every new chord passes the AltGr CI for TR-Q, TR-F, DE, FR and PL (`crates/ui/hacivat-ui/tests/keymap_collisions.rs`). No binding fires during an IME composition (F15).
- **Shots harness.** The `shots` target is `harness = false` and owns `main` (macOS text system on the main thread); surfaces are added to it, never as `#[test]` functions. Grid and results surfaces render from in-memory `ResultStore::from_rows` fixtures with no core runtime whenever a live connection is not the subject (R-M3-15); a shot that boots core stops the core runtime after it. Every surface renders at 1280 × 800 and 960 × 600 × `en`, `tr`, `en-XA`, `tr-XT` × dark and light. `cargo xtask shot-diff --baseline <main>/.superpowers/sdd/2026-11-14-m3-walking-skeleton/baseline-m2b --candidate target/shots` gates every pre-existing shot at **zero tolerance** against the preserved M2b render (P17). The agent **looks at the PNGs** of every new surface (M2b T24 lesson); markers must be specific to the surface, not satisfied by unrelated labels.
- **Rate limits.** The controller watches `~/.claude/rate-limits-<this session id>.json`, not the shared `~/.claude/rate-limits.json` (other sessions write it; its 93 % on 2026-11-14 was stale, P3), and pauses new dispatches at `seven_day ≥ 98 %` (the PO's threshold, 2026-11-14). This account has no 5-hour window.

## Review Focus

The five input classes the spec implies that no feature test exercises on its own and that are most likely to hurt a person using M3a. Most likely first. Each line names the test that pins it and the task that owns it.

1. **Real result data is hostile or odd.**
   - Cases: text that is not valid UTF-8 (a PostgreSQL `SQL_ASCII` database); a 5 MiB `bytea`; a numeric with 1,001 digits; MySQL `0000-00-00`; a SQLite file whose declared type is not UTF-8; bidi controls and newlines inside cell text; the text `'NULL'` next to a real NULL; 2,000+ columns; duplicate column names.
   - Expected: never a panic, never a silently altered value. Invalid text is kept as bytes and flagged; large cells are truncated at a character boundary with their total length; controls are visible in the grid and verbatim in copy; NULL and `'NULL'` look different.
   - Tests: `invalid_utf8_text_is_lossy_never_replaced`, `cell_over_1_mib_is_truncated_at_a_char_boundary`, `numeric_with_1001_digits_is_an_error_cell_not_a_panic` (Task 12); `mysql_zero_date_is_special_not_null` (Task 14); `non_utf8_declared_type_does_not_panic` (Task 15); `text_null_string_is_not_the_null_marker` (Task 16); `bidi_controls_paint_as_chips` (Task 20); `copy_keeps_control_characters_verbatim` and `duplicate_column_names_keep_their_order` (Task 26); `two_thousand_columns_render_with_the_hint` (Task 30).
2. **A cursor is interrupted.**
   - Cases: the user runs another statement while a result is suspended; closes the result's console; the network drops while suspended; the server's 30 s idle-in-transaction kill on a read-only session; a SQLite cursor in rollback-journal mode while another process wants to write; quitting with a suspended cursor.
   - Expected: the cursor is released before anything else runs; rows already fetched stay browsable and say why the cursor closed; Hacivat's own transaction always ends (ROLLBACK unless the result completed, R-M3-23); no connection is left stuck.
   - Tests: `another_statement_releases_the_cursor_first`, `incomplete_cursor_rolls_back_hacivats_own_transaction`, `connection_lost_while_suspended_keeps_rows` (Task 18); `rollback_journal_cursor_uses_the_30s_bound` (Task 18); `ro_cursor_is_released_before_the_server_idle_kill` (Task 22, Docker); `closing_a_console_releases_its_cursor` (Task 33).
3. **The text in the editor is large or odd.**
   - Cases: 50k lines; a 100k-character line; a mix of CRLF, CR and LF; a BOM; U+2028; tabs; combining marks; Turkish `İ`/`ı`; emoji; a NUL in pasted text; an IME composition across all of these.
   - Expected: typing stays within B3; offsets reported to the OS are exact; line endings survive byte for byte; controls are visible.
   - Tests: `crlf_cr_lf_mix_round_trips_byte_for_byte`, `u2028_is_not_a_line_break`, `bom_round_trips` (Task 5); `marked_text_across_a_surrogate_pair_reports_the_adjusted_range`, `dotted_capital_i_with_combining_dot_is_one_grapheme` (Task 10); `one_line_reshaped_per_typed_character` (Task 19); `nul_paste_warns_and_shows_the_control_glyph` (Task 27).
4. **Run-at-caret is ambiguous.**
   - Cases: caret exactly on a `;`; caret inside a comment, a string or a dollar-quoted function body; MySQL `DELIMITER $$`; `;` inside `/*! … */`; an unterminated quote above 50 statements; a selection that starts inside a string.
   - Expected: Run never executes a range the preview did not show; an uncertain boundary is drawn as uncertain and sends nothing (R-M3-8, R-M3-24).
   - Tests: `unterminated_quote_keeps_later_boundaries_provisional`, `semicolon_in_executable_comment_is_ambiguous` (Task 8); `caret_in_dollar_body_runs_the_whole_create_function`, `mysql_delimiter_block_is_one_unit` (Task 13); `preview_equals_execution_for_exact_items` (Task 24, property); `ambiguous_resolution_sends_nothing` (Task 33).
5. **Production and transaction edges.**
   - Cases: a profile is tagged production while a console is open; the production session reconnects; single-session mode on a production profile; a user types `BEGIN` on a dev console and then closes the tab or quits; a Cancel inside that transaction reaches the server.
   - Expected: a production console never writes in M3a; a user transaction is never discarded silently (D-014); a server cancel inside it leaves it `Failed`, as in psql, the transaction segment says so, and the close prompt then offers only Roll back and Cancel (R-M3-16 as amended).
   - Tests: `production_session_is_read_only_after_reconnect`, `production_tag_applies_to_the_next_session`, `single_session_production_is_read_only`, `pooled_borrow_on_production_reuses_its_session`, `single_session_production_borrow_succeeds`, `cancel_in_a_user_transaction_marks_it_failed` (Task 18); `production_console_rejects_insert_on_every_engine` (Task 31, Docker); `closing_a_console_with_an_open_transaction_asks`, `quit_with_an_open_transaction_asks_and_cancel_keeps_the_app`, `close_prompt_on_a_failed_transaction_offers_roll_back_only` (Task 33).

## Assumptions and rulings

### Binding rulings R-M3-1..21 (ledger), as M3a applies them

These are final; the plan applies them and does not re-open them.

| # | Ruling (short) | How M3a applies it | Where |
|---|---|---|---|
| R-M3-1 | `pg_query =18.1.0` with a crate-scoped PostgreSQL-License exception, `THIRD_PARTY_NOTICES.md`, libclang proven on three OSes, vendored fallback, protoc guard | **Fallback taken now (P1):** `pg_query` vendored under `third_party/pg_query` through `[patch.crates-io]` (the `mysql_async` pattern); `PATCHES.md` §0 pristine, §1 the protobuf regeneration branch removed from `build.rs`, so the shipped pre-generated `src/protobuf.rs` is always used and a `protoc` on `PATH` (this machine has one) changes nothing; bindgen kept, so libclang is still proven on three OSes; pre-generated `bindings.rs` only if the Windows or universal-macOS proof fails. `deny.toml` clarify + exception; notices shipped through a new `resources` key in `packaging/Packager.toml`; the CI action `.github/actions/libclang` (`clang --version`, Windows `LIBCLANG_PATH`, Linux `libclang-dev`, the protoc guard) on every job that builds a crate reaching `pg_query`, and a check that a build leaves `third_party/pg_query` unchanged; a `workflow_dispatch` nightly-package run proves the macOS universal build | Task 1a |
| R-M3-2 | Generated visitor from `pg_query.proto`; interim sqlparser path | M3b (no classifier in M3a) | — |
| R-M3-3 | Nesting limit before any parser; 16 MiB classifier stacks | Applied to M3a's only parser call: the PG split oracle runs on a 16 MiB-stack thread and only for units whose lexer nesting depth is ≤ 256 (R-M3-35) | Tasks 9, 23 |
| R-M3-4 | `twox-hash`, not `xxhash-rust` | `StmtKey` (XXH3-128) and the buffer content hash use `twox-hash` (with `alloc`, for the streaming hasher, P6) | Tasks 1b, 5, 8 |
| R-M3-5 | `alg/10` SPLIT-02 errata | Task 8 fixes the four cases in tests and edits `docs/research/algorithms/10-sql-lexing-splitting-params.md` (named exception, docs only) | Task 8 |
| R-M3-6 REVISED | Production consoles read-only through the M2b guard until M3b's confirmations | `effective_read_only(profile, production_read_only)` forces read-only sessions for `EnvTag::Production` (R-M3-29); the console toolbar says so; no interim classifier | Tasks 1b, 18, 32 |
| R-M3-7 | macOS IME matrix re-run on the real editor + scripted traces; Windows/Linux IME stays open; notify GPUI only after non-IME edits | Traces in Tasks 10 and 27; checklist `docs/benchmarks/m3a-ime-checklist.md` at the gate | Tasks 10, 27, 37 |
| R-M3-8 | Preview and execution agree on every exact statement; uncertain → drawn uncertain, Run asks | `resolve(.., SplitMode)` serves both; a property test proves agreement | Tasks 13, 24 |
| R-M3-9 | Chunked lexer interface, O(log n) queries, incremental ≤ 2 ms, fixed in the interface commit | Signatures (`TextChunks`, `lex_chunks`, `relex`, `Lexed::token_at`/`tokens_in`) are fixed in Task 1b. M3a's `relex` body is a full relex and runs off the UI thread (R-M3-22). **The ≤ 2 ms incremental budget is technically unreachable without incremental relex, which the SPLIT DECISION puts in M3b. Smallest change: measure that budget at M3b's gate; M3a's keystroke budget for the language service is ≤ 0.5 ms because nothing is lexed on the keystroke frame** | Tasks 1b, 4, 24 |
| R-M3-10 | Turkish find through regex classes | M3b (find) | — |
| R-M3-11 | B3 in the real app; view caching; Ln/Col its own entity; bench variant with a grid open | Tasks 32 and 36 | Tasks 19, 32, 36 |
| R-M3-12 | B1-real 1M × 50 from PG in `tr-TR`; fast-scroll; display cache not keyed on version; sibling views cached; shaping caps | Tasks 20, 26, 30, 36 | — |
| R-M3-13 | TestScheduler lock-order patch on the fork after a licence check; rev bump | Task 1a Step 1 builds the commit in a scratch clone (the lock order; the fork's §4(b) header in `test_scheduler.rs`; `HACIVAT-PATCHES.md` widened to "`crates/gpui*` and `crates/scheduler`"); the **controller** pushes it fast-forward only; then Task 1a edits `rev` (P13). The scheduler crate declares `license = "Apache-2.0"`, links `LICENSE-APACHE` and has permissive dependencies (pre-flight); Task 1a re-checks at the fork head | Task 1a |
| R-M3-14 | Confirmations policy | M3b | — |
| R-M3-15 | Shots from fixtures with no core runtime; sharding; grid a11y on demand | `ResultStore::from_rows` fixtures; `--shard k/n` in the harness. The grid a11y subtree is M3b (R-M3-32) | Tasks 7, 35 |
| R-M3-16 | Chunk-step fetching; Cancel = stop fetching; server cancel only when a chunk is > 1 s late; never abort the user's PG transaction | The actor drains commands between chunks; a suspendable unit inside a user transaction runs under `SAVEPOINT hv_cursor` (R-M3-23), so stopping it is undone by `ROLLBACK TO SAVEPOINT`. **Amended (P16, psql semantics):** a server cancel inside a user transaction (an overdue chunk of a non-suspendable unit: `UPDATE … RETURNING`, a locking `SELECT`, `CALL`) leaves that transaction `Failed`, as in psql; the transaction segment says so, and while it is `Failed` the close prompt offers only Roll back and Cancel. No per-unit savepoint (controller) | Tasks 18, 33, 34 |
| R-M3-17 | Typed RO guard: post-steps at cursor end + canary after the first chunk; PG `idle_in_transaction_session_timeout = 30s`; RO cursors released after 20 s idle; D-019 | Task 22 | Task 22 |
| R-M3-18 | Vendored `tokio-postgres` with one patch; fallback `DECLARE`/`FETCH` | Pristine copy in Task 1b; patch §1 in Task 12, with three entry points (P5) | Tasks 1b, 12 |
| R-M3-19 | 256 MiB per result; RAM figure injected; B1-real uses Fetch all | `physical_memory_bytes()` in `hacivat-ui-core/src/platform/memory.rs` (macOS through `libc`, Windows through `windows-sys` `Win32_System_SystemInformation`: Task 1b's rows, P6); `MemoryBudget::from_ram` | Tasks 7, 33, 36 |
| R-M3-20 | Per-engine idle bounds; 120 s on production | Bounds table in Task 18 | Task 18 |
| R-M3-21 | EXPLAIN plan model → M4; M3 only the D-062 wrap and raw EXPLAIN | Raw EXPLAIN runs as an ordinary statement in M3a; the D-062 wrap needs the classifier → M3b | — |

### New rulings (R-M3-22 onwards)

Controller calls taken while writing this plan, under the PO's 2026-11-14 mandate. Each takes effect now; each is listed for the PO at the gate.

- **R-M3-22. M3a lexes off the UI thread.** The SQL adapter sends each committed editor `Snapshot` to a per-document worker on the GPUI background executor, which builds a fresh `SqlDocument` (full lex plus editor-mode split) and publishes a `DocSnapshot` tagged with the editor version. Highlight decorations from an older version are rebased through the editor's `ChangeLog` (ranges an edit touched paint plain until the next snapshot); the Run preview uses only a snapshot whose version equals the editor's. Run reuses the current snapshot when the versions match, else analyses the request's own snapshot in the background. *Why:* B3 in the real app is an M3a gate item, while incremental relex is M3b (SPLIT DECISION); a full relex of 2 MB on the keystroke frame (≈ 10–20 ms) would break B3. *Cost if wrong:* highlighting lags by one background lex (≈ 20 ms at 2 MB); no correctness cost, because the preview never shows a stale range.
- **R-M3-23. Cursor eligibility without a classifier.** A unit may stay suspended only when the lexer shows a read head: the first significant keyword is `SELECT`, `VALUES` or `TABLE` (MySQL also `SHOW`), or `WITH` with no `INSERT`/`UPDATE`/`DELETE`/`MERGE` keyword token in the unit, and the unit has no `INTO` keyword and no locking clause (`FOR UPDATE|SHARE|NO KEY UPDATE|KEY SHARE`, `LOCK IN SHARE MODE`). Every other unit streams to completion, with rows beyond the cap counted, not kept. **A suspended cursor that does not complete ends Hacivat's own transaction with `ROLLBACK`** (PostgreSQL in Auto: Hacivat's `BEGIN`; inside a user transaction: `SAVEPOINT hv_cursor` … `ROLLBACK TO SAVEPOINT hv_cursor; RELEASE SAVEPOINT hv_cursor`); a cursor that completes commits (or releases the savepoint), exactly like an autocommit statement. MySQL releases by drain or `KILL QUERY` (statement-atomic); SQLite by finalize. M3b ANDs this with the classifier's Read verdict. *Why:* the SPLIT DECISION puts cursors and stop-fetching in M3a, but ER5's Read verdict needs M3b's classifier; this is a fetch strategy, not a safety layer (read-only safety stays with the guard), so it does not contradict R-M3-6's "no interim classifier". *Cost if wrong:* a writing function called from a read-head `SELECT` is undone rather than half-applied whenever the cursor does not complete: when the user stops it (the same as cancelling it in psql), and also when another statement or the idle bound releases it (P26). The `CursorReleased` log line then says "Hacivat's transaction was rolled back". Listed for the PO.
- **R-M3-24. Ambiguous boundaries send nothing in M3a.** Execution never sends an `Ambiguous` range. For PostgreSQL, `hacivat-exec` asks `sql_oracle_pg::split_with_parser` and uses its boundaries when they cover the unit exactly. Otherwise the chooser shows a "boundary uncertain" state with no run action. "Run as one" (simple protocol, several result sets) arrives with multiple result sets in M3b. *Cost if wrong:* a script with `;` inside `/*! … */` or an unknown block cannot be run at the caret in M3a; selecting a clean range still works.
- **R-M3-25. One result set per unit in M3a.** A unit's first result set is stored and shown; further result sets (MySQL `CALL`, multi-result) are drained, counted, logged (`exec-more-result-sets`) and noted in the strip. One result tab per unit. *Cost if wrong:* `CALL` users see only the first set until M3b.
- **R-M3-26. Error policy StopOnError in M3a.** A selection of N statements runs as N units in one request; the first failure stops the run and the rest show Skipped. The Ask bar and PostgreSQL "roll back and continue" are M3b. *Cost if wrong:* a long script stops at the first error.
- **R-M3-27. Open user transactions in M3a.** Production consoles are read-only (always Auto), so a user transaction exists only after a user `BEGIN` on a non-production console. M3a gives it: a read-only status-bar segment ("Transaction open · 3 statements"), palette commands `tx.commit` and `tx.rollback`, and the **close-with-open-transaction prompt** (Commit / Roll back / Cancel, Cancel focused) on console close and on `app.quit`. While the transaction is `Failed` (an error or a server cancel inside it, R-M3-16 amended) the segment says so and the prompt offers only Roll back and Cancel. The badge with buttons, the mode and isolation chip, the aborted banner, the idle reminder and Smart mode are M3b. *Why:* D-014 forbids discarding a transaction silently. *Cost if wrong:* one small dialog built before the rest of the transaction UI.
- **R-M3-28. ADR numbering.** Digests 01 and 03 both claimed ADR 0005. M3a uses 0005 (SQL text and safety), 0006 (execution and results) and 0007 (editor and grid). *Cost if wrong:* a rename.
- **R-M3-29. How production is forced read-only** (amended by P4). One function, `hacivat_session::effective_read_only(profile: &Profile, production_read_only: bool) -> bool` = `profile.read_only || (production_read_only && profile.environment == Some(EnvTag::Production))`, replaces every site that reads `read_only` for a session: `manager.rs:583`, `session/pool.rs:559` (`pooled`), `:776` (single-session `shared`), `:819`, `:868`, `:1211`, and `test_conn.rs:110` (Test connection, so a production Test exercises the read-only path). `pool.rs:1005` then keys idle sessions by the effective value; `actor.rs:174` (`spec.read_only || profile.read_only`) stays, because the spec carries the effective value. `SessionManager` gets `set_policy(SessionPolicy { production_read_only })`, which `ConnectionManager` calls right after `SessionManager::new` (`manager.rs:385`), so the constructor and its four callers are unchanged. `ManagerConfig.production_read_only` defaults to `false` (M2b tests unchanged); the app's boot sets it `true`. M3b deletes the flag when confirmations land. *Cost if wrong:* a pooled production borrow that asks for a read-write key opens a new connection per borrow until the budget is exhausted; the review grep `grep -nE '\.read_only\b' crates/core/hacivat-session/src` and Task 18's tests (every session kind on a production profile, pooled reuse, the single-session borrow) catch a missed site.
- **R-M3-30. Shots and DESIGN.md in M3a.** Task 35 adds shots for every M3a surface (0 violations) and diffs every M1/M2 shot at zero tolerance. The DESIGN.md "Query, Results and Grid" section is written once, in M3b (SPLIT DECISION); until then `design/m3a-query/NOTES.md` is the binding component reference. *Cost if wrong:* a DESIGN.md section that lags the code by one milestone half.
- **R-M3-31. Value types in M3a.** Typed decoders cover bool, integers, floats, exact numeric/decimal, text (and PG enum labels), bytea/blob, uuid, json/jsonb, date, time(tz), timestamp(tz), interval and the special values. PostgreSQL arrays, composites, ranges, network, bit-string, geometry, `money`, `reg*` and every unknown OID are requested in **text** format and stored as `Value::Opaque(OpaqueVal { type_name, text })` (P8); MySQL and SQLite map the same way. The full type matrix is M3b. *Cost if wrong:* those columns sort and align as text until M3b.
- **R-M3-32. Grid and results scope cuts.** Count rows, local sort, find in the grid, go to row, transposed/record/text views, the value viewer, the grid context menu, history and the grid's AccessKit subtree are M3b. M3a's grid declares role `Grid` with its label (one declaration, as the M0 probe did). *Cost if wrong:* read-heavy work lacks sort and find for one milestone half.
- **R-M3-33. Fetch-all and memory in M3a.** "Fetch all" fetches up to the per-result cap (256 MiB). At the cap the strip says "Memory limit for this result reached (N rows, X MB)" and offers **Fetch more anyway** (raises the cap to the global budget) and **Close cursor**: that banner is the confirmation, with no separate dialog. No eviction in M3a; global exhaustion pauses with `PauseReason::GlobalBudget`. *Cost if wrong:* two open 1M-row results on an 8 GB machine pause rather than evict.
- **R-M3-34. No `sql-ident`, `sql-safety` or `sqlparser` in M3a.** M3b's interface commit adds them. The check-deps rules `SqlNotRuntimeFree`, `SqlReachesCore` and the `pg_query` arm of `BackendOutsideOwner` land now. *Cost if wrong:* none; M3b adds crates.
- **R-M3-35. The PG oracle in M3a** runs only on a dedicated thread with a 16 MiB stack and only for units whose lexer nesting depth is ≤ 256 (R-M3-3 on M3a's one parser call). *Cost if wrong:* deep generated SQL is not oracle-resolved; it is only affected when ambiguous.
- **R-M3-36. The D-011 chooser is in M3a.** `Ask` opens a popup menu anchored at `EditorView::caret_bounds()`, built on `hacivat-ui`'s `PopupMenu`: items in resolve order with whitespace-collapsed labels, hover sets the preview override, Enter runs, Escape cancels. The chosen `CandidateKind` is remembered per console for the session and preselected next time. CHOOSE-06 ("Customize…") is M3b. The chooser calls `PopupMenu::set_access_keys(false)`: its labels are SQL text, a data label never gets an access key, and typing a letter never runs a statement (P15). *Cost if wrong:* small; the popup is data-driven.
- **R-M3-37. An unbound console keeps the M2 home.** A console tab without a connection renders exactly today's `editor_empty` element (M2 shots unchanged). The first successful connect binds the focused unbound console; `workspace.new_console_tab` opens a console bound to the capsule's connection when one is connected; a restored console keeps its saved binding and shows the editor (disconnected) with the preview in `NoConnection`. Digest 02's "editor usable with no connection" is not taken. *Cost if wrong:* a scratch editor without a connection waits for M4b's files.
- **R-M3-38. M3a chords.** `exec.cancel`: `cmd-.` (macOS) and, on Windows/Linux, `ctrl-break` plus a second row `ctrl-f2` (most laptops have no Break key, P25) in the hacivat preset; `ctrl-f2` in the datagrip preset; Stop stays on the toolbar. Escape never cancels a statement (it collapses the selection, and Escape-then-Tab leaves the editor). M3a binds no editor `cmd-f`/`ctrl-f`, so the Workspace "Focus filter" chord is untouched until M3b's find. The Grid's "select column" is unbound in M3a (palette `grid.select_column`): `ctrl-space` is macOS's input-source switch and a CJK IME toggle on Windows, so it never reaches the app (P25). *Cost if wrong:* keymap data only; `ctrl-f2` switches desktops on KDE Plasma, where the toolbar's Stop remains.
- **R-M3-39. The SQL context is read by the execution layer.** `hacivat-exec::sqlctx` builds the `DialectCtx` from `DbmsKind`, the connected server version (`LiveInfo.server.version`) and one probe row (PostgreSQL `SHOW standard_conforming_strings`; MySQL/MariaDB `SELECT @@SESSION.sql_mode`), read when a console binds and again after any unit whose first keyword is `SET`. No session or driver change. *Cost if wrong:* a `SET sql_mode` inside a stored procedure is not seen until the next bind; splitting may then disagree with the server, which execution mode's refusal of uncertain boundaries contains.
- **R-M3-40. Benches are examples behind a crate feature.** `hacivat-app` gets `bench = ["hacivat-ui-core/bench", "hacivat-driver/fake"]` and `[[example]]`s `b3` and `b1_real` with `required-features = ["bench"]`, the `shots` precedent (`crates/app/hacivat-app/Cargo.toml:10-12`). Digest 04's `bench-b1` test target is not taken. Task 1b creates the example files and `src/bench/mod.rs` as fail-closed stubs (exit 2), so the manifests that declare them are valid from the start (P7). *Cost if wrong:* none; `check-deps` `AdapterFeatureInProduct` checks dependency declarations, which stay clean.
- **R-M3-41. The results tile follows the focused pane's active console.** Result tabs belong to their console; focusing another pane swaps the tile. *Cost if wrong:* per-pane tiles are an M-size layout change.
- **R-M3-42. Shared-type shapes.** `sql_text::TextEdit { delete: TextRange, insert: String }` (digest 01's shape, not digest 02's `{ old, new_len }`); the lexer's highlight vocabulary is `sql_lexer::TokenClass` (the editor keeps `hacivat_ui_editor::HighlightClass`; the adapter maps); `StmtKey(pub u128)` is the XXH3-128 itself, with no interner. *Cost if wrong:* renames.
- **R-M3-43. "Waiting for a connection" is derived in the app** from a pending `SessionManager::open` future plus `SessionManager::budget_use() -> (in_use, max)` (Task 1b adds the accessor). No new `SessionEvent`. *Cost if wrong:* none in M3a.
- **R-M3-44. Settings in M3a.** The `[editor]` keys exist in the settings schema and file; their Settings-UI rows are M3b. The grid's raw-display setting (L3) is M3b; M3a always shows L3's default with ID-like columns raw. *Cost if wrong:* editing `settings.toml` by hand for one milestone half.
- **R-M3-45. Run on a disconnected bound console** starts that profile's connect (the explorer's connect action) and runs nothing; the user runs again once connected. *Cost if wrong:* one extra key press.
- **R-M3-46. PostgreSQL `BEGIN`/`SAVEPOINT` before a cursor is a separate round trip in M3a** (not pipelined with Parse). *Cost if wrong:* one RTT per suspendable unit; pipelining is a follow-up.
- **R-M3-48. The result store publishes on every append.** `alg/21`'s 16 ms writer throttle is dropped: each append and state change publishes a new `StoreSnapshot` (O(chunks), ≤ 250 `Arc` clones for 1M rows), and consumers coalesce through the `watch` to one `cx.notify()` per frame. *Why:* simpler, and no chunk waits unpublished during a slow stream. *Cost if wrong:* a few thousand extra `Arc` clones per million rows.
- **R-M3-49. No content above 20 MB enters the editor in M3a.** There is no T2 tier until M4b, so a paste (or a restored console) that would take the document past T1's 20 MB is refused with `PasteRefused`; the 20–100 MB confirmation of UI-EDIT-14 arrives with T2 in M4b. A restored console file over 20 MB is refused with a notice and left on disk (Task 32, P26). *Cost if wrong:* a user cannot paste a 30 MB dump into a console until M4b.
- **R-M3-47. The M2b debug cancel demo goes away** when the real cancel lands (Task 33 removes the `## m2b-t22-live` block in `commands/debug.rs` and its two install lines in `connections/mod.rs:54-55`, a named exception; digest 03 C5).

### Pre-flight rulings P1–P28 (2026-11-14)

Findings of `preflight.md` (3 Blocker, 14 Important, 11 Minor), each applied where it belongs and listed for the PO at the gate; the ledger's "Plan amend pass" records the reason and the cost if wrong.

- **P1** → R-M3-1's fallback taken now: `pg_query` 18.1.0 vendored under `third_party/pg_query` (`[patch.crates-io]`, the `mysql_async` pattern); `PATCHES.md` §1 removes the protobuf regeneration from `build.rs`, so the shipped pre-generated `src/protobuf.rs` is always used and the `protoc` on this machine's `PATH` cannot rewrite anything; bindgen kept; nobody builds the registry copy (pre-flight).
- **P2** → `m3a` is cut from the m2b tip `5d5832f`, whose PR CI run 43512432001 is green (14 passed, 8 skipped by trigger); `5d5832f` is `ffa78ea` plus a docs commit, so every cited line holds (pre-flight).
- **P3** → rejected by the controller: the 93 % came from the shared rate file, stale from another session; this session is at 40 %. The controller watches `~/.claude/rate-limits-<this session id>.json` and pauses at `seven_day ≥ 98 %`, the PO's threshold; the account has no 5-hour window (pre-flight).
- **P4** → `effective_read_only(profile, production_read_only: bool)`; `SessionManager::set_policy(SessionPolicy)` called right after `new`; seven sites (`manager.rs:583`, `pool.rs:559`, `:776`, `:819`, `:868`, `:1211`, `test_conn.rs:110`); the wider review grep; two Task 18 tests (pre-flight).
- **P5** → patch §1 gains `query_with_formats` (the unnamed portal, Bind+Execute(0)+Sync in one batch) for every non-suspendable unit; on PostgreSQL `suspendable: true` needs a transaction block, else `Unsupported`; PostgreSQL is the one exception to "one open query per connection" (the canary); three Task 12 tests (pre-flight).
- **P6** → the missing manifest rows (xtask `phf_codegen`; sql-editor `tokio`, `futures`; twox-hash `alloc`; ui-core `Win32_System_SystemInformation` and macOS `libc`; app `smallvec`; `[[test]] required-features`); `DisplaySettings::new`; `hacivat_session::testing::fake_session`; Task 31's matrix moves to `hacivat-exec`, which joins `DEFAULT_TEST_CRATES` (pre-flight).
- **P7** → Task 1b creates `examples/{b1_synthetic,b3,b1_real}.rs` and `hacivat-app/src/bench/mod.rs` as fail-closed stubs; Tasks 20 and 36 modify them (pre-flight).
- **P8** → `Value` keeps its wide payloads behind `Arc` (`Bytes` and `Lossy` as `Arc<[u8]>`, `Error(Arc<ErrorVal>)`, `Opaque(Arc<OpaqueVal>)`) and is 32 bytes; ADR 0006 records it (pre-flight).
- **P9** → `xtask docker test --test <name>`; perf and wall-time tests self-gate on `HACIVAT_PERF=1` and the gate runs them in `--release` through the perf lines, which name crates and targets because the pre-flight's `-- --ignored perf` name filter matches almost no test; pre-build before the lock; the wrapper exits 1 when its wait expires (pre-flight).
- **P10** → test lines 6–8 in `build-test` and `turkey-test` (Task 1b); one `.github/actions/libclang` action on every job that builds a crate reaching `pg_query` (Task 1a); GC "Full test set" names the real jobs (pre-flight).
- **P11** → exact editor seam signatures and module-owned state (`MarkStore`, `BandState`, `A11yState`, `TierState`, `PasteState`; the band's state is `BandState` because `PreviewState` is already the exported verdict enum); `EditorView::last_layout()`; Task 19 owns every call site (pre-flight).
- **P12** → Task 1b declares every `results`, `console` and `exec_ui` submodule with stubs and keeps `results/mod.rs`; `results::view::build`; `console::editor_segment`; `register_test_console` with a real editor over a FakeDriver session; `TabKind::Console { n, profile }`, and `ConsoleViewFn` carries the profile (pre-flight).
- **P13** → Task 1a prepares the fork commit (the §4(b) header, the `HACIVAT-PATCHES.md` scope "`crates/gpui*` and `crates/scheduler`", `HEAD^` = `a1eaa20…`, `cargo +1.98.1 check -p scheduler`); the controller pushes it fast-forward only; the `rev` edit needs no `cargo update`; the clone is deleted (pre-flight).
- **P14** → gpui caps 4 (Docker matrix down), 3 (up), 2 (below 60 GiB free); `CARGO_BUILD_JOBS` 4 (core) / 6 (gpui) whenever 3 or more agents build; the controller checks `memory_pressure`/`vm_stat` before each dispatch (pre-flight).
- **P15** → the chooser assigns no access keys (`chooser_assigns_no_access_keys`); a shared run-joining label finder; no duplicated `allow`s; `Path` comparisons; CRLF generated at test time (pre-flight).
- **P16** → psql semantics: a server cancel inside a user transaction leaves it `Failed`; the transaction segment says so; the close prompt offers only Roll back and Cancel; `cancel_in_a_user_transaction_marks_it_failed` (Task 18) and `close_prompt_on_a_failed_transaction_offers_roll_back_only` (Task 33); the per-unit savepoint alternative is not adopted (pre-flight).
- **P17** → the M2b render is preserved as `baseline-m2b/` in the SDD folder before Task 1a, with its source commit; every `shot-diff` names it (pre-flight).
- **P18** → the wrong test values in Tasks 3, 4, 9, 10, 17, 23 and 26 are corrected, and the `SavedWorkspace` test becomes a unit test inside `workspace.rs` (pre-flight).
- **P19** → the Docker stack is up from W1, for Task 3's `--from-docker` (pre-flight).
- **P20** → `stop_core_runtime` gets a `HeadlessAppContext` and a `TestAppContext` entry over one body; `shots.rs` gains a `#[path]` `mod support;`; `register_focus_context(cx, handle, context)` (pre-flight).
- **P21** → Task 1b names the eight M2b app test files that the boot's `production_read_only: true` flips, and updates their read-only expectations without weakening them (pre-flight).
- **P22** → `PRODUCT_CRATES` gains the five new Ui and App crates; `SQL_FORBIDDEN` stays whole and ADR 0005 names the M5 exemptions; every type lives in the module file of the task that fills it; `kw::*` get placeholder values (pre-flight).
- **P23** → `Packager.toml` gets a new `resources` key, resolved from the rendered config; the `nightly-package` run lists the notices in the dmg, the NSIS installer and the AppImage (pre-flight).
- **P24** → `LogMsg::fluent_id()` returns a literal id in every arm; `ID_MODULES` gains `exec` and `cursor` (pre-flight).
- **P25** → the Grid's select-column is unbound in M3a; `exec.cancel` gains `ctrl-f2` on Windows/Linux; Task 25's Turkish-key test uses the bound `ctrl-m` (pre-flight).
- **P26** → R-M3-23's cost names releases by another statement or the idle bound, and the log line says "Hacivat's transaction was rolled back"; R-M3-49 covers a restored console over 20 MB (pre-flight).
- **P27** → E1 gets `dml_and_ddl_report_update_counts_on_a_dev_console` (Task 31); Task 27's 1 ms assert is a perf test; E3 and E10 cite the perf lines (pre-flight).
- **P28** → the rules are copied before Task 1a; `b1_real` documents `HACIVAT_DOCKER_SET`; the bench uses `hacivat_ui_core::frames::{FrameRecorder, verdict}`; Task 15 keeps a valid column name when only the declared type is invalid; Task 24 is security-reviewed (pre-flight).
- **Split** → Task 1 becomes Task 1a "Proofs" and Task 1b "Seams" (Part 4); no other task is renumbered. Task 1b starts from Task 1a's first commit (the rev bump and the vendored `pg_query`) and merges after it; the bundled `hacivat` binary reaches `pg_query` from Task 1a on, through a `--smoke` line, so the universal-macOS proof and the size delta are real (pre-flight).

### Inherited pre-flight lessons (M2b appendix Q1–Q46)

| Lesson | M3a application |
|---|---|
| Q1 disk is the binding constraint | Global Constraints "Disk and target directories" (on this machine RAM and CPU bind first, P14); wave notes |
| Q3 Task 1 names every site its new variants and fields break | Task 1b Files lists every `match` on `Request`, every `TxState`/`SessionSnapshot`/`ManagerConfig` literal, the `SavedKind` arms, every `TabKind::Console` pattern and the M2b app tests the boot change flips |
| Q4 override: awaiting `tokio::sync` from foreground tasks is allowed; condition waits; `allow_parking` | Global Constraints "Threading", "Test timing" |
| Q5 tests compile without features; `cfg(unix)` gating | Global Constraints "Full test set" |
| Q6/Q7 set-aware Docker targets, idempotent fixtures, controller-owned stack | Global Constraints "Docker" |
| Q8 Task 1 ships working helpers that same-wave tasks consume | Working in Task 1b: `TextBuffer`, `EditorModel::dispatch` (apply + selection), `Chunk::from_values`, FakeDriver typed scripting, `effective_read_only`, `impl FetchControl for SessionHandle`, `hacivat_session::testing::fake_session`, the `Consoles` registry, `register_test_console`, hook routing, `perf_enabled` |
| Q9 seams for cross-task call sites live in Task 1 | Stub functions with exact signatures that later tasks fill (P11, P12): `gutter::paint`, `marks::{add, set_state, clear, on_commit, paint}`, `preview::paint_band`, `scrollbar::paint`, `a11y::declare`, `readonly::guarded_query`, `exec_ui::attach`, `results::view::build`, `console::editor_segment` |
| Q25 no `start_paused` with real sockets | Global Constraints "Test timing" |
| Q29 vendored-copy hygiene | `third_party/pg_query` (Task 1a) and `third_party/tokio-postgres` (Task 1b) copied pristine, dev sections removed, `exclude` keeps them out of the workspace; each `PATCHES.md` lists its §N |
| Q32 real asserts | F9 in Global Constraints; reviewers flag tautologies |
| Q34 modal layers and safe defaults | The close-with-open-transaction prompt (Task 33): Cancel focused, Enter acts on key-down |
| Q40 ownership list synced with task Files | "Waves" ownership table |
| M2b ledger: Docker lock wrapper; never delete `target/docker`; clean members when a shared target switches path; shots look-at-the-PNGs; core runtime stopped after each shot | Global Constraints "Docker", "Disk", "Shots harness" |

## Deferred work (explicit; each item has an owner and a reason)

| Item | To | Why |
|---|---|---|
| Parameters and binds (SQL-PARAM-01, HV-EXEC-03), template detection (PARAM-02), `HvNorm1`/fingerprints (PARAM-03) | M3b | SPLIT DECISION; `:name`/`$1` are sent as written in M3a and the server rejects them |
| Incremental relex and resplit (SQL-LEX-03, SQL-SPLIT-05), `StreamSplitter`, the ≤ 2 ms keystroke budget (R-M3-9) | M3b | SPLIT DECISION; R-M3-22 bridges M3a |
| Executable-comment threshold views, `sql-ident`, `sql-safety` core, PG backend with the generated visitor, interim sqlparser backend, function data, orchestrator, SAFE-06 runs A/A′ with the classifier on, MySQL `sql_mode` pinning in the guard, lexer/splitter fuzz targets and the PG regression differential | M3b | SPLIT DECISION; R-M3-34 |
| The gate and confirmations (R-M3-14), read-write on production, D-062 EXPLAIN ANALYZE wrap, `RunInTxThenConfirm` with the D-014 deadline, the implicit-commit warning, Smart commit, `CALL` denial on read-only (needs L2) | M3b | They need the classifier |
| Error policy Ask/IgnoreAll and PostgreSQL roll-back-and-continue; the error-policy bar | M3b | R-M3-26 |
| Multiple result sets per unit and "Run as one" | M3b | R-M3-24, R-M3-25 |
| History store, auditor chain sinks beyond the log, the history dialog | M3b | SPLIT DECISION |
| Notice text and MySQL warnings in the log; Count rows; eviction; the fetch-all confirmation dialog; large-value messages and `results.max_value_bytes` | M3b | Scope (R-M3-32, R-M3-33) |
| Full type matrix (arrays, composites, ranges, network, bit, geometry, `money` decoders) | M3b | R-M3-31 |
| M2 carries C3 (startup scripts), C6, C7, C8, C9 (queue cap), C10, C11, C13 | M3b | SPLIT DECISION ("M2 carries") |
| Multi-cursor, column selection, IME mirroring (D-072), clipboard metadata, Linux primary selection | M3b | Digest 02 split |
| Folding and the chunked height map; find panel (R-M3-10); gutter extras; Undo selection; mixed line endings and Normalize; comment/duplicate/delete/move-line commands; `[editor]` Settings-UI rows | M3b | Digest 02 split; R-M3-44 |
| Frozen-left columns, grow/shrink, multi-rect selection; local sort with collation; grid find; go to row; transposed, record and text views; the value viewer; the grid context menu; double-click auto-fit; the grid AccessKit subtree (HV-GRID-16) | M3b | R-M3-32 |
| Result-tab comment titles and pinning; the grid raw-display setting | M3b | R-M3-44; M3a titles are `Result N` |
| Transaction badge with buttons, mode/isolation chip, aborted banner, idle-transaction toast, single-session toggle; confirmation, confirm-in-transaction and cancel-escalation dialogs; slow-cancel toast | M3b | R-M3-27 keeps M3a minimal |
| DESIGN.md "Query, Results and Grid" section; B1-real re-run | M3b | R-M3-30 |
| CHOOSE-06 "Customize…" in the chooser | M3b | R-M3-36 |
| A Grid "select column" chord | M3b | P25: `ctrl-space` never reaches the app on macOS or under a CJK IME |
| HV-EXEC-13 explain plan model and views | M4 | R-M3-21 |
| Table row estimates (HV-RES-10 part); SQLite WAL readers (budget stays 1) | M4 | Needs the catalog; M2b B16 |
| T2 large-file preview tier, the 20–100 MB paste confirmation (R-M3-49) and "Run as script" for huge files; opening files and encoding detection | M4b | They need files (UI-EDIT-23, spec §4 Execution) |
| Soft wrap (D-071); L2 syntactic highlighting; `SmallestSubqueryOrStatement` (CST); replace; parser folds; completion, diagnostics, hover; the inactive-window IME blur (M0 gap 2); result-tab titles from the source table | M5 | Need the parser or are M5 by spec |
| HV-RES-06 table paging, HV-RES-09 origin badges (M3a already captures origin hints), HV-RES-11/-12; editing; HTML and private clipboard flavours; export including "Export the rest"; inspector statistics; column reorder; width persistence across restarts | M6 | spec §6 M6 |
| MCP observer and remote-audit sinks; PgBouncer detection; the C11 read-only credential | M8 | Their consumer is M8 |
| VoiceOver/NVDA walkthroughs that gate 1.0 (L4); the Windows and Linux IME matrix (needs PO-provided hosts, R-M3-7) | M9 / open item | L4; hosts |
| Rich copy, visual bidi movement, inlays, the tree view, variable row heights, spill to disk | v1.x | spec §4 deferrals |

## New UI surfaces (need mocks; Task 2)

M3a adds these surfaces. None has a mock yet. Task 2 mocks all of them in `design/m3a-query/` in the harman world, EN/TR × dark/light, at 1280 × 800 and 960 × 600, with the M2a mock bar. Surfaces 1–14 are built in M3a; the mock also sketches the M3b surfaces the M3a ones must leave room for (marked *M3b sketch*), so the layout does not move later.

1. **Console editor area.** Empty console; SQL with lexical colours (comments italic, executable comments their own tint); Turkish identifiers and strings; a long line scrolled horizontally; focused (caret 2 × 16 px `blue`) and unfocused (dimmed selection, no caret); a selection on one line and across lines; line numbers at 1, 4, 5 and 7 digits (the gutter widens: 58 px + (digits − 3) × advance); current line number at `text` ink, others `text-3`; the IME composition underline with a note on the candidate window. Variants: read-only connection; **production** (1 px red hairline over the pane, tab swatch, connection name in mono); **M3a production read-only notice**.
2. **Editor toolbar** (Overflow Rule). Run / Stop (Stop while running, the 7 px amber run marker and a mono elapsed clock); "Cancelling…"; the connection label (mono, environment mark); the read-only lock chip; the production read-only notice; "Waiting for a free connection to orders-dev (4 of 4 in use)" with Cancel; the toolbar collapsed at 960 in `tr`.
3. **Run preview band and current-statement bar.** One multi-line statement (harman band, `blue_tint` at .55 dark / .8 light); two statements on one line (full-row band vs exact-range tint: decide); a selection that runs as separate statements (bars with separators); AskPreselected; Nothing, with "Nothing to run here" in the status bar; **Uncertain** (a heuristic boundary, drawn with existing tokens); NoConnection (muted); the chooser hover override; frozen while composing.
4. **Statement chooser (D-011).** A popup at the caret listing the candidates in resolve order (Statement, Largest statement, From caret, Whole script) with whitespace-collapsed labels (no access keys, R-M3-36) and the preselected item; hover previews the range; the **"boundary uncertain"** state with no run item (R-M3-24).
5. **Gutter execution status.** Queued; Running (spinner + "3 s", once a second; no spinner under 50 ms); Succeeded (green check, 1 px green executed-range bar, end-of-line "executed · 42 ms · 100 rows"); Succeeded DML ("12 rows affected"); Failed (**ink** alert glyph, never red; tooltip with SQLSTATE and the server message verbatim); Cancelling (amber); Cancelled; Skipped; Stale (dimmed, "Edited since execution"); two marks on one row; the end-of-line status colliding with a long line.
6. **Status bar segments.** "Ln 14, Col 49"; selected-character count; "Reduced mode" (T1 tier); **"Transaction open · 3 statements"** (R-M3-27, read-only) and its **Failed** form after an error or a server cancel inside the transaction (R-M3-16 amended); rows · timing of the active result (mono, `text-2`); "Cursor open" (a button that closes the cursor); the production segment following the active result tab vs a staging one; the collapse order at 960.
7. **Large-file and clipboard notices.** The T1 banner (inline-notice anatomy); the paste refusal when the document would pass 20 MB (R-M3-49); the NUL warning toast; clipboard unavailable. *Sketch:* the M4b T2 preview banner and its 20–100 MB paste confirmation.
8. **Editor context menu.** Cut, Copy, Paste, Select all · Run (⌘↵), Describe execution range. *M3b sketch:* Find…
9. **Results tile.** Empty (the M1 state, unchanged); first rows arriving (static skeleton rows); rows; zero rows (header only, "0 rows"); update count only (Output tab active with its unread dot); an error (Output tab with the error line); two units → "Result 1", "Result 2"; a staging result tab (hollow swatch, "(staging)"); more than 2,000 columns (the hint). Tab anatomy from `TabStrip`, Output first.
10. **Grid cells and header.** Header: name in `body-strong`, mono type subtitle (`numeric(14,2)`) at `text-3`, numeric columns right-aligned. Selection: range, active cell, 1 px `blue` box with a 5 px handle, selected row numbers in `blue`, a row set, a column set, select-all from the corner; hover outline. Cells: the `NULL` marker (synthesized small caps, `text-3`) next to the text `'NULL'`; `''`; truncated with the `… 5 MB` chip; binary as hex and as UUID; control chips (`↵`, `→`, `⟨RLO⟩`); numbers EN vs TR and an ID column raw; timestamps (mono 11.5 at `text-2`, as harman `td.ts`: confirm); booleans; a JSON preview; long Turkish text ellipsized; 8-digit row numbers; the decode-error cell (ink glyph). The scrollbar with the `green-fill` loaded band. *M3b sketch:* sort indicator, frozen-left column.
11. **Cursor strip** (26 px, `raised`, `caption` at `text-2`). Streaming ("Fetching… 3,500 rows", amber marker); Suspended ("10,000+ rows", "Cursor open — holding a database snapshot", Close cursor, Fetch more, Fetch all); Complete; Closed with each reason (another statement needed the session, idle, transaction ended, connection lost) and Re-run; Cancelled ("Cancelled after 3,500 rows"); the memory banner ("Memory limit for this result reached (N rows, X MB)", Fetch more anyway, Close cursor); "2 more result sets not shown" (R-M3-25); selection dimensions on the right.
12. **Output log** (21 px rows, ink glyphs). Rows retrieved; rows affected; completed; transaction committed / rolled back (from `tx.commit`/`tx.rollback`); an error (localized title, mono SQLSTATE chip, verbatim server text in its own field; click selects the statement in the editor); the run summary ("2 of 3 statements executed, 1 failed"); cancelled; read-only denial; "n events not shown"; toolbar Clear / Copy / Scroll to end; empty.
13. **Close with an open transaction** (420 px alertdialog): "orders-dev has an open transaction with 3 statements". Commit / Roll back / **Cancel (focused)**. Variants: console close; `app.quit`; a **Failed** transaction (Roll back / **Cancel (focused)** only; R-M3-16 amended).
14. **Copy feedback.** Toast "Copied 3 × 120 cells" (locale-formatted counts); refusal over 1,000,000 cells or 100 MB ("Too much to copy; export arrives later" — copy decided in the mock).

## Waves

The work is interface-first. Task 1a proves the build (the fork rev, the vendored `pg_query` on three OSes); Task 1b commits every cross-stream type with fail-closed stubs and the working helpers listed in "Inherited pre-flight lessons" (Q8). Task 2 runs beside both.

| Wave | Tasks (agent model; gpui marked) | Unblocked by |
|---|---|---|
| W0 | **T1a** proofs (opus, gpui, `<main>/target`) · **T1b** seams (opus, gpui, `target-core/m3a-t01b`) · **T2** mocks (opus) | — (T1b: Task 1a's first commit) |
| W1 | **T3** `sql-text` + keyword tables (opus) · **T4** `sql-lexer` (opus) · **T5** `hacivat-text` core (opus) · **T6** `hacivat-values` model + chunks (opus) · **T7** `hacivat-results` store (opus) | T1a and T1b merged |
| W2 | **T8** `sql-split` (opus) · **T9** `sql-oracle-pg` (opus) · **T10** selections, units, IME core (opus) · **T11** display model (sonnet) · **T12** PostgreSQL typed + `tokio-postgres` patch §1 (opus) | T3, T4 (T8); T5 (T10); T6 (T12) |
| W3 | **T13** `exec_range` + D-011 goldens (opus) · **T14** MySQL typed (opus) · **T15** SQLite typed (opus) · **T16** `valfmt` (opus) · **T17** grid model (opus, gpui) | T8 (T13); T6 (T14–T16) |
| W4 | **T18** session Run, cursors, production read-only (opus) · **T19** editor view + element (opus, gpui) · **T20** grid element (opus, gpui) · **T21** Overflow Rule toolbar (sonnet, gpui) | T7, T12, T14, T15 (T18); T5, T10, T11 (T19); T17 (T20) |
| W5 | **T22** typed read-only path (opus) · **T23** `hacivat-exec` (opus) · **T24** SQL adapter (opus, gpui) · **T25** editor commands (opus, gpui) · **T26** grid adapter + results model + copy (opus, gpui) | T18 (T22, T23); T13, T9 (T23); T8, T13, T19 (T24); T19 (T25); T7, T16, T20 (T26) |
| W6 | **T27** IME handler + clipboard (opus, gpui) · **T28** gutter, marks, preview (opus, gpui) · **T29** a11y baseline + large-file tiers (opus, gpui) · **T30** results tile, strip, output log (opus, gpui) · **T31** typed-execution conformance over Docker (opus) | T19 (T27–T29); T20, T21, T23, T26 (T30); T12, T14, T15, T18, T22, T23 (T31) |
| W7 | **T32** console host (opus, gpui) · **T33** execution glue (opus, gpui) · **T34** status bar and capsule follow the console (sonnet, gpui) | T21, T24, T25, T27–T29 (T32); T19, T23, T26 (T33); T26 (T34) |
| W8 | **T35** shots and cross-surface tests (opus, gpui) · **T36** real-app benches B3 and B1-real (opus, gpui) | W7 merged |
| W9 | **T37** M3a gate (sonnet, with the controller) | everything |

- W1 and W2 are core-only (`hacivat-text` is gpui-free by `TextCoreNotHeadless`), so five agents can run under the core admission rule (≥ 8 GiB each).
- **gpui caps (P14):** 3 gpui agents while the Docker matrix is up (W1–W6), 4 while it is down (W7), 2 whenever less than 60 GiB is free; `CARGO_BUILD_JOBS` (4 core, 6 gpui) whenever 3 or more agents build; a memory check before each dispatch. Suggested order: W4 T19 + T20 + T21 beside T18; W5 T24 + T25 + T26 beside T22 and T23; W6 T27 + T28 + T30 beside T31, then T29 as one finishes; W7 T32 + T33 + T34 (P11 and P12 make the same-wave editor and app tasks independent).
- W8 has two tasks and W9 one: shots and the real-app benches both need every UI task merged, and the gate needs everything.
- **Critical paths:** SQL T3/T4 → T8 → T13 → T23 → T33; editor T5 → T19 → T27/T28 → T32 → T36; execution T6 → T12 → T18 → T22/T23 → T31; grid T17 → T20 → T26 → T30 → T36.
- **Docker:** the controller brings the stack up (`cargo xtask docker up --set full`) before W1 (Task 3's `--from-docker`, P19) and keeps it up through W6; in W8 it brings `pg17` up again for Task 36's `b1_real`. Tasks 3 (one read-only query per server), 12, 14, 18, 22 and 31 use it, the tests through the lock wrapper only.
- **CI proof for Task 1a:** the controller pushes `m3a-proofs` and waits for `build-test` on macOS, Windows and Linux plus a `workflow_dispatch` `nightly-package` run before merging Task 1a (R-M3-1). **Task 1b** merges after Task 1a, after its review and after `build-test` × 3 on `m3a-seams` (Windows compiles the new test files first). W1 starts when both have merged.
- **W0 order:** P2 and P3 settled (base `5d5832f`; threshold 98 %) → this amend pass → the controller's pre-dispatch steps (rules copied, `baseline-m2b/`) → Task 1a and Task 2 dispatched → Task 1a's fork commit, pushed by the controller → Task 1a's first commit (rev bump, vendored `pg_query`, build proof green locally) → Task 1b dispatched on it → Task 1a merges after green CI → Task 1b rebases, is reviewed, goes green and merges → W1.

**File ownership.** "Owns" means creates or modifies. Anything not listed belongs to Task 1b, which creates every crate (except Task 1a's minimal `sql-oracle-pg`), every `lib.rs` module line and every stub, and puts each type in the module file of the task that fills it, never in `lib.rs` (P22).
- **T1a:** `third_party/pg_query/**`; `crates/sql/sql-oracle-pg/{Cargo.toml,src/lib.rs,tests/build_proof.rs}` (minimal; Task 1b adds the module lines and stubs); in `Cargo.toml` the GPUI `rev`, `members += "crates/sql/*"`, the `pg_query`, `text-size` and `sql-oracle-pg` workspace lines, `[patch.crates-io] pg_query` and `[profile.dev.package.pg_query]`, and their `Cargo.lock` entries; `deny.toml`; `THIRD_PARTY_NOTICES.md`; `packaging/Packager.toml`; `.github/actions/{libclang,linux-deps}/action.yml`; the `ci.yml` steps marked `# ## m3a-t01a-proofs`; `crates/bin/hacivat/{Cargo.toml,src/smoke.rs}` (named lines); `crates/app/hacivat-app/tests/teardown_stress.rs`; `crates/ui/hacivat-ui-core/patches/{README.md,scheduler-*.patch}`; ADR 0005 (creates it; section `m3a-t01a`).
- **T2:** `design/m3a-query/{index.html,NOTES.md}`.
- **T3:** `crates/sql/sql-text/src/{line_index,edit,chunks}.rs`; `crates/sql/sql-dialect/src/{keywords,version,modes,id}.rs`, `src/generated/keywords_*.rs`, `data/keywords/*.toml`; `xtask/src/gen_sql_tables.rs` (and its one dispatch line in `xtask/src/main.rs`); tests under both crates; a `ci.yml` block.
- **T4:** `crates/sql/sql-lexer/src/**` except `lib.rs` module lines; `crates/sql/sql-dialect/src/lex_profiles/**`; `crates/sql/sql-lexer/tests/**`; `crates/sql/sql-testkit/corpus/lex/*.toml`.
- **T5:** `crates/ui/hacivat-text/src/{buffer,change,anchor,changelog,model,transaction,history}.rs`; `crates/ui/hacivat-text/tests/{buffer,change,change_props,model,history}.rs`.
- **T6:** `crates/core/hacivat-values/src/{logical,native,value,decimal,temporal,special,bind,columnar/**}.rs`; `crates/core/hacivat-values/tests/{decimal,temporal,columnar,perf_cell}.rs`.
- **T7:** `crates/core/hacivat-results/src/{store,snapshot,writer,window,budget,cursor,fixture}.rs`; its tests.
- **T8:** `crates/sql/sql-split/src/{client,splitter,recover,item,key,document,head}.rs`; `crates/sql/sql-dialect/src/split_profile/**`, `data/split/*.toml`; `crates/sql/sql-split/tests/{split,document,head,split_props,sqlite_differential}.rs`; `crates/sql/sql-testkit/corpus/fixtures/FX-SPLIT-*.sql`; the `alg/10` errata paragraph (named exception).
- **T9:** `crates/sql/sql-oracle-pg/src/**` except Task 1b's `lib.rs` lines (Task 1a's working `version`/`statement_count` may move into `split.rs` or `error.rs`, unchanged); its tests.
- **T10:** `crates/ui/hacivat-text/src/{selection,units,ime}.rs`; `tests/{selection,units,ime_traces}.rs`.
- **T11:** `crates/ui/hacivat-text/src/display/**`; `tests/display.rs`.
- **T12:** `third_party/tokio-postgres/src/{client,query,bind}.rs`, `third_party/tokio-postgres/PATCHES.md` (named exception); `crates/core/hacivat-driver-pg/src/{typed,decode/**}.rs`, the typed `RawConn` methods in `src/probe.rs` (named lines); `crates/core/hacivat-driver-pg/tests/{tp_patches,decode,docker_typed}.rs`; an ADR 0002 section.
- **T13:** `crates/sql/sql-split/src/exec_range/**`; `crates/sql/sql-testkit/src/golden/**`, `corpus/datagrip/**`, `corpus/fixtures/FX-CARET-*.sql`; `crates/sql/sql-split/tests/{d011_goldens,exec_range,exec_range_props,pg_differential}.rs`.
- **T14:** `crates/core/hacivat-driver-mysql/src/{typed,decode}.rs`, the typed `RawConn` methods and the `@@net_write_timeout` read in `src/connect.rs` (named lines); `tests/{decode,docker_typed}.rs`; an ADR 0002 section.
- **T15:** `crates/core/hacivat-driver-sqlite/src/{typed,decode}.rs`, the typed job and methods in `src/open.rs`, the `catch_unwind` around column metadata in `src/exec.rs` (named lines); `tests/{typed,typed_hostile}.rs`; an ADR 0002 section.
- **T16:** `crates/core/hacivat-values/src/valfmt/**`; `tests/{valfmt_display,valfmt_raw}.rs`.
- **T17:** `crates/ui/hacivat-ui-grid/src/model/**`; `tests/model_*.rs`.
- **T18:** `crates/core/hacivat-session/src/session/{run,cursor}.rs`; named lines in `session/{actor,cancel,liveness,pool}.rs` and `manager.rs`; `tests/{run,cursor,prod_read_only,docker_m3a_cursor}.rs`; an ADR 0004 section.
- **T19:** `crates/ui/hacivat-ui-editor/src/{view,element,layout_cache,paint,mouse,theme,probe,scroll}.rs`; `tests/{element,view_scene}.rs`.
- **T20:** `crates/ui/hacivat-ui-grid/src/{element/**,view.rs,list.rs,testing.rs}`; `examples/b1_synthetic.rs` (Task 1b's stub); `tests/{element,view,list}.rs`; the `ci.yml` bench step replacing `spike-grid` (one block).
- **T21:** `crates/ui/hacivat-ui/src/toolbar.rs`; `crates/ui/hacivat-ui/tests/toolbar.rs`.
- **T22:** `crates/core/hacivat-session/src/session/readonly.rs` (the typed path); `tests/{readonly_typed,docker_m3a_readonly}.rs`; an ADR 0004 section.
- **T23:** `crates/core/hacivat-exec/src/**` except Task 1b's `lib.rs` lines; its tests (helpers in a block of `tests/common/mod.rs`); an ADR 0006 section.
- **T24:** `crates/app/hacivat-sql-editor/src/**`; its tests.
- **T25:** `crates/ui/hacivat-ui-editor/src/commands/**`; `tests/commands.rs`; FTL block `hv-cmd-editor-*`.
- **T26:** `crates/app/hacivat-app/src/results/{adapter,cache,tabs,context,copy}.rs`; `tests/results_model.rs`. `results/mod.rs` (module lines, re-exports, `install`) stays Task 1b's (P12).
- **T27:** `crates/ui/hacivat-ui-editor/src/{ime,clipboard}.rs`; `tests/{ime_view,clipboard}.rs`.
- **T28:** `crates/ui/hacivat-ui-editor/src/{gutter,marks,preview,scrollbar}.rs`; `tests/{marks,gutter_scene,preview}.rs`.
- **T29:** `crates/ui/hacivat-ui-editor/src/{a11y,large_file}.rs`; `tests/{a11y,large_file}.rs`.
- **T30:** `crates/app/hacivat-app/src/results/{view,toolbar,strip,output_view}.rs` (the body of `results::view::build`, which Task 1b registers); `tests/results_view.rs`.
- **T31:** `crates/core/hacivat-exec/tests/docker_m3a_matrix.rs` and its helpers block `// ## m3a-t31-matrix` in `crates/core/hacivat-exec/tests/common/mod.rs` (P6).
- **T32:** `crates/app/hacivat-app/src/console/**` except Task 1b's `console/mod.rs` (module lines, re-exports, `install`) and the names Task 1b declares; the "New console" entry in `connections/home.rs:20-32` (named lines); `tests/console.rs`.
- **T33:** `crates/app/hacivat-app/src/exec_ui/**`; `crates/ui/hacivat-ui-core/src/platform/memory.rs` (`physical_memory_bytes`, named function); the debug demo removal (R-M3-47); `tests/exec_ui.rs`.
- **T34:** `segments()` in `crates/app/hacivat-app/src/connections/status.rs` and `current_profile()` in `connections/capsule.rs` (named functions); `tests/status_follow.rs`.
- **T35:** `crates/app/hacivat-app/tests/shots.rs` (surfaces after the existing 78; `--shard k/n`; grid text checks) and `tests/cross_surface_m3a.rs`.
- **T36:** `crates/app/hacivat-app/src/bench/**` and `examples/{b3,b1_real}.rs` (Task 1b's stubs); the `ci.yml` bench block; `docs/benchmarks/m3a-ime-checklist.md`.
- **T37:** `docs/benchmarks/{m3a.md,po-demo-m3a.md}`; `design/baselines/m3a/**` after PO sign-off.

Each UI task appends its own FTL block and owns the shots fixtures of its surfaces as public `#[cfg(feature = "shots")]` fixture builders in its own modules; Task 35 consumes them.

---

## File Structure

```
hacivat/
  Cargo.toml                         # members += "crates/sql/*"; workspace deps (Task 1a/1b tables); [patch.crates-io]
                                     #   pg_query, tokio-postgres = { path = "third_party/…" }; gpui/gpui_platform rev bump (T1a)
  deny.toml                          # [[licenses.clarify]] + [[licenses.exceptions]] for pg_query (R-M3-1)
  THIRD_PARTY_NOTICES.md             # NEW: PostgreSQL License, BSD-3 (libpg_query, upb), BSD-2 (xxHash)
  packaging/Packager.toml            # new resources key: THIRD_PARTY_NOTICES.md (T1a)
  third_party/pg_query/              # NEW: 18.1.0 pristine + PATCHES.md §0–§1 (T1a: no protobuf regeneration)
  third_party/tokio-postgres/        # NEW: 0.7.18 pristine (T1b) + PATCHES.md §0–§1 (T12)
  docs/decisions/{0005-sql-text-and-safety, 0006-execution-and-results, 0007-editor-and-grid}.md   # NEW
  docs/benchmarks/{m3a.md, po-demo-m3a.md, m3a-ime-checklist.md}                                 # NEW
  design/m3a-query/{index.html, NOTES.md}                                                        # NEW
  .github/workflows/ci.yml           # m3a blocks: libclang steps, size print, test lines 6–8, gen-sql-tables --check, bench
  .github/actions/libclang/action.yml     # NEW (T1a): clang --version, LIBCLANG_PATH, libclang-dev, protoc guard
  .github/actions/linux-deps/action.yml   # + libclang-dev
  xtask/Cargo.toml                   # + phf_codegen
  xtask/src/{check_deps.rs (4 rules, PRODUCT_CRATES), docker.rs (--test, hacivat-exec), l10n/code_scan.rs (exec, cursor), gen_sql_tables.rs (NEW), main.rs (one arm)}
  crates/sql/                        # NEW family (Core layer)
    sql-text/src/{lib, line_index, edit, chunks}.rs
    sql-dialect/src/{lib, id, modes, version, keywords, lex_profiles/*, split_profile/*, generated/*}.rs; data/{keywords,split}/*.toml
    sql-lexer/src/{lib, cursor, state, token, comments, strings, dollar, numbers, params, ops, words, exec_comment, class, relex}.rs
    sql-split/src/{lib, client, splitter, recover, item, key, document, head, exec_range/{mod,candidates,policy}}.rs
    sql-oracle-pg/src/{lib, error, split, stack}.rs
    sql-testkit/src/{lib, golden/{mod,markers,runner}}.rs; corpus/{lex,datagrip,fixtures}/**   # publish = false, dev-only
  crates/ui/
    hacivat-ui-core/src/{gpui_surface.rs (re-exports), platform/memory.rs (+ physical_memory_bytes)}; patches/{README.md, scheduler-*.patch}
    hacivat-ui/src/toolbar.rs        # NEW (T21)
    hacivat-text/src/{lib, buffer, change, anchor, changelog, selection, units, model, transaction, history, ime, display/{mod,rangeset,compose,height,scroll}}.rs  # NEW
    hacivat-ui-editor/src/{lib, config, service, decor, view, element, layout_cache, paint, mouse, scroll, theme, probe, commands/*, ime, clipboard, gutter, marks, preview, scrollbar, a11y, large_file}.rs  # NEW
    hacivat-ui-grid/src/{lib, source, model/{mod,scroll,columns,selection,keys,autosize}, element/*, view, list, testing}.rs; examples/b1_synthetic.rs  # NEW
  crates/core/
    hacivat-values/src/{lib, logical, native, value, decimal, temporal, special, bind, columnar/*, valfmt/{mod,display,raw,idlike}}.rs  # NEW
    hacivat-results/src/{lib, store, snapshot, writer, window, budget, cursor, fixture}.rs                                         # NEW
    hacivat-exec/src/{lib, units, sqlctx, service, observer, status, log, ux, titles}.rs                                          # NEW
    hacivat-driver/src/{typed.rs (NEW), driver.rs, fake.rs}
    hacivat-driver-pg/src/{typed.rs, decode/*}.rs; -mysql/src/{typed,decode}.rs; -sqlite/src/{typed,decode}.rs                   # NEW files
    hacivat-session/src/session/{run.rs, cursor.rs (NEW), mod.rs, actor.rs, readonly.rs, pool.rs}; manager.rs; test_conn.rs; testing.rs (NEW, `test-support`)
    hacivat-exec/tests/{common/mod, docker_m3a_matrix}.rs                                                                         # T23, T31
    hacivat-settings/src/schema.rs   # + [editor]
  crates/app/
    hacivat-sql-editor/src/{lib, service, worker, highlight, statements, preview, words, chunks}.rs   # NEW
    hacivat-app/src/{console/*, results/*, exec_ui/*, bench/*}.rs (NEW); shell/{hooks,workspace}.rs; menus.rs; connections/{status,capsule,home}.rs
    hacivat-app/examples/{b3, b1_real}.rs                                                              # NEW (T1b stubs; T36)
  crates/bin/hacivat/src/smoke.rs    # + the libpg_query line (T1a)
  locales/{en,tr}/{hacivat-ui,hacivat-core,hui}.ftl   # appended blocks
```

---

**How to read the test blocks.** Each task's Step 1 (Task 1a's Step 2) lists its tests by name, with the assertion written out where the spec or a ruling fixes a value. A test shown with an empty body (`{}`) or a comment carries its assertion in its name and in the spec case it cites (for example "UI-EDIT-04 cases 1–8" in `alg/25`); the implementer writes the body with real asserts (F9), and the reviewer rejects a tautological or comment-only body. Helper names inside test bodies (`session`, `fake_pg`, `resolve_at`, …) are test-local helpers the task writes in its own test files or in its append block of a shared `common`/`support` module.

### Task 1a: Proofs (GPUI fork scheduler patch and rev bump, vendored `pg_query` with the libpg_query build proof on three OSes, licences and notices, CI libclang action, teardown stress, size baseline) (W0)

**Build:** `CARGO_TARGET_DIR=<main>/target`, `CARGO_INCREMENTAL=0`. Measure the size baseline before the first build. The GPUI rev bump invalidates every gpui build; after this task merges the controller cleans every stale `target-core/*` directory except Task 1b's.

**Files:**
- Create:
  - `third_party/pg_query/**`: a pristine copy of `~/.cargo/registry/src/*/pg_query-18.1.0` (Q29: its `Cargo.toml` drops `[dev-dependencies]`, `[[test]]`, `[[bench]]`, `[[example]]` and profile sections; the copy omits `Cargo.toml.orig`, `Cargo.lock`, `tests/`, `benches/` and CI files). `PATCHES.md`: §0 "vendored copy; test, bench and CI files removed"; **§1** "`build.rs`: the protobuf regeneration branch removed (it ran when `REGENERATE_PROTOBUF` was set or `protoc --version` succeeded, and rewrote `src/protobuf.rs` inside the crate directory); the shipped pre-generated `src/protobuf.rs` is always used; bindgen is kept", the change fenced by `// HACIVAT PATCH §1` (R-M3-1, P1).
  - `crates/sql/sql-oracle-pg/{Cargo.toml,src/lib.rs,tests/build_proof.rs}`: the minimal crate, with `version`, `statement_count` and `PgError` working and the signatures of Task 1b's Interfaces (`PgError.cursor` is `text_size::TextSize`, the type `sql_text` re-exports). Task 1b adds every other module line and stub.
  - `THIRD_PARTY_NOTICES.md`: the PostgreSQL License (PostgreSQL source and `kwlist.h` inside libpg_query), BSD-3 (libpg_query, upb), BSD-2 (xxHash), each with its copyright lines copied from the files inside `third_party/pg_query`.
  - `docs/decisions/0005-sql-text-and-safety.md`: Status, Context, a Rule heading naming the per-task section format, and this task's section `## m3a-t01a — proofs`.
  - `crates/ui/hacivat-ui-core/patches/scheduler-block-until-lock-order.patch` and its section in `patches/README.md`.
  - `.github/actions/libclang/action.yml` (composite, P10): prints `clang --version`; on Windows exports `LIBCLANG_PATH=C:\Program Files\LLVM\bin` when `libclang.dll` exists there; on Linux installs `libclang-dev`; on every OS the protoc guard, which after §1 only prints `protoc --version` when one is present (the vendored `build.rs` never runs it).
  - `crates/app/hacivat-app/tests/teardown_stress.rs` (R-M3-13): today's app only, with a test-local boot-and-shutdown helper.
- Modify:
  - `Cargo.toml`: `members` += `"crates/sql/*"`; workspace deps `pg_query = "=18.1.0"`, `text-size = "1.1.1"` and the path entry for `sql-oracle-pg`; `[patch.crates-io] pg_query = { path = "third_party/pg_query" }` (the existing `exclude = ["third_party"]` covers it); `gpui`/`gpui_platform` `rev` = the pushed fork commit; `[profile.dev.package.pg_query] opt-level = 1`. `Cargo.lock`.
  - `deny.toml`: `[[licenses.clarify]] crate = "pg_query"`, `expression = "MIT AND BSD-3-Clause AND BSD-2-Clause AND PostgreSQL"`, `license-files` with the hashes `cargo deny` reports for the crate's `LICENSE` and `libpg_query/LICENSE`; `[[licenses.exceptions]] crate = "pg_query", allow = ["PostgreSQL"]` with a comment citing R-M3-1.
  - `packaging/Packager.toml`: a **new** `resources` key (there is none today) with `THIRD_PARTY_NOTICES.md`, spelled so that it resolves from the rendered `packaging/.Packager.generated.toml` (`package-macos.sh:50`) the way cargo-packager resolves a relative path (P23).
  - `.github/actions/linux-deps/action.yml` (`libclang-dev`).
  - `.github/workflows/ci.yml`: one `uses: ./.github/actions/libclang` step, marked `# ## m3a-t01a-proofs` (a named exception to append-only), in every job that builds a crate reaching `pg_query`: `build-test`, `turkey-test`, `switch`, `launch-smoke`, `bench`, `nightly-package`, `pseudo-screens`, `docker-pr` and `docker-full` (Task 1b adds `hacivat-exec` to the Docker crates), and any other job that builds `hacivat` or `hacivat-app`. In `build-test` after Build: `ls -l target/release/hacivat*` (binary size) and `git diff --exit-code -- third_party/pg_query` (a build never rewrites the vendored source). `nightly-package` needs no other change; the controller triggers it by `workflow_dispatch` on `m3a-proofs`.
  - `crates/bin/hacivat/Cargo.toml` (`sql-oracle-pg`) and `src/smoke.rs` (named lines): `--smoke` also prints `sql_oracle_pg::version()` and the result of `statement_count("SELECT 1")`. Until Task 1b nothing in the binary reaches `pg_query`, so without this line the universal bundle would neither compile libpg_query for both architectures nor show a real size delta.

**Interfaces:**
- Consumes: the GPUI fork at `a1eaa20`; the registry source of `pg_query` 18.1.0 (read once, to copy).
- Produces: the new GPUI rev; `third_party/pg_query`; `sql_oracle_pg::{version, statement_count, PgError}` working (the build proof); the CI libclang action; the size baseline and the proof record in ADR 0005.

- [ ] **Step 1: Size baseline, licence check, the fork commit (R-M3-13, P13).**
  1. Record `ls -l <main>/target/release/hacivat` (37.1 MB arm64 at the M2b gate) for ADR 0005 **before** anything rebuilds it.
  2. Read `crates/scheduler/Cargo.toml` at the head of the fork branch `hacivat/912159c` (`https://github.com/berkaygkc/zed`, named in `Cargo.toml:21-26` and `crates/ui/hacivat-ui-core/patches/README.md`): `license = "Apache-2.0"` and a `LICENSE-APACHE` file. Anything else: stop, report BLOCKED.
  3. `git clone --filter=blob:none --single-branch --branch hacivat/912159c https://github.com/berkaygkc/zed <scratchpad>/zed-fork` (never under `<main>`, never `.refs/`). `git rev-parse HEAD` must be `a1eaa20480d5bfe62e21ab1248e3900232dab92b`, else BLOCKED.
  4. In `crates/scheduler/src/test_scheduler.rs`, `block_until` (`:112-116`) evaluates its argument under the `rng` guard and then locks `state`, while `schedule_local` (`:609-618`) and `schedule_background_with_priority` (`:641-643`) lock `state` then `rng`. Read `timeout_ticks` under `state`, drop that guard, then lock `rng`. Audit `:132` (a temporary guard in the `while` condition), `:252` (`rng` alone) and `:388`/`:391` (`rng` under `state`): the order is always `state` before `rng`, and no `rng` guard is held while `state` is locked. Add the fork's §4(b) header comment to the file; in `HACIVAT-PATCHES.md` add the patch line and change the scope sentence to "Nothing outside `crates/gpui*` and `crates/scheduler` is changed, apart from this file". Change nothing else.
  5. `cargo +1.98.1 check -p scheduler` in the clone (the explicit toolchain keeps the fork's `rust-toolchain.toml` from pulling its three extra targets; no gpui build). Commit `scheduler: take state before rng in block_until (Hacivat carry)` with the trailer; `git rev-parse HEAD^` must be `a1eaa20480d5bfe62e21ab1248e3900232dab92b`.
  6. **Controller checkpoint.** Write `git format-patch -1` and the commit SHA to `<main>/.superpowers/sdd/2026-11-14-m3-walking-skeleton/t01a-fork-commit/`, leave the clone in place and hand back. The **controller** checks the patch (only `test_scheduler.rs` and `HACIVAT-PATCHES.md`), pushes fast-forward only from the clone (`git -C <scratchpad>/zed-fork push origin HEAD:refs/heads/hacivat/912159c`; never `--force`, never another branch), confirms with `git ls-remote`, logs the SHA and resumes this task with it.
  7. Delete `<scratchpad>/zed-fork` (about 0.5 GB; the 100 MB `/tmp` rule).
- [ ] **Step 2: Write the failing tests.**
```rust
// crates/sql/sql-oracle-pg/tests/build_proof.rs  (the libpg_query build proof, run on all three CI OSes)
#[test] fn pg_query_links_and_counts_statements() { assert_eq!(sql_oracle_pg::statement_count("SELECT 1; SELECT 2"), Ok(2)); }
#[test] fn version_names_postgres_18() { assert!(sql_oracle_pg::version().contains("18")); }
#[test] fn parse_error_is_an_error_not_a_panic() { assert!(sql_oracle_pg::statement_count("SELEC 1").is_err()); }
// crates/app/hacivat-app/tests/teardown_stress.rs (R-M3-13)
#[gpui::test] fn fifty_core_boot_and_quit_cycles_never_hang(cx: &mut TestAppContext) { for i in 0..50 { with_watchdog(secs(10), i, || boot_core_then_shutdown(cx)); } }
```
- [ ] **Step 3:** Run `cargo test -p sql-oracle-pg --test build_proof` and `cargo test -p hacivat-app --test teardown_stress`. Expected: FAIL for the missing crate; the stress test may already trip its watchdog at the old rev (the deadlock is intermittent).
- [ ] **Step 4: The rev bump and the vendored `pg_query` (this task's first commit, Task 1b's base).**
  1. Edit the `rev` of `gpui`/`gpui_platform` in `Cargo.toml` to the pushed SHA and run `cargo metadata --format-version 1 >/dev/null`. No `cargo update`: editing `rev` re-resolves the 21 fork-sourced lock entries; `git diff Cargo.lock` may show only `?rev=` changes on those entries, plus additions.
  2. Copy `third_party/pg_query` (Files), apply §1, add the workspace lines and `[patch.crates-io]`, and create the minimal `sql-oracle-pg`. `diff -r ~/.cargo/registry/src/*/pg_query-18.1.0 third_party/pg_query` shows only the manifest edits, the removed files and §1.
  3. Run `cargo test -p sql-oracle-pg --test build_proof`; then `git status --porcelain -- third_party/pg_query` must be empty (no rewrite, with `protoc` on this machine's `PATH`); then `cargo check -p hacivat-ui-core` (gpui at the new rev). Commit only when all three are green.
  4. Commit `build(m3a): GPUI rev with the scheduler lock-order fix; vendored pg_query 18.1.0 without protobuf regeneration; sql-oracle-pg build proof`. The body lists the GPUI rev move and the `pg_query` lock entry losing `source`/`checksum`. **The controller watches `m3a-proofs` for this commit, records its SHA in the ledger and dispatches Task 1b on it**; this task goes on without waiting.
- [ ] **Step 5: The rest of the proofs.** `deny.toml`, `THIRD_PARTY_NOTICES.md`, `Packager.toml`, the libclang action and `linux-deps`, the `ci.yml` steps, the smoke line, and the patch file with its README section (the change and its reason: ledger, M2b T22/T24 "teardown deadlock"). Record in ADR 0005's section: the macOS release binary size before (Step 1) and after; that `protoc` is on this machine's `PATH` and, after §1, changes nothing; the `clang --version` line. Budget ≤ +10 MiB per architecture, otherwise a ruling inside this task (`opt-level = "s"` for the package, or LTO).
- [ ] **Step 6:** Run Step 3's commands, then `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo xtask check-deps`, `cargo deny --all-features check licenses bans sources`, `test -z "$(cargo tree -i aws-lc-rs --workspace 2>/dev/null)"`, `cargo tree -i pg_query` (only `sql-oracle-pg` as a direct dependent), `cargo build --release -p hacivat` and `target/release/hacivat --smoke` (the libpg_query line), the M2b app suites `cargo test -p hacivat-app --features scene-check,pseudo` (the GPUI rev moved), and `cargo test -p hacivat-app --features shots,scene-check,pseudo --test shots` with `cargo xtask shot-diff --baseline <main>/.superpowers/sdd/2026-11-14-m3-walking-skeleton/baseline-m2b --candidate target/shots`; then `git status --porcelain -- third_party/pg_query` once more. Expected: PASS; every pre-existing shot identical.
- [ ] **Step 7: Commit** `feat(m3a): libpg_query build proof — deny clarify and exception, third-party notices in every bundle, libclang CI action, smoke line; TestScheduler teardown stress`. The controller pushes `m3a-proofs`, waits for `build-test` green on macOS, Windows and Linux and for a `workflow_dispatch` `nightly-package` run (the universal macOS dmg, the NSIS installer and the AppImage, each listing `THIRD_PARTY_NOTICES.md`), records both in the ledger, then merges. A failure on Windows or in the universal macOS build triggers R-M3-1's second step (pre-generated `bindings.rs` in `third_party/pg_query`, `PATCHES.md` §2) as a fix round of this task.

**Review:** opus, adversarial (security). Probes:
- Is the fork commit exactly the lock-order change plus the §4(b) header and the `HACIVAT-PATCHES.md` scope line, with `HEAD^` = `a1eaa20…`, pushed fast-forward to `hacivat/912159c` only?
- Does `Cargo.lock` move only `?rev=` on the fork entries, plus the `pg_query` and `sql-oracle-pg` additions?
- Does `diff -r` against the registry show only the manifest edits, the removed files and §1? Does any build, with `protoc` on `PATH`, leave `third_party/pg_query` changed?
- Does any public signature of `sql-oracle-pg` carry a `pg_query` type?
- Is `THIRD_PARTY_NOTICES.md` complete against the licence files inside `third_party/pg_query`, and listed inside the dmg, the NSIS installer and the AppImage of the `nightly-package` run?
- Does the libclang action reach every job that builds a crate reaching `pg_query`?
- Does any M1/M2 shot move after the rev bump? Does the stress test's watchdog really fail a hung cycle?

### Task 1b: Seams (crates, types, stubs and working helpers, manifests, check-deps rules, vendored tokio-postgres, keymaps, test support) (W0)

**Build:** `CARGO_TARGET_DIR=<main>/target-core/m3a-t01b`, `CARGO_INCREMENTAL=0`. **Base (named exception to GC "Branches"):** `git checkout -B m3a-seams <Task 1a's first commit>` (the controller names the SHA), then `git merge-base --is-ancestor m3a HEAD`; this task builds on the new GPUI rev and the vendored `pg_query` from its first command and never reaches the registry `pg_query`. Before review it rebases onto Task 1a's final commit (`m3a` once Task 1a has merged); `Cargo.toml` and `Cargo.lock` conflicts are mechanical: keep Task 1a's lines, then `cargo metadata`. After this task merges the controller cleans and deletes `target-core/m3a-t01b`.

**Files:**
- Create, crates (each with `Cargo.toml` carrying `[lints] workspace = true`, `license`, `description`, `repository`, `publish = false`, and a `lib.rs` with every module line):
  - `crates/sql/{sql-text,sql-dialect,sql-lexer,sql-split,sql-testkit}/**`, and the module lines and stubs of Task 1a's `sql-oracle-pg` (stubs listed under Interfaces);
  - `crates/ui/{hacivat-text,hacivat-ui-editor,hacivat-ui-grid}/**`;
  - `crates/core/{hacivat-values,hacivat-results,hacivat-exec}/**`;
  - `crates/app/hacivat-sql-editor/**`.
- Create, files in existing crates:
  - `crates/core/hacivat-driver/src/typed.rs`; `crates/core/hacivat-session/src/session/{run,cursor}.rs`; `crates/core/hacivat-session/src/testing.rs` (`test-support`, P6).
  - `crates/app/hacivat-app/src/results/{mod,adapter,cache,tabs,context,copy,view,toolbar,strip,output_view}.rs`, `console/{mod,registry,view,toolbar,decor,autosave,status,commands}.rs`, `exec_ui/{mod,orchestrator,chooser,marks,cancel,tx,close_prompt}.rs` and `bench/mod.rs`: every submodule line declared, bodies empty or stubbed (P12, P7).
  - Fail-closed example stubs (P7): `crates/app/hacivat-app/examples/{b3,b1_real}.rs` and `crates/ui/hacivat-ui-grid/examples/b1_synthetic.rs`, each `fn main() { eprintln!("bench not built yet (Task 20/36)"); std::process::exit(2) }`.
- Create, vendored and docs:
  - `third_party/tokio-postgres/**`: a pristine copy of `~/.cargo/registry/src/*/tokio-postgres-0.7.18`. Its `Cargo.toml` drops `[dev-dependencies]`, `[[test]]`, `[[bench]]`, `[[example]]` and profile sections; the copy omits `Cargo.toml.orig`, `Cargo.lock`, `tests/` and `benches/`. `PATCHES.md` starts with §0 "vendored copy; test, bench and CI files removed" and lists §1 as "Task 12".
  - `docs/decisions/0006-execution-and-results.md` and `0007-editor-and-grid.md` (Status, Context, Rule heading, and this task's section `## m3a-t01b — seams`); the same section appended to Task 1a's ADR 0005.
- Create, tests: `crates/ui/hacivat-text/tests/buffer_minimal.rs`; `crates/ui/hacivat-ui-editor/tests/stubs.rs`; `crates/core/hacivat-values/tests/m3a_types.rs`; `crates/core/hacivat-session/tests/m3a_interfaces.rs`; `crates/core/hacivat-exec/tests/stubs.rs`; `crates/app/hacivat-app/tests/m3a_routing.rs`.
- Modify, workspace: `Cargo.toml` and `Cargo.lock` (Task 1a's lines kept), `xtask/Cargo.toml` (`phf_codegen`, P6), `.github/workflows/ci.yml` (block `# ## m3a-t01b-seams`: test lines 6–8 in `build-test` and `turkey-test`, P10), `xtask/src/check_deps.rs` (four rules and their fixture tests; `PRODUCT_CRATES` gains five crates, P22), `xtask/src/docker.rs` (`--test <name>` passed through before `--`, with its unit test; `hacivat-exec` in `DEFAULT_TEST_CRATES`; P9, P6), `xtask/src/l10n/code_scan.rs` (`ID_MODULES` gains `exec` and `cursor`, P24).
- Modify, existing crates (minimal arms, fields and seams only; Q3):
  - `crates/ui/hacivat-ui-core/src/gpui_surface.rs` (re-exports below); `src/platform/memory.rs` (`physical_memory_bytes` signature returning `None`; Task 33 implements); `Cargo.toml` (the `windows-sys` feature and the macOS `libc` row, P6).
  - `crates/core/hacivat-driver/src/{driver.rs,fake.rs,lib.rs}` (typed defaults; FakeDriver typed scripting, working, P5's PostgreSQL contract included).
  - `crates/core/hacivat-driver-sqlite/Cargo.toml` (`rusqlite` gains `column_decltype`).
  - `crates/core/hacivat-testkit/src/lib.rs` (`perf_enabled`, `perf_enabled_from`; P9).
  - `crates/core/hacivat-session/src/session/mod.rs` (types below), `session/actor.rs` (the `Request` arms in `handle` `:411`, `is_activity` `:735`, `is_retryable` `:747`, `may_open_tx` `:1144`, all answering or classifying the new variants fail-closed; the `TxState` literals at `:189` and `:639`; the `SessionSnapshot` literal at `:196`), `session/reset.rs:44` (`TxState` literal), `session/readonly.rs` (the `guarded_query` stub), `session/pool.rs` (`budget_use`; `SessionPolicy` and `set_policy`; `effective_read_only` at `:559`, `:776`, `:819`, `:868`, `:1211`), `manager.rs` (`ManagerConfig.production_read_only` in `Default` `:155` and `m2b()` `:169`; `effective_read_only` at `:583`; `set_policy` right after `SessionManager::new` at `:385`), `test_conn.rs:110` (`effective_read_only`), `lib.rs` (exports; `#[cfg(feature = "test-support")] pub mod testing;`), `Cargo.toml` (feature `test-support` gains `hacivat-driver/fake`).
  - `crates/core/hacivat-session/tests/manager.rs:60, 230, 252, 275` (`ManagerConfig` literals gain `production_read_only: false`; `tests/manager_m2b.rs:38` uses `..ManagerConfig::default()` and needs no edit).
  - `crates/core/hacivat-settings/src/schema.rs` (`[editor]`).
  - `crates/app/hacivat-app/src/shell/{hooks.rs,workspace.rs,tabs.rs,mod.rs}`: three hook slots plus `tab_close_guard` and `quit_guard`; `TabKind::Console` routed to `console_view` in `tab_content` (`workspace.rs:1203-1216`), built once per tab like `build_view`; the results tile at `workspace.rs:1066` uses `results_view` when set; `TabSpec.env`/title from `tab_decor` (`workspace.rs:1151`); the 1 px production hairline drawn at the pane's top edge when `TabDecor.hairline`; `SavedKind::Console { n, #[serde(default)] profile: Option<String> }` (`workspace.rs:87-90`, `:255`, `:777`) and `TabKind::Console { n, profile: Option<ProfileId> }`, with every pattern updated: `shell/tabs.rs:17`, `:34`, `shell/workspace.rs:179`, `:255`, `:325`, `:348`, `:380`, `:665`, `:777` (P12); `close_tab` (`:467`) consults `tab_close_guard`, and `pub fn force_close_tab(&mut self, tab: TabId, cx: &mut Context<Self>)` closes without asking (the guard calls it after its prompt).
  - `crates/app/hacivat-app/src/commands/registry.rs:160-163` (`app.quit` consults `quit_guard`), `src/menus.rs` (an Edit group shown only when `editor.*` actions are registered), `src/lib.rs` (`console::install`, `results::install`, `exec_ui::install` lines; `#[cfg(feature = "bench")] pub mod bench;`; `results::install` registers `results_view` through `results::view::build`, a stub that keeps today's tile until Task 30), `src/connections/boot.rs:171` (`ManagerConfig { production_read_only: true, ..ManagerConfig::m2b() }`).
  - `crates/app/hacivat-app/Cargo.toml` (deps; `bench` feature; `[[example]] b3` and `b1_real`, `required-features = ["bench"]`).
  - `crates/app/hacivat-app/tests/support/mod.rs` (block `// ## m3a-t01b-seams`: `stop_core_runtime` moved from `tests/shots.rs:1217` with two entry points over one body, `stop_core_runtime(&mut HeadlessAppContext)` and `stop_core_runtime_test(&mut TestAppContext)` (P20); `register_test_console(vcx, profile, session) -> ConsoleId`, which builds a real `EditorView` with `SqlLanguageService` over a FakeDriver session from `hacivat_session::testing::fake_session`, so Tasks 33 and 34 need no Task 32 code (P12); `editor_text(vcx, console)`; `find_label_runs` (P15)), and `tests/shots.rs` (`#[path = "support/mod.rs"] mod support;`, since it has no `mod support;` today, and the moved helper).
  - `crates/ui/hacivat-ui/tests/support/mod.rs` (block `// ## m3a-t01b-seams`: `find_label_runs`, lifted from the closure in `tests/a11y.rs`, which then calls it; P15).
  - **The M2b app tests the boot change flips (P21).** `boot.rs:171`'s `production_read_only: true` makes production-profile sessions read-only in every app test that boots through `start()`: `crates/app/hacivat-app/tests/{connection_dialog,connections_errors,connections_explorer,connections_live,connections_live_m2b,connections_model,connections_prompts,cross_surface_m2a}.rs`. Each assertion that changes (Auto mode, the read-only access label, the guard steps in FakeDriver's `executed()`) gets the read-only expectation and is listed in the report; none is deleted or weakened. The shots use static fixtures (`capsule::production_ro_fixture`, `DemoCapsule`) and must not move (`shot-diff`).
  - Keymaps: block `## m3a-t01b-seams` in `crates/ui/hacivat-ui/keymaps/default-{macos,windows,linux}.toml` and `presets/datagrip.toml` (rows below).
  - Locales: block `## m3a-t01b-seams` in `locales/{en,tr}/hacivat-ui.ftl` with the Edit menu title (`hv-menu-edit`) and the stub notice `hv-console-run-unavailable` (`# Review: PO` on TR).

**Interfaces (Produces; every later task consumes these names).** Derives follow M2b Task 1: data types derive `Debug, Clone, PartialEq`, plus `Eq, Hash` where the fields allow, `Copy` for small fieldless enums, `Default` where a default is named. `BoxFuture` is `futures::future::BoxFuture`. Positions are UTF-8 byte offsets everywhere: `TextSize` (u32) in `crates/sql`, `usize` in `crates/ui`; the SQL adapter converts. Stubs fail closed (Global Constraints). **File placement (P22):** each type lives in the module file of the task that fills it, never in `lib.rs` (`LexDialect` in `sql-dialect/src/lex_profiles/mod.rs`, `Lexed` in `sql-lexer/src/token.rs`, `SplitProfile` in `sql-dialect/src/split_profile/mod.rs`, `ConsoleResults` in `results/tabs.rs`, …); `lib.rs` and `mod.rs` files hold module lines and re-exports only.

```rust
// ===== crates/sql/sql-text (T3 bodies) =====
pub use text_size::{TextRange, TextSize};
pub struct TextEdit { pub delete: TextRange, pub insert: String }              // char-boundary aligned (R-M3-42)
pub struct LineCol { pub line: u32, pub col: u32 }                             // 0-based line, byte column
pub struct LineIndex { /* line starts */ }
impl LineIndex {
    pub fn new(text: &str) -> Self;
    pub fn line_col(&self, at: TextSize) -> LineCol;
    pub fn offset(&self, lc: LineCol) -> Option<TextSize>;
    pub fn char_to_byte(&self, text: &str, char_idx: usize) -> Option<TextSize>;   // PG error cursors (1-based chars, caller subtracts 1)
    pub fn len_lines(&self) -> u32;
}
pub trait TextChunks {                                                         // R-M3-9: the lexer's chunked input
    fn len(&self) -> usize;
    fn slice(&self, r: std::ops::Range<usize>) -> std::borrow::Cow<'_, str>;   // borrowed inside one chunk
    fn chunks(&self) -> Box<dyn Iterator<Item = &str> + '_>;
}
impl TextChunks for str { /* working in T1b */ }

// ===== crates/sql/sql-dialect (T3: keywords, version, modes; T4: lex_profiles; T8: split_profile) =====
#[non_exhaustive] pub enum DialectId { Postgres, MySql, MariaDb, Sqlite, Generic, PgWire(PgWireFlavor), SqlServer, ClickHouse, Oracle }
pub enum PgWireFlavor { Redshift, Cockroach, Yugabyte, Timescale, Other }
pub struct ServerVersion { pub major: u16, pub minor: u16, pub patch: u16 }      // Ord
pub struct LexModes { pub standard_conforming_strings: bool, pub ansi_quotes: bool, pub no_backslash_escapes: bool,
                      pub pipes_as_concat: bool, pub high_not_precedence: bool, pub mariadb_oracle_mode: bool }
impl LexModes { pub fn default_for(d: DialectId) -> Self; pub fn from_mysql_sql_mode(sql_mode: &str) -> Self; }
pub struct DialectCtx { pub id: DialectId, pub version: Option<ServerVersion>, pub modes: LexModes }
#[non_exhaustive] pub enum ClientFlavor { Psql, MySqlCli, Sqlite3, None }
pub struct LexDialect { /* opaque table-driven profile (T4) */ }
pub struct UnsupportedDialect(pub DialectId);
pub fn lex_profile(ctx: &DialectCtx) -> Result<&'static LexDialect, UnsupportedDialect>;   // SqlServer/ClickHouse/Oracle → Err (R19)
pub fn profile_version(ctx: &DialectCtx) -> &'static str;                                 // e.g. "lex:pg-18@2026.11.1"
pub struct KeywordId(pub u16);
pub enum KwCategory { Unreserved, ColName, TypeFuncName, Reserved }
pub struct KwEntry { pub id: KeywordId, pub category: KwCategory }
pub fn keyword_lookup(d: DialectId, word: &str) -> Option<KwEntry>;            // ASCII-only, ≤ 32 bytes, else None
pub fn keyword_text(id: KeywordId) -> &'static str;                             // canonical upper case
pub mod kw { pub const SELECT: KeywordId; pub const VALUES: KeywordId; pub const TABLE: KeywordId; pub const WITH: KeywordId;
             pub const SHOW: KeywordId; pub const INSERT: KeywordId; pub const UPDATE: KeywordId; pub const DELETE: KeywordId;
             pub const MERGE: KeywordId; pub const INTO: KeywordId; pub const FOR: KeywordId; pub const LOCK: KeywordId;
             pub const SET: KeywordId; pub const BEGIN: KeywordId; pub const END: KeywordId; /* … placeholder values in T1b (Rust needs them); T3 generates the full set (P22) */ }
pub struct SplitProfile { /* block openers/closers, statement starts (T8) */ }
pub fn split_profile(ctx: &DialectCtx) -> &'static SplitProfile;

// ===== crates/sql/sql-lexer (T4) =====
#[repr(u8)] #[non_exhaustive]
pub enum TokenKind { Whitespace, Newline, LineComment, BlockComment, HintComment, ExecutableComment, Word, QuotedIdent,
    UnicodeQuotedIdent, String, DollarString, Number, BitString, HexString, InlineData, PositionalParam, QuestionParam,
    NamedParam, UserVar, SystemVar, ClientVar, Op, LParen, RParen, LBracket, RBracket, LBrace, RBrace, Comma, Semicolon,
    Dot, ClientCommand, Unknown }
bitflags::bitflags! { pub struct TokenFlags: u8 { const UNTERMINATED = 1; const JUNK = 2; const HAS_SEPARATORS = 4; const BOM = 8; const ERROR = 16; } }
pub enum LexState { Initial, InBlockComment { depth: u32 }, InString { rule: StringRule }, InDollar { tag: SmallVec<[u8; 16]> }, InCopyData }
pub enum StringRule { Standard, Escape, Backslash, Unicode }
pub struct LexError { pub range: TextRange, pub code: LexErrorCode }
#[non_exhaustive] pub enum LexErrorCode { UnterminatedString, UnterminatedComment, UnterminatedDollar, UnterminatedQuotedIdent, InvalidEscape, Nul }
pub struct Lexed { /* struct of arrays: kinds, starts (sorted), flags, keywords */ }
impl Lexed {
    pub fn len(&self) -> usize;
    pub fn kind(&self, i: usize) -> TokenKind;
    pub fn range(&self, i: usize) -> TextRange;
    pub fn flags(&self, i: usize) -> TokenFlags;
    pub fn keyword(&self, i: usize) -> Option<KwEntry>;
    pub fn is_trivia(&self, i: usize) -> bool;                                // whitespace, newline, comments (not executable comments)
    pub fn errors(&self) -> &[LexError];
    pub fn end_state(&self) -> &LexState;
    pub fn token_at(&self, at: TextSize) -> Option<usize>;                    // O(log n)
    pub fn tokens_in(&self, r: TextRange) -> std::ops::Range<usize>;          // O(log n)
    pub fn max_paren_depth(&self, tokens: std::ops::Range<usize>) -> u16;     // R-M3-35
}
pub fn lex(text: &str, profile: &LexDialect, modes: LexModes, start: &LexState) -> Lexed;              // total, lossless
pub fn lex_chunks(text: &dyn TextChunks, profile: &LexDialect, modes: LexModes, start: &LexState) -> Lexed;
pub struct RelexDelta { pub changed: std::ops::Range<usize>, pub full: bool }
pub fn relex(lexed: &mut Lexed, new_text: &dyn TextChunks, edit: &TextEdit, profile: &LexDialect, modes: LexModes) -> RelexDelta;
                                                                              // M3a body: full relex, `full: true` (R-M3-9, R-M3-22)
#[non_exhaustive] pub enum TokenClass { Keyword, Identifier, QuotedIdentifier, String, Number, Comment, Operator,
    Punctuation, Parameter, DollarTag, ExecutableComment, OptimizerHint, ClientCommand, Error }
pub fn token_class(kind: TokenKind, flags: TokenFlags, kw: Option<KwEntry>) -> TokenClass;
pub enum GateStatus { Active, Inactive, Unknown }
pub fn exec_comment_gate(text: &str, lexed: &Lexed, i: usize, ctx: &DialectCtx) -> GateStatus;
// T1b stub: lex() returns ONE Unknown token over the whole input (lossless, fail-closed).

// ===== crates/sql/sql-split (T8: split, document, head; T13: exec_range) =====
pub enum SplitMode { Editor, Execution }
pub enum Confidence { Exact, Heuristic }
pub struct StmtKey(pub u128);                                                 // XXH3-128 of (dialect, modes, delimiter, text) (R-M3-42)
pub struct StmtHead { pub first_keyword: Option<KeywordId>, pub first_token: Option<TextRange> }
pub struct Stmt { pub strict: TextRange, pub smart: TextRange, pub terminator: Option<TextRange>,
                  pub leading_comments: Option<TextRange>, pub confidence: Confidence, pub head: StmtHead, pub key: StmtKey }
pub enum Item { Statement(Stmt), Empty { range: TextRange } }
pub enum AmbiguityReason { SemicolonInExecutableComment, UnterminatedConstruct, UnknownBlock, DelimiterChange }
pub struct AmbiguousBoundary { pub at: TextRange, pub reason: AmbiguityReason }
pub struct ClientSegment { pub range: TextRange, pub supported: bool }        // psql \cmd, DELIMITER, sqlite3 .cmd
pub struct Split { pub items: Vec<Item>, pub ambiguous: Vec<AmbiguousBoundary>, pub client: Vec<ClientSegment> }
pub fn split(text: &str, lexed: &Lexed, ctx: &DialectCtx, flavor: ClientFlavor, mode: SplitMode) -> Split;
pub struct DocDelta { pub changed: Vec<StmtKey>, pub moved: Vec<StmtKey>, pub removed: Vec<StmtKey>, pub dirty: TextRange }
pub struct SqlDocument { /* String mirror, Lexed, editor-mode Split, revision */ }
impl SqlDocument {
    pub fn new(text: String, ctx: DialectCtx, flavor: ClientFlavor) -> Self;
    pub fn from_chunks(text: &dyn TextChunks, ctx: DialectCtx, flavor: ClientFlavor) -> Self;
    pub fn apply(&mut self, edits: &[TextEdit]) -> DocDelta;                  // M3a body: full rebuild (R-M3-22)
    pub fn set_context(&mut self, ctx: DialectCtx, flavor: ClientFlavor) -> DocDelta;
    pub fn text(&self) -> &str; pub fn lexed(&self) -> &Lexed; pub fn revision(&self) -> u64;
    pub fn statements(&self) -> &[Item];                                      // editor mode
    pub fn statement_at(&self, at: TextSize) -> Option<usize>;
    pub fn snapshot(&self) -> DocSnapshot;                                    // O(1)
}
#[derive(Clone)] pub struct DocSnapshot { /* Arc<text>, Arc<Lexed>, Arc<Split>, OnceLock<Arc<Split>> for execution mode */ } // Send + Sync
impl DocSnapshot {
    pub fn text(&self) -> &str; pub fn lexed(&self) -> &Lexed; pub fn ctx(&self) -> &DialectCtx; pub fn flavor(&self) -> ClientFlavor;
    pub fn revision(&self) -> u64;
    pub fn editor_split(&self) -> &Split;
    pub fn execution_split(&self) -> &Split;                                  // computed once, from the same Lexed
}
pub mod head { pub fn cursor_eligible(doc: &DocSnapshot, range: TextRange) -> bool; }   // R-M3-23 (T8); T1b stub: false
pub mod exec_range {
    pub enum InsidePolicy { Ask, SmallestSubqueryOrStatement, SmallestStatement, LargestStatement, LargestStatementOrBatch, WholeScript, EverythingFromCaret }
    pub enum OutsidePolicy { Nothing, WholeScript, EverythingBelow }
    pub enum SelectionPolicy { AsOne, AsSeparate, SmartExpand }
    pub struct ExecPolicy { pub inside: InsidePolicy, pub outside: OutsidePolicy, pub selection: SelectionPolicy }
    impl Default for ExecPolicy { /* D-011: Ask (one-candidate shortcut), Nothing, AsSeparate */ }
    impl ExecPolicy { pub fn largest() -> Self; /* LargestStatement, Nothing, AsSeparate */ pub fn whole_script() -> Self; /* WholeScript, WholeScript, AsSeparate */ }
    pub enum CaretOrSelection { Caret(TextSize), Selection(TextRange) }
    pub enum UnitKind { Statement, Subquery, Batch, Verbatim, CopyIn { data: TextRange } }
    pub struct UnitSpan { pub range: TextRange /* strict: no terminator, no leading comments */, pub kind: UnitKind,
                          pub key: Option<StmtKey>, pub title_comment: Option<TextRange> }
    pub enum CandidateKind { Subquery, Statement, LargestOrBatch, FromCaret, WholeScript }
    pub struct Candidate { pub kind: CandidateKind, pub units: Vec<UnitSpan>, pub label: String /* whitespace collapsed, controls → space */ }
    pub enum RangeResolution { Units(Vec<UnitSpan>), Ask { candidates: Vec<Candidate>, preselect: usize }, NothingToRun,
                               Ambiguous(Vec<AmbiguousBoundary>), UnsupportedClientCommand { at: TextRange } }
    pub trait SubqueryOracle: Send + Sync { fn innermost_runnable_subquery(&self, doc: &DocSnapshot, stmt: &Stmt, caret: TextSize) -> Option<TextRange>; }
    pub fn resolve(doc: &DocSnapshot, at: &[CaretOrSelection], policy: &ExecPolicy, mode: SplitMode,
                   subq: Option<&dyn SubqueryOracle>, remembered: Option<CandidateKind>) -> RangeResolution;
    // T1b stub: NothingToRun. Execution mode never returns a unit crossing a Heuristic boundary (T13).
}

// ===== crates/sql/sql-oracle-pg (Task 1a: version + statement_count WORKING = the build proof; T1b: module lines and stubs; T9: the rest) =====
pub struct PgError { pub message: String /* English, untranslated */, pub cursor: Option<TextSize> }
pub fn version() -> &'static str;                                            // "pg_query 18.1.0 (PostgreSQL 18.6)"
pub fn statement_count(sql: &str) -> Result<usize, PgError>;
pub fn split_with_parser(sql: &str) -> Result<Vec<TextRange>, PgError>;      // T9 (`$name` normalised first)
pub fn on_parser_stack<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> Option<R>;   // 16 MiB thread (R-M3-35); None if it panicked; T9
pub const MAX_ORACLE_NESTING: u16 = 256;

// ===== crates/sql/sql-testkit (dev-only, publish = false; T13) =====
pub mod golden { pub struct Case; pub fn load(path: &std::path::Path) -> Vec<Case>; pub fn run(case: &Case) -> Result<(), String>; }
```

```rust
// ===== crates/ui/hacivat-text (headless; T5, T10, T11). WORKING in T1b: TextBuffer, EditorModel::{new,dispatch,snapshot} =====
#[derive(Clone)] pub struct TextBuffer { /* ropey::Rope, Arc<BufferMeta> */ }        // O(1) clone, Send + Sync
pub struct BufferMeta { pub bom: bool, pub line_endings: LineEndingStats }
pub enum LineEnding { Lf, CrLf, Cr }
pub struct LineEndingStats { pub lf: u32, pub crlf: u32, pub cr: u32 }
pub enum Bias { Before, After }
impl TextBuffer {
    pub fn from_str(text: &str) -> Self;                                    // strips one leading U+FEFF into meta.bom
    pub fn meta(&self) -> &BufferMeta;
    pub fn len_bytes(&self) -> usize; pub fn len_utf16(&self) -> usize; pub fn len_lines(&self) -> usize;
    pub fn slice(&self, r: Range<usize>) -> Cow<'_, str>;
    pub fn chunks_in(&self, r: Range<usize>) -> impl Iterator<Item = &str> + '_;
    pub fn byte_to_utf16(&self, b: usize) -> usize; pub fn utf16_to_byte(&self, u: usize) -> usize;
    pub fn byte_to_line(&self, b: usize) -> usize; pub fn line_to_byte(&self, l: usize) -> usize;
    pub fn line_range(&self, l: usize, with_break: bool) -> Range<usize>;
    pub fn snap(&self, b: usize, bias: Bias) -> usize;                      // char boundary, never inside CRLF
    pub fn dominant_line_ending(&self) -> LineEnding;
    pub fn content_hash(&self) -> u128;                                     // XXH3-128 (twox-hash)
    pub fn to_string(&self) -> String;                                      // the worker, Run and autosave only
}
pub enum Section { Retain(usize), Replace { del: usize, ins: Arc<str> } }
pub struct ChangeSet { /* sections, len_old, len_new */ }
pub struct ChangeDesc { /* lengths only */ }
pub enum MapMode { Simple, TrackDel, TrackBefore, TrackAfter }
pub enum RangeKind { Content /* (After, Before) */, Container /* (Before, After) */ }
pub struct OverlapError;
impl ChangeSet {
    pub fn from_edits(len_old: usize, edits: impl IntoIterator<Item = (Range<usize>, Arc<str>)>) -> Result<Self, OverlapError>;
    pub fn invert(&self, old: &TextBuffer) -> ChangeSet; pub fn compose(&self, next: &ChangeSet) -> ChangeSet; pub fn desc(&self) -> ChangeDesc;
}
impl ChangeDesc { pub fn map_pos(&self, pos: usize, bias: Bias, mode: MapMode) -> Option<usize>;
                  pub fn map_range(&self, r: Range<usize>, kind: RangeKind) -> Option<Range<usize>>; }
pub struct AnchorSet<T> { /* sorted (offset, bias, mode, T) */ }
impl<T> AnchorSet<T> { pub fn insert(&mut self, at: usize, bias: Bias, mode: MapMode, v: T); pub fn map(&mut self, c: &ChangeDesc); pub fn iter(&self) -> impl Iterator<Item = (usize, &T)>; }
pub enum Rebased { Mapped(Range<usize>), Deleted, TooOld }
pub struct ChangeLog { /* ring of the last 256 (version, ChangeDesc) */ }
impl ChangeLog { pub fn rebase(&self, from_version: u64, r: Range<usize>, kind: RangeKind) -> Rebased; pub fn version(&self) -> u64; }
pub struct SelectionRange { pub anchor: usize, pub head: usize, pub goal_x: Option<f32>, pub assoc: Bias }
pub struct SelectionSet { /* ranges: SmallVec<[SelectionRange; 1]>, main */ }
impl SelectionSet { pub fn cursor(at: usize) -> Self; pub fn single(r: SelectionRange) -> Self;
    pub fn normalized(r: impl IntoIterator<Item = SelectionRange>, main: usize) -> Self;
    pub fn map(&self, c: &ChangeDesc) -> Self; pub fn main(&self) -> &SelectionRange; pub fn ranges(&self) -> &[SelectionRange]; }
pub enum Unit { Grapheme, Word, SubWord, Line, Document }
pub enum Dir { Back, Forward, Up, Down }
pub trait TokenOracle { fn words_on_line(&self, line: Range<usize>) -> Option<SmallVec<[Range<usize>; 16]>>; }
pub fn move_offset(buf: &TextBuffer, at: usize, dir: Dir, unit: Unit, tok: &dyn TokenOracle) -> usize;
pub enum Origin { User, Ime, Program, Service }
pub enum UserEvent { InputType, InputTypeCompose, InputPaste, InputDrop, DeleteBackward, DeleteForward, DeleteSelection,
                     DeleteCut, Move, Select, SelectPointer, Undo, Redo, Command(&'static str) }
pub struct Annotations { pub user_event: UserEvent, pub add_to_history: bool, pub time: Instant, pub origin: Origin }
pub struct Transaction { pub changes: ChangeSet, pub selection: Option<SelectionSet>, pub ann: Annotations }
pub struct Veto(pub &'static str);
pub trait Field: 'static { fn on_commit(&mut self, tr: &Transaction, old: &TextBuffer, new: &TextBuffer, version: u64); }
pub trait TransactionFilter: 'static { fn filter(&self, m: &EditorModel, tr: Transaction) -> Result<Transaction, Veto>; }
#[derive(Clone)] pub struct Snapshot { pub buffer: TextBuffer, pub selection: SelectionSet, pub version: u64 }   // O(1), Send + Sync
pub struct Committed { pub version: u64, pub desc: ChangeDesc, pub buffer_changed: bool }
pub struct ModelConfig { pub history_groups: u32 /* 1,000 */, pub history_bytes: u64 /* 64 MiB */, pub group_window: Duration /* 500 ms */ }
pub struct FieldId(pub u32); pub struct BracketId(pub u32);
pub struct EditorModel { .. }
impl EditorModel {
    pub fn new(text: &str, cfg: ModelConfig) -> Self;
    pub fn dispatch(&mut self, tr: Transaction) -> Result<Committed, Veto>;   // T1b: apply + selection; T5: filters, fields, history, log
    pub fn snapshot(&self) -> Snapshot; pub fn version(&self) -> u64; pub fn changelog(&self) -> &ChangeLog;
    pub fn undo(&mut self) -> Option<Committed>; pub fn redo(&mut self) -> Option<Committed>; pub fn seal_history(&mut self);
    pub fn begin_bracket(&mut self, kind: UserEvent) -> BracketId; pub fn end_bracket(&mut self, b: BracketId);
    pub fn add_field(&mut self, f: Box<dyn Field>) -> FieldId; pub fn add_filter(&mut self, f: Box<dyn TransactionFilter>);
}
pub struct ImeCore { /* composition, bracket */ }
impl ImeCore {
    pub fn mark(&mut self, m: &mut EditorModel, r16: Option<Range<usize>>, text: &str, sel16: Option<Range<usize>>) -> Committed;
    pub fn commit(&mut self, m: &mut EditorModel, r16: Option<Range<usize>>, text: &str) -> Committed;
    pub fn unmark(&mut self, m: &mut EditorModel);
    pub fn blur(&mut self, m: &mut EditorModel, window_active: bool) -> bool;
    pub fn text_for_range(&self, b: &TextBuffer, r16: Range<usize>, adjusted: &mut Option<Range<usize>>) -> String;
    pub fn marked_utf16(&self, b: &TextBuffer) -> Option<Range<usize>>; pub fn is_composing(&self) -> bool;
}
pub mod display {
    pub struct StyleId(pub u16); pub struct LineClass(pub u16); pub struct PlaceholderId(pub u16); pub struct WidgetId(pub u32);
    pub enum Decoration { Mark { style: StyleId }, Line { class: LineClass }, Replace { placeholder: PlaceholderId }, Widget { id: WidgetId, side: i8 } }
    pub struct RangeSet<T> { /* sorted, O(log n + k) queries */ }
    impl<T: Clone> RangeSet<T> { pub fn from_sorted(v: Vec<(Range<usize>, T)>) -> Self; pub fn query(&self, r: Range<usize>) -> impl Iterator<Item = (Range<usize>, &T)>; }
    pub enum Fragment { Text { bytes: Range<usize> }, Tab { width: u8 }, Control { byte: usize, glyph: char }, Placeholder { id: PlaceholderId } }
    pub struct VisualLine { pub first_line: u32, pub last_line: u32, pub fragments: SmallVec<[Fragment; 8]> }
    pub fn compose_line(buf: &TextBuffer, line: u32, tab_size: u8, decos: &[&RangeSet<Decoration>]) -> VisualLine;
    pub enum HeightMap { Uniform { lines: u32, line_height: f32 } /* M3b adds Chunked */ }
    pub struct ScrollAnchor { pub line: u32, pub wrap_index: u16, pub offset_in_row: f32 }
}

// ===== crates/ui/hacivat-ui-editor (T19 view/element; T25 commands; T27 ime/clipboard; T28 gutter/marks/preview; T29 a11y/tiers) =====
pub const EDITOR_CONTEXT: &str = "Editor";
pub struct DocId(pub u64);
pub enum CaretBlink { System, Off }
pub struct TierLimits { pub full_max_bytes: u64 /* 4 MB */, pub reduced_max_bytes: u64 /* 20 MB */ }
pub struct EditorConfig { pub tab_size: u8 /* 4 */, pub insert_spaces: bool, pub copy_line_without_selection: bool,
    pub caret_blink: CaretBlink, pub reduced_motion: bool, pub tiers: TierLimits, pub a11y_window_lines: u32 /* 100, D-073 */,
    pub a11y_optimized: bool, pub read_only: bool }
pub enum Tier { Full, Reduced, Preview }
pub struct DocContext { pub connected: bool, pub tier: Tier }
pub enum Confidence { Exact, Heuristic }
pub struct StatementSpan { pub smart: Range<usize>, pub strict: Range<usize>, pub header: usize, pub confidence: Confidence }
pub enum PreviewState { Units, AskPreselected, Nothing, Uncertain, NoConnection }
pub struct ExecPreview { pub ranges: SmallVec<[Range<usize>; 2]>, pub state: PreviewState }
pub enum ExecAction { Execute1, Execute2, Execute3 }
pub enum ExecAt { Carets(SmallVec<[usize; 1]>), Selections(SmallVec<[Range<usize>; 1]>) }   // bytes at snapshot.version
pub struct ExecRequest { pub doc: DocId, pub action: ExecAction, pub at: ExecAt, pub snapshot: Snapshot }
pub type InvalidateFn = Rc<dyn Fn(&mut App)>;
pub struct SourceId(pub u16);
pub trait DecorationSource: 'static {
    fn id(&self) -> SourceId; fn priority(&self) -> i16;
    fn decorations(&self, snap: &Snapshot, log: &ChangeLog, range: Range<usize>) -> Arc<RangeSet<Decoration>>;   // rebased to snap.version
}
pub trait LanguageService: 'static {                                          // Rc<dyn LanguageService>, UI thread
    fn open(&self, doc: DocId, snap: &Snapshot, ctx: DocContext, cx: &mut App);
    fn set_context(&self, doc: DocId, ctx: DocContext, cx: &mut App);
    fn change(&self, doc: DocId, snap: &Snapshot, changes: &ChangeDesc, cx: &mut App);   // never lexes on this thread (R-M3-22)
    fn close(&self, doc: DocId);
    fn on_update(&self, doc: DocId, notify: InvalidateFn);                   // called when a fresh analysis lands
    fn statements(&self, doc: DocId, version: u64, range: Range<usize>) -> Arc<[StatementSpan]>;
    fn exec_preview(&self, doc: DocId, version: u64, at: &ExecAt, action: ExecAction) -> Option<ExecPreview>;   // PREVIEW only; None = no analysis at this version
    fn token_oracle(&self, doc: DocId) -> Rc<dyn TokenOracle>;
    fn decoration_sources(&self, doc: DocId) -> SmallVec<[Rc<dyn DecorationSource>; 2]>;
}
pub struct FailClosedService;                                                 // T1b: Some(preview Uncertain, no ranges); no statements; no decorations
#[repr(u8)] pub enum HighlightClass { Plain, Keyword, Ident, QuotedIdent, String, Number, Comment, Operator, Punct, Param,
    DollarTag, ExecComment, Hint, MetaCommand, ErrorToken, Function }
pub struct MarkKey(pub u64);
pub enum RowsOutcome { Fetched { rows: u64, more: bool }, Affected(u64) }
pub enum ExecMarkState { Queued, Running { started: Instant }, Succeeded { took: Duration, rows: Option<RowsOutcome> },
                         Failed { sqlstate: Option<SharedString> }, Cancelling { since: Instant }, Cancelled, Skipped }
pub struct EditorStatus { pub line: u32, pub column: u32 /* 1-based graphemes */, pub selected_chars: u64, pub tier: Tier, pub composing: bool }
pub enum EditorEvent { Changed { version: u64 }, SelectionChanged { version: u64 }, ExecuteRequested(ExecRequest),
                       CompositionChanged(bool), StatusChanged, TierChanged(Tier), PasteRefused { bytes: u64 } }
pub trait FrameProbe { fn input(&self, at: Instant); fn painted(&self, cx: &mut App); }
pub struct FocusedEditor(pub Option<WeakEntity<EditorView>>);                 // Global
pub struct EditorCommand { pub id: &'static str, pub title_id: &'static str, pub run: fn(&mut EditorView, &mut Window, &mut Context<EditorView>), pub seals_history: bool }
pub fn commands() -> &'static [EditorCommand];                                // T1b: the table with no-op bodies; T25 fills them
pub struct EditorView { .. }   // impl Render, Focusable, EventEmitter<EditorEvent>; EntityInputHandler in ime.rs
impl EditorView {
    pub fn new(doc: DocId, text: &str, cfg: EditorConfig, service: Rc<dyn LanguageService>, a11y_label: TextFn,
               window: &mut Window, cx: &mut Context<Self>) -> Self;
    pub fn snapshot(&self) -> Snapshot;
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>);           // Origin::Program, one undo group
    pub fn set_config(&mut self, cfg: EditorConfig, cx: &mut Context<Self>);
    pub fn set_doc_context(&mut self, ctx: DocContext, cx: &mut Context<Self>);
    pub fn run_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool;
    pub fn select_range(&mut self, r: Range<usize>, cx: &mut Context<Self>);  // "go to error"
    pub fn caret_bounds(&self) -> Option<Bounds<Pixels>>;                     // chooser anchor (window coordinates)
    pub fn set_preview_override(&mut self, ranges: Option<SmallVec<[Range<usize>; 2]>>, cx: &mut Context<Self>);
    pub fn status(&self) -> EditorStatus;
    pub fn set_probe(&mut self, probe: Option<Rc<dyn FrameProbe>>);
    pub fn add_exec_marks(&mut self, version: u64, marks: Vec<(MarkKey, Range<usize>)>, cx: &mut Context<Self>);
    pub fn set_exec_state(&mut self, key: MarkKey, state: ExecMarkState, label: Option<TextFn>, eol: Option<TextFn>, cx: &mut Context<Self>);
    pub fn clear_exec_marks(&mut self, cx: &mut Context<Self>);
}
// Seams with exact signatures and module-owned state (Q9, P11). `EditorView` holds one state struct per module, and its
// public methods delegate in one line (`add_exec_marks` → `marks::add(&mut self.marks, ..)`). T19 owns every call site
// (view.rs, element.rs) and `last_layout`; T27, T28 and T29 own only their modules' bodies. T1b stubs paint nothing.
pub struct VisibleRows { pub bounds: Bounds<Pixels>, pub line_height: Pixels, pub first_line: u32, pub last_line: u32,
                         pub first_y: Pixels /* top of first_line, window coordinates */ }
pub struct GutterCtx { pub bounds: Bounds<Pixels>, pub line_height: Pixels, pub first_line: u32, pub last_line: u32,
                       pub digits: u8, pub current_line: u32, pub scroll: ScrollAnchor }
pub struct ScrollbarCtx { pub track: Bounds<Pixels>, pub total_lines: u32, pub first_line: u32, pub visible_lines: u32,
                          pub current_statement: Option<Range<u32>> /* lines */ }
pub struct FrameLayout { .. }                                                 // T19: the last painted frame
impl FrameLayout { pub fn rows(&self) -> &VisibleRows; pub fn x_for_offset(&self, offset: usize) -> Option<Pixels>;
                   pub fn offset_for_point(&self, p: Point<Pixels>) -> Option<usize>; }   // byte offsets; None off the painted lines
impl EditorView { pub fn last_layout(&self) -> Option<&FrameLayout>; }      // T27's bounds_for_range / character_index_for_point
pub mod gutter { pub fn paint(g: &GutterCtx, window: &mut Window, cx: &mut App); }                                    // T28
pub mod marks { pub struct MarkStore { .. }                                                                          // T28
    pub fn add(s: &mut MarkStore, version: u64, marks: Vec<(MarkKey, Range<usize>)>, log: &ChangeLog);
    pub fn set_state(s: &mut MarkStore, key: MarkKey, state: ExecMarkState, label: Option<TextFn>, eol: Option<TextFn>);
    pub fn clear(s: &mut MarkStore); pub fn on_commit(s: &mut MarkStore, c: &Committed, log: &ChangeLog);
    pub fn paint(m: &MarkStore, g: &GutterCtx, window: &mut Window, cx: &mut App); }
pub mod preview { pub struct BandState { .. }   // the last ExecPreview, the override, frozen-while-composing, the 50 ms query delay (T28)
    pub fn set_override(s: &mut BandState, ranges: Option<SmallVec<[Range<usize>; 2]>>);
    pub fn paint_band(p: &BandState, rows: &VisibleRows, window: &mut Window, cx: &mut App); }   // (the exported enum PreviewState stays the verdict)
pub mod scrollbar { pub fn paint(s: &ScrollbarCtx, m: &marks::MarkStore, window: &mut Window, cx: &mut App); }      // T28
pub mod a11y { pub struct A11yState { .. }                                                                           // T29
    pub fn declare(a: &mut A11yState, snap: &Snapshot, window: &mut Window, cx: &mut App);
    pub fn announce(text: SharedString, window: &mut Window, cx: &mut App); }
pub mod large_file { pub struct TierState { .. }                                                                     // T29
    pub fn tier_for(bytes: u64, longest_line: u64, limits: &TierLimits) -> Tier;
    pub fn update(s: &mut TierState, bytes: u64, longest_line: u64, limits: &TierLimits) -> Option<Tier>; }   // Some on a change (80 % rule)
pub mod clipboard { pub struct PasteState { .. }                                                                     // T27; T1b: no-ops
    pub fn copy(v: &mut EditorView, window: &mut Window, cx: &mut Context<EditorView>);
    pub fn cut(v: &mut EditorView, window: &mut Window, cx: &mut Context<EditorView>);
    pub fn paste(v: &mut EditorView, window: &mut Window, cx: &mut Context<EditorView>); }
// ime.rs: `impl EntityInputHandler for EditorView` — T1b WORKING minimal: replace_text_in_range inserts plain text at the
//         selection through dispatch; marked text unsupported (returns None)   (T27)

// ===== crates/app/hacivat-sql-editor (T24) =====
pub struct SqlLanguageService { .. }   // impl LanguageService; T1b behaves like FailClosedService
impl SqlLanguageService {
    pub fn new() -> Rc<Self>;
    pub fn set_sql_context(&self, doc: DocId, ctx: DialectCtx, flavor: ClientFlavor);
    pub fn doc_snapshot(&self, doc: DocId, version: u64) -> Option<DocSnapshot>;   // only when versions match (R-M3-22)
}
```

```rust
// ===== crates/core/hacivat-values (T6 model/chunks; T16 valfmt). Types COMPLETE in T1b =====
pub enum LogicalType { Bool, Int { bits: u8, signed: bool }, Decimal { precision: Option<u32>, scale: Option<i32> },
    Float { bits: u8 }, Money, Text { max_chars: Option<u32>, fixed: bool }, Bytes { max: Option<u32> }, Uuid, Date,
    Time { precision: u8, tz: bool }, Timestamp { precision: u8, tz: bool }, Interval, Json { binary: bool }, Xml,
    Array(Box<LogicalType>), Composite(Arc<[(Arc<str>, LogicalType)]>), Enum(Arc<[Arc<str>]>), Range(Box<LogicalType>),
    Network(NetKind), BitString { len: Option<u32> }, Geometry { srid: Option<i32> }, Document, Opaque { type_name: Arc<str> } }
pub enum NetKind { Inet, Cidr, MacAddr, MacAddr8 }
pub enum SqliteAffinity { Integer, Text, Blob, Real, Numeric }
pub enum NativeType { Pg { oid: u32, typmod: i32, name: Arc<str> }, MySql { column_type: u8, flags: u16, charset: u16, length: u32, decimals: u8 },
                      Sqlite { decl: Option<Arc<str>>, affinity: SqliteAffinity }, Unknown }
pub enum OriginHint { Pg { table_oid: u32, attnum: i16 }, MySql { schema: Arc<str>, table: Arc<str>, column: Arc<str> },
                      Sqlite { database: Arc<str>, table: Arc<str>, column: Arc<str> } }       // captured now, used in M6
pub struct ColumnMeta { pub name: Arc<str>, pub logical: LogicalType, pub native: NativeType, pub origin: Option<OriginHint> }
pub struct ResultSchema { pub columns: Arc<[ColumnMeta]> }
pub enum Value { Null, Missing, Default, Bool(bool), Int(i64), UInt(u64), F32(f32), F64(f64), Decimal(DecimalVal), Text(Arc<str>),
    Bytes(Arc<[u8]>), Uuid([u8; 16]), Date(DateVal), Time(TimeVal), TimeTz(TimeVal, i32), Timestamp(TsVal),
    TimestampTz { utc: TsVal, offset: Option<i32> }, Interval { months: i32, days: i32, micros: i64 }, Json(Arc<str>), Xml(Arc<str>),
    Special(SpecialVal), Opaque(Arc<OpaqueVal>), Lossy(Arc<[u8]>), Truncated(Box<TruncatedVal>),
    Error(Arc<ErrorVal>) }          // size_of::<Value>() <= 32 (test): wide payloads behind Arc; largest inline payload 24 B (P8; ADR 0006)
pub struct OpaqueVal { pub type_name: Arc<str>, pub text: Arc<str> }
pub struct ErrorVal { pub code: Arc<str>, pub message: Arc<str> }
pub enum ValueRef<'a> { /* borrowed mirror of Value: Text(&'a str), Bytes(&'a [u8]), Lossy(&'a [u8]), … */ }
pub struct DecimalVal(/* Small { mant: [u64; 2], scale: i16 } | Big(Arc<str>) */);
pub enum DecimalError { Empty, Invalid, TooLong }
impl DecimalVal { pub fn parse(text: &str) -> Result<Self, DecimalError>; pub fn from_base10000(neg: bool, weight: i16, dscale: u16, digits: &[u16]) -> Self;
                  pub fn scale(&self) -> i16; pub fn write_canonical(&self, out: &mut String); }  // exact; never through f64
pub struct DateVal(pub i32);                                                    // days since 1970-01-01, proleptic Gregorian
pub struct TimeVal { pub nanos_of_day: u64 }
pub struct TsVal { pub secs: i64, pub nanos: u32 }
pub enum SpecialVal { PosInfinity, NegInfinity, NaN, ZeroDate, PartialZeroDate(Arc<str>) }
pub enum LenUnit { Bytes, Chars }
pub struct TruncatedVal { pub prefix: Value, pub total_len: Option<u64>, pub unit: LenUnit }
bitflags::bitflags! { pub struct CellFlags: u8 { const TRUNCATED = 1; const LOSSY = 2; const ERROR = 4; const SPECIAL = 8; } }
pub enum TempUnit { Days, Micros }
pub enum VarKind { Text, Bytes, Json }
pub enum BufKind { Bool, I64, U64, F64 { f32: bool }, Dec64 { scale: i16 }, Dec128, Temporal { unit: TempUnit }, Temporal2, Var(VarKind), Fixed16, Values }
pub struct Chunk { .. }
impl Chunk { pub fn rows(&self) -> u32; pub fn bytes(&self) -> u64; pub fn cell(&self, row: u32, col: u32) -> ValueRef<'_>;
             pub fn flags(&self, row: u32, col: u32) -> CellFlags;
             pub fn from_values(schema: &ResultSchema, rows: Vec<Vec<Value>>) -> Chunk; }   // WORKING in T1b (Values buffers only)
pub struct ChunkBuilder { .. }
impl ChunkBuilder { pub fn new(plan: &[BufKind], capacity_rows: u32) -> Self; pub fn push_null(&mut self, col: u32);
    pub fn push_bool(&mut self, col: u32, v: bool); pub fn push_i64(&mut self, col: u32, v: i64); pub fn push_u64(&mut self, col: u32, v: u64);
    pub fn push_f64(&mut self, col: u32, v: f64); pub fn push_dec64(&mut self, col: u32, mant: i64); pub fn push_decimal(&mut self, col: u32, v: &DecimalVal);
    pub fn push_temporal(&mut self, col: u32, a: i64); pub fn push_temporal2(&mut self, col: u32, a: i64, b: i32);
    pub fn push_bytes(&mut self, col: u32, v: &[u8], cap: u32);   // Var; truncates at cap, UTF-8-safe for text; invalid UTF-8 text → Lossy
    pub fn push_fixed16(&mut self, col: u32, v: [u8; 16]); pub fn push_value(&mut self, col: u32, v: Value);
    pub fn end_row(&mut self); pub fn rows(&self) -> u32; pub fn bytes(&self) -> u64; pub fn seal(self) -> Chunk; }
pub enum BindValue { Null, Typed { hint: LogicalType, text: Arc<str> } }       // M3b uses it; in the driver signature now
pub mod valfmt {                                                               // T16
    pub enum DisplayMode { Raw, Default /* L3 */ }
    pub enum BinaryDisplay { Detect, Hex, Uuid, Text }
    pub struct DisplaySettings { pub mode: DisplayMode, pub format_locale: icu_locale::Locale, pub tz: jiff::tz::TimeZone,
                                 pub max_display_chars: u32 /* 1,024 */, pub version: u32 }
    impl DisplaySettings { pub fn new(region_bcp47: &str, tz_iana: Option<&str>) -> Self; }   // the app builds it without ICU or jiff (P6);
                                                                              // T1b: minimal body (bad tag → und, unknown zone → UTC); T16 completes it
    pub enum Purpose { Cell, Tooltip, A11y }
    pub struct ColumnFacts<'a> { pub logical: &'a LogicalType, pub native_name: &'a str, pub id_like: bool, pub binary: BinaryDisplay, pub engine: DbmsKind }
    pub enum DisplayKind { Text, Number, Null, Empty, Bool, Temporal, Binary, Json, Error }
    pub enum ChipKind { Truncated { total: u64, unit: LenUnit } }             // data; the app renders it through Fluent
    pub struct Display { pub text: String, pub kind: DisplayKind, pub flags: u8, pub align_end: bool, pub chip: Option<ChipKind> }
    pub fn format_cell(v: ValueRef<'_>, col: &ColumnFacts<'_>, s: &DisplaySettings, p: Purpose) -> Display;
    pub enum RawText<'a> { Null, Text(Cow<'a, str>) }
    pub fn raw_text<'a>(v: ValueRef<'a>, col: &ColumnFacts<'_>) -> RawText<'a>;   // I18N-11, locale-free
    pub fn id_like(name: &str, logical: &LogicalType, native_name: &str) -> bool;
}

// ===== crates/core/hacivat-results (T7) =====
pub struct ResultId(pub u64);
pub enum CloseReason { SessionNeeded, IdleTimeout, TabClosed, TxEnded, UserClosed, MemoryLimit, ConnectionLost, ServerTerminated, Cancelled, EnforcementGap }
pub enum TruncReason { ResultCap, FollowingResultSets }
pub struct CursorInfo { pub opened_at: Instant, pub last_fetch: Instant, pub holds_tx: bool, pub idle_bound: Duration }
pub enum Completeness { Streaming, Suspended(CursorInfo), Complete, Closed { reason: CloseReason }, Cancelled, Truncated(TruncReason), Released }
pub enum PauseReason { ResultCap, GlobalBudget }
pub enum Admission { Ok, Pause(PauseReason) }
pub struct FetchPolicy { pub first_rows: u32 /* 500 */, pub chunk_rows: u32 /* 4,096 */, pub read_ahead_rows: u32 /* 10,000 */,
    pub read_ahead_bytes: u64 /* 16 MiB */, pub prefetch_margin_rows: u32 /* 1,000 */, pub idle_timeout: Duration /* 10 min */,
    pub release_drain: Duration /* 250 ms */, pub cell_cap: u32 /* 1 MiB */, pub chunk_bytes: u64 /* 4 MiB */, pub overdue: Duration /* 1 s */,
    pub idle_cap: Option<Duration> /* caps every engine bound; tests inject 2 s */ }
pub struct MemoryBudget { .. }
impl MemoryBudget { pub fn new(global: u64, per_result: u64) -> Arc<Self>; pub fn from_ram(ram: Option<u64>) -> Arc<Self>; pub fn used(&self) -> u64; }
                                                                              // from_ram: min(25 % RAM, 4 GiB), unknown → 1 GiB; per result 256 MiB
pub struct StoreSnapshot { .. }
impl StoreSnapshot { pub fn generation(&self) -> u64; pub fn version(&self) -> u64; pub fn schema(&self) -> &Arc<ResultSchema>;
    pub fn row_count(&self) -> u64; pub fn completeness(&self) -> &Completeness; pub fn bytes(&self) -> u64;
    pub fn cell(&self, row: u64, col: u32) -> ValueRef<'_>; pub fn flags(&self, row: u64, col: u32) -> CellFlags; }
pub struct ResultStore { .. }
impl ResultStore {
    pub fn new(id: ResultId, schema: Arc<ResultSchema>, budget: Arc<MemoryBudget>, generation: u64) -> (Arc<ResultStore>, ResultWriter);
    pub fn snapshot(&self) -> Arc<StoreSnapshot>;                          // one atomic load
    pub fn changes(&self) -> tokio::sync::watch::Receiver<(u64, u64)>;     // (generation, version)
    pub fn raise_cap_to_global(&self);                                     // "Fetch more anyway": an atomic the writer's next admit reads
    #[cfg(feature = "test-support")] pub fn from_rows(schema: Arc<ResultSchema>, rows: Vec<Vec<Value>>, completeness: Completeness) -> Arc<ResultStore>;
}
pub struct ResultWriter { .. }   // single writer: the session actor
impl ResultWriter { pub fn id(&self) -> ResultId; pub fn admit(&self, next_chunk_estimate: u64) -> Admission; pub fn append(&mut self, chunk: Chunk);
                    pub fn set_state(&mut self, c: Completeness); }
pub trait FetchControl: Send + Sync + 'static { fn fetch_more(&self, result: ResultId, rows: u64); fn close(&self, result: ResultId, reason: CloseReason); fn fetch_all(&self, result: ResultId); }
pub struct SourceCaps { pub has_cursor: bool, pub editable: bool }
pub trait ResultSource: Send + Sync + 'static {
    fn snapshot(&self) -> Arc<StoreSnapshot>; fn changes(&self) -> tokio::sync::watch::Receiver<(u64, u64)>;
    fn ensure_rows(&self, upto: u64); fn caps(&self) -> SourceCaps;
    fn fetch_all(&self); fn close_cursor(&self); fn raise_cap(&self);
}
pub struct StoreSource { .. }
impl StoreSource { pub fn new(store: Arc<ResultStore>, control: Arc<dyn FetchControl>, policy: FetchPolicy) -> Arc<Self>; }   // impl ResultSource
```

```rust
// ===== crates/core/hacivat-driver/src/typed.rs (block `// ## m3a-t01b-seams`) =====
pub struct QuerySpec<'a> { pub sql: &'a str, pub binds: &'a [BindValue] /* empty in M3a */, pub protocol: Protocol /* Extended only in M3a */,
    pub suspendable: bool, pub first_rows: u32 /* 501 */, pub chunk_rows: u32, pub chunk_bytes: u64, pub cell_cap: u32,
    pub max_result_sets: u32 /* 1 kept in M3a, R-M3-25 */ }
pub enum NoticeSeverity { Debug, Log, Info, Notice, Warning }
pub struct ResultEnd { pub rows: u64, pub kept: u64, pub rows_affected: Option<u64>, pub command_tag: Option<String> }
pub struct QueryDone { pub tx: TxStatus, pub warnings: u16, pub skipped_result_sets: u32 }
pub enum QueryEvent { ResultStart(Arc<ResultSchema>), Rows(Chunk), ResultEnd(ResultEnd), Suspended, Done(QueryDone) }
                      // grammar: (ResultStart Rows* (Suspended {fetch_more} Rows*)* ResultEnd)* Done
pub enum CloseHow { Drain(Duration), Now, AfterKill(Duration) }
pub enum Closed { Completed { rows: u64 }, Closed { rows: u64 }, NeedsServerKill { thread_id: u64 } }
pub struct TypedCaps { pub suspend: bool, pub binds: bool, pub close_needs_kill: bool,
                       pub idle_bound: Option<Duration> /* MySQL net_write_timeout − 15 s; SQLite 30 s in rollback-journal mode */ }
impl TypedCaps { pub const NONE: TypedCaps; /* all false, idle_bound None */ }
// RawConn gains (defaults fail closed with RawErrorKind::Unsupported; driver.rs, after `autocommit`):
fn typed_caps(&self) -> TypedCaps { TypedCaps::NONE }
fn query<'a>(&'a mut self, spec: QuerySpec<'a>) -> BoxFuture<'a, Result<(), RawError>>;
fn next_event(&mut self) -> BoxFuture<'_, Result<QueryEvent, RawError>>;
fn fetch_more(&mut self, rows: u32) -> BoxFuture<'_, Result<(), RawError>>;
fn close_query(&mut self, how: CloseHow) -> BoxFuture<'_, Result<Closed, RawError>>;
// Rule: the session never drops a next_event/fetch_more/close_query future before it completes, except when it
// discards the connection. One open query per connection, except on PostgreSQL: while the query is `Suspended`,
// `execute`/`batch` may run in the same transaction and the named portal survives (the R-M3-17 canary); MySQL and
// SQLite answer `Unsupported` to a statement while a query is open (P5).
// Contract (P5): on PostgreSQL `suspendable: true` requires `tx_status() != Idle`, else `RawErrorKind::Unsupported`
// (fail closed); the session opens BEGIN or SAVEPOINT (T18), or the guard its read-only BEGIN (T22), before `query`.
// FakeDriver typed scripting (WORKING in T1b, feature `fake`; a `fake_pg()` connection enforces P5's contract like the real driver):
pub enum FakeRows { Literal(Vec<Vec<Value>>), Generated { count: u64, cell: fn(u64, usize) -> Value } }
pub struct FakeResultSet { pub columns: Vec<(&'static str, LogicalType)>, pub rows: FakeRows }
pub struct FakeQuery { pub prefix: &'static str, pub sets: Vec<FakeResultSet>, pub fail_after: Option<(u64, RawError)>,
    pub first_delay: Duration, pub chunk_delay: Duration, pub never_pending: bool, pub ignores_cancel: bool,
    pub tx_after: Option<TxStatus>, pub rows_affected: Option<u64>, pub kill_needed: bool }   // Default derived
pub enum FakeTypedCall { Query(String), FetchMore(u32), Close(CloseHow) }
impl FakeDriver { pub fn queries(self, q: Vec<FakeQuery>) -> Self; pub fn typed_log(&self) -> Vec<FakeTypedCall>; }

// ===== crates/core/hacivat-session (block `// ## m3a-t01b-seams` in session/mod.rs) =====
pub struct RunId(pub u64);
pub struct UnitId { pub run: RunId, pub ordinal: u32 }
#[non_exhaustive] pub enum ErrorPolicy { StopOnError }                        // R-M3-26
pub struct RunUnit { pub id: UnitId, pub sql: Arc<str>, pub protocol: Protocol, pub suspendable: bool }
pub struct RunSpec { pub run: RunId, pub units: Arc<[RunUnit]>, pub policy: ErrorPolicy, pub fetch: FetchPolicy, pub observer: ObserverRef }
pub trait RunObserver: Send + Sync + 'static {                                // called on the actor task; O(1), never blocks
    fn open_result(&self, unit: UnitId, ordinal: u32, schema: Arc<ResultSchema>) -> ResultWriter;
    fn event(&self, ev: &RunEvent);
}
#[derive(Clone)] pub struct ObserverRef(pub Arc<dyn RunObserver>);            // PartialEq/Eq/Hash by pointer (Request keeps its derives)
pub enum UnitEnd { Succeeded { rows: Option<RowsOutcome> }, Failed(ExecError), Cancelled, Skipped }
pub enum RowsOutcome { Fetched { rows: u64, more: bool }, Affected(u64) }
pub struct RunSummary { pub executed: u32, pub total: u32, pub failed: u32, pub skipped: u32, pub aborted: bool, pub duration_ms: u64 }
pub enum RunEvent {
    UnitStarted { unit: UnitId, at: Instant },
    ResultOpened { unit: UnitId, ordinal: u32, result: ResultId },
    RowsFetched { result: ResultId, total: u64 },                            // at most one per 50 ms per result
    ResultEnded { result: ResultId, rows: u64, completeness: Completeness, exec_ms: u64, fetch_ms: u64 },
    UpdateCount { unit: UnitId, rows: Option<u64>, command_tag: Option<String> },
    MoreResultSets { unit: UnitId, skipped: u32 },                           // R-M3-25
    CursorReleased { result: ResultId, rows: u64, reason: CloseReason, rolled_back: bool },   // rolled_back: Hacivat's own transaction ended in ROLLBACK (P26)
    TxChanged { from: TxStatus, to: TxStatus },
    UnitEnded { unit: UnitId, outcome: UnitEnd, exec_ms: u64, fetch_ms: u64 },
    RunEnded(RunSummary),
}
// Request gains (actor stubs answer Err(ExecError::of(ExecErrorKind::Internal)) until T18):
//   Run(RunSpec), FetchMore { result: ResultId, rows: u64 }, FetchAll { result: ResultId }, CloseCursor { result: ResultId, reason: CloseReason }
impl SessionHandle { pub fn run(&self, spec: RunSpec) -> RequestTicket; }   // submit(Request::Run); reply after RunEnded
impl FetchControl for SessionHandle { /* WORKING: submits FetchMore / FetchAll / CloseCursor, ignores the ticket */ }
pub struct CursorBadge { pub result: ResultId, pub rows: u64, pub since: Instant, pub holds_tx: bool }
// SessionSnapshot gains `pub cursor: Option<CursorBadge>`; TxState gains `pub owner: TxOwner`:
pub enum TxOwner { None, User, Cursor, Guard }                                // Default None
pub fn effective_read_only(profile: &Profile, production_read_only: bool) -> bool;   // WORKING (R-M3-29, P4)
// ManagerConfig gains `pub production_read_only: bool` (false in Default and m2b()).
pub struct SessionPolicy { pub production_read_only: bool }                  // Default: false
impl SessionManager { pub fn budget_use(&self) -> (u8, u8);                 // (in_use, max), R-M3-43
                      pub fn set_policy(&self, p: SessionPolicy); }         // ConnectionManager calls it right after `new` (manager.rs:385)
// session/readonly.rs: pub(crate) async fn guarded_query(..) -> Result<(), ExecError>  — T1b stub: Err(Internal); T22 implements.
#[cfg(feature = "test-support")] pub mod testing {                          // WORKING (P6), lifted from tests/common/mod.rs:1242
    pub async fn fake_session(rt: tokio::runtime::Handle, fake: FakeDriver, profile: Profile) -> (SessionHandle, Arc<FakeDriver>); }
                                                                              // read-only iff profile.read_only

// ===== crates/core/hacivat-testkit (block `// ## m3a-t01b-seams`) =====
pub fn perf_enabled() -> bool;                                               // HACIVAT_PERF=1 (P9); every perf test self-gates on it
pub fn perf_enabled_from(v: Option<&str>) -> bool;                           // the testable core, like docker_enabled_from

// ===== crates/core/hacivat-exec (T23) =====
pub mod units {
    pub enum RunAction { Execute1, Execute2, Execute3 }
    pub fn policy_for(a: RunAction) -> ExecPolicy;                            // Execute1 = D-011 default; 2 = largest(); 3 = whole_script()
    pub struct ExecUnit { pub text: Arc<str>, pub doc_range: Range<usize>, pub kind: UnitKind, pub suspendable: bool }
    pub enum Resolution { Units(Vec<ExecUnit>), Ask { candidates: Vec<Candidate>, preselect: usize }, NothingToRun,
                          Uncertain(Vec<AmbiguousBoundary>), UnsupportedClientCommand { at: Range<usize> } }
    pub fn resolve_units(doc: &DocSnapshot, at: &[CaretOrSelection], action: RunAction, remembered: Option<CandidateKind>) -> Resolution;
    pub fn units_for_candidate(doc: &DocSnapshot, c: &Candidate) -> Vec<ExecUnit>;
    // T1b stub: NothingToRun
}
pub mod sqlctx {
    pub fn dialect_id(kind: DbmsKind) -> DialectId;
    pub fn probe_sql(kind: DbmsKind) -> Option<&'static str>;                  // PG `SHOW standard_conforming_strings`; MySQL/MariaDB `SELECT @@SESSION.sql_mode`
    pub fn dialect_ctx(kind: DbmsKind, version: Option<&SemVerLoose>, probe_row: Option<&str>) -> DialectCtx;
}
pub mod status {
    pub struct UnitSummary { pub rows: Option<RowsOutcome>, pub exec_ms: u64, pub fetch_ms: u64 }
    pub struct ErrorView { pub title_id: &'static str, pub code: Option<Arc<str>>, pub server_text: Option<Arc<str>>, pub doc_offset: Option<usize> }
    pub enum UnitStatus { Queued { position: u32 }, Running { since: Instant }, Fetching { rows: u64 }, Suspended { rows: u64 },
                          Cancelling, Succeeded(UnitSummary), Failed(ErrorView), Cancelled, Skipped }
    pub struct UnitStatusEntry { pub unit: UnitId, pub doc_range: Range<usize>, pub status: UnitStatus }
    pub struct RunStatus { pub units: Arc<[UnitStatusEntry]>, pub summary: Option<RunSummary> }
}
pub mod log {
    pub enum Severity { Info, Notice, Warning, Error }
    pub enum LogMsg { RowsRetrieved { rows: u64, more: bool, exec_ms: u64, fetch_ms: u64 }, RowsAffected { rows: u64, ms: u64 },
        Completed { ms: u64 }, TxCommitted, TxRolledBack, CursorClosed { rows: u64, reason: CloseReason, rolled_back: bool }, MoreResultSets { skipped: u32 },
        Summary(RunSummary), Error(ErrorView), Cancelled, ReadOnlyDenied(DenyLayer), Dropped { n: u64 } }
    pub enum LogArg { U64(u64), Str(Arc<str>) }
    impl LogMsg { pub fn fluent_id(&self) -> &'static str; pub fn args(&self) -> Vec<(&'static str, LogArg)>; }   // fluent_id: a literal in every arm (P24)
    pub struct LogLine { pub seq: u64, pub at: SystemTime, pub severity: Severity, pub msg: LogMsg, pub source: Option<Range<usize>>, pub unit: Option<UnitId> }
    pub struct SessionLog { /* ring of 10,000 */ }
    impl SessionLog { pub fn new() -> Arc<Self>; pub fn push(&self, line: LogLine); pub fn lines(&self) -> Arc<[LogLine]>;
                      pub fn changes(&self) -> tokio::sync::watch::Receiver<u64>; pub fn clear(&self); }
    pub fn copy_invariant(lines: &[LogLine]) -> String;                     // ISO timestamps, ASCII digits, "ms"
}
pub mod service {
    pub struct TabKey { pub console: u64, pub unit_ordinal: u32, pub result_ordinal: u32 }
    pub struct ResultTab { pub key: TabKey, pub title: Arc<str>, pub unit: UnitId, pub source: Arc<dyn ResultSource> }
    pub struct RunRequest { pub console: u64, pub units: Vec<ExecUnit>, pub fetch: FetchPolicy, pub log: Arc<SessionLog> }   // the console's own log
    pub struct RunTicket { pub seq: u64, pub run: RunId, pub status: watch::Receiver<RunStatus>,
                           pub results: watch::Receiver<Arc<[ResultTab]>>, pub done: oneshot::Receiver<RunSummary> }
    pub struct ExecService { .. }
    impl ExecService { pub fn new(budget: Arc<MemoryBudget>) -> Self;                             // one per app
                       pub fn run(&self, console: &SessionHandle, req: RunRequest) -> RunTicket; }   // T1b stub: done answers aborted, nothing sent
}
pub mod ux {
    pub enum TxBadge { Auto, UserOpen { since: Instant, statements: u32 }, Failed, CursorSnapshot { since: Instant } }
    pub enum CancelUx { Idle, Cancelling { since: Instant } }
    pub struct ConsoleUx { pub env: Option<EnvTag>, pub production: bool, pub read_only: bool, pub forced_read_only: bool,
        pub access: AccessLabel, pub tx: TxBadge, pub cursor: Option<CursorBadge>, pub cancel: CancelUx,
        pub running: Option<(RunId, Instant)>, pub waiting_for_connection: Option<(u8, u8)> }
    pub fn console_ux(env: Option<EnvTag>, forced: bool, snap: &SessionSnapshot, run: Option<&RunStatus>, waiting: Option<(u8, u8)>) -> ConsoleUx;
}
```

```rust
// ===== crates/ui/hacivat-ui-grid (T17 model; T20 element/view/list/testing) =====
pub struct ModelRow(pub u64); pub struct ViewRow(pub u64); pub struct ColId(pub u32);
pub struct ModelCell { pub row: ModelRow, pub col: ColId }
pub enum ColumnKind { Text, Number, Bool, Temporal, Uuid, Binary, Json, Other }
pub enum Align { Start, End, Center }
pub struct ColumnSpec { pub id: ColId, pub name: SharedString, pub type_label: SharedString, pub kind: ColumnKind, pub align: Align,
                        pub default_width: f32, pub key: bool }
pub struct SourceCaps { pub has_cursor: bool, pub editable: bool }
pub struct RowCount { pub loaded: u64, pub more: bool, pub exact_total: Option<u64> }
pub enum CellKind { Text, Number, Null, Empty, Bool, Temporal, Binary, Json, Error, Placeholder }
bitflags::bitflags! { pub struct GridCellFlags: u8 { const TRUNCATED = 1; const LOSSY = 2; const HAS_CONTROLS = 4; const MULTILINE = 8; const RTL = 16; } }
pub struct CellContent { pub text: SharedString, pub kind: CellKind, pub flags: GridCellFlags, pub align: Align, pub chip: Option<SharedString> }
pub trait GridFrame { fn generation(&self) -> (u64, u64); fn row_count(&self) -> RowCount; fn cell(&self, row: ModelRow, col: ColId) -> CellContent; }
pub trait GridDataSource: 'static {
    fn begin_frame(&self, cx: &App) -> Rc<dyn GridFrame>;                     // one snapshot load per frame
    fn columns(&self) -> Arc<[ColumnSpec]>;
    fn caps(&self) -> SourceCaps;
    fn ensure_rows(&self, upto: u64, cx: &mut App);                           // fire-and-forget; the adapter coalesces
}
pub mod model {
    pub struct ScrollState { pub top_row: u64, pub top_row_offset: f32, pub left_x: f64, pub follow_tail: bool }
    pub struct ColumnLayout { /* order, widths, f64 prefix sums, hidden */ }
    pub struct Selection { /* anchor, focus, rects (view), rows (model), cols, all */ }
    pub struct CopyRegion { pub rows: Vec<Range<u64>>, pub cols: Vec<ColId> }
    pub enum CopyRefusal { MultipleSelection, TooMany { cells: u128 } }
    pub enum Dir { Left, Right, Up, Down }
}
pub const NULL_MARKER: &str = "NULL";                                         // D-065, not Fluent
pub const GRID_CONTEXT: &str = "Grid";
pub struct GridLabels { pub loading: TextFn, pub too_many_columns: TextFn }
pub struct GridConfig { pub row_height: f32 /* 21 */, pub zebra: bool, pub labels: GridLabels, pub a11y_label: TextFn }
pub enum GridCommand { Move { dir: model::Dir, extend: bool }, JumpEdge { dir: model::Dir, extend: bool }, Home { extend: bool }, End { extend: bool },
    FirstRow { extend: bool }, LastRow { extend: bool }, Page { up: bool, extend: bool }, NextCell { back: bool },
    SelectAll, SelectRow, SelectColumn, Copy { headers: bool }, Escape }
pub enum GridEvent { SelectionChanged, FocusChanged(Option<ModelCell>), VisibleRegion { rows: Range<u64>, cols: Range<usize> },
                     CopyRequested { region: model::CopyRegion, headers: bool }, CopyRefused(model::CopyRefusal), ColumnResized { col: ColId, width: f32 } }
pub struct GridView { .. }   // impl Render, Focusable, EventEmitter<GridEvent>
impl GridView { pub fn new(source: Rc<dyn GridDataSource>, config: GridConfig, cx: &mut Context<Self>) -> Self;
    pub fn set_source(&mut self, source: Rc<dyn GridDataSource>, keep_widths: bool, cx: &mut Context<Self>);
    pub fn dispatch(&mut self, cmd: GridCommand, window: &mut Window, cx: &mut Context<Self>);
    pub fn selection(&self) -> &model::Selection; pub fn scroll_px(&mut self, dy: f64, dx: f64, cx: &mut Context<Self>); }
pub trait RowRenderer: 'static { fn len(&self) -> u64; fn row(&self, ix: u64, window: &mut Window, cx: &mut App) -> AnyElement; }
pub struct VirtualList<R: RowRenderer> { .. }
// testing.rs (feature `test-support`): SyntheticSource (the M0 spike source behind GridDataSource)

// ===== crates/ui/hacivat-ui-core::gpui_surface (re-exports added in T1b) =====
// Hitbox, HitboxBehavior, CursorStyle, StyleRefinement, A11ySubtreeBuilder, TextInputConfiguration, Autocapitalize,
// ClipboardString, MouseExitEvent, ModifiersChangedEvent, PaintQuad, quad, Corners, Edges, BorderStyle, ScrollDelta,
// TouchPhase, StrikethroughStyle (verified at a1eaa20 while writing the plan; T1b re-checks them at the new rev). platform::memory::physical_memory_bytes() -> Option<u64> (stub None).

// ===== crates/app/hacivat-app (T1b stubs; T26, T30, T32, T33, T34 fill) =====
pub type ConsoleViewFn = Rc<dyn Fn(TabId, u32, Option<ProfileId>, &mut Window, &mut App) -> AnyView>;   // built once per tab; the profile is the saved binding (P12)
pub struct TabDecor { pub title: Option<SharedString>, pub title_source: TextSource, pub env: Option<EnvMark>, pub hairline: bool }
pub type TabDecorFn = Rc<dyn Fn(TabId, &App) -> TabDecor>;
pub enum CloseDecision { Proceed, Defer }
pub type TabCloseGuardFn = Rc<dyn Fn(TabId, &mut Window, &mut App) -> CloseDecision>;   // Defer: the guard closes later via Workspace::force_close_tab
pub type QuitGuardFn = Rc<dyn Fn(&mut App) -> CloseDecision>;                           // Defer: the guard calls cx.quit() later
// ShellHooks gains: console_view, results_view: Option<ViewBuilder>, tab_decor, tab_close_guard, quit_guard (+ setters).
// Every empty slot keeps today's rendering (M1/M2 shots unchanged).
// shell: TabKind::Console { n: u32, profile: Option<ProfileId> }; SavedKind::Console { n, #[serde(default)] profile: Option<String> } (P12).
pub mod console {
    pub struct ConsoleId(pub u64);
    pub struct ConsoleState { pub id: ConsoleId, pub tab: TabId, pub n: u32, pub profile: Option<ProfileId>, pub client: ClientId,
        pub session: Option<SessionHandle>, pub editor: Option<Entity<EditorView>>, pub results: Option<Entity<results::ConsoleResults>>,
        pub sql: Option<DialectCtx>, pub log: Arc<SessionLog> }
    pub struct Consoles { /* Global; by_tab, states, focused */ }            // WORKING minimal registry in T1b
    impl Consoles { pub fn register(cx: &mut App, s: ConsoleState) -> ConsoleId; pub fn get(cx: &App, id: ConsoleId) -> Option<&ConsoleState>;
                    pub fn update(cx: &mut App, id: ConsoleId, f: impl FnOnce(&mut ConsoleState)); pub fn by_tab(cx: &App, tab: TabId) -> Option<ConsoleId>;
                    pub fn remove(cx: &mut App, id: ConsoleId); }
    pub fn focused_console(window: &Window, cx: &App) -> Option<ConsoleId>;
    pub fn editor_segment(console: ConsoleId, cx: &App) -> Option<AnyView>;   // T1b stub: None; T32 fills it (the R-M3-11 entity); T34 places it
    // files (P22): ConsoleId, ConsoleState, Consoles in registry.rs; focused_console, editor_segment in view.rs; mod.rs: module lines, re-exports, install
}
pub mod results {
    pub enum ActiveResult { Output, Tab(usize) }
    pub enum CompletenessView { Streaming, Suspended, Complete, Closed, Cancelled, Truncated, Released }
    pub struct ResultContext { pub profile: ProfileId, pub env: Option<EnvTag>, pub access: AccessLabel, pub loaded: u64,
        pub exact: Option<u64>, pub exec_ms: Option<u64>, pub fetch_ms: Option<u64>, pub completeness: CompletenessView }
    pub struct ConsoleResults { /* output, tabs, active, unread_output */ }
    pub enum ResultsEvent { Rerun { console: ConsoleId, unit_ordinal: u32 }, GoToSource { console: ConsoleId, range: Range<usize> } }
    pub fn active_result_context(window: &Window, cx: &App) -> Option<ResultContext>;
    pub struct GridAdapter { .. }   // impl GridDataSource (T26); T1b stub: zero rows, no columns
    pub mod view { pub fn build(window: &mut Window, cx: &mut App) -> AnyView; }   // the `results_view` ViewBuilder; T1b registers it in results::install (stub: today's tile); T30 fills
    // files (P22): ConsoleResults, ActiveResult in tabs.rs; ResultContext, CompletenessView, active_result_context in context.rs; GridAdapter in adapter.rs;
    // ResultsEvent in view.rs; mod.rs: module lines, re-exports, install (T1b's; T26 and T30 never edit it)
}
pub mod exec_ui {
    pub fn attach(console: console::ConsoleId, editor: &Entity<EditorView>, window: &mut Window, cx: &mut App);   // T1b stub: subscribes, sends nothing
}
```

**Keymap rows** (block `## m3a-t01b-seams`; commands with no registration yet do nothing, `default-macos.toml:51-52`):
- Editor (context `Editor`), macOS / Windows-Linux: arrows and Shift+arrows (`editor.move_*`, `editor.select_*`); `alt-left/right` / `ctrl-left/right` word moves; `cmd-left/right` and `home`/`end` / `home`/`end` smart line start/end; `cmd-up/down` / `ctrl-home/end` document; `pageup`/`pagedown`; `enter`, `tab`, `shift-tab`; `backspace`, `delete`; `alt-backspace`/`alt-delete` / `ctrl-backspace`/`ctrl-delete`; `cmd-backspace` delete to line start (macOS only); `cmd-a/z/shift-z` / `ctrl-a/z/shift-z`; `cmd-c/x/v` / `ctrl-c/x/v`; `cmd-enter` / `ctrl-enter` (`editor.run`); `escape`; `ctrl-m` (`editor.toggle_tab_moves_focus`); `shift-f10` (`editor.context_menu`). `editor.run_2`, `editor.run_3`, `editor.describe_exec_range` unbound (palette).
- Grid (context `Grid`): arrows/Shift; `cmd-arrow` / `ctrl-arrow` jump to edge; `home`/`end`; `cmd-up/down` / `ctrl-home/end` first/last row; `pageup`/`pagedown`; `tab`/`shift-tab`; `cmd-a` / `ctrl-a`; `shift-space` select row; select column unbound in M3a (palette `grid.select_column`, R-M3-38); `cmd-c`/`cmd-shift-c` / `ctrl-c`/`ctrl-shift-c`; `escape`.
- Console (context `Console`): `exec.cancel` `cmd-.` / `ctrl-break` and a second row `ctrl-f2` (Windows/Linux, R-M3-38). `presets/datagrip.toml`: `exec.cancel` `ctrl-f2`.

- [ ] **Step 1: Write the failing tests.**
```rust
// xtask/src/check_deps.rs (mod tests, with fx_at/rule_of)
#[test] fn sql_crate_with_tokio_is_flagged() { let md = fx_at(&[("sql-lexer", "crates/sql", &["tokio"])]); assert_eq!(rule_of(&md, "sql-lexer"), Rule::SqlNotRuntimeFree); }
#[test] fn sql_crate_depending_on_core_is_flagged() { let md = fx_at(&[("sql-split", "crates/sql", &["hacivat-proto"])]); assert_eq!(rule_of(&md, "sql-split"), Rule::SqlReachesCore); }
#[test] fn pg_query_outside_the_oracle_is_flagged() { let md = fx_at(&[("sql-split", "crates/sql", &["pg_query"])]); assert_eq!(rule_of(&md, "sql-split"), Rule::BackendOutsideOwner); }
#[test] fn text_core_with_the_adapter_is_flagged() { let md = fx_at(&[("hacivat-text", "crates/ui", &["hacivat-ui-core"])]); assert_eq!(rule_of(&md, "hacivat-text"), Rule::TextCoreNotHeadless); }
#[test] fn new_ui_and_app_crates_are_product_crates() { for c in ["hacivat-ui-editor", "hacivat-ui-grid", "hacivat-sql-editor", "hacivat-text", "hacivat-ui"] { assert!(PRODUCT_CRATES.contains(&c), "{c}"); } }
// xtask/src/docker.rs (mod tests)
#[test] fn test_target_passes_through_before_the_double_dash() { let a = test_args(&["hacivat-exec".into()], Some("docker_m3a_matrix"));
    let t = a.iter().position(|s| s == "--test").unwrap(); assert_eq!(a[t + 1], "docker_m3a_matrix"); assert!(t < a.iter().position(|s| s == "--").unwrap()); }
#[test] fn exec_is_a_default_docker_crate() { assert!(DEFAULT_TEST_CRATES.contains(&"hacivat-exec")); }
// crates/core/hacivat-testkit/src/lib.rs (mod tests)
#[test] fn perf_needs_the_variable() { assert!(!perf_enabled_from(None)); assert!(!perf_enabled_from(Some("0"))); assert!(perf_enabled_from(Some("1"))); }
// crates/ui/hacivat-text/tests/buffer_minimal.rs
#[test] fn utf16_round_trips_turkish_and_emoji() { let b = TextBuffer::from_str("İı😀x"); assert_eq!(b.len_utf16(), 5); assert_eq!(b.utf16_to_byte(b.byte_to_utf16(8)), 8); }
#[test] fn crlf_is_one_line_break() { assert_eq!(TextBuffer::from_str("a\r\nb").len_lines(), 2); }
#[test] fn u2028_is_not_a_line_break() { assert_eq!(TextBuffer::from_str("a\u{2028}b").len_lines(), 1); }
#[test] fn bom_moves_into_meta() { let b = TextBuffer::from_str("\u{FEFF}x"); assert!(b.meta().bom); assert_eq!(b.to_string(), "x"); }
#[test] fn dispatch_inserts_and_maps_the_selection() { let mut m = EditorModel::new("ab", ModelConfig::default());
    let c = m.dispatch(insert_at(1, "X")).unwrap(); assert_eq!(m.snapshot().buffer.to_string(), "aXb"); assert_eq!(c.version, 1); }
// crates/ui/hacivat-ui-editor/tests/stubs.rs
#[test] fn fail_closed_service_never_previews_a_range() { let p = FailClosedService.exec_preview(DocId(1), 0, &ExecAt::Carets(smallvec![0]), ExecAction::Execute1).unwrap();
    assert!(p.ranges.is_empty()); assert_eq!(p.state, PreviewState::Uncertain); }
// crates/core/hacivat-values/tests/m3a_types.rs
#[test] fn value_is_at_most_32_bytes() { assert!(std::mem::size_of::<Value>() <= 32); }
#[test] fn chunk_from_values_round_trips() { let c = Chunk::from_values(&schema2(), vec![vec![Value::Int(1), Value::Null]]); assert_eq!(c.rows(), 1); assert!(matches!(c.cell(0, 1), ValueRef::Null)); }
// crates/core/hacivat-session/tests/m3a_interfaces.rs (FakeDriver; `BareConn` is a test-local `RawConn` with the 12 required methods)
#[tokio::test] async fn run_fails_closed_until_task_18() { let (h, _) = session(fake_pg()); let t = h.run(spec_of(&["SELECT 1"]));
    assert_eq!(t.reply.await.unwrap().unwrap_err().kind, ExecErrorKind::Internal); }
#[tokio::test] async fn typed_defaults_are_unsupported() { let mut c = BareConn::default(); assert_eq!(c.next_event().await.unwrap_err().kind, RawErrorKind::Unsupported); }
#[tokio::test] async fn fake_driver_scripts_a_suspending_query() { let f = fake_pg().queries(vec![FakeQuery { prefix: "SELECT", sets: vec![generated(1_200)], ..Default::default() }]);
    let mut c = connect_fake(&f).await; c.batch("BEGIN").await.unwrap(); c.query(spec("SELECT * FROM t", true)).await.unwrap();
    assert!(matches!(c.next_event().await.unwrap(), QueryEvent::ResultStart(_)));
    assert!(matches!(c.next_event().await.unwrap(), QueryEvent::Rows(ch) if ch.rows() == 501));
    assert!(matches!(c.next_event().await.unwrap(), QueryEvent::Suspended));
    assert_eq!(f.typed_log(), vec![FakeTypedCall::Query("SELECT * FROM t".into())]); }
#[tokio::test] async fn fake_pg_refuses_a_suspendable_query_outside_a_transaction() { let f = fake_pg().queries(vec![FakeQuery { prefix: "SELECT", sets: vec![generated(10)], ..Default::default() }]);
    let mut c = connect_fake(&f).await; assert_eq!(c.query(spec("SELECT * FROM t", true)).await.unwrap_err().kind, RawErrorKind::Unsupported); }   // P5
#[test] fn production_is_read_only_only_with_the_flag() { let p = profile_env(EnvTag::Production);
    assert!(effective_read_only(&p, true)); assert!(!effective_read_only(&p, false)); assert!(effective_read_only(&read_only_profile(), false)); }
#[tokio::test] async fn production_policy_reaches_a_connected_session() { /* ConnectionManager with production_read_only: true over FakeDriver: a production profile's default session is read-only, a dev profile's is not (set_policy, P4) */ }
// crates/core/hacivat-exec/tests/stubs.rs
#[test] fn resolve_stub_runs_nothing() { assert!(matches!(resolve_units(&doc("SELECT 1"), &[caret(0)], RunAction::Execute1, None), Resolution::NothingToRun)); }
// crates/app/hacivat-app/tests/m3a_routing.rs (gpui; support helpers)
#[gpui::test] fn unbound_console_renders_the_m2_home(cx: &mut TestAppContext) { let vcx = shell_with_connections(cx); install_m3a_hooks(&vcx);
    assert!(rendered_markers(&vcx).contains(&"connections-home")); }
#[gpui::test] fn console_view_is_built_once_per_tab(cx: &mut TestAppContext) { let (vcx, builds) = counting_console_hook(cx);
    for _ in 0..3 { vcx.run_until_parked(); redraw(&vcx); } assert_eq!(builds.get(), 1); }
#[gpui::test] fn empty_results_hook_keeps_the_m1_tile(cx: &mut TestAppContext) { let vcx = shell(cx, "en"); assert!(rendered_markers(&vcx).contains(&"results-empty")); }
#[cfg(feature = "scene-check")] #[gpui::test] fn hairline_is_one_red_quad_at_the_pane_top(cx: &mut TestAppContext) { let vcx = shell_with_decor(cx, TabDecor { hairline: true, ..decor() });
    let q = red_quads(&vcx); assert_eq!(q.len(), 1); assert_eq!(q[0].bounds.size.height, px(1.0)); }
#[gpui::test] fn edit_menu_appears_only_with_editor_commands(cx: &mut TestAppContext) { /* menu_model without, then with one editor.* ActionDef */ }
#[gpui::test] fn quit_guard_can_defer_quit(cx: &mut TestAppContext) { let vcx = shell_with_quit_guard(cx, CloseDecision::Defer); run(&vcx, "app.quit"); assert!(app_still_running(&vcx)); }
// crates/app/hacivat-app/src/shell/workspace.rs (mod tests: `SavedState`/`SavedKind` are private, workspace.rs:87-101; P18)
#[test] fn old_workspace_file_parses_with_the_new_console_field() { let s: SavedState = serde_json::from_str(M2B_WORKSPACE_JSON).unwrap(); assert!(matches!(first_console(&s), SavedKind::Console { n: 1, profile: None })); }
```
- [ ] **Step 2:** Run `cargo test -p xtask -- check_deps docker`, `cargo test -p hacivat-testkit perf`, `cargo test -p hacivat-text --test buffer_minimal`, `cargo test -p hacivat-session --test m3a_interfaces`, `cargo test -p hacivat-app --test m3a_routing` and `cargo test -p hacivat-app --lib workspace`. Expected: FAIL (crates and types missing).
- [ ] **Step 3: Implement the interfaces, stubs and manifests.** The manifest table is exhaustive; Task 1a's rows are already in the base and stay:

  | Manifest | Adds |
  |---|---|
  | workspace `Cargo.toml` | (Task 1a: `members` += `"crates/sql/*"`, `pg_query`, `text-size`, the `sql-oracle-pg` path entry, `[patch.crates-io] pg_query`, the GPUI `rev`, `[profile.dev.package.pg_query]`.) Workspace deps `twox-hash = { version = "2.1.5", default-features = false, features = ["xxhash3_64", "xxhash3_128", "alloc"] }` (`alloc` gates the streaming XXH3-128 `Hasher` that `content_hash` over rope chunks and the multi-part `StmtKey` need, `src/xxhash3_128.rs:138`; `mysql_async` enables only `xxhash64`; P6), `phf = "0.13"`, `phf_codegen = "0.13"` (xtask only), `memchr = "2"`, `bitflags = "2"`, `smallvec = "1"`, `proptest = "1.12"`, `ropey = { version = "1.6.1", default-features = false, features = ["cr_lines", "simd"] }`, `unicode-width = "0.2"`, `unicode-ident = "1"`, `lru = "0.18"`, `slotmap = "1"`, `bytes = "1"`, `postgres-types = "0.2.14"`, `libc = "0.2"` (in the lock), and path entries for every other new crate; `[patch.crates-io] tokio-postgres = { path = "third_party/tokio-postgres" }` |
  | `xtask` | `phf_codegen` (Task 3's generator; P6) |
  | `sql-text` | `text-size` |
  | `sql-dialect` | `sql-text`, `phf`, `bitflags`. Dev: `proptest` |
  | `sql-lexer` | `sql-dialect`, `sql-text`, `memchr`, `bitflags`, `smallvec`. Dev: `proptest`, `sql-testkit` |
  | `sql-split` | `sql-lexer`, `sql-dialect`, `sql-text`, `twox-hash`, `smallvec`. Dev: `proptest`, `sql-testkit`, `rusqlite` (bundled; the SQLite differential), `sql-oracle-pg` (the PG differential) |
  | `sql-oracle-pg` | `pg_query`, `text-size` (Task 1a), `sql-text` |
  | `sql-testkit` | `sql-split`, `sql-lexer`, `sql-dialect`, `serde`, `toml`; `publish = false` |
  | `hacivat-text` | `ropey`, `smallvec`, `unicode-segmentation`, `unicode-ident`, `unicode-width`, `twox-hash`. Dev: `proptest` |
  | `hacivat-ui-core` | `windows-sys` features += `Win32_System_SystemInformation` (`GlobalMemoryStatusEx`); `[target.'cfg(target_os = "macos")'.dependencies] libc` (`sysctlbyname("hw.memsize")`); both for Task 33's `physical_memory_bytes` (P6) |
  | `hacivat-ui-editor` | `hacivat-ui-core`, `hacivat-ui`, `hacivat-text`, `smallvec`, `lru`, `unicode-segmentation`, `twox-hash`; `[features] scene-check = ["hacivat-ui-core/scene-check"]`. Dev: `gpui` (`test-support`), `hacivat-ui-core` (`test-support`) |
  | `hacivat-ui-grid` | `hacivat-ui-core`, `hacivat-ui`, `smallvec`, `bitflags`, `unicode-segmentation`; `[features] test-support = []`, `scene-check = ["hacivat-ui-core/scene-check"]`, `bench = ["hacivat-ui-core/bench"]`; `[[example]] b1_synthetic required-features = ["bench", "test-support"]`; `[[test]]` `element`, `view`, `list` with `required-features = ["test-support"]` (they use `SyntheticSource`; P6). Dev: `gpui` (`test-support`), `hacivat-ui-core` (`test-support`) |
  | `hacivat-sql-editor` | `hacivat-ui-editor`, `hacivat-ui-core`, `hacivat-text`, `sql-split`, `sql-lexer`, `sql-dialect`, `sql-text`, `smallvec`, `tokio` (`sync`: the worker's `watch`), `futures` (P6). Dev: `gpui` (`test-support`), `proptest`, `ropey` |
  | `hacivat-values` | `hacivat-proto`, `bytes`, `bitflags`, `smallvec`, `icu_decimal`, `fixed_decimal`, `icu_locale`, `jiff`, `unicode-segmentation`. Dev: `proptest` |
  | `hacivat-results` | `hacivat-values`, `arc-swap`, `tokio` (`sync`), `futures`; `[features] test-support = []`; `[[test]] store` with `required-features = ["test-support"]` (`from_rows`; P6) |
  | `hacivat-exec` | `hacivat-proto`, `hacivat-values`, `hacivat-results`, `hacivat-session`, `hacivat-driver`, `sql-split`, `sql-lexer`, `sql-dialect`, `sql-text`, `sql-oracle-pg`, `tokio` (`sync`, `time`), `futures`. Dev: `hacivat-driver` (`fake`), `hacivat-session` (`test-support`: `testing::fake_session`), `hacivat-testkit` (`ssh`), `tokio` (`macros`, `rt-multi-thread`, `test-util`), and the other dev dependencies the session crate's Docker tests use, so Task 31's `docker_m3a_matrix` builds real sessions here (P6) |
  | `hacivat-driver` | `hacivat-values`, `bytes` |
  | `hacivat-driver-pg` | `hacivat-values`, `postgres-types`, `bytes` |
  | `hacivat-driver-mysql` | `hacivat-values` |
  | `hacivat-driver-sqlite` | `hacivat-values`; `rusqlite` adds `column_decltype` |
  | `hacivat-session` | `hacivat-values`, `hacivat-results`; `[features] test-support` += `"hacivat-driver/fake"` (`testing::fake_session`, P6). Dev: `hacivat-results` (`test-support`) |
  | `hacivat-app` | `hacivat-ui-editor`, `hacivat-ui-grid`, `hacivat-text`, `hacivat-sql-editor`, `hacivat-values`, `hacivat-results`, `hacivat-exec`, `sql-split`, `sql-dialect`, `sql-text`, `smallvec` (the app builds `ExecAt` and the preview override, P6); `[features] bench = ["hacivat-ui-core/bench", "hacivat-driver/fake"]`, `shots` += `"hacivat-results/test-support", "hacivat-ui-grid/test-support"`; `[[example]] b3`, `[[example]] b1_real` (`required-features = ["bench"]`). Dev: `hacivat-results` (`test-support`), `hacivat-ui-grid` (`test-support`); `register_test_console` reaches `hacivat_session::testing` through the existing `test-support` feature |

  Then:
  - `check_deps.rs`: the four rules, built like `GuardNotRuntimeFree`. `SqlNotRuntimeFree`: a normal-edge breadth-first walk from every member under `crates/sql/` except the dev-only `sql-testkit`, against `SQL_FORBIDDEN` = the guard list + `async-std`, `smol`. `SqlReachesCore`: any direct dependency (normal, build or dev) of a `crates/sql/*` member that lives under `crates/core/`. `BackendOutsideOwner`: `pg_query` as a direct dependency of any member other than `sql-oracle-pg`. `TextCoreNotHeadless`: the normal-edge closure of `hacivat-text` contains no `hacivat-ui-core`, `gpui` or zed-sourced crate. `PRODUCT_CRATES` gains `hacivat-ui-editor`, `hacivat-ui-grid`, `hacivat-sql-editor`, `hacivat-text` and `hacivat-ui` (P22).
  - CI block `# ## m3a-t01b-seams` in `build-test` and `turkey-test`, after `Test (4/4)`: lines 6–8 (`cargo test -p hacivat-testkit --features ssh`, `cargo test -p hacivat-ui-editor --features scene-check`, `cargo test -p hacivat-ui-grid --features test-support,scene-check`; P10).
  - ADR sections `## m3a-t01b — seams`: 0005 names M5's `salsa` exemptions (`sql-sema`, `sql-ide`; P22); 0006 records the `Value` layout (P8) and the PostgreSQL exception to "one open query per connection" (P5); 0007 the editor seams and their owners (P11).
  - `effective_read_only` replaces the seven sites of R-M3-29 (P4), and `ConnectionManager` calls `set_policy` right after `SessionManager::new`. `impl FetchControl for SessionHandle` submits requests and drops the tickets.
  - `testing::fake_session` is `tests/common/mod.rs:1242`'s `session`/`ro_session` lifted behind `test-support`; the session crate's own tests may keep their helpers.
  - Do not reformat any upstream file in `third_party/tokio-postgres`; `diff -r` against the registry source shows only the documented manifest edits.
- [ ] **Step 4:** Run Step 2's commands, then `cargo check --workspace --all-targets`, `cargo test -p hacivat-session --no-run`, `cargo test -p hacivat-app --no-run`, `cargo test -p hacivat-results --no-run` and `cargo test -p hacivat-ui-grid --no-run` (no features), `cargo clippy --workspace --all-targets -- -D warnings`, `cargo xtask check-deps`, `cargo xtask l10n check`, `cargo deny --all-features check licenses bans sources`, `test -z "$(cargo tree -i aws-lc-rs --workspace 2>/dev/null)"`, test lines 6–8, the M2b suites (`cargo test -p hacivat-session -p hacivat-driver -p hacivat-driver-pg -p hacivat-driver-mysql -p hacivat-driver-sqlite` and `cargo test -p hacivat-app --features scene-check,pseudo`, the eight P21 files included), and `cargo test -p hacivat-app --features shots,scene-check,pseudo --test shots` with `cargo xtask shot-diff --baseline <main>/.superpowers/sdd/2026-11-14-m3-walking-skeleton/baseline-m2b --candidate <this run's shots directory>`. Then `git status --porcelain -- third_party/pg_query` is empty. Expected: PASS; every pre-existing shot identical; `cargo tree -i pg_query` shows only `sql-oracle-pg` as a direct dependent; each example stub exits 2.
- [ ] **Step 5: Commit** `feat(m3a): interface commit — sql/editor/grid/values/results/exec crates, typed RawConn surface, Run requests, vendored tokio-postgres, check-deps rules, editor and app seams, test support`. The body lists the `tokio-postgres` lock entry change and every M2b app test expectation that changed (P21). After the rebase onto Task 1a, the controller pushes `m3a-seams`, waits for `build-test` green on macOS, Windows and Linux, and merges once the review passes. W1 starts.

**Review:** opus, adversarial (security). Focus: every "Consumes" line of Tasks 3–36 against this task's names and signatures (pre-flight Part 4). Probes:
- Does any stub succeed without its check (No silent weakening): typed defaults, the PostgreSQL suspendable contract (real and fake), `Run`, the typed read-only path, `resolve_units`, the preview, the app's Run stub?
- Does `effective_read_only` cover the seven sites, does `set_policy` run right after `SessionManager::new`, and does `grep -nE '\.read_only\b' crates/core/hacivat-session/src` find any other site (`current().read_only` included)?
- Does any public signature outside `sql-oracle-pg` carry a `pg_query` type?
- Is the vendored `tokio-postgres` byte-identical to the registry source apart from the documented manifest edits, and absent from `cargo metadata`'s workspace members?
- Does each type sit in the file of the task that fills it (P22), so that no later task must edit a `lib.rs` or `mod.rs` it does not own? Do the editor seams (P11) and the app seams (P12) let Tasks 27–29 and 32–34 run in parallel without touching each other's files?
- Is `Value` 32 bytes? Do the example and bench stubs exit non-zero? Does `cargo test -p <crate> --no-run` pass without features for every new crate?
- Did any M2b app test change its meaning beyond the read-only expectation (P21)? Does any M1/M2 shot move? Does an empty hook slot change rendering anywhere?

### Task 2: Mocks for the M3a surfaces (W0, parallel with Tasks 1a and 1b)

**Files:** Create `design/m3a-query/index.html` and `design/m3a-query/NOTES.md`.

**Interfaces:**
- Consumes: `design/m2b-secure/index.html` (CSS, the Lucide sprite, the mock bar, the `L` EN/TR, `T` theme and `N` notes toggles, deep links); harman (`design/directions/harman/index.html`: editor band `:331`, gutter `:323-330`, grid `:429-463`, strip `:790-797`, caret `:349`); `DESIGN.md`; `.impeccable/design.json`.
- Produces: the binding mock of surfaces 1–14 in "New UI surfaces" (with the M3b sketches) and a copy table `id | EN | TR | Status` mapped to Fluent prefixes: `hv-console-*`, `hv-editor-*` (app), `hui-editor-*` (editor widget text, `hui.ftl`), `hv-chooser-*`, `hv-results-*`, `hv-grid-*`, `hv-strip-*`, `hv-tx-*`, `hv-copy-*`, and `exec-*` / `cursor-*` (log and cursor messages, `hacivat-core.ftl`). Every Turkish string carries "PO review".

- [ ] **Step 1:** Load the `impeccable` skill (or `frontend-design:frontend-design`). Read DESIGN.md "Editor", "Results, Grid", "Status Bar", "Overflow Rule", "Dialog anatomy (prompts)" and the named rules; read `design/m2b-secure/NOTES.md` cross-cutting rules.
- [ ] **Step 2: Build the surfaces**, one surface-switcher entry each, with realistic harman data (orders-dev, orders-staging, orders-prod; Turkish names and cities; a 1M-row result with numeric, timestamp, uuid and JSON columns).
- [ ] **Step 3: Decide in the mock, and record each as a ruling in NOTES "Open questions for the PO":** two statements on one line (full-row band or exact-range tint); timestamp cell style (mono 11.5 `text-2` or body); caret blink (system half-period, static under reduced motion and after 10 s idle; or always static); the gutter growth rule (58 px + (digits − 3) × advance); Failed = ink glyph and Cancelling = amber (never red); the production read-only notice copy (no milestone names in user copy); the chooser's "boundary uncertain" copy; the copy-refusal copy; the close-with-open-transaction copy and button order, and the `Failed` forms of the transaction segment and the prompt (R-M3-16 amended).
- [ ] **Step 4: Write NOTES.md:** per surface Intent, Anatomy, States, Keyboard; then the copy table; then the open questions with their rulings.
- [ ] **Step 5: Verify.** A headless pass (built-in browser skill) of every surface × EN/TR × dark/light at 1280 × 800 and 960 × 600: no console errors, no clipped text in any card, popover, strip or toolbar. `grep -c "PO review" NOTES.md` equals the number of TR strings.
- [ ] **Step 6: Commit** `design(m3a): mocks for the console editor, preview, chooser, gutter, results tile, grid, cursor strip, output log, status segments and the open-transaction prompt`. The controller reviews against DESIGN.md (One Filled Badge, Hue Is Meaning, Raised Ink, 11 px floor), logs `Ruling (PO-away): M3a mocks controller-approved` with the link, and only then dispatches W4's UI tasks.

**Review:** opus (design). Probes: a second filled badge; a new hue (red outside production); `text-3` on a raised surface; server text inside a translated sentence; a Turkish label clipped at 960; a destructive or trust-widening default button.

### Task 3: `sql-text` and the keyword tables (SQL-LEX-01 tables, SQL-LEX-04 lookup) (W1)

**Files:**
- Modify: `crates/sql/sql-text/src/{line_index,edit,chunks}.rs`; `crates/sql/sql-dialect/src/{keywords,version,modes,id}.rs`.
- Create: `crates/sql/sql-dialect/data/keywords/{pg,mysql80,mysql84,mariadb,sqlite}.toml`, `crates/sql/sql-dialect/src/generated/keywords_{pg,mysql,mariadb,sqlite}.rs` (generated, checked in), `xtask/src/gen_sql_tables.rs` (+ one dispatch arm in `xtask/src/main.rs`), `crates/sql/sql-text/tests/line_index.rs`, `crates/sql/sql-dialect/tests/{keywords,modes}.rs`.
- Append: `ci.yml` block `# ## m3a-t03-sql-tables` (`cargo xtask gen-sql-tables --check` in `check-deps`'s job).

**Interfaces:**
- Consumes: Task 1b's `sql-text` and `sql-dialect` types.
- Produces: working `LineIndex`, `TextChunks for str`, `keyword_lookup`, `keyword_text`, `kw::*` constants, `LexModes::{default_for, from_mysql_sql_mode}`, `profile_version`.
  - **Data sources (licence-clean):** PostgreSQL from `libpg_query/src/postgres/include/parser/kwlist.h` inside the vendored `third_party/pg_query` (PostgreSQL License, attributed in `THIRD_PARTY_NOTICES.md`; P1); MySQL 8.0/8.4 and MariaDB from the servers' `INFORMATION_SCHEMA.KEYWORDS` (data the server produces; **never** MySQL's `sql/lex.h`, which is GPL); SQLite from the public-domain keyword list (`https://sqlite.org/lang_keywords.html`). Every row carries `source`, `since` and `until`.
  - `gen-sql-tables` turns the TOML into `phf` maps; `--check` fails when the generated files are stale. `--from-pg-query` rewrites `pg.toml` from the vendored copy's `kwlist.h`; `--from-docker` rewrites the MySQL/MariaDB TOML by running `docker exec <container> mysql -N -e "SELECT WORD, RESERVED FROM INFORMATION_SCHEMA.KEYWORDS"` against the running compose containers (`hacivat-m2a-<service>-1`, with the credentials in `docker/m2a/targets.toml`; a read-only query; the stack stays controller-owned and is up from W1, P19). CI never runs `--from-docker`, only `--check`. `sql-dialect` tests never reach Docker or a core crate (`SqlReachesCore`).
  - Lookup is ASCII-only (`eq_ignore_ascii_case` semantics through an ASCII-upper key ≤ 32 bytes); anything longer or non-ASCII is `None`.

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn keyword_lookup_is_ascii_case_insensitive() { for w in ["LIMIT", "limit", "Limit"] { assert!(keyword_lookup(DialectId::Postgres, w).is_some(), "{w}"); } }
#[test] fn turkish_letters_never_match_keywords() { for w in ["lımıt", "lİmİt", "SELECT\u{0307}"] { assert_eq!(keyword_lookup(DialectId::Postgres, w), None, "{w}"); } }
#[test] fn cyrillic_lookalike_is_not_select() { assert_eq!(keyword_lookup(DialectId::Postgres, "SЕLECT"), None); }   // U+0415
#[test] fn word_over_32_bytes_is_none() { assert_eq!(keyword_lookup(DialectId::MySql, &"A".repeat(33)), None); }
#[test] fn pg_categories_follow_kwlist() { assert_eq!(keyword_lookup(DialectId::Postgres, "select").unwrap().category, KwCategory::Reserved);
    assert_eq!(keyword_lookup(DialectId::Postgres, "abort").unwrap().category, KwCategory::Unreserved); }
#[test] fn ansi_sql_mode_implies_quotes_and_pipes() { let m = LexModes::from_mysql_sql_mode("ANSI"); assert!(m.ansi_quotes && m.pipes_as_concat); }
#[test] fn no_backslash_escapes_is_read() { assert!(LexModes::from_mysql_sql_mode("STRICT_TRANS_TABLES,NO_BACKSLASH_ESCAPES").no_backslash_escapes); }
#[test] fn profile_version_is_stable() { assert_eq!(profile_version(&pg18()), "lex:pg-18@2026.11.1"); }
#[test] fn sqlite_list_has_147_keywords() { assert_eq!(keyword_count(DialectId::Sqlite), 147); }
#[test] fn line_index_round_trips_crlf_and_multibyte() { let t = "a\r\nçğ\nx"; let ix = LineIndex::new(t);
    assert_eq!(ix.line_col(TextSize::from(7)), LineCol { line: 1, col: 4 }); assert_eq!(ix.line_col(TextSize::from(8)), LineCol { line: 2, col: 0 });
    assert_eq!(ix.char_to_byte(t, 3), Some(TextSize::from(3))); }   // byte 7 is the `\n` after `çğ` (P18)
// xtask/src/gen_sql_tables.rs (mod tests)
#[test] fn generated_tables_are_fresh() { assert!(check_fresh(&workspace_root()).is_ok()); }
#[test] fn every_row_names_its_source() { for t in all_tables() { assert!(t.rows.iter().all(|r| !r.source.is_empty())); } }
```
- [ ] **Step 2:** Run `cargo test -p sql-text -p sql-dialect`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies and `xtask gen-sql-tables`; run `--from-pg-query` and `--from-docker` once (the stack is up from W1; the controller confirms it, P19) and commit the TOML. Never call `to_uppercase`/`to_lowercase`.
- [ ] **Step 4:** Run `cargo test -p sql-text -p sql-dialect -p xtask`, `cargo run -p xtask -- gen-sql-tables --check`, and the same tests under `LANG=tr_TR.UTF-8 LC_ALL=tr_TR.UTF-8`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(sql): sql-text line index and chunks; keyword tables for PG, MySQL, MariaDB and SQLite from licence-clean sources; LexModes from sql_mode`.

**Review:** data provenance of every TOML row (no GPL source); ASCII-only casing; version gates (`since`/`until`); the staleness check in CI.

### Task 4: `sql-lexer` (SQL-LEX-02, -04 use, -05 lexing, token classes) (W1)

**Files:**
- Modify: `crates/sql/sql-lexer/src/{cursor,state,token,comments,strings,dollar,numbers,params,ops,words,exec_comment,class,relex}.rs`; `crates/sql/sql-dialect/src/lex_profiles/{pg,mysql,mariadb,sqlite,generic}.rs`.
- Create: `crates/sql/sql-lexer/tests/{cases,props,complexity,exec_comment}.rs`; `crates/sql/sql-testkit/corpus/lex/{pg,mysql,sqlite}.toml` (a carriage return is spelled `\r` inside a TOML string, never a raw CR, P15).

**Interfaces:**
- Consumes: Task 1b's lexer and dialect types. Keyword-dependent tests use a fixture table until Task 3 merges; the wave verification runs them against Task 3's tables.
- Produces: `lex`, `lex_chunks` (M3a: materialises the chunks into one `String` and calls `lex`), `relex` (M3a: full relex, `RelexDelta { changed: 0..len, full: true }`), every `Lexed` accessor (`token_at` and `tokens_in` by binary search over the sorted starts), `token_class`, `exec_comment_gate`, `max_paren_depth`.
  - Rules beyond `alg/10` LEX-02: MySQL `--` starts a comment only before whitespace or a control character; executable comments do not nest, **except that a skipped version-gated comment may hold one nested comment** (MySQL/MariaDB `consume_comment(lip, 1)`, ADR 0004 :807-812); PG operators (`+-` rule, `::`, `->>`, `@>`); dollar tags are case-sensitive and `$1` is not a tag; `x$$y` is one word; `U&'…'` escapes; PG ≤ 15 junk after numbers vs PG 16 separators by `DialectCtx.version`; no recursion anywhere (a depth-10⁶ nested comment uses a counter).
  - `TokenKind::ExecutableComment` carries the version gate; `exec_comment_gate` is `Active`/`Inactive` from `DialectCtx.version`, `Unknown` without one.

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn lex02_cases_1_to_18() { for c in corpus("lex/pg.toml").chain(corpus("lex/mysql.toml")).chain(corpus("lex/sqlite.toml")) { assert_tokens(&c); } }
#[test] fn mysql_double_dash_needs_whitespace() { assert_eq!(kinds(mysql(), "1--1"), [Number, Op, Op, Number]); assert_eq!(kinds(mysql(), "1-- x"), [Number, LineComment]); }
#[test] fn dollar_tags_are_case_sensitive() { assert_eq!(kinds(pg(), "$a$x$A$y$a$"), [DollarString]); }
#[test] fn positional_param_is_not_a_tag() { assert_eq!(kinds(pg(), "$1$"), [PositionalParam, Unknown]); }
#[test] fn skipped_versioned_comment_holds_one_nested_comment() { let l = lex_mysql84("/*!99999 a /* b */ c */ SELECT 1");
    assert_eq!(l.kind(0), ExecutableComment); assert_eq!(l.range(0).end(), TextSize::from(23)); }   // the outer comment closes at byte 23 (P18)
#[test] fn million_deep_comment_does_not_recurse() { let s = "/*".repeat(1_000_000) + &"*/".repeat(1_000_000); let l = lex(&s, pg_profile(), modes(), &LexState::Initial); assert_eq!(l.len(), 1); }
#[test] fn unterminated_string_is_one_flagged_token() { let l = lex_pg("SELECT 'abc"); assert!(l.flags(l.len() - 1).contains(TokenFlags::UNTERMINATED)); }
#[test] fn token_at_and_tokens_in_are_logarithmic() { let l = lex_pg(&"SELECT 1;\n".repeat(50_000)); let probes = counted(|| l.token_at(TextSize::from(250_000))); assert!(probes <= 20); }
#[test] fn max_paren_depth_counts_nesting() { let l = lex_pg("SELECT ((1), (2))"); assert_eq!(l.max_paren_depth(0..l.len()), 2); }
proptest! {
    #[test] fn lossless_for_every_profile_and_mode(s in any_sql(), p in profiles(), m in modes_strategy()) {
        let l = lex(&s, p, m, &LexState::Initial); prop_assert_eq!(concat_tokens(&s, &l), s.clone()); prop_assert!(l.len() <= s.len().max(1)); }
    #[test] fn resuming_from_an_end_state_equals_a_full_lex(a in any_sql(), b in any_sql()) {
        let full = lex(&(a.clone() + &b), pg_profile(), modes(), &LexState::Initial); let left = lex(&a, pg_profile(), modes(), &LexState::Initial);
        prop_assert_eq!(tail_kinds(&full, a.len()), kinds_of(&lex(&b, pg_profile(), modes(), left.end_state()))); }
}
#[test] fn lexing_is_linear() { let small = steps(&"SELECT 1;".repeat(1_000)); let big = steps(&"SELECT 1;".repeat(100_000)); assert!(big <= small * 110); }
#[test] #[ignore = "perf"] fn one_megabyte_lexes_within_10_ms_release() { /* gen-sql fixture, median of 10 */ }
```
- [ ] **Step 2:** Run `cargo test -p sql-lexer`. Expected: FAIL.
- [ ] **Step 3: Implement** a byte cursor with `memchr` for comment, string and quoted-identifier ends, table-driven by `LexDialect`; the struct-of-arrays `Lexed` (kinds `Vec<TokenKind>`, starts `Vec<u32>`, flags `Vec<TokenFlags>`, keywords `Vec<Option<KwEntry>>`).
- [ ] **Step 4:** Run `cargo test -p sql-lexer` and the same under `LANG=tr_TR.UTF-8 LC_ALL=tr_TR.UTF-8`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(sql): total, lossless, table-driven lexer for PG, MySQL, MariaDB, SQLite and Generic; executable comments with version gates; token classes`.

**Review:** losslessness under every profile × mode; MySQL `--`, `#`, backslash and `ANSI_QUOTES` modes against each server's documented lexer; unterminated constructs; no recursion; no casing API.

### Task 5: `hacivat-text` core: buffer, change sets, anchors, change log, model, transactions, history (UI-EDIT-01 model, -02, -03, -04) (W1, gpui-free)

**Files:**
- Modify: `crates/ui/hacivat-text/src/{buffer,change,anchor,changelog,model,transaction,history}.rs`.
- Create: `crates/ui/hacivat-text/tests/{buffer,change,change_props,model,history}.rs`.

**Interfaces:**
- Consumes: Task 1b's `hacivat-text` types and its working `TextBuffer`/minimal `dispatch`.
- Produces: the full model (UI-EDIT-01 dispatch order: filters → buffer apply → selection (given or mapped) → fields in order → version += 1 → observers → dirty ranges; a dispatch from an observer is queued, never re-entrant), `ChangeSet::{from_edits, invert, compose, desc}`, `ChangeDesc::{map_pos, map_range}` with the four modes, `AnchorSet::map` in O(k + s), the 256-entry `ChangeLog::rebase`, and linear history: grouping by family within 500 ms when adjacent; whitespace or `;` closes a group; paste, drop and undo/redo never join; brackets for IME and commands; identity groups dropped; bounds 1,000 groups and 64 MiB; seals on focus change, Run, a non-adjacent move and any command flagged `seals_history`; non-history transactions rebase stored groups.

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn ui_edit_02_cases_1_to_7() { /* alg/25 UI-EDIT-02 test cases */ }
#[test] fn crlf_cr_lf_mix_round_trips_byte_for_byte() { let t = "a\r\nb\rc\nd"; let b = TextBuffer::from_str(t); assert_eq!(b.to_string(), t);
    assert_eq!(b.meta().line_endings, LineEndingStats { lf: 1, crlf: 1, cr: 1 }); }
#[test] fn u2028_is_not_a_line_break() { assert_eq!(TextBuffer::from_str("x\u{2028}y\u{85}z").len_lines(), 1); }
#[test] fn bom_round_trips() { let b = TextBuffer::from_str("\u{FEFF}SELECT 1"); assert!(b.meta().bom); assert_eq!(b.to_string(), "SELECT 1"); }
#[test] fn cr_then_lf_insert_merges_into_crlf_without_splitting_anchors() { /* insert "\r" then "\n" at the same point; one line break; an anchor after it maps once */ }
#[test] fn ui_edit_03_cases_1_to_6() { /* mapping modes, TrackDel deletion, container vs content ranges */ }
#[test] fn ui_edit_01_cases_1_to_5() { /* case 3: dispatch inside an observer is queued; case 4: a snapshot read on another thread is unchanged by later edits */ }
#[test] fn ui_edit_04_cases_1_to_8() { /* 500 ms window, adjacency, word rule, paste never joins, brackets nest, identity dropped */ }
#[test] fn run_seals_the_open_group() { let mut m = model("x"); type_text(&mut m, "ab"); m.seal_history(); type_text(&mut m, "c"); m.undo(); assert_eq!(text(&m), "xab"); }
#[test] fn history_evicts_by_byte_budget() { /* 64 MiB of pastes → oldest groups dropped, newest undoable */ }
#[test] fn programmatic_set_text_rebases_history() { /* Origin::Program edit above; undo still restores the right bytes */ }
proptest! {
    #[test] fn invert_undoes_apply(t in text_s(), e in edits_s()) { let c = ChangeSet::from_edits(t.len(), e).unwrap(); prop_assert_eq!(apply(&apply(&t, &c), &c.invert(&buf(&t))), t); }
    #[test] fn compose_is_associative(a in cs(), b in cs(), c in cs()) { prop_assert_eq!(a.compose(&b).compose(&c), a.compose(&b.compose(&c))); }
    #[test] fn rebase_equals_the_composed_mapping(log in logs_s(), r in range_s()) { prop_assert_eq!(log.rebase(0, r.clone(), RangeKind::Content), direct_map(&log, r)); }
}
#[test] #[ignore = "perf"] fn byte_to_line_at_the_end_of_20_mb_under_5_us() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-text --test buffer --test change --test change_props --test model --test history`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies. `Field`s are plain mutable structs on the UI thread; a `Snapshot` is the rope clone plus the selection and the version. Nothing is O(document) per transaction.
- [ ] **Step 4:** Run `cargo test -p hacivat-text` and `cargo xtask check-deps` (`TextCoreNotHeadless`). Expected: PASS.
- [ ] **Step 5: Commit** `feat(text): rope buffer with exact line endings and BOM, change sets with mapping modes, anchors, 256-entry change log, transactional model and grouped linear history`.

**Review:** char-boundary snapping; CRLF never split by a line command; O(k + s) mapping; grouping exactly as specified; no casing; no gpui in the closure.

### Task 6: `hacivat-values` model and columnar chunks (HV-RES-01, HV-RES-02 buffers) (W1)

**Files:**
- Modify: `crates/core/hacivat-values/src/{logical,native,value,decimal,temporal,special,bind,columnar/{mod,bitmap,buf,builder}}.rs`.
- Create: `crates/core/hacivat-values/tests/{decimal,temporal,columnar,perf_cell}.rs`.

**Interfaces:**
- Consumes: Task 1b's types.
- Produces: `DecimalVal::{parse, from_base10000, scale, write_canonical}` (exact; `[u64; 2]` mantissa; more than 38 digits → `Big(Arc<str>)`), temporal conversions `DateVal::{from_pg_days, to_civil}`, `TimeVal::from_micros`, `TsVal::from_pg_micros` (PG 2000 epoch ↔ 1970; proleptic Gregorian with astronomical years for BC; `24:00:00` kept; timetz offset stored east-positive), `ChunkBuilder` and `Chunk` for every `BufKind` (validity bitmap; exceptions in a sparse map per chunk; `Var` text cut at a UTF-8 boundary with a `Truncated` exception; invalid UTF-8 text → `Lossy`; a chunk seals early before a `Var` offset passes `u32::MAX`), `Chunk::cell` returning a `ValueRef` without allocating.
  - A column index outside the builder's plan panics with a message (the actor's `catch_unwind` turns a driver bug into `Broken(Internal)`; no undefined behaviour).

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn decimals_round_trip_text_exactly() { for s in ["1.50", "-0.00", "12345678901234567890123456789012345678", "0.000001"] { assert_eq!(canon(DecimalVal::parse(s).unwrap()), s); } }
#[test] fn decimal_over_38_digits_is_big() { assert!(matches!(DecimalVal::parse(&"9".repeat(39)).unwrap().repr(), Repr::Big(_))); }
#[test] fn pg_base10000_vectors() { assert_eq!(canon(DecimalVal::from_base10000(false, 1, 2, &[1, 2345])), "12345.00");
    assert_eq!(canon(DecimalVal::from_base10000(true, -1, 6, &[1200])), "-0.120000"); }
#[test] fn no_f64_on_the_decimal_path() { assert!(!source_of("src/decimal.rs").contains("f64")); }
#[test] fn pg_epoch_shift_and_bc_dates() { assert_eq!(DateVal::from_pg_days(0), DateVal(10_957)); assert_eq!(DateVal::from_pg_days(-2_451_546).to_civil(), (-4713, 11, 23)); }   // 4714 BC = year −4713
#[test] fn time_24_00_is_kept() { assert_eq!(TimeVal::from_micros(86_400_000_000).nanos_of_day, 86_400_000_000_000); }
#[test] fn timetz_offset_is_east_positive() { assert_eq!(timetz_from_pg(0, -10_800).1, 10_800); }
#[test] fn every_buf_kind_holds_nulls_and_exceptions() { /* one chunk per BufKind: values, NULLs, one exception; cell() equals pushed */ }
#[test] fn var_text_truncates_at_a_char_boundary() { let c = chunk_text("ğ".repeat(600_000), 1 << 20); assert!(matches!(c.cell(0, 0), ValueRef::Truncated(t) if t.total_len == Some(1_200_000))); }
#[test] fn invalid_utf8_text_becomes_lossy() { let c = chunk_text_bytes(&[0x66, 0xff, 0x6f], 1 << 20); assert!(matches!(c.cell(0, 0), ValueRef::Lossy(b) if b == [0x66, 0xff, 0x6f])); }
#[test] fn bytes_accounting_is_exact() { let c = chunk_i64(4_096); assert_eq!(c.bytes(), 4_096 * 8 + 4_096 / 8); }
#[test] #[should_panic(expected = "column 3 outside plan of 2")] fn out_of_plan_column_panics() { ChunkBuilder::new(&[BufKind::I64, BufKind::Bool], 1).push_i64(3, 1); }
#[test] #[ignore = "perf"] fn cell_lookup_p50_under_30_ns() { /* 4,096 × 50, release */ }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-values --test decimal --test temporal --test columnar`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies. No `to_lowercase`/`to_uppercase`; no `f64` in `decimal.rs`.
- [ ] **Step 4:** Run `cargo test -p hacivat-values`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(values): exact decimals, temporal values, columnar chunks with validity bitmaps, exceptions, lossless text and UTF-8-safe truncation`.

**Review:** exactness (no `f64`, no rounding); `i128`-free arithmetic and the `Big` fallback; truncation at a character boundary; alignment and `size_of::<Value>()`; every builder path bounded.

### Task 7: `hacivat-results` store, snapshots, writer, window, budget (HV-RES-02, -03, -05 types, -07 v1) (W1)

**Files:**
- Modify: `crates/core/hacivat-results/src/{store,snapshot,writer,window,budget,cursor,fixture}.rs`.
- Create: `crates/core/hacivat-results/tests/{store,window,budget,perf_store}.rs`.

**Interfaces:**
- Consumes: Task 1b's types and its working `Chunk::from_values` (typed chunks once Task 6 merges).
- Produces:
  - `ResultStore`/`ResultWriter`/`StoreSnapshot`: `ArcSwap<StoreSnapshot>` published on **every** append and state change (each publish clones `Arc<[Arc<Chunk>]>` and `chunk_starts: Arc<[u64]>`, O(chunks) ≤ 250 for 1M rows); consumers coalesce through the `watch` (one `cx.notify()` per frame). This replaces `alg/21`'s 16 ms writer throttle: simpler, and no chunk waits unpublished (R-M3-48). `generation` bumps when a re-run replaces the rows; `version` on every publish.
  - `cell(row, col)`: `partition_point` over `chunk_starts`, then the chunk.
  - `MemoryBudget`: `from_ram(Some(ram))` = min(25 % of RAM, 4 GiB), `from_ram(None)` = 1 GiB; per result 256 MiB; `admit` answers `Pause(ResultCap)` past the per-result cap and `Pause(GlobalBudget)` past the global one; `ResultStore::raise_cap_to_global` (an atomic the writer reads) lifts the per-result cap. No eviction (R-M3-33).
  - `StoreSource` (`ResultSource`): `ensure_rows(upto)` sends **one** `fetch_more` while the result is `Suspended` and `upto + prefetch_margin_rows > row_count`, then sends nothing until the snapshot version moves (coalescing without a timer); never while `Streaming`, `Complete`, `Closed` or `Cancelled`. `fetch_all` and `close_cursor` forward to `FetchControl`; `raise_cap` ("Fetch more anyway") is `store.raise_cap_to_global()` followed by `control.fetch_all(result)`.
  - `ResultStore::from_rows` (feature `test-support`): a complete, immutable store for shots and UI tests with no core runtime (R-M3-15).

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn lookups_at_every_chunk_boundary() { let s = store_with_chunks(&[500, 4_096, 1_308]); for r in [0, 499, 500, 4_595, 4_596, 5_903] { assert_eq!(int_at(&s, r, 0), r as i64); } }
#[test] fn every_append_publishes() { let (s, mut w) = new_store(); w.append(chunk(500)); assert_eq!(s.snapshot().row_count(), 500); w.append(chunk(10)); assert_eq!(s.snapshot().row_count(), 510); }
#[test] fn generation_and_version_are_separate() { /* re-run: generation 1 → 2, version restarts its count; append bumps version only */ }
#[test] fn an_old_snapshot_never_sees_new_rows() { let (s, mut w) = new_store(); w.append(chunk(10)); let old = s.snapshot(); w.append(chunk(10)); assert_eq!(old.row_count(), 10); }
#[test] fn per_result_cap_pauses_then_raise_lifts_it() { let (s, w) = store_with_budget(MemoryBudget::new(1 << 30, 1 << 20)); fill(&w, 1 << 20);
    assert_eq!(w.admit(4_096), Admission::Pause(PauseReason::ResultCap)); s.raise_cap_to_global(); assert_eq!(w.admit(4_096), Admission::Ok); }
#[test] fn global_budget_pauses_across_results() { /* two stores share 1 MiB global */ }
#[test] fn from_ram_takes_a_quarter_capped_at_4_gib() { assert_eq!(global_of(MemoryBudget::from_ram(Some(8 << 30))), 2 << 30); assert_eq!(global_of(MemoryBudget::from_ram(Some(64 << 30))), 4 << 30);
    assert_eq!(global_of(MemoryBudget::from_ram(None)), 1 << 30); }
#[test] fn ensure_rows_sends_one_fetch_per_version_while_suspended() { let (src, ctl) = suspended_source(10_000); src.ensure_rows(9_500); src.ensure_rows(9_900);
    assert_eq!(ctl.fetches(), 1); publish_more(&src, 4_096); src.ensure_rows(13_900); assert_eq!(ctl.fetches(), 2); }
#[test] fn ensure_rows_is_silent_unless_suspended() { for c in [Streaming, Complete, closed(UserClosed), Cancelled] { let (src, ctl) = source_in(c); src.ensure_rows(u64::MAX); assert_eq!(ctl.fetches(), 0); } }
#[test] fn from_rows_fixture_is_complete() { let s = ResultStore::from_rows(schema2(), rows2(3), Completeness::Complete); assert_eq!(s.snapshot().row_count(), 3); }
#[test] #[ignore = "perf"] fn ten_million_rows_cell_p50_under_30_ns() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-results --features test-support`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies. Nothing on the read path locks; the writer is single-owner.
- [ ] **Step 4:** Run `cargo test -p hacivat-results --features test-support`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(results): columnar result store with ArcSwap snapshots, budgets, coalesced fetch-more source and test fixtures`.

**Review:** nothing locks on the read path; publication order (`chunk_starts` and chunks consistent in one snapshot); accounting across concurrent results; a held snapshot keeps its chunks alive.

### Task 8: `sql-split`: pre-pass, splitter, recovery, `SqlDocument`, cursor eligibility (SQL-SPLIT-01..03; R-M3-5, R-M3-23) (W2)

**Files:**
- Modify: `crates/sql/sql-split/src/{client,splitter,recover,item,key,document,head}.rs`; `crates/sql/sql-dialect/src/split_profile/**`.
- Create: `crates/sql/sql-dialect/data/split/{pg,mysql,mariadb,sqlite}.toml` (one fixture row per table row of `alg/10` SPLIT-02); `crates/sql/sql-split/tests/{split,document,head,split_props,sqlite_differential}.rs`; `crates/sql/sql-testkit/corpus/fixtures/FX-SPLIT-*.sql` (verbatim from `datagrip/13` §3.3, LF; CRLF variants generated at test time).
- Modify (named exception, docs only): the SPLIT-02 section of `docs/research/algorithms/10-sql-lexing-splitting-params.md`, adding an "Errata (M3a, R-M3-5)" paragraph with cases (a)–(d).

**Interfaces:**
- Consumes: Tasks 3 and 4.
- Produces: `split`, `SqlDocument` (all methods; `apply` = full rebuild in M3a), `DocSnapshot` (`execution_split` computed once from the same `Lexed` through a `OnceLock`), `StmtKey` (XXH3-128 of dialect id, modes, delimiter and statement text), `head::cursor_eligible`.
  - **Pre-pass:** psql `\cmd`, `\;`, `COPY … FROM STDIN` data up to `\.`; mysql `DELIMITER` (a character-level scan with MySQL quote and comment rules; the delimiter matches as raw text even inside a word, `END$$`; `DELIMITER $$;` takes the whole run and adds the diagnostic `delimiter-ends-with-semicolon`), `\G`, `\g`, `\c`, `source`; sqlite3 `.cmd` lines.
  - **Block stack:** PG `BEGIN ATOMIC … END`, `CREATE RULE … DO (…)`; MySQL/MariaDB stored programs (`BEGIN … END`, `IF`, `CASE`, and `LOOP`/`REPEAT`/`WHILE` **only in statement position**, so `REPEAT('a', 3)` is a function), MariaDB top-level `BEGIN NOT ATOMIC`; SQLite `CREATE [TEMP|TEMPORARY] TRIGGER … BEGIN … END` with `CASE … END` inside; PG/SQLite `END [TRANSACTION|WORK]` at statement start with an empty stack is a COMMIT synonym; SQLite top-level `BEGIN DEFERRED|IMMEDIATE|EXCLUSIVE` is a transaction (the R-M3-5 errata).
  - **Modes:** editor mode adds `Heuristic` items from missing-semicolon and blank-line recovery; execution mode never produces a `Heuristic` runnable item — an uncertain boundary becomes an `AmbiguousBoundary`. `;` inside `/*! … */` → `Ambiguous(SemicolonInExecutableComment)`. The **stability rule**: an unterminated quote keeps every later boundary `Heuristic` in editor mode.
  - **`cursor_eligible` (R-M3-23):** the first significant token's keyword is `SELECT`, `VALUES` or `TABLE` (MySQL/MariaDB also `SHOW`), or `WITH` with no `INSERT`/`UPDATE`/`DELETE`/`MERGE` keyword token in the range; and the range has no `INTO` keyword, no locking clause (`FOR` followed by `UPDATE`/`SHARE`/`NO KEY UPDATE`/`KEY SHARE`; MySQL `LOCK IN SHARE MODE`) and no `ExecutableComment` token. Comments before the head are skipped.

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn split_cases_pg_mysql_sqlite() { for c in split_cases() { assert_eq!(strict_texts(&c), c.expect, "{}", c.id); } }   // alg/10 SPLIT-01/-02/-03 + FX-SPLIT
#[test] fn end_at_statement_start_is_commit() { assert_eq!(strict_texts_pg("BEGIN; SELECT 1; END; SELECT 2"), ["BEGIN", "SELECT 1", "END", "SELECT 2"]); }
#[test] fn repeat_function_is_not_a_block() { assert_eq!(strict_texts_mysql("SELECT REPEAT('a', 3); SELECT 2"), ["SELECT REPEAT('a', 3)", "SELECT 2"]); }
#[test] fn mariadb_begin_not_atomic_is_a_block() { assert_eq!(count_mariadb("BEGIN NOT ATOMIC SELECT 1; SELECT 2; END; SELECT 3"), 2); }
#[test] fn sqlite_begin_immediate_is_a_transaction() { assert_eq!(strict_texts_sqlite("BEGIN IMMEDIATE; SELECT 1"), ["BEGIN IMMEDIATE", "SELECT 1"]); }
#[test] fn mysql_delimiter_matches_inside_end() { let s = split_mysql("DELIMITER $$\nCREATE PROCEDURE p() BEGIN SELECT 1; END$$\nDELIMITER ;\nSELECT 2;");
    assert_eq!(statements(&s), ["CREATE PROCEDURE p() BEGIN SELECT 1; END", "SELECT 2"]); }
#[test] fn semicolon_in_executable_comment_is_ambiguous() { let s = split_exec_mysql("SELECT 1 /*! ; DROP TABLE t */"); assert!(matches!(s.ambiguous[0].reason, AmbiguityReason::SemicolonInExecutableComment)); }
#[test] fn unterminated_quote_keeps_later_boundaries_provisional() { let d = SqlDocument::new(format!("SELECT 'x;\n{}", "SELECT 1;\n".repeat(50)), pg18(), ClientFlavor::None);
    assert!(d.statements().iter().skip(1).all(|i| is_heuristic(i))); }
#[test] fn copy_from_stdin_data_is_one_segment() { /* psql: COPY t FROM STDIN; rows; \. → one CopyIn unit, data range */ }
#[test] fn cursor_eligibility_reads_the_head() { for (sql, want) in [("SELECT 1", true), ("  -- c\n select 1", true), ("VALUES (1)", true), ("TABLE t", true),
    ("WITH x AS (SELECT 1) SELECT * FROM x", true), ("WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d", false), ("SELECT * FROM t FOR UPDATE", false),
    ("SELECT 1 INTO t2", false), ("INSERT INTO t VALUES (1) RETURNING *", false), ("CALL p()", false)] { assert_eq!(eligible_pg(sql), want, "{sql}"); }
    assert!(eligible_mysql("SHOW TABLES")); assert!(!eligible_mysql("/*!50000 DELETE FROM t*/ SELECT 1")); }
#[test] fn statement_keys_are_content_addressed() { assert_eq!(key_of("SELECT 1"), key_of("SELECT 1")); assert_ne!(key_of("SELECT 1"), key_of("SELECT 2")); }
proptest! {
    #[test] fn items_never_overlap_and_cover_every_significant_token(s in any_sql()) { let sp = split_all(&s); prop_assert!(disjoint(&sp.items)); prop_assert!(covers_significant(&sp, &s)); }
    #[test] fn execution_mode_never_returns_heuristic_items(s in any_sql()) { prop_assert!(split_exec(&s).items.iter().all(|i| !is_heuristic(i))); }
}
#[test] fn sqlite_splitter_matches_rusqlite_batch() { for script in sqlite_scripts() { assert_eq!(strict_texts_sqlite(&script), rusqlite_batch_texts(&script)); } }
```
- [ ] **Step 2:** Run `cargo test -p sql-split`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies and write the `alg/10` errata paragraph.
- [ ] **Step 4:** Run `cargo test -p sql-split` (and under `LANG=tr_TR.UTF-8`). Expected: PASS.
- [ ] **Step 5: Commit** `feat(sql): client-command pre-pass, block-stack splitter with editor and execution modes, SqlDocument, statement keys, cursor eligibility; alg/10 SPLIT-02 errata`.

**Review:** security-sensitive (a wrong range executes). Probes: the execution-mode refusal path; `DELIMITER` scanning with quotes and comments; `;` inside `/*! */`; COPY data; the block stack per dialect; `cursor_eligible` with a DML keyword hidden in a comment (must not count) or in an executable comment (must count).

### Task 9: `sql-oracle-pg` (W2)

**Files:** Modify `crates/sql/sql-oracle-pg/src/{error,split,stack}.rs`; create `crates/sql/sql-oracle-pg/tests/{oracle,threads,depth,perf}.rs`.

**Interfaces:**
- Consumes: Task 1a's working `version`/`statement_count` and Task 1b's stubs; `sql_text::LineIndex`.
- Produces: `split_with_parser` (statement ranges from libpg_query's split; `$name` placeholders normalised to `$n` before the call so the oracle accepts them), errors mapped to `PgError { message, cursor }` (libpg_query's 1-based character cursor → byte `TextSize` through `LineIndex::char_to_byte`), `on_parser_stack` (a `std::thread::Builder::stack_size(16 << 20)` thread per call, joined; a panic inside becomes `None`). No `pg_query` type in any public signature.

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn split_returns_statement_ranges() { assert_eq!(texts("SELECT 1; SELECT 2", split_with_parser("SELECT 1; SELECT 2").unwrap()), ["SELECT 1", "SELECT 2"]); }
#[test] fn named_params_are_normalised_before_the_oracle() { assert_eq!(split_with_parser("SELECT :a; SELECT 2").unwrap().len(), 2); }
#[test] fn error_cursor_maps_after_multibyte_text() { let e = statement_count("SELECT 'çğ' FRM t").unwrap_err(); assert_eq!(e.cursor, Some(TextSize::from(18))); }   // `FRM` is an alias; the error is at `t` (P18)
#[test] fn eight_threads_parse_in_parallel() { std::thread::scope(|s| for _ in 0..8 { s.spawn(|| for _ in 0..1_000 { statement_count("SELECT 1").unwrap(); }); }); }
#[test] fn ten_thousand_parens_on_a_small_stack_is_an_error() { let sql = format!("SELECT {}1{}", "(".repeat(10_000), ")".repeat(10_000));
    let r = std::thread::Builder::new().stack_size(2 << 20).spawn(move || statement_count(&sql)).unwrap().join().unwrap(); assert!(r.is_err()); }
#[test] fn long_chains_and_unions_parse() { assert!(statement_count(&chain(50)).is_ok()); assert!(statement_count(&unions(100)).is_ok()); }
#[test] fn on_parser_stack_returns_the_result_and_contains_a_panic() { assert_eq!(on_parser_stack(|| 1), Some(1)); assert_eq!(on_parser_stack(|| -> i32 { panic!("boom") }), None); }
#[test] #[ignore = "perf"] fn hundred_thousand_parses_keep_rss_flat() { /* `ps -o rss= -p <pid>` before/after (std::process::Command; no unsafe) */ }
#[test] #[ignore = "perf"] fn one_mib_statement_parses_within_budget() {}
```
- [ ] **Step 2:** Run `cargo test -p sql-oracle-pg`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies.
- [ ] **Step 4:** Run `cargo test -p sql-oracle-pg`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(sql): libpg_query oracle — parser split with placeholder normalisation, byte error cursors, 16 MiB parser stack`.

**Review:** security-sensitive (FFI). Probes: no panic crosses FFI; the cursor mapping; the version string; inputs with NUL bytes (must be an error, not a truncated parse).

### Task 10: `hacivat-text` selections, text units and the IME core (UI-EDIT-05 model, -06, -12 model) (W2, gpui-free)

**Files:** Modify `crates/ui/hacivat-text/src/{selection,units,ime}.rs`; create `crates/ui/hacivat-text/tests/{selection,units,ime_traces}.rs`.

**Interfaces:**
- Consumes: Task 5's model and change sets.
- Produces: `SelectionSet` normalisation and mapping (many ranges held, single-cursor commands use one), `move_offset` (graphemes from `unicode-segmentation`, CRLF one unit; words from the `TokenOracle`, else character classes `Ident` = XID_Continue + `$ # @`, `Space`, `Punct`, `LineBreak`; sub-words by ASCII case transitions only; smart Home), and `ImeCore` (marked text is real buffer text inside an IME bracket; UTF-16 ranges converted through the buffer; the app-switch blur leaves the composition with the platform, an in-window focus move commits locally; a programmatic edit during a composition commits it first).

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn ui_edit_05_cases_1_2_4_6() { /* normalisation, merge on overlap, goal column with x from the test, main kept */ }
#[test] fn ui_edit_06_cases_1_to_6() { /* graphemes, token words with a fake oracle, class fallback, smart Home */ }
#[test] fn dotted_capital_i_with_combining_dot_is_one_grapheme() { let b = TextBuffer::from_str("I\u{307}x"); assert_eq!(move_offset(&b, 0, Dir::Forward, Unit::Grapheme, &NoTokens), 3); }
#[test] fn dotless_i_is_an_ordinary_letter() { assert_eq!(word_at("sık_isim", 1), 0..9); }   // bytes: `ı` is two (P18)
#[test] fn quoted_identifier_is_one_token_word() { assert_eq!(token_word(r#"SELECT "Müşteri Adı" FROM t"#, 9), 7..23); }   // bytes, quotes included (P18)
#[test] fn spike_ime_traces_on_byte_offsets() { for t in spike_traces() { run_trace(&t); } }      // the 13 traces of spikes/spike-editor/src/input.rs:362-496
#[test] fn marked_text_across_a_surrogate_pair_reports_the_adjusted_range() { let (mut ime, m) = composing("a😀b"); let mut adj = None;
    ime.text_for_range(&m.snapshot().buffer, 2..3, &mut adj); assert_eq!(adj, Some(1..3)); }
#[test] fn an_empty_mark_cancels_the_composition() {}
#[test] fn a_mark_replacing_a_selection_deletes_it_once() {}
#[test] fn programmatic_edit_during_composition_commits_first() {}
#[test] fn app_switch_blur_keeps_the_composition_and_focus_move_commits() { assert!(!blur(false)); assert!(blur(true)); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-text --test selection --test units --test ime_traces`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies. Throw away the spike's `is_identifier_char` (`char::is_alphanumeric`).
- [ ] **Step 4:** Run `cargo test -p hacivat-text`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(text): selection sets, grapheme/token/class text units, window-free IME core with byte-offset traces`.

**Review:** UTF-16 arithmetic; never splitting a surrogate pair or a CRLF; the bracket lifecycle; ASCII-only sub-words.

### Task 11: `hacivat-text` display model (UI-EDIT-07 pure parts, -08 uniform) (W2, gpui-free)

**Files:** Modify `crates/ui/hacivat-text/src/display/{rangeset,compose,height,scroll}.rs`; create `crates/ui/hacivat-text/tests/display.rs`.

**Interfaces:**
- Consumes: Task 1b's display types; Task 5's buffer.
- Produces: `RangeSet` (sorted; O(log n + k) query; a counter in tests), `compose_line` (tab expansion by `unicode-width`, wide characters 2, zero-width 0; `Control` fragments for C0/C1, U+FEFF and every bidi control U+202A–E, U+2066–9, U+200E/F, U+061C, always visible), `HeightMap::Uniform` (y = line × line height in `f64`), `ScrollAnchor` (row-anchored; edits above the anchor keep the visible content still; the absolute `f64` y is for the scrollbar only).

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn rlo_is_a_visible_control_fragment() { assert!(matches!(frag(&line("a\u{202E}b"))[1], Fragment::Control { glyph: _, .. })); }
#[test] fn nul_is_a_visible_control_fragment() { assert!(matches!(frag(&line("a\0b"))[1], Fragment::Control { .. })); }
#[test] fn tabs_expand_to_the_next_stop_with_wide_chars() { assert_eq!(columns("日\tx", 4), [2, 2, 1]); }
#[test] fn ui_edit_08_cases_1_2_4_5_without_wrap() {}
#[test] fn anchor_survives_a_thousand_lines_inserted_above() { let mut s = anchored_at(40_000); insert_lines_above(&mut s, 1_000); assert_eq!(first_visible_text(&s), line_text(40_000)); }
#[test] fn million_lines_have_exact_y() { assert_eq!(HeightMap::uniform(1_000_000, 18.0).y_of(999_999), 17_999_982.0_f64); }
#[test] fn rangeset_query_is_logarithmic() { let rs = rangeset_of(100_000); assert!(probes(|| rs.query(50_000..50_010).count()) <= 40); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-text --test display`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies; no work over the whole document.
- [ ] **Step 4:** Run `cargo test -p hacivat-text`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(text): decoration range sets, line composition with visible controls, uniform height map, row-anchored scrolling`.

**Review:** `f64` for absolute pixels, `f32` only viewport-relative; controls always visible ("Trojan Source"); no O(document).

### Task 12: PostgreSQL typed streaming, with the `tokio-postgres` patch §1 (HV-RES-04 PG, HV-RES-01 PG mapping; R-M3-18, R-M3-31) (W2)

**Files:**
- Modify (named exception): `third_party/tokio-postgres/src/{client.rs,query.rs,bind.rs}` and `third_party/tokio-postgres/PATCHES.md` (§1, with an upstream PR text), each change fenced by `// HACIVAT PATCH §1`.
- Modify: `crates/core/hacivat-driver-pg/src/{typed.rs, decode/{mod,scalar,numeric,temporal}.rs}`; the typed `RawConn` methods in `src/probe.rs` (named lines).
- Create: `crates/core/hacivat-driver-pg/tests/{tp_patches,decode,docker_typed}.rs`.
- Append: ADR 0002 `## m3a-t12 — PostgreSQL typed`.

**Interfaces:**
- Consumes: Task 1b's typed surface (with P5's contract) and Task 6's builders.
- Produces:
  - **Patch §1 (three entry points, P5):** `Client::bind_with_formats(&self, statement: &Statement, params: I, result_formats: &[i16]) -> Result<Portal, Error>` and `Client::query_portal_raw(&self, portal: &Portal, max_rows: i32) -> Result<RowStream, Error>`, outside `Transaction`, with named portals from the existing counter (`bind.rs:24`), used only for suspendable units inside a transaction block (a Sync outside one ends the implicit transaction and every portal with it); and `Client::query_with_formats(&self, statement: &Statement, params: I, result_formats: &[i16]) -> Result<RowStream, Error>`, the unnamed portal with Bind, Execute(0) and Sync in one batch, used for every non-suspendable unit. Existing calls stay byte-identical.
  - **Typed path:** `typed_caps() = { suspend: true, binds: false, close_needs_kill: false, idle_bound: None }`. `query(spec)`: prepare (type info through `tokio-postgres`'s cache; no `DEALLOCATE ALL` anywhere), result formats per column (**binary** for the R-M3-31 scalar set and enums; **text** for everything else, decoded as `Opaque`), bind into a named portal, `Execute(first_rows)` then `Execute(chunk_rows)` on each `fetch_more`. `PortalSuspended` → `QueryEvent::Suspended` (`rows_affected()` stays `None`); `CommandComplete` → `ResultEnd`. `suspendable == false` uses `query_with_formats` (the unnamed portal and `Execute(0)` in one batch) with TCP backpressure; rows beyond `cell`/byte caps are counted. `suspendable == true` while `tx_status()` is `Idle` answers `Unsupported` before sending anything (the driver contract, P5); the session or the guard opens the transaction first. `close_query(Now)` closes the portal; `Drain(d)` runs one `Execute(4_096)` within `d`. Every Bind, Execute and Close of a named portal carries one `Sync` inside the transaction block, so the portal survives it; `synced()` holds after every path. `COPY` is refused before sending with `RawErrorKind::Unsupported` (Task 23 logs it as `exec-copy-unsupported`). `Protocol::Simple` answers `Unsupported` (M3b).
  - **Decoders** (byte fixtures, never a panic): bool, int2/4/8, oid/xid/cid, float4/8 (shortest form; NaN, ±Inf, −0.0 kept), numeric (base-10000; NaN/±Inf → `Special`; > 38 digits → `Big`; ndigits > 1,000 → cell `Error`), text family (invalid UTF-8 → `Lossy`), bytea, uuid, json, jsonb (version byte 1, else `Error`), date/time/timetz/timestamp/timestamptz (±infinity → `Special`), interval, enum labels.
  - `ColumnMeta.origin = OriginHint::Pg { table_oid, attnum }` from the statement's columns.

- [ ] **Step 1: Write the failing tests.**
```rust
// tests/tp_patches.rs (the scripted in-memory server of tests/wire.rs)
#[tokio::test] async fn bind_with_formats_sends_per_column_codes() { let srv = scripted(); bind_with_formats(&srv, &[1, 0]).await; assert_eq!(srv.last_bind().result_formats, [1, 0]); }
#[tokio::test] async fn query_portal_raw_sends_execute_max_rows() { assert_eq!(execute_of(2).await, 2); }
#[tokio::test] async fn suspended_portal_leaves_rows_affected_none() {}
#[tokio::test] async fn existing_query_path_is_byte_identical() { assert_eq!(wire_of_query_raw(PATCHED).await, wire_of_query_raw(GOLDEN_0_7_18).await); }
#[tokio::test] async fn autocommit_non_suspendable_unit_runs_in_one_batch() { assert_eq!(messages_of_unit(false).await, [Bind, Execute(0), Sync]); }   // P5
#[tokio::test] async fn suspendable_outside_a_transaction_is_refused() { let (srv, r) = typed_query_while_idle(true).await;
    assert_eq!(r.unwrap_err().kind, RawErrorKind::Unsupported); assert!(srv.received().is_empty()); }   // P5: nothing sent
// tests/decode.rs
#[test] fn every_scalar_decoder_against_fixtures() { for f in pg_fixtures() { assert_eq!(decode(&f), f.expect, "{}", f.name); } }
#[test] fn invalid_utf8_text_is_lossy_never_replaced() { assert!(matches!(decode_text(&[0x61, 0xc3]), Decoded::Lossy(_))); }
#[test] fn cell_over_1_mib_is_truncated_at_a_char_boundary() { assert!(matches!(decode_text_capped("ş".repeat(700_000).as_bytes(), 1 << 20), Decoded::Truncated { total: 1_400_000, .. })); }
#[test] fn numeric_with_1001_digits_is_an_error_cell_not_a_panic() { assert!(matches!(decode_numeric(&numeric_ndigits(1_001)), Decoded::Error(_))); }
#[test] fn malformed_lengths_are_error_cells() { for b in hostile_lengths() { assert!(matches!(decode_any(&b), Decoded::Error(_))); } }
#[test] fn unknown_oid_is_requested_as_text_and_opaque() { assert_eq!(format_for(oid_of("tsvector")), 0); assert_eq!(format_for(oid_of("int8")), 1); }
// tests/docker_typed.rs (#[ignore = "docker"]; require_target("pg17") on pr; pg14–18 on full)
#[tokio::test] async fn type_golden_table_hv_types() {}
#[tokio::test] async fn million_rows_suspend_resume_close() { /* BEGIN first (P5); first chunk 501; Suspended after read-ahead; fetch_more; close_query(Now); synced() */ }
#[tokio::test] async fn statement_between_portal_executes_keeps_the_portal() { /* BEGIN; Execute(501) → Suspended; SELECT 1 through execute; fetch_more still returns rows (the canary's premise, P5) */ }
#[tokio::test] async fn portal_inside_a_user_transaction_keeps_in_tx() {}
#[tokio::test] async fn copy_is_refused_before_sending() {}
#[tokio::test] async fn protocol_cancel_during_execute() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-driver-pg --test tp_patches --test decode`. Expected: FAIL.
- [ ] **Step 3: Implement** the patch (smallest diff; `PATCHES.md` lists every touched file and range) and the typed path.
- [ ] **Step 4:** Run `cargo test -p hacivat-driver-pg`, then `<main>/target/docker/run-docker-tests.sh --set pr -p hacivat-driver-pg` (and `--set full` once). Expected: PASS, with the M2b PG tests green.
- [ ] **Step 5: Commit** `feat(driver-pg): typed streaming through named portals with per-column result formats (vendored tokio-postgres patch §1), exact decoders, lossless text`.

**Review:** security-sensitive (untrusted server bytes). Hostile-server probes: a `DataRow` with the wrong column count (→ `Protocol`, connection broken); negative or huge lengths; a numeric with ndigits above 1,000; a jsonb version byte 2; the patch's diff size and no change to existing calls.

### Task 13: `exec_range`, chooser candidates and the D-011 goldens (SQL-SPLIT-04 aligned to D-011; R-M3-8) (W3)

**Files:**
- Modify: `crates/sql/sql-split/src/exec_range/{mod,candidates,policy}.rs`; `crates/sql/sql-testkit/src/golden/{mod,markers,runner}.rs`.
- Create: `crates/sql/sql-testkit/corpus/datagrip/{caret,sel,choose}.toml`, `corpus/datagrip/ids/{caret,sel,choose}.txt`, `corpus/datagrip/divergences.toml`, `corpus/fixtures/FX-CARET-*.sql`; `crates/sql/sql-split/tests/{d011_goldens,exec_range,exec_range_props,pg_differential}.rs`.

**Interfaces:**
- Consumes: Task 8's `DocSnapshot` and splits; Task 9's `split_with_parser` (dev-dependency, for the PG differential).
- Produces: `resolve` and the policies.
  - **Candidates** are tried in this order: subquery (needs the `SubqueryOracle`, always `None` in M3a) → statement → largest-or-batch → from-caret → whole script. Duplicate ranges keep the earliest kind, **except that a `FromCaret` equal to `WholeScript` is `WholeScript`** (so `FromCaret` appears only as a strict subset). One candidate → `Units` (the D-011 shortcut). `remembered` sets `preselect`.
  - **Inside / outside / selection:** a caret inside a statement follows `InsidePolicy`; a caret outside every statement follows `OutsidePolicy` (D-011: `Nothing` → `NothingToRun`); a selection follows `SelectionPolicy` (D-011: `AsSeparate`, splitting the selected text with the lexer state at the selection start, so a selection that starts inside a dollar quote or after an active `DELIMITER //` splits correctly). A whitespace-only selection is `NothingToRun`. Caret-at-boundary rules follow `alg/10` SQL-SPLIT-04.
  - **Sent text = the strict range:** no terminator (MySQL rejects it over the protocol) and no leading comments (kept as `title_comment`).
  - **Modes:** `SplitMode::Execution` never returns a unit that crosses a `Heuristic` boundary; an uncertain boundary returns `Ambiguous`. `SplitMode::Editor` returns the same ranges for every `Exact` item (R-M3-8).
  - **`SmallestSubqueryOrStatement`** falls back to the statement in M3a; goldens that need a CST run their `expect_without_cst` form.
  - **Goldens (clean room, ALG-TEST-01):** fixtures are the Hacivat-authored SQL of `datagrip/13` §3.3, copied verbatim with their `-- FX-…` line; expectations come from `alg/10` SPLIT-04 aligned to D-011 and `alg/20` HV-EXEC-01/-02, never from DataGrip's "Expected hint" column. The ID-coverage meta-test requires every ID in `ids/*.txt` to be present or explicitly skipped with a reason, and asserts the skip list exactly: CARET-19, -20 (`dialect:mssql`), CARET-22 (`dialect:oracle`); SEL-06 (`dialect:mssql`); CHOOSE-04 (`dialect:mssql`), CHOOSE-05, -06, -07 (`app-ui`: Task 33 covers hover and the remembered kind; CHOOSE-06 is M3b). The divergence register records CHOOSE-01's missing subquery candidate (correlated; `alg/10` :552).

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn caret_sel_choose_goldens() { for case in golden::load(corpus("datagrip/caret.toml")).into_iter().chain(sel()).chain(choose()) { golden::run(&case).unwrap(); } }
#[test] fn every_corpus_id_is_present_or_skipped_with_a_reason() { assert_eq!(skips(), EXPECTED_SKIPS); assert!(missing_ids().is_empty()); }
#[test] fn choose_01_offers_statement_and_whole_script() { let r = resolve_at(FX_CARET_01, "SELECT id FROM recent r"); assert_eq!(kinds(&r), [CandidateKind::Statement, CandidateKind::WholeScript]); }
#[test] fn one_candidate_runs_directly() { assert!(matches!(resolve_at("SELECT 1", "SELECT"), RangeResolution::Units(u) if u.len() == 1)); }
#[test] fn from_caret_equal_to_the_whole_script_is_labelled_whole_script() {}
#[test] fn caret_outside_statements_runs_nothing() { assert!(matches!(resolve_caret("SELECT 1;\n\n\nSELECT 2;", 11), RangeResolution::NothingToRun)); }
#[test] fn selection_runs_as_separate_statements() { assert_eq!(unit_texts(resolve_sel("SELECT 1; SELECT 2;", 0..19)), ["SELECT 1", "SELECT 2"]); }
#[test] fn selection_starting_inside_a_dollar_quote_carries_the_state() {}
#[test] fn sent_text_has_no_terminator_and_no_leading_comment() { let u = only_unit(resolve_at("-- title\nSELECT 1;", "SELECT")); assert_eq!(text(&u), "SELECT 1"); assert!(u.title_comment.is_some()); }
#[test] fn caret_in_dollar_body_runs_the_whole_create_function() { let sql = "CREATE FUNCTION f() RETURNS int AS $$ SELECT 1; SELECT 2; $$ LANGUAGE sql;";
    assert_eq!(unit_texts(resolve_at(sql, "SELECT 2")), [&sql[..sql.len() - 1]]); }
#[test] fn mysql_delimiter_block_is_one_unit() { assert_eq!(unit_texts(resolve_mysql_at(PROC_WITH_DELIMITER, "SELECT 1")), [PROC_BODY]); }
#[test] fn ambiguous_boundary_is_never_a_unit() { assert!(matches!(resolve_mysql_exec("SELECT 1 /*! ; */ ;", 0), RangeResolution::Ambiguous(_))); }
proptest! {
    #[test] fn execution_never_crosses_a_heuristic_boundary(s in any_sql(), c in caret_s()) { prop_assert!(no_unit_crosses_heuristic(&s, c)); }
    #[test] fn editor_and_execution_agree_on_exact_items(s in any_sql(), c in caret_s()) { prop_assert!(agree_on_exact(&s, c)); }   // R-M3-8
}
#[test] fn pg_execution_split_matches_the_parser_on_fixtures() { for f in pg_fixtures() { assert_eq!(exec_ranges(&f), split_with_parser(&f).unwrap()); } }
```
- [ ] **Step 2:** Run `cargo test -p sql-split --test d011_goldens --test exec_range --test exec_range_props --test pg_differential`. Expected: FAIL.
- [ ] **Step 3: Implement** `resolve`, the candidate builder and labels (whitespace collapsed, controls → space; the UI truncates), the golden loader (`┃` carets and `«…»` selections; `caret = { after = "…", occurrence = n }` anchors), and the corpus files.
- [ ] **Step 4:** Run `cargo test -p sql-split -p sql-testkit`. Expected: PASS. Record the run and skip counts per family in ADR 0005.
- [ ] **Step 5: Commit** `feat(sql): caret and selection to execution units with D-011 defaults, chooser candidates, CARET/SEL/CHOOSE goldens and the PG parser differential`.

**Review:** security-sensitive (what runs). Probes: D-011 defaults; the dedupe rule; strict-range sending; caret-at-boundary rules; the meta-test cannot pass with a silently dropped ID; no expectation copied from DataGrip.

### Task 14: MySQL/MariaDB typed streaming (HV-RES-04 MySQL; R-M3-31) (W3)

**Files:**
- Modify: `crates/core/hacivat-driver-mysql/src/{typed.rs,decode.rs}`; the typed `RawConn` methods and the `SELECT @@net_write_timeout` read in `src/connect.rs` (named lines).
- Create: `crates/core/hacivat-driver-mysql/tests/{decode,docker_typed}.rs`.
- Append: ADR 0002 `## m3a-t14 — MySQL typed`.

**Interfaces:**
- Consumes: Task 1b's typed surface, Task 6's builders, the vendored `mysql_async`.
- Produces:
  - `typed_caps() = { suspend: true, binds: false, close_needs_kill: true, idle_bound: Some(net_write_timeout − 15 s) }` (45 s at the default 60 s; never raises the server value).
  - `query(spec)`: the binary protocol (`exec_iter`) with the M2b routing kept (`SET` over `COM_QUERY`, `exec.rs:208-240`); on a read-write session a statement the server cannot prepare (1295) is retried once over `COM_QUERY` with text decoding; on read-only, 1295 stays `Unsupported` (guard layer). **Suspension without a self-referential struct:** the pending result lives in `Conn` (`third_party/mysql_async/src/conn/mod.rs:89-94, 350-376`); the driver drops its `QueryResult` at a chunk boundary and resumes with `QueryResult::new(&mut conn)` and the same protocol type parameter. Result sets after the first are drained and counted (`QueryDone.skipped_result_sets`, R-M3-25); a multi-result unit never suspends.
  - `close_query(Drain(d))` drains for at most `d`, else answers `Closed::NeedsServerKill { thread_id }`; `close_query(AfterKill(d))` reads up to error 1317 and leaves the connection usable.
  - Decoders for both protocols: integers (± UNSIGNED; `TINYINT(1)` stays an integer, D-065), `YEAR`, `DECIMAL` (text → `DecimalVal`), `FLOAT`/`DOUBLE`, `CHAR`/`VARCHAR`/`TEXT` (charset ≠ 63; invalid UTF-8 → `Lossy`), `BINARY`/`VARBINARY`/`BLOB` (charset 63), `JSON`, `DATE`, `DATETIME(n)`, `TIMESTAMP(n)` (naive; the session `time_zone` recorded in `NativeType`), `TIME(n)` within 0–24 h, else an `Interval` exception; zero and partial-zero dates → `Special` (never NULL); `ENUM`/`SET` → text; `BIT`, `GEOMETRY` and anything unknown → `Opaque` text. `OriginHint::MySql { schema, table, column }` from `org_table`/`org_name`.

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn mapping_table_over_synthetic_columns() { for c in mysql_columns() { assert_eq!(logical_of(&c.column), c.expect, "{}", c.name); } }
#[test] fn mysql_zero_date_is_special_not_null() { assert!(matches!(decode_date_text("0000-00-00"), Decoded::Special(SpecialVal::ZeroDate))); }
#[test] fn time_over_24_hours_is_an_interval_exception() { assert!(matches!(decode_time_bin(false, 1, 2, 0, 0, 0), Decoded::Exception(Value::Interval { .. }))); }
#[test] fn decimal_65_30_is_big() { assert!(matches!(decode_decimal_text(&format!("{}.{}", "9".repeat(35), "1".repeat(30))), Decoded::Decimal(d) if d.is_big())); }
#[test] fn tinyint_1_stays_an_integer() { assert_eq!(logical_of(&tinyint1()), LogicalType::Int { bits: 8, signed: true }); }
// tests/docker_typed.rs (#[ignore = "docker"]; mysql84 on pr; mysql80 and mariadb106/1011/114 on full)
#[tokio::test] async fn type_golden_table() {}
#[tokio::test] async fn hundred_thousand_rows_suspend_and_resume() {}
#[tokio::test] async fn external_kill_then_drain_leaves_the_connection_usable_within_1s() {}
#[tokio::test] async fn call_with_three_sets_keeps_the_first_and_counts_two() { assert_eq!(done_of(call3()).await.skipped_result_sets, 2); }
#[tokio::test] async fn non_preparable_statement_retries_over_com_query_on_read_write() {}
#[tokio::test] async fn read_only_1295_stays_unsupported() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-driver-mysql --test decode`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies. No driver future is dropped in flight; a `KILL` names only this connection's thread id.
- [ ] **Step 4:** Run `cargo test -p hacivat-driver-mysql`, `<main>/target/docker/run-docker-tests.sh --set pr -p hacivat-driver-mysql` and `--set full` once. Expected: PASS, M2b MySQL tests green.
- [ ] **Step 5: Commit** `feat(driver-mysql): typed streaming with suspend and resume over the pending result, drain-or-kill close, exact decoders, zero dates as specials`.

**Review:** security-sensitive (untrusted server bytes). Probes: the protocol type parameter on resume; a future dropped in flight; `KILL` scope; `max_allowed_packet` errors; a column count that disagrees with the row (→ `Protocol`).

### Task 15: SQLite typed streaming (HV-RES-04 SQLite) (W3)

**Files:**
- Modify: `crates/core/hacivat-driver-sqlite/src/{typed.rs,decode.rs}`; the typed job and methods in `src/open.rs`; the column-metadata `catch_unwind` in `src/exec.rs` (named lines).
- Create: `crates/core/hacivat-driver-sqlite/tests/{typed,typed_hostile}.rs`.
- Append: ADR 0002 `## m3a-t15 — SQLite typed`.

**Interfaces:**
- Consumes: Task 1b's typed surface and the `column_decltype` feature; Task 6's builders.
- Produces:
  - A typed query is one job on the connection's dedicated thread (`open.rs:143-200`): prepare (the read-only authorizer judges here), step rows into a `ChunkBuilder`, send each chunk on a bounded channel of depth 4, and at the read-ahead point block on a command channel (`FetchMore(n)` / `Close`). `Statement<'conn>` lives on the job's stack. A closed command channel ends the job; the cancel latch covers the sub-loop.
  - `typed_caps() = { suspend: true, binds: false, close_needs_kill: false, idle_bound: Some(30 s) }` in rollback-journal mode (from `ProbeWarning::SqliteJournal`), `None` in WAL.
  - Affinity mapping (`INT` → I64; `CHAR`/`CLOB`/`TEXT` → Var; `REAL`/`FLOA`/`DOUB` → F64; `BLOB`/none → decided by the first non-NULL value of the first chunk; `BOOL` → Bool while 0/1; `DATE`/`DATETIME`/`TIMESTAMP` → text verbatim; else the first value's class). A value whose storage class differs from its column's buffer is a full-`Value` exception.
  - Column names and declared types are read inside `catch_unwind` (rusqlite `expect`s on invalid UTF-8, `column.rs:181-212`); when only the declared type is invalid, the valid name is kept and the column is typed `Opaque`; only an invalid name becomes `?` (P28). M2b's `column_names()` (`exec.rs:217`) gets the same guard.
  - The execution busy timeout is 5 s after open; `SQLITE_BUSY` is a classified error with the "locked by another process" hint; no retry.

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn million_row_temp_file_first_chunk_then_suspend_resume_finalize() { /* first chunk ≤ 20 ms (ignored perf assert separate); memory bounded by channel depth */ }
#[test] fn storage_class_mixing_becomes_exceptions() { assert!(matches!(cell_of("CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1), ('abc')", 1, 0), ValueRef::Text("abc"))); }
#[test] fn affinity_table() { for (decl, want) in affinity_cases() { assert_eq!(buf_kind_of(decl), want, "{decl}"); } }
#[test] fn read_only_authorizer_refuses_attach_on_the_typed_path() {}
#[test] fn interrupt_during_a_long_step_and_between_steps() {}
#[test] fn non_utf8_declared_type_does_not_panic() { let f = hostile_file_with_decltype(&[0xff, 0xfe]); let s = schema_of(&f, "SELECT * FROM t");
    assert_eq!(&*s.columns[0].name, "a"); assert!(matches!(s.columns[0].logical, LogicalType::Opaque { .. })); }   // the valid name is kept (P28)
#[test] fn non_utf8_column_name_becomes_a_question_mark() { let s = schema_of(&hostile_file_with_column_name(&[0xff]), "SELECT * FROM t"); assert_eq!(&*s.columns[0].name, "?"); }
#[test] fn busy_after_five_seconds_has_the_hint() { /* a second connection holds RESERVED; the error class and hint id */ }
#[test] fn closed_command_channel_ends_the_job() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-driver-sqlite --test typed --test typed_hostile`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies.
- [ ] **Step 4:** Run `cargo test -p hacivat-driver-sqlite`. Expected: PASS, M2b SQLite tests green.
- [ ] **Step 5: Commit** `feat(driver-sqlite): typed streaming in a stepping sub-loop with bounded chunks, affinity mapping, panic-proof metadata, 5 s busy timeout`.

**Review:** security-sensitive (untrusted files). Probes: the thread never blocks forever; no `rusqlite` API that `expect`s on file text; the authorizer still runs on the typed path.

### Task 16: `valfmt`: display formatting, raw text, ID heuristic (HV-GRID-10; L3; I18N-11) (W3)

**Files:** Modify `crates/core/hacivat-values/src/valfmt/{mod,display,raw,idlike}.rs`; create `crates/core/hacivat-values/tests/{valfmt_display,valfmt_raw}.rs`.

**Interfaces:**
- Consumes: Task 6's values.
- Produces:
  - `format_cell` (no `hacivat-l10n`: ICU4X `icu_decimal`/`fixed_decimal` with a formatter memoised per `(format_locale, settings.version)`): numbers exact (decimals never through `f64`), grouped by the format locale unless `id_like` or `DisplayMode::Raw`; floats in shortest round-trip form with `NaN`/`Infinity` as the server spells them; booleans `true`/`false` (never translated); dates, times and timestamps in ISO form at server precision, `timestamptz` in the display zone with its offset (`+03`); specials verbatim; uuid canonical lower-case; bytes by `BinaryDisplay::Detect` (16 bytes → UUID text; ≥ 90 % printable UTF-8 → text; else `\x…` hex); JSON verbatim (key order and number spelling kept); `Opaque` server text; `Truncated` → the formatted prefix plus a chip `ChipKind::Truncated { total, unit }` (the app renders the chip through Fluent); `Error` → kind `Error` with the message; `Null` → kind `Null` (the grid paints `NULL_MARKER`); `Text("")` → kind `Empty`. Text is capped at `max_display_chars` graphemes; control characters are kept (the grid element visualises them, copy keeps them).
  - `raw_text`: the locale-free canonical form (I18N-11): `1.50`, `2026-11-13 10:00:00.123456+03`, `\x00ff`, `NaN`, `Infinity`; `Null` → `RawText::Null`.
  - `id_like`: an integer column named `id` or ending in `_id` (ASCII match, no locale casing), or a PG `oid`/`reg*` column.
  - `Display.chip` is `Option<ChipKind>` (Task 1b), never a string: a number joined to a unit must be a Fluent message, rendered by the app.
  - `DisplaySettings::new(region_bcp47, tz_iana)` (Task 1b's signature): the region tag through `icu_locale`, the zone through `jiff`, falling back to `und` and UTC, so the app builds settings without either crate (P6).

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn decimal_tr_and_en_and_raw() { let v = dec("1234567.50"); assert_eq!(fmt(&v, "tr-TR"), "1.234.567,50"); assert_eq!(fmt(&v, "en-US"), "1,234,567.50"); assert_eq!(raw(&v), "1234567.50"); }
#[test] fn id_column_is_never_grouped() { assert_eq!(fmt_col(&Value::Int(1_234_567), "customer_id", "tr-TR"), "1234567"); }
#[test] fn shortest_f32_and_negative_zero() { assert_eq!(fmt(&Value::F32(0.1), "en-US"), "0.1"); assert_eq!(fmt(&Value::F64(-0.0), "en-US"), "-0"); }
#[test] fn timestamptz_in_istanbul() { assert_eq!(fmt_tz(&ts_utc("2026-11-11T07:00:00Z"), "Europe/Istanbul"), "2026-11-11 10:00:00+03"); }
#[test] fn sixteen_byte_bytea_is_a_uuid() { assert_eq!(fmt(&Value::Bytes(uuid_bytes()), "en-US"), "6f1e9a0c-0000-4000-8000-000000000001"); }
#[test] fn infinity_date_is_verbatim() { assert_eq!(fmt(&Value::Special(SpecialVal::PosInfinity), "tr-TR"), "infinity"); }
#[test] fn json_keeps_number_spelling_and_key_order() { assert_eq!(fmt(&Value::Json("{\"b\":1.50,\"a\":1e3}".into()), "en-US"), "{\"b\":1.50,\"a\":1e3}"); }
#[test] fn text_null_string_is_not_the_null_marker() { assert_eq!(kind(&Value::Text("NULL".into())), DisplayKind::Text); assert_eq!(kind(&Value::Null), DisplayKind::Null); }
#[test] fn empty_text_is_empty_kind() { assert_eq!(kind(&Value::Text("".into())), DisplayKind::Empty); }
#[test] fn raw_text_is_locale_free_under_turkish_process_locale() { /* run in the turkey-test job: raw of decimals, dates, floats unchanged */ }
#[test] fn truncated_value_carries_a_chip_not_a_string() { assert!(matches!(display_of(&truncated(5 << 20)).chip, Some(ChipKind::Truncated { total, .. }) if total == 5 << 20)); }
#[test] #[ignore = "perf"] fn raw_under_1_us_localized_under_5_us() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-values --test valfmt_display --test valfmt_raw`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies. No `to_lowercase`/`to_uppercase`.
- [ ] **Step 4:** Run `cargo test -p hacivat-values` and the same under `LANG=tr_TR.UTF-8 LC_ALL=tr_TR.UTF-8`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(values): cell display formatting by format locale with raw ID columns, locale-free raw text, ID heuristic`.

**Review:** decimals never through `f64`; ID columns never grouped; dates stay ISO; the chip is data; formatter memoisation keyed correctly (a locale change re-formats).

### Task 17: Grid model (HV-GRID-02 vertical/horizontal, -04 basic, -05 core) (W3, gpui)

**Files:** Modify `crates/ui/hacivat-ui-grid/src/model/{mod,scroll,columns,selection,keys,autosize}.rs`; create `crates/ui/hacivat-ui-grid/tests/{model_scroll,model_columns,model_selection,model_autosize}.rs`.

**Interfaces:**
- Consumes: Task 1b's grid types.
- Produces (no GPUI import in `model/`):
  - `scroll`: the spike's `ScrollPos` math (`spikes/spike-grid/src/scroll.rs:3-92`, its nine tests ported) as `ScrollState` (`top_row: u64` + `f32` offset in `[0, row_h)`; `left_x: f64`), plus `follow_tail` (stick to the bottom when the user was at the bottom while rows stream in), shrink clamping, `scroll_into_view(row)`, `f64` thumb drag with exact extremes, coalesced deltas, and `velocity_rows_per_frame()` for fast-scroll.
  - `columns`: `f64` prefix sums over visible columns in view order; a width change updates only the suffix; hide and show keep `left_x` anchored to the first visible `ColId`.
  - `selection`: anchor/focus, rects in view coordinates, row set (model rows), column set, `all`; membership; `cell_count` in `u128` by inclusion–exclusion; `normalize_for_copy` (a union that is neither stacked nor concatenable → `CopyRefusal::MultipleSelection`; > 1,000,000 cells → `TooMany`); a generation change clears and clamps the focus; an append keeps it.
  - `keys`: `GridCommand` semantics (direction, extend, jump never "next non-empty", last row = last loaded while streaming, Tab wraps rows inside the grid and leaves from the last cell).
  - `autosize`: type default widths (bool 64, int 96, decimal 120, uuid 300, timestamp 200, text 160, json/binary 200), sampled measure (first 50 + last 10 + 20 stratified loaded rows) through a closure, outlier trim at ≥ 2μ (up to 3 passes when n ≥ 20), header width capped at 320, clamps min 48 / initial max 480 / absolute max 2,000, fill for ≤ 8 columns (text-like only).

- [ ] **Step 1: Write the failing tests** (the `alg/22` cases).
```rust
#[test] fn spike_scroll_tests_ported() {}
#[test] fn billion_rows_viewport_math() { let s = view(1_000_000_000, 600.0, 24.0); assert_eq!(s.visible_rows().end, 25); assert_eq!(end_top_row(&s), 999_999_975); }   // row height explicit: 24 px here, 21 px in GridConfig (P18)
#[test] fn thousand_3px_scrolls_move_125_rows() { let mut s = view(10_000, 600.0, 24.0); for _ in 0..1_000 { s.scroll_by(3.0); } assert_eq!(s.top_row, 125); }
#[test] fn visible_columns_from_prefix_sums() { assert_eq!(visible_cols(&[100.0, 200.0, 50.0, 300.0], 250.0, 300.0), 1..4); }
#[test] fn shrink_clamps_top_row() { let mut s = at_row(9_000, 10_000); s.on_row_count(10); assert_eq!(s.top_row, 0); }
#[test] fn outlier_trim() { assert_eq!(trim(&[40.0, 42.0, 45.0, 41.0, 300.0, 44.0]), 60.0); }
#[test] fn select_all_on_a_huge_grid_counts_in_u128() { assert_eq!(all().cell_count(1_000_000_000, 10_000), 10_000_000_000_000); }
#[test] fn copy_of_disjoint_rects_is_refused() { assert_eq!(two_rects().normalize_for_copy(&ident(), &cols(4)), Err(CopyRefusal::MultipleSelection)); }
#[test] fn follow_tail_sticks_only_when_at_the_bottom() {}
#[test] fn generation_change_clears_selection_and_clamps_focus() {}
proptest! { #[test] fn offset_stays_in_row(dys in deltas()) { let mut s = view(1_000_000, 600.0, 21.0); for d in dys { s.scroll_by(d); prop_assert!((0.0..21.0).contains(&s.top_row_offset)); } } }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-ui-grid --test model_scroll --test model_columns --test model_selection --test model_autosize`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies.
- [ ] **Step 4:** Run `cargo test -p hacivat-ui-grid`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(grid): row-anchored two-axis scroll model, f64 column layout, u128 selection model, key semantics, column auto-size`.

**Review:** no absolute pixel math in `f32`; `u64`/`f64`/`u128` where it matters; no GPUI in `model/`.

### Task 18: Session: `Run`, typed streaming into the store, cursors, production read-only (HV-EXEC-05, HV-RES-05; R-M3-16, -20, -23, -29) (W4)

**Files:**
- Modify: `crates/core/hacivat-session/src/session/{run,cursor}.rs`; named lines in `session/actor.rs` (the `Run`/`FetchMore`/`FetchAll`/`CloseCursor` arms, release-before-request, the idle-cursor branch in the idle `select!` near `:940-961`, chunk-step command draining, the liveness skip while suspended), `session/cancel.rs` (the stop-fetching rung), `session/liveness.rs` (skip pings while a cursor is suspended), `session/pool.rs` and `manager.rs` (only if a site beyond Task 1b's seven is found).
- Create: `crates/core/hacivat-session/tests/{run,cursor,prod_read_only,docker_m3a_cursor}.rs`.
- Append: ADR 0004 `## m3a-t18 — runs and cursors`.

**Interfaces:**
- Consumes: Task 1b's session types, FakeDriver typed scripting (it enforces P5's PostgreSQL contract), Task 7's store; the drivers of Tasks 12, 14, 15 for the Docker legs.
- Produces:
  - **`Request::Run`:** units run in order; `ErrorPolicy::StopOnError` (later units → `UnitEnd::Skipped`). For each unit: `query` → events → `RunObserver::open_result` → `ResultWriter::append` per chunk with `admit` before each chunk; `UpdateCount` from `ResultEnd.rows_affected`; `MoreResultSets` when `skipped_result_sets > 0`; `RowsFetched` at most every 50 ms per result. The reply comes after `RunEnded`. Read-only sessions route each unit through `readonly::guarded_query` (Task 22; fail-closed `Internal` until then).
  - **Chunk steps (R-M3-16):** after every chunk the actor drains its command channel with `try_recv`, so a cancel is seen within one chunk even when the driver is always ready (`never_pending`).
  - **Suspension:** only the **last** unit of a run may stay suspended, and only when `RunUnit.suspendable`. Read-ahead stops at `read_ahead_rows` or `read_ahead_bytes`, then the result is `Suspended(CursorInfo)` and the actor keeps the open query. Earlier suspendable units read ahead and are released with `SessionNeeded`. Non-suspendable units stream to completion; rows past the cap are counted, not kept (`Truncated(ResultCap)`).
  - **Hacivat's own transaction around a PostgreSQL cursor (R-M3-23, R-M3-46):** read-write and tx Idle → `BEGIN` before `query` (`TxOwner::Cursor`); tx InTx → `SAVEPOINT hv_cursor`; tx Failed → not suspendable. On completion: `COMMIT` / `RELEASE SAVEPOINT hv_cursor`. On any early release or stop: close the query, then `ROLLBACK` / `ROLLBACK TO SAVEPOINT hv_cursor; RELEASE SAVEPOINT hv_cursor`.
  - **Release triggers:** any other request (`SessionNeeded`, also in single-session mode), `CloseCursor` (tab closed or replaced), the idle bound, the end of the user transaction that owns the cursor (`TxEnded`), a lost connection (rows kept, `Closed{ConnectionLost}`). **Release action:** PG `Drain(250 ms)` (one `Execute` of ≤ 4,096 rows; complete → `Complete`), MySQL drain ≤ 250 ms else `KILL QUERY` on the side connection then `close_query(AfterKill)`, SQLite step ≤ 250 ms then finalize. `CursorReleased` is emitted with the reason.
  - **Idle bounds (R-M3-20)** from `cursor::idle_bound(kind, read_only, production, caps, policy) -> Duration`: PG read-write `policy.idle_timeout` (10 min); PG read-only 20 s; MySQL/MariaDB `caps.idle_bound` (net_write_timeout − 15 s); SQLite `caps.idle_bound` (30 s rollback journal) else 10 min; production `min(bound, 120 s)`; `policy.idle_cap` (tests inject 2 s) caps every bound. The timer is a third branch of the actor's idle `select!`. Liveness pings are skipped while a cursor is suspended.
  - **Stop fetching:** on a `Streaming` or `Suspended` result, stop requesting chunks and close the engine's way; when a chunk is overdue by more than `policy.overdue` (1 s), send the server cancel (PG protocol cancel, MySQL `KILL QUERY`, SQLite interrupt). Rows stay browsable, `Cancelled`. Inside a user transaction that server cancel leaves the transaction `Failed`, as in psql: `TxChanged { to: Failed }`, owner `User`, and no savepoint is added around non-suspendable units (R-M3-16 amended, P16).
  - **`FetchMore`/`FetchAll`** resume a suspended cursor; `FetchAll` keeps fetching until the result completes or `admit` pauses it (per-result cap or global budget, R-M3-33); a paused result resumes when the store's cap is raised and a new `FetchAll` arrives. Two queued `FetchMore`s coalesce into one fetch of the sum. The stale-cancel guard also covers `FetchMore` sequence numbers.
  - **Production read-only (R-M3-29):** verify `effective_read_only` reaches every session kind (default console, other consoles, pooled, single-session borrows, reconnects), and that a pooled production borrow finds its parked session again (`pool.rs:1005` keys idle sessions by the effective value, P4).
  - `SessionSnapshot.cursor` mirrors the open cursor; `TxState.owner` is `Cursor` for Hacivat's own transaction, `User` for a user's, `Guard` inside the read-only plan.

- [ ] **Step 1: Write the failing tests** (FakeDriver; `start_paused` only here, never with sockets).
```rust
#[tokio::test(start_paused = true)] async fn never_pending_stream_is_cancelled_within_one_chunk() { let (h, f) = session(fake_pg().queries(vec![endless(never_pending())]));
    let t = h.run(spec_of(&["SELECT * FROM big"])); wait_rows(&h, 4_096).await; h.cancel(t.seq).await.unwrap(); assert!(chunks_after_cancel(&f) <= 1); }
#[tokio::test(start_paused = true)] async fn first_chunk_is_published_before_the_second_is_requested() {}
#[tokio::test(start_paused = true)] async fn read_ahead_stops_at_10k_rows_or_16_mib() {}
#[tokio::test(start_paused = true)] async fn queued_fetch_mores_coalesce() {}
#[tokio::test(start_paused = true)] async fn another_statement_releases_the_cursor_first() { let (h, f) = suspended_session(1_000_000).await; h.submit(exec("SELECT 1")).reply.await.unwrap().unwrap();
    assert_eq!(released(&h), Some(CloseReason::SessionNeeded)); assert!(f.typed_log().ends_with(&[FakeTypedCall::Close(CloseHow::Drain(ms(250)))])); }
#[tokio::test(start_paused = true)] async fn every_idle_bound_of_the_table() { for (k, ro, prod, caps, want) in idle_cases() { assert_eq!(idle_bound(k, ro, prod, &caps, &FetchPolicy::default()), want); } }
#[tokio::test(start_paused = true)] async fn incomplete_cursor_rolls_back_hacivats_own_transaction() { let (h, f) = suspended_pg(1_000_000).await; close_cursor(&h).await;
    assert_eq!(&f.executed()[..], ["BEGIN", "ROLLBACK"]); }
#[tokio::test(start_paused = true)] async fn completed_cursor_commits() { let (h, f) = suspended_pg(12_000).await; fetch_all(&h).await; assert_eq!(&f.executed()[..], ["BEGIN", "COMMIT"]); }
#[tokio::test(start_paused = true)] async fn cursor_inside_a_user_transaction_uses_a_savepoint() { /* BEGIN by the user; then SAVEPOINT hv_cursor … ROLLBACK TO SAVEPOINT hv_cursor; RELEASE SAVEPOINT hv_cursor; status stays InTx, owner User */ }
#[tokio::test(start_paused = true)] async fn non_suspendable_unit_streams_to_completion_and_counts_past_the_cap() {}
#[tokio::test(start_paused = true)] async fn only_the_last_unit_stays_suspended() {}
#[tokio::test(start_paused = true)] async fn overdue_chunk_sends_the_server_cancel() {}
#[tokio::test(start_paused = true)] async fn cancel_in_a_user_transaction_marks_it_failed() { /* user BEGIN; a non-suspendable unit's chunk overdue > 1 s → server cancel; TxState Failed, owner User; TxChanged { to: Failed }; no savepoint sent (P16) */ }
#[tokio::test(start_paused = true)] async fn stop_on_error_skips_the_rest() { /* 3 units, the 2nd fails: Succeeded, Failed, Skipped; summary 1 of 3 executed, 1 failed */ }
#[tokio::test(start_paused = true)] async fn connection_lost_while_suspended_keeps_rows() {}
#[tokio::test(start_paused = true)] async fn liveness_skips_pings_while_suspended() {}
#[tokio::test(start_paused = true)] async fn rollback_journal_cursor_uses_the_30s_bound() {}
// tests/prod_read_only.rs
#[tokio::test] async fn production_session_is_read_only_after_reconnect() {}
#[tokio::test] async fn production_tag_applies_to_the_next_session() {}
#[tokio::test] async fn single_session_production_is_read_only() {}
#[tokio::test] async fn every_session_kind_on_a_production_profile_is_read_only() {}
#[tokio::test] async fn pooled_borrow_on_production_reuses_its_session() { /* two pooled borrows in a row open one connection; the second finds the parked read-only session (P4) */ }
#[tokio::test] async fn single_session_production_borrow_succeeds() { /* single-session mode: the borrow at pool.rs:776 returns the read-only session (P4) */ }
// tests/docker_m3a_cursor.rs (#[ignore = "docker"])
#[tokio::test] async fn pg_suspended_million_rows_released_by_select_1() {}
#[tokio::test] async fn pg_injected_idle_bound_rolls_back_hacivats_transaction() {}
#[tokio::test] async fn mysql_release_kills_on_the_side_connection_and_reuses_within_1s() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-session --test run --test cursor --test prod_read_only`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies. No request runs on a connection held by a cursor without a release first; no driver future is dropped; a release always ends Hacivat's own transaction.
- [ ] **Step 4:** Run `cargo test -p hacivat-session` (M2b suites green) and `<main>/target/docker/run-docker-tests.sh --set pr -p hacivat-session`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(session): Run requests with typed streaming into the result store, console cursors with per-engine idle bounds, stop-fetching, rollback of incomplete cursors, production consoles read-only`.

**Review:** security-sensitive (connection reuse, transactions). Probes: a cursor surviving a request; Hacivat's transaction left open; a savepoint name collision with a user savepoint named `hv_cursor`; the stale-cancel guard on `FetchMore`; a `read_only` site that `grep -nE '\.read_only\b'` finds beyond the seven; a Commit still offered after a server cancel inside a user transaction.

### Task 19: Editor view and element (UI-EDIT-01 view, -07 paint, -08 scroll, -09 layout cache; mouse) (W4, gpui)

**Files:** Modify `crates/ui/hacivat-ui-editor/src/{view,element,layout_cache,paint,mouse,scroll,theme,probe}.rs`; create `crates/ui/hacivat-ui-editor/tests/{element,view_scene}.rs`.

**Interfaces:**
- Consumes: Tasks 5, 10, 11; Task 1b's seams (`gutter`, `marks`, `preview`, `scrollbar`, `a11y`, `large_file`, `ime`, `clipboard`: exact signatures and module-owned state, P11).
- Produces: `EditorView` (model, `ImeCore`, decorations from the service's sources, config, probe) and one custom `EditorElement` (text) plus the gutter area painted by calling `gutter::paint`/`marks::paint` (Task 28's bodies), `preview::paint_band`, `scrollbar::paint` and `a11y::declare` at their paint points (Q9 seams). `EditorView` holds `marks::MarkStore`, `preview::BandState`, `a11y::A11yState`, `large_file::TierState` and `clipboard::PasteState`; its marks and preview methods delegate in one line; every committed transaction calls `marks::on_commit` and `large_file::update`; the element stores each frame's `FrameLayout` for `last_layout()` (P11). This task owns every call site; Tasks 27–29 own only their modules.
  - **Shaping:** `window.text_system().shape_line(..)` under a content-keyed LRU of 4,096 visual lines or 32 MB, key `(text_hash, runs_hash /* font + colour + underline */, font_size_bits, column_window)` (colours in the key: `ShapedLine` cannot be recoloured at the pin). Long lines (> 10,000 columns or 64 KB) shape a 512-column window stepping by 128 (the spike's `column_window` and its tests). Gutter digits are shaped once per font size.
  - **Paint order:** `ground`; gutter (`gutter` + 1 px `seam`); `Line` backgrounds (preview band); selection (`blue_tint_strong`); text runs (classes → `syn_*` tokens; comments italic; executable comments their own tint); IME underline; carets (2 × 16 px `blue`); overlays via `defer_draw`. Text starts 8 px after the gutter. **No red quad.**
  - **Scrolling:** `ScrollAnchor` + `scroll_x: f64`; rows painted relative to the anchor (1M lines exact). **Mouse:** click, drag, double (word) and triple (line) click by `click_count`; I-beam through `insert_hitbox` + `set_cursor_style`. **Caret blink:** platform half-period (530 ms fallback), never while typing, stops after 10 s idle, static under reduced motion (the mock's ruling decides the default).
  - Decorations are queried per visible range only; the `FrameProbe` is called once per input and once per painted frame. The view is embeddable as `AnyView::cached(..)`.

- [ ] **Step 1: Write the failing tests.**
```rust
#[gpui::test] fn scrolling_back_hits_the_cache_95_percent(cx: &mut TestAppContext) {}
#[gpui::test] fn theme_switch_reshapes_only_visible_lines(cx: &mut TestAppContext) {}
#[gpui::test] fn one_line_reshaped_per_typed_character(cx: &mut TestAppContext) { let (v, vcx) = editor_50k(cx); let before = shapes(&v); type_keys(&vcx, "x"); assert_eq!(shapes(&v) - before, 1); }
#[gpui::test] fn long_line_shapes_a_window(cx: &mut TestAppContext) { /* 100k chars: shaped length ≤ 512 + 128 columns */ }
#[gpui::test] fn click_drag_double_triple(cx: &mut TestAppContext) {}
#[gpui::test] fn scroll_keeps_the_anchor_across_edits_above(cx: &mut TestAppContext) {}
#[gpui::test] fn probe_called_once_per_input_and_frame(cx: &mut TestAppContext) {}
#[gpui::test] fn caret_blink_rules_with_a_fake_clock(cx: &mut TestAppContext) {}
#[cfg(feature = "scene-check")] #[gpui::test] fn caret_selection_gutter_geometry(cx: &mut TestAppContext) { /* caret 2×16 blue; selection blue_tint_strong; gutter 58 px gutter + 1 px seam; text at gutter + 8 px; zero red quads */ }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-ui-editor --features scene-check --test element --test view_scene`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies. Commands stay out of `on_key_down` (keys arrive as commands from the app's dispatcher; printable text through `EntityInputHandler`).
- [ ] **Step 4:** Run `cargo test -p hacivat-ui-editor --features scene-check`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(editor): virtualized editor element with a content-keyed shape cache, long-line windows, row-anchored scrolling, mouse selection and the harman paint order`.

**Review:** one element per area; nothing O(n) per frame; colours in the cache key; `FocusedInput` bookkeeping; the cost of the `cached()` embedding.

### Task 20: Grid element, view, `VirtualList` and the synthetic B1 (HV-GRID-01, -02 fast-scroll, -03) (W4, gpui)

**Files:** Modify `crates/ui/hacivat-ui-grid/src/{element/{mod,grid,layers,text_cache,truncate,scrollbar,pointer},view,list,testing}.rs`; fill Task 1b's stub `crates/ui/hacivat-ui-grid/examples/b1_synthetic.rs` (P7); create `crates/ui/hacivat-ui-grid/tests/{element,view,list}.rs` (Task 1b's `[[test]]` entries require `test-support`); append a `ci.yml` block `# ## m3a-t20-bench` (the `bench` job builds and runs `b1_synthetic` under `xvfb-run` in place of `spike-grid`, which stays as the frozen M0 record).

**Interfaces:**
- Consumes: Task 17's model.
- Produces: `GridView` with a custom `Element` (hitboxes, focus, a11y declaration), regions painted in order body → header → gutter → corner → scrollbars, each inside its content mask; layers 1 (background), 2 (zebra, off), 5 (empty in M3a), 6 (selection fill and selected row numbers), 7 (content: text, `NULL_MARKER` in synthesized small caps at `text-3`, chips), 8 (`seam-soft` hairlines snapped by `metrics::snap_to_device_px`), 9 (hover outline, 1 px `blue` selection box with 5 px handle and 1 px `panel` ring), 10 (resize guide).
  - **Text cache:** the spike's `FrameLru` (`spikes/spike-grid/src/element.rs:28-110`) keyed `(content hash, font id, size bits, colour class, width bucket)`, capacity 50k, never evicting the current frame.
  - **Truncation:** cap the string at `ceil(col_px / min_advance_px) + 2` graphemes before shaping; if wider, binary-search the cluster boundary and shape `prefix + "…"`.
  - **Controls:** `\n` → `↵`, `\t` → `→`, C0/C1 → control pictures, bidi controls → a chip (`⟨RLO⟩`); the cell's text is unchanged (copy keeps the real characters).
  - **Fast-scroll (R-M3-12):** while velocity > 40 rows per frame (a thumb jump counts as infinite for one frame), cached shapes paint as usual, uncached cells paint a ≤ 24-character ASCII-safe prefix as one run, chips are skipped, at most 150 shapes per frame; a full-quality frame follows within 100 ms of the velocity dropping.
  - **Scrollbars:** 9 px vertical track on `gutter` with a 1 px `seam` edge; thumb `text-3` at 0.6 opacity, 3 px radius; the **loaded band in `green-fill`**; while the total is unknown the track maps to `loaded`.
  - Header: 36 px on `head`, `seam-strong` bottom border, name in `body-strong`, mono type subtitle (`mono-kbd` 11 px, `text-3`), numeric columns right-aligned. Row-number gutter ≥ 46 px, widened to fit. Rows at or past `loaded` are `Placeholder` (a static `seam-soft` bar) and trigger at most one `ensure_rows` per frame.
  - Every painted run is recorded in `TextRegistry` when it is enabled (cells `Data`, headers `Catalog`; `truncated` for ellipsized text).
  - `a11y_role` = `Grid` with the `a11y_label` (R-M3-32); no synthetic children in M3a.
  - `VirtualList<R>` (one column, the same scroll model, fixed row height) and `SyntheticSource` (the spike's source behind `GridDataSource`, feature `test-support`).

- [ ] **Step 1: Write the failing tests.**
```rust
#[gpui::test] fn ten_million_by_fifty_first_frame_renders(cx: &mut TestAppContext) {}
#[gpui::test] fn placeholders_trigger_one_ensure_rows_per_frame(cx: &mut TestAppContext) { let (g, src) = grid_over(partial_source(1_000)); scroll_to(&g, 5_000); assert_eq!(src.ensure_calls_this_frame(), 1); }
#[gpui::test] fn tab_and_newline_paint_as_glyphs(cx: &mut TestAppContext) { assert_eq!(painted_cell_text(cx, "a\tb\nc"), "a→b↵c"); }
#[gpui::test] fn bidi_controls_paint_as_chips(cx: &mut TestAppContext) { assert!(painted_chips(cx, "x\u{202E}y").contains(&"⟨RLO⟩".to_string())); }
#[gpui::test] fn fast_scroll_caps_shapes_per_frame(cx: &mut TestAppContext) { let g = fling_at_rows_per_frame(cx, 400.0); assert!(g.max_shapes_per_frame() <= 150); }
#[gpui::test] fn quality_frame_within_100ms_of_slowing(cx: &mut TestAppContext) {}
#[gpui::test] fn cache_hits_allocate_nothing(cx: &mut TestAppContext) { /* debug allocation counter on the paint path */ }
#[gpui::test] fn keyboard_moves_and_select_all(cx: &mut TestAppContext) {}
#[gpui::test] fn grid_declares_role_and_label(cx: &mut TestAppContext) { assert_eq!(declared(cx), (Role::Grid, "Result 1".into())); }
#[cfg(feature = "scene-check")] #[gpui::test] fn eleven_px_floor_no_cp_fill_and_snapped_hairlines(cx: &mut TestAppContext) { /* at 1×, 1.5×, 2× */ }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-ui-grid --features test-support,scene-check --test element --test view --test list`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies and the `b1_synthetic` example (feature `bench`).
- [ ] **Step 4:** Run `cargo test -p hacivat-ui-grid --features test-support,scene-check`, then `cargo run --release -p hacivat-ui-grid --features bench,test-support --example b1_synthetic -- --bench fling` on the PO's Mac (record; ≥ 120 fps at 120 Hz expected as in M0). Expected: PASS.
- [ ] **Step 5: Commit** `feat(grid): grid element with layered regions, frame-LRU text cache, width-capped truncation, visible controls, fast-scroll, loaded-band scrollbars; VirtualList; synthetic B1`.

**Review:** paint-path allocations; content masks per region; device-pixel snapping; the text-cache key; fast-scroll recovery within 100 ms.

### Task 21: Overflow Rule toolbar (DESIGN.md Overflow Rule) (W4, gpui)

**Files:** Modify `crates/ui/hacivat-ui/src/toolbar.rs` (module line from Task 1b); create `crates/ui/hacivat-ui/tests/toolbar.rs`.

**Interfaces:**
- Consumes: `hacivat-ui`'s `titlebar_fit` precedent (`crates/ui/hacivat-ui/src/titlebar.rs:34-60`), `segmented.rs:298` (`natural_width`), `menu.rs` (`PopupMenu`).
- Produces: `Toolbar::new(id).item(ToolbarItem { priority: u8, element: Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>, overflow_label: TextFn, action: Option<RunCommand> }).trailing(..)`; natural-width measurement; trailing items move into a 24 px overflow menu (in priority order) before any label truncates; 34 px height on `raised`, `text-2` ink.

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn width_math_moves_lowest_priority_first() { assert_eq!(fit(&[(80.0, 2), (60.0, 1), (90.0, 3)], 200.0), Fit { shown: vec![0, 2], overflow: vec![1] }); }
#[gpui::test] fn nothing_truncates_at_960_in_tr_xt(cx: &mut TestAppContext) {}
#[gpui::test] fn overflow_menu_works_from_the_keyboard(cx: &mut TestAppContext) {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-ui --test toolbar`. Expected: FAIL.
- [ ] **Step 3: Implement** the body. The measured width equals the painted width.
- [ ] **Step 4:** Run `cargo test -p hacivat-ui`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(ui): Overflow Rule toolbar — items move into an overflow menu before labels truncate`.

**Review:** measured vs painted width; the Raised Ink Rule; keyboard reachability of the overflow menu.

### Task 22: Typed read-only (guarded) execution (HV-SESS-04 on the typed path; R-M3-17) (W5)

**Files:** Modify `crates/core/hacivat-session/src/session/readonly.rs` (`guarded_query` and its helpers); create `crates/core/hacivat-session/tests/{readonly_typed,docker_m3a_readonly}.rs`; append ADR 0004 `## m3a-t22 — typed read-only path`.

**Interfaces:**
- Consumes: Task 18's run loop (which calls `guarded_query` for read-only sessions); M2b's guard plan, `check_input`, `check_post`, step 1a keywords and replay.
- Produces: `guarded_query` — the M2b steps on the typed path:
  1. `check_input`, else `ReadOnlyDenied(Input)`; 1a. transaction keywords → `ReadOnlyDenied(Transaction)` (M2b `guard_keywords`, executable comments read both ways); M2b's function check (`DenyLayer::Function`) unchanged;
  2. `plan.before` (`BEGIN … READ ONLY` / `START TRANSACTION READ ONLY`) through `batch`, `TxOwner::Guard`;
  3. the typed `query` (extended/prepared protocol only);
  4. **PostgreSQL: a canary after the first chunk** (`SELECT txid_current_if_assigned()` while the named portal stays open: the PostgreSQL exception to "one open query per connection" in Task 1b's driver contract, P5); a non-NULL canary → enforcement gap;
  5. at the cursor's end (complete, released, cancelled or idle-bounded): the canary (PG), `ROLLBACK`, `check_post`, `reset`, `replay` — **the post-steps wait for the cursor** (R-M3-17), and no other statement joins the guard's transaction because the actor releases the cursor first (D-019);
  6. a gap discards the connection, emits `SessionEvent::EnforcementGap`, and marks the result `Closed { EnforcementGap }` with its rows kept.
  - Idle bound 20 s on PostgreSQL read-only sessions (the server kills idle transactions at 30 s, `plan.rs:37`); MySQL uses the final OK packet for `WriteSucceeded`; SQLite keeps the authorizer and `stmt_readonly`.

- [ ] **Step 1: Write the failing tests.**
```rust
#[tokio::test(start_paused = true)] async fn guard_begin_precedes_the_query_and_rollback_ends_it() { let (h, f) = ro_session(fake_pg().queries(vec![rows_q(10)]));
    run_ok(&h, "SELECT * FROM t").await; assert_eq!(steps(&f), [Step::PlanBefore, Step::Query, Step::Canary, Step::Canary, Step::Rollback, Step::Reset, Step::Replay]); }   // plan.before = plan_for(Postgres).before
#[tokio::test(start_paused = true)] async fn post_steps_wait_for_the_cursor() {}
#[tokio::test(start_paused = true)] async fn canary_after_the_first_chunk_catches_a_writing_function() {}
#[tokio::test(start_paused = true)] async fn another_request_releases_and_runs_the_post_steps_first() {}
#[tokio::test(start_paused = true)] async fn gap_discards_the_connection_and_flags_the_rows() {}
#[tokio::test(start_paused = true)] async fn ro_bound_is_20s() {}
#[tokio::test(start_paused = true)] async fn mysql_read_only_runs_reset_after_release() {}
#[tokio::test] async fn sqlite_authorizer_and_stmt_readonly_on_the_typed_path() {}
#[tokio::test(start_paused = true)] async fn nothing_else_runs_inside_the_guard_transaction() { /* D-019 via executed() */ }
// tests/docker_m3a_readonly.rs (#[ignore = "docker"])
#[tokio::test] async fn server_blocked_bypass_rows_are_blocked_through_run() { /* the M2b rows of docker_m2b_bypass.rs, sent through Request::Run; same outcomes as run B */ }
#[tokio::test] async fn ro_cursor_is_released_before_the_server_idle_kill() { /* real 20 s bound vs the server's 30 s; the cursor ends as IdleTimeout, never ConnectionLost */ }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-session --test readonly_typed`. Expected: FAIL.
- [ ] **Step 3: Implement** the body. Never send user text through `batch`.
- [ ] **Step 4:** Run `cargo test -p hacivat-session` and `<main>/target/docker/run-docker-tests.sh --set full -p hacivat-session`. Expected: PASS; the M2b bypass suite (run B) unchanged.
- [ ] **Step 5: Commit** `feat(session): typed guarded read-only path — post-steps at the cursor's end, canary after the first chunk, 20 s cursor bound, gap discard with rows kept`.

**Review:** security-sensitive (the read-only guard). Probes: a row stream that ends the transaction partway; a function that writes on the 10,001st row (caught at release); a stream that never ends; `CALL` with the classifier off (the documented M2b residual stays recorded).

### Task 23: `hacivat-exec`: units, run service, run status, output-log model, console UX state (HV-EXEC-01 wrapping, -05 submit, -11 model) (W5)

**Files:** Modify `crates/core/hacivat-exec/src/{units,sqlctx,service,observer,status,log,ux,titles}.rs`; create `crates/core/hacivat-exec/tests/{units,service,log,ux}.rs`; append ADR 0006 `## m3a-t23 — run service`; append `hacivat-core.ftl` blocks (EN, TR) `## m3a-t23-exec` with every `LogMsg` id (`exec-rows-retrieved`, `exec-rows-affected`, `exec-completed`, `exec-tx-committed`, `exec-tx-rolled-back`, one `cursor-closed-reason-<reason>` per `CloseReason` (each with the "Hacivat's transaction was rolled back" variant, R-M3-23), `exec-more-result-sets`, `exec-summary`, `exec-error`, `exec-cancelled`, one `exec-deny-read-only-<layer>` per `DenyLayer`, `exec-events-dropped`, `exec-copy-unsupported`), TR marked `# Review: PO`. `LogMsg::fluent_id()` returns a literal id in every arm, which the `l10n check` code scan counts as used; no family enters `dynamic-ids.toml` (P24).

**Interfaces:**
- Consumes: Task 13's `resolve`, Task 8's `cursor_eligible`, Task 9's oracle, Task 18's sessions; Task 1b's `hacivat_session::testing::fake_session`, which the tests' `service_over` wraps (P6).
- Produces:
  - `resolve_units`: `exec_range::resolve(doc, at, policy_for(action), SplitMode::Execution, None, remembered)`; each `UnitSpan` → `ExecUnit { text = strict slice, doc_range, kind, suspendable = head::cursor_eligible(..) }`. **R-M3-24:** for `Ambiguous` on PostgreSQL, when every involved unit's `max_paren_depth ≤ MAX_ORACLE_NESTING`, run `split_with_parser` on the statement region inside `on_parser_stack` and use its boundaries when they cover the region exactly; otherwise `Resolution::Uncertain`. `UnsupportedClientCommand` passes through.
  - `sqlctx` (R-M3-39): `DbmsKind` → `DialectId` (PG-wire flavours → `PgWire`), probe SQL, `dialect_ctx` (PG `standard_conforming_strings`; MySQL/MariaDB `LexModes::from_mysql_sql_mode`; version from `SemVerLoose`).
  - `ExecService::run`: builds `RunSpec` (`RunId` from a counter; units in order; `StopOnError`), submits `Request::Run`, and returns a `RunTicket`. Its `ExecObserver` (`RunObserver`) creates `ResultStore`s on the shared budget, publishes `RunStatus` (Queued → Running → Fetching/Suspended → Succeeded/Failed/Cancelled/Skipped), `Arc<[ResultTab]>` (one per unit, titled `Result N` by `titles`, keyed `TabKey { console, unit_ordinal, 0 }`), and `LogLine`s to the console's `SessionLog` (ring of 10,000; a full ring drops the oldest; `Dropped { n }` when a burst outruns it). `RowsFetched` updates are coalesced to one status publish per 50 ms. Errors map to `ErrorView` (title id by error kind, SQLSTATE/vendor code chip, verbatim server text, `doc_offset` from the server position mapped through `LineIndex::char_to_byte` into the unit and then the document).
  - `copy_invariant`: ISO timestamps, ASCII digits, `ms`.
  - `console_ux`: the pure table (production, forced read-only, access, `TxBadge::UserOpen` only for `TxOwner::User`, `TxBadge::Failed` for a user transaction whose status is `Failed` (R-M3-16 amended), cursor badge, cancelling, running, waiting).

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn units_carry_strict_text_and_eligibility() { let r = resolve_doc("SELECT 1; INSERT INTO t VALUES (1);", sel(0..35), RunAction::Execute1);
    assert_eq!(units(&r).iter().map(|u| (&*u.text, u.suspendable)).collect::<Vec<_>>(), [("SELECT 1", true), ("INSERT INTO t VALUES (1)", false)]); }
#[test] fn pg_ambiguity_resolved_by_the_oracle_when_it_covers_the_region() {}
#[test] fn deep_nesting_is_never_sent_to_the_oracle() { assert!(matches!(resolve_pg_ambiguous_deep(300), Resolution::Uncertain(_))); }
#[test] fn mysql_ambiguity_stays_uncertain() {}
#[test] fn sql_mode_ansi_changes_quote_lexing() { assert!(dialect_ctx(DbmsKind::MySql, None, Some("ANSI")).modes.ansi_quotes); }
#[tokio::test] async fn select_1_event_order_and_status() { let (svc, h, _) = service_over(fake_pg().queries(vec![rows_q(1)])); let t = svc.run(&h, req(&["SELECT 1"]));
    assert_eq!(status_trail(&t).await, ["Queued", "Running", "Succeeded"]); assert_eq!(log_ids(&svc), ["exec-rows-retrieved"]); }
#[tokio::test] async fn two_units_two_tabs_titled_result_n() {}
#[tokio::test] async fn rerun_reuses_the_tab_key_and_bumps_generation() {}
#[tokio::test] async fn failed_unit_maps_server_position_to_the_document() {}
#[tokio::test] async fn more_result_sets_are_logged() {}
#[test] fn log_ring_keeps_10k_and_counts_drops() {}
#[test] fn copy_invariant_is_iso_and_ascii_under_turkish() {}
#[test] fn every_log_id_exists_in_en_and_tr() { for m in all_log_msgs() { assert!(ftl_has("en", m.fluent_id()) && ftl_has("tr", m.fluent_id()), "{}", m.fluent_id()); } }
#[test] fn console_ux_table() { for (inputs, want) in ux_cases() { assert_eq!(console_ux_of(inputs), want); } }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-exec`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies. Resolution runs where the caller runs it (the app calls it on the GPUI background executor); the observer is O(1) per event and never blocks.
- [ ] **Step 4:** Run `cargo test -p hacivat-exec`, `cargo xtask l10n check`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(exec): execution units from the execution-mode split with cursor eligibility and the PG oracle for ambiguity, run service with status, result tabs and the output-log model, console UX state`.

**Review:** security-sensitive (what runs). Probes: no unit crosses an uncertain boundary; the text sent equals the resolved strict range byte for byte; the oracle is skipped above the nesting limit; status order under cancel.

### Task 24: SQL adapter `hacivat-sql-editor` (UI-EDIT-11 L1, -21 source, -06 token oracle; R-M3-8, R-M3-22) (W5, gpui)

**Files:** Modify `crates/app/hacivat-sql-editor/src/{service,worker,highlight,statements,preview,words,chunks}.rs`; create `crates/app/hacivat-sql-editor/tests/{highlight,worker,preview_truth,budget}.rs`.

**Interfaces:**
- Consumes: Tasks 8 and 13 (`SqlDocument`, `resolve`), Task 19's `EditorView` (for the gpui tests), Task 1b's `LanguageService`.
- Produces: `SqlLanguageService`:
  - **Worker (R-M3-22):** per document, a task on the GPUI background executor fed by a `tokio::sync::watch` of the latest `(version, Snapshot)`; it builds `SqlDocument::from_chunks(&RopeChunks(snapshot.buffer), ctx, flavor)` and publishes `(version, DocSnapshot)`; only the newest snapshot is analysed (at most one analysis in flight). A foreground task awaits the result, stores it and calls the `on_update` notifier. `change()` only updates the watch: nothing is lexed on the UI thread.
  - **HighlightSource** (`DecorationSource`): `TokenClass` → `HighlightClass` (`Function` for a `Word` directly followed by `(`; executable comments `ExecComment`); decorations of an older version are rebased through the editor's `ChangeLog` (`RangeKind::Content`); ranges an edit touched since then are left plain until the next analysis.
  - **`statements`** from the latest editor-mode split, rebased; **`exec_preview`** returns `None` unless a snapshot exists at exactly the editor's version, then `resolve(.., SplitMode::Editor, ..)` mapped to `ExecPreview` (`Units`, `AskPreselected` with the preselected candidate's ranges, `Nothing`, `Uncertain` when an involved item is `Heuristic` or the result is `Ambiguous`, `NoConnection` from `DocContext`).
  - **`token_oracle`:** words on a line from the current-version tokens; `None` (class fallback) when stale.
  - `doc_snapshot(doc, version)` lets Run reuse the analysis when versions match.

- [ ] **Step 1: Write the failing tests.**
```rust
#[gpui::test] fn ui_edit_11_cases_1_and_3(cx: &mut TestAppContext) { /* keywords, dollar strings, E-strings, executable comments get their classes */ }
#[gpui::test] fn worker_result_equals_a_fresh_document_after_random_edits(cx: &mut TestAppContext) { /* 10⁴ random edits on a ropey-backed TextChunks; final DocSnapshot == SqlDocument::new(final text) */ }
#[gpui::test] fn change_never_lexes_on_the_calling_thread(cx: &mut TestAppContext) { assert_eq!(lex_calls_on_ui_thread(cx), 0); }
#[gpui::test] fn stale_snapshot_never_yields_a_preview(cx: &mut TestAppContext) { let s = svc_with_pending(cx); assert!(s.exec_preview(DOC, 7, &caret(3), ExecAction::Execute1).is_none()); }
proptest! { #[test] fn preview_equals_execution_for_exact_items(doc in sql_docs(), c in carets()) { prop_assert!(preview_matches_resolve(&doc, c)); } }
#[gpui::test] fn single_candidate_preview_is_units(cx: &mut TestAppContext) {}
#[gpui::test] fn heuristic_boundary_previews_uncertain(cx: &mut TestAppContext) {}
#[gpui::test] fn touched_range_paints_plain_until_the_next_analysis(cx: &mut TestAppContext) {}
#[test] #[ignore = "perf"] fn change_p95_under_half_a_millisecond_on_50k_lines() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-sql-editor`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies.
- [ ] **Step 4:** Run `cargo test -p hacivat-sql-editor` and `cargo test -p hacivat-ui-editor`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(sql-editor): SQL language service with off-thread analysis, rebased lexical highlighting, truthful Run preview and token words`.

**Review:** security-sensitive (a preview that execution would not run is the §2.3 anti-goal R-M3-8 rests on; P28). Probes: never a preview that execution would not run; no whole-document `String` on the UI thread per keystroke (the worker's copy is off-thread); the dialect comes from `set_sql_context`.

### Task 25: Editor commands and keymap behaviour (UI-EDIT-05 single-cursor, -06 movement, -13 data) (W5, gpui)

**Files:** Modify `crates/ui/hacivat-ui-editor/src/commands/{mod,movement,edit,selection,run}.rs`; create `crates/ui/hacivat-ui-editor/tests/commands.rs`; append `locales/{en,tr}/hacivat-ui.ftl` block `## m3a-t25-editor-commands` (`hv-cmd-editor-*` titles, R5 naming).

**Interfaces:**
- Consumes: Task 19's view; Task 5's model and history; Task 10's units; the keymap rows of Task 1b.
- Produces: the bodies of every `commands()` entry. Each goes through `EditorModel::dispatch` with the right `UserEvent`, never through `on_key_down`:
  - movement and selection (`move_*`/`select_*` by grapheme, token word, smart line start, document, page with `goal_x` kept);
  - `newline` (the dominant line ending; keeps the indentation), `tab` (4 spaces by default, `\t` with `insert_spaces = false`), `outdent`, `delete_*` (word deletes use token words), `delete_to_line_start`, `select_all`, `undo`, `redo`, `escape` (collapse the selection; a second Escape is not consumed, so Escape-then-Tab leaves the editor), `toggle_tab_moves_focus` (Ctrl+M);
  - `copy`/`cut`/`paste` delegate to `clipboard::*` (Task 27);
  - `run`, `run_2`, `run_3`: seal history; while composing, refuse (F15: the dispatcher already never fires a binding then); emit `EditorEvent::ExecuteRequested(ExecRequest { doc, action, at, snapshot })` where `at` is `ExecAt::Selections` when any range is non-empty, else `ExecAt::Carets`;
  - `describe_exec_range`: a polite announcement of the current preview ("Run would execute lines 3–7") through `a11y::announce` plus the status text;
  - `context_menu`: a `PopupMenu` with Cut, Copy, Paste, Select all · Run, Describe execution range.

- [ ] **Step 1: Write the failing tests.**
```rust
#[gpui::test] fn every_command_has_a_test(cx: &mut TestAppContext) { assert_eq!(untested_commands(), Vec::<&str>::new()); }
#[gpui::test] fn enter_inserts_the_dominant_ending_and_keeps_indent(cx: &mut TestAppContext) { assert_eq!(after(cx, "  a\r\n  b┃", "editor.newline"), "  a\r\n  b\r\n  ┃"); }
#[gpui::test] fn tab_inserts_four_spaces_or_a_tab(cx: &mut TestAppContext) {}
#[gpui::test] fn delete_word_uses_token_words(cx: &mut TestAppContext) { assert_eq!(after(cx, r#"SELECT "Müşteri Adı"┃"#, "editor.delete_word_backward"), "SELECT ┃"); }
#[gpui::test] fn escape_collapses_then_lets_tab_leave(cx: &mut TestAppContext) {}
#[gpui::test] fn ctrl_m_toggles_tab_moves_focus(cx: &mut TestAppContext) {}
#[gpui::test] fn run_emits_selections_or_carets_and_seals_history(cx: &mut TestAppContext) { let e = run_with_selection(cx, 0..8); assert!(matches!(e.at, ExecAt::Selections(_))); }
#[gpui::test] fn run_while_composing_does_nothing(cx: &mut TestAppContext) {}
#[gpui::test] fn ctrl_m_fires_from_its_key_on_tr_q_and_tr_f(cx: &mut TestAppContext) {}   // UI-EDIT-13 case 1 on the bound `ctrl-m` (M3a binds no `ctrl-i`, P25)
```
- [ ] **Step 2:** Run `cargo test -p hacivat-ui-editor --test commands`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies.
- [ ] **Step 4:** Run `cargo test -p hacivat-ui-editor --features scene-check` and `cargo test -p hacivat-ui --test keymap_collisions`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(editor): single-cursor movement, editing, history and Run commands through transactions; context menu`.

**Review:** every command dispatches a transaction; history seal flags; no binding fires during a composition; chords per platform.

### Task 26: Grid adapter, results model, raw-TSV copy (HV-GRID-01 source, -10 use; HV-EXEC-07 basic; HV-XFER-02 minimal) (W5, gpui)

**Files:** Modify `crates/app/hacivat-app/src/results/{adapter,cache,tabs,context,copy}.rs` (Task 1b's stubs; `results/mod.rs` stays Task 1b's, P12); create `crates/app/hacivat-app/tests/results_model.rs`; append `hacivat-ui.ftl` block `## m3a-t26-results-model` (`hv-results-tab-default`, `hv-copy-done`, `hv-copy-refused-*`, the truncation chip `hv-grid-chip-truncated`).

**Interfaces:**
- Consumes: Task 7's `ResultSource`, Task 16's `valfmt`, Task 20's `GridDataSource`, Task 23's `ResultTab`.
- Produces:
  - `GridAdapter` (`GridDataSource`): `begin_frame` = one `snapshot()`; `columns` from the `ResultSchema` (`name` as `Catalog` text; `type_label` = the native type name, e.g. `numeric(14,2)`; kind, alignment and default width by logical type; `ColId` by ordinal so duplicate names keep their order); `cell` = `snapshot.cell` → `format_cell` → `CellContent` through a display cache keyed `(generation, row, col, settings_version)` (**not** `version`), 64k entries, LRU by frame generation, so streaming appends never empty it; `ensure_rows` forwards (the source coalesces). The truncation chip text comes from Fluent here.
  - `ConsoleResults`: Output first, then one tab per unit result keyed `TabKey`; a re-run of the same key reuses the tab (generation bump; widths kept when the column names match); titles `Result N` (`hv-results-tab-default`); `ResultContext` and `active_result_context` for the status bar.
  - Copy (`GridEvent::CopyRequested`): `normalize_for_copy` → raw values through `raw_text` → TSV (NULL → empty; quoting for `\t`, `\n`, `\r`, `"` and outer whitespace; binary as `\x…`), one `ClipboardItem::new_string`; above 10k cells on the background executor; above 1,000,000 cells or 100 MB refused with a toast; success toast "Copied R × C cells" with locale-formatted counts.

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn copy_region_with_null_tab_and_quote_is_exact_tsv() { assert_eq!(tsv(&fixture_3x3()), "1\t\t\"a\tb\"\n2\t\"say \"\"hi\"\"\"\tx\n3\tNULLTEXT\t\" \"\n"); }   // 3 × 3; a whitespace-only cell is quoted (P18)
#[test] fn copy_keeps_control_characters_verbatim() { assert!(tsv(&one_cell("x\u{202E}y")).contains("x\u{202E}y")); }
#[test] fn duplicate_column_names_keep_their_order() { assert_eq!(names(&adapter_over(dupes())), ["id", "id", "name"]); }
#[gpui::test] fn display_cache_survives_version_bumps(cx: &mut TestAppContext) { let a = streaming_adapter(cx); paint_rows(&a, 0..40); let n = format_calls(); append_chunk(&a); paint_rows(&a, 0..40); assert_eq!(format_calls(), n); }
#[gpui::test] fn generation_change_clears_the_selection(cx: &mut TestAppContext) {}
#[test] fn rerun_reuses_the_tab_and_keeps_widths_when_names_match() {}
#[test] fn titles_are_result_n() { assert_eq!(titles(&two_units()), ["Result 1", "Result 2"]); }
#[gpui::test] fn copy_over_a_million_cells_is_refused(cx: &mut TestAppContext) {}
#[test] #[ignore = "perf"] fn cell_through_the_adapter_hit_under_200_ns() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-app --features scene-check --test results_model`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies; raw and display channels never mix (copy uses raw, the grid uses display).
- [ ] **Step 4:** Run `cargo test -p hacivat-app --features scene-check,pseudo --test results_model`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(app): grid adapter over the result store with a version-independent display cache, console result tabs, raw-TSV copy`.

**Review:** cache keying; raw vs display channels; nothing awaits on the UI thread; the copy guard thresholds.

### Task 27: Editor IME handler and clipboard (UI-EDIT-12 view side, -14; R-M3-7) (W6, gpui)

**Files:** Modify `crates/ui/hacivat-ui-editor/src/{ime,clipboard}.rs`; create `crates/ui/hacivat-ui-editor/tests/{ime_view,clipboard}.rs`; append `hui.ftl` block `## m3a-t27-clipboard` (NUL warning, paste refusal, clipboard unavailable).

**Interfaces:**
- Consumes: Task 19's view and `last_layout()`, Task 10's `ImeCore`, Task 5's model; Task 1b's `clipboard::PasteState` (P11).
- Produces:
  - `impl EntityInputHandler for EditorView`, complete: UTF-16 ranges through the buffer; marked text through `ImeCore` (inside an IME bracket); `bounds_for_range` from `EditorView::last_layout()` (Task 19, P11), with the estimate path between an edit and the next frame; `character_index_for_point`; reversed `selected_text_range`; `accepts_text_input` false when `read_only`; `text_input_configuration` with autocorrect and autocapitalise off; `FocusedInput { entity, composing }` written on focus and on every mark/unmark (the `TextInput` pattern, `crates/ui/hacivat-ui/src/input/mod.rs:266-289`); the blur rule (app switch keeps the composition; an in-window focus move commits). **`notify_text_input_changed` only after non-IME edits** (commands, paste, undo, programmatic) (R-M3-7).
  - Clipboard: copy and cut (line-wise without a selection when `copy_line_without_selection`); paste reads **asynchronously** (`read_from_clipboard_async`) and applies to the selection current when the read lands, as one `InputPaste` transaction; a leading BOM stripped; line endings converted to the dominant one; a NUL shows a warning toast and is inserted visibly (the `Control` fragment); a paste that would take the document over T1's 20 MB is refused with `PasteRefused { bytes }` (R-M3-49).

- [ ] **Step 1: Write the failing tests.**
```rust
#[gpui::test] fn bounds_for_range_is_within_2px_of_the_caret(cx: &mut TestAppContext) {}
#[gpui::test] #[ignore = "perf"] fn bounds_for_range_at_line_49000_under_1ms(cx: &mut TestAppContext) {}   // a wall-time assert: perf-gated (P27)
#[gpui::test] fn reversed_selection_reports_reversed(cx: &mut TestAppContext) {}
#[gpui::test] fn read_only_editor_refuses_text_input(cx: &mut TestAppContext) {}
#[gpui::test] fn focused_input_composing_flips_on_mark_and_unmark(cx: &mut TestAppContext) {}
#[gpui::test] fn notify_only_after_non_ime_edits(cx: &mut TestAppContext) { assert_eq!(notifies_after(cx, &[Ime("ş"), Paste("x"), Undo]), 2); }
#[gpui::test] fn blur_rule_active_and_inactive_window(cx: &mut TestAppContext) {}
#[gpui::test] fn ui_edit_14_cases_1_4_5_6(cx: &mut TestAppContext) {}
#[gpui::test] fn paste_applies_to_the_selection_current_when_it_lands(cx: &mut TestAppContext) {}
#[gpui::test] fn nul_paste_warns_and_shows_the_control_glyph(cx: &mut TestAppContext) {}
#[gpui::test] fn paste_over_20_mb_is_refused(cx: &mut TestAppContext) {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-ui-editor --test ime_view --test clipboard`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies. Clipboard reads never block the UI thread.
- [ ] **Step 4:** Run `cargo test -p hacivat-ui-editor --features scene-check`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(editor): full IME input handler with UTF-16 ranges and the blur rule; asynchronous normalised paste with size guards`.

**Review:** UTF-16 vs bytes; never splitting a surrogate pair; no binding during a composition; clipboard reads never block.

### Task 28: Gutter, execution marks, Run preview band, scrollbar markers (UI-EDIT-19, -21 rendering) (W6, gpui)

**Files:** Modify `crates/ui/hacivat-ui-editor/src/{gutter,marks,preview,scrollbar}.rs`; create `crates/ui/hacivat-ui-editor/tests/{marks,gutter_scene,preview}.rs`; append `hui.ftl` block `## m3a-t28-gutter` (the Stale tooltip, "Nothing to run here", the preview states).

**Interfaces:**
- Consumes: Task 19's paint and commit points; Task 1b's `GutterCtx`, `VisibleRows`, `ScrollbarCtx`, `MarkStore` and `BandState` (P11); `LanguageService::{statements, exec_preview}`.
- Produces:
  - **Gutter:** line numbers from the digit cache; width 58 px + (digits − 3) × advance, padded so it never jitters; the current line's number at `text`, others `text-3`.
  - **Marks** (the bodies of `marks::{add, set_state, clear, on_commit, paint}` behind `add_exec_marks`, `set_exec_state` and `clear_exec_marks`, P11): container anchors (`RangeKind::Container`), header anchor at the first non-comment token; rebased from the submitting version through the `ChangeLog` (`TooOld` → dropped); an edit touching a mark's range marks it **stale** (dimmed, "Edited since execution"); a range mapped to empty removes the mark with a tombstone kept for 64 transactions so undo restores it; re-execution replaces the mark; a 1 Hz timer runs only while a mark is Running or Cancelling and the window is visible; no spinner under 50 ms; glyph at x = 8, numbers right-padded 14 px; **Failed is an ink glyph and Cancelling amber, never red**; Succeeded draws the 1 px `green` executed-range bar; the end-of-line status (label from the app's `TextFn`) hides when it would collide with text.
  - **Preview band** (UI-EDIT-21): queried 50 ms after the last change or selection move; `None` from the service draws no band; `Units` → a full-row band (`blue_tint` .55 dark / .8 light) over the units, with separators between AsSeparate units; `AskPreselected` → the preselected candidate; `Nothing` → no band and the status text "Nothing to run here"; `Uncertain` → the uncertain style from the mock; `NoConnection` → muted; `set_preview_override` (chooser hover, through `preview::set_override`) wins; the band is frozen while composing. The 1 px `blue` current-statement bar sits in the gutter.
  - **Scrollbar markers** for execution marks and the current statement.

- [ ] **Step 1: Write the failing tests.**
```rust
#[gpui::test] fn ui_edit_19_cases_1_to_5(cx: &mut TestAppContext) {}
#[gpui::test] fn edit_inside_a_mark_makes_it_stale(cx: &mut TestAppContext) {}
#[gpui::test] fn tombstone_restores_a_mark_on_undo(cx: &mut TestAppContext) {}
#[gpui::test] fn timer_stops_when_nothing_runs(cx: &mut TestAppContext) {}
#[gpui::test] fn every_preview_state_renders(cx: &mut TestAppContext) {}
#[gpui::test] fn stale_service_answer_draws_no_band(cx: &mut TestAppContext) {}
#[gpui::test] fn chooser_override_wins_and_composition_freezes(cx: &mut TestAppContext) {}
#[cfg(feature = "scene-check")] #[gpui::test] fn gutter_geometry_and_hues(cx: &mut TestAppContext) { /* green 1 px bar; blue 1 px current bar; band alpha; glyph at x=8; no red for Failed/Cancelling; 5-digit gutter wider */ }
#[gpui::test] fn end_of_line_status_hides_on_collision(cx: &mut TestAppContext) {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-ui-editor --features scene-check --test marks --test gutter_scene --test preview`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies.
- [ ] **Step 4:** Run `cargo test -p hacivat-ui-editor --features scene-check`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(editor): gutter with execution marks, staleness and tombstones, truthful Run preview band, scrollbar markers`.

**Review:** marks as container anchors; rebasing a stale version; hue rules; the band never drawn from a stale preview.

### Task 29: Editor accessibility baseline and large-file tiers (UI-EDIT-24 subset, -23 T0/T1; D-071, D-073) (W6, gpui)

**Files:** Modify `crates/ui/hacivat-ui-editor/src/{a11y,large_file}.rs`; create `crates/ui/hacivat-ui-editor/tests/{a11y,large_file}.rs`; append `hui.ftl` block `## m3a-t29-a11y-tiers` (the editor label, announcements, the T1 banner).

**Interfaces:**
- Consumes: Task 19's view and its `a11y::declare` and `large_file::update` call points; Task 1b's `A11yState` and `TierState` (P11).
- Produces:
  - **A11y:** role `MultilineTextInput` with the `a11y_label`; a pure window builder (`a11y::window(buf, caret_line, lines: u32) -> Vec<RunNode>`): ±100 lines (D-073), runs split at 255 characters (AccessKit `u8` lengths), CRLF counted as one character, node ids from stable line anchors (`synthetic_node_id(key)`), the text selection; built **only while assistive technology is active** (`window.is_a11y_active()`); polite live announcements for execution status (`a11y::announce`); Escape-then-Tab leaves the editor.
  - **Tiers (M3a: T0 ≤ 4 MB, T1 ≤ 20 MB):** `large_file::tier_for(bytes, longest_line, limits)`; T1 ("Reduced mode") shows the inline banner and the status segment, disables the a11y-optimised window growth, and keeps typing within budget; switching back to T0 only below 80 % of the T0 limit (no flapping). Content above 20 MB never enters the editor in M3a (R-M3-49).

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn window_is_plus_minus_100_lines() {}
#[test] fn runs_split_at_255_chars_and_crlf_is_one() { let w = window_of("x".repeat(600) + "\r\n"); assert_eq!(w[0].runs.len(), 3); }
#[test] fn selection_250_to_260_spans_runs_1_and_2() {}
#[test] fn keys_stay_stable_when_lines_are_inserted_above() {}
#[gpui::test] fn editor_declares_multiline_text_input_with_its_label(cx: &mut TestAppContext) {}
#[gpui::test] fn nothing_is_built_without_assistive_technology(cx: &mut TestAppContext) {}
#[gpui::test] fn execution_status_is_announced_politely(cx: &mut TestAppContext) {}
#[test] fn tier_choice() { assert_eq!(tier(3 << 20, 60_000), Tier::Full); assert_eq!(tier(12 << 20, 1_000), Tier::Reduced); assert_eq!(tier(15 << 20, 15 << 20), Tier::Reduced); }
#[test] fn tiers_do_not_flap_near_the_limit() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-ui-editor --test a11y --test large_file`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies.
- [ ] **Step 4:** Run `cargo test -p hacivat-ui-editor --features scene-check`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(editor): screen-reader window with stable run ids built only for assistive technology, polite announcements, T0/T1 large-file tiers`.

**Review:** stable node ids; zero cost when inactive; the 80 % rule; manual VoiceOver check deferred to the gate.

### Task 30: Results tile, cursor strip, output log view (HV-EXEC-07 UI, -11 view; HV-RES-05/-07 strip) (W6, gpui)

**Files:** Modify `crates/app/hacivat-app/src/results/{view,toolbar,strip,output_view}.rs`; create `crates/app/hacivat-app/tests/results_view.rs`; append `hacivat-ui.ftl` block `## m3a-t30-results-view` (strip, banner, log toolbar, empty and hint states; copy from the mock).

**Interfaces:**
- Consumes: Task 26's model and adapter, Task 20's `GridView` and `VirtualList`, Task 21's `Toolbar`, Task 23's `SessionLog`.
- Produces: the body of `results::view::build`, the `results_view` hook that Task 1b registers in `results::install` (P12):
  - the tile shows the `ConsoleResults` of the **focused pane's active console** (R-M3-41); a console that never ran keeps the **unchanged M1 empty state**;
  - a `TabStrip` with Output first (unread dot when a line arrives while another tab is active) and one tab per result (production results carry the 7 px swatch; staging the hollow swatch and "(staging)");
  - a minimal toolbar on `Toolbar`: Copy, and rows · timing in mono `text-2` on the right;
  - one cached `GridView` per result tab; the zero-row (header only, "0 rows"), zero-column (the Output tab is shown), error and > 2,000-column (`hv-grid-too-many-columns-hint`) states;
  - the **cursor strip** (26 px, `raised`, `caption` `text-2`) per `Completeness`: Streaming (amber marker, "Fetching… N rows"), Suspended ("N+ rows", "Cursor open — holding a database snapshot", Close cursor, Fetch more, Fetch all), Complete (green check, "N rows"), Closed with its reason and Re-run (`ResultsEvent::Rerun`), Cancelled, the memory banner (Fetch more anyway → `raise_cap`, Close cursor; R-M3-33), "N more result sets not shown" (R-M3-25), selection dimensions on the right; counts locale-formatted;
  - the **output log view** on `VirtualList` (21 px rows; ink severity glyphs; the Fluent sentence; a mono code chip; the verbatim server text in its own field; a click on an error line emits `ResultsEvent::GoToSource`); toolbar Clear, Copy (`copy_invariant`), Scroll to end; "n events not shown";
  - commands `results.copy`, `results.fetch_more`, `results.fetch_all`, `results.close_cursor`, `results.rerun` and the `grid.*` ids (dispatching to the focused `GridView`).

- [ ] **Step 1: Write the failing tests.**
```rust
#[gpui::test] fn empty_tile_is_the_m1_state_until_a_run(cx: &mut TestAppContext) {}
#[gpui::test] fn switching_tabs_swaps_the_strip_and_toolbar(cx: &mut TestAppContext) {}
#[gpui::test] fn strip_text_per_completeness(cx: &mut TestAppContext) { for (c, id) in strip_cases() { assert_eq!(strip_id(cx, c), id); } }
#[gpui::test] fn memory_banner_actions_reach_the_source(cx: &mut TestAppContext) {}
#[gpui::test] fn two_thousand_columns_render_with_the_hint(cx: &mut TestAppContext) {}
#[gpui::test] fn error_line_keeps_server_text_out_of_the_title(cx: &mut TestAppContext) {}
#[gpui::test] fn log_copy_is_iso_and_ascii_under_tr(cx: &mut TestAppContext) {}
#[gpui::test] fn ten_thousand_log_lines_render_at_constant_cost(cx: &mut TestAppContext) {}
#[gpui::test] fn clicking_an_error_line_emits_go_to_source(cx: &mut TestAppContext) {}
#[cfg(feature = "scene-check")] #[gpui::test] fn raised_ink_on_the_strip_and_toolbar(cx: &mut TestAppContext) {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-app --features scene-check --test results_view`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies; expose the shots fixtures (`#[cfg(feature = "shots")]`) for surfaces 9–12.
- [ ] **Step 4:** Run `cargo test -p hacivat-app --features scene-check,pseudo` and `cargo xtask l10n check`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(app): results tile with Output and result tabs, cursor strip and memory banner, output log view`.

**Review:** the Overflow Rule at 960 in `tr-XT`; follow-the-tab consistency; no M3b or M6 affordance leaking in (sort, find, Export, Edit).

### Task 31: Typed-execution conformance over the Docker matrix (M3a exit: "the Docker matrix for typed execution on all three engines") (W6)

**Files:** Create `crates/core/hacivat-exec/tests/docker_m3a_matrix.rs`; append helpers to `crates/core/hacivat-exec/tests/common/mod.rs` (block `// ## m3a-t31-matrix`); append ADR 0006 `## m3a-t31 — conformance`. The matrix lives in `hacivat-exec`, which depends on the session crate, so it can drive `ExecService`; `hacivat-exec` is one of `docker-pr`'s crates (P6).

**Interfaces:**
- Consumes: Tasks 12, 14, 15, 18, 22, 23 (through `ExecService` on real sessions).
- Produces: a matrix over `targets(Some(engine))` for PostgreSQL (pg14–18 and pg17-tr on `full`, pg17 on `pr`), MySQL (mysql80, mysql84), MariaDB (mariadb106, 1011, 114) and SQLite (files), each test failing when the active set has no target for its engine (M2b ruling).

- [ ] **Step 1: Write the tests.** They are RED until W5 is merged.
```rust
#[tokio::test] #[ignore = "docker"] async fn typed_golden_tables_per_engine_and_version() {}
#[tokio::test] #[ignore = "docker"] async fn select_streams_suspends_fetches_more_and_closes() {}
#[tokio::test] #[ignore = "docker"] async fn stop_fetching_keeps_rows_and_frees_the_session() {}
#[tokio::test] #[ignore = "docker"] async fn cancel_within_one_second_on_every_engine() {}
#[tokio::test] #[ignore = "docker"] async fn read_only_bypass_rows_blocked_through_run_without_classifier() {}   // M2 exit on the typed path
#[tokio::test] #[ignore = "docker"] async fn production_console_rejects_insert_on_every_engine() {}
#[tokio::test] #[ignore = "docker"] async fn turkish_collation_text_round_trips_on_pg17_tr() {}
#[tokio::test] #[ignore = "docker"] async fn incomplete_cursor_leaves_no_transaction_in_pg_stat_activity() {}
#[tokio::test] #[ignore = "docker"] async fn dml_and_ddl_report_update_counts_on_a_dev_console() {}   // E1: rows_affected and the command tag per engine (P27)
```
- [ ] **Step 2:** Run `cargo test -p hacivat-exec --no-run`, then `<main>/target/docker/run-docker-tests.sh --set full -p hacivat-exec --test docker_m3a_matrix`. Expected: FAIL before W5 merges; PASS after. A failure is triaged to the owning task as a fix round, never patched here.
- [ ] **Step 3: Implement** the helpers (idempotent fixtures; injected 2 s idle caps).
- [ ] **Step 4:** Run Step 2's command three times (flake check). Expected: three green runs. Write the matrix table (engine × version × case) into ADR 0006.
- [ ] **Step 5: Commit** `test(exec): M3a typed-execution conformance over the Docker matrix`.

**Review:** security-sensitive. Probes: a target silently skipped; a bypass row that "passes" because nothing ran; side channels checked after each row (M2b method).

### Task 32: Console host: binding, editor tab, toolbar, production decor, autosave, editor status segment, Edit menu (W7, gpui)

**Files:** Modify `crates/app/hacivat-app/src/console/{registry,view,toolbar,decor,autosave,status,commands}.rs` (Task 1b's stubs; `console/mod.rs` stays Task 1b's, P12); the "New console" entry in `crates/app/hacivat-app/src/connections/home.rs:20-32` (named lines); create `crates/app/hacivat-app/tests/console.rs`; append `hacivat-ui.ftl` block `## m3a-t32-console`.

**Interfaces:**
- Consumes: Task 19's `EditorView`, Task 24's `SqlLanguageService`, Task 25's commands, Tasks 27–29, Task 21's `Toolbar`, Task 18's sessions, Task 23's `console_ux`, Task 1b's hooks, `Consoles` registry and `console::editor_segment` stub.
- Produces:
  - **Binding (R-M3-37):** the `console_view` hook body. An unbound console renders exactly today's `editor_empty` element. The first successful connect binds the focused unbound console; `workspace.new_console_tab` binds the new console to the capsule's connected profile; a restored console keeps its saved `profile` (the `Option<ProfileId>` that `ConsoleViewFn` receives, P12). Console 1 of a profile uses the default session (`ClientId::DEFAULT`, B4); others open `SessionManager::open(SessionSpec { kind: Console(ClientId(n)), read_only: effective_read_only(..), department: Console }, ix)`; while that future is pending the toolbar shows "Waiting for a free connection to <name> (in_use of max in use)" from `budget_use()` with Cancel (R-M3-43).
  - **View:** an `EditorView` with the `SqlLanguageService`, embedded as `AnyView::cached`; `exec_ui::attach(console, &editor, ..)` is called once at creation.
  - **Toolbar** (on `Toolbar`): Run / Stop (`editor.run` / `exec.cancel`), the connection label (mono, env mark), the read-only lock chip, the **production read-only notice** (R-M3-6), the waiting state.
  - **Decor:** the `tab_decor` hook body — the title is the connection name (`TextSource::Data`) with ` (n)` from the second console of a profile; the env swatch; `hairline` for production.
  - **Autosave:** `<data>/consoles/<profile-uuid>/<n>.sql` (`LocalPaths::consoles_dir`, `paths.rs:80`) 1 s after the last edit and at quit, written on the core runtime (never awaited on the UI thread), skipped when `content_hash` is unchanged; restored at launch with the tier chosen by size. A file over 20 MB is refused with a notice and left on disk (R-M3-49, P26). Path assertions compare `Path`s (P15).
  - **Editor status segment:** an entity of its own (R-M3-11) showing "Ln 14, Col 49", the selected-character count and "Reduced mode"; notified only by the editor. It is the body of Task 1b's `console::editor_segment`, which Task 34 places.
  - **Commands:** every `hacivat_ui_editor::commands()` entry registered as an `ActionDef` (context `Editor`, `enabled` = an editor is focused, `run` = `EditorView::run_command` on the `FocusedEditor`); the focus handle registered with `register_focus_context(cx, handle, EDITOR_CONTEXT)` (`shell/hooks.rs:113`, P20); `FocusedEditor` and `FocusedInput` kept current; the Edit menu then shows them.

- [ ] **Step 1: Write the failing tests.**
```rust
#[gpui::test] fn keyboard_only_walkthrough(cx: &mut TestAppContext) { /* cmd-t → type "SELECT 1;" → shift-home → cmd-enter emits ExecuteRequested with the selection → escape, ctrl-m, tab leaves the editor */ }
#[gpui::test] fn unbound_console_shows_the_m2_home_and_binds_on_first_connect(cx: &mut TestAppContext) {}
#[gpui::test] fn second_console_opens_its_own_session_and_waits_when_the_budget_is_full(cx: &mut TestAppContext) {}
#[gpui::test] fn production_console_shows_the_notice_and_the_hairline(cx: &mut TestAppContext) {}
#[gpui::test] fn editor_commands_are_disabled_without_a_focused_editor(cx: &mut TestAppContext) {}
#[gpui::test] fn text_input_copy_still_works_with_the_edit_menu(cx: &mut TestAppContext) {}
#[gpui::test] fn autosave_writes_off_the_ui_thread_and_restores(cx: &mut TestAppContext) {}
#[gpui::test] fn restored_console_over_20_mb_is_refused_and_kept_on_disk(cx: &mut TestAppContext) {}   // R-M3-49, P26
#[gpui::test] fn typing_notifies_the_status_segment_but_not_the_grid(cx: &mut TestAppContext) { let (vcx, grid_renders) = console_with_results(cx); type_keys(&vcx, "abc"); assert_eq!(grid_renders.get(), 0); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-app --features scene-check --test console`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies; expose shots fixtures for surfaces 1, 2, 6 (editor segment) and 7.
- [ ] **Step 4:** Run `cargo test -p hacivat-app --features scene-check,pseudo` (M2 suites unchanged). Expected: PASS.
- [ ] **Step 5: Commit** `feat(app): consoles bound to connections with an editor tab, toolbar, production decor and read-only notice, autosave and an editor status segment`.

**Review:** the UI thread never awaits I/O; every console session goes through `effective_read_only`; M2 shots unchanged for unbound consoles; no workspace-wide re-render per keystroke.

### Task 33: Execution glue in the app: resolve → chooser → run → marks; cancel; transactions; close and quit prompts (W7, gpui)

**Files:** Modify `crates/app/hacivat-app/src/exec_ui/{mod,orchestrator,chooser,marks,cancel,tx,close_prompt}.rs`; `crates/ui/hacivat-ui-core/src/platform/memory.rs` (`physical_memory_bytes`, named function; `unsafe` only here with `// SAFETY:` per block); the debug demo removal (R-M3-47: the `## m2b-t22-live` block of `commands/debug.rs` and `connections/mod.rs:54-55`); create `crates/app/hacivat-app/tests/exec_ui.rs`; append `hacivat-ui.ftl` block `## m3a-t33-exec-ui` (chooser, end-of-line statuses, transaction segment and prompt copy).

**Interfaces:**
- Consumes: Task 23's `resolve_units`, `ExecService`, `sqlctx`, `console_ux`; Task 24's `doc_snapshot`; Task 19's marks API and `caret_bounds`; Task 26's `ConsoleResults`; Task 32's consoles (the tests use Task 1b's `register_test_console`, a real editor over a FakeDriver session, so they need no Task 32 code, P12).
- Produces: `exec_ui::attach` and its handlers:
  - **SQL context (R-M3-39):** at attach and after any unit whose first keyword is `SET`, run `sqlctx::probe_sql` as an internal `Request::Exec` (`ExecOpts::values`) and call `set_sql_context`.
  - **Run:** on `ExecuteRequested`, take `doc_snapshot(doc, version)` or build `SqlDocument` from the request's snapshot on the background executor; `resolve_units` there too; then on the foreground: `Units` → `ExecService::run`; `Ask` → the **chooser** (R-M3-36: `PopupMenu` at `caret_bounds()` with `set_access_keys(false)` (P15), hover → `set_preview_override`, Enter → `units_for_candidate` → run, Escape → nothing; the chosen kind remembered per console); `NothingToRun` → the status text; `Uncertain` → the chooser's "boundary uncertain" state, **nothing sent** (R-M3-24); `UnsupportedClientCommand` → a log line. A disconnected bound console starts that profile's connect and runs nothing (R-M3-45).
  - **Marks:** `add_exec_marks(snapshot.version, units)`; `RunStatus` → `ExecMarkState` with Fluent labels ("executed · 42 ms · 100 rows", "12 rows affected"); the result tabs from `RunTicket.results` into the console's `ConsoleResults`.
  - **Cancel:** `exec.cancel` → `RequestTicket::cancel` (stop fetching on a streaming or suspended result); `ResultsEvent::{Rerun, GoToSource}` handled (`select_range`).
  - **Transactions (R-M3-27):** `tx.commit` and `tx.rollback` palette commands (`Request::Tx`), logging `exec-tx-committed`/`-rolled-back`; the `tab_close_guard` and `quit_guard` bodies: when a console's session has `TxOwner::User` and status ≠ Idle, open the **close-with-open-transaction prompt** (420 px alertdialog; Commit / Roll back / **Cancel focused**; Enter acts on key-down; any close answers Cancel) and proceed only after Commit or Roll back succeeds. While the status is `Failed` (an error or a server cancel inside the transaction, R-M3-16 amended) the prompt offers only Roll back and **Cancel (focused)** (P16). Closing a console releases its cursor and closes its non-default session.
  - **Memory:** `physical_memory_bytes()` (macOS `libc::sysctlbyname("hw.memsize")`, Windows `GlobalMemoryStatusEx` with `windows-sys` `Win32_System_SystemInformation`, Linux `/proc/meminfo` `MemTotal`; the dependencies are Task 1b's rows, P6), and the app's `ExecService` built on `MemoryBudget::from_ram(..)` (R-M3-19).

- [ ] **Step 1: Write the failing tests** (gpui, FakeDriver sessions).
```rust
#[gpui::test] fn one_candidate_runs_without_a_chooser(cx: &mut TestAppContext) {}
#[gpui::test] fn selection_of_two_statements_runs_two_units(cx: &mut TestAppContext) {}
#[gpui::test] fn ask_opens_the_chooser_hover_previews_and_enter_runs_the_preselected(cx: &mut TestAppContext) {}
#[gpui::test] fn chooser_remembers_the_kind_per_console(cx: &mut TestAppContext) {}
#[gpui::test] fn ambiguous_resolution_sends_nothing(cx: &mut TestAppContext) { let (vcx, f) = console_mysql(cx, "SELECT 1 /*! ; */ ;"); run_at(&vcx, 0); assert!(f.typed_log().is_empty()); assert!(chooser_shows_uncertain(&vcx)); }
#[gpui::test] fn read_only_denial_marks_the_gutter_failed_with_its_reason(cx: &mut TestAppContext) {}
#[gpui::test] fn cancel_reaches_the_session_and_stop_fetching_keeps_rows(cx: &mut TestAppContext) {}
#[gpui::test] fn closing_a_console_releases_its_cursor(cx: &mut TestAppContext) {}
#[gpui::test] fn closing_a_console_with_an_open_transaction_asks(cx: &mut TestAppContext) { /* Cancel focused; Roll back → closes; Cancel → stays */ }
#[gpui::test] fn quit_with_an_open_transaction_asks_and_cancel_keeps_the_app(cx: &mut TestAppContext) {}
#[gpui::test] fn close_prompt_on_a_failed_transaction_offers_roll_back_only(cx: &mut TestAppContext) {}   // P16
#[gpui::test] fn chooser_assigns_no_access_keys(cx: &mut TestAppContext) {}   // P15: the labels are SQL; typing a letter runs nothing
#[gpui::test] fn set_statement_refreshes_the_sql_context(cx: &mut TestAppContext) {}
#[gpui::test] fn disconnected_console_run_starts_the_connect(cx: &mut TestAppContext) {}
#[gpui::test] fn the_ui_thread_awaits_only_tokio_sync_receivers(cx: &mut TestAppContext) {}
#[test] fn physical_memory_is_known_on_this_platform() { assert!(physical_memory_bytes().unwrap_or(0) > 1 << 30); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-app --features scene-check --test exec_ui`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies; expose shots fixtures for surfaces 3–5 and 13–14.
- [ ] **Step 4:** Run `cargo test -p hacivat-app --features scene-check,pseudo`, `cargo test -p hacivat-ui-core --features test-support,scene-check`, `cargo xtask l10n check`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(app): run at caret and selection with the D-011 chooser, gutter marks, cancel and stop fetching, transaction commands and the open-transaction prompt; RAM-based result budget`.

**Review:** security-sensitive (what runs, prompts). Probes: anything sent after `Uncertain`; the chooser's Enter running a candidate other than the highlighted one; a close or quit that discards a user transaction; a Commit offered on a `Failed` transaction; an access key on a chooser item; a prompt answered by window close; the `unsafe` blocks' `SAFETY` comments.

### Task 34: Status bar and capsule follow the console (DESIGN.md "Status Follows The Tab") (W7, gpui)

**Files:** Modify `segments()` in `crates/app/hacivat-app/src/connections/status.rs` and `current_profile()` in `crates/app/hacivat-app/src/connections/capsule.rs` (named functions); create `crates/app/hacivat-app/tests/status_follow.rs`.

**Interfaces:**
- Consumes: Task 26's `active_result_context`, Task 32's console registry and the editor segment behind Task 1b's `console::editor_segment` (the tests use `register_test_console`, P12), Task 23's `console_ux`.
- Produces: `capsule::current_profile` returns the focused console's profile when a bound console has focus, else the M2 rule (explorer selection, else the last attempt; R19). `segments()` shows, in collapse order at 960 (mock): the env and access segments of the **active result tab** (`ResultContext`), else the console's; the transaction segment "Transaction open · N statements" only for `TxOwner::User` (R-M3-27), and its `Failed` form while that transaction is `Failed` (R-M3-16 amended, P16); rows · timing; "Cursor open" (a button that closes the cursor); then the console's editor segment entity (`console::editor_segment`).

- [ ] **Step 1: Write the failing tests.**
```rust
#[gpui::test] fn focusing_a_production_or_staging_console_swaps_the_capsule(cx: &mut TestAppContext) {}
#[gpui::test] fn switching_result_tabs_swaps_the_env_segment(cx: &mut TestAppContext) {}
#[gpui::test] fn transaction_segment_only_for_user_transactions(cx: &mut TestAppContext) {}
#[gpui::test] fn failed_transaction_segment_says_so(cx: &mut TestAppContext) {}   // P16
#[gpui::test] fn cursor_open_segment_closes_the_cursor(cx: &mut TestAppContext) {}
#[gpui::test] fn without_a_console_the_m2_behaviour_is_unchanged(cx: &mut TestAppContext) {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-app --features scene-check --test status_follow`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies.
- [ ] **Step 4:** Run `cargo test -p hacivat-app --features scene-check,pseudo` (the `connections_live*.rs` suites untouched and green). Expected: PASS.
- [ ] **Step 5: Commit** `feat(app): capsule follows the focused console; status segments follow the active result tab, with transaction and cursor segments`.

**Review:** M2 regressions (`current_profile` has many callers); the 960 collapse order (M2b T22 rulings).

### Task 35: Shots and cross-surface tests (W8, gpui)

**Files:** Modify `crates/app/hacivat-app/tests/shots.rs` (append the M3a surfaces after the existing 78; `--shard k/n`; grid text checks); create `crates/app/hacivat-app/tests/cross_surface_m3a.rs`.

**Interfaces:**
- Consumes: every M3a task's shots fixtures; `ResultStore::from_rows` (no core runtime for grid and results surfaces, R-M3-15).
- Produces: shots of surfaces 1–14 at 1280 × 800 and 960 × 600 × `en`, `tr`, `en-XA`, `tr-XT` × dark and light; harness checks extended: a grid surface without its specific marker fails, a clipped grid cell fails, a pseudo-bracketed header fails (headers are data); cross-surface flows: run → rows → copy; production console → INSERT → read-only denial in the log and a Failed mark; chooser → hover → run; stop fetching → Cancelled strip; close with an open transaction → Roll back.

- [ ] **Step 1:** Write the cross-surface tests; run `cargo test -p hacivat-app --features scene-check --test cross_surface_m3a`. Expected: FAIL until the fixtures exist, then PASS.
- [ ] **Step 2:** Run `cargo test -p hacivat-app --features shots,scene-check,pseudo --test shots` (sharded where useful). Expected: 0 violations; the harness self-checks pass.
- [ ] **Step 3:** Run `cargo xtask shot-diff --baseline <main>/.superpowers/sdd/2026-11-14-m3-walking-skeleton/baseline-m2b --candidate target/shots` (P17). Expected: every M1/M2 shot unchanged at zero tolerance.
- [ ] **Step 4:** **Look at the PNGs** of every new surface at 960 in `tr-XT` and dark/light; record the per-shot time and the total harness time in the report.
- [ ] **Step 5: Commit** `test(app): M3a shots and cross-surface flows`.

**Review:** opus (design and a11y). Probes: can every new surface be completed from the keyboard alone; any Turkish overflow at 960; any marker satisfied by an unrelated label.

### Task 36: Real-app benches: B3 and B1-real (R-M3-11, R-M3-12, R-M3-40) (W8, gpui)

**Files:** Modify Task 1b's stubs `crates/app/hacivat-app/src/bench/mod.rs` and `crates/app/hacivat-app/examples/{b3,b1_real}.rs` (P7); create `src/bench/{b3,b1_real}.rs` (feature `bench`) and `docs/benchmarks/m3a-ime-checklist.md` (the M0 rows retargeted to the real editor, plus Windows TR-Q AltGr `~`+Space, UI-EDIT-12 test 3); append the `ci.yml` bench block `# ## m3a-t36-bench`.

**Interfaces:**
- Consumes: the whole app; `hacivat_ui_core::frame_loop::{bench_dir, stamp_run, record_frames}`, `hacivat_ui_core::frames::{FrameRecorder, verdict}` (the app has no direct `hacivat-bench` dependency, P28); the B1 fling of `spikes/spike-grid/src/bench.rs:16-28` and the B3 method of `spikes/spike-editor/src/bench.rs` (keystrokes replayed at 60 Hz through `Window::dispatch_keystroke`, the latency probe at handler entry, `record_after_present`).
- Produces:
  - `b3` modes on the `cargo xtask gen-sql --lines 50000 --long-line 100000` fixture: `typing`, `typing-long-line`, `scroll`, `open` (first paint and the first full analysis of a 2 MB file), `typing-with-results` (a 1M-row FakeDriver result open in the grid beside the editor), `ime-bounds`. Keys pass the app's dispatcher. JSON through `bench_dir`/`stamp_run`, read by `cargo xtask bench-report`.
  - `b1_real`: a real window; a temporary `LocalPaths`; a `pg17` profile through `hacivat_testkit::require_target` (it answers `None` unless `HACIVAT_DOCKER_SET` is set: the example's usage line says to run it with `HACIVAT_DOCKER_SET=pr`, and it exits 2 naming the variable when the target is missing, P28); explorer hidden, editor tile at `MIN_TILE_PX` (`shell/tiles.rs:40`), inspector closed; 1440 × 900 at 2× and a second run at 2560 × 1440 at 1×; **format locale `tr-TR`**; the dataset `SELECT … FROM generate_series(1, 1000000)` with 50 columns cycling int8 / text (md5 mixed with a 16-word Turkish array) / numeric(14,2) / timestamptz; **Fetch all** and **Fetch more anyway** until `Complete`; record load time, store bytes and RSS; then the M0 fling (V0 = 12,000 px/s, τ = 0.325 s, refling every 1.5 s, 20 s, 750 ms warm-up) with `FrameRecorder("b1-real-fling")`; the verdict from `hacivat_ui_core::frames::verdict` (≥ 55 fps macOS/Windows, ≥ 45 Linux, no gap > 100 ms); paint p99 reported against 6 ms and 3 ms, not gating. `--engine sqlite` builds the same shape with a recursive CTE into a temp file (the CI regression signal). A third, non-gating "streaming" run starts the fling after the first 10,000 rows.
  - CI `bench` job: build `hacivat-app --features bench --examples`, run `b3 --bench typing` and `b1_real --engine sqlite` under `xvfb-run` (relative signal only, L2).

- [ ] **Step 1:** Implement the harnesses (they are benches; their checks are the run's completeness: expected sample counts, `inactive_keys = 0`, the window frontmost, the dataset `Complete` before the fling).
- [ ] **Step 2:** Run `cargo run --release -p hacivat-app --features bench --example b3 -- --file target/fixtures/b3-50k.sql --bench typing` and `--example b1_real -- --engine sqlite` locally. Expected: JSON written; verdict printed.
- [ ] **Step 3: Commit** `bench(app): B3 and B1-real in the real app (typing with a 1M-row grid open; 1M × 50 from PG in tr-TR); M3a IME checklist`.

**Review:** the probe point stays at handler entry (addendum finding 6); `inactive_keys = 0` (finding 7); the dataset is really fetched before the fling; nothing in the bench path changes product behaviour (feature-gated).

### Task 37: M3a gate (W9)

**Files:** Create `docs/benchmarks/m3a.md` and `docs/benchmarks/po-demo-m3a.md`; `design/baselines/m3a/macos/*.png` after the PO signs off.

**Gate housekeeping (controller ruling, T1b review I6):** delete the `M3A_PENDING` list in `crates/app/hacivat-app/tests/commands.rs` (it must be empty: every M3a command registered and bound), and run the keymap tests; a non-empty list fails the gate.

**M3a exit criteria.** Each is PASS or FAIL in `m3a.md`, with evidence:

| # | Criterion | Evidence |
|---|---|---|
| E1 | A developer runs typed SELECTs on PostgreSQL, MySQL/MariaDB and SQLite and browses the rows; DML and DDL run on non-production consoles | Task 31 matrix (`dml_and_ddl_report_update_counts_on_a_dev_console` for DML and DDL, P27); demo steps 2–6 |
| E2 | **B1-real:** 1M rows × 50 columns from PostgreSQL in the `tr-TR` format locale, fling ≥ 55 fps on macOS (Windows/Linux when hosts exist), no blank frame > 100 ms; paint p99, load time, store bytes and RSS recorded | Task 36 `b1_real` on the PO's Mac (two resolutions) |
| E3 | **B3 in the real app:** typing ≤ 16 ms p95 on the 50k-line file, also on the 100k-character line and with a 1M-row grid open; open 2 MB first paint ≤ 150 ms and full analysis ≤ 600 ms | Task 36 `b3` modes on the PO's Mac; the component budgets through the perf lines (P9, P27) |
| E4 | **D-011 goldens:** CARET/SEL/CHOOSE green with the ID-coverage meta-test; the preview/execution agreement property | Tasks 13, 24 |
| E5 | **Docker matrix for typed execution** on all three engines: `docker-pr` green on the PR, `docker-full` green (manual dispatch) | Task 31; CI links |
| E6 | Read-only on the typed path: SAFE-06 server-blocked rows blocked with the classifier off through `Run`; production consoles cannot write on any engine | Tasks 22, 31 |
| E7 | **macOS IME matrix re-run** on the real editor (Turkish Q and F, dead keys, Japanese and Chinese smoke), plus the scripted traces green; Windows/Linux rows listed as open | `m3a-ime-checklist.md`; Tasks 10, 27 |
| E8 | **Shots: 0 violations** for every M3a surface; every M1/M2 shot unchanged at zero tolerance; the PO signs off the M3a surfaces and Turkish strings | Task 35; PO |
| E9 | libpg_query builds on macOS, Windows and Linux CI and in the universal macOS bundle; `cargo deny` green with the PostgreSQL clarify; `THIRD_PARTY_NOTICES.md` in every bundle; release size delta ≤ +10 MiB per architecture | Task 1a CI runs; ADR 0005 |
| E10 | Gates green: clippy `-D warnings`, the full test set (the perf lines included), `check-deps` (with the four new rules), `l10n check`, `tokens --check`, `gen-sql-tables --check`, `cargo deny`, `cargo tree -i aws-lc-rs` empty | Step 2 output |
| E11 | Budgets: first frame ≤ 500 ms and idle RSS ≤ 200 MB with no connection; idle RSS with one connection and an open console recorded | `--smoke` ×3 |
| E12 | Cancel within 1 s on every engine on the typed path; stop fetching keeps rows; incomplete cursors leave no transaction open | Tasks 18, 31 |
| E13 | The TestScheduler teardown deadlock does not recur: the 50-cycle stress test and every shots run green | Task 1a test; Task 35 |
| E14 | A screen reader (VoiceOver) reads the editor by line and announces the selection; the grid is announced as a grid with its title (manual, non-gating, L4 gates at M9) | demo step 12 |

- [ ] **Step 1:** Make sure W8 is merged on `m3a`. In `<main>/target`, clean every workspace member (the shared target switched paths across the waves).
- [ ] **Step 2:** Run:
  ```
  cargo clippy --workspace --all-targets -- -D warnings \
    && <the full test set, every line, the perf lines included> \
    && cargo xtask shot-diff --baseline <main>/.superpowers/sdd/2026-11-14-m3-walking-skeleton/baseline-m2b --candidate target/shots \
    && cargo xtask check-deps && cargo xtask l10n check && cargo xtask tokens --check \
    && cargo run -p xtask -- gen-sql-tables --check \
    && cargo deny --all-features check licenses bans sources \
    && test -z "$(cargo tree -i aws-lc-rs --workspace 2>/dev/null)"
  ```
  Expected: all green. The controller pushes `m3a` and records the CI runs (`build-test` ×3, `docker-pr`, a manual `docker-full`, `nightly-package`, `pseudo-screens`, `bench`).
- [ ] **Step 3:** On the PO's Mac: `cargo run -p hacivat --release -- --smoke` ×3; `b3` all modes; `b1_real` at both resolutions. Write the numbers with the machine, build hash and raw captures.
- [ ] **Step 4: Write `po-demo-m3a.md`.** The PO runs it with the Docker stack up; the gate agent pre-runs it and records results:
  1. Launch; the console shows the M2 home; connect `pg17`; the console binds (title in mono).
  2. Type a three-statement script with Turkish identifiers; watch the highlight and the preview band follow the caret; ⌘↵ in the middle statement opens the chooser; hover shows the range; Enter runs.
  3. Select two statements; ⌘↵ runs both: two result tabs, gutter marks with "executed · … ms · … rows".
  4. `SELECT * FROM generate_series(1, 1000000)`: rows arrive; the strip says "Cursor open"; scroll fetches more; Close cursor; run `SELECT 1`: the cursor is released first.
  5. Stop a long query with ⌘. within 1 s; the rows so far stay.
  6. Copy a 3 × 5 block and paste it into a text editor: raw values, NULL as empty.
  7. A production-tagged profile: the hairline, the swatch, the read-only notice; `INSERT …` is denied and logged.
  8. `BEGIN; UPDATE …;` on a dev console, then close the tab: the prompt (Cancel focused); Roll back closes it.
  9. MySQL 8.4 and SQLite: the same SELECT flows; MySQL `CALL` with several sets shows the first and the note.
  10. Switch EN ↔ TR on every new surface: no English leaks; numbers in the grid follow the region (`1.234,56` with region Türkiye).
  11. Quit and relaunch: the console text is restored.
  12. VoiceOver on: the editor reads by line; the grid announces itself.
- [ ] **Step 5:** Linux and Windows: record the CI results; if the PO provides hosts, run demo steps 2, 4 and 10 and the IME rows there; otherwise list them as open items.
- [ ] **Step 6:** The PO reviews the shots and the Turkish strings; after sign-off, copy the macOS PNGs to `design/baselines/m3a/macos/`.
- [ ] **Step 7: Write `m3a.md`:** the E1–E14 table; the rulings R-M3-22..49 and the pre-flight rulings P1–P28 for PO confirmation (especially R-M3-22 off-thread lexing, R-M3-23 cursor eligibility and rollback, R-M3-24 ambiguity, R-M3-27 the transaction prompt, R-M3-29 the production mechanism, R-M3-37 the unbound console, P16's psql semantics for a cancel inside a user transaction, P26's rollback of what read-head functions wrote); the deferred list with owners; the open items (Windows/Linux IME; NVDA; the M3b plan to be written now).
- [ ] **Step 8: Commit** `docs(m3a): gate record, PO demo checklist, approved M3a baselines`.

---

## Self-review notes

- **Spec coverage (§6 M3, as split).** Every M3a item of the SPLIT DECISION has an owner:

  | SPLIT DECISION item (M3a) | Tasks |
  |---|---|
  | Interface commit, libpg_query proof on 3 OSes, fork scheduler patch | T1a (proof, fork patch), T1b (interface commit) |
  | Mocks | T2 |
  | Keywords, lexer (chunked interface), splitter + `SqlDocument` | T3, T4, T8 |
  | Caret → units chooser + D-011 goldens | T13, T23, T33 |
  | `sql-oracle-pg` | T1a (proof), T9, T23 |
  | Single-cursor editor: IME, clipboard, lexical highlight, undo, core commands | T5, T10, T11, T19, T24, T25, T27 |
  | Gutter marks + Run preview; a11y baseline; large-file tiers 1–2; console autosave | T28, T29, T32 |
  | B3 in the real app | T36, T37 |
  | Typed single-statement execution on PG/MySQL/SQLite (vendored tokio-postgres portals) | T1b, T12, T14, T15, T18 |
  | Result store with caps, cursors, idle bounds, stop-fetching; typed RO guard | T7, T18, T22 |
  | Grid core + element with fast-scroll; all cell types with L3 numbers; adapter | T6, T16, T17, T20, T26 |
  | Result tabs, raw-TSV copy, results tile, cursor strip, minimal output log | T23, T26, T30 |
  | Consoles bound to connections; production hairline; capsule/status follow the console; production consoles read-only | T1b, T18, T32, T34 |
  | Gate: B1-real, B3, D-011 goldens, Docker matrix | T31, T36, T37 |

  The M3b half and later work are in "Deferred work", each with a reason.
- **Digest conflicts resolved** (ledger first, then safer/simpler, logged as rulings): ADR numbering (R-M3-28); `TextEdit` shape and lexer class naming (R-M3-42); the cancel chord and Escape (R-M3-38); the unbound console (R-M3-37); the bench target shape (R-M3-40); the raw-text owner (`valfmt`, Task 16, not `hacivat-values::rawfmt`); B3 vs incremental relex (R-M3-22); cursor suspension without a classifier (R-M3-23); the store's publication throttle (R-M3-48).
- **Type consistency.** Names used across tasks as defined in Task 1b: `DocSnapshot::{editor_split, execution_split}`, `exec_range::resolve(.., SplitMode, ..)`, `head::cursor_eligible`, `ExecUnit.suspendable` → `RunUnit.suspendable` → `QuerySpec.suspendable`; `RunObserver::open_result` → `ResultWriter`; `ResultSource::{ensure_rows, fetch_all, close_cursor, raise_cap}`; `TxOwner::{User, Cursor, Guard}`; `effective_read_only(profile, production_read_only)`; `SessionManager::set_policy`; `ManagerConfig.production_read_only`; `SessionManager::budget_use`; `TypedCaps.idle_bound`; `FetchPolicy.idle_cap`; `LanguageService::exec_preview -> Option<ExecPreview>`; `valfmt::Display.chip: Option<ChipKind>`; `exec_ui::attach`; `ResultsEvent::{Rerun, GoToSource}`; `CloseDecision`; `results::view::build`; `console::editor_segment`; `marks::MarkStore`, `preview::BandState`, `EditorView::last_layout`; `Value::{Bytes, Lossy}(Arc<[u8]>)`, `Value::{Opaque, Error}(Arc<…>)`.
- **Conflict control.** Shared call sites are seams created by Task 1b (Q9, P11, P12): the editor element's calls into `gutter`/`marks`/`preview`/`scrollbar`/`a11y` (filled by T28/T29); `readonly::guarded_query` (T22) called from `run.rs` (T18); `exec_ui::attach` (T33) called by the console host (T32); the shell hooks, `results::view::build` and `console::editor_segment` (T1b) with bodies in T30/T32/T33/T34. Named exceptions: T1a (the `crates/bin/hacivat` smoke line; the `libclang` step inside existing CI jobs), T1b (based on Task 1a's first commit), T8 (the `alg/10` errata), T12 (`third_party/tokio-postgres`), T32 (`connections/home.rs` entry), T33 (`platform/memory.rs`, the debug demo), T34 (two named functions).
- **Proportion.** Code blocks are signatures, test names and assertions. Bodies appear only for values the spec or rulings fix (keyword rules, idle bounds, budgets, the cursor transaction rule).
- **Known risks** (also in the gate report):
  - **The wave count and gpui build pressure.** 18 of the 38 tasks build gpui (T1a and T1b included); RAM and CPU cap them at 3 while the Docker matrix is up (P14); W4–W8 are the long pole. A gpui-free grid model crate would shorten W3, at the cost of a crate the spec does not name.
  - **R-M3-22 and R-M3-23 are new structural calls** that the PO has not seen: highlight lag on very large files, and "incomplete cursors roll back" semantics.
  - **libpg_query on Windows and the universal macOS build** (R-M3-1) are proven only when Task 1a's CI runs; `pg_query` is already vendored (P1), so a failure costs a pre-generated-bindings fix round inside Task 1a.
  - **B1-real in `tr-TR`** may miss the 120 Hz paint budget as M0 did (3.6 ms vs 3 ms); M3a gates on fps and gaps and reports paint p99.
  - **Task 1b's size**: thirteen crates' worth of signatures in one commit; its review checks every later task's "Consumes" line against them (pre-flight Part 4), and the split keeps the slow three-OS proof out of it.

