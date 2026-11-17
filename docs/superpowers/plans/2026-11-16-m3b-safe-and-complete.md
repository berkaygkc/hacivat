# M3b "Safe and complete" Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish M3 on top of what M3a shipped. The classifier (`sql-safety` v1: libpg_query for PostgreSQL, the interim sqlparser path for MySQL/MariaDB/SQLite) and the pre-execution gate with its confirmations land first, so **production consoles become writable behind confirmations** (R-M3-14) and SQL-SAFE-06 passes with the classifier on and off. Around that: parameters and binds, incremental relex, multiple result sets and "Run as one", the error-policy bar, the transaction UI, cancel escalation, the auditor chain with the history store, the full type matrix, the M2 carries; then editor comfort (multi-cursor, folding, find), the rest of the grid (sort, find, go to row, the three views, the value viewer, the context menu, the AccessKit subtree), the DESIGN.md section and the B1-real re-run.

**This plan proposes a split (R-M3-51, "Split check" below): M3b "Safe" (W0–W7, gate T30) and M3c "Complete" (W8–W13, gate T45).** Task numbers run through both halves, so the plan stays valid whichever way the controller rules: if the split is refused, T30 becomes an internal checkpoint (no PO demo) and T45 is the single M3b gate.

**Architecture:**
- **Four streams, as in M3a**, meeting only through the seams that Tasks 1a, 1b and 1c commit:
  - **SQL text and safety** (`crates/sql/*`): new crates `sql-ident` (identifier folding, qualified names) and `sql-safety` (effects, reasons, the policy table, `GateClass`, the PG backend over a generated visitor, the interim sqlparser backend, the orchestrator `classify`); new `sql-split` modules `params`, `templates`, `fingerprint`, `incremental`, `stream`; `sql-lexer::incremental`. `sql-oracle-pg` gains the parse-tree entry the PG backend needs, on its existing serial 16 MiB parser thread.
  - **Execution** (`crates/core/*`): `hacivat-exec` gains `gate`, `params`, `explain_wrap`, `suppress`, `classify_pool`, `audit`, `history/**`, `count`; the session gains bound parameters, per-unit verdicts, multiple result sets, "Run as one", the out-of-band error-policy choice, confirm-in-transaction with the D-014 deadline, Smart mode and the M2 carries; the three drivers gain binds, multi-result streaming and the full type matrix.
  - **Editor** (`crates/ui/hacivat-text`, `hacivat-ui-editor`, `crates/app/hacivat-sql-editor`): incremental analysis in the existing per-document worker (R-M3-22 stays), multi-cursor with IME mirroring, folding over a chunked height map, the find panel, the gutter extras and the line commands.
  - **Grid and workspace** (`crates/ui/hacivat-ui-grid`, `crates/app/hacivat-app`): a permuted `ViewMap` (sort, filter rows), frozen columns, multi-rect selection, find, go to row, transposed/record/text views, the value viewer, the context menu and the AccessKit subtree; the confirmation, confirm-in-transaction, cancel-escalation and parameter dialogs, the error-policy bar, the transaction chip and badge, the idle-transaction toast, the history dialog.
- **Interface first, in three parts** (the M3a precedent, pre-flight Part 4, P-M3b-20). **Task 1a "Manifests and SQL seams"** owns every manifest and `deny.toml` change of M3b and M3c, the SQL types with fail-closed stubs, the check-deps rules and the xtask/fuzz skeletons; its first commit (manifests only) is the base of **Task 1b "UI and app seams"** and **Task 1c "Core seams"**, which run in parallel. Tasks 2a and 2b mock the new surfaces (2a in W0, 2b before W8).
- **The production unlock is one task** (T26), after the classifier, the gate, the confirmation UI and the session's confirm-in-transaction have merged and their Docker proofs are green. Until T26 merges, `effective_read_only` keeps forcing production read-only exactly as M3a ships it (R-M3-6 REVISED, R-M3-29). No interim classifier and no interim gate is ever wired into Run (R-M3-52).

**Tech Stack:**
- Everything from M2a, M2b and M3a; their manifests and rulings stand.
- New: `sqlparser = "=0.64.0"` (Apache-2.0; `default-features = false`, features `std`, `visitor`, `recursive-protection`; only `sql-safety` may depend on it, R-M3-54), `icu_collator` 2.4 (Unicode-3.0, already allowed; ICU4X is in the graph), `serde_json` for the PG coverage walk (workspace dep, dev only in `sql-safety`), `regex` (workspace dep) for the find panel and the grid find in `crates/app` only (the Ui crates take compiled matchers, R-M3-10). `rusqlite` (bundled, workspace) for `history.sqlite`, with FTS5 (proved by Task 1a's `fts5_is_compiled_in`).
- No other crate without a ruling. Exact versions are what Task 1a's manifest table pins; `Cargo.lock` is the record.

**Spec:** `docs/superpowers/specs/2026-11-12-hacivat-v1-design.md`. The plan argues from it; executors read both.

**Execution (PO mandate 2026-11-13/14: roll on, take rulings, never wait on the question list):**
- Subagent-driven, in parallel waves of 3–5 worktree agents. Branch **`m3b`** is cut from the **M3a gate tip** (`m3a` after Task 37 of M3a merges; the controller records the SHA in the ledger). Task branches are `m3b-<slug>` based on `m3b`. No merge to `main` without the PO. If the controller accepts the split, `m3c` is cut from the M3b gate tip and M3c's task branches are `m3c-<slug>`.
- The controller commits this plan to `docs/superpowers/plans/2026-11-16-m3b-safe-and-complete.md` on `m3b`, copies `implementer-rules.md` / `reviewer-rules.md` retargeted to `m3b` (branch names, `target-core/m3b-tNN`, `## m3b-tNN-<slug>` block headers) **before Task 1a is dispatched**, copies this plan's Global Constraints to `global-constraints-m3b.md` in the SDD folder, and keeps the ledger (`progress.md`, new heading `## M3b`) and `~/.claude/ilerleme-hacivat.json` current.
- A pre-flight conflict scan (M3a precedent, `preflight.md`) runs over this plan before Task 1a; its amendments are applied in one pass.
- Every ruling is logged in the ledger with "why" and "cost if wrong" and lands in the gate's PO list.

**Spec reading list** (every task brief cites the parts it needs):
- Decision log: L1, **L3**, L4, **L9** and **L10** (only the classifier vocabulary they fix: operation classes, `unknown` is critical; no MCP work here), L15.
- §2.3 anti-goals; §3.3 budgets; §3.4; **§4 rows** "SQL language", "Execution", "Explain and history", "Result store", "Grid", "Safety and read-only" with the **"Safety model (normative summary)"** and the CVE-2026-85620 lessons, "Production UX", "Sessions and transactions", "Accessibility", "i18n", "Keyboard and command palette"; §5.1 (layout, rules 1–4), §5.4, §5.5, §5.6 (history path); **§6 M3 scope and exit**; §7.1; §10 C3, C10, C20.
- Register (`docs/decisions/open-product-decisions.md`): D-008 (vocabulary only), **D-009**, D-011 (CHOOSE-06), **D-013**, **D-014**, D-015, D-019, **D-035**, **D-036**, D-053, D-058, D-059, **D-061**, D-062, D-063, D-064/L3, D-065, D-066 (read-only), D-071, D-072, D-073, D-074, D-085.
- Algorithms: `alg/10` SQL-LEX-03, SQL-SPLIT-05, SQL-PARAM-01..03; **`alg/13` SQL-SAFE-01..07**; `alg/11` (combine rule); `alg/20` HV-EXEC-03, -04, -06, -07, -08, -09, -10, -11, -12; `alg/21` HV-RES-07 (eviction), -08 (large values), -10 (count); `alg/22` HV-GRID-02 (frozen), -05 (multi-rect), -06, -07, -09, -12, -14, -16; `alg/25` UI-EDIT-05 (multi-cursor), -10 (folding), -12 (mirroring), -14 (metadata), -22 (find, no replace); `alg/26` I18N-09, -11.
- `docs/research/datagrip/13-behavioral-test-corpus.md` §3.1 (seed schema), PARAM rows, UNSAFE-01..10, and `docs/research/verification/01-db-behavior-tests.md` (the SAFE-06 anchors).
- ADR 0004 (guard, sessions; the M2b SAFE-06 run B), 0005, 0006, 0007 with their `m3a-*` sections.
- `DESIGN.md`, `design/m3a-query/{index.html,NOTES.md}` (binding for every M3a surface this plan touches), `design/m2-connect/NOTES.md` (the binding "No extra confirmations" copy off production), `design/m2b-secure/NOTES.md`, and `design/m3b-safe/` once Task 2 lands (binding).
- The M3a plan `docs/superpowers/plans/2026-11-14-m3a-run-a-query.md` (its rulings R-M3-22..49, P1–P28, its Deferred table) and the ledger `<main>/.superpowers/sdd/2026-11-14-m3-walking-skeleton/progress.md` (every controller ruling after the plan; the routings this plan collects in "Carried items"). The digests in `inputs/` are background; where this plan differs, the plan wins.

## Global Constraints

Every task's requirements include this section. `<main>` is `/Users/berkaygokce/Documents/hobby-projects/hacivat`. Line numbers cited in this plan are at the m3a tip **`7e9209f`** (merge worktree `/Users/berkaygokce/Documents/hobby-projects/hacivat-m3a-merge`); the M3a gate (Task 37 of M3a) adds only docs, `docs/benchmarks/*` and the deletion of `M3A_PENDING`, so the cited lines hold at the `m3b` base unless the pre-flight says otherwise.

- **Inherited.** The M3a Global Constraints (`global-constraints.md` in the SDD folder) apply unchanged except where a line below replaces or extends one: the M2b/M1/M2a rules they carry, the GPUI fork pin (rev `ff0d409` on `hacivat/912159c`), toolchain 1.98.1, `unsafe` only in `hacivat-ui-core/src/platform/**`, ASCII-only casing, the Fluent rules, semantic tokens, F31 and F9, secrets, modal layers, key bindings, single writers, the threading rules, the test-timing rules, the cross-platform test rules (`find_label_runs`, `Path` comparisons, CRLF at test time) and the shots-harness rules.
- **Branches.** Feature branch **`m3b`**. Each task works in its own worktree on `m3b-<task-slug>`; its first commands are `git checkout -B m3b-<task-slug> m3b` then `git merge-base --is-ancestor m3b HEAD`; stop and report BLOCKED if either fails. Implementers never push and never merge. **Named exception:** Tasks 1b and 1c start, in parallel, from Task 1a's first commit (the manifests), as M3a's Task 1b did (P-M3b-20).
- **SDD files.** The ledger, `global-constraints-m3b.md` (an exact copy of this section) and the retargeted implementer and reviewer rules live in `<main>/.superpowers/sdd/2026-11-14-m3-walking-skeleton/`. Report files go there as `task-m3b-NN-report.md` (review: `task-m3b-NN-review.md`); if the write is refused, inside the worktree at the same relative path.
- **Commit trailer.** Every commit message ends with exactly the trailer `implementer-rules.md` names (today: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`; the controller updates the rules file if the PO's attribution rule changes, ledger line 140 precedent).
- **Disk and target directories.**
  - One project target `<main>/target` for the controller's merge checks and the gate; the persistent merge target `<main>/target-core/merge` stays the controller's.
  - **Per task:** `CARGO_TARGET_DIR=<main>/target-core/m3b-tNN` (Task 1a: `m3b-t01a`, Task 1b: `m3b-t01b`). The controller runs `cargo clean` in it and deletes it right after the task merges; nobody leaves a `target-*` directory behind (disk-hygiene rule, 2026-11-13).
  - **Admission** (R-M3-50 / P14, restated for this machine: 14 cores, 48 GB RAM): a gpui agent (every `crates/ui/*` crate except `hacivat-text`, every `crates/app/*` crate) starts only with ≥ 25 GiB free, a core agent with ≥ 8 GiB; no dispatch while swap grows. **At most 3 gpui agents while the Docker matrix is up, 2 whenever less than 60 GiB is free, 4 while Docker is down.** `CARGO_BUILD_JOBS=4` (core) / `6` (gpui) whenever 3 or more agents build. `CARGO_INCREMENTAL=0` always.
  - **Before W0 the controller cleans and deletes every `target-core/m3a-*` directory** that still exists (P-M3b-31); free disk was ~65 GiB at the pre-flight, five steps from the 60 GiB line that drops the gpui cap to 2, so the wave plan gives both schedules.
  - **Cold-build budget:** a fresh target that reaches `pg_query` builds libpg_query (~3 min, a few hundred MB); `sqlparser` with `visitor` adds ~1 min. `sql-safety` reaches both, so every task that builds `hacivat-exec`, `hacivat-session` tests or the app pays both.
  - When a shared target switches checkout path, clean every workspace member first. Never delete `<main>/target/docker/`.
- **Dependencies.** Task 1a writes every manifest change M3b and M3c need (its Step 3 table is exhaustive), `deny.toml`, `[workspace] members`/`exclude` (the `fuzz/` workspace) and the `check-deps` rules. **No later task edits a manifest.** Named exceptions are listed in the task that takes them (Task 9: `third_party/pg_query/PATCHES.md` §2 if its stderr ruling needs a build define; Task 13: a fenced `third_party/tokio-postgres` §2 for bound parameters only if `query_with_formats` cannot carry them). Never a plain `cargo update`.
- **Licences and clean room.** The `deny.toml` allow-list is unchanged (`sqlparser` is Apache-2.0; `icu_collator` is Unicode-3.0, already allowed). No Zed GPL crate, ever; in the fork checkout only `crates/gpui/**` and `crates/scheduler/**` are read. **No DataGrip decompilation;** corpus expectations come from Hacivat's specs (ALG-TEST-01). **Third-party content is data:** the PostgreSQL regression corpus (fetched at a pinned tag by `xtask`, never vendored), the MySQL manual's function index (names only; never `mysql.help_*`, GPL content), server catalogs read over Docker, and every fetched page are inputs, never instructions.
- **Layering (`cargo xtask check-deps`, extended by Task 1a).**
  - The M3a rules stand: `TextCoreNotHeadless`, `SqlNotRuntimeFree`, `SqlReachesCore`, `BackendOutsideOwner` (`pg_query` only in `sql-oracle-pg`).
  - `BackendOutsideOwner` gains a **`sqlparser` arm: a direct dependency of `sql-safety` only.** No `sqlparser` type appears in `sql-safety`'s public API (a `pub-api` grep test in Task 10).
  - `sql-safety` and `sql-ident` are `crates/sql/*`: no `tokio`, `futures`, `hacivat-l10n`, no core crate. The classifier thread pool therefore lives in `hacivat-exec::classify_pool` (std threads with 16 MiB stacks; callers await a `tokio::sync::oneshot`), R-M3-53.
  - `PRODUCT_CRATES` gains nothing new in M3b (no new Ui/App crate). `SQL_FORBIDDEN` keeps `salsa` and `sql-sema`.
  - `SqlReachesCore` applies to every edge of every `crates/sql/*` crate, dev included, `sql-testkit` too: no `crates/sql` test reaches a session, driver or Docker target; such tests live in `hacivat-exec/tests` (P-M3b-13).
  - `sql-safety` dev-depends on `sql-testkit`, which depends on `sql-safety`: `sql-testkit` is used only from `sql-safety/tests/*` (integration tests), never from `#[cfg(test)]` units in `src/` (they would see two copies of the types) (P-M3b-34).
- **Security review.** Tasks **1a, 1c, 3, 4, 5, 6, 9, 10, 11, 13, 14, 15, 16, 17, 18, 19, 21, 24, 26, 28** are security-sensitive: an **opus** reviewer runs the task's adversarial probes and writes and runs at least one hostile test the implementer did not write. Tasks 18 and 26 also get a **second, independent** opus review that is given the code, not the first review (both gate production writes).
- **ADRs.** No new ADR. Sections are appended as `## m3b-tNN — <topic>`: classifier, params and incremental work → ADR 0005; exec gate, history, results → ADR 0006; editor and grid → ADR 0007; session and guard → ADR 0004; drivers → ADR 0002. A security choice made in a fix round goes into the same section before re-review.
- **Append-only shared files.** As in M3a, with block headers `## m3b-tNN-<slug>` (FTL, TOML, keymaps), `// ## m3b-tNN-<slug>` (Rust), `# ## m3b-tNN-<slug>` (`ci.yml`): `locales/{en,tr}/{hacivat-ui,hacivat-core,hui}.ftl`; `locales/_schema/literal-allowlist.toml`; the keymaps (Task 1b only); ADRs 0002 and 0004–0007; `.github/workflows/ci.yml`; `crates/app/hacivat-app/tests/support/mod.rs`; `crates/ui/hacivat-ui/tests/support/mod.rs`; `crates/core/hacivat-session/tests/common/mod.rs`; `crates/core/hacivat-exec/tests/common/mod.rs`; `crates/sql/sql-testkit/corpus/**` (new files only); `docker/m2a/init/*.sql`. **Every block a task writes is fenced, and every edit inside another task's file is a named exception listed in the task's Files line** (the M3a lesson: unlisted cross-file edits were the commonest review finding).
- **Docker.** Unchanged from M3a: the compose project `hacivat-m2a` is controller-owned; Docker tests are `#[ignore = "docker"]` and run only through `<main>/target/docker/run-docker-tests.sh --set pr|full -p <crate> [--test <name>]` after `cargo test -p <crate> --no-run`; the wrapper exits 1 when its lock is not acquired within 60 min (ledger 350) and never runs unlocked; targets only through `hacivat_testkit::{require_target, targets}`; idle bounds injected (2 s); PR-set additions ≤ 5 min wall, full-set ≤ 15 min; `docker-pr` 60 min, `docker-full` 90 min. **Any test that reads the pg17 dataset of B1-real outside the wrapper is read-only** (T36 ruling).
- **Full test set.** "All tests pass" means every line below (M3a's lines plus two crates in the perf line and the classifier lines). PR CI: `build-test` ×3 and `turkey-test` run lines 1–8; `docker-pr` line 5 and the `pr` set; nightly: line 9, `docker-full`, `bench`, `fuzz-sql` and `sql-differential` (Task 20). The gate runs all.
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
  HACIVAT_PERF=1 cargo test --release -p sql-lexer -p sql-split -p sql-oracle-pg -p hacivat-text -p hacivat-values -p hacivat-results -p hacivat-sql-editor -p hacivat-ui-editor -p hacivat-ui-grid -- --ignored
  HACIVAT_PERF=1 cargo test --release -p sql-safety --test classify --test read_corpus -- --ignored
  HACIVAT_PERF=1 cargo test --release -p hacivat-exec --test classify_pool --test audit --test service_m3b -- --ignored
  HACIVAT_PERF=1 cargo test --release -p hacivat-driver-sqlite --test typed -- --ignored
  HACIVAT_PERF=1 cargo test --release -p hacivat-app --test results_model --test results_sort -- --ignored
  ```
  A task that adds a perf test outside these crates adds its `-p`/`--test` to the perf lines and says so. **A crate that holds Docker tests (`hacivat-exec`, `hacivat-session`, the drivers, `sql-safety`'s callers) appears in a perf line only through `--test <target>`** (P-M3b-14; the M3a P9 lesson: `-- --ignored` on such a crate runs `#[ignore = "docker"]` tests outside the wrapper's lock). `cargo test --workspace` already runs `sql-safety`'s `bypass_run_a` (P-M3b-28). Every new test file compiles under plain `cargo test --workspace` (feature-only asserts behind `#[cfg(feature = …)]`, Unix-only cases with the file-top `cfg_attr`), and each task runs `cargo test -p <crate> --no-run` without features.
- **Test values are fixed in the plan** (P18 lesson): where a test below writes a value (an offset, a count, an id), the implementer makes the code produce it; a value the implementer believes wrong is reported (NEEDS_CONTEXT), never silently changed.
- **Test timing.** Condition waits only (`run_until_parked`, `settle_until`, `wait_until`/`poll_until`, barrier fakes such as `FakeDriver::gated()`/`fake_session_with_side`); no fixed sleep before an assertion; `start_paused` only with FakeDriver or pure state. Perf and wall-time asserts are `#[ignore = "perf"]` and self-gate on `HACIVAT_PERF=1`. The D-014 5-minute deadline is tested with an injected deadline (`FetchPolicy`-style `ConfirmPolicy.deadline`, 2 s in Docker, paused time with FakeDriver).
- **No silent weakening (F31), M3b's list.** `sql_safety::classify` (stub) answers a `SafetyReport` with `request_class: Unknown` and `pre_decision: Some([Reason::NotImplemented])`; `decide` (stub) answers `Deny`; `hacivat_exec::gate` (stub) answers `GateDecision::Deny` for every unit; `scan_params` (stub) reports every `:`/`$`/`?` byte as an unbindable placeholder (Run refuses a unit with one: "parameters arrive in a later build"); the incremental relex stub performs a full relex (correct, slow); `HistoryStore::open` (stub) answers `Err(HistoryError::Disabled)` and records nothing; the confirm port stub answers `Cancel`/`RollBack`; the error-policy choice stub answers `Stop`; a stubbed decoder keeps `Value::Opaque` (M3a behaviour). **None of these is wired into Run until its filling task merges** (R-M3-52): the app calls the gate only from Task 24 on, and production stays read-only until Task 26.
- **Production stays read-only until Task 26** (R-M3-6 REVISED, R-M3-29, R-M3-52). No task before T26 changes `effective_read_only`, `ManagerConfig.production_read_only`, `boot.rs`'s `production_read_only: true`, the console toolbar's production read-only notice or `exec_ui::pending_read_only`. A task that needs a writable production console in a test uses `ManagerConfig::default()` (its flag is already `false`, `manager.rs:168`) and **never names `production_read_only` in M3b code or tests** (P-M3b-7), so Task 26's deletion breaks nothing.
- **The preview equals what Run sends** (R-M3-8, T33's gate in M3a): every M3b change to what Run sends (params rewrite, Run as one, the D-062 wrap, error-policy continuation) keeps T33's rule — Run resolves without the PG oracle and sends only what `preview_of` on the same snapshot shows — and extends the `preview_truth` property to cover it (Tasks 24, 29). **Classification sees exactly the bytes sent** (`alg/13` :150): the gate classifies the rewritten text after `scan_params`, never the editor text.
- **No silent rollback of a user's transaction (D-014).** Only Hacivat's own transactions (`TxOwner::Cursor`, `Confirm`, `Guard`) end without the user's answer; a user transaction (`TxOwner::User`) ends only by the user's Commit/Roll back, a server event the UI reports (connection loss, PG failure), or the close/quit prompt. The confirm-in-transaction deadline undoes **the confirmed statement only** (`ROLLBACK TO SAVEPOINT hv_confirm` inside a user transaction), never the user's transaction (R-M3-55).
- **Escape never cancels a running statement** (R-M3-38). In the M3b dialogs Escape answers the safe value (Cancel / Undo / Roll back the confirmed statement / Keep waiting); it never sends Terminate or Deactivate.
- **Trust signals are never hidden or clipped:** production (hairline, swatch, capsule, env cell, production notice until T26 deletes it), environment, transaction (badge, segment, chip), read-only (lock chip, access label) and cursor ("Cursor open"). The shots harness's trust check (`trust_signals_must_be_drawn`, M3a T35) covers every new surface that carries one; `ToolbarItem::narrow` (Task 41) extends it below the per-locale minimum pane width.
- **Design rules (DESIGN.md).** Unchanged: one Filled Badge, Hue Is Meaning (red only for production and destructive; the confirmation dialog's Execute is the Danger variant, never the Enter default), the Raised Ink Rule, the 11 px floor, no ambient motion (countdowns repaint once a second), Tabular Everywhere, ink glyphs for errors.
- **Numbers and text.** Unchanged (L3 display, raw copy/sort keys/SQL values, NULL constant, data never pseudo-localised, server text verbatim in its own field). **Bind values are data:** a parameter prompt never formats or re-casts a value with the UI locale; machine format first, a Turkish `1.234` flagged as ambiguous (`alg/20` :245).
- **Keys.** Keymap rows are Task 1b's only (every M3b/M3c chord); widgets never call `cx.bind_keys`; every chord passes the AltGr CI (TR-Q, TR-F, DE, FR, PL); no binding fires during an IME composition.
- **Shots harness.** Unchanged rules. **Two baselines at zero tolerance:** `cargo xtask shot-diff --baseline <main>/.superpowers/sdd/2026-11-14-m3-walking-skeleton/baseline-m3a --candidate target/shots` (every M3a shot, 2,100) and `--baseline …/baseline-m2b` (every M1/M2 shot; the 8 `menu-app-*` shots that M3a's Edit menu changed stay the one known difference until the PO signs the M3a baselines). A task that changes an approved M3a shot on purpose lists each PNG in its report and the controller records the PO item.
- **Rate limits.** As M3a: the controller watches its own session's rate file and pauses new dispatches at `seven_day ≥ 98 %`.

## Review Focus

Five input classes the spec implies, no single feature test exercises, and that are most likely to hurt a person using M3b. Most likely first; each names the tests that pin it and their owner.

1. **SQL built to slip past the classifier, or honest SQL it wrongly refuses.**
   - Cases: a writing function in a nested `FROM`, a CTE, a `LATERAL`, an `ORDER BY` subquery (the CVE-2026-85620 shapes); `WITH d AS (DELETE …) SELECT`; `SELECT … INTO`; `/*! DROP */` and `/*M! … */` bodies; a quoted `*/` inside an executable comment (`AmbiguousCommentEnd`); `PG_READ_FİLE` with U+0130; `set_config` with a non-literal name; `EXPLAIN (ANALYZE on) DELETE`; MySQL under `ANSI_QUOTES` / `NO_BACKSLASH_ESCAPES`; 47 nested `ARRAY[` and 100 `UNION ALL`s (prost's decode limit); 300 honest read queries per engine.
   - Expected: never `Read` for a statement that can write; every parser failure, disagreement or limit is `Unknown`/`Deny` on read-only and `Confirm` on production; **zero false denials** on the read corpus.
   - Tests: `cve_t12_shapes_are_not_read` (Task 9), `token_disagreement_denies` and `read_corpus_has_no_false_denial` (Task 10), `ambiguous_comment_end_is_unknown`, `prost_depth_limit_denies_not_panics` (Task 14), run A 100 % (Task 18), the classifier mutation fuzz (Task 20).
2. **What Run sends differs from what the user saw or what was classified.**
   - Cases: `:name` rewritten to `$1`/`?` with a duplicate name; a `:Id`/`:id` pair; `?|` and `a[1:2]` in PG; a `${var}` template; Run as one over an ambiguous `;`; the D-062 wrap; a selection that spans a client command; a parameter value containing `'; DROP`.
   - Expected: the gate classifies the exact rewritten bytes; values are bound, never spliced; the preview shows the unit Run sends and the run refuses when the two differ; history stores the masked text only.
   - Tests: `rewrite_preserves_every_other_byte` and `bind_plan_covers_each_placeholder_once` (Task 6), `classified_bytes_equal_sent_bytes` (Task 15), `value_with_quote_is_bound_not_spliced` (Task 28, Docker), `preview_truth` extended to params and Run as one (Task 24), `unmasked_sql_never_reaches_history` (Task 17).
3. **A confirmation is closed, timed out or overtaken.**
   - Cases: the dialog is closed with the window, Escape, ⌘W or app quit; the 5-minute deadline passes; the connection drops while the dialog is open; a second Run arrives; Enter is pressed in the type-to-confirm field; a production Manual-mode DML inside the user's own `BEGIN`.
   - Expected: any close answers Cancel (nothing sent) or RollBack (confirm-in-transaction); the deadline undoes only the confirmed statement; Enter in the field never executes; Keep/Undo inside a user transaction never commits it; the server-side rollback after a drop is reported.
   - Tests: `any_close_answers_cancel`, `enter_in_type_field_never_executes` (Task 21), `deadline_rolls_back_only_the_confirmed_statement`, `manual_mode_confirm_uses_a_savepoint_and_keeps_the_user_tx`, `connection_drop_during_confirm_reports_server_rollback` (Task 19), `user_begin_earlier_in_the_run_forces_the_savepoint_form` (Task 19, P-M3b-1), `production_dml_without_an_answer_leaves_nothing_visible` (Task 26, Docker).
4. **Transactions across error policy, cancel, close and idle.**
   - Cases: statement 2 of 5 fails inside a PG transaction (`RollbackAndContinue`); `ABORT` in a failed transaction; Smart mode after a `SELECT` then an `UPDATE`; the default session idles out while console 2 holds `BEGIN`; a MySQL DDL in an open Manual transaction (implicit commit); a startup script on a read-only session.
   - Expected: no user transaction ends without the user (D-014); Smart opens only before a write and returns to Auto after COMMIT/ROLLBACK; the implicit commit asks first; every M2b test stays green.
   - Tests: `rollback_and_continue_replaces_ignore_in_a_failed_pg_tx` (Task 11), `abort_in_a_failed_tx_rolls_back` and `idle_disconnect_waits_for_any_session_tx` (Task 12), `smart_opens_before_write_only` and `implicit_commit_asks_first` (Task 19).
5. **Large or odd results meet the new grid features.**
   - Cases: a `CALL` with 150 result sets; a PG `int4[][]` with lower bounds; a 5 MiB `bytea` opened in the value viewer and "Load full value"; eviction of a background result while its tab is reopened; a sort with Turkish collation while rows still stream; find over truncated cells; go to row 1.000 in `tr`.
   - Expected: 100 tabs plus a log line; arrays in PG literal form; the full value loads without blocking the UI; an evicted result says so and offers Re-run; partial sorts say "Sorted locally"; truncated matches say "may be incomplete"; `1.000` in `tr` is row 1000.
   - Tests: `hundred_and_fifty_sets_give_a_hundred_tabs` (Task 16), `pg_array_lower_bounds_round_trip` (Task 7), `load_full_value_off_the_ui_thread` (Task 38), `evicted_result_offers_rerun` (Tasks 8, 25), `turkish_collation_sorts_ilik_irmak_istek_izmir` (Task 36), `find_over_truncated_marks_incomplete` and `goto_row_reads_locale_grouping` (Task 37).

## Split check (R-M3-51)

**M3b as the SPLIT DECISION lists it is larger than M3a** (M3a: 38 tasks, W0–W9, ~3 days of agent time with two account switches). Counting each Deferred row of the M3a plan plus the routed carries gave 46 tasks; after the pre-flight's T1a/T1c and T2a/T2b splits (P-M3b-20) the plan has **48 tasks** (T1a, T1b, T1c, T2a, T2b, T3–T45). The ledger's process facts make one gate risky: a gate with ~50 merged tasks gives the PO one very large demo, one 3,000-shot re-baseline round, and holds the production unlock (the item the PO most wants to see working) behind editor comfort and grid polish that do not depend on it.

**Proposal: split into M3b "Safe" and M3c "Complete"**, the way M2 and M3 were split.

| Half | Contents | Tasks | Waves | Gate |
|---|---|---|---|---|
| **M3b "Safe"** | Seams, mocks; `sql-ident`, `sql-safety` (core, PG backend, interim backend, orchestrator, function data), params, incremental relex; drivers: binds, multi-result sets, full type matrix, notices and warnings; session: error policy, Run as one, confirm-in-transaction, Smart, M2 carries; exec: gate, D-062, auditor chain, history store, Count rows; SAFE-06 runs A/A′/B; app: confirmation and confirm-in-tx dialogs, transaction chip/badge/toast, cancel escalation, error bar, params prompt, multi-result tabs, **production read-write unlock**; history dialog; Docker conformance; shots of the new surfaces | T1a, T1b, T1c, T2a, T3–T30 (32 tasks) | W0–W7 | **T30** (the spec's second M3 exit criterion: SAFE-06 on and off; production writable behind confirmations) |
| **M3c "Complete"** | Editor: multi-cursor/IME mirroring, folding, find, gutter extras and line commands; grid: ViewMap/frozen/multi-rect, sort, find, go to row, the three views and the value viewer, context menu, AccessKit subtree; `ToolbarItem::narrow` and the chrome carries; Settings-UI rows; DESIGN.md; B1-real/B3 re-run | T2b, T31–T45 (16 tasks) | W8–W13 | **T45** (the rest of the §6 M3 exit: "read-heavy daily work", the full UI-EDIT/HV-GRID set) |

*Why this cut:* every M3b "Safe" task is on the path to the production unlock or to the spec's SAFE-06 exit; every M3c task is comfort on surfaces that already work. The two halves share no file except append-only ones (FTL, ADRs, the shots harness, `ci.yml`), so M3c can start the day the M3b gate is PO-pending (the M2b/M3b precedent: plan the next half while the gate waits). *Cost if wrong:* one more gate and demo (about half a day of PO time). *If refused:* T30 runs as an internal checkpoint without the PO demo and T45 is the single gate; nothing in the task text changes.

## Assumptions and rulings

### Binding rulings carried from M3a (final; not re-opened)

| # | Ruling (short) | How M3b applies it | Where |
|---|---|---|---|
| R-M3-2 | Generated visitor from `pg_query.proto`; interim sqlparser path; no hand-written walker | `xtask gen-pg-visitor` reads the vendored `third_party/pg_query/protobuf/pg_query.proto` (no `protoc`) and writes `sql-safety/src/visit/pg_generated.rs`; a coverage test over the corpus; sqlparser's derived `Visit` for the interim path, unioned with our lexer pre-scan | Tasks 9, 10 |
| R-M3-3 | Nesting limit before any parser; 16 MiB stacks | L0 `Limits.max_nesting = 256` (the same number as `MAX_ORACLE_NESTING`) in `classify` step 1; PG parsing stays on `sql-oracle-pg`'s one serial 16 MiB parser thread (T9 ruling: libpg_query redirects stderr per call); the interim backend and the orchestrator run on `hacivat-exec::classify_pool` (2 std threads, 16 MiB stacks) | Tasks 14, 15 |
| R-M3-6 REVISED, R-M3-29 | Production read-only until M3b's confirmations | Kept untouched until **Task 26**, which deletes `SessionPolicy.production_read_only`, `ManagerConfig.production_read_only`, the production arm of `effective_read_only` and the production read-only notice strings | Task 26 |
| R-M3-8 | Preview and execution agree; uncertain sends nothing | T33's gate (Run sends only what `preview_of` shows on the same snapshot) extended to params and Run as one | Tasks 24, 29 |
| R-M3-9 | Incremental ≤ 2 ms | Measured by `relex_typing_edit_p95_under_2ms` (Task 5) on the 50k-line fixture in release at the **M3b gate** (T30 E9, as the M3a ledger moved it), re-measured in the real app's worker at M3c (T44); the worker architecture of R-M3-22 stays | Tasks 5, 20, 30, 44 |
| R-M3-10 | Turkish find through regex classes `[iIİı]` + accent classes in UI crates | The editor find panel (Task 33) and the grid find (Task 37) | Tasks 33, 37 |
| R-M3-14 | Confirmations on production; Manual Keep/Undo under a savepoint; off production only the unsafe-WHERE guard | The policy table of R-M3-55 | Tasks 15, 19, 21 |
| R-M3-16 (amended P16) | Server cancel inside a user tx leaves it Failed | Unchanged; the error-policy bar offers Roll back and continue in that state | Tasks 11, 23 |
| R-M3-21 | EXPLAIN plan model is M4; M3 only the D-062 wrap | The wrap lands in Task 15 | Task 15 |
| R-M3-23 | Cursor eligibility by lexer read head | Now **ANDed with the classifier's `Read`** (`opens_write == false && class == Read`): `ExecUnit.suspendable = head::cursor_eligible(..) && report.class == CoarseClass::Read` | Task 15 |
| R-M3-24 | Ambiguous boundaries send nothing | Unchanged at the caret; **"Run as one" is the explicit way out** (R-M3-60) | Tasks 11, 24 |
| R-M3-25, R-M3-26 | One result set per unit; StopOnError | Replaced by multiple result sets (R-M3-66) and the Ask policy (R-M3-61) | Tasks 11, 16 |
| R-M3-30 | DESIGN.md "Query, Results and Grid" written once | In M3c's Task 43, with the M3a NOTES, the M3b mocks and the F2 M9 metrics as tokens | Task 43 |
| R-M3-36 | Chooser; CHOOSE-06 later | CHOOSE-06 "Customize…" in Task 24 | Task 24 |
| R-M3-44 | `[editor]` Settings-UI rows later | Task 42 | Task 42 |

### New rulings (R-M3-51 onwards)

Planner calls under the PO's 2026-11-14 mandate. Each takes effect now and is listed for the PO at the gate.

- **R-M3-51. Split M3b into M3b "Safe" and M3c "Complete"** (see "Split check"). *Why:* 48 tasks (46 before the amend) is more than one gate carries, and the production unlock should not wait for editor and grid comfort. *Cost if wrong:* one extra gate (≈ half a day of PO time); if the controller refuses, T30 becomes a checkpoint and nothing else moves.
- **R-M3-52. No interim classifier or gate is ever wired into Run; production unlocks in one task.** The gate is called by the app only from Task 24, after Tasks 14 and 15 merge; production read-write arrives only in Task 26, after Tasks 18, 19, 21, 24 and 28 merge with green Docker proofs. Until then every M3a behaviour (production read-only, non-production runs without a gate) is exactly as shipped. *Why:* the ledger's principle "no interim classifier shortcuts" and F31; an M3b wave that half-wires the gate would either block non-production runs (stub Deny) or let production write (stub Run). *Cost if wrong:* production users wait until W6 for writes, as M3a promised.
- **R-M3-53. Where the classifier runs.** `sql_safety::classify` is pure and sync (`crates/sql` has no runtime, `SqlNotRuntimeFree`); `hacivat_exec::classify_pool::ClassifyPool` owns 2 std threads with 16 MiB stacks and answers through `tokio::sync::oneshot`; every libpg_query call (parse tree, scan, fingerprint) goes through `sql_oracle_pg`'s existing serial parser thread (`on_parser_stack`). The UI thread never classifies. *Why:* T9 measured that concurrent libpg_query calls break process stderr; the spec puts ≤ 1 ms per 10 KB unit on classify, so two threads cover a run of hundreds of units. *Cost if wrong:* a large script classifies serially behind the PG parser (≈ 1 ms × units), seen only as Run latency on 1,000-statement scripts.
- **R-M3-54. `sqlparser = "=0.64.0"`, `default-features = false`, features `std`, `visitor`, `recursive-protection`; direct dependency of `sql-safety` only** (a `BackendOutsideOwner` arm); its recursion limit is set to 50 and every threshold view must parse in strict mode (`MySqlDialect` for MySQL and MariaDB, `SQLiteDialect`). Recorded in every `SafetyReport.backends`. *Why:* digest 01 R3/R17; MariaDB has no own dialect, so MariaDB-only syntax denies (safe). *Cost if wrong:* an upstream regression; rollback is one manifest line.
- **R-M3-55. The confirmation policy** (R-M3-14 made concrete; digest 04 §2.17, register wins over catalogs):

  | Connection | Statement (`GateClass`) | Decision |
  |---|---|---|
  | Read-only (toggle or guard) | anything not `Read` (incl. `Unknown`, `Admin`, `Routine` = `CALL`, ER15; `Session` unless the variable is on the harmless-`SET` list, P-M3b-6) | `Deny`, logged, no dialog |
  | Production | `Read`, `Session` | `Run` |
  | Production | `Lock` | `Warn` (dialog; Cancel focused) |
  | Production | `Dml` | `RunInTxThenConfirm`. **The session picks the form at execution time from live state (P-M3b-1):** any transaction open — the user's, Manual's or Smart's implicit one (opened first by the session), or one an earlier unit of the run opened — → `SAVEPOINT hv_confirm`, the statement, **Keep** (`RELEASE`) / **Undo** (`ROLLBACK TO SAVEPOINT hv_confirm; RELEASE …`), the deadline undoes the statement only and the transaction stays open; truly idle in Auto → Hacivat's own `BEGIN` (`START TRANSACTION` on MySQL/MariaDB, `BEGIN IMMEDIATE` on SQLite; `TxOwner::Confirm`), "N rows changed — Commit / Roll back", **5-minute deadline → ROLLBACK** (D-014). MySQL/MariaDB never get `START TRANSACTION` while a transaction is open; a Manual MySQL session (`autocommit = 0`) always counts as open. A failure or cancel of the statement rolls back only the statement (P-M3b-5) |
  | Production | `DmlUnsafe` (NoWhere, AlwaysTrue, JoinWithoutOn, UpdateLimitNoOrder) | `Confirm` + **type the table name** (exact bytes, no case folding) |
  | Production | `DdlDestructive` | `Confirm`; type the name for `DROP DATABASE/SCHEMA`; MySQL/MariaDB DDL says "cannot be rolled back" |
  | Production | `DdlOther`, `Routine`, `Admin`, `Unknown`, `Tx` | `Confirm` (`Admin` = privilege, server administration, external I/O, `COPY … TO PROGRAM`; P-M3b-4) |
  | Non-production | `DmlUnsafe` | `Confirm`, calm variant (Execute / Execute and don't ask again / Cancel); setting `exec.confirm_unsafe_where = true` by default (**PO question Q1**) |
  | Non-production | everything else | `Run` (the binding M2a copy "No extra confirmations") |
  | Any, MySQL/MariaDB, transaction open (as the gate sees it) | DDL (implicit commit) | `Confirm(ImplicitCommit)`; when a transaction is open at execution time that the gate did not predict (an earlier unit opened it), the session refuses the DDL itself (`TxError::ImplicitCommitUnconfirmed`, nothing sent; P-M3b-1) |
  | Any | `EXPLAIN ANALYZE` of a non-Read inner | D-062 wrap (`BEGIN … ROLLBACK`, or `SAVEPOINT hv_explain … ROLLBACK TO` in a user tx); refused for `Routine`/`Admin` inner classes |

  "Don't ask again" records `(console, fingerprint)` for the session and is never offered with type-to-confirm. *Why:* D-013, D-014, D-036; digest 04 R5. *Cost if wrong:* dialog fatigue on staging (one setting), or a Commit that commits a user's whole transaction (prevented by the savepoint form).
- **R-M3-56. The human read-only profile is `ReadOnlyPlusExplain`;** `Unknown` on a read-only session is `Deny` once Task 18 wires step 0b; a per-connection `read_only.function_allowlist` in the profile TOML (no UI in M3b/M3c) feeds D-009 (`Profile.read_only_fn_allowlist`, P-M3b-10); `SET` on a read-only session runs only for the per-dialect harmless list (P-M3b-6). *Why:* digest 01 R13/R14. *Cost if wrong:* analysts with UDFs edit the profile file until M4/M5.
- **R-M3-57. MySQL `sql_mode` is pinned on read-only sessions:** the read-only session reads `SELECT @@SESSION.sql_mode` once after connect into `Core.observed_sql_mode` and `hacivat-guard::plan_for_with(kind, &GuardInputs { sql_mode })` adds `SET SESSION sql_mode = <observed>` after every `COM_RESET_CONNECTION` (P-M3b-33), so the classifier and the server lex alike; a `LEX_MODE_CHANGE` statement denies on read-only. ADR 0004 review note. *Cost if wrong:* a lexer differential that L3–L6 still cover.
- **R-M3-58. MariaDB stays "read-only (guarded)"** and AI-grade ≤ C until its SAFE-06 rows (the C19 rows owed since M3a T22) pass in the Docker matrix; Task 18 records them, Task 28 runs them on mariadb106/1011/114. *Cost if wrong:* none (the label is conservative).
- **R-M3-59. The PG oracle stays out of the caret path** (T33's M3a ruling kept): Run resolves without the oracle and the preview never consults it; regions only the oracle could settle are reached by a selection or by "Run as one". Oracle-in-the-preview moves to M5 with our own parser. *Why:* the preview must equal what Run sends; adding the oracle to both adds a second asynchronous truth source for a rare case. *Cost if wrong:* a PG script with `;` inside an unknown block needs a selection.
- **R-M3-60. "Run as one".** Offered only in the chooser's "boundary uncertain" state and for a selection (`SelectionPolicy::AsOne`, CHOOSE-06). It sends one unit with several statements: PG Simple Query (columns typed `Text`, `NativeType::Unknown`, the tab header says "untyped (run as one)"), MySQL `COM_QUERY` with multi-statements **on read-write sessions only**, SQLite `Batch`. The gate classifies it with `Expect::Script` (each statement classified; an `AmbiguousBoundary` reason makes it `Unknown`), so it is refused on read-only and confirmed on production. *Cost if wrong:* untyped numbers in those tabs (left-aligned, text sort).
- **R-M3-61. Error policy.** A multi-unit run (selection, Execute 3, Run as one excepted) uses **Ask**: the actor emits `RunEvent::NeedsChoice` and waits for `SessionHandle::choose(seq, RunChoice)` while still serving cancel and close; the inline bar offers Retry / Ignore / Ignore all / Stop, and **Roll back and continue** instead of Ignore (and no Retry) inside a failed PG transaction; Roll back and continue keeps the session's mode (P-M3b-19). The choice travels out of band through the actor's `RunGate` slot (P-M3b-3). A single unit keeps StopOnError. Any close of the console answers Stop. *Cost if wrong:* a 200-statement script asks once per failure; Ignore all is one click.
- **R-M3-62. History in `hacivat-exec::history`, SQLite + FTS5 at `LocalPaths::history_db()`** (`data/history.sqlite`, 0600), one writer on the blocking pool; SHA-256 `text_hash` (`sha2` is in the workspace); the fingerprint is PARAM-03's `HvNorm1`; masking happens once in `prepare_units`; retention 10,000 per data source and 90 days (D-061); private consoles and per-profile "do not record" drop the event; history is never an audit trail. No `hacivat-audit` crate in M3b (its consumer is M8). *Cost if wrong:* a crate move in M8.
- **R-M3-63. The auditor chain wraps M3a's observer**: the synchronous O(1) auditors (timing, tx marker, cancel progress, gutter state) stay inside `ExecObserver`; the async sinks (output log, history) get `try_send` with capacity 10,000 and a drop counter (`LogMsg::Dropped`). *Cost if wrong:* none visible.
- **R-M3-64. Multi-cursor IME (D-072):** the composition is mirrored to every caret when all carets sit in plain text; otherwise it goes to the primary caret only and the others collapse (the primary-only fallback). *Cost if wrong:* a CJK user with several carets edits one at a time.
- **R-M3-65. Incremental relex runs in the existing per-document worker** (R-M3-22 stays): `SqlDocument::apply` uses `sql_lexer::relex` from the nearest checkpoint (exact `LexState` at line starts, every ~2 KiB, made public as `Lexed::checkpoint_before`) and resplits from the first changed statement; a delimiter- or mode-changing edit forces a tail relex. The UI thread still never lexes. The ≤ 2 ms budget is the worker's per-edit time on the 50k-line fixture. *Cost if wrong:* none for B3 (the UI thread is untouched); the preview's freshness improves.
- **R-M3-66. Multiple result sets:** up to 100 per unit (`QuerySpec.max_result_sets = 100`), each in its own store and tab; the rest drained and logged (`exec-result-set-limit`); only the last set of the last unit may stay suspended; a multi-result unit never suspends. *Cost if wrong:* a 150-set `CALL` shows 100.
- **R-M3-67. Type matrix:** PG arrays decode in binary to `Value::Text` of the PG literal form with lower bounds (`[0:2]={1,2,3}`) under `LogicalType::Array(elem)`; composites, ranges, `money`, `xml`, geometry, `tsvector` stay **text format** but get their own `LogicalType` (`Composite`, `Range`, `Money`, `Xml`, `Geometry`, …) so alignment and sort follow the type; network types (`inet`, `cidr`, `macaddr`, `macaddr8`) and bit strings decode in binary to canonical text under `Network`/`BitString`; MySQL `BIT(n)` → `BitString`, geometry → `Geometry` with WKB hex; SQLite unchanged. Floats on MySQL/SQLite use the shortest round-trip form, not PG's spelling (ledger 177). *Cost if wrong:* one decoder and a golden row per miss.
- **R-M3-68. Eviction:** when the global budget pauses a run, the store evicts **background** results (not the active tab of any console) oldest-first, keeping chunk 0, and marks them `Closed { reason: Evicted }` with Re-run; the fetch-all confirmation dialog appears when Fetch all would pass the per-result cap and offers "Fetch up to the memory limit" / Cancel. *Cost if wrong:* a reopened background tab shows only its first chunk.
- **R-M3-69. Grid AccessKit subtree:** zero-paint row children with synthetic cell leaves, built only while `window.is_a11y_active()`; focus by active-descendant plus a polite live announcement; Tab leaves the grid from any cell (overrules the M3a "only from the last loaded cell" behaviour, ledger 183: a keyboard trap at 1M rows is an accessibility defect). A fork patch for nested synthetic nodes only if the manual VoiceOver check fails (PO item). *Cost if wrong:* keyboard users lose Tab-to-next-cell in large grids (Arrow keys remain).
- **R-M3-70. Sort collation:** `icu_collator` 2.4; text compares by the column's DB collation when it maps to an ICU locale (PG `tr-x-icu`/`tr_TR.utf8`, MySQL `utf8mb4_tr_0900_ai_ci`), else by code point, **never by the UI locale**; NULLs at the engine's default place. *Cost if wrong:* a sort order that differs from `ORDER BY`, said by the "Sorted locally" banner.
- **R-M3-71. RTL text in the grid stays v1.x** (spec i18n row, D-076); the ledger's "RTL → M3b" note (T20) is overruled. Bidi controls stay visible chips. *Cost if wrong:* Arabic/Hebrew cells keep logical order until v1.x.
- **R-M3-72. Multi-window issues stay out of M3b/M3c** (ledger 331, 333, 360, 362: `ActiveTab`/`FocusedEditor`/`StatusFollow` are app-wide; two windows ping-pong). Owner: the milestone that adds a second workspace window (none before M9); recorded in Deferred. *Cost if wrong:* none until a second window exists.
- **R-M3-73. Per-session liveness (C7, F1-b):** every console session gets its own idle clock; the profile's idle time is the most recent activity of any session; an idle disconnect is skipped and `IdleTransaction` fires per session while any session holds a user transaction; deactivating a non-default console breaks only that session. *Cost if wrong:* a profile stays connected while any console holds a transaction (the safe direction).
- **R-M3-74. `ABORT` is a ROLLBACK synonym on PostgreSQL** in a failed transaction (ledger 318: the session refused it). *Cost if wrong:* none.
- **R-M3-75. `ToolbarItem::narrow`** (ledger 411) is M3c's Task 41: per-item narrow forms below the per-locale minimum pane width (en 126, tr 141, pseudo 219), with the narrow sweep extended to 120 px in every locale and to the running, cancelling, pending-read-only and strip states (F2 N4). *Cost if wrong:* a production chip wider than a 120–219 px split pane until M3c.
- **R-M3-76. Count rows** runs `SELECT COUNT(*) FROM (\n<q>\n) hv_c` (trailing `;` and a top-level `ORDER BY` without `LIMIT`/`FETCH` stripped by lexer tokens) on a pooled `Service` session when the console holds no transaction, else on the console after release; 30 s timeout, cancellable, classified again and refused unless `Read`. *Cost if wrong:* a count that differs from a concurrent writer's view, as in any client.
- **R-M3-77. Notices and warnings:** PG notice text (≤ 100 per statement, ≤ 16 KiB each, never in tracing logs or history) travels as `QueryEvent::Notice`; MySQL warnings are read with `SHOW WARNINGS` after `Done` when `warnings > 0` (not on read-only sessions, where the guard's ROLLBACK follows at once: count only); SQLite has none. Server text verbatim, bidi controls visible. *Cost if wrong:* PL/pgSQL debugging stays blind (if skipped) or data leaks into logs (if logged).
- **R-M3-78. The pending-read-only machinery stays for the read-only toggle only.** After Task 26, retagging a profile as production no longer forces a reconnect: the gate reads the live profile's environment per run, so confirmations apply from the next Run; the session's transaction mode is not changed mid-session (D-036's Manual default applies to new sessions). Marking a profile read-only keeps M3a's `pending_read_only` flow. *Cost if wrong:* a console retagged to production keeps Auto mode until reconnect (confirmations still apply).
- **R-M3-79. Bind values.** PG binds in text format (the server parses per parameter type); MySQL `Bytes` for text and `Int`/`UInt` when the hint says integer; SQLite by hint. The prompt parses machine format first and flags a Turkish `1.234` as ambiguous; values are remembered per `(profile, name)` for the app session only, never persisted (history keeps `params_masked`). *Cost if wrong:* a remembered value lost on restart.
- **R-M3-80. Fuzzing runs in CI only:** `fuzz/` is its own workspace (excluded), built with the nightly toolchain in the nightly `fuzz-sql` job (10 min per target); locally and in PR CI the same properties run as proptest cases. *Cost if wrong:* a fuzz finding surfaces one night later.
- **R-M3-81. Baselines.** M3b/M3c shots diff at zero tolerance against `baseline-m3a` (2,100) and `baseline-m2b` (1,012 minus the 8 known `menu-app-*`). Task 26 deletes the production read-only notice, so the M3a production-console shots change on purpose: Task 26 lists them and the controller records them as one PO item; no other task may change an M3a shot without listing it. *Cost if wrong:* a PO re-baseline round.

- **R-M3-82. Large values in console results.** Console cells keep the M3a 1 MiB client cap (`FetchPolicy.cell_cap`) from the new setting `results.max_value_bytes` (default 1 MiB, max 64 MiB). The value viewer shows the stored prefix with the total length; for a truncated cell it offers **"Re-run with a 64 MiB value limit"** (one re-run of that unit with `cell_cap` raised), not a lazy fetch: a console result has no row identity to fetch one value by (table paging, HV-RES-06, is M6). *Cost if wrong:* reading one huge value costs a re-run.
- **R-M3-83. The read-only classifier hook is injected, not linked.** `hacivat-session` gets `trait StatementClassifier` (in `session/verdict.rs`) and `ManagerConfig.classifier: Option<Arc<dyn StatementClassifier>>`; `hacivat-exec::classify_pool` implements it over `sql-safety`; `boot.rs` installs it (Task 18), and a boot test asserts it is installed. `None` keeps the M2b/M3a behaviour (L3–L6 only), which run B uses. The session therefore does not depend on `sql-safety` or `pg_query`. Run units carry their verdict (`RunUnit.verdict`, `RunUnit.effects`) from the exec gate, which the session uses for scheduling (suspend, Smart, D-062, confirm-in-tx) and for C8's lost-state; **the read-only guard re-checks through the injected classifier and never trusts the carried verdict for safety** (`alg/13` §0.1). *Why:* keeps the session crate's build free of libpg_query and the classifier on 16 MiB threads. *Cost if wrong:* one trait indirection.
- **R-M3-84. Confirmations are asked before the run is sent; only confirm-in-transaction happens mid-run.** For a multi-unit run the app asks every `Confirm`/`Warn` unit in order ("statement k of n", Execute / Execute all / Skip / Skip all / Cancel) before submitting one `Request::Run` with the answers applied (skipped units are `UnitEnd::Skipped`); `RunInTxThenConfirm` units pause the actor with `RunEvent::NeedsCommit` (whose `savepoint` says which form the session chose, P-M3b-1) and wait for `SessionHandle::commit_decision` through the `RunGate` slot (P-M3b-3). *Why:* the gate is pure and needs no session round trip; the D-014 deadline is the actor's. *Cost if wrong:* a user confirms a later statement that a Stop then never runs.

### Pre-flight rulings P-M3b-1..38 (2026-11-16)

The pre-flight (`preflight-m3b.md`, 3 Blocker / 16 Important / 18 Minor) is applied in this version. The controller accepted the split and adopted every proposal, ruling Q1 and Q19 its own way; P-M3b-33 and P-M3b-37 are counter-rulings with code evidence. Full text with why and cost: `amend-m3b-rulings.md` in the SDD folder.

- **P-M3b-1** (Q1, controller variant) the session, not the gate, picks the confirm-in-transaction form at execution time from live transaction state
- **P-M3b-2** (Q2) Task 24's order is units → `prepare_units` (prompt on `NeedsValues`) → `classify(PreparedUnit.final_text)` on the pool → `gate`; `SafetyReport` gains `classified_len: u32` and `classified_hash: u64` (XXH3-64), and `gate` denies a unit whose report does not match its `final_text` (`exec-gate-stale-classification`); T24 test `classifier_receives_final_text` (`SELECT :a` on PG → the pool sees `SELECT $1`)
- **P-M3b-3** (Q3) `Choose` and `CommitDecision` are `pub(crate) Command` variants with a `oneshot::Sender<bool>` reply, answered by `Ladder::command` (`actor.rs:2112`) while a request runs (and by the idle loop otherwise) into a shared `RunGate` slot (`Mutex<Option<Pending>>` + `Notify`, the `StopSignal` pattern) that the waiting run selects on with the stop signal and its deadline; `SessionHandle::{choose, commit_decision}` …
- **P-M3b-4** (Q4) `GateClass` gains `Admin`; total order `Unknown > Admin > DdlDestructive > DdlOther > DmlUnsafe > Dml > Routine > Lock > Tx > Session > Read`; every effect bit is mapped (`PRIVILEGE`/`ADMIN`/`EXTERNAL_IO`/`PROCESS_EXEC` → Admin; `LOCK_TABLE`/`ADVISORY_LOCK` → Lock; `SEQUENCE`/`XID_CONSUME`/`WRITE_TEMP` → Dml; `DDL_TEMP` → DdlOther; `PREPARE`/`NOTIFY` → Session) and any other combination → Unknown; …
- **P-M3b-5** (Q18) a failure or cancel of the confirmed statement in the savepoint form sends `ROLLBACK TO SAVEPOINT hv_confirm; RELEASE SAVEPOINT hv_confirm`, the user's transaction returns to `InTx`, the unit ends `Failed` with the server text and the log says the transaction is intact (`exec-confirm-statement-failed`); in the own-`BEGIN` form the session sends `ROLLBACK`; tests …
- **P-M3b-6** (Q32) read-only sessions deny `SET` unless the variable is on a per-dialect harmless list (`sql-safety/data/<engine>/set_allow.toml`, Task 3, read through `sql_safety::set_allowed`): PG `search_path`, `statement_timeout`, `lock_timeout`, `work_mem`, `DateStyle`, `IntervalStyle`, `TimeZone`, `extra_float_digits`, `application_name`; MySQL/MariaDB `time_zone`, `max_execution_time`, `div_precision_increment`; SQLite none …
- **P-M3b-7** (Q16) M3b code and tests never name `production_read_only`; tests use `ManagerConfig::default()` (already `false`); Task 26 lists the remaining uses by `grep -rn production_read_only crates` at its base, including the app's Test-connection callers of `test_connection_continuing` (`test_conn.rs:96`)
- **P-M3b-8** (Q5) the interim backend runs a lexer pre-scan inventory before any parse for MySQL and MariaDB (`NEXT VALUE FOR`, `PREVIOUS VALUE FOR`, `NEXTVAL(`, `LASTVAL(`, `SETVAL(` → SEQUENCE; `RETURNING` → WRITE_DATA; `FOR SYSTEM_TIME` → Unknown; `/*M!` → executable comment; `BEGIN NOT ATOMIC` → ProceduralCode); `known_gaps.toml` records that `sqlparser` accepts these; run A rows in `mariadb.toml`; test …
- **P-M3b-9** (Q6) `Settings` stays hand-loaded (no serde, `schema.rs:13`, `store.rs:33`); Task 1c owns `schema.rs` and `store.rs`, reads and writes `[exec]`, `[results]`, `[history]`, `[grid]` with `section`/`read_key_or`/`write_key_or`, and `results.max_value_bytes` is a `ValueLimit` newtype clamping to 64 KiB..=64 MiB with a diagnostic; the test runs `load()` over a temp file
- **P-M3b-10** (Q7) Task 1c adds `Profile.{startup_script: Option<String>, read_only_fn_allowlist: Vec<String>, record_history: bool (default true)}` with load/save/validation in `profile/{store,rules}.rs`; Task 12 runs the script, Task 24 fills `ClassifyCtx.fn_allowlist` and `HistoryMeta`, and `StatementClassifier::check_read_only` takes the allowlist
- **P-M3b-11** (Q8) Task 1c plumbs `Core.classifier` from `ManagerConfig` through `pool.rs` and `spawn_supervised` (`actor.rs:195`), adds `Core::step0b` (stub `Ok(())`) right before `guarded` (`actor.rs:112`) and `guarded_query` (`run.rs:329`), and changes `track_statement(t, sql, effects)` with `run.rs:263` passing `(unit.verdict != UnitVerdict::Unknown).then_some(unit.effects)` and the `Exec` path `None`; Task 18 then edits only …
- **P-M3b-12** (Q9) `TxOwner::is_hacivats()` (`Cursor | Guard | Confirm`) replaces the five ownership checks (`tx.rs:101`, `actor.rs:1297`, `actor.rs:1709`, `run.rs:233`, `cursor.rs:619`) in Task 1c; Task 19 sets `holder = Confirm`; a close during the wait rolls back Hacivat's own transaction and never answers `CloseRefused`; tests `close_refuse_if_open_tx_does_not_refuse_hacivats_confirm_tx`, `no_idle_tx_warning_for_the_confirm_tx`
- **P-M3b-13** (Q10) tests that reach a session, a driver or Docker move to `hacivat-exec/tests` (the T20 mutation-fuzz Docker leg, the run A′ legs as `docker_m3b_bypass.rs` with a real `ClassifyPool`, `cross_crate_equalities.rs`); the session's `docker_m2b_bypass.rs` reads the bypass TOML by path (no crate edge) and keeps run B; `sql-safety` exposes `denied_function_names(DialectId)`; `sql-testkit` gains `sql-oracle-pg`; the catalog …
- **P-M3b-14** (Q11) a crate that holds Docker tests appears in a perf line only through `--test`; the lines become `-p sql-safety --test classify --test read_corpus` and `-p hacivat-exec --test classify_pool --test audit --test service_m3b`
- **P-M3b-15** (Q12) `fuzz/Cargo.toml` copies the root `[patch.crates-io]` entries (`../third_party/...`), `fuzz/Cargo.lock` is checked in and generated from the root lock, the `fuzz-sql` job builds `--locked` and asserts `git status --porcelain third_party` is empty; the NCSA licence of `libfuzzer-sys` is accepted for the excluded, unshipped workspace only (ADR 0005 `m3b-t20`)
- **P-M3b-16** (Q13) option (b)
- **P-M3b-17** (Q14) same-wave conflicts are removed by seams
- **P-M3b-18** (Q15) fixed test values corrected against the code
- **P-M3b-19** (Q17) `RollbackAndContinue` sends `ROLLBACK`, keeps the session's mode and continues; later units' confirm-in-transaction form follows P-M3b-1's live state; the bar hides Retry when `rollback_and_continue` is true; test `rollback_and_continue_keeps_manual_mode`
- **P-M3b-20** (Q19, controller variant) Task 1a splits into Task 1a (manifests, `sql-ident`/`sql-safety`/SQL stubs, check-deps, xtask and fuzz skeletons) and Task 1c (core seams: session incl. P-M3b-3/-11/-12, driver fake, results, exec, settings store, profile fields); Tasks 1b and 1c start in parallel on Task 1a's manifest commit; Task 2 splits into Task 2a (surfaces 1–13, W0) and Task 2b (surfaces 14–24, any time before W8); no existing number changes
- **P-M3b-21** (Q20) Task 1b adds a `Results` key context on the results host root (`register_focus_context`, `shell/hooks.rs:228`); `results.next_tab`/`prev_tab` bind `ctrl-pagedown`/`ctrl-pageup` there; ledger 336's Ctrl+Tab/⌘3 is not taken (`ctrl-tab` is Workspace tab switching) and log ↑/↓ stay element-level keys (GC); the datagrip `ctrl-w` [Grid] shadowing `workspace.close_tab` and `cmd-f`/`ctrl-f` [Editor]/[Grid] shadowing …
- **P-M3b-22** (Q21) line drifts corrected (`crates/core/hacivat-results/src/window.rs:57`; MySQL `connect.rs:127` `.multi_statements(!req.read_only)`); `RunRequest` has 7 struct-literal sites and no `Default`, so Task 1c updates them (the plan's "compile unchanged" was false)
- **P-M3b-23** (Q22) `ConfirmPort` and `StatementClassifier` take a `Debug` supertrait (`RunRequest` derives `Debug`); `hacivat_driver::Notice` derives `Eq, Hash` (it rides in `RunEvent`) and reuses the existing `NoticeSeverity` (`typed.rs:70-78`)
- **P-M3b-24** (Q23) `ViewMap` and `HeightMap` lose `Copy`; Task 1b lists every by-value use (`view.rs`, `element/grid.rs`, `element/pointer.rs`, `model/keys.rs`, the app's `results/{copy,view}.rs`)
- **P-M3b-25** (Q24) covered by P-M3b-3 (`choose` returns a receiver)
- **P-M3b-26** (Q25) MySQL `BIT(n)` text is `"0101"` (as PG `bit`); the float test formats `Value::F32(0.1)` through `format_cell` with the shortest `f32` round trip, never widened to `f64`
- **P-M3b-27** (Q26) the count wrap trims trailing trivia after stripping `;` and the top-level `ORDER BY`
- **P-M3b-28** (Q27) the full-set line `cargo test -p sql-safety --test bypass_run_a` is dropped (`cargo test --workspace` runs it, `turkey-test` included)
- **P-M3b-29** (Q28) Task 8 owns `hacivat-results/src/writer.rs` (`admit()`, `:125-139`, where `Pause(GlobalBudget)` is decided and eviction hooks in); `evict_for` returns bytes released from the budget when the evicted `StoredChunk`s drop, not bytes freed at once; test `eviction_with_a_held_snapshot_releases_on_drop`
- **P-M3b-30** (Q29) there is no non-production confirm-in-transaction (R-M3-55 has no such row): surface 2 and Task 21 drop the "non-production, no countdown" variant
- **P-M3b-31** (Q30) the controller cleans and deletes every `target-core/m3a-*` directory before W0; the wave plan states the 3-slot and the 2-slot gpui schedules and the controller dispatches by whichever holds at each admission check
- **P-M3b-32** (Q31) `hacivat-exec`'s tokio features gain `rt` (the history writer's `spawn_blocking`) in Task 1a's table
- **P-M3b-33** (Q33, COUNTER-RULING) `SchemaCtx` holds no `sql_mode` (`session/schema.rs:15-23`) and the session never stores the context probe's answer
- **P-M3b-34** (Q34) GC line
- **P-M3b-35** (Q35) the read corpus includes MySQL/MariaDB `SHOW`, `DESCRIBE`/`DESC` and `EXPLAIN` (no ANALYZE) and SQLite read `PRAGMA`s; where `sqlparser` rejects one of those forms, the pre-scan's narrow lexer-level grammar (`SHOW <words> [LIKE 'x' | WHERE <read expr>]`, `DESCRIBE <qualified name> [<column>]`, `EXPLAIN [FORMAT=<w>] <statement classified recursively>`) classifies it, with no function call or subquery allowed, else …
- **P-M3b-36** (Q36) Task 24's "Unblocked by" adds Task 18 (no read-only `Deny` for Unknown reaches Run before run A proves 0 false denials) and states that it uses Task 23's bar only through Task 1b's `on_choice` stub
- **P-M3b-37** (Q37, PARTIAL COUNTER-RULING) icons are embedded by an `include_bytes!` table in `crates/ui/hacivat-ui-core/src/assets.rs` (`:11-12`), not by `rust-embed`, and `IconName::Search` and `ChevronsUpDown` already exist (`icon.rs:83, :96`); Task 1b adds only `Play`, `Pin`, `Sigma`, `Filter`, `ArrowUpDown`, `Snowflake` with their Lucide SVGs (ISC) under `hacivat-ui-core/assets/icons/lucide/` and six rows in that table, as a named exception, plus the …
- **P-M3b-38** (Q38) no change

### Pre-flight lessons applied in this plan (M3a P1–P28)

| Lesson | Where M3b applies it |
|---|---|
| P6 manifest table not exhaustive | Task 1a's Step 3 table lists every row M3b **and M3c** need; later tasks may not add one; a missing row is a named manifest exception ruled by the controller |
| P7 declared targets without files | Task 1a creates `fuzz/` (excluded workspace, its own `Cargo.toml`) and `xtask/src/{gen_pg_visitor,gen_fn_catalog,bench_sql,fetch_pg_regress}.rs` as exit-2 stubs with their dispatch lines |
| P11/P12 seams too thin, same-wave tasks editing the same file | Tasks 1a/1b/1c declare every module and stub that two tasks share, and the Waves table lists file ownership; the M3a lesson "unlisted cross-file edits" becomes the Global Constraint "every edit inside another task's file is a named exception" |
| P18 wrong test values | Values in the test blocks are fixed and computed against the real code (byte offsets counted in the plan's fixtures); Global Constraint "Test values are fixed in the plan" |
| P9 perf lines and Docker wrapper | Perf line names crates; Docker only through the wrapper |
| P15 cross-platform labels | Every new dialog test finds labels with `find_label_runs` (the F3 CI lesson: prompt labels split by the mnemonic on Windows/Linux) |
| P17 unprotected baselines | Both baselines are in the SDD folder; every shot-diff names one |
| P21 boot change flips tests | Task 26 names every app test that the production unlock flips and updates them without weakening |
| T1b review rounds | Task 1b's review checks every later task's Consumes line against the seams before merge (the M3a T1b ruling: the report's seam table is the authoritative seam map) |

### Carried items (every M3b routing in the ledger and reviews, with its owner)

| Item (source) | Owner |
|---|---|
| F2 review N1 (status-everything exemption only for pseudo-locales at 960) | T41 |
| F2 review N2 (require `#button-label-console-waiting-cancel` in waiting states + narrow test) | T41 |
| F2 review N3 (`toolbar.rs` docs :14-16, :75-76; ⋯ width reservation after M7) | T41 |
| F2 review N4 (narrow sweep covers running/cancelling, pending read-only chip, strip "Cursor open") | T41 |
| `ToolbarItem::narrow` below the per-locale minimum (ledger 411, R-M3-75) | T41 |
| F2 M8 = T34 M6: Reduced mode never gives way (still `GIVE_EDITOR = Whole(40)` in `connections/status.rs`); Ln/Col vs path order | T41 |
| F2 M9: raw px metrics (`WRAP_LINE_PX`, `WRAP_PAD_PX`, `COMPACT_*`, strip/menu gaps) become tokens | T43 (DESIGN.md) + T41 (code) |
| M2 carries C3 (startup scripts), C6 (MySQL autocommit flag), C7 (profile-wide idle, per-session liveness), C10 (side-connection cache), C11 (PG `SET TIME ZONE`), C13 (cancel by seq; out-of-band choices) | T12 (C13's `choose` in T11) |
| C8 (lost state from classifier effects) | T18 |
| C9 (queue cap 256, `QueueFull`) | T11 |
| C19 MariaDB bypass rows (ledger 294, `docker_m3a_readonly.rs:1082`) | T18 rows, T28 Docker |
| R-M3-9 ≤ 2 ms incremental relex budget | T5 (perf test), T30 (gate figure), T44 (real-app re-run) |
| B1-real re-run (and the M3a B3/B1-real PENDING-PO numbers if still open) | T30 (B3), T44 (B1-real) |
| DESIGN.md "Query, Results and Grid" (R-M3-30) | T43 |
| Per-source FetchMore coalescing (ledger 109) | T8 |
| `cap_raised` onto `ResultSource` (ledger 253 M7) | T8 |
| Copy through a permuted `ViewMap` (ledger 253 I5; `results/copy.rs:45`) | T36 |
| libpg_query stderr pipe and child processes (ledger 151, 164); prost depth limit fail-closed (ledger 151; ADR 0005 :291); `deparse_comments_for_query`/`is_utility_stmt` null-slice hazard | T9 (stderr ruling, PATCHES §2 if needed), T14 (depth) |
| Exec-comment bodies read both ways by the classifier (ledger 126/138) | T10 (interim), T14 (`AmbiguousCommentEnd` → Unknown) |
| T4 lexer: `x'…'`/`b'…'` inside comment bodies (ledger 200); client-command argument ends at end of line (ledger 274) | T5 |
| `UnitKind::Verbatim` refused; `RefuseReason::RunAsOne` (`units.rs:93`) | T15 (exec), T24 (app) |
| Selections inside `$$` bodies refused in M3a (ledger 226, PO note) | stays (M5 child documents); recorded in Deferred |
| PG float spelling for MySQL/SQLite (ledger 177); MySQL type labels generic (ledger 246); MySQL session `time_zone` in `NativeType::MySql` (ledger 194) | T7 |
| `ViewMap` permuted; fast-scroll/auto-size time slicing; per-type width bounds; frozen columns (ledger 179) | T35 |
| Tab exits a large grid only from the last loaded cell (ledger 183) | T40 (R-M3-69) |
| `selected_chars` O(selection); no drag autoscroll timer; horizontal thumb not draggable (ledger 207) | T34 |
| Grid edge autoscroll while the pointer is still (ledger 213) | T35 |
| RTL in the grid (ledger 213) | overruled → v1.x (R-M3-71) |
| MySQL driver's `CALL` text-tag check (ledger 279); drained MySQL result without final OK judged without a tag (seam change, ledger 279); `CALL` on read-only (ER15) | T18 |
| Startup option refused by a pooler (pgbouncer) (ledger 294) | T12 (falls back to `SET` after connect when the server rejects the startup option) |
| Preview vs PG oracle (ledger 307, 344) | R-M3-59 (stays out; M5) |
| The session refuses `ABORT` in a failed tx; a cancelled queued run shows Skipped without `with_runtime` (ledger 318) | T11 |
| Session publishes tx and drops the cursor on close/break (ledger 331) | T12 |
| Editor segment left of the version (mock wants right: `status_right` hook) (ledger 324, 355) | T41 |
| Sub-25 px blank sliver in the status bar (ledger 324) | T41 (check), T43 (shot) |
| Keymap rows for log line selection/↑↓, Ctrl+Tab/⌘3 between result tabs (ledger 336) | T1b rows, T25 bodies |
| Output log re-reads ≤ 10k lines per change (O(n)) → `SessionLog::lines_since` (ledger 336, 342, 362) | T16 (model), T25 (view) |
| MySQL chip shows SQLSTATE 25006, not vendor 1792 (ledger 340) | T25 |
| Run service drops the command tag (ledger 340) | T16 |
| DDL logs "Completed" on PG vs "0 rows affected" on MySQL/SQLite (ledger 340, PO) | T16 (one rule: DDL → `exec-completed` on every engine) |
| Toolbar `+` on `Truncated(FollowingResultSets)` vs strip (ledger 342) | T25 |
| "Nothing to run here" as toast, no status-sentence seam (ledger 347) | T41 |
| `set_display` applied once at attach (ledger 347) | T41 (re-applied on settings change) |
| Windows/Linux `physical_memory_bytes` only cross-checked in a throwaway crate (ledger 347) | T28 (CI asserts on all three OSes) |
| Editor commands don't run from the palette (ledger 355) | T41 |
| `FocusContexts` has no unregister (one handle leak per closed console) (ledger 355) | T41 |
| Empty-console placeholder line not drawn (ledger 355) | T41 |
| Run has no play icon (`play.svg` lacks an `IconName`) (ledger 355) | T1b (`IconName::Play`), T41 (use) |
| Per-session liveness; another console's idle tx gets no warning (ledger 370, 373) | T12 (R-M3-73) |
| PO notes: the default's loss ends every console session; idle-race toast wording (ledger 373) | T12 (wording), PO list |
| Re-clicking the already-active console tab after an explorer pick keeps M2 (ledger 331) | T41 |
| Multi-window globals and `ResultsEvent` double handling (ledger 331, 333, 360, 362) | Deferred (R-M3-72) |
| T29 notes: VoiceOver manual check; `word_starts`/char positions; 19 MB typing perf in release (ledger 298) | T40 (VoiceOver), T44 (perf), Deferred (word_starts → M9) |
| `hacivat-ui-editor/tests/marks.rs:158` "escalation badge is M3b" | T34 (gutter cancel-escalation badge) |
| `valfmt/display.rs:68` raw tooltip under a fraction cap; `Purpose` ignored in M3a | T38 (tooltip), T40 (`Purpose::A11y`) |
| `crates/core/hacivat-results/src/window.rs:57` largest jump (go to row, thumb drag) | T37 |
| `results/toolbar.rs:14` no M3b affordances | T25, T36–T39 |
| `[editor]` Settings-UI rows (`schema.rs:18, :100`) | T42 |
| `M3A_PENDING` deleted at the M3a gate | (M3a T37); M3b keeps no pending list: every M3b/M3c id is registered by its task and Task 1b's keymap rows are checked by `keymap_commands_are_registered` |

## Deferred work (explicit; beyond M3b and M3c)

| Item | To | Why |
|---|---|---|
| HV-EXEC-13 explain plan model and views | M4 | R-M3-21 |
| `FnCatalog` over `hacivat-meta` (unknown UDFs resolved by catalog); table row estimates | M4 | Needs the catalog; M3b uses curated allowlists (R-M3-56) |
| T2 preview tier, "Run as script" for huge files (the `StreamSplitter` consumer), the 20–100 MB paste confirmation, files and encodings | M4b | They need files; `StreamSplitter` lands in Task 5 with tests and no UI consumer |
| Oracle-in-the-preview; `SmallestSubqueryOrStatement` (`SubqueryOracle`); selections inside `$$` bodies (child documents); soft wrap; L2 highlighting; replace; parser folds; completion; own MySQL/SQLite parsers replacing the interim classifier path | M5 | R-M3-59; spec §6 M5 |
| Editing, table paging and lazy full-value fetch by row identity (HV-RES-06); origin badges; HTML/private clipboard flavours; export ("Export the rest"); width persistence across restarts; column reorder | M6 | spec §6 M6; R-M3-82 |
| AI write modes, intent statements, the MCP broker, `hacivat-audit` chain sinks, remote audit; C11 read-only credential; PgBouncer detection | M8 | Their consumer is M8 |
| NVDA walkthrough; the Windows/Linux IME matrix (PO-provided hosts); VoiceOver walkthrough gating 1.0; editor a11y `word_starts`/char positions | M9 / open item | L4; R-M3-7 |
| Multi-window correctness of the app-wide globals (R-M3-72) | M9 (or the first milestone that adds a window) | No second window exists |
| RTL in the grid; visual bidi movement; rich copy; inlays; variable row heights; spill to disk; the local value-list filter (HV-GRID-08) | v1.x | spec §4 deferrals; R-M3-71 |

## New UI surfaces (need mocks; Tasks 2a and 2b)

M3b and M3c add these surfaces. Task 2a mocks surfaces 1–13 in W0 and Task 2b surfaces 14–24 before W8, both in **`design/m3b-safe/`** (one mock for both halves; P-M3b-20), in the harman world, EN/TR × dark/light at 1280 × 800 and 960 × 600, with the M2a mock bar, a NOTES.md that is binding for both halves, and the copy table (TR marked for PO review). Surfaces 1–13 are M3b "Safe"; 14–24 are M3c "Complete".

1. **Gate confirmation dialog** (480 px alertdialog): production single DML; multi-unit with "statement k of n" and Execute / Execute all / Execute and don't ask again / Skip / Skip all / Cancel; type-to-confirm (NoWhere on production; `DROP SCHEMA`) with the mono field; MySQL DDL "cannot be rolled back"; Unknown/Routine on production; the non-production unsafe-WHERE calm variant; Warn (`FOR UPDATE`); the suppression label naming its scope; **Cancel focused, Execute Danger**; the TR button row at 960 (wrapping rule).
2. **Confirm-in-transaction dialog** (480 px): own-transaction form Commit / **Roll back (focused)** with the "Rolls back in 4:32" countdown (mono, `text-2`, once a second); savepoint form **Keep / Undo (focused)** naming its semantics ("Undo this statement; your transaction stays open"); "The server rolled back these changes" after a drop; production only (P-M3b-30).
3. **Transaction chip and menu** (editor toolbar): "Auto-commit" / "Manual" / "Smart" chip; menu with the modes, the Isolation submenu, disabled items with reasons ("Commit or roll back first"); read-only sessions always Auto (disabled chip with the reason).
4. **Transaction badge** (editor toolbar, replaces M3a's status-only segment as the primary surface): 7 px amber stamp, "Transaction · 3 statements · 2 min", Commit / Roll back (Default buttons, ink icons); the aborted form "Transaction aborted — roll back" with only Roll back; the status-bar segment as a button opening the chip menu; trust-signal rules (never clipped).
5. **Idle-transaction toast**: "Transaction open for 10 min on orders-prod; it may hold locks", Commit / Roll back / Keep; one per session, a newer one replaces it.
6. **Cancel escalation**: the slow-cancel toast at 10 s (Terminate on server / Deactivate); the second-press dialog (420 alertdialog, **Keep waiting focused**, Terminate on server, Deactivate as Danger with "the statement may continue on the server"); a failed terminate with the server's error verbatim; the gutter's escalation badge.
7. **Error-policy bar** (inline 32 px between editor and results, never a modal): "Statement 2 of 5 failed", Retry / Ignore / Ignore all / Stop; PG aborted form with **Roll back and continue**; keyboard order.
8. **Parameter prompt** (480 px dialog): one row per name (mono name, value field, type hint, NULL toggle, "remembered" marker); the Turkish `1.234` ambiguity flag; a template variable `${var}` refused with its diagnostic; PG `?` off.
9. **Chooser additions**: "Run as one" in the boundary-uncertain state; CHOOSE-06 "Customize…" opening the per-console policy popover (Inside: Ask / Smallest statement / Largest statement / Whole script; Selection: As separate / As one).
10. **Results additions**: tab titles from comments, `(2)` suffixes, the pin glyph and the tab context menu (Pin, Close, Close others); "untyped (run as one)" header note; result-set-limit note; **Count rows** in the result toolbar (Σ) with estimate vs exact; the eviction-closed strip ("Closed to free memory" + Re-run); the fetch-all confirmation dialog; notices and MySQL warnings in the log (severity glyphs, code chip with vendor code); "n events not shown".
11. **History dialog** (640 px): list rows (time, env mark, connection mono, outcome glyph, duration, rows, first SQL line, pin); search with Turkish-tolerant key; filters (this connection / all, outcome); preview pane with the masked SQL and a redacted marker; Enter inserts, ⌘↵ inserts and runs; empty and no-match states.
12. **Single-session checkbox** in the connection dialog's Advanced area and the console toolbar's "Shared session" chip.
13. **Production console after the unlock**: the M3a production read-only notice removed; hairline, swatch, capsule, env cell unchanged; the production Manual-mode default visible in the chip.
14. **Editor multi-cursor**: several carets and selections, the column (box) selection, the IME composition mirrored and the primary-only fallback.
15. **Folding**: the gutter fold column (chevrons on hover and focus), a folded statement's placeholder (`⋯ 12 lines`), folds and execution marks on the same row.
16. **Editor find panel**: the bar (query, Match case, Regex, Words, Turkish-tolerant toggle, "3 of 41", ↑↓, Select all occurrences, close), invalid regex, no matches, scrollbar hit markers.
17. **Gutter extras and line commands**: ▶ on hover, focused gutter buttons, the 3-outcome tooltip, mixed-line-endings indicator in the status bar with "Normalize…", "Undo selection".
18. **Grid model states**: sort indicator (levels 1 and 2), "Sorted locally — the database might order differently", the unsorted tail "N new rows (not sorted) — Re-sort", a frozen-left column (`seam-strong` edge), multi-rect selection, column grow/shrink handle.
19. **Grid find bar**: replaces the result toolbar's left group; Match case / Regex / Words / Filter rows / Turkish-tolerant; "k of N"; "may be incomplete"; scanning count; invalid regex.
20. **Go to row popover**: `<row>[:<column>]` placeholder, out-of-range, unknown column, hidden column, fetch-to-row confirmation above 1M rows.
21. **Transposed, record and text views**: the Segmented control (Table / Transposed / Text) with Transposed disabled above 100k rows; the record inspector (252 px, ▲▼, duplicate-name suffixes, "Show more"); the text view (Pretty table, line selection, >100k rows confirmation).
22. **Value viewer** (480 px popover): wrapped text, JSON/XML pretty toggle, hex for binary, truncated value with total length and "Re-run with a 64 MiB value limit", Copy raw value.
23. **Grid context menu**: cell, header and row-number variants (Copy, Copy with headers, Sort ascending/descending, Clear sort, Freeze up to here, Hide column, Show all, Auto-fit, Raw display, Open value viewer, Go to row); disabled items with reasons.
24. **Narrow panes** (`ToolbarItem::narrow`): the production chip's narrow form and the waiting item's two-line form at 120–219 px in every locale; the Reduced-mode editor cell that never gives way.

## Waves

| Wave | Tasks (agent model; **gpui** marked) | Unblocked by |
|---|---|---|
| **M3b "Safe"** | | |
| W0 | **T1a** manifests and SQL seams (opus) → on its first commit, in parallel: **T1b** UI and app seams (opus, **gpui**) · **T1c** core seams (opus) · **T2a** mocks 1–13 (opus, no build) | — (T1b, T1c: Task 1a's manifest commit) |
| W1 | **T3** `sql-ident` + function data (opus) · **T4** `sql-safety` core (opus) · **T5** incremental relex/resplit + `StreamSplitter` (opus) · **T6** params, templates, HvNorm1 (opus) · **T7** type matrix decoders (opus) | T1a, T1b, T1c merged |
| W2 | **T8** store: eviction, caps, coalescing (opus) · **T9** PG backend (opus) · **T10** interim backend (opus) · **T11** session runs: binds, multi-results, Run as one, error policy, queue cap (opus) · **T13** drivers: binds, multi-results, notices, warnings (opus) | T4, T3 (T9, T10); T7 (T13); — (T8, T11) |
| W3 | **T12** session M2 carries (opus) · **T14** orchestrator + classify pool (opus) · **T15** exec gate, params prepare, D-062 (opus; tests use a report builder, P-M3b-16) · **T16** run service: tabs, titles, error policy, count, auditor chain, log (opus) · **T17** history store (opus) | T11 (T12, T15, T16); T5, T9, T10 (T14); T4, T6 (T15); T13 (T16); T6 (T17) |
| W4 | **T18** SAFE-06 runs A/A′/B + read-only hook + C8 (opus) · **T19** confirm-in-tx, Smart, implicit commit (opus) · **T20** differential, fuzz, perf (opus) · **T21** confirmation dialogs + ConfirmPort (opus, **gpui**) · **T22** transaction UI (opus, **gpui**) | T14, T12 (T18); T11, T15 (T19); T5, T14 (T20); T15 (T21); T12 (T22) |
| W5 | **T23** cancel escalation + error bar (opus, **gpui**) · **T24** exec_ui wiring: params, classify, gate, Run as one, CHOOSE-06 (opus, **gpui**) · **T25** results: multi-result tabs, count, notices, eviction, log (opus, **gpui**) · **T28** Docker conformance M3b (opus) | T11, T16 (T23); T14, T15, T16, **T18**, T19, T21 (T24; T23 only through Task 1b's `on_choice` stub, P-M3b-36); T8, T16 (T25); T13, T16, T18, T19 (T28) |
| W6 | **T26** production read-write unlock (opus, **gpui**) · **T27** history dialog (sonnet, **gpui**) | T18, T19, T21, T24, T28 (T26); T17 (T27) |
| W7 | **T29** shots + cross-surface M3b (opus, **gpui**) → **T30** M3b gate (opus, with the controller) | W6 merged |
| **M3c "Complete"** | | |
| W8 | **T31** multi-cursor, IME mirroring, clipboard metadata (opus, **gpui**) · **T32** folding + chunked height map (opus, **gpui**) · **T35** grid model: ViewMap, frozen, multi-rect (opus, **gpui**) | M3b gate; **T2b** mocks 14–24 approved (T2b runs any time before W8, e.g. while the M3b gate waits) |
| W9 | **T33** editor find panel + scrollbar items (opus, **gpui**) · **T34** gutter extras, line commands, editor carries (opus, **gpui**) · **T36** local sort + collation + copy through ViewMap (opus, **gpui**) | T31 (T33); T31, T32 (T34: moving lines across folds); T35 (T36) |
| W10 | **T37** grid find + go to row (opus, **gpui**) · **T38** views + value viewer (opus, **gpui**) · **T40** grid AccessKit subtree (opus, **gpui**) | T36 (T37); T35 (T38, T40) |
| W11 | **T39** grid context menu, raw display, auto-fit (sonnet, **gpui**) · **T41** narrow toolbar items + chrome carries (opus, **gpui**) · **T42** Settings-UI rows (sonnet, **gpui**) | T36–T38 (T39); — (T41, T42) |
| W12 | **T43** shots, cross-surface M3c, DESIGN.md (opus, **gpui**) · **T44** B3/B1-real re-run + perf (opus, **gpui**) | W11 merged |
| W13 | **T45** M3c gate (opus, with the controller) | everything |

**Counts:** 48 tasks — M3b "Safe" 32 (T1a, T1b, T1c, T2a, T3–T30; W0–W7), M3c "Complete" 16 (T2b, T31–T45; W8–W13; T2b may run earlier). No existing task number changed (P-M3b-20).

**gpui schedules (P-M3b-31).** The admission rule (GC) gives 3 gpui slots while Docker is up and ≥ 60 GiB is free, 4 while Docker is down and ≥ 60 GiB is free, and **2 whenever < 60 GiB is free**. Free disk was ~65 GiB at the pre-flight, so the controller re-checks before every dispatch and follows whichever column holds; it first cleans every `target-core/m3a-*` directory (before W0) and each `target-core/m3b-tNN` right after its merge. Core agents are not capped by this rule (≥ 8 GiB each).

| Wave | gpui tasks | 3-slot schedule (dispatch order) | 2-slot schedule (dispatch order; the rest starts as a slot frees) |
|---|---|---|---|
| W0 | T1b | T1b | T1b |
| W4 | T21, T22 | T21 · T22 | T21 · T22 |
| W5 | T23, T24, T25 | T24 · T25 · T23 | T24 · T25 → T23 (T23 has no M3b dependent but T29) |
| W6 | T26, T27 | T26 · T27 | T26 · T27 |
| W7 | T29 | T29 | T29 |
| W8 | T31, T32, T35 | T31 · T35 · T32 | T31 · T35 → T32 |
| W9 | T33, T34, T36 | T36 · T33 · T34 | T36 · T33 → T34 (needs T32 too) |
| W10 | T37, T38, T40 | T37 · T38 · T40 | T37 · T38 → T40 |
| W11 | T39, T41, T42 | T41 · T39 · T42 | T41 · T39 → T42 |
| W12 | T43, T44 | T43 · T44 | T43 · T44 |

On the 2-slot schedule M3b loses about half a wave (T23 trails T24/T25 in W5) and M3c about three waves of wall time; no dependency changes, because the trailing task in each row is the one with the fewest dependents. The critical paths start their wave first in both schedules.

- **Core admission:** W1–W3 are core-only (5 agents each), as in M3a W1–W2.
- **Critical paths:** safety T1a → T4 → T9/T10 → T14 → T18 → T26; execution T1a → T1c → T11 → T15/T16 → T19 → T24 → T26; UI T1b → T21 → T24 → T26 → T29 → T30; M3c grid T35 → T36 → T37 → T43.
- **Docker:** up before W1 (the catalog read of Task 3 runs as a `hacivat-exec` Docker test through the wrapper on pg14–18/mysql80/84/mariadb, P-M3b-13), through W6; `pg17` again for T44.
- **CI proofs:** Task 1a merges after `build-test` ×3 (sqlparser and the FTS5 probe compile on Windows first) and a `fuzz/` resolution check; Tasks 1b and 1c after their reviews and `build-test` ×3 each.
- **Same-wave shared files** (P-M3b-17): none remain unnamed. W2 T11 (`run.rs`, `cursor.rs`, `actor.rs` run-gate wait, `run_gate.rs`) vs T13 (drivers) disjoint; W3 T12 (env/liveness/pool/cancel, `actor.rs` prewarm lines `:1814-1852`, `:2293`) is the only session task; W4 T18 (`readonly.rs`, `recovery.rs`) vs T19 (`run.rs`, `tx.rs`, `actor.rs:214, :682`, `confirm.rs`, `smart.rs`, `run_gate.rs`) disjoint thanks to Task 1c's plumbing (P-M3b-11); W5 T23 (`escalation.rs`, `error_bar.rs` incl. `on_choice`, `cancel.rs` lines) vs T24 (`orchestrator.rs`, `params_prompt.rs`, `policy_popover.rs`, `chooser.rs`, `sql-editor/preview.rs`) vs T25 (`results/*`, `exec_ui::fetch_policy_for` in `exec_ui/mod.rs`) disjoint; W9 T33 sole owner of `scrollbar.rs`; W10 T38 (`element/transpose.rs`, `valfmt/tooltip.rs`) vs T40 (`element/a11y_rows.rs`, `view.rs` `a11y_node`, `model/keys.rs`, `valfmt/a11y_text.rs`) disjoint.

**File ownership.** "Owns" means creates or modifies. Anything not listed belongs to Tasks 1a/1b/1c (every new crate, every module line, every stub, every manifest). A task editing a file another task owns lists it as a named exception with the lines.
- **T1a:** `crates/sql/{sql-ident,sql-safety}/**` (stubs, `data/` placeholders), `crates/sql/sql-split/src/{params,templates,fingerprint,incremental,stream}.rs` (stubs), `crates/sql/sql-lexer/src/incremental.rs` (stub), `crates/sql/sql-oracle-pg/src/tree.rs` (stub); manifests, `Cargo.lock`, `deny.toml`, `xtask/src/{check_deps,docker,main}.rs`, `xtask/src/{gen_pg_visitor,gen_fn_catalog,bench_sql,fetch_pg_regress}.rs` (stubs + dispatch), `fuzz/**` (skeleton with the copied `[patch.crates-io]` and its lockfile), ADR 0005 `m3b-t01a`.
- **T1c:** `crates/core/hacivat-exec/src/{gate,params,explain_wrap,suppress,classify_pool,audit,count}.rs`, `history/**` (stubs), `lib.rs`, `service.rs` (`RunRequest` fields), `log.rs`, `units.rs` (named items); `crates/core/hacivat-session/src/session/{verdict,confirm,smart,run_gate}.rs`, the new items and fail-closed arms in `session/{mod,tx,actor,run,cursor,readonly,recovery,pool}.rs` and `manager.rs` (P-M3b-3, -11, -12), `testing.rs`, `profile/{mod,store,rules}.rs`; `crates/core/hacivat-driver/src/{typed,driver,fake}.rs`; `crates/core/hacivat-results/src/{cursor,window,budget,evict}.rs`; `crates/core/hacivat-settings/src/{schema,store}.rs`; `crates/core/hacivat-values/src/{bind.rs,valfmt/{display,tooltip,a11y_text}.rs}`; ADR sections `m3b-t01c`.
- **T1b:** `crates/ui/hacivat-text/src/{multi,fold}.rs`, `display/height.rs` (`Chunked` variant stub); `crates/ui/hacivat-ui-editor/src/{find,fold_gutter}.rs` and `commands/{multi,find,lines}.rs` (stubs + the command rows in `commands/mod.rs`); `crates/ui/hacivat-ui-grid/src/model/{sort,filter,frozen,goto}.rs`, `view.rs` new `GridEvent`/`GridCommand` variants, `element/a11y_rows.rs` (stubs); `crates/ui/hacivat-ui/src/toolbar.rs` (`ToolbarItem::narrow` signature, stub = ignored), `icon.rs` (`IconName::{Play, Pin, Sigma, Filter, ArrowUpDown, Snowflake}`), `crates/ui/hacivat-ui-core/{assets/icons/lucide/*.svg,src/assets.rs}` (six icons, named exception, P-M3b-37); `exec_ui::error_bar::on_choice` and `exec_ui::fetch_policy_for` stubs; the `Results` key context; `crates/app/hacivat-app/src/exec_ui/{confirm,commit_prompt,error_bar,escalation,params_prompt,policy_popover}.rs`, `console/{tx_chip,tx_badge,idle_toast}.rs`, `results/{sort,find,goto,views,value_viewer,context_menu,count,pin}.rs`, `history/{mod,dialog,rows}.rs` (stubs + module lines + `install` lines); the keymap rows; the FTL stub blocks; `crates/app/hacivat-app/tests/m3b_routing.rs`.
- **T2a:** `design/m3b-safe/{index.html,NOTES.md}` (surfaces 1–13). **T2b:** the same files, M3c sections appended (surfaces 14–24).
- **T3:** `crates/sql/sql-ident/src/**` (bodies); `crates/sql/sql-safety/src/functions/**`, `data/{pg,mysql,mariadb,sqlite}/{allow,deny,operators,types,set_allow}.toml`, `data/functions.digest`; `xtask/src/gen_fn_catalog.rs` (offline `--check`); `crates/core/hacivat-exec/tests/gen_fn_catalog.rs` (Docker read, P-M3b-13).
- **T4:** `crates/sql/sql-safety/src/{effects,report,reason,policy,destructive,gate_class,limits}.rs`, `data/profiles.toml`; FTL block `## m3b-t04-safety` in `hacivat-core.ftl`.
- **T5:** `crates/sql/sql-lexer/src/{incremental,relex}.rs`, the lexer follow-ups in `sql-lexer/src/{comments,strings,cursor}.rs` (named lines); `crates/sql/sql-split/src/{incremental,stream}.rs`, `document.rs` (internals of `apply`, same API); `crates/app/hacivat-sql-editor/src/worker.rs` (named lines: use `apply` instead of a rebuild).
- **T6:** `crates/sql/sql-split/src/{params,templates,fingerprint}.rs`; `crates/sql/sql-testkit/corpus/datagrip/param.toml`.
- **T7:** `crates/core/hacivat-driver-pg/src/decode/**`, `crates/core/hacivat-driver-mysql/src/decode.rs`, `crates/core/hacivat-driver-sqlite/src/decode.rs`; `crates/core/hacivat-values/src/{logical,native}.rs` (named items), `valfmt/display.rs` (float spelling per engine, type labels).
- **T8:** `crates/core/hacivat-results/src/{store,budget,cursor,window,writer,evict}.rs` (P-M3b-29).
- **T9:** `crates/sql/sql-safety/src/{backend/{mod,pg},visit/pg_generated.rs}`, `data/pg/statements.toml`; `crates/sql/sql-oracle-pg/src/tree.rs`; `xtask/src/gen_pg_visitor.rs`; `third_party/pg_query/PATCHES.md` §2 (named exception, only if the stderr ruling needs it); a `ci.yml` block.
- **T10:** `crates/sql/sql-safety/src/backend/{interim,prescan}.rs`, `data/{mysql,sqlite}/statements.toml`, `data/mysql/known_gaps.toml`; `crates/sql/sql-testkit/corpus/read/*.sql` (SHOW/DESCRIBE/EXPLAIN/PRAGMA included, P-M3b-35).
- **T11:** `crates/core/hacivat-session/src/session/{run,cursor}.rs`; named lines in `session/{actor,mod,run_gate}.rs` (`Run` arms, the run's `RunGate` wait, queue cap, `ABORT`).
- **T12:** `crates/core/hacivat-session/src/session/{env,liveness,pool,cancel}.rs`, `manager.rs` (named lines), `tests/carries_m3b.rs`.
- **T13:** `crates/core/hacivat-driver-{pg,mysql,sqlite}/src/typed.rs` (and the bind/notice lines of `exec.rs`/`open.rs`/`connect.rs`, named); `third_party/tokio-postgres` §2 only if needed (named exception).
- **T14:** `crates/sql/sql-safety/src/{classify,backend/plan,fingerprint}.rs`; `crates/core/hacivat-exec/src/classify_pool.rs`.
- **T15:** `crates/core/hacivat-exec/src/{gate,params,explain_wrap,suppress,units}.rs`.
- **T16:** `crates/core/hacivat-exec/src/{service,observer,status,log,titles,count,audit}.rs`.
- **T17:** `crates/core/hacivat-exec/src/history/**`.
- **T18:** `crates/sql/sql-testkit/bypass/**`; `crates/sql/sql-safety/tests/bypass_run_a.rs`; `crates/core/hacivat-session/src/session/{readonly,recovery}.rs` (`step0b` body, `observed_sql_mode`, C8); `crates/core/hacivat-session/tests/{readonly_classifier,lost_state_m3b,docker_m2b_bypass}.rs` (the last: rows to TOML, named exception); `crates/core/hacivat-exec/tests/{cross_crate_equalities,docker_m3b_bypass}.rs` (P-M3b-13); `crates/core/hacivat-guard/src/plan.rs` (`plan_for_with`, named); `crates/app/hacivat-app/src/connections/boot.rs` (installs the classifier, named line).
- **T19:** `crates/core/hacivat-session/src/session/{confirm,smart}.rs` (bodies), named lines in `run.rs`, `tx.rs`, `run_gate.rs`, `actor.rs` (`:214`, `:682`).
- **T20:** `fuzz/**`, `crates/sql/sql-testkit/src/differential/**`, `crates/sql/sql-safety/tests/mutation_props.rs`, `crates/core/hacivat-exec/tests/docker_mutation_fuzz.rs`, `xtask/src/{bench_sql,fetch_pg_regress}.rs`; `ci.yml` blocks `fuzz-sql`, `sql-differential`.
- **T21:** `crates/app/hacivat-app/src/exec_ui/{confirm,commit_prompt}.rs`; `tests/confirm.rs`.
- **T22:** `crates/app/hacivat-app/src/console/{tx_chip,tx_badge,idle_toast}.rs`, named lines in `console/toolbar.rs` and `connections/status.rs` (tx segment as a button), `connections/dialog/form.rs` (single-session checkbox, named lines); `tests/tx_ui.rs`.
- **T23:** `crates/app/hacivat-app/src/exec_ui/{escalation,error_bar}.rs` (incl. `on_choice`), named lines in `exec_ui/cancel.rs`; `tests/escalation.rs`.
- **T24:** `crates/app/hacivat-app/src/exec_ui/{orchestrator,params_prompt,policy_popover,chooser}.rs`; `crates/app/hacivat-sql-editor/src/preview.rs` (verbatim units, named lines); `tests/exec_ui_m3b.rs`.
- **T25:** `crates/app/hacivat-app/src/results/{tabs,strip,toolbar,output_view,count,pin,view}.rs` (named items), `exec_ui::fetch_policy_for` body in `exec_ui/mod.rs`; `tests/results_m3b.rs`.
- **T26:** every `production_read_only` site listed in Task 26; `console/{toolbar,view}.rs` notice removal; the M2b/M3a app tests that flip.
- **T27:** `crates/app/hacivat-app/src/history/**`; `tests/history_ui.rs`.
- **T28:** `crates/core/hacivat-exec/tests/docker_m3b_matrix.rs` and its block in `tests/common/mod.rs`.
- **T29:** `crates/app/hacivat-app/tests/shots/m3b.rs`, the `Surface::M3b` arm in `shots.rs`, `tests/cross_surface_m3b.rs`.
- **T30:** `docs/benchmarks/{m3b.md,po-demo-m3b.md}`.
- **T31:** `crates/ui/hacivat-text/src/{multi,selection,ime}.rs` (named items), `crates/ui/hacivat-ui-editor/src/{commands/multi,ime,clipboard}.rs`.
- **T32:** `crates/ui/hacivat-text/src/{fold,display/height}.rs`, `crates/ui/hacivat-ui-editor/src/fold_gutter.rs`, named lines in `element.rs` (:737) and `a11y.rs`; `crates/app/hacivat-sql-editor/src/folds.rs` (new; `LanguageService::folding_ranges`).
- **T33:** `crates/ui/hacivat-ui-editor/src/{find,commands/find,scrollbar}.rs` (sole owner of `scrollbar.rs` in W9); `crates/app/hacivat-app/src/console/find_bar.rs`.
- **T34:** `crates/ui/hacivat-ui-editor/src/{commands/lines,gutter,marks,mouse}.rs` (named items).
- **T35:** `crates/ui/hacivat-ui-grid/src/model/{selection,columns,frozen,filter}.rs`, `element/**` (frozen layer, autoscroll), `view.rs` (named).
- **T36:** `crates/ui/hacivat-ui-grid/src/model/sort.rs`; `crates/app/hacivat-app/src/results/{sort,copy}.rs`; `crates/core/hacivat-values/src/valfmt/compare.rs` (new).
- **T37:** `crates/ui/hacivat-ui-grid/src/model/goto.rs`; `crates/app/hacivat-app/src/results/{find,goto}.rs`.
- **T38:** `crates/app/hacivat-app/src/results/{views,value_viewer}.rs`, `crates/ui/hacivat-ui-grid/src/element/transpose.rs`, `crates/core/hacivat-values/src/valfmt/tooltip.rs`.
- **T39:** `crates/app/hacivat-app/src/results/context_menu.rs`.
- **T40:** `crates/ui/hacivat-ui-grid/src/element/a11y_rows.rs`, named lines in `view.rs` (`a11y_node`) and `model/keys.rs` (Tab rule); `crates/core/hacivat-values/src/valfmt/a11y_text.rs`.
- **T41:** `crates/ui/hacivat-ui/src/toolbar.rs` (`narrow`), `crates/app/hacivat-app/src/console/toolbar.rs`, `connections/status.rs` (named items), `commands/palette.rs`, `shell/hooks.rs` (`unregister_focus_context`), `tests/{toolbar_narrow,status_follow}.rs`, `tests/shots/m3a.rs` (N1/N2 checks, named).
- **T42:** `crates/app/hacivat-app/src/settings_ui/**` (named rows).
- **T43:** `crates/app/hacivat-app/tests/shots/m3c.rs`, `tests/cross_surface_m3c.rs`, `DESIGN.md` (new section).
- **T44:** `crates/app/hacivat-app/src/bench/**` (named modes), `docs/benchmarks/m3c-bench-notes.md`.
- **T45:** `docs/benchmarks/{m3c.md,po-demo-m3c.md}`; `design/baselines/m3b/**` after PO sign-off.

---

**How to read the test blocks** (as in M3a). Each task's Step 1 lists its tests by name, with the assertion written out where the spec or a ruling fixes a value. A test shown with an empty body (`{}`) or a comment carries its assertion in its name and in the case it cites; the implementer writes the body with real asserts (F9), and the reviewer rejects a tautological or comment-only body. Helper names inside test bodies are test-local helpers the task writes in its own test files or its block of a shared `common`/`support` module.

## M3b "Safe" (W0–W7)

### Task 1a: Manifests, SQL seams, check-deps, xtask and fuzz skeletons (W0)

**Build:** `CARGO_TARGET_DIR=<main>/target-core/m3b-t01a`, `CARGO_INCREMENTAL=0`, base `m3b`. **Two commits:** the first holds only `Cargo.toml`, `Cargo.lock`, every crate's manifest, `deny.toml`, the new crates' `lib.rs` module lines and `fuzz/Cargo.toml` + `fuzz/Cargo.lock` (the controller dispatches **Tasks 1b and 1c in parallel** on it, P-M3b-20); the second holds the SQL seams.

**Files:**
- Create crates (`[lints] workspace = true`, `license`, `description`, `repository`, `publish = false`, every module line in `lib.rs`): `crates/sql/sql-ident/**`, `crates/sql/sql-safety/**` (modules `effects, report, reason, policy, destructive, gate_class, limits, functions/{mod,data}, backend/{mod,pg,interim,prescan,plan}, visit/{mod,pg_generated}, classify, fingerprint`; `data/` with empty TOML files carrying a `# owner: Task NN` header, including `data/{pg,mysql,mariadb,sqlite}/set_allow.toml` for Task 3, P-M3b-6; `data/functions.digest` for Task 3's offline freshness test).
- Create files in existing SQL crates (stubs, fail closed): `sql-split/src/{params,templates,fingerprint,incremental,stream}.rs`; `sql-lexer/src/incremental.rs`; `sql-oracle-pg/src/tree.rs`.
- Workspace: `Cargo.toml` (`exclude = ["third_party", "fuzz"]`; workspace deps `sqlparser`, `icu_collator`; path deps `sql-ident`, `sql-safety`), `Cargo.lock`, `deny.toml` (no change expected; `cargo deny check` must pass), `xtask/src/check_deps.rs` (`BackendOutsideOwner` `sqlparser` arm + fixture tests), `xtask/src/{gen_pg_visitor,gen_fn_catalog,bench_sql,fetch_pg_regress}.rs` (exit-2 stubs with their dispatch arms in `xtask/src/main.rs`; `gen-fn-catalog` keeps only the offline `--check`, P-M3b-13), `fuzz/{Cargo.toml,Cargo.lock,README.md,fuzz_targets/.gitkeep}` (own workspace; **its `Cargo.toml` copies the root `[patch.crates-io]` entries with `../third_party/...` paths and `fuzz/Cargo.lock` is generated from the root lock**, P-M3b-15), `.gitignore` (`target-pg-regress/`).
- ADRs: `## m3b-t01a — SQL seams` appended to 0005.
- Tests: `crates/sql/sql-safety/tests/stubs.rs`; `xtask` `check_deps` fixture tests.

**Step 3 manifest table (exhaustive for M3b and M3c; no later task edits a manifest; amended by P-M3b-13 and -32):**

| Crate | Adds |
|---|---|
| workspace | `sqlparser = { version = "=0.64.0", default-features = false, features = ["std", "visitor", "recursive-protection"] }`; `icu_collator = "2.4.0"`; path deps `sql-ident`, `sql-safety` |
| `sql-ident` | `sql-dialect`, `sql-text`, `smallvec` |
| `sql-safety` | `sql-ident`, `sql-dialect`, `sql-text`, `sql-lexer`, `sql-split`, `sql-oracle-pg`, `sqlparser`, `bitflags`, `smallvec`, `toml` + `serde` (data loader, `include_str!` tables), `twox-hash`; dev: `proptest`, `serde_json`, `sql-testkit` (integration tests only, GC) |
| `sql-split` | (none new; `twox-hash` present) |
| `sql-oracle-pg` | (none new; re-exports `pg_query::protobuf` from `tree.rs`) |
| `sql-testkit` | `sql-safety`, `sql-oracle-pg` (the differential, Task 20) — dev-only crate; `SqlReachesCore` still applies, so nothing here reaches `crates/core` |
| `hacivat-exec` | `sql-safety`, `sql-ident`, `rusqlite` (workspace, `bundled`), `sha2`; tokio features gain `rt` (the history writer's `spawn_blocking`, P-M3b-32); dev: `tempfile`, `proptest` (the app passes paths and settings values in; `hacivat-exec` reads no settings) |
| `hacivat-session` | (none new: the classifier is injected, R-M3-83; the equality and A′ tests live in `hacivat-exec/tests`, P-M3b-13) |
| `hacivat-values` | `icu_collator` (for `valfmt::compare`, Task 36) |
| `hacivat-app` | `regex` (find matchers, Tasks 33/37); collation through `hacivat-values`' re-export |
| `hacivat-ui-grid`, `hacivat-ui-editor`, `hacivat-ui-core` | (none new) |
| `xtask` | (none new; `serde_json` and `toml` present; no DB client — Docker reads run as `hacivat-exec` tests through the wrapper, P-M3b-13) |
| `fuzz/` (excluded workspace) | `libfuzzer-sys` (`(MIT OR Apache-2.0) AND NCSA`; NCSA accepted only because the workspace is excluded and never shipped, recorded in ADR 0005 `m3b-t20`), path deps `sql-lexer`, `sql-split`, `sql-safety`; `[patch.crates-io]` copied from the root |

Any further row discovered later is a controller-ruled named manifest exception (P6 lesson), never an implementer edit.

**Interfaces (Produces).** Derives as in M3a (data types `Debug, Clone, PartialEq` plus `Eq, Hash` where fields allow; `Copy` for small fieldless enums; `Default` where named). Positions: `TextSize` in `crates/sql`, `usize` elsewhere. Each type lives in the module file of the task that fills it (P22).

```rust
// ===== crates/sql/sql-ident (T3 bodies) =====
pub struct Ident { pub text: Arc<str>, pub quoted: bool }
pub struct QualifiedName { pub parts: SmallVec<[Ident; 3]> }          // schema.name or db.schema.name
pub fn fold(d: DialectId, raw: &str, quoted: bool) -> Arc<str>;        // ASCII-only (INV-3): PG lowercases unquoted ASCII; MySQL/SQLite keep text
pub fn ident_eq(d: DialectId, a: &Ident, b: &Ident) -> bool;          // PG exact after fold; MySQL/SQLite ASCII-case-insensitive for functions; never locale casing
pub fn parse_qualified(d: DialectId, text: &str) -> Option<QualifiedName>;
// stub: fold returns raw unchanged; ident_eq is byte equality; parse_qualified None

// ===== crates/sql/sql-safety (T4 core, T9 PG, T10 interim, T3 data, T14 classify) =====
bitflags! { pub struct EffectSet: u32 {
    const READ = 1; const WRITE_DATA = 1<<1; const WRITE_TEMP = 1<<2; const DDL = 1<<3; const DDL_TEMP = 1<<4;
    const TXN_CONTROL = 1<<5; const SESSION_STATE = 1<<6; const LOCK_ROW = 1<<7; const LOCK_TABLE = 1<<8; const ADVISORY_LOCK = 1<<9;
    const PRIVILEGE = 1<<10; const ADMIN = 1<<11; const EXTERNAL_IO = 1<<12; const PROCESS_EXEC = 1<<13; const SEQUENCE = 1<<14;
    const XID_CONSUME = 1<<15; const PREPARE = 1<<16; const ROUTINE_CALL = 1<<17; const NOTIFY = 1<<18; const UNKNOWN = 1<<31; } }
bitflags! { pub struct RiskFlags: u16 { const EXEC_COMMENT = 1; const DYNAMIC_SQL = 2; const LEX_MODE_CHANGE = 4; const EXPLAIN_ANALYZE = 8; const VOLATILE_FN = 16; } }
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)] pub enum CoarseClass { Read, Session, Write, Ddl, Admin, Unknown }
#[non_exhaustive] #[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum ReasonCode { NotImplemented, LexError, ClientCommand, TemplateVariable, NoStatement, MultiStatement, AmbiguousBoundary,
    ParseError, ParserDisagreement, TooComplex, SyntaxNewerThanAgent, UnsupportedDialect, FnDenylisted, FnUnknown, OperatorUnknown,
    CastUnknown, ExecutableComment, LexModeChange, PrivilegeChange, DataModifyingCte, LockingClause, ExplainAnalyze, DynamicSql,
    ProceduralCode, TxnControl, SessionState, ExternalFile, ExternalNetwork, ProcessExec, AdminControl, XidConsume, SequenceAdvance,
    WriteTemp, HintExecTime, HintSetVar, RoutineCall, SetNotAllowlisted }
impl ReasonCode { pub fn fluent_id(self) -> &'static str; }               // "safety-reason-<kebab>", a literal per arm (P24)
pub struct Reason { pub code: ReasonCode, pub range: Option<TextRange>, pub arg: Option<Arc<str>> }
pub enum ObjectKind { Table, View, Function, Sequence, Schema, Database, Role, Other }
pub struct ObjectRef { pub name: QualifiedName, pub kind: ObjectKind }
pub struct TouchedObjects { pub reads: Vec<ObjectRef>, pub writes: Vec<ObjectRef>, pub calls: Vec<ObjectRef> }
#[non_exhaustive] pub enum DestructiveKind { NoWhere, AlwaysTrue, JoinWithoutOn, Drop { cascade: bool }, Truncate, DropColumn, DropConstraint, AlterType, MergeDelete, UpdateLimitNoOrder }
pub struct DestructiveReason { pub kind: DestructiveKind, pub range: TextRange, pub target: Option<ObjectRef> }
pub enum StmtKind { Select, Values, Table, Insert, Update, Delete, Merge, Copy, Call, Do, Explain { analyze: bool }, CreateTable, CreateOther, Alter, Drop, Truncate, Grant, Revoke, Set, Show, Begin, Commit, Rollback, Savepoint, Lock, Prepare, Execute, Listen, Notify, Use, Pragma, Other }
pub enum Confidence { Exact, Conservative }
pub struct StmtReport { pub range: TextRange, pub kind: StmtKind, pub effects: EffectSet, pub flags: RiskFlags, pub class: CoarseClass,
    pub touched: TouchedObjects, pub reasons: Vec<Reason>, pub destructive: Vec<DestructiveReason>, pub confidence: Confidence }
pub struct BackendVersion { pub name: &'static str, pub version: &'static str }   // "libpg_query 18.1.0", "sqlparser 0.64.0", "lexer-prescan 1"
pub struct SafetyReport { pub items: Vec<StmtReport>, pub request_class: CoarseClass, pub request_effects: EffectSet, pub flags: RiskFlags,
    pub fingerprint: Fingerprint /* sql_split::fingerprint */, pub backends: SmallVec<[BackendVersion; 3]>, pub pre_decision: Option<Vec<Reason>>,
    pub classified_len: u32, pub classified_hash: u64 }   // XXH3-64 of the exact bytes classified (P-M3b-2); the gate compares them with final_text
pub enum Expect { SingleStatement, Script }
pub struct Limits { pub max_bytes: usize /* 1 MiB */, pub max_tokens: u32 /* 200_000 */, pub max_nesting: u16 /* 256 */, pub max_exec_comment_views: u8 /* 16 */ }
pub trait FnCatalog: Send + Sync { fn lookup(&self, f: &QualifiedName, arity: Option<u16>) -> FnLookup; }   // None in M3b (M4)
pub enum FnLookup { Known { volatile: bool, writes: bool }, Unknown }
#[derive(Clone)] pub struct ClassifyCtx { pub dialect: DialectCtx, pub flavor: ClientFlavor, pub expect: Expect,
    pub catalog: Option<Arc<dyn FnCatalog>>, pub fn_allowlist: Arc<[Arc<str>]>, pub search_path: Arc<[Arc<str>]>, pub limits: Limits }
pub fn classify(sql: &str, ctx: &ClassifyCtx) -> SafetyReport;            // pure, sync, never panics. STUB: Unknown + pre_decision [NotImplemented], classified_len/hash filled
pub enum Profile { ReadOnly, ReadOnlyPlusExplain, ReadWriteDml, Ddl, Admin }
pub enum Actor { Human, AiAgent, Automation }
pub enum Verdict { Allow, Confirm(Vec<Reason>), Deny(Vec<Reason>) }
pub struct Decision { pub verdict: Verdict, pub class: CoarseClass }
pub fn decide(r: &SafetyReport, p: Profile, a: Actor) -> Decision;       // STUB: Deny([NotImplemented])
// P-M3b-4: GateClass gains Admin; the total order and the default (Unknown) are Task 4's
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)] pub enum GateClass { Read, Lock, Dml, DmlUnsafe, DdlDestructive, DdlOther, Routine, Session, Tx, Admin, Unknown }
pub fn gate_class(r: &StmtReport) -> GateClass;                          // STUB: Unknown
pub fn opens_write(r: &StmtReport) -> bool;                              // STUB: true (Smart would open a tx: the safe direction)
pub fn denied_function_names(d: DialectId) -> Box<dyn Iterator<Item = &'static str>>;   // for the guard equality test (P-M3b-13); STUB: empty
pub fn set_allowed(d: DialectId, name: &str) -> bool;                    // harmless-SET allowlist (P-M3b-6); STUB: false
pub(crate) trait Backend { fn analyse(&self, sql: &str, items: &[TextRange], ctx: &ClassifyCtx) -> Vec<StmtReport>; }

// ===== crates/sql/sql-oracle-pg/src/tree.rs (T9 bodies) =====
pub use pg_query::protobuf as pb;                                        // the only re-export of pg_query types (BackendOutsideOwner)
pub enum ScanKind { Keyword, Ident, Literal, Operator, Param, Comment, Other }
pub fn parse_tree(sql: &str) -> Result<pb::ParseResult, PgError>;       // on the parser thread; 8 MiB cap; depth ≤ 256; a prost decode-depth failure is Err. STUB: Err
pub fn scan_tokens(sql: &str) -> Result<Vec<(TextRange, ScanKind)>, PgError>;   // STUB: Err
pub fn fingerprint(sql: &str) -> Result<u64, PgError>;                  // STUB: Err

// ===== crates/sql/sql-split (T6 params/templates/fingerprint, T5 incremental/stream) =====
pub struct ParamSettings { pub pg_question_marks: bool /* false, D-08 */, pub colon_params: bool /* true */, pub in_ddl: bool /* false */ }
pub enum PlaceholderKind { Positional(u32), Question(u32), Named(Arc<str>) }      // names compared exactly
pub struct Placeholder { pub range: TextRange, pub kind: PlaceholderKind, pub bindable: bool }
pub enum TemplatePattern { DollarBrace, Percent, Angle }                          // detect only (D-053)
pub struct TemplateVar { pub range: TextRange, pub name: Arc<str>, pub pattern: TemplatePattern }
pub struct BindPlan { pub names: Vec<Arc<str>>, pub slots: Vec<SmallVec<[u32; 2]>> }   // name i → driver slots (MySQL repeats a name)
pub struct RewrittenSql { pub text: String, pub plan: BindPlan, pub edits: Vec<(TextRange, TextRange)> }   // (in unit, in rewritten)
pub enum ParamDiag { TemplateVariable, QuestionMarkOff, ParamInDdl, MixedStyles, TooManyBinds }
pub struct ParamScan { pub placeholders: Vec<Placeholder>, pub templates: Vec<TemplateVar>, pub rewritten: Option<RewrittenSql>, pub diags: Vec<(TextRange, ParamDiag)> }
pub fn scan_params(unit: &str, ctx: &DialectCtx, head: &StmtHead, s: &ParamSettings) -> ParamScan;
// STUB: every ':', '$', '?' byte is reported as an unbindable placeholder; rewritten None
pub enum FingerprintAlgo { HvNorm1, PgQuery }
pub struct Fingerprint { pub algo: FingerprintAlgo, pub hash: u64, pub normalized: Arc<str>, pub version: u16 }
pub fn hv_norm1(text: &str, ctx: &DialectCtx) -> Fingerprint;          // STUB: hash of the raw text, algo HvNorm1, version 0
pub struct StreamSplitter { /* LexState + prepass state */ }
pub enum StreamItem { Statement { text: String, offset: u64, confidence: Confidence }, Client { text: String, offset: u64, supported: bool },
    Ambiguous { offset: u64, reason: AmbiguityReason }, TooLong { offset: u64 } }
impl StreamSplitter { pub fn new(ctx: DialectCtx, flavor: ClientFlavor) -> Self; pub fn feed(&mut self, chunk: &str) -> Vec<StreamItem>; pub fn finish(self) -> Vec<StreamItem>; }
// STUB: buffers everything and returns one Ambiguous { offset: 0, reason: UnknownBlock } at finish
pub const STREAM_STATEMENT_MAX: usize = 64 << 20;
// sql-lexer (T5): checkpoints become public
impl Lexed { pub fn checkpoint_before(&self, at: TextSize) -> (TextSize, LexState); }   // STUB: (0, LexState::Initial)
```

- [ ] **Step 1: Write the failing tests.**
```rust
// crates/sql/sql-safety/tests/stubs.rs
#[test] fn classify_stub_is_unknown_with_a_pre_decision() { let r = classify("SELECT 1", &ctx_pg()); assert_eq!(r.request_class, CoarseClass::Unknown); assert!(r.pre_decision.is_some()); assert_eq!(r.classified_len, 8); }
#[test] fn decide_stub_denies() { assert!(matches!(decide(&classify("SELECT 1", &ctx_pg()), Profile::ReadOnly, Actor::Human).verdict, Verdict::Deny(_))); }
#[test] fn every_reason_code_has_a_literal_fluent_id() { for c in all_reason_codes() { assert!(c.fluent_id().starts_with("safety-reason-")); } }
#[test] fn no_sqlparser_type_in_the_public_api() { assert!(!public_api_text().contains("sqlparser::")); }   // grep over src/lib.rs re-exports + pub items
// xtask check_deps tests
#[test] fn sqlparser_outside_safety_is_flagged() { let md = fx_at(&[("sql-split", "crates/sql", &["sqlparser"])]); assert_eq!(rule_of(&md, "sql-split"), Rule::BackendOutsideOwner); }
#[test] fn sqlparser_in_safety_is_allowed() {}
```
- [ ] **Step 2:** Run `cargo test -p sql-safety --test stubs` and `cargo test -p xtask`. Expected: FAIL.
- [ ] **Step 3: Implement** the manifests (first commit; `cargo metadata` clean; `cargo deny --all-features check licenses bans sources` green; `cd fuzz && cargo metadata --locked` resolves `pg_query` to `../third_party/pg_query`), then the SQL types and stubs, the `check_deps` arm, xtask stubs (exit 2, naming the task that fills them), the ADR section.
- [ ] **Step 4:** Run `cargo test -p sql-safety -p sql-split -p sql-lexer -p sql-oracle-pg -p sql-ident`, `cargo xtask check-deps`, `cargo clippy --workspace --all-targets -- -D warnings`. Expected: PASS; no M3a test changed.
- [ ] **Step 5: Commit** `feat(seams): M3b manifests and SQL seams — sql-ident, sql-safety, params, incremental/stream, oracle tree; check-deps sqlparser arm; xtask and fuzz skeletons`.

**Review:** security-sensitive. Probes: a `sqlparser` type leaking; the fuzz workspace resolving the registry `pg_query`; every later task's Consumes line (Tasks 3–30) that names an SQL item checked against these names before merge.

### Task 1c: Core seams (session, driver fake, results, exec, settings store, profile fields, values) (W0)

**Build:** `CARGO_TARGET_DIR=<main>/target-core/m3b-t01c`. **Base (named exception):** `git checkout -B m3b-seams-core <Task 1a's first commit>`, in parallel with Task 1b; rebase onto `m3b` after Task 1a merges (mechanical: Task 1a's lines kept).

**Files:**
- Create (stubs, fail closed): `hacivat-exec/src/{gate,params,explain_wrap,suppress,classify_pool,audit,count}.rs`, `hacivat-exec/src/history/{mod,schema,writer,search,redact}.rs`; `hacivat-session/src/session/{verdict,confirm,smart,run_gate}.rs`; `hacivat-results/src/evict.rs`; `hacivat-values/src/valfmt/{tooltip,a11y_text}.rs` (stubs dispatched from `display.rs`'s `Purpose` match so Tasks 38 and 40 never share `display.rs`, P-M3b-17).
- Modify (minimal arms, fields and seams; **every literal and `match` the new variants and fields break, listed in the report** — Q3; at least the 7 `RunRequest` struct literals, P-M3b-22):
  - `hacivat-session/src/session/{mod,tx,actor,run,cursor,readonly,recovery,pool}.rs`, `manager.rs`, `testing.rs`, `profile/{mod,store,rules}.rs`:
    - the new items below;
    - `Command::{Choose, CommitDecision}` arms in `Ladder::command` (`actor.rs:2112`) filling the `RunGate` slot (P-M3b-3);
    - `TxOwner::is_hacivats` replacing the five ownership checks at `tx.rs:101`, `actor.rs:1297`, `actor.rs:1709`, `run.rs:233`, `cursor.rs:619` (P-M3b-12);
    - `Core.classifier` plumbed `ManagerConfig` → `pool.rs` spawn → `spawn_supervised` (`actor.rs:195`) → `Core`, and `Core::step0b` (stub `Ok(())`) called right before `guarded` (`actor.rs:112`) and `guarded_query` (`run.rs:329`) (P-M3b-11);
    - `track_statement(t, sql, effects)` with `None` at every call site (`run.rs:263` and the `Exec` path) (P-M3b-11);
    - `Core.observed_sql_mode: Option<String>` field (filled by Task 18, P-M3b-33).
  - `hacivat-driver/src/{typed,driver,fake}.rs`; `hacivat-results/src/{cursor,window,budget}.rs`; `hacivat-exec/src/{lib,service,log,units}.rs`.
  - `hacivat-settings/src/{schema,store}.rs`: the four new tables are read and written through `section`/`read_key_or`/`write_key_or` like `[editor]` (`store.rs:76-87, :115-132`); `results.max_value_bytes` is a newtype `ValueLimit` that clamps to 64 KiB..=64 MiB with a diagnostic, as `TabSize` does (`schema.rs:126-140`) (P-M3b-9).
  - `hacivat-values/src/{bind.rs,valfmt/display.rs}` (the `Purpose` dispatch lines).
- ADRs: `## m3b-t01c — core seams` appended to 0002, 0004 and 0006.
- Tests: `crates/core/hacivat-session/tests/m3b_interfaces.rs`, `crates/core/hacivat-exec/tests/m3b_stubs.rs`, `crates/core/hacivat-results/tests/m3b_types.rs`, `crates/core/hacivat-settings` (store tests), `crates/core/hacivat-session` profile store tests.

**Interfaces (Produces):**
```rust
// ===== crates/core/hacivat-session (T11 run, T12 carries, T18 hook, T19 confirm/smart) =====
// session/verdict.rs
// P-M3b-1: TxThenConfirm carries no form; the session decides savepoint vs own BEGIN at execution time from live state
pub enum UnitVerdict { Read, Write, Unknown, TxThenConfirm, ExplainAnalyzeRollback, ImplicitCommit }   // default Unknown
bitflags! { pub struct SessionEffects: u16 { const SESSION_STATE = 1; const WRITE_TEMP = 2; const ADVISORY_LOCK = 4; const LOCK_TABLES = 8; const PREPARE = 16; const OPENS_WRITE = 32; const DDL = 64; } }
pub struct SafetyDenial { pub class_id: &'static str, pub reasons: Vec<&'static str> }   // Fluent ids; Clone + Eq + Hash
pub enum RoCheck { Allow, Deny(SafetyDenial) }
pub trait StatementClassifier: Send + Sync + std::fmt::Debug + 'static {
    fn check_read_only(&self, kind: DbmsKind, sql: Arc<str>, allow: Arc<[Arc<str>]>) -> BoxFuture<'static, RoCheck>;   // allow = the profile's function allowlist (P-M3b-10)
    fn effects(&self, kind: DbmsKind, sql: Arc<str>) -> BoxFuture<'static, Option<SessionEffects>>;   // C8
}
// session/run_gate.rs (P-M3b-3: out-of-band answers through a shared slot, like StopSignal)
pub(crate) struct RunGate { /* Mutex<Option<Pending>> + Notify */ }
pub(crate) enum Pending { Choice { run_seq: u64, unit: UnitId }, Commit { run_seq: u64, unit: UnitId } }
pub(crate) enum Answer { Choice(RunChoice), Commit(CommitAnswer) }
// session/mod.rs additions
pub enum DenyLayer { Input, Protocol, Transaction, Engine, Function, Classifier }       // + Classifier
pub enum ExecErrorKind { /* M3a variants */ SafetyDenied(SafetyDenial), QueueFull }
#[non_exhaustive] pub enum ErrorPolicy { #[default] StopOnError, Ask }
pub enum RunChoice { Retry, Ignore, IgnoreAll, Stop, RollbackAndContinue }
pub enum CommitAnswer { Commit, RollBack }
pub struct RunUnit { pub id: UnitId, pub sql: Arc<str>, pub protocol: Protocol, pub suspendable: bool,
    pub binds: Arc<[BindValue]>, pub verdict: UnitVerdict, pub effects: SessionEffects, pub masked: Arc<str>, pub max_result_sets: u32 /* 1 until T11 */, pub cell_cap: Option<u32> }
pub struct RunSpec { /* M3a fields */ pub confirm_deadline: Duration /* 300 s; tests inject */ }
pub enum RunEvent { /* M3a variants */
    NeedsChoice { unit: UnitId, error: ExecError, rollback_and_continue: bool },
    NeedsCommit { unit: UnitId, affected: Option<u64>, deadline: Instant, savepoint: bool },   // `savepoint` reports the form the session chose
    CommitDecided { unit: UnitId, answer: CommitAnswer, by_deadline: bool },
    Notice { unit: UnitId, notice: hacivat_driver::Notice } }
pub(crate) enum Command { /* M3a variants */
    Choose { run_seq: u64, choice: RunChoice, reply: oneshot::Sender<bool> },
    CommitDecision { run_seq: u64, unit: UnitId, answer: CommitAnswer, reply: oneshot::Sender<bool> } }
// `Request` gains NO variant (P-M3b-3).
impl SessionHandle {
    pub fn choose(&self, run_seq: u64, c: RunChoice) -> oneshot::Receiver<bool>;              // true when a run waited on that seq
    pub fn commit_decision(&self, run_seq: u64, unit: UnitId, a: CommitAnswer) -> oneshot::Receiver<bool>;
}
// STUBS: no run waits yet, so both answer false; a RunUnit with verdict TxThenConfirm or ExplainAnalyzeRollback is refused with
// ExecErrorKind::Internal until Task 19; max_result_sets > 1 is clamped to 1 until Task 11; Smart stays NeedsClassifier until Task 19.
// session/tx.rs
pub enum TxOwner { #[default] None, User, Cursor, Guard, Confirm }                     // + Confirm
impl TxOwner { pub fn is_hacivats(self) -> bool { matches!(self, Self::Cursor | Self::Guard | Self::Confirm) } }
pub enum TxError { /* M3a variants */ ImplicitCommitUnconfirmed }                     // P-M3b-1 (3)
// Core (pub(crate)): classifier: Option<Arc<dyn StatementClassifier>>, observed_sql_mode: Option<String>,
//   async fn step0b(&self, sql: &str) -> Result<(), ExecError>   // STUB Ok(()); Task 18 fills
// recovery.rs: pub fn track_statement(t: &mut Tracked, sql: &str, effects: Option<SessionEffects>)   // None everywhere until Task 18
// manager.rs
pub struct ManagerConfig { /* M3a fields */ pub classifier: Option<Arc<dyn StatementClassifier>> }   // None = M2b/M3a behaviour (R-M3-83)
// profile/mod.rs (P-M3b-10): defaults keep M2b profile files parsing; load/save/validation in profile/{store,rules}.rs
pub struct Profile { /* M3a fields */ pub startup_script: Option<String>, pub read_only_fn_allowlist: Vec<String>, pub record_history: bool /* true */ }

// ===== crates/core/hacivat-driver (T13 bodies; FakeDriver working) =====
pub struct Notice { pub severity: NoticeSeverity /* the existing enum, typed.rs:70-78 */, pub code: Option<Arc<str>>, pub text: Arc<str> }   // Eq + Hash (P-M3b-23); text ≤ 16 KiB, cut at a char boundary
pub enum QueryEvent { /* M3a variants */ Notice(Notice) }
// RawConn: new provided method (default Ok(vec![]))
fn warnings(&mut self) -> BoxFuture<'_, Result<Vec<Notice>, RawError>>;           // MySQL SHOW WARNINGS (T13)
// FakeDriver (working): FakeQuery gains `notices: Vec<Notice>`, `warnings: Vec<Notice>`; FakeTypedCall gains `Binds(Vec<BindValue>)`
// (logged right after `Query`); `FakeDriver::binds_seen() -> Vec<Vec<BindValue>>`. Tests assert sent statements on `call_log()`
// (typed queries are not in `executed()`, which records only execute/batch text, `fake.rs:745`; P-M3b-18).

// ===== crates/core/hacivat-results (T8 bodies) =====
pub enum CloseReason { /* M3a variants */ Evicted }
pub trait ResultSource { /* M3a methods */ fn cap_raised(&self) -> bool { false } fn set_visible(&self, _visible: bool) {} }
impl MemoryBudget { pub fn evict_for(&self, need: u64) -> u64; }                  // bytes released from the budget; STUB: 0
pub struct EvictionRegistry { /* weak stores, last visible time */ }

// ===== crates/core/hacivat-exec (T14 pool, T15 gate/params, T16 service/audit/count, T17 history) =====
// classify_pool.rs
#[derive(Debug)] pub struct ClassifyPool { /* 2 std threads, 16 MiB stacks */ }
impl ClassifyPool {
    pub fn new(threads: usize) -> Arc<Self>;
    pub fn classify(&self, sql: Arc<str>, ctx: ClassifyCtx) -> oneshot::Receiver<SafetyReport>;   // STUB: answers the sql-safety stub
}
impl StatementClassifier for ClassifyPool { /* STUB: Deny(NotImplemented) */ }
// params.rs
pub struct OffsetMap { /* piecewise-linear unit → document */ }
impl OffsetMap { pub fn identity(start: usize) -> Self; pub fn to_doc(&self, unit_offset: usize) -> usize; }
pub struct PreparedUnit { pub unit: ExecUnit, pub final_text: Arc<str>, pub binds: Arc<[BindValue]>, pub masked: Arc<str>, pub offsets: OffsetMap }
pub struct ParamPrompt { pub name: Arc<str>, pub occurrences: u32, pub remembered: Option<BindValue> }
pub enum Prepared { Ready(Vec<PreparedUnit>), NeedsValues(Vec<ParamPrompt>), Refused { unit: u32, at: Range<usize>, diag: ParamDiag } }
pub trait ParamValueStore { fn get(&self, profile: &ProfileId, name: &str) -> Option<BindValue>; fn put(&self, profile: &ProfileId, name: &str, v: BindValue); }
pub fn prepare_units(units: Vec<ExecUnit>, ctx: &DialectCtx, s: &ParamSettings, values: &dyn ParamValueStore, profile: &ProfileId) -> Prepared;
// STUB: Refused { diag: TooManyBinds } for any unit with a ':', '$' or '?' byte, else Ready with identity maps
// gate.rs
pub struct ConnSafety { pub engine: DbmsKind, pub env: Option<EnvTag>, pub read_only: bool, pub tx_status: TxStatus, pub tx_owner: TxOwner, pub tx_mode: TxMode, pub confirm_unsafe_where: bool }
pub struct GateReason { pub fluent_id: &'static str, pub args: Vec<(&'static str, Arc<str>)> }
pub enum GateDecision { Run, Warn(Vec<GateReason>), Confirm { reasons: Vec<GateReason>, type_to_confirm: Option<Arc<str>>, irreversible: bool, suppressible: bool },
    RunInTxThenConfirm { reasons: Vec<GateReason> }, ExplainAnalyzeRollback, Deny(GateReason) }   // no form: the session picks it (P-M3b-1)
pub struct GateItem { pub unit: u32, pub decision: GateDecision, pub verdict: UnitVerdict, pub effects: SessionEffects, pub class: GateClass, pub fingerprint: u64 }
pub struct GatePlan { pub items: Vec<GateItem> }
impl GatePlan { pub fn needs_prompt(&self) -> bool; pub fn first_denied(&self) -> Option<&GateItem>; }
pub fn gate(units: &[PreparedUnit], reports: &[SafetyReport], conn: &ConnSafety, suppressed: &SuppressSet, console: u64) -> GatePlan;   // STUB: Deny every unit
pub trait ConfirmPort: Send + Sync + std::fmt::Debug + 'static {          // Debug: RunRequest derives Debug (P-M3b-23)
    fn confirm(&self, q: ConfirmQuestion) -> oneshot::Receiver<ConfirmAnswer>;
    fn commit_decision(&self, q: CommitQuestion) -> oneshot::Receiver<CommitAnswer>;
}
pub struct ConfirmQuestion { pub run: RunId, pub unit: u32, pub index: u32, pub total: u32, pub sql_masked: Arc<str>, pub reasons: Vec<GateReason>,
    pub type_to_confirm: Option<Arc<str>>, pub irreversible: bool, pub suppressible: bool, pub warn_only: bool, pub production: bool, pub connection: Arc<str> }
pub enum ConfirmAnswer { Execute, ExecuteAll, ExecuteAndSuppress, Skip, SkipAll, Cancel }   // any close = Cancel
pub struct CommitQuestion { pub run: RunId, pub unit: UnitId, pub affected: Option<u64>, pub deadline: Instant, pub savepoint: bool /* from NeedsCommit */, pub production: bool, pub connection: Arc<str> }
// explain_wrap.rs, suppress.rs
pub fn explain_analyze_inner(report: &StmtReport) -> Option<GateClass>;
pub struct SuppressSet { /* (console, fingerprint) */ } impl SuppressSet { pub fn insert(&mut self, console: u64, fp: u64); pub fn contains(&self, console: u64, fp: u64) -> bool; }
// service.rs: RunRequest gains (every one of its 7 literal sites is updated with the M3a values)
pub struct RunRequest { /* M3a fields */ pub prepared: Option<Vec<PreparedUnit>>, pub plan: Option<GatePlan>, pub policy: ErrorPolicy,
    pub run_as_one: bool, pub history: Option<HistoryMeta>, pub confirm: Option<Arc<dyn ConfirmPort>> }
impl ExecService { pub fn with_history(self, h: Arc<HistoryStore>) -> Self; pub fn with_classifier(self, p: Arc<ClassifyPool>) -> Self;
    pub fn count(&self, session: &SessionHandle, sql: String) -> oneshot::Receiver<Result<u64, ExecError>>; }   // STUB: Err(Internal)
// count.rs
pub fn count_sql(doc: &DocSnapshot, unit: &ExecUnit) -> Option<String>;             // STUB: None
// log.rs: LogMsg gains (literal fluent ids per arm, P24)
//   Notice { severity: NoticeSeverity, code: Option<Arc<str>>, text: Arc<str> }  → "exec-notice-<severity>"
//   ResultSetLimit { total: u32, limit: u32 } → "exec-result-set-limit";  ImplicitCommit → "exec-implicit-commit"
//   SafetyDenied { class_id: &'static str } → "exec-deny-classifier";  ConfirmCommitted { rows: Option<u64> } → "exec-confirm-committed"
//   ConfirmRolledBack { by_deadline: bool, savepoint: bool } → "exec-confirm-rolled-back" / "-deadline" / "exec-confirm-undone"
//   ConfirmStatementFailed → "exec-confirm-statement-failed" ("…your transaction is intact", P-M3b-5)
//   SkippedByUser { n: u32 } → "exec-skipped-by-user";  Retried { n: u32 } → "exec-retried";  Evicted { rows: u64 } → "cursor-closed-evicted"
impl SessionLog { pub fn lines_since(&self, seq: u64) -> (Arc<[LogLine]>, bool /* gap: lines were dropped from the ring */); }
// audit.rs
pub trait SyncAuditor: Send { fn on_event(&mut self, ev: &RunEvent); }
pub struct AuditorChain { /* sync auditors + async sinks (capacity 10_000, drop counter) */ }
// history/mod.rs (T17 bodies)
pub enum HistoryOrigin { Console, File, GridDml, Mcp, RunConfig, Import }
pub enum Outcome { Ok, Failed, Cancelled, Denied }
pub struct HistoryMeta { pub profile: ProfileId, pub console: u64, pub private: bool, pub origin: HistoryOrigin }
pub struct HistoryRecord { pub ts_start_ms: i64, pub duration_ms: u32, pub profile: ProfileId, pub database: Option<String>, pub schema: Option<String>,
    pub console: u64, pub origin: HistoryOrigin, pub statement_masked: Arc<str>, pub text_hash: [u8; 32], pub fingerprint: u64,
    pub params_masked: Option<String>, pub outcome: Outcome, pub rows_affected: Option<u64>, pub rows_fetched: Option<u64>,
    pub error_code: Option<String>, pub error_message: Option<String> /* ≤ 4 KiB */, pub tx_mode: TxMode, pub run: Option<RunId>, pub redacted: bool }
pub struct HistoryConfig { pub per_source: u32 /* 10_000 */, pub max_age_days: u32 /* 90 */, pub statement_cap: usize /* 64 KiB */ }
pub struct HistoryQuery { pub text: Option<String>, pub profile: Option<ProfileId>, pub outcome: Option<Outcome>, pub since_ms: Option<i64>, pub limit: u32 }
pub struct HistoryHit { pub id: i64, pub record: HistoryRecord, pub run_count: u32, pub pinned: bool }
pub enum HistoryError { Disabled, Io(String), Corrupt }
pub struct HistoryStore { /* single writer on the blocking pool */ }
impl HistoryStore {
    pub fn open(path: PathBuf, cfg: HistoryConfig) -> BoxFuture<'static, Result<Arc<HistoryStore>, HistoryError>>;   // STUB: Err(Disabled)
    pub fn sink(&self) -> mpsc::Sender<Arc<HistoryRecord>>;
    pub fn search(&self, q: HistoryQuery) -> BoxFuture<'static, Result<Vec<HistoryHit>, HistoryError>>;
    pub fn recall(&self, profile: ProfileId, prefix: String, n: u32) -> BoxFuture<'static, Result<Vec<Arc<str>>, HistoryError>>;
    pub fn pin(&self, id: i64, pinned: bool) -> BoxFuture<'static, Result<(), HistoryError>>;
    pub fn shutdown(&self, grace: Duration) -> BoxFuture<'static, ()>;
}

// ===== crates/core/hacivat-settings (P-M3b-9: hand-written loader, no serde) =====
pub struct ExecSettings { pub confirm_unsafe_where: bool /* true */, pub error_policy_ask: bool /* true */ }
pub struct ValueLimit(u32);   // clamps to 65_536..=67_108_864 with a diagnostic; default 1_048_576
pub struct ResultsSettings { pub max_value_bytes: ValueLimit }
pub struct HistorySettings { pub enabled: bool /* true */ }
pub struct GridSettings { pub raw_display: bool /* false (L3 default) */ }
// Settings gains `exec`, `results`, `history`, `grid`; store.rs reads/writes `[exec]`, `[results]`, `[history]`, `[grid]` with
// section/read_key_or/write_key_or; a missing table means the defaults; an M3a file loads with no diagnostic.

// ===== crates/core/hacivat-values/src/valfmt (P-M3b-17) =====
// display.rs: Purpose::Tooltip → tooltip::text(..) (Task 38), Purpose::A11y → a11y_text::text(..) (Task 40); STUBS return the Cell text (M3a behaviour)
```

- [ ] **Step 1: Write the failing tests.**
```rust
// crates/core/hacivat-session/tests/m3b_interfaces.rs
#[tokio::test] async fn choose_without_a_waiting_run_answers_false() { let (h, _) = session(fake_pg()); assert!(!h.choose(7, RunChoice::Ignore).await.unwrap()); }
#[tokio::test] async fn choose_is_served_while_a_request_runs() { /* a gated FakeQuery holds the run; choose(seq_other) answers false within the run — proves the command reaches Ladder::command, not the FIFO */ }
#[tokio::test] async fn tx_then_confirm_unit_fails_closed_until_task_19() { let (h, f) = session(fake_pg()); let before = f.call_log().len(); let t = h.run(spec_with_verdict("UPDATE t SET a=1", UnitVerdict::TxThenConfirm));
    assert!(matches!(unit_end(&t, 0).await, UnitEnd::Failed(e) if e.kind == ExecErrorKind::Internal)); assert!(f.call_log()[before..].is_empty()); }
#[tokio::test] async fn manager_default_has_no_classifier() { assert!(ManagerConfig::default().classifier.is_none()); }
#[test] fn hacivats_owners() { assert!(TxOwner::Confirm.is_hacivats() && TxOwner::Cursor.is_hacivats() && TxOwner::Guard.is_hacivats()); assert!(!TxOwner::User.is_hacivats() && !TxOwner::None.is_hacivats()); }
#[tokio::test] async fn fake_driver_logs_binds_after_the_query() { /* typed query with 2 binds → typed_log [Query(..), Binds([Null, Typed{..}])] */ }
#[tokio::test] async fn fake_driver_emits_scripted_notices() {}
#[test] fn m2b_profile_file_loads_with_new_defaults() { /* record_history true, no startup script, empty allowlist; round-trips */ }
// crates/core/hacivat-exec/tests/m3b_stubs.rs
#[test] fn gate_stub_denies_every_unit() { let p = gate(&[prepared("SELECT 1")], &[stub_report()], &conn_dev(), &SuppressSet::default(), 1); assert!(matches!(p.items[0].decision, GateDecision::Deny(_))); }
#[test] fn prepare_stub_refuses_a_colon() { assert!(matches!(prepare_units(units(&["SELECT :a"]), &pg(), &ParamSettings::default(), &NoValues, &pid()), Prepared::Refused { .. })); }
#[tokio::test] async fn history_stub_is_disabled() { assert!(matches!(HistoryStore::open(tmp(), HistoryConfig::default()).await, Err(HistoryError::Disabled))); }
#[test] fn fts5_is_compiled_in() { let c = rusqlite::Connection::open_in_memory().unwrap(); c.execute_batch("CREATE VIRTUAL TABLE t USING fts5(x)").unwrap(); }
#[test] fn run_request_m3a_shape_still_builds() { let r = RunRequest { prepared: None, plan: None, ..m3a_request() }; assert_eq!(r.policy, ErrorPolicy::StopOnError); }
// crates/core/hacivat-results/tests/m3b_types.rs
#[test] fn evict_stub_frees_nothing() { assert_eq!(MemoryBudget::new(1 << 20, 1 << 20).evict_for(1 << 19), 0); }
#[test] fn source_cap_raised_defaults_false() {}
// crates/core/hacivat-settings store tests (P-M3b-9)
#[test] fn m3a_settings_file_loads_with_new_defaults_and_no_diagnostic() { let o = load(&write_tmp(M3A_SETTINGS_TEXT)); assert!(o.diagnostics.is_empty()); assert!(o.settings.exec.confirm_unsafe_where); assert_eq!(o.settings.results.max_value_bytes.get(), 1 << 20); }
#[test] fn value_limit_clamps_with_a_diagnostic() { let o = load(&write_tmp("[results]\nmax_value_bytes = 1\n")); assert_eq!(o.settings.results.max_value_bytes.get(), 65_536); assert_eq!(o.diagnostics.len(), 1); }
#[test] fn new_tables_round_trip_through_save() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-exec -p hacivat-session -p hacivat-results -p hacivat-settings -p hacivat-values --no-run`, then the tests. Expected: FAIL.
- [ ] **Step 3: Implement** the types and stubs, the `RunGate` slot (`Command::{Choose, CommitDecision}` answered by `Ladder::command` while a request runs and by the idle loop otherwise), the classifier plumbing and `step0b`, the five `is_hacivats` sites, the profile fields with their store rules, the settings tables, the FakeDriver knobs (working). Every M3a behaviour stays: `RunUnit.max_result_sets` 1, `verdict` `Unknown`, `binds` empty in `ExecService::run`'s M3a path.
- [ ] **Step 4:** Run the crates' suites, `cargo test -p hacivat-app --features scene-check,pseudo` (M3a app tests unchanged and green), `cargo clippy --workspace --all-targets -- -D warnings`. Expected: PASS; no M3a test changed except literal additions listed in the report.
- [ ] **Step 5: Commit** `feat(seams): M3b core seams — session verdicts, run gate for out-of-band answers, classifier plumbing, Hacivat-owned transactions, driver notices and binds, eviction, gate/params/history stubs, settings tables, profile fields`.

**Review:** security-sensitive. Probes: any stub that lets a statement run that M3a would not have run; `ManagerConfig::default()` changing an M2b behaviour; `Command::Choose` queued behind the running request; a sixth ownership check missed (`grep -n "TxOwner::\(Cursor\|Guard\|User\)" crates/core/hacivat-session/src`); every later task's Consumes line that names a core item checked against these names before merge.

### Task 1b: UI and app seams (editor, grid, toolbar, app modules, keymap rows, FTL stubs) (W0, gpui)

**Build:** `CARGO_TARGET_DIR=<main>/target-core/m3b-t01b`. **Base (named exception):** `git checkout -B m3b-seams-ui <Task 1a's first commit>`, in parallel with Task 1c; rebase onto `m3b` after Task 1a merges. It consumes no Task 1c item (Ui crates never reach Core), so the two never wait on each other.

**Files:**
- `crates/ui/hacivat-text/src/{multi,fold}.rs` (stubs), `display/height.rs` (`HeightMap::Chunked` variant + constructor stub), `lib.rs` (module lines, re-exports).
- `crates/ui/hacivat-ui-editor/src/{find,fold_gutter}.rs`, `commands/{multi,find,lines}.rs` (stub bodies returning `false`/no-op), `commands/mod.rs` (command rows, block `// ## m3b-t01b-seams`), `service.rs` (`LanguageService::folding_ranges` with a default body), `view.rs` (named seam methods below, stub bodies), `lib.rs`.
- `crates/ui/hacivat-ui-grid/src/model/{sort,filter,frozen,goto}.rs` (stubs), `model/selection.rs` (`ViewMap` variants; M3a behaviour for `Identity`), `view.rs` (new `GridEvent`/`GridCommand` variants, `GridView` seam methods), `element/a11y_rows.rs` (stub), `commands.rs` (grid command rows).
- `crates/ui/hacivat-ui/src/toolbar.rs` (`ToolbarItem::narrow`, stored, unused until Task 41), `icon.rs` (`IconName::{Play, Pin, Sigma, Filter, ArrowUpDown, Snowflake}`; `Search` and `ChevronsUpDown` already exist; block `// ## m3b-t01b-seams`); **named exception** `crates/ui/hacivat-ui-core/assets/icons/lucide/{play,pin,sigma,filter,arrow-up-down,snowflake}.svg` (Lucide, ISC) and their six rows in the `include_bytes!` table of `crates/ui/hacivat-ui-core/src/assets.rs` (P-M3b-37).
- `crates/app/hacivat-app/src/exec_ui/{confirm,commit_prompt,error_bar,escalation,params_prompt,policy_popover}.rs`, `console/{tx_chip,tx_badge,idle_toast,find_bar}.rs`, `results/{sort,find,goto,views,value_viewer,context_menu,count,pin}.rs`, `history/{mod,dialog,rows}.rs`: module lines, `install` lines in the owning `mod.rs`/`lib.rs`, stub bodies that render nothing and answer the safe value. **Same-wave seams (P-M3b-17):** `exec_ui::error_bar::on_choice(console: ConsoleId, ev: &RunEvent, cx: &mut App)` (stub answers `Stop`; Task 23 fills it in its own file, Task 24's orchestrator only calls it) and `exec_ui::fetch_policy_for(console: ConsoleId, cx: &App) -> FetchPolicy` in `exec_ui/mod.rs` (stub: M3a's `FetchPolicy::default()`; Task 25 fills it, Task 24 only calls it); `GridView::set_transposed` and the `element/{transpose,a11y_rows}.rs` hooks called from `element/grid.rs`, so Tasks 38 and 40 fill their own files. **The `Results` key context (P-M3b-21):** registered on the results host root through `register_focus_context(cx, handle, "Results")` (`shell/hooks.rs:228`), with its entry in the dispatcher's context stack named in the report.
- Keymaps: block `## m3b-t01b-seams` in `crates/ui/hacivat-ui/keymaps/default-{macos,windows,linux}.toml` and `presets/datagrip.toml` (rows below).
- Locales: block `## m3b-t01b-seams` in `locales/{en,tr}/hacivat-ui.ftl` with only the command titles of the new ids (`hv-cmd-*`, `# Review: PO` on TR) — surface strings belong to the filling tasks.
- Tests: `crates/app/hacivat-app/tests/m3b_routing.rs`; `crates/ui/hacivat-ui-editor/tests/m3b_seams.rs`; `crates/ui/hacivat-ui-grid/tests/m3b_seams.rs`.
- `ViewMap` and `HeightMap` lose `Copy` (`selection.rs:26`, `height.rs:6`): every by-value use in `view.rs`, `element/grid.rs`, `element/pointer.rs`, `model/keys.rs` and the app's `results/{copy,view}.rs` becomes a borrow or a clone, each listed (P-M3b-24).
- Every `match` and literal the new `ExecMarkState`, `EditorEvent`, `GridEvent`, `GridCommand` and `ViewMap` variants break (at least `exec_ui/marks.rs:380 mark_of`, `hacivat-ui-editor/src/a11y.rs:686 exec_announcement`, `marks.rs` paint, `model/keys.rs:60 apply_command`, `results/view.rs` grid event handling): minimal arms with M3a behaviour, each listed in the report (Q3).

**Interfaces (Produces):**
```rust
// ===== hacivat-text (T31 multi, T32 fold) =====
pub fn add_caret(sel: &SelectionSet, at: usize) -> SelectionSet;
pub fn add_caret_vertical(buf: &TextBuffer, sel: &SelectionSet, down: bool, tab_size: u8) -> SelectionSet;
pub fn select_next_occurrence(buf: &TextBuffer, sel: &SelectionSet) -> Option<SelectionSet>;
pub fn select_all_occurrences(buf: &TextBuffer, sel: &SelectionSet, cap: usize /* 10_000 */) -> SelectionSet;
pub fn column_selection(buf: &TextBuffer, anchor: (u32, usize), head: (u32, usize), tab_size: u8) -> SelectionSet;   // (line, display column)
pub fn split_into_lines(buf: &TextBuffer, sel: &SelectionSet) -> SelectionSet;
impl EditorModel { pub fn undo_selection(&mut self) -> Option<Committed>; pub fn redo_selection(&mut self) -> Option<Committed>; }   // selection-only history (spec Ctrl+U)
pub struct FoldRange { pub range: Range<usize>, pub kind: FoldKind }
pub enum FoldKind { Statement, Block, Comment, Region }
pub struct FoldSet { /* sorted, non-overlapping-or-nested folded ranges, mapped through changes */ }
impl FoldSet { pub fn fold(&mut self, r: Range<usize>) -> bool; pub fn unfold_at(&mut self, at: usize) -> bool; pub fn unfold_all(&mut self) -> bool;
    pub fn map(&mut self, c: &ChangeDesc); pub fn folded(&self) -> &[Range<usize>]; pub fn hidden_lines(&self, buf: &TextBuffer) -> Vec<Range<u32>>; }
pub enum HeightMap { Uniform { lines: u32, line_height: f64 }, Chunked { /* T32 */ } }
impl HeightMap { pub fn chunked(lines: u32, line_height: f64, hidden: &[Range<u32>]) -> Self; }   // STUB: Uniform (no folds)

// ===== hacivat-ui-editor (T31–T34) =====
pub trait FindMatcher { fn find(&self, hay: &str, from: usize) -> Option<Range<usize>>; fn find_back(&self, hay: &str, before: usize) -> Option<Range<usize>>; }
pub struct FindStatus { pub current: Option<u32>, pub total: Option<u32> /* None while counting */, pub capped: bool }
impl EditorView {
    pub fn set_find(&mut self, m: Option<Rc<dyn FindMatcher>>, cx: &mut Context<Self>);
    pub fn find_step(&mut self, back: bool, cx: &mut Context<Self>) -> Option<Range<usize>>;
    pub fn select_all_matches(&mut self, cap: u32, cx: &mut Context<Self>) -> u32;
    pub fn find_status(&self) -> FindStatus;
    pub fn folds(&self) -> &FoldSet;
    pub fn toggle_fold_at(&mut self, line: u32, cx: &mut Context<Self>) -> bool;
}
pub enum EditorEvent { /* M3a variants */ FindRequested { seed: Option<String> }, LineEndingsChanged { mixed: bool } }
// LanguageService: provided method
fn folding_ranges(&self, _doc: DocId, _version: u64) -> Arc<[FoldRange]> { Arc::from([]) }
// commands rows (ids; bodies stub until the named task): editor.add_caret_above/below, editor.select_next_occurrence,
// editor.select_all_occurrences, editor.split_selection_into_lines, editor.undo_selection, editor.redo_selection (T31);
// editor.fold, editor.unfold, editor.fold_all, editor.unfold_all (T32); editor.find, editor.find_next, editor.find_prev,
// editor.find_close (T33); editor.toggle_comment, editor.duplicate_line, editor.delete_line, editor.move_line_up,
// editor.move_line_down, editor.normalize_line_endings, editor.history_prev, editor.history_next (T34)
pub enum ExecMarkState { /* M3a variants */ Escalated { since: Instant } }                // gutter cancel-escalation badge (T34)

// ===== hacivat-ui-grid (T35–T40) =====
pub enum ViewMap { Identity { loaded: u64 }, Permuted { order: Arc<[u64]>, tail_from: u64, loaded: u64 }, Filtered { rows: Arc<[u64]>, source_loaded: u64 } }
// view_to_model/model_to_view keep their M3a signatures; Identity unchanged
pub struct SortKey { pub col: ColId, pub desc: bool }
pub struct SortSpec { pub keys: SmallVec<[SortKey; 3]> }
pub enum GotoTarget { Row(u64), RowCol(u64, ColId) }
pub enum GotoError { Empty, NotANumber, OutOfRange { last: Option<u64> }, UnknownColumn, HiddenColumn(ColId) }
pub fn parse_goto(text: &str, columns: &[ColumnSpec], decimal: char, group: char) -> Result<GotoTarget, GotoError>;   // STUB: Err(Empty)
impl ColumnLayout { pub fn set_frozen(&mut self, n: usize); pub fn frozen(&self) -> usize; }      // STUB: 0
pub struct GridConfig { /* M3a fields */ pub transposed: bool }
pub enum GridCommand { /* M3a variants */ GrowSelection, ShrinkSelection, Find, GotoRow, OpenValue, ContextMenu, AutoFit, Freeze, Unfreeze }
pub enum GridEvent { /* M3a variants */ SortRequested(SortSpec), FindRequested, GotoRequested, ValueRequested(ModelCell),
    ContextMenuRequested { at: Point<Pixels>, cell: Option<ModelCell>, header: Option<ColId> }, FrozenChanged(usize) }
impl GridView { pub fn set_view_map(&mut self, map: ViewMap, cx: &mut Context<Self>); pub fn view_map(&self) -> &ViewMap; pub fn set_transposed(&mut self, on: bool, cx: &mut Context<Self>);
    pub fn set_highlights(&mut self, cells: Option<Rc<dyn Fn(ModelRow, ColId) -> bool>>, cx: &mut Context<Self>); }
// grid command rows: grid.grow_selection, grid.shrink_selection, grid.find, grid.goto_row, grid.open_value, grid.context_menu,
// grid.auto_fit, grid.freeze, grid.unfreeze, grid.sort_asc, grid.sort_desc, grid.clear_sort, grid.view_table, grid.view_transposed, grid.view_text, grid.toggle_record

// ===== hacivat-ui =====
impl ToolbarItem { pub fn narrow(self, element: ItemFn) -> Self; }                        // stored; Task 41 lays it out

// ===== hacivat-app (stubs answer the safe value) =====
// exec_ui::confirm::open(question, cx) -> oneshot::Receiver<ConfirmAnswer>              STUB: Cancel
// exec_ui::commit_prompt::open(question, cx) -> oneshot::Receiver<CommitAnswer>        STUB: RollBack
// exec_ui::params_prompt::open(prompts, cx) -> oneshot::Receiver<Option<Vec<(Arc<str>, BindValue)>>>   STUB: None (run cancelled)
// exec_ui::error_bar::show(console, ask: ErrorAsk, cx) -> oneshot::Receiver<RunChoice> STUB: Stop
// exec_ui::escalation::{on_slow_cancel, open_dialog}                                      STUB: no-op
// console::{tx_chip, tx_badge, idle_toast}::render(..) -> Option<AnyElement>            STUB: None (M3a segment stays)
// results::{sort, find, goto, views, value_viewer, context_menu, count, pin}             STUB: no UI
// history::open(cx)                                                                       STUB: no-op
pub struct AppConfirmPort { /* mpsc to the foreground, the AppUiPort pattern */ }       // implements hacivat_exec::ConfirmPort; stub answers Cancel/RollBack
```

**Keymap rows** (context in brackets; every chord passes `keymap_collisions.rs`; a chord that fails is reported NEEDS_CONTEXT with the failing layout, never silently changed; ids not listed are palette-only):

| Id | macOS | Windows / Linux | datagrip preset |
|---|---|---|---|
| `editor.add_caret_above` / `_below` [Editor] | `cmd-alt-up` / `cmd-alt-down` | `ctrl-alt-shift-up` / `ctrl-alt-shift-down` | — |
| `editor.select_next_occurrence` [Editor] | `ctrl-g` | `alt-j` | same |
| `editor.select_all_occurrences` [Editor] | `ctrl-cmd-g` | `ctrl-alt-shift-j` | same |
| `editor.undo_selection` [Editor] | `cmd-u` | `ctrl-u` | same (spec: "Ctrl+U undoes the selection") |
| `editor.find` / `find_next` / `find_prev` [Editor] | `cmd-f` / `cmd-g` / `cmd-shift-g` | `ctrl-f` / `f3` / `shift-f3` | same |
| `editor.toggle_comment` [Editor] | `cmd-/` | `ctrl-/` | same |
| `editor.duplicate_line` [Editor] | `cmd-d` | `ctrl-d` | same |
| `editor.delete_line` [Editor] | `cmd-shift-k` | `ctrl-y` | `cmd-backspace` / `ctrl-y` |
| `editor.move_line_up` / `_down` [Editor] | `alt-shift-up` / `alt-shift-down` | same | same |
| `editor.history_prev` / `_next` [Editor] | `ctrl-alt-up` / `ctrl-alt-down` (palette on Linux desktops that take them) | `ctrl-alt-up` / `ctrl-alt-down` | — |
| `grid.select_column` [Grid] | `cmd-shift-space` | `ctrl-shift-space` | same (closes P25) |
| `grid.grow_selection` / `shrink_selection` [Grid] | `alt-up` / `alt-down` | `alt-shift-up` / `alt-shift-down` | `ctrl-w` / `ctrl-shift-w` [Grid] |
| `grid.find` [Grid] | `cmd-f` | `ctrl-f` | same |
| `grid.goto_row` [Grid] | `cmd-l` | `ctrl-g` | same |
| `grid.open_value` [Grid] | `shift-enter` | `shift-enter` | same |
| `grid.context_menu` [Grid] | `shift-f10` | `shift-f10` | same |
| `results.next_tab` / `prev_tab` [Results] | `ctrl-pagedown` / `ctrl-pageup` | same | same |
| `history.open` [Workspace] | `cmd-shift-h` | `ctrl-shift-h` | same |
| `exec.cancel` second press [Console] | (same chord; Task 23 escalates) | | |

**Shadowing and deviations (P-M3b-21; PO Q10):** the datagrip preset's `ctrl-w` [Grid] shadows `workspace.close_tab` while a grid has focus (`default-windows.toml:21`); `editor.find` [Editor] and `grid.find` [Grid] shadow `explorer.focus_filter` (`default-macos.toml:41`) while they have focus. Ledger 336's Ctrl+Tab/⌘3 is not taken (`ctrl-tab` is Workspace tab switching): `ctrl-pagedown`/`ctrl-pageup` in the new `Results` context instead. The output log's ↑/↓ line selection stays element-level keys (GC "row-level keys are element-level"), not keymap rows.

- [ ] **Step 1: Write the failing tests.**
```rust
// hacivat-app/tests/m3b_routing.rs
#[gpui::test] fn every_m3b_command_id_is_registered_and_bound_as_listed(cx: &mut TestAppContext) { /* the table above, per platform keymap */ }
#[gpui::test] fn confirm_stub_answers_cancel(cx: &mut TestAppContext) {}
#[gpui::test] fn commit_prompt_stub_answers_roll_back(cx: &mut TestAppContext) {}
#[gpui::test] fn new_slots_render_nothing_so_m3a_shots_hold(cx: &mut TestAppContext) { /* tx chip/badge/find bar/history: no element */ }
#[gpui::test] fn results_context_is_active_on_the_results_root(cx: &mut TestAppContext) {}
#[test] fn new_icons_are_embedded() { for n in [IconName::Play, IconName::Pin, IconName::Sigma, IconName::Filter, IconName::ArrowUpDown, IconName::Snowflake] { assert!(asset_bytes(n.path()).is_some(), "{n:?}"); } }
// hacivat-ui-grid/tests/m3b_seams.rs
#[test] fn identity_view_map_is_unchanged() { let m = ViewMap::identity(10); assert_eq!(m.view_to_model(ViewRow(3)), Some(ModelRow(3))); }
#[test] fn permuted_view_map_round_trips() { let m = ViewMap::Permuted { order: Arc::from([2u64, 0, 1]), tail_from: 3, loaded: 3 }; assert_eq!(m.view_to_model(ViewRow(0)), Some(ModelRow(2))); assert_eq!(m.model_to_view(ModelRow(2)), Some(ViewRow(0))); }
// hacivat-ui-editor/tests/m3b_seams.rs
#[test] fn folding_ranges_default_is_empty() {}
#[test] fn chunked_height_map_stub_is_uniform() {}
```
- [ ] **Step 2:** Run the three test targets. Expected: FAIL.
- [ ] **Step 3: Implement** (the `ViewMap::Permuted`/`Filtered` mapping is working — `view_to_model`, `model_to_view` and `len` — because Tasks 35–37 and the M3a copy path depend on it; `GridView::set_transposed` is working too (stores the flag, notifies; the axis swap is Task 38's `element/transpose.rs` hook); everything else is a stub).
- [ ] **Step 4:** Run `cargo test -p hacivat-ui-editor --features scene-check`, `cargo test -p hacivat-ui-grid --features test-support,scene-check`, `cargo test -p hacivat-ui`, `cargo test -p hacivat-app --features scene-check,pseudo`, `cargo test -p hacivat-app --features shots,scene-check,pseudo --test shots` and `cargo xtask shot-diff` against **both** baselines. Expected: PASS; 0 shot changes (every new slot renders nothing).
- [ ] **Step 5: Commit** `feat(seams): M3b UI and app seams — multi-cursor, fold, find, ViewMap variants, grid commands, toolbar narrow, confirmation/transaction/history stubs, keymap rows`.

**Review:** every Consumes line of Tasks 21–45 that names a UI item resolves without editing a Task 1b file (P11/P12 lesson); every new keymap row passes the AltGr CI; no shot moved; no stub can send a statement.

### Task 2a: Mocks for the M3b "Safe" surfaces 1–13 (W0, parallel with Tasks 1a–1c)

**Files:** `design/m3b-safe/{index.html,NOTES.md}`.

- [ ] **Step 1:** Mock surfaces 1–13 of "New UI surfaces" in the harman world (EN/TR × dark/light × 1280 × 800 and 960 × 600), with the M2a mock bar, real Turkish strings at 960, the Overflow Rule applied, trust signals never clipped, the DESIGN.md rules (Danger only on destructive execute; Cancel focused in confirmations; countdowns once a second; ink glyphs for errors; one Filled Badge). Surface 2 has no non-production variant (P-M3b-30).
- [ ] **Step 2:** NOTES.md: per surface the anatomy, sizes, tokens, states, keyboard order, accessibility roles (alertdialog for confirmations, Cancel focused), and the copy table EN/TR (TR marked `Review: PO`); rulings for every open layout question; a header "binding for M3b; Task 2b appends the M3c surfaces".
- [ ] **Step 3:** Render every surface headless (the M3a T2 harness) and look at the PNGs; record the scenario count and findings.
- [ ] **Step 4: Commit** `design(m3b): mocks for confirmations, transactions, cancel escalation, error bar, params, results additions, history`.

**Review:** opus (design). The controller approves under the PO-away mandate and records the PO item.

### Task 2b: Mocks for the M3c "Complete" surfaces 14–24 (any time before W8; may run while the M3b gate waits)

**Files:** `design/m3b-safe/index.html` (new sections), `design/m3b-safe/NOTES.md` (appended `## M3c surfaces` part).

- [ ] **Step 1:** Mock surfaces 14–24 under the same rules, re-using Task 2a's anatomy (find bar vs grid find bar share one field anatomy; the value viewer follows the dialog anatomy).
- [ ] **Step 2:** NOTES.md part for M3c with its copy table.
- [ ] **Step 3:** Render headless; look at the PNGs.
- [ ] **Step 4: Commit** `design(m3c): mocks for multi-cursor, folding, find, gutter extras, grid sort/find/views, value viewer, context menu, narrow panes`.

**Review:** opus (design); controller approval before W8.

### Task 3: `sql-ident` and the SAFE-03 function, operator, cast and type data (W1)

**Files:** `crates/sql/sql-ident/src/**` (bodies); `crates/sql/sql-safety/src/functions/{mod,data}.rs`; `crates/sql/sql-safety/data/{pg,mysql,mariadb,sqlite}/{allow,deny,operators,types}.toml`; `crates/sql/sql-safety/data/{pg,mysql,mariadb,sqlite}/set_allow.toml` and `data/functions.digest`; `xtask/src/gen_fn_catalog.rs` (offline `--check` only); `crates/core/hacivat-exec/tests/gen_fn_catalog.rs` (the catalog read on Docker and the bundled SQLite, `#[ignore = "docker"]`, run only with `HACIVAT_GEN_FN_CATALOG=1` through the wrapper, writing the TOML candidates; P-M3b-13); tests `crates/sql/sql-ident/tests/fold.rs`, `crates/sql/sql-safety/tests/{functions,set_allow}.rs`; ADR 0005 `## m3b-t03 — identifiers and function data`.

**Interfaces:**
- Consumes: Task 1a's `sql-ident` signatures; `sql_dialect::DialectId`.
- Produces: `sql-ident` bodies; `pub(crate) fn lookup_fn(d: DialectId, name: &QualifiedName, version: Option<ServerVersion>) -> FnClass` in `functions/mod.rs` with `pub enum FnClass { Allowed, Denied(DenyGroup), Unknown }` and `pub enum DenyGroup { Sleep, FileIo, Network, ProcessExec, AdminSignal, Replication, LargeObject, DynamicSql, Advisory, Sequence, XidConsume, SetConfig }`; `pub(crate) fn operator_known(d, op: &str) -> bool`, `pub(crate) fn cast_known(d, ty: &str) -> bool`. Data rows carry `name`, `since`, `until`, `source` (`pg_proc@pg17` / `manual-index` / `pragma_function_list` / `manual`) and a `reviewed = true|false` column; only `reviewed = true` rows count as Allowed. Bodies of `denied_function_names(d)` and `set_allowed(d, name)` (the P-M3b-6 list, exact ASCII-case-insensitive variable names).

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn pg_folds_unquoted_ascii_only() { assert_eq!(&*fold(DialectId::Postgres, "PG_SLEEP", false), "pg_sleep"); assert_eq!(&*fold(DialectId::Postgres, "PG_SLEEP", true), "PG_SLEEP"); }
#[test] fn turkish_dotted_capital_never_folds_to_i() { assert_eq!(&*fold(DialectId::Postgres, "PG_READ_FİLE", false), "pg_read_fİle"); }   // only ASCII letters lowered
#[test] fn qualified_names_split_on_dots_outside_quotes() { let q = parse_qualified(DialectId::Postgres, r#"pg_catalog."pg_sleep""#).unwrap(); assert_eq!(q.parts.len(), 2); assert!(q.parts[1].quoted); }
#[test] fn sleep_is_denied_in_every_spelling() { for n in ["pg_sleep", "PG_SLEEP", "pg_catalog.pg_sleep", "public.pg_sleep", r#""pg_sleep""#] { assert!(matches!(lookup(pg(), n), FnClass::Denied(DenyGroup::Sleep)), "{n}"); } }
#[test] fn turkish_i_lookalike_is_unknown_not_allowed() { assert!(matches!(lookup(pg(), "PG_READ_FİLE"), FnClass::Unknown)); }
#[test] fn volatile_but_pure_builtins_are_allowed() {   // hand rows, source = "manual", listed in ADR 0005 (P-M3b-18: pg_proc marks them VOLATILE, the generator never proposes them)
    for n in ["random", "clock_timestamp", "gen_random_uuid", "now"] { assert!(matches!(lookup(pg(), n), FnClass::Allowed), "{n}"); } }
#[test] fn query_to_xml_is_denied_dynamic_sql() { assert!(matches!(lookup(pg(), "query_to_xml"), FnClass::Denied(DenyGroup::DynamicSql))); }
#[test] fn guard_non_transactional_functions_are_a_subset_of_the_deny_list() { for f in hacivat_guard_list_fixture() { assert!(matches!(lookup(pg(), f), FnClass::Denied(_)), "{f}"); } }   // fixture copied by name; equality test lives in Task 18
#[test] fn sqlite_builtins_come_from_pragma_function_list() { assert!(matches!(lookup(sqlite(), "load_extension"), FnClass::Denied(DenyGroup::FileIo))); assert!(matches!(lookup(sqlite(), "abs"), FnClass::Allowed)); }
#[test] fn every_row_names_its_source_and_review_state() {}
#[test] fn data_matches_the_checked_in_digest() { assert_eq!(digest_of("data"), include_str!("../data/functions.digest").trim()); }
#[test] fn harmless_set_list() { assert!(set_allowed(DialectId::Postgres, "search_path")); assert!(set_allowed(DialectId::Postgres, "STATEMENT_TIMEOUT")); for v in ["role", "session authorization", "transaction_read_only", "default_transaction_read_only", "idle_in_transaction_session_timeout"] { assert!(!set_allowed(DialectId::Postgres, v), "{v}"); } assert!(!set_allowed(DialectId::MySql, "sql_mode")); }
```
- [ ] **Step 2:** Run `cargo test -p sql-ident -p sql-safety --test fold --test functions`. Expected: FAIL.
- [ ] **Step 3: Implement.** The `hacivat-exec` test `gen_fn_catalog` (run once by the implementer with `HACIVAT_GEN_FN_CATALOG=1 <main>/target/docker/run-docker-tests.sh --set full -p hacivat-exec --test gen_fn_catalog`) reads `pg_proc`/`pg_operator`/`pg_type` (OID < 16384) on pg14–18 through `hacivat_testkit::targets` (read-only queries; the controller-owned stack), `pragma_function_list` from the bundled SQLite (safe rusqlite API), and the MySQL/MariaDB names from a hand-curated `data/mysql/manual-index.txt` (names only; never `mysql.help_*`). Candidates are immutable/stable, `internal`/`sql` language, no `text` argument used as SQL or a filename, minus the `alg/13` :271-290 denylist; every candidate lands `reviewed = false` and the implementer reviews and flips each row by hand, listing the count in the report. `cargo run -p xtask -- gen-fn-catalog --check` is offline (recomputes `data/functions.digest`) and runs in CI.
- [ ] **Step 4:** Run the tests also under `LC_ALL=tr_TR.UTF-8`; `cargo run -p xtask -- gen-fn-catalog --check`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(sql-safety): identifier folding and the SAFE-03 function, operator, cast and type data for PG, MySQL, MariaDB and SQLite`.

**Review:** security-sensitive. Probes: a `STABLE` builtin that runs SQL left Allowed; a name match that folds non-ASCII; a `reviewed = false` row counted as Allowed; a GPL source used for MySQL names.

### Task 4: `sql-safety` core: effects, reports, policy, destructive reasons, `GateClass` (SAFE-01, -04, the -07 model) (W1)

**Files:** `crates/sql/sql-safety/src/{effects,report,reason,policy,destructive,gate_class,limits}.rs`, `data/profiles.toml`; tests `crates/sql/sql-safety/tests/{policy,gate_class,reasons}.rs`; FTL block `## m3b-t04-safety` in `locales/{en,tr}/hacivat-core.ftl` (`safety-class-*`, `safety-reason-*`, `safety-destructive-*`; TR `# Review: PO`); ADR 0005 `## m3b-t04 — the policy table`.

**Interfaces:**
- Consumes: Task 1a's types.
- Produces: bodies of `decide`, `gate_class`, `opens_write`, `ReasonCode::fluent_id`; `pub(crate) fn class_of(effects: EffectSet) -> CoarseClass` (the most critical class wins: `Unknown > Admin > Ddl > Write > Session > Read`); `pub(crate) fn merge(items: &[StmtReport]) -> (CoarseClass, EffectSet, RiskFlags)`. `decide` follows `alg/13` :317-329 (profiles) with: `Unknown` → `Deny` in every built-in profile except `Admin`; `ExecutableComment` denies in `ReadOnly`, `ReadOnlyPlusExplain` and every `AiAgent` decision (D-058); `WriteTemp` denies on read-only (D-059); `ReadOnlyPlusExplain` allows `EXPLAIN ANALYZE` only when the inner class is `Read`. `gate_class` maps (`alg/20` :266-278): `Read` → Read; `LOCK_ROW` → Lock; `WRITE_DATA` with any `DestructiveReason` of kind NoWhere/AlwaysTrue/JoinWithoutOn/UpdateLimitNoOrder/MergeDelete → DmlUnsafe, else Dml; DDL with Drop/Truncate/DropColumn/DropConstraint/AlterType → DdlDestructive, else DdlOther; `ROUTINE_CALL` → Routine; `SESSION_STATE` only → Session; `TXN_CONTROL` → Tx; anything with `UNKNOWN` → Unknown. `opens_write(r) = r.class != Read && r.class != Session`. **Totality (P-M3b-4):** the order is `Unknown > Admin > DdlDestructive > DdlOther > DmlUnsafe > Dml > Routine > Lock > Tx > Session > Read` (the most critical bit wins); `PRIVILEGE`/`ADMIN`/`EXTERNAL_IO`/`PROCESS_EXEC` → Admin; `LOCK_TABLE`/`ADVISORY_LOCK` → Lock; `SEQUENCE`/`XID_CONSUME`/`WRITE_TEMP` → Dml; `DDL_TEMP` → DdlOther; `PREPARE`/`NOTIFY` → Session; a set with no mapped bit → Unknown, never Read; `class_of(LOCK_ROW) = Write`. `decide` denies `SESSION_STATE` on read-only profiles unless every `SET` target passes `set_allowed` (P-M3b-6, `SetNotAllowlisted`).

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn safe01_cases() { /* alg/13 :141 cases, data-driven from tests/data/safe01.toml */ }
#[test] fn safe04_profile_table() { /* alg/13 :317-329 every (profile, class) cell */ }
#[test] fn unknown_denies_in_every_profile_but_admin() { for p in [Profile::ReadOnly, Profile::ReadOnlyPlusExplain, Profile::ReadWriteDml, Profile::Ddl] { assert!(matches!(decide(&unknown(), p, Actor::Human).verdict, Verdict::Deny(_)), "{p:?}"); } }
#[test] fn executable_comment_denies_on_read_only_and_for_ai() {}
#[test] fn temp_writes_deny_on_read_only() {}
#[test] fn explain_analyze_of_a_read_is_allowed_for_humans_only() { assert!(matches!(decide(&explain_analyze_read(), Profile::ReadOnlyPlusExplain, Actor::Human).verdict, Verdict::Allow)); assert!(matches!(decide(&explain_analyze_read(), Profile::ReadOnly, Actor::AiAgent).verdict, Verdict::Deny(_))); }
#[test] fn gate_class_is_total() { for e in all_effect_combinations(12) { for d in destructive_variants() { let _ = gate_class(&report(e, &d)); } } }
#[test] fn read_only_effects_only_are_read() { for e in all_effect_combinations(12) { if !(e - EffectSet::READ).is_empty() { assert_ne!(gate_class(&report(e, &[])), GateClass::Read, "{e:?}"); } } }
#[test] fn copy_to_program_is_admin() { assert_eq!(gate_class(&report(EffectSet::READ | EffectSet::PROCESS_EXEC, &[])), GateClass::Admin); }
#[test] fn lock_row_is_write_class() { assert_eq!(class_of(EffectSet::READ | EffectSet::LOCK_ROW), CoarseClass::Write); }
#[test] fn read_only_denies_set_unless_allowlisted() {}
#[test] fn delete_without_where_is_dml_unsafe() { assert_eq!(gate_class(&report(EffectSet::WRITE_DATA, &[DestructiveKind::NoWhere])), GateClass::DmlUnsafe); }
#[test] fn the_most_critical_class_wins() { assert_eq!(class_of(EffectSet::READ | EffectSet::WRITE_DATA), CoarseClass::Write); assert_eq!(class_of(EffectSet::READ | EffectSet::UNKNOWN), CoarseClass::Unknown); }
#[test] fn every_reason_class_and_destructive_id_exists_in_en_and_tr() { /* reads locales via a test-local FTL parser; hacivat-l10n may not be a dependency (SqlNotRuntimeFree) */ }
```
- [ ] **Step 2:** Run `cargo test -p sql-safety --test policy --test gate_class --test reasons`. Expected: FAIL.
- [ ] **Step 3: Implement** the bodies and the FTL block (one message per `ReasonCode`, `CoarseClass`, `DestructiveKind`; table names reach the message as FSI/PDI-isolated arguments).
- [ ] **Step 4:** Run the crate's tests and `cargo xtask l10n check`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(sql-safety): effects, reports, policy table, destructive reasons and GateClass`.

**Review:** security-sensitive. Probes: any path from `Unknown` to `Allow` outside `Admin`; `gate_class` returning `Read` for a set with `UNKNOWN`; a missing TR id.

### Task 5: Incremental relex and resplit, public checkpoints, `StreamSplitter`; lexer follow-ups (SQL-LEX-03, SQL-SPLIT-05; R-M3-9, R-M3-65) (W1)

**Files:** `crates/sql/sql-lexer/src/{incremental,relex}.rs`, named lines in `sql-lexer/src/{comments,strings,cursor,token}.rs` (`Lexed::checkpoint_before`; the two follow-ups); `crates/sql/sql-split/src/{incremental,stream}.rs`, `document.rs` (`apply` internals; same public API); `crates/app/hacivat-sql-editor/src/worker.rs` (named lines: the worker calls `SqlDocument::apply` with the change set instead of rebuilding); tests `crates/sql/sql-lexer/tests/incremental.rs`, `crates/sql/sql-split/tests/{incremental,stream}.rs`; ADR 0005 `## m3b-t05 — incremental analysis`.

**Interfaces:**
- Consumes: M3a's `relex` (`relex.rs:22`, currently a full relex), `LexState` (exact at line starts), `Analysis.checkpoints` (`splitter.rs:481`, every `CHECKPOINT_BYTES = 2048`), `SqlDocument::apply` (`document.rs:153`).
- Produces:
  - `relex(lexed, new_text, edit, profile, modes) -> RelexDelta` really incremental: restart at `checkpoint_before(edit.delete.start())`, relex until the new token stream re-synchronises (same `LexState` at a line start at or after the edit end and the same token kind at `j-1`), splice; `RelexDelta { changed, full: false }`. A delimiter-changing or mode-changing edit, or an edit inside a `ClientCommand`, sets `full: true` and relexes the tail.
  - `SqlDocument::apply` reuses the relexed tokens and resplits from the first statement whose range the edit touched (or from the previous statement when the edit touches a terminator), keeping later `StmtKey`s for moved statements; `DocDelta { changed, moved, removed, dirty }` as declared.
  - `StreamSplitter::{new, feed, finish}` with a carried `LexState` and prepass state; one statement capped at `STREAM_STATEMENT_MAX` (64 MiB) → `TooLong`.
  - Follow-ups: (a) inside comment bodies, `x'…'`/`b'…'` are not read as escaping strings (ledger 200); (b) a client command's argument ends at the end of its line (ledger 274), which removes the M3a odd-quote cost (T13's PO note).
- [ ] **Step 1: Write the failing tests.**
```rust
// sql-lexer/tests/incremental.rs
proptest! { #[test] fn incremental_relex_equals_full_relex(doc in sql_doc(), edit in edit_in(&doc)) { prop_assert_eq!(tokens(relexed(&doc, &edit)), tokens(full_lex(&apply(&doc, &edit)))); } }   // 10_000 cases in PR (proptest cases from env), 100_000 nightly
#[test] fn typing_inside_one_statement_relexes_one_line() { let d = lines(50_000); let delta = relex_at(&d, 1_000_000, "x"); assert!(!delta.full); assert!(delta.changed.len() < 200); }
#[test] fn opening_quote_at_offset_zero_relexes_the_tail() {}
#[test] fn delimiter_change_forces_a_tail_relex() {}
#[test] fn checkpoint_before_is_exact_at_line_starts() { /* lex from the checkpoint equals the tokens of a full lex after it */ }
#[test] fn hex_literal_inside_a_comment_body_is_not_a_string() { assert_eq!(kinds(mysql(), "/* x'a */ SELECT 1"), [BlockComment, Whitespace, Word, Whitespace, Number]); }
#[test] fn client_command_argument_ends_at_end_of_line() { let l = lex_pg_psql("\\set x 'a\nSELECT 1;"); assert_eq!(kind_at(&l, 10), TokenKind::Word); }   // byte 10 = 'S'
#[test] #[ignore = "perf"] fn relex_typing_edit_p95_under_2ms() { if std::env::var_os("HACIVAT_PERF").is_none() { return; } /* gen-sql 50k-line fixture, 1,000 single-char edits at random lines, release: p95 ≤ 2 ms for relex + resplit (R-M3-9) */ }
#[test] #[ignore = "perf"] fn worst_case_relex_under_10ms() {}   // opening quote at offset 0 of 1 MiB
// sql-split/tests/incremental.rs
proptest! { #[test] fn incremental_resplit_equals_full_split(doc in sql_doc(), edit in edit_in(&doc)) { /* editor and execution splits, items + ambiguities + client segments */ } }
// fixtures for the next two tests use 1,000 distinct statements (StmtKey is content-addressed, key.rs:6-10)
#[test] fn typing_in_one_of_1000_statements_changes_one_key() { let (_, d) = apply_typing(1000, 500); assert_eq!(d.changed.len(), 1); assert_eq!(d.moved.len(), 499); }
#[test] fn deleting_a_semicolon_removes_two_and_changes_one() { let d = delete_semicolon(1000, 500); assert_eq!(d.removed.len(), 2); assert_eq!(d.changed.len(), 1); }
#[test] fn crlf_split_by_an_edit_stays_one_break() {}
#[test] fn multi_range_edit_batches_apply_in_order() {}
// sql-split/tests/stream.rs
proptest! { #[test] fn stream_splitter_equals_full_split_for_any_chunking(doc in sql_doc(), cuts in chunking()) {} }
#[test] fn one_byte_chunks_equal_64k_chunks() {}
#[test] fn statement_over_64_mib_is_too_long() {}
```
- [ ] **Step 2:** Run `cargo test -p sql-lexer --test incremental` and `cargo test -p sql-split --test incremental --test stream`. Expected: FAIL.
- [ ] **Step 3: Implement.** The worker change in `hacivat-sql-editor/src/worker.rs` keeps R-M3-22: analysis stays off the UI thread; only the rebuild becomes `apply`. Every M3a property (`split_props`, `exec_range_props`, `preview_truth`) must stay green unchanged.
- [ ] **Step 4:** Run `cargo test -p sql-lexer -p sql-split -p hacivat-sql-editor` (also under `tr_TR`), and `HACIVAT_PERF=1 cargo test --release -p sql-lexer -p sql-split -- --ignored`. Expected: PASS; the ≤ 2 ms figure recorded in the report.
- [ ] **Step 5: Commit** `feat(sql): incremental relex and resplit from public checkpoints, StreamSplitter, comment-body and client-argument lexing fixes`.

**Review:** security-sensitive (a wrong incremental cut is a wrong Run range). Probes: an edit that changes `standard_conforming_strings`-dependent lexing; an edit inside `/*! … */` with `AmbiguousCommentEnd`; an edit inside COPY data; an edit that turns `BEGIN` into a name; the equivalence property's generator reaching all of these.

### Task 6: Parameters, template detection and `HvNorm1` (SQL-PARAM-01, -02, -03) (W1)

**Files:** `crates/sql/sql-split/src/{params,templates,fingerprint}.rs`; `crates/sql/sql-testkit/corpus/datagrip/param.toml`; tests `crates/sql/sql-split/tests/{params,fingerprint}.rs`; ADR 0005 `## m3b-t06 — parameters`.

**Interfaces:**
- Consumes: Task 1a's params types; `sql_lexer::params::{colon, mysql_var, tcl}` shapes (`params.rs`); `TokenKind::{PositionalParam, QuestionParam, NamedParam}`.
- Produces: `scan_params` and `hv_norm1` bodies. Rules: PG `$n` passes through (positional, bindable); PG `?` is not a parameter unless `pg_question_marks` (D-08); `:name` (and `:'name'`/`:"name"` psql forms) is bindable outside strings, comments, `::` casts, array slices `a[1:2]`, `:=` and jsonb operators (`?|`, `?&`, `@>`); names compare exactly (`:Id` ≠ `:id`); in DDL statements (`StmtHead.first_keyword` is CREATE/ALTER/DROP) placeholders are not bindable (`ParamInDdl`) unless `in_ddl`; mixing `$n` and `:name` → `MixedStyles`; more than 65,535 slots → `TooManyBinds`. Rewrite: PG `:name` → `$k` (first occurrence order; a repeated name reuses `$k`), MySQL/SQLite `:name` → `?` with one slot per occurrence; `edits` record every replacement. Templates `${var}`, `%var%`, `<var>` are detected only (D-053). `hv_norm1` replaces literals and `IN (…)` lists with placeholders, collapses whitespace and comments, ASCII-uppercases keywords (by the lexer's keyword table), keeps identifiers and quoted text byte-exact, hashes with XXH3-64 (P-M3b-18).
- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn hv_exec_03_cases_1_to_8() { /* alg/20 :237-244 */ }
#[test] fn param_goldens() { /* corpus/datagrip/param.toml rows 01–06, 12–15, 18–20 for PG and MySQL, D-053 divergences recorded */ }
#[test] fn pg_colon_name_rewrites_to_dollar_with_reuse() { let s = scan_params("SELECT :a, :b, :a", &pg(), &head_select(), &ParamSettings::default()); let r = s.rewritten.unwrap(); assert_eq!(r.text, "SELECT $1, $2, $1"); assert_eq!(r.plan.names.len(), 2); }
#[test] fn mysql_repeats_a_name_per_slot() { let r = scan_params("SELECT :a, :a", &mysql(), &head_select(), &ParamSettings::default()).rewritten.unwrap(); assert_eq!(r.text, "SELECT ?, ?"); assert_eq!(&r.plan.slots[0][..], &[0, 1]); }
#[test] fn false_positives_are_not_params() { for s in ["SELECT a::int", "SELECT a[1:2]", "SELECT '{}'::jsonb ?| array['a']", "SET @x := 1", "SELECT ':a'", "-- :a\nSELECT 1"] { assert!(scan_params(s, &pg_or_mysql(s), &head_select(), &ParamSettings::default()).placeholders.iter().all(|p| !p.bindable), "{s}"); } }
#[test] fn names_compare_exactly() { assert_eq!(scan_params("SELECT :Id, :id", &pg(), &head_select(), &ParamSettings::default()).rewritten.unwrap().plan.names.len(), 2); }
#[test] fn pg_question_mark_is_off_by_default() {}
#[test] fn ddl_placeholders_are_not_bindable() {}
#[test] fn template_variable_is_detected_not_substituted() { let s = scan_params("SELECT * FROM ${t}", &pg(), &head_select(), &ParamSettings::default()); assert_eq!(s.templates.len(), 1); assert!(s.diags.iter().any(|(_, d)| matches!(d, ParamDiag::TemplateVariable))); }
proptest! { #[test] fn rewrite_preserves_every_other_byte(sql in sql_with_params()) { /* every token outside a placeholder is byte-identical in the rewritten text */ } }
proptest! { #[test] fn bind_plan_covers_each_placeholder_once(sql in sql_with_params()) {} }
#[test] fn hv_norm1_groups_in_lists() { assert_eq!(hv_norm1("SELECT * FROM t WHERE a IN (1,2,3)", &pg()).hash, hv_norm1("select * from t where a in (9)", &pg()).hash); }
#[test] fn hv_norm1_keeps_turkish_identifiers_exact() { assert_ne!(hv_norm1("SELECT müşteri FROM t", &pg()).hash, hv_norm1("SELECT musteri FROM t", &pg()).hash); }
```
- [ ] **Step 2:** Run `cargo test -p sql-split --test params --test fingerprint`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Run the crate's tests (also `tr_TR`). Expected: PASS.
- [ ] **Step 5: Commit** `feat(sql-split): bind parameter detection and rewrite, template detection, HvNorm1 fingerprints`.

**Review:** security-sensitive. Probes: a placeholder inside a string or dollar body rewritten; a rewrite that moves a byte outside a placeholder; a `:name` inside an executable comment; `$1` and `:a` in one unit.

### Task 7: The full type matrix in the decoders (R-M3-67) (W1)

**Files:** `crates/core/hacivat-driver-pg/src/decode/**`, `crates/core/hacivat-driver-mysql/src/decode.rs`, `crates/core/hacivat-driver-sqlite/src/decode.rs`; named items in `crates/core/hacivat-values/src/{logical,native}.rs` (MySQL `NativeType::MySql` gains `time_zone: Option<Arc<str>>`, ledger 194) and `valfmt/display.rs` (per-engine float spelling, type labels from `NativeType`); tests `crates/core/hacivat-driver-pg/tests/decode_m3b.rs`, `crates/core/hacivat-driver-mysql/tests/decode_m3b.rs`, `crates/core/hacivat-values/tests/valfmt_m3b.rs`, Docker goldens `crates/core/hacivat-driver-pg/tests/docker_types_m3b.rs` and `crates/core/hacivat-driver-mysql/tests/docker_types_m3b.rs`; ADR 0002 `## m3b-t07 — type matrix`.

**Interfaces:**
- Consumes: M3a's `Col::for_type` (`decode/mod.rs:118`, `_ => Col::Opaque` at :132), MySQL `Col` (`decode.rs:120-170`), `LogicalType` (already has `Array`, `Composite`, `Range`, `Network`, `BitString`, `Geometry`, `Money`, `Xml`).
- Produces: PG binary decoders for arrays of every M3a scalar (n-dimensional, lower bounds, NULL elements) → `Value::Text` holding the PG literal under `LogicalType::Array(elem)`; `inet`/`cidr`/`macaddr`/`macaddr8` → canonical text under `Network`; `bit`/`varbit` → `BitString` text; text-format but typed `LogicalType` for composites/records (`Composite`), ranges/multiranges (`Range`), `money` (`Money`, right-aligned, text verbatim — `lc_monetary` decides its form), `xml` (`Xml`), geometry/`tsvector`/extension types (`Opaque` stays only for unknown OIDs); MySQL `BIT(n)` → `BitString` (text `0101`, as PG `bit`; P-M3b-26), geometry → `Geometry` (WKB as hex), `VECTOR` stays `Opaque`; MySQL `TIMESTAMP` columns carry the session `time_zone` read at connect; `valfmt` prints MySQL/SQLite floats in the shortest round-trip form (not PG's spelling) and MySQL type labels from `NativeType::MySql` (`decimal(14,2)`, `bigint unsigned`).
- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn pg_array_lower_bounds_round_trip() { assert_eq!(text_of(decode_pg_array(&int4_array_with_lower_bound(0, &[1, 2, 3]))), "[0:2]={1,2,3}"); }
#[test] fn pg_two_dimensional_array_with_nulls() { assert_eq!(text_of(decode_pg_array(&int4_2d(&[[Some(1), None], [Some(3), Some(4)]]))), "{{1,NULL},{3,4}}"); }
#[test] fn pg_text_array_quotes_like_pg() { assert_eq!(text_of(decode_pg_array(&text_array(&["a b", "\"", ""]))), r#"{"a b","\"",""}"#); }
#[test] fn inet_and_cidr_canonical_text() { assert_eq!(text_of(decode_pg(&INET, &inet_v4([10,0,0,1], 32))), "10.0.0.1"); assert_eq!(text_of(decode_pg(&CIDR, &cidr_v4([10,0,0,0], 8))), "10.0.0.0/8"); }
#[test] fn varbit_keeps_its_length() { assert_eq!(text_of(decode_pg(&VARBIT, &varbit(5, 0b10110_000))), "10110"); }
#[test] fn money_is_typed_text() { assert!(matches!(logical_of(&MONEY), LogicalType::Money)); }
#[test] fn hostile_array_header_is_an_error_cell() { /* ndim 7, negative length, element length past the buffer → Value::Error, never a panic */ }
#[test] fn mysql_bit_is_a_bit_string() { assert_eq!(text_of(decode_mysql_bit(&[0b0000_0101], 4)), "0101"); }
#[test] fn mysql_f32_uses_its_own_shortest_round_trip() { assert_eq!(format_cell(ValueRef::F32(0.1), &mysql_float_col(), &settings_en(), Purpose::Cell).text, "0.1"); }   // never widened to f64 (P-M3b-26)
#[test] fn mysql_type_label_from_native() { assert_eq!(type_label(&mysql_native_decimal(14, 2)), "decimal(14,2)"); }
// Docker goldens (#[ignore = "docker"]): one round trip per engine and version, every type in tests/data/types_<engine>.sql, expected text checked in
#[tokio::test] async fn pg_type_goldens_pg14_to_18() {}
#[tokio::test] async fn mysql_type_goldens_80_84_and_mariadb() {}
```
- [ ] **Step 2:** Run the crates' tests. Expected: FAIL.
- [ ] **Step 3: Implement.** Hostile bytes give `Value::Error`, never a panic (M3a rule); constructors via `try_*`; chunks via `ChunkBuilder::with_schema` (ledger 129).
- [ ] **Step 4:** Run the crates' tests and `<main>/target/docker/run-docker-tests.sh --set pr -p hacivat-driver-pg --test docker_types_m3b` and `-p hacivat-driver-mysql --test docker_types_m3b`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(drivers): arrays, network, bit strings and typed text columns for composites, ranges, money and xml; MySQL bit/geometry and session time zone; per-engine float spelling`.

**Review:** hostile wire data (array headers, bit lengths) never panics; no value silently altered; goldens per version.

### Task 8: Result store: eviction, `cap_raised` on the source, per-source coalescing, value-size setting (HV-RES-07 eviction; R-M3-68, R-M3-82) (W2)

**Files:** `crates/core/hacivat-results/src/{store,budget,cursor,window,writer,evict}.rs` (`writer.rs` `admit()` at `:125-139` is where `Pause(GlobalBudget)` is decided and eviction hooks in, P-M3b-29); tests `crates/core/hacivat-results/tests/{evict,source_m3b}.rs`; ADR 0006 `## m3b-t08 — eviction`.

**Interfaces:**
- Consumes: Task 1c's `CloseReason::Evicted`, `ResultSource::{cap_raised, set_visible}`, `MemoryBudget::evict_for`, `EvictionRegistry`.
- Produces: `MemoryBudget::evict_for(need) -> u64` (bytes **released from the budget** as the evicted `StoredChunk`s drop — chunks stay alive while a snapshot holds them, P-M3b-29; `admit()` calls it before answering `Pause(GlobalBudget)`) evicts registered stores that are not visible, oldest-visible-first, keeping chunk 0 of each, publishing `Completeness::Closed { reason: Evicted }`; `StoreSource::set_visible(bool)` (the app calls it for the active tab of every console); `StoreSource::cap_raised()` forwards `ResultStore::cap_raised` (ledger 253 M7); queued `FetchMore`s from one source coalesce into one request of the sum (ledger 109); `FetchPolicy::cell_cap` from `ResultsSettings.max_value_bytes` (wired by the app in Task 25).
- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn eviction_skips_visible_results() { let b = MemoryBudget::new(4 << 20, 4 << 20); let (a, b2) = (store_filled(&b, 2 << 20, true), store_filled(&b, 2 << 20, false)); assert!(b.evict_for(1 << 20) >= 1 << 20); assert!(!closed(&a)); assert_eq!(closed_reason(&b2), Some(CloseReason::Evicted)); }
#[test] fn eviction_keeps_chunk_zero() {}
#[test] fn oldest_hidden_result_goes_first() {}
#[test] fn source_cap_raised_follows_the_store() {}
#[test] fn fetch_mores_from_one_source_coalesce() { /* two ensure_rows before the control answers → one fetch_more of the sum */ }
#[test] fn evicted_snapshot_still_reads_chunk_zero() {}
#[test] fn eviction_with_a_held_snapshot_releases_on_drop() { /* a held Arc<StoreSnapshot> keeps the chunks; budget.used() drops when it is dropped */ }
#[test] fn admit_evicts_before_pausing() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-results --features test-support --test evict --test source_m3b`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Run `cargo test -p hacivat-results --features test-support`. Expected: PASS (M3a tests unchanged).
- [ ] **Step 5: Commit** `feat(results): evict hidden results under global pressure, cap_raised on sources, coalesced fetches per source`.

**Review:** an evicted result never reports `Complete`; a visible result is never evicted; accounting returns every byte.

### Task 9: PG classifier backend: generated visitor, statement table, SAFE-03/-07 for PG; libpg_query hygiene (SQL-SAFE-02 PG; R-M3-2) (W2)

**Files:** `crates/sql/sql-safety/src/{backend/{mod,pg},visit/{mod,pg_generated}}.rs`, `data/pg/statements.toml`; `crates/sql/sql-oracle-pg/src/tree.rs` (bodies); `xtask/src/gen_pg_visitor.rs` (replaces the stub); `.github/workflows/ci.yml` block `# ## m3b-t09-visitor` (staleness check); tests `crates/sql/sql-safety/tests/{pg_backend,pg_coverage}.rs`, `crates/sql/sql-oracle-pg/tests/tree.rs`; **named exception** `third_party/pg_query/PATCHES.md` §2 (+ the fenced build change) only if Step 3's stderr ruling needs it; ADR 0005 `## m3b-t09 — PG backend`.

**Interfaces:**
- Consumes: Task 3's `lookup_fn`, Task 4's report model, `sql_oracle_pg::{on_parser_stack, MAX_ORACLE_NESTING}`; `third_party/pg_query/protobuf/pg_query.proto` (read by the generator; no `protoc`).
- Produces:
  - `sql_oracle_pg::{parse_tree, scan_tokens, fingerprint}` bodies on the existing parser thread (8 MiB cap, NUL refused, depth ≤ 256; every libpg_query call passes the guard; whitespace-only texts never reach `scan`); a prost decode-depth failure is `Err` (ledger 151).
  - `struct PgBackend; impl Backend for PgBackend` with the **combine rule**: libpg_query is the authority; parse error → `Unknown` + `ParseError`; server newer than PG 18 → `SyntaxNewerThanAgent`; libpg_query's statement count must equal our item count else `MultiStatement`/`ParserDisagreement`; **token-boundary agreement** between our lexer and `scan_tokens` on significant tokens else `ParserDisagreement`.
  - `visit/pg_generated.rs`: an exhaustive visitor over every message and oneof of `pg_query.proto`, generated by `cargo run -p xtask -- gen-pg-visitor` (a small proto3 parser in xtask), checked in; CI fails when stale.
  - `data/pg/statements.toml`: one row per `pb::node::Node` statement variant → `StmtKind`, base effects.
- **stderr ruling (ledger 151, 164):** measure whether concurrent `eprintln!` from another thread during 10,000 parses loses output. If it does, apply `PATCHES.md` §2: build libpg_query without its per-call stderr redirect (a fenced build define), proven by the same test; if the redirect cannot be removed, record the limit in ADR 0005 and route `tracing` to the log file only (never stderr) in product builds. The unused `deparse_comments_for_query`/`is_utility_stmt` stay unreachable (a grep test).
- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn safe02_cases_1_to_21_and_33_34() { /* alg/13 :213-248 */ }
#[test] fn cve_t12_shapes_are_not_read() { for s in ["SELECT * FROM (SELECT f_write()) s", "WITH w AS (SELECT f_write()) SELECT * FROM w", "SELECT * FROM t, LATERAL (SELECT f_write()) l", "SELECT * FROM t ORDER BY (SELECT f_write())"] { assert_ne!(classify_pg(s).request_class, CoarseClass::Read, "{s}"); } }
#[test] fn data_modifying_cte_is_write() { assert_eq!(classify_pg("WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d").request_class, CoarseClass::Write); }
#[test] fn select_into_is_ddl() {}
#[test] fn explain_analyze_option_spellings() { for o in ["ANALYZE", "ANALYZE true", "ANALYZE on", "ANALYZE 1", "ANALYZE yes"] { assert!(classify_pg(&format!("EXPLAIN ({o}) DELETE FROM t")).flags.contains(RiskFlags::EXPLAIN_ANALYZE), "{o}"); } }
#[test] fn set_config_with_a_non_literal_name_is_unknown() {}
#[test] fn copy_variants() { /* COPY t TO STDOUT read; COPY t FROM STDIN write; COPY ... TO PROGRAM process exec */ }
#[test] fn unsafe_01_to_10() { /* alg/13 :541 destructive reasons */ }
#[test] fn token_disagreement_is_parser_disagreement() {}
#[test] fn prost_depth_limit_denies_not_panics() { let s = format!("SELECT {}1{}", "ARRAY[".repeat(47), "]".repeat(47)); assert_eq!(classify_pg(&s).request_class, CoarseClass::Unknown); }
#[test] fn pg_visitor_reaches_every_node_kind_in_the_corpus() { /* serde_json walk of each parse tree counts node kinds independently; every kind seen is visited and classified, none falls through */ }
#[test] fn concurrent_stderr_is_not_lost_during_parses() { /* 10,000 parses on the parser thread while another thread writes 10,000 lines to stderr, captured by a child process; all lines present */ }
#[test] fn generated_visitor_is_fresh() {}
```
- [ ] **Step 2:** Run `cargo test -p sql-safety --test pg_backend --test pg_coverage` and `cargo test -p sql-oracle-pg --test tree`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Run `cargo test -p sql-safety -p sql-oracle-pg`; `cargo run -p xtask -- gen-pg-visitor --check`; check `third_party/pg_query` is unchanged by the build (M3a CI step). Expected: PASS.
- [ ] **Step 5: Commit** `feat(sql-safety): PostgreSQL backend over a generated libpg_query visitor with statement-count and token agreement`.

**Review:** security-sensitive. Probes: a node kind the visitor skips (the coverage test's generic walk must be independent); `pg_query`'s own `nodes()/tables()/functions()` used anywhere (forbidden, R9); a nested function in a `FROM` inside a `LATERAL` inside a CTE; a parse that panics the parser thread.

### Task 10: Interim classifier backend for MySQL, MariaDB and SQLite (sqlparser + lexer pre-scan; R-M3-54) (W2)

**Files:** `crates/sql/sql-safety/src/backend/{interim,prescan}.rs`, `data/{mysql,sqlite}/statements.toml`, `data/mysql/known_gaps.toml`; `crates/sql/sql-testkit/corpus/read/{pg,mysql,mariadb,sqlite}.sql` (≥ 300 Hacivat-authored read queries each over the `datagrip/13` §3.1 seed schema, MySQL/MariaDB `SHOW`/`DESCRIBE`/`EXPLAIN` and SQLite read `PRAGMA`s included); the MariaDB run A rows go to Task 18's `mariadb.toml` (handed over in the report); tests `crates/sql/sql-safety/tests/{interim,read_corpus}.rs`; ADR 0005 `## m3b-t10 — interim backend`.

**Interfaces:**
- Consumes: Tasks 3, 4; `sql_lexer::{lex, exec_comment}`; `sqlparser` (strict, `MySqlDialect` for MySQL **and** MariaDB, `SQLiteDialect`, recursion limit 50).
- Produces: `struct InterimBackend; impl Backend for InterimBackend` with the combine rule `MostDangerousOfAll { deny_if_any_fails: true }` (`alg/11` :66): every **threshold view** of the text (each executable-comment version gate active and inactive, at most `max_exec_comment_views` = 16; an `AmbiguousCommentEnd` body read both ways, ledger 126/138) must parse in strict mode; token-boundary agreement between our lexer and `sqlparser::tokenizer` (line/column spans mapped through `LineIndex`) else `ParserDisagreement`; effects = the sqlparser AST visitor (derived `Visit`) ∪ **our pre-scan inventory** (`Word(` calls, `INTO OUTFILE|DUMPFILE|@`, `FOR UPDATE|SHARE`, `LOCK IN SHARE MODE`, `:=`, `SET`, `HANDLER`, `XA`, `LOAD DATA`) ∪ the executable-comment flag. SQLite `PRAGMA` is read only for the allowlist that equals `hacivat_guard::SQLITE_READ_PRAGMAS` (equality test in Task 18). **MySQL/MariaDB pre-scan before any parse (P-M3b-8):** `NEXT VALUE FOR`, `PREVIOUS VALUE FOR`, `NEXTVAL(`, `LASTVAL(`, `SETVAL(` → SEQUENCE; `RETURNING` → WRITE_DATA; `FOR SYSTEM_TIME` → Unknown; `/*M!` → executable comment; `BEGIN NOT ATOMIC` → ProceduralCode — `sqlparser` parses several of these across dialects, so parse failure is not the mechanism; `known_gaps.toml` records each. **Read forms (P-M3b-35):** MySQL/MariaDB `SHOW`, `DESCRIBE`/`DESC`, `EXPLAIN` (no ANALYZE) and SQLite read `PRAGMA`s are in the read corpus; where `sqlparser` rejects one, a narrow lexer-level grammar (`SHOW <words> [LIKE '…' | WHERE <read expr>]`, `DESCRIBE <qualified name> [<column>]`, `EXPLAIN [FORMAT=<w>] <statement classified recursively>`, no function call or subquery) classifies it, else Unknown. No `sqlparser` type is public.
- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn safe02_cases_22_to_27_and_31() {}
#[test] fn safe06_m1_to_m19_and_l1_to_l9_offline() { /* run A rows for MySQL and SQLite from the bypass TOML once Task 18 lands; here: the alg/13 table copied into tests/data */ }
#[test] fn read_corpus_has_no_false_denial() { for (engine, q) in read_corpus() { assert_eq!(classify_interim(engine, &q).request_class, CoarseClass::Read, "{engine:?}: {q}"); } }
#[test] fn executable_comment_views_are_all_classified() { assert_eq!(classify_mysql("SELECT 1 /*!80000 , (SELECT 1 FROM t FOR UPDATE) */").request_class, CoarseClass::Write); }
#[test] fn ambiguous_comment_end_is_read_both_ways() {}
#[test] fn ansi_quotes_token_mismatch_denies() { assert_eq!(classify_with_modes(mysql_ansi(), r#"SELECT "a" FROM t"#).request_class, CoarseClass::Read); assert_eq!(classify_with_modes(mysql_default(), r#"SELECT "a"" FROM t"#).request_class, CoarseClass::Unknown); }
#[test] fn mariadb_sequence_functions_are_not_read() { for s in ["SELECT NEXT VALUE FOR s", "SELECT NEXTVAL(s)", "SELECT SETVAL(s, 5)"] { assert_ne!(classify_mariadb(s).request_class, CoarseClass::Read, "{s}"); } }
#[test] fn show_describe_explain_are_read() { for s in ["SHOW TABLES", "SHOW FULL COLUMNS FROM t LIKE 'a%'", "DESCRIBE t", "DESC db.t c", "EXPLAIN FORMAT=JSON SELECT * FROM t"] { assert_eq!(classify_mysql(s).request_class, CoarseClass::Read, "{s}"); } }
#[test] fn mariadb_only_syntax_is_unknown() { for s in ["DELETE FROM t RETURNING *", "SELECT NEXT VALUE FOR s", "/*M!100000 DROP TABLE t */ SELECT 1"] { assert_ne!(classify_mariadb(s).request_class, CoarseClass::Read, "{s}"); } }
#[test] fn sqlite_pragma_allowlist() {}
#[test] fn known_gaps_each_have_a_test() {}
```
- [ ] **Step 2:** Run `cargo test -p sql-safety --test interim --test read_corpus`. Expected: FAIL.
- [ ] **Step 3: Implement.** Every gap found goes into `known_gaps.toml` with a test; the read-corpus bar is 0 false denials.
- [ ] **Step 4:** Run `cargo test -p sql-safety` (also `tr_TR`). Expected: PASS.
- [ ] **Step 5: Commit** `feat(sql-safety): interim MySQL/MariaDB/SQLite backend — strict sqlparser per threshold view, token agreement, lexer pre-scan union`.

**Review:** security-sensitive. Probes: a threshold view skipped; the token-agreement check skippable; a sqlparser type in the API; `/*!` inside a string literal; MariaDB syntax misclassified as Read.

### Task 11: Session runs: binds, multiple result sets, Run as one, error policy, queue cap, `ABORT` (HV-EXEC-05/-07/-08 session side; C9, C13; R-M3-60, -61, -66) (W2)

**Files:** `crates/core/hacivat-session/src/session/{run,cursor}.rs`; named lines in `session/{actor,mod,run_gate}.rs` (the run's wait on Task 1c's `RunGate`; `Ladder::command` at `actor.rs:2112` already routes `Command::Choose` there; queue cap; `TX_CONTROL`/`ABORT`); tests `crates/core/hacivat-session/tests/{run_m3b,error_policy,queue}.rs`, Docker `tests/docker_m3b_run.rs`; ADR 0004 `## m3b-t11 — runs`.

**Interfaces:**
- Consumes: Task 1c's `RunUnit` fields, `RunChoice`, `Command::Choose` + `RunGate` (P-M3b-3), `RunEvent::{NeedsChoice, Notice}`, `ExecErrorKind::QueueFull`, FakeDriver binds/notices; M3a's `run.rs:276 query_spec` (binds `&[]`, `max_result_sets: 1`).
- Produces:
  - `query_spec` passes `unit.binds` and `unit.max_result_sets` (≤ 100); `unit.protocol == Simple` with several statements runs "as one" (no suspension; every set its own result via `RunObserver::open_result(unit, ordinal, schema)`); a multi-result unit never suspends; only the last set of the last unit may suspend (R-M3-66).
  - `QueryEvent::Notice` → `RunEvent::Notice { unit, notice }`, at most 100 per statement (rest counted into `UnitEnd` via a `notices_dropped` log line).
  - `ErrorPolicy::Ask`: on a failed unit with later units, emit `RunEvent::NeedsChoice { unit, error, rollback_and_continue }` (true when the PG transaction is `Failed`) and wait on the `RunGate` slot (selecting on its `Notify`, the stop signal and close) while serving cancel and close; `Retry` re-runs the failed unit once more; `Ignore` continues; `IgnoreAll` continues without asking again; `Stop` skips the rest; `RollbackAndContinue` sends `ROLLBACK` (user tx ends, logged), **keeps the session's mode** (a Manual session reopens its transaction before the next unit, `run.rs:366`) and continues (P-M3b-19); a session close or cancel answers `Stop`. `SessionHandle::choose` answers `false` on its receiver when no run waits on that seq.
  - Queue cap (C9): 256 queued requests per session; the 257th answers `ExecErrorKind::QueueFull` without being queued; queued `FetchMore`s coalesce (M3a behaviour kept).
  - `ABORT` is a ROLLBACK synonym on PG in a failed transaction (R-M3-74); a cancelled queued run reports `UnitEnd::Cancelled`, not `Skipped`, with or without `with_runtime` (ledger 318).
- [ ] **Step 1: Write the failing tests.**
```rust
#[tokio::test(start_paused = true)] async fn binds_reach_the_driver_in_slot_order() { let (h, f) = session(fake_pg()); h.run(spec_binds("SELECT $1, $2", &[text("a"), BindValue::Null])).reply.await.unwrap().unwrap(); assert_eq!(f.binds_seen(), vec![vec![text("a"), BindValue::Null]]); }
#[tokio::test(start_paused = true)] async fn hundred_and_fifty_sets_give_a_hundred_results() { /* CALL with 150 sets; max_result_sets 100 → 100 open_result calls, MoreResultSets{skipped: 50} */ }
#[tokio::test(start_paused = true)] async fn multi_result_unit_never_suspends() {}
#[tokio::test(start_paused = true)] async fn run_as_one_uses_simple_protocol_and_one_unit() {}
#[tokio::test(start_paused = true)] async fn ask_policy_waits_for_a_choice_and_serves_cancel() { /* 3 units, unit 2 fails → NeedsChoice; cancel while waiting → Stop: Succeeded, Failed, Skipped */ }
#[tokio::test(start_paused = true)] async fn ignore_all_does_not_ask_again() {}
#[tokio::test(start_paused = true)] async fn rollback_and_continue_replaces_ignore_in_a_failed_pg_tx() { /* user BEGIN; unit fails → tx Failed; NeedsChoice.rollback_and_continue == true; choose RollbackAndContinue → executed contains "ROLLBACK", next unit runs, TxChanged to Idle */ }
#[tokio::test(start_paused = true)] async fn retry_runs_the_failed_unit_once_more() {}
#[tokio::test(start_paused = true)] async fn rollback_and_continue_keeps_manual_mode() { /* Manual; unit 2 fails in PG → RollbackAndContinue → ROLLBACK, mode still Manual, the next unit runs inside a new user tx */ }
#[tokio::test(start_paused = true)] async fn choice_reaches_a_run_that_is_waiting() { /* NeedsChoice emitted → h.choose(seq, Ignore).await == true; the run continues */ }
#[tokio::test(start_paused = true)] async fn close_while_waiting_answers_stop() {}
#[tokio::test(start_paused = true)] async fn queue_over_256_is_queue_full() {}
#[tokio::test(start_paused = true)] async fn abort_in_a_failed_tx_rolls_back() {}
#[tokio::test(start_paused = true)] async fn notices_become_run_events_capped_at_100() {}
#[tokio::test(start_paused = true)] async fn cancelled_queued_run_reports_cancelled() {}
// Docker (#[ignore = "docker"]): PG binds text-format; MySQL CALL with 3 sets; SQLite batch as one; PG RAISE NOTICE text
#[tokio::test] async fn pg_mysql_sqlite_binds_and_multi_results_on_the_matrix() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-session --features test-support --test run_m3b --test error_policy --test queue`. Expected: FAIL.
- [ ] **Step 3: Implement.** No driver future is dropped; a release always ends Hacivat's own transaction (M3a rule); the Ask wait never holds a cursor open past its idle bound (the bound releases it, and the choice still applies).
- [ ] **Step 4:** Run `cargo test -p hacivat-session --features test-support` (every M2b/M3a suite green) and `<main>/target/docker/run-docker-tests.sh --set pr -p hacivat-session --test docker_m3b_run`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(session): bound parameters, multiple result sets, run as one, the Ask error policy with roll back and continue, queue cap`.

**Review:** security-sensitive (what runs after an error). Probes: a unit run after `Stop`; a choice applied to the wrong run seq; `RollbackAndContinue` offered outside a failed PG tx; a suspended cursor left by a multi-result unit; `ABORT` accepted on MySQL.

### Task 13: Drivers: binds, multiple result sets, Run as one, notices and MySQL warnings (HV-EXEC-03 driver half; ER10; R-M3-77, -79) (W2)

**Files:** `crates/core/hacivat-driver-{pg,mysql,sqlite}/src/typed.rs`; named lines in `hacivat-driver-pg/src/exec.rs` (notice text, `exec.rs:108`), `hacivat-driver-mysql/src/{exec,connect}.rs`, `hacivat-driver-sqlite/src/open.rs` (`:484`); **named exception** `third_party/tokio-postgres/src/**` + `PATCHES.md` §2 (fenced) only if `query_with_formats` cannot carry text-format parameters; tests `crates/core/hacivat-driver-{pg,mysql,sqlite}/tests/typed_m3b.rs`, Docker `tests/docker_typed_m3b.rs` (PG, MySQL); ADR 0002 `## m3b-t13 — binds and result sets`.

**Interfaces:**
- Consumes: `QuerySpec.binds` (`typed.rs:54`), `max_result_sets`, `Protocol::Simple`, Task 1c's `Notice`/`QueryEvent::Notice`/`RawConn::warnings`; T7's decoders.
- Produces: `TypedCaps.binds: true` on all three drivers. PG: binds as text-format parameters (the server parses per parameter type, R-M3-79), `statement.params()` count must equal `binds.len()` else `Unsupported` before send; notice text kept (≤ 100, ≤ 16 KiB each, never in `tracing`); `Simple` protocol with several statements yields one `ResultStart…ResultEnd` per set, columns `LogicalType::Text` + `NativeType::Unknown`. MySQL: binary-protocol binds (`Bytes` for text, `Int`/`UInt` when the hint says integer, `NULL`), sets up to `max_result_sets` then drained and counted (existing logic), `COM_QUERY` multi-statements only when the connection is read-write (read-only connections have none, `connect.rs:127` `.multi_statements(!req.read_only)`: `Unsupported` otherwise); `warnings()` runs `SHOW WARNINGS` after `Done` when `warnings > 0` (never on a read-only session: the guard's ROLLBACK follows). SQLite: bind by hint (Integer, Real, Text, Blob, Null); a placeholder without a bind is refused before stepping (`typed.rs:11` "would run as NULL" closed); `Batch` for Run as one.
- [ ] **Step 1: Write the failing tests.**
```rust
#[tokio::test] async fn pg_param_count_mismatch_is_refused_before_send() {}
#[tokio::test] async fn sqlite_unbound_placeholder_is_refused_not_null() { assert_eq!(query_err(sqlite(), "SELECT ?", &[]).await.kind, RawErrorKind::Unsupported); }
#[tokio::test] async fn sqlite_binds_by_hint() {}
#[tokio::test] async fn mysql_binds_integer_hint_as_int() {}
#[tokio::test] async fn pg_notice_text_is_kept_and_capped() {}
#[tokio::test] async fn pg_simple_protocol_yields_one_result_per_statement_untyped() {}
#[tokio::test] async fn mysql_multi_statement_refused_on_read_only() {}
#[tokio::test] async fn value_with_quote_is_bound_not_spliced() { /* bind "x'); DROP TABLE t; --" on all three engines (SQLite in-process; PG/MySQL Docker): the row reads back the exact string, the table still exists */ }
// Docker (#[ignore = "docker"])
#[tokio::test] async fn pg_raise_notice_text_reaches_the_event() {}
#[tokio::test] async fn mysql_show_warnings_after_done() {}
#[tokio::test] async fn mysql_call_three_sets() {}
```
- [ ] **Step 2:** Run the drivers' `typed_m3b` tests. Expected: FAIL.
- [ ] **Step 3: Implement.** If a vendored tokio-postgres change is needed, it is one fenced entry point with its upstream PR text in `PATCHES.md` (M3a T12 pattern).
- [ ] **Step 4:** Run the drivers' suites and `<main>/target/docker/run-docker-tests.sh --set pr -p hacivat-driver-pg --test docker_typed_m3b` and `-p hacivat-driver-mysql --test docker_typed_m3b`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(drivers): bound parameters, multiple result sets, run as one, notice text and MySQL warnings`.

**Review:** security-sensitive. Probes: a bind spliced into text anywhere; a MySQL multi-statement on a read-only connection; notice text written to a log; an unbound SQLite placeholder executing.

### Task 12: Session M2 carries: C3, C6, C7 + per-session liveness, C10, C11; tx publish on close/break (R-M3-73) (W3)

**Files:** `crates/core/hacivat-session/src/session/{env,liveness,pool,cancel}.rs`; named lines in `manager.rs`, `session/actor.rs` (prewarm cache at `:1814-1852`, `:2293`; the close/break publish); tests `crates/core/hacivat-session/tests/{carries_m3b,liveness_m3b}.rs`; ADR 0004 `## m3b-t12 — M2 carries`.

**Interfaces:**
- Consumes: `EnvStep::{StartupScript, TimeZone}` (`env.rs:18`, both produce nothing today, `:70`, `:97`); `LivenessPolicy`, `ProfileActivity` (`liveness.rs:276`), `Hooked { liveness }` (`pool.rs:1212`, `None` for non-default sessions); `SIDE_PREWARM` (`cancel.rs:45`); `TxState`, `SessionEvent::IdleTransaction`.
- Produces:
  - **C3:** `EnvStep::StartupScript` runs `Profile.startup_script` (Task 1c, P-M3b-10) (split by `sql-split` execution mode; refused whole if any boundary is uncertain) after connect and after reset on **read-write** sessions; a read-only session ignores it with one log notice (`exec-startup-script-skipped-read-only`) because the guard's reset would wipe it.
  - **C6:** in Manual mode, when MySQL reports `autocommit` on (`RawConn::autocommit`), the session switches to Auto, emits `TxChanged`, logs one line, and replays nothing that turns autocommit off.
  - **C7 + R-M3-73:** every console session registers its own liveness (`Hooked { liveness: Some(policy) }`); profile idle time is the most recent activity of any session; idle disconnect is skipped while any session holds a user transaction or a user request; `SessionEvent::IdleTransaction` fires **per session**; deactivating a non-default console breaks only that session.
  - **C10:** a MySQL session keeps one warm side connection while a statement has run ≥ 500 ms or a cursor is suspended; it closes 30 s after its last use.
  - **C11:** PG sessions get `SET TIME ZONE <settings ui.time_zone>` in the plan and the replay (never on MySQL, R5); a pooler that refuses the M3a startup option (`idle_in_transaction_session_timeout` as a startup parameter, ledger 294) gets the same setting by `SET` after connect.
  - Close/break: a closed or broken session publishes its final `TxState { status: Idle, owner: None }` and `cursor: None` (ledger 331).
  - The idle-race toast wording (ledger 373) stays a PO note; no event shape changes here.
- [ ] **Step 1: Write the failing tests.**
```rust
#[tokio::test(start_paused = true)] async fn startup_script_runs_after_connect_and_reset_on_read_write() { let (h, f) = session_with_profile(fake_pg(), profile_script("SET search_path = app; SET work_mem = '64MB';")); assert!(f.executed().contains(&"SET search_path = app".into())); reset(&h).await; assert_eq!(count(&f, "SET work_mem = '64MB'"), 2); }
#[tokio::test(start_paused = true)] async fn startup_script_is_skipped_on_read_only_with_a_notice() {}
#[tokio::test(start_paused = true)] async fn mysql_autocommit_on_in_manual_switches_to_auto() {}
#[tokio::test(start_paused = true)] async fn idle_disconnect_waits_for_any_session_tx() { /* default idle; console 2 BEGIN → no disconnect; console 2 COMMIT then idle period → disconnect */ }
#[tokio::test(start_paused = true)] async fn idle_transaction_fires_per_session() {}
#[tokio::test(start_paused = true)] async fn deactivating_a_non_default_console_breaks_only_it() {}
#[tokio::test(start_paused = true)] async fn mysql_side_connection_stays_warm_for_30s() {}
#[tokio::test(start_paused = true)] async fn pg_time_zone_in_plan_and_replay_never_mysql() {}
#[tokio::test(start_paused = true)] async fn closed_session_publishes_idle_tx_and_no_cursor() {}
#[tokio::test(start_paused = true)] async fn startup_option_refused_falls_back_to_set() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-session --features test-support --test carries_m3b --test liveness_m3b`. Expected: FAIL.
- [ ] **Step 3: Implement.** Recovery never replays a user statement; every M2b test stays green (named in the report if any expectation changes, never weakened).
- [ ] **Step 4:** Run `cargo test -p hacivat-session --features test-support` and the Docker `pr` set for the session. Expected: PASS.
- [ ] **Step 5: Commit** `feat(session): startup scripts, MySQL autocommit follow, per-session liveness and profile-wide idle, warm MySQL side connection, PG time zone; publish state on close`.

**Review:** no carry silently changes M2b behaviour; a user transaction is never ended by idle logic; the startup script never runs on read-only.

### Task 14: The orchestrator `classify` and the classifier pool (SQL-SAFE-02 steps; R-M3-3, R-M3-53) (W3)

**Files:** `crates/sql/sql-safety/src/{classify,backend/plan,fingerprint}.rs`; `crates/core/hacivat-exec/src/classify_pool.rs`; tests `crates/sql/sql-safety/tests/classify.rs`, `crates/core/hacivat-exec/tests/classify_pool.rs`; ADR 0005 `## m3b-t14 — classify`.

**Interfaces:**
- Consumes: Tasks 4, 5 (incremental split is not used here; execution-mode `split`), 6 (`hv_norm1`), 9, 10; `sql_split::split(.., SplitMode::Execution)`.
- Produces: `classify` (`alg/13` :149-168): 1. L0 — `max_bytes`, NUL, lexer errors (any `LexErrorCode`, `AmbiguousCommentEnd` included → `Unknown`), `max_tokens`, `max_nesting` (paren + block depth from the lexer) before any backend; 2. pre-pass — client command or COPY data → `pre_decision` Deny (`ClientCommand`); template variable → `TemplateVariable`; 3. execution-mode split with `Expect` → `NoStatement`, `MultiStatement` (for `SingleStatement`), `AmbiguousBoundary`; 4. threshold views; 5. dispatch by `plan_for(dialect)` (`Postgres`/`PgWire(_)` → `PgBackend`; `MySql`/`MariaDb`/`Sqlite` → `InterimBackend`; others → `UnsupportedDialect`); 6. combine, fingerprint (PG: `sql_oracle_pg::fingerprint`, falling back to `hv_norm1` on a parse error, `algo` recorded), backend versions. `ClassifyPool::new(2)` runs `classify` on two std threads with 16 MiB stacks; `classify` fills `classified_len`/`classified_hash` over the exact input (P-M3b-2). `StatementClassifier for ClassifyPool` classifies with `ClassifyCtx.fn_allowlist = allow` (the profile's list, P-M3b-10) and maps the report through `decide(.., Profile::ReadOnlyPlusExplain, Actor::Human)` to `RoCheck`, and `effects` to `SessionEffects`.
- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn prechecks_deny_before_any_backend() { for (s, code) in [("\0", ReasonCode::LexError), ("\\set x 1", ReasonCode::ClientCommand), ("SELECT ${x}", ReasonCode::TemplateVariable), ("", ReasonCode::NoStatement)] { assert!(classify(s, &pg()).pre_decision.unwrap().iter().any(|r| r.code == code), "{s:?}"); } }
#[test] fn nesting_over_256_is_too_complex() { let s = format!("SELECT {}1{}", "(".repeat(257), ")".repeat(257)); assert!(has_reason(&classify(&s, &pg()), ReasonCode::TooComplex)); }
#[test] fn ambiguous_comment_end_is_unknown() {}
#[test] fn single_statement_expect_denies_two() { assert!(has_reason(&classify("SELECT 1; SELECT 2", &pg_single()), ReasonCode::MultiStatement)); }
#[test] fn script_expect_classifies_each_item() { assert_eq!(classify("SELECT 1; DELETE FROM t", &pg_script()).items.len(), 2); }
#[test] fn turkish_process_locale_changes_nothing() { /* run under LC_ALL=tr_TR.UTF-8 in turkey-test */ }
#[test] fn fingerprint_falls_back_on_parse_errors() {}
#[test] fn every_failure_path_is_unknown_or_denied() { /* table of injected backend failures */ }
// exec classify_pool
#[tokio::test] async fn pool_classifies_deep_input_without_overflowing() { let s = nested_scalar_subqueries(250);   // SELECT (SELECT (… (SELECT 1) …)): past prost's decode depth → Unknown (P-M3b-18)
    assert_ne!(pool().classify(s.into(), pg()).await.unwrap().request_class, CoarseClass::Read); }
#[tokio::test] async fn read_only_check_allows_select_denies_insert() {}
#[test] #[ignore = "perf"] fn classify_10kb_unit_under_1ms() {}
```
- [ ] **Step 2:** Run `cargo test -p sql-safety --test classify` and `cargo test -p hacivat-exec --test classify_pool`. Expected: FAIL.
- [ ] **Step 3: Implement.** The bytes classified are exactly the bytes passed; every failure path is `Unknown` with a reason.
- [ ] **Step 4:** Run `cargo test -p sql-safety -p hacivat-exec` and the perf line for `sql-safety`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(sql-safety): the classify orchestrator with L0 limits, pre-pass, threshold views and backend plan; exec classifier pool on 16 MiB threads`.

**Review:** security-sensitive. Probes: input that reaches a backend past a failed precheck; a deep input on a 2 MiB tokio worker; the PG backend called concurrently on two threads (must go through the oracle's serial thread).

### Task 15: Exec gate, parameter preparation, the D-062 wrap, suppression; cursor eligibility ANDed with Read (HV-EXEC-03 binding, -04; R-M3-23, R-M3-55, R-M3-84) (W3)

**Files:** `crates/core/hacivat-exec/src/{gate,params,explain_wrap,suppress,units}.rs`; tests `crates/core/hacivat-exec/tests/{gate,params,explain}.rs`; FTL block `## m3b-t15-gate` in `locales/{en,tr}/hacivat-core.ftl` (`exec-gate-*`, `exec-unsafe-*`, `exec-param-*`); ADR 0006 `## m3b-t15 — gate`.

**Interfaces:**
- Consumes: Task 4 (`gate_class`, `decide`, `opens_write`), Task 6 (`scan_params`); **not** Task 14 — its tests build `SafetyReport`s with a test-local builder (`report(StmtKind::Delete, EffectSet::WRITE_DATA, &[DestructiveKind::NoWhere], target "müşteri")`), and one end-to-end `#[ignore]` test over the real `classify` is turned on by Task 28 (P-M3b-16); M3a `ExecUnit` (`units.rs:84`), `Resolution::Refused { RunAsOne }` (`units.rs:93`).
- Produces:
  - `prepare_units`: `scan_params` per unit → `NeedsValues` (one `ParamPrompt` per distinct name, `remembered` from the store) or `Ready` with `final_text` (rewritten), `binds` in slot order, `masked` (bind values and `PASSWORD :x`/`IDENTIFIED BY` literals masked per HV-EXEC-03), `OffsetMap` from `RewrittenSql.edits`; `TemplateVariable`/`ParamInDdl`/`MixedStyles`/`TooManyBinds` → `Refused`.
  - `gate`: the R-M3-55 table exactly, per unit, from `SafetyReport.items` and `ConnSafety`; `type_to_confirm` = the target table's name as the user wrote it (exact bytes) for `DmlUnsafe` on production and `DROP DATABASE/SCHEMA`; `suppressible = false` whenever `type_to_confirm` is set; `irreversible` for MySQL/MariaDB DDL; `verdict` per unit (`Read` only when `class == Read`; `TxThenConfirm` — no form: the session picks savepoint or its own `BEGIN` at execution time (P-M3b-1); `ExplainAnalyzeRollback`; `ImplicitCommit` for MySQL/MariaDB DDL while `ConnSafety` shows a transaction open (the session refuses one it meets unpredicted); else `Write`/`Unknown`); `effects` mapped to `SessionEffects` (`DDL` set for every DDL statement). **Stale reports:** `gate` denies a unit whose report's `classified_len`/`classified_hash` do not match `final_text` (`exec-gate-stale-classification`, P-M3b-2). `Admin` per R-M3-55 (P-M3b-4); `Session` on read-only per the harmless-`SET` list (P-M3b-6).
  - `ExecUnit.suspendable = head::cursor_eligible(..) && report.class == CoarseClass::Read` (R-M3-23 ANDed).
  - `RefuseReason::RunAsOne` deleted: a `Verbatim` unit becomes runnable as one `Protocol::Simple` unit with `Expect::Script`.
  - `explain_analyze_inner(report)`: `Some(inner class)`; `Routine`/`Admin` inner → `Deny(exec-explain-analyze-refused)`.
- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn hv_exec_04_table_per_level() { /* alg/20 :266-278 every row, mapped through R-M3-55 */ }
#[test] fn read_only_denies_every_non_read_class_including_call() { for s in ["INSERT INTO t VALUES (1)", "CALL p()", "SET x = 1", "CREATE TEMP TABLE x (a int)"] { assert!(matches!(decision(s, conn_ro()), GateDecision::Deny(_)), "{s}"); } }
#[test] fn production_delete_without_where_asks_to_type_the_table() { let d = decision("DELETE FROM müşteri", conn_prod()); assert!(matches!(d, GateDecision::Confirm { type_to_confirm: Some(t), suppressible: false, .. } if &*t == "müşteri")); }
#[test] fn production_dml_is_run_in_tx_then_confirm_in_every_tx_state() { for c in [conn_prod_auto(), conn_prod_manual_idle(), conn_prod_user_tx()] { assert!(matches!(decision("UPDATE t SET a = 1 WHERE id = 3", c), GateDecision::RunInTxThenConfirm { .. })); } }   // the form is the session's (P-M3b-1)
#[test] fn admin_class_confirms_on_production_and_denies_read_only() {}
#[test] fn read_only_allows_set_search_path_only_from_the_list() {}
#[test] fn non_production_unsafe_where_is_calm_confirm_and_setting_turns_it_off() {}
#[test] fn non_production_everything_else_runs() {}
#[test] fn mysql_ddl_with_open_manual_tx_confirms_implicit_commit() {}
#[test] fn unknown_on_production_confirms_never_runs() {}
#[test] fn no_path_turns_deny_into_run() { /* property over generated reports × conns: a Deny from decide(ReadOnly) on a read-only conn is never Run/Warn/Confirm */ }
#[test] fn suppression_keyed_by_console_and_fingerprint() {}
#[test] fn stale_report_is_denied() { let u = prepared_pg("SELECT :a"); let rep = report_over("SELECT :a"); assert!(matches!(gate(&[u], &[rep], &conn_dev(), &SuppressSet::default(), 1).items[0].decision, GateDecision::Deny(ref r) if r.fluent_id == "exec-gate-stale-classification")); }   // final_text is "SELECT $1"
#[test] fn masking_alter_role_password() { assert!(!prepared("ALTER ROLE app PASSWORD :pwd", &[("pwd", "s3cret")]).masked.contains("s3cret")); }
#[test] fn more_than_65535_binds_refused() {}
#[test] fn offset_map_round_trips() {}
#[test] fn cursor_needs_a_read_verdict() { assert!(!unit_suspendable("SELECT f_write()", report_write())); }
#[test] fn explain_analyze_of_delete_is_wrapped_and_of_call_refused() {}
#[test] fn a_verbatim_unit_runs_as_one() {}   // replaces M3a's a_verbatim_unit_is_refused_until_m3b
```
- [ ] **Step 2:** Run `cargo test -p hacivat-exec --test gate --test params --test explain`. Expected: FAIL.
- [ ] **Step 3: Implement.** The gate is pure and never awaits.
- [ ] **Step 4:** Run `cargo test -p hacivat-exec` and `cargo xtask l10n check`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(exec): pre-execution gate with production confirmations, parameter preparation and masking, D-062 wrap, suppression; cursors need a Read verdict`.

**Review:** security-sensitive. Probes: Unknown ever `Run` on production or read-only; a `Commit` wording reachable inside a user tx (must be the savepoint form); `type_to_confirm` case-folded; masked text containing a bind value; a suspendable unit without a Read verdict.

### Task 16: Run service: multiple results and tabs, comment titles, error policy, Count rows, auditor chain, full log, history sink (HV-EXEC-06/-07/-08/-11; HV-RES-10; R-M3-61, -63, -66, -76) (W3)

**Files:** `crates/core/hacivat-exec/src/{service,observer,status,log,titles,count,audit}.rs`; tests `crates/core/hacivat-exec/tests/{service_m3b,titles,count,audit,log_m3b}.rs`; FTL block `## m3b-t16-run` in `hacivat-core.ftl`; ADR 0006 `## m3b-t16 — run service`.

**Interfaces:**
- Consumes: Task 11's events (`NeedsChoice`, `Notice`, multi-result `open_result` ordinals), Task 13 (notices), Task 15's `PreparedUnit`/`GatePlan`, Task 1c's `LogMsg` additions, `RunRequest` fields, `ConfirmPort`, `HistoryMeta`.
- Produces:
  - `ExecService::run` with `prepared`/`plan`: builds `RunUnit`s from `PreparedUnit` (binds, masked, verdict, effects, `max_result_sets: 100`, `protocol: Simple` for Run as one) and the `RunRequest.policy` the app passes (Task 24 sets `Ask` for multi-unit runs when `exec.error_policy_ask` is on), applies pre-run answers (R-M3-84: skipped units logged `SkippedByUser`); without `prepared` it keeps M3a's path unchanged (so the app works until Task 24 rewires it).
  - `RunTicket` gains `pub choices: watch::Receiver<Option<(u64 /*seq*/, RunEvent)>>` carrying the pending `NeedsChoice`/`NeedsCommit` for the app; `NeedsCommit` also goes to `RunRequest.confirm.commit_decision`, whose answer the service forwards with `SessionHandle::commit_decision`.
  - Tabs and titles: one `ResultTab` per result set (`TabKey { console, unit_ordinal, result_ordinal }`), title from the directly preceding comment (`UnitSpan.title_comment`) else `Result {n}`, further sets of the same unit `Result {n} (2)`, titles over 40 graphemes truncated (tooltip keeps the full text; user text never translated or case-mapped); 100-set cap with `ResultSetLimit`.
  - Count rows: `count_sql` (R-M3-76; trailing trivia trimmed after stripping `;` and the top-level `ORDER BY`, P-M3b-27; refused unless the unit's report is Read and no locking clause), `ExecService::count` on a pooled `Service` session when the console has no open transaction, else on the console after release; 30 s timeout; cancellable.
  - Auditor chain (R-M3-63): synchronous auditors (timing, tx marker, cancel progress, gutter state) then `try_send` to the output log and history sinks (capacity 10,000, `Dropped { n }`); `RowsFetched` coalesced to one per 50 ms per result.
  - Log: every new `LogMsg` with literal ids; `command_tag` kept (ledger 340); DDL logs `exec-completed` on every engine (ledger 340 PO rule); notices as `Notice` lines with the server text verbatim in its own field; `SessionLog::lines_since`.
  - History: one `HistoryRecord` per unit through the sink when `history` is `Some` and not private; `statement_masked` only.
- [ ] **Step 1: Write the failing tests.**
```rust
#[tokio::test] async fn hundred_and_fifty_sets_give_a_hundred_tabs() { let t = run_fake_call_sets(150).await; assert_eq!(tabs(&t).len(), 100); assert!(log_has(&t, "exec-result-set-limit")); }
#[test] fn title_from_preceding_comment_and_suffixes() { assert_eq!(titles("-- müşteriler\nCALL p(); -- two sets"), ["müşteriler", "müşteriler (2)"]); }
#[test] fn title_truncates_at_40_graphemes() {}
#[tokio::test] async fn error_policy_choice_flows_through_the_ticket() {}
#[tokio::test] async fn pre_run_skip_is_logged_and_the_unit_skipped() {}
#[tokio::test] async fn needs_commit_reaches_the_confirm_port_and_answer_returns() {}
#[test] fn count_wrap_rules() { assert_eq!(count_sql_of("SELECT * FROM t ORDER BY a;").unwrap(), "SELECT COUNT(*) FROM (\nSELECT * FROM t\n) hv_c"); assert!(count_sql_of("SELECT * FROM t FOR UPDATE").is_none()); assert_eq!(count_sql_of("SELECT * FROM t ORDER BY a LIMIT 5").unwrap(), "SELECT COUNT(*) FROM (\nSELECT * FROM t ORDER BY a LIMIT 5\n) hv_c"); }
#[tokio::test] async fn count_uses_a_service_session_without_a_tx() {}
#[tokio::test] async fn slow_sink_never_slows_the_run_and_counts_drops() {}
#[test] #[ignore = "perf"] fn sync_chain_under_5us_per_event() {}
#[test] fn every_new_log_id_exists_in_en_and_tr() {}
#[test] fn lines_since_reports_a_gap_after_the_ring_wraps() {}
#[test] fn ddl_logs_completed_on_every_engine() {}
#[tokio::test] async fn unmasked_sql_never_reaches_the_history_sink() {}
#[tokio::test] async fn m3a_path_without_prepared_is_unchanged() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-exec --test service_m3b --test titles --test count --test audit --test log_m3b`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Run `cargo test -p hacivat-exec`, `cargo xtask l10n check`, the perf line for `hacivat-exec`. Expected: PASS (every M3a exec test green).
- [ ] **Step 5: Commit** `feat(exec): multiple result tabs with comment titles, error-policy and commit questions on the ticket, Count rows, auditor chain, full log message set, history records`.

**Review:** security-sensitive (masking). Probes: an unmasked statement reaching history or the log copy; a choice applied to a different run; the error policy running a unit after Stop; the M3a path changed.

### Task 17: History store (SQLite + FTS5, redaction, retention) (HV-EXEC-12; D-061; R-M3-62) (W3)

**Files:** `crates/core/hacivat-exec/src/history/**`; tests `crates/core/hacivat-exec/tests/history.rs`; ADR 0006 `## m3b-t17 — history`.

**Interfaces:**
- Consumes: Task 1c's history types; Task 6's `hv_norm1`; `LocalPaths::history_db()` (`hacivat-settings/src/paths.rs:77`). **Search key (ruling, no new dependency):** the FTS5 `unicode61 remove_diacritics 2` tokenizer plus a Turkish pre-fold applied identically to the indexed text and the query (`İ`, `I`, `ı` → `i`; `Ş/ş` → `s`, `Ğ/ğ` → `g`, `Ç/ç` → `c`, `Ö/ö` → `o`, `Ü/ü` → `u`; a fixed table, never locale casing), the D-074 behaviour without reaching `hacivat-l10n`.
- Produces: `HistoryStore` bodies: one writer task on the blocking pool; schema per `alg/20` :620-631 with `text_hash` SHA-256 of the masked text, `fingerprint`, `run_count` (dedup by `(profile, text_hash)` increments it); FTS5 over the masked statement; retention 10,000 per profile and 90 days, pinned entries kept; statements over 64 KiB stored as a 64 KiB prefix with the hash of the whole; file mode 0600 (`create_dir_all_private`); a corrupt file is renamed `history.sqlite.corrupt-<ts>` and a fresh one created; `shutdown` flushes within the grace (2 s); redaction patterns (`alg/20` :611) applied before hashing; private consoles never reach the store (the sink is `None`).
- [ ] **Step 1: Write the failing tests.**
```rust
#[tokio::test] async fn dedup_increments_run_count() {}
#[tokio::test] async fn fingerprint_groups_literal_variants() {}
#[tokio::test] async fn redaction_patterns_before_hashing() { assert!(!stored_text("CREATE USER u IDENTIFIED BY 's3cret'").contains("s3cret")); }
#[tokio::test] async fn retention_keeps_10000_per_source_and_pins() {}
#[tokio::test] async fn turkish_search_matches_both_ways() { for (stored, q) in [("SELECT * FROM İSTANBUL", "istanbul"), ("SELECT müşteri", "musteri")] { assert_eq!(search(stored, q).await.len(), 1, "{q}"); } }
#[tokio::test] async fn recall_by_prefix() {}
#[tokio::test] async fn corrupt_file_rotates() {}
#[tokio::test] async fn statement_over_64_kib_is_capped_with_the_full_hash() {}
#[cfg(unix)] #[tokio::test] async fn file_mode_is_0600() {}
#[tokio::test] async fn shutdown_flushes_within_the_grace() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-exec --test history`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Run `cargo test -p hacivat-exec` (also `tr_TR`). Expected: PASS.
- [ ] **Step 5: Commit** `feat(exec): query history in SQLite with FTS5, redaction, retention and Turkish-tolerant search`.

**Review:** security-sensitive (secrets at rest). Probes: a password literal surviving redaction; history readable by others; the store used as an audit trail anywhere.

### Task 18: SAFE-06 runs A, A′ and B; the read-only classifier hook; MySQL `sql_mode` pin; C8 lost state; C19 MariaDB rows (SQL-SAFE-06; R-M3-56, -57, -58, -83) (W4)

**Files:** `crates/sql/sql-testkit/bypass/{pg,mysql,mariadb,sqlite}.toml` (every row of P1–P39, M1–M19, L1–L9, X1–X5 and the SAFE-02 table; each row: `id`, `sql`, `classifier = { verdict, reasons }`, `server_off = [allowed outcomes]`, `accounts`, `kind = server_blocked | neutralised | privilege_only | non_transactional`, `anchor`); `crates/sql/sql-safety/tests/bypass_run_a.rs`; **named exceptions**: `crates/core/hacivat-session/src/session/readonly.rs` (the body of Task 1c's `Core::step0b`, called before step 1a; the lazily read `Core.observed_sql_mode`), `session/recovery.rs` (`track_statement` uses its `effects` argument, first-keyword scan as the fallback; Task 1c already passes it at every call site, P-M3b-11), `crates/core/hacivat-session/tests/docker_m2b_bypass.rs` (rows move from Rust constants to the TOML, read by path with no crate edge; run B keeps `classifier: None`), `crates/core/hacivat-session/tests/docker_m3a_readonly.rs:1082` (the C19 rows judged), `crates/core/hacivat-guard/src/plan.rs` (MySQL/MariaDB `session_setup` gains the observed `SET SESSION sql_mode`, via a new `plan_for_with(kind, &GuardInputs { sql_mode })` with `sql_mode` from `Core.observed_sql_mode`, read once per connection by an internal `SELECT @@SESSION.sql_mode` on the first `step0b` of a read-only MySQL/MariaDB session (P-M3b-33); `plan_for` unchanged for its M2b callers), `crates/app/hacivat-app/src/connections/boot.rs:171` (one fenced line installing `classifier: Some(ClassifyPool::new(2))`); tests `crates/core/hacivat-session/tests/{readonly_classifier,lost_state_m3b}.rs` (FakeDriver, a fake `StatementClassifier`), `crates/core/hacivat-exec/tests/{cross_crate_equalities,docker_m3b_bypass}.rs` (a real `ClassifyPool`; P-M3b-13); ADR 0004 `## m3b-t18 — classifier on the read-only path`.

**Interfaces:**
- Consumes: Task 14 (`ClassifyPool`, `StatementClassifier`), Task 12 (session carries merged), M3a's guarded path (`guarded_query` → `GuardedCursor::{after_first_chunk, finish}`), `hacivat_guard::{PG_NON_TRANSACTIONAL_FUNCTIONS, SQLITE_READ_PRAGMAS}`.
- Produces:
  - **Step 0b:** every user statement on a read-only session (typed `Run` and M2b `Request::Exec { origin: User }`) is checked by `Core.classifier` in `Core::step0b` (Task 1c's call sites before `guarded` and `guarded_query`) with the profile's `read_only_fn_allowlist`, when it is `Some`; `RoCheck::Deny(d)` → `ExecErrorKind::SafetyDenied(d)`, nothing sent; internal requests (the context probe, the schema probe) are exempt as today. `None` keeps M2b/M3a behaviour (run B).
  - `Profile::ReadOnlyPlusExplain` for humans (R-M3-56); `CALL` → Deny on read-only (ER15); the MySQL driver's `CALL` text-tag check stays as defence in depth (ledger 279); a drained MySQL result without a final OK is judged without a tag (seam: `PostFacts.command_tag: None` handled as "unknown", never as clean — ledger 279).
  - **C8:** `track_statement(t, sql, effects)` uses classifier effects (`SESSION_STATE`, `WRITE_TEMP`, `ADVISORY_LOCK`, `LOCK_TABLES`, `PREPARE`) when present; the first-keyword scan stays the fallback for `None`/Unknown (it errs toward reporting more).
  - **R-M3-57:** MySQL/MariaDB read-only sessions pin the observed `sql_mode` after `COM_RESET_CONNECTION`.
  - **R-M3-58:** the MariaDB rows (C19) are recorded per row in `mariadb.toml` with their observed outcome; the access label stays "read-only (guarded)" for MariaDB.
  - Cross-crate equalities (digest 01 R18): `PG_NON_TRANSACTIONAL_FUNCTIONS` ⊆ the PG denylist; the SQLite pragma lists equal; for random inputs the M2 first-keyword readings (`guard_keywords`) ⊆ the lexer-derived ones.
- [ ] **Step 1: Write the failing tests.**
```rust
// sql-safety/tests/bypass_run_a.rs (PR, offline)
#[test] fn run_a_every_row_matches_verdict_and_reasons() { for row in bypass_rows() { let r = classify(&row.sql, &ctx_for(&row)); assert_eq!(verdict_of(&r, Profile::ReadOnlyPlusExplain), row.classifier.verdict, "{}", row.id); assert_eq!(reason_codes(&r), row.classifier.reasons, "{}", row.id); } }
#[test] fn the_row_set_equals_the_m2b_constants() { /* one-time equality: ids and SQL of every M2b row present in the TOML */ }
// hacivat-session/tests/readonly_classifier.rs (FakeDriver)
#[tokio::test] async fn denied_statement_sends_nothing() { let (h, f) = ro_session_with(fake_classifier_denying("INSERT")); let before = f.call_log().len(); let e = exec_user(&h, "INSERT INTO t VALUES (1)").await.unwrap_err(); assert!(matches!(e.kind, ExecErrorKind::SafetyDenied(_))); assert!(f.call_log()[before..].is_empty()); }   // the env plan ran at connect (P-M3b-18)
#[tokio::test] async fn internal_probe_is_not_classified() {}
#[tokio::test] async fn no_classifier_keeps_m2b_behaviour() {}
#[tokio::test] async fn call_is_denied_on_read_only() {}
#[tokio::test] async fn boot_installs_the_classifier() {}   // hacivat-app test (named exception file list): ManagerConfig from boot has classifier.is_some()
// lost_state_m3b.rs
#[test] fn lost_state_from_effects_and_scan_fallback() {}
// hacivat-exec/tests/cross_crate_equalities.rs (P-M3b-13; uses sql_safety::denied_function_names)
#[test] fn guard_function_list_is_a_subset_of_the_denylist() {}
#[test] fn sqlite_pragma_lists_are_equal() {}
proptest! { #[test] fn guard_keywords_are_a_subset_of_lexer_readings(sql in sql_text()) {} }
// Docker (#[ignore = "docker"]): A′ in hacivat-exec/tests/docker_m3b_bypass.rs (real ClassifyPool); run B in the session's docker_m2b_bypass.rs
#[tokio::test] async fn run_a_prime_classifier_denies_and_side_channel_unchanged() { /* pr = pg17 + mysql84; full = pg14–18, mysql80/84, mariadb106/1011/114 */ }
#[tokio::test] async fn run_b_without_classifier_is_unchanged() {}
#[tokio::test] async fn mysql_sql_mode_is_pinned_after_reset() {}
```
- [ ] **Step 2:** Run `cargo test -p sql-safety --test bypass_run_a` , `cargo test -p hacivat-session --features test-support --test readonly_classifier --test lost_state_m3b` and `cargo test -p hacivat-exec --test cross_crate_equalities`. Expected: FAIL.
- [ ] **Step 3: Implement.** Run B's behaviour is untouched (`classifier: None`).
- [ ] **Step 4:** Run the crates' suites and `<main>/target/docker/run-docker-tests.sh --set full -p hacivat-session --test docker_m2b_bypass` and `--test docker_m3a_readonly`, and `-p hacivat-exec --test docker_m3b_bypass`. Expected: PASS; run A 100 %; A′ every classifier-deny row `SafetyDenied` with the side channel unchanged; classifier-allow rows (P20, P27, the M14 in-statement form) still stopped by L3–L6 exactly as in run B.
- [ ] **Step 5: Commit** `feat(safety): SAFE-06 runs A, A-prime and B; classifier step 0b on read-only sessions; MySQL sql_mode pin; lost state from effects; MariaDB rows recorded`.

**Review:** security-sensitive, **two independent opus reviews** (Global Constraints). Probes: a product build reaching `classifier: None` (`boot_installs_the_classifier`); run B changed; a row whose reasons are "denied" without the expected codes; an `Exec { origin: User }` path that skips step 0b; the classifier trusted for anything but early denial.

### Task 19: Confirm-in-transaction with the D-014 deadline, Smart mode, implicit-commit, the D-062 wrap in the session (HV-EXEC-04 session side; C2; R-M3-55) (W4)

**Files:** `crates/core/hacivat-session/src/session/{confirm,smart}.rs` (bodies); named lines in `session/{run,tx,actor,run_gate}.rs` (`TxMode::Smart` arm at `actor.rs:682`; `spawn_supervised`'s `Smart → Manual` mapping at `actor.rs:214`; the verdict dispatch and the live-state form choice in `run.rs`; `holder = Confirm`); tests `crates/core/hacivat-session/tests/{confirm_tx,smart,explain_wrap}.rs`, Docker `tests/docker_m3b_confirm.rs`; ADR 0004 `## m3b-t19 — confirm in transaction`.

**Interfaces:**
- Consumes: Task 1c's `UnitVerdict`, `TxOwner::{Confirm, is_hacivats}`, `SessionEffects::DDL`, `TxError::ImplicitCommitUnconfirmed`, `RunEvent::{NeedsCommit, CommitDecided}`, `Command::CommitDecision` + `RunGate` (P-M3b-3), `RunSpec.confirm_deadline`; Task 11's run loop; `manual_begin` (`actor.rs:632`).
- Produces:
  - **The form is chosen at execution time from live state (P-M3b-1), per unit:**
    - **Savepoint form** whenever any transaction is open when the unit starts — the user's (`BEGIN` typed earlier, also earlier in the same run), Manual's (the session first sends `manual_begin`: PG `BEGIN`, SQLite `BEGIN IMMEDIATE`, MySQL nothing because `autocommit = 0` keeps one open), or Smart's (opened first for the write, owner `User`): `SAVEPOINT hv_confirm`, the statement, `NeedsCommit { savepoint: true, .. }`; Keep → `RELEASE SAVEPOINT hv_confirm`; Undo/deadline/close → `ROLLBACK TO SAVEPOINT hv_confirm; RELEASE SAVEPOINT hv_confirm`; **the transaction stays open in every branch** and `TxState.owner` stays `User`.
    - **Own-transaction form** only when the session is truly idle in Auto: `BEGIN` (`START TRANSACTION` on MySQL/MariaDB, `BEGIN IMMEDIATE` on SQLite; `holder = Confirm`), the statement, `NeedsCommit { savepoint: false, .. }`; `Commit` → `COMMIT`; `RollBack`, the deadline, a close or a cancel → `ROLLBACK` (Hacivat's own tx; D-014). A close during the wait never answers `CloseRefused` (`is_hacivats`, P-M3b-12) and no idle-transaction warning fires for it.
    - **MySQL/MariaDB never get `START TRANSACTION` while a transaction is open** (it would commit the open one implicitly).
    - `CommitDecided { by_deadline }` is emitted either way. The wait selects on the `RunGate` slot, the stop signal and the deadline.
    - **A failure or cancel of the statement** (before or after `NeedsCommit`): savepoint form → `ROLLBACK TO SAVEPOINT hv_confirm; RELEASE SAVEPOINT hv_confirm`, the user's transaction returns to `InTx`, the unit ends `Failed` with the server text and the log line `exec-confirm-statement-failed` says the transaction is intact; own form → `ROLLBACK` (P-M3b-5).
  - A connection drop while waiting: the server rolled back; `CommitDecided` is not emitted; the unit ends `Failed(ConnectionLost)` and the log says "The server rolled back these changes".
  - `ExplainAnalyzeRollback`: `BEGIN … ROLLBACK` (tx Idle) or `SAVEPOINT hv_explain … ROLLBACK TO SAVEPOINT hv_explain; RELEASE …` (user tx); MySQL `START TRANSACTION … ROLLBACK`.
  - **Smart (C2):** before a unit whose verdict is not `Read` while tx is Idle in `TxMode::Smart`, the session sends `begin_sql` (owner `User`, since the user chose Smart) — a `TxThenConfirm` unit then takes the savepoint form; after a user `COMMIT`/`ROLLBACK` the mode stays Smart and the next non-Read opens again; a `Read` verdict never opens one. `SetMode(Smart)` is accepted from now on (the `NeedsClassifier` arm goes away for sessions created with a classifier-backed exec; without verdicts — `UnitVerdict::Unknown` — Smart opens a tx, the safe direction).
  - `ImplicitCommit`: the unit runs only after the pre-run confirmation the app collected (Task 24); the session logs `ImplicitCommit` when the protocol status shows the tx ended. **A MySQL/MariaDB unit with `SessionEffects::DDL` that meets an open transaction while its verdict is not `ImplicitCommit`** (an earlier unit opened the transaction after the gate ran) is refused with `ExecErrorKind::Tx(TxError::ImplicitCommitUnconfirmed)`, nothing sent (P-M3b-1).
- [ ] **Step 1: Write the failing tests.**
```rust
#[tokio::test(start_paused = true)] async fn auto_idle_confirm_uses_hacivats_own_tx() { let (h, f) = session(fake_pg().tracks_transactions()); let before = f.call_log().len(); let t = run_verdict(&h, "UPDATE t SET a=1 WHERE id=3", UnitVerdict::TxThenConfirm); answer_commit(&h, &t).await;
    assert_eq!(&f.call_log()[before..], [FakeCall::Statement("BEGIN".into()), FakeCall::Typed(FakeTypedCall::Query("UPDATE t SET a=1 WHERE id=3".into())), FakeCall::Statement("COMMIT".into())]); }   // call_log, not executed() (P-M3b-18)
// P-M3b-1, each on PG, MySQL and SQLite FakeDrivers (the expected call logs per engine are in the test table):
#[tokio::test(start_paused = true)] async fn manual_idle_confirm_opens_the_user_tx_then_a_savepoint() { /* PG [BEGIN, SAVEPOINT hv_confirm, Query]; MySQL [SAVEPOINT hv_confirm, Query] (no BEGIN, autocommit = 0); SQLite [BEGIN IMMEDIATE, SAVEPOINT hv_confirm, Query]; owner User */ }
#[tokio::test(start_paused = true)] async fn user_begin_earlier_in_the_run_forces_the_savepoint_form() { /* units: BEGIN; UPDATE … (TxThenConfirm) → SAVEPOINT hv_confirm, no second BEGIN */ }
#[tokio::test(start_paused = true)] async fn smart_write_uses_the_savepoint_form() {}
#[tokio::test(start_paused = true)] async fn mysql_never_starts_a_transaction_inside_one() { /* no START TRANSACTION/BEGIN in the call log after any open tx */ }
#[tokio::test(start_paused = true)] async fn unpredicted_open_tx_refuses_mysql_ddl() { /* units: START TRANSACTION; CREATE TABLE x … (verdict Write, effects DDL) → Tx(ImplicitCommitUnconfirmed), CREATE never sent */ }
#[tokio::test(start_paused = true)] async fn failed_statement_in_savepoint_form_restores_the_user_tx() {}   // P-M3b-5
#[tokio::test(start_paused = true)] async fn cancel_before_the_question_rolls_back_only_the_statement() {}
#[tokio::test(start_paused = true)] async fn close_refuse_if_open_tx_does_not_refuse_hacivats_confirm_tx() {}   // P-M3b-12
#[tokio::test(start_paused = true)] async fn no_idle_tx_warning_for_the_confirm_tx() {}
#[tokio::test(start_paused = true)] async fn deadline_rolls_back_only_the_confirmed_statement() { /* user BEGIN; savepoint form; advance past confirm_deadline → ROLLBACK TO SAVEPOINT hv_confirm; RELEASE SAVEPOINT hv_confirm; status InTx, owner User; CommitDecided { by_deadline: true } */ }
#[tokio::test(start_paused = true)] async fn manual_mode_confirm_uses_a_savepoint_and_keeps_the_user_tx() {}
#[tokio::test(start_paused = true)] async fn close_while_waiting_rolls_back_hacivats_tx_only() {}
#[tokio::test(start_paused = true)] async fn connection_drop_during_confirm_reports_server_rollback() {}
#[tokio::test(start_paused = true)] async fn explain_analyze_is_wrapped_in_a_rollback() {}
#[tokio::test(start_paused = true)] async fn smart_opens_before_write_only() { /* Smart: SELECT (Read) → no BEGIN; UPDATE (Write) → BEGIN first; COMMIT by the user → Idle, mode Smart */ }
#[tokio::test(start_paused = true)] async fn smart_without_a_verdict_opens_a_tx() {}
#[tokio::test(start_paused = true)] async fn savepoint_name_collision_with_a_user_hv_confirm() { /* the user created SAVEPOINT hv_confirm: Hacivat's RELEASE releases its own (PG savepoint stacking) and the user's remains — asserted on the fake's savepoint stack */ }
// Docker (#[ignore = "docker"]): injected 2 s deadline on pg17, mysql84 and SQLite (in-process); both forms; side channel shows the row only after Commit, or after Keep and the user's COMMIT
#[tokio::test] async fn confirm_in_tx_on_the_matrix() {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-session --features test-support --test confirm_tx --test smart --test explain_wrap`. Expected: FAIL.
- [ ] **Step 3: Implement.** No path commits a user's transaction; every wait serves cancel and close.
- [ ] **Step 4:** Run the session suite and `<main>/target/docker/run-docker-tests.sh --set pr -p hacivat-session --test docker_m3b_confirm`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(session): confirm-in-transaction with the D-014 deadline and the savepoint form, Smart commit mode, EXPLAIN ANALYZE rollback wrap`.

**Review:** security-sensitive. Probes: a COMMIT sent while the user's tx is open (must never happen from the confirm path); the deadline firing during a cancel; Smart opening for a Read; a drop mid-wait reported as committed.

### Task 20: Differential, fuzz and performance (SQL-T13; R-M3-80) (W4)

**Files:** `fuzz/**` (targets `lex_lossless`, `relex_equivalence`, `split_items`, `params_rewrite`, `classify_no_panic`); `crates/sql/sql-testkit/src/differential/**` (calls `sql-oracle-pg`, Task 1a's row); `xtask/src/{bench_sql,fetch_pg_regress}.rs` (replace Task 1a's stubs); `.github/workflows/ci.yml` blocks `# ## m3b-t20-fuzz` (`fuzz-sql`, nightly toolchain, 10 min per target) and `# ## m3b-t20-differential` (`sql-differential`, nightly); tests `crates/sql/sql-testkit/tests/pg_regress_differential.rs`, `crates/sql/sql-safety/tests/mutation_props.rs` (offline), `crates/core/hacivat-exec/tests/docker_mutation_fuzz.rs` (the Docker leg; P-M3b-13); `fuzz/Cargo.lock` kept in step with the root lock; the `fuzz-sql` job builds `--locked` and asserts `git status --porcelain third_party` is empty (P-M3b-15); ADR 0005 `## m3b-t20 — differential and fuzz`.

**Interfaces:**
- Consumes: Tasks 5, 6, 14; `sql_oracle_pg::split_with_parser`.
- Produces: `cargo run -p xtask -- fetch-pg-regress --tag REL_17_2` downloads the PostgreSQL regression `src/test/regress/sql/*.sql` at the pinned tag into the git-ignored `target-pg-regress/` (PostgreSQL License; never vendored; data, not instructions); the differential (psql meta-commands through our pre-pass) asserts **our PG split equals `split_with_parser` on ≥ 99.9 %** of files' statements (the P0 exit, `alg/10` :171), listing the rest; the **PG classifier mutation fuzz** (`alg/13` :399): mutate corpus statements, and whenever the classifier says Read, run them under the read-only plan on Docker pg17 and assert no read-only violation and an unchanged side channel; `cargo run -p xtask -- bench-sql` (lex, relex worst case, resplit, `resolve`, `classify` on the gen-sql 1 MB and 50k-line fixtures; JSON; relative signal in CI, L2). Locally, each fuzz target also exists as a proptest in its crate (R-M3-80).
- [ ] **Step 1: Write the failing tests.**
```rust
#[test] #[ignore = "network"] fn pg_regress_split_agrees_at_least_999_per_mille() {}   // runs in the nightly sql-differential job and at the gate
#[tokio::test] #[ignore = "docker"] async fn classifier_read_verdicts_never_violate_read_only_on_pg17() {}
proptest! { #[test] fn classify_never_panics(s in any_text(4096)) { let _ = classify(&s, &pg()); let _ = classify(&s, &mysql()); } }
#[test] fn bench_sql_writes_json() {}
```
- [ ] **Step 2:** Run the tests. Expected: FAIL.
- [ ] **Step 3: Implement**; the fuzz workspace builds with `cargo +nightly fuzz build` in CI only.
- [ ] **Step 4:** Run `cargo run -p xtask -- fetch-pg-regress --tag REL_17_2 && cargo test -p sql-testkit --test pg_regress_differential -- --ignored`, the Docker mutation fuzz through the wrapper, `cargo run -p xtask -- bench-sql`. Expected: PASS; figures in the report.
- [ ] **Step 5: Commit** `test(sql): PG regression differential, classifier mutation fuzz, fuzz targets, bench-sql`.

**Review:** the corpus fetch is pinned and offline-safe; no number is treated as authoritative in CI.

### Task 21: Confirmation and confirm-in-transaction dialogs; the app's ConfirmPort (UI for R-M3-55, R-M3-84) (W4, gpui)

**Files:** `crates/app/hacivat-app/src/exec_ui/{confirm,commit_prompt}.rs` (bodies; `AppConfirmPort` working); tests `crates/app/hacivat-app/tests/confirm.rs`; FTL block `## m3b-t21-confirm` in `hacivat-ui.ftl`; shots fixtures for surfaces 1–2 (`#[cfg(feature = "shots")]`).

**Interfaces:**
- Consumes: Task 15's `ConfirmQuestion`/`ConfirmAnswer`/`CommitQuestion`/`CommitAnswer`, `GateReason`; Task 1b's stubs; `hacivat-ui` `Dialog` (`set_role`, `set_access_keys`), the vault-reset type-to-confirm precedent (`design/m2b-secure/NOTES.md:86-90`); Task 2's mock (binding).
- Produces: `exec_ui::confirm::open(q, cx) -> oneshot::Receiver<ConfirmAnswer>` (480 px alertdialog: env mark, connection mono, "statement k of n", masked SQL ≤ 6 lines in a scroll inset, reasons list with FSI/PDI-isolated names, effect class; type-to-confirm field when `type_to_confirm` — Execute stays disabled until the exact bytes match, Enter in the field never executes; buttons per R-M3-55 with **Cancel focused**, Execute as Danger, "Execute and don't ask again…" only when `suppressible`, its label naming the scope on production; multi-unit footer with Execute all / Skip / Skip all); `exec_ui::commit_prompt::open(q, cx) -> oneshot::Receiver<CommitAnswer>` (480 px: affected rows `exec-confirm-commit`, the countdown "Rolls back in 4:32" repainted once a second on the executor clock (confirm-in-transaction exists only on production, so there is no non-production variant, P-M3b-30); the wording follows `CommitQuestion.savepoint`, which the session reported in `NeedsCommit` (P-M3b-1); **Roll back / Undo focused**; Commit / Keep Default; savepoint wording "Undo this statement; your transaction stays open"). **Any close (Escape, window close, ⌘W, app quit) answers Cancel / RollBack.** `AppConfirmPort` implements `hacivat_exec::ConfirmPort` with the AppUiPort pattern (mpsc to the foreground, oneshot back).
- [ ] **Step 1: Write the failing tests.**
```rust
#[gpui::test] fn any_close_answers_cancel(cx: &mut TestAppContext) { for close in [Close::Escape, Close::Window, Close::CmdW] { assert_eq!(answer_after(cx, close), ConfirmAnswer::Cancel); } }
#[gpui::test] fn enter_in_type_field_never_executes(cx: &mut TestAppContext) { let (vcx, rx) = open_type_to_confirm(cx, "müşteri"); type_text(&vcx, "müşteri"); press(&vcx, "enter"); assert!(rx_is_pending(&rx)); }
#[gpui::test] fn execute_disabled_until_exact_bytes(cx: &mut TestAppContext) { /* "MÜŞTERİ" and "musteri" keep Execute disabled */ }
#[gpui::test] fn cancel_has_initial_focus_and_execute_is_danger(cx: &mut TestAppContext) {}
#[gpui::test] fn no_suppress_button_with_type_to_confirm(cx: &mut TestAppContext) {}
#[gpui::test] fn commit_prompt_focuses_roll_back_and_counts_down_once_a_second(cx: &mut TestAppContext) {}
#[gpui::test] fn savepoint_prompt_says_the_transaction_stays_open(cx: &mut TestAppContext) {}
#[gpui::test] fn quit_with_an_open_commit_prompt_answers_roll_back(cx: &mut TestAppContext) {}
#[gpui::test] fn labels_found_with_access_key_runs(cx: &mut TestAppContext) { /* find_label_runs on every button (F3 CI lesson) */ }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-app --features scene-check --test confirm`. Expected: FAIL.
- [ ] **Step 3: Implement** per the mock.
- [ ] **Step 4:** Run `cargo test -p hacivat-app --features scene-check,pseudo`, `cargo xtask l10n check`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(app): gate confirmation and confirm-in-transaction dialogs with type-to-confirm and the D-014 countdown`.

**Review:** security-sensitive (what the dialog lets run). Probes: Enter executing from any field; a close that answers Execute or Commit; Commit wording in the savepoint form; access keys on SQL text.

### Task 22: Transaction UI: mode and isolation chip, badge, status segment button, idle-transaction toast, single-session toggle (D-014, D-036; C1) (W4, gpui)

**Files:** `crates/app/hacivat-app/src/console/{tx_chip,tx_badge,idle_toast}.rs`; named lines in `console/toolbar.rs` (chip and badge items, pinned), `connections/status.rs` (the tx segment becomes a button opening the chip menu; `TxSegment` at `:290`), `connections/dialog/form.rs` (single-session checkbox in Advanced, writes through `ConnectionsModel`); tests `crates/app/hacivat-app/tests/tx_ui.rs`; FTL block `## m3b-t22-tx-ui`; shots fixtures for surfaces 3–5, 12.

**Interfaces:**
- Consumes: `Request::Tx(TxCommand::{SetMode, SetIsolation, Commit, Rollback})`, `TxState`, `TxError::{InProgress, FailedNeedsRollback}`, Task 12's per-session `IdleTransaction`, `exec_ui::{user_tx, end_tx}` (M3a), `ux::TxBadge`.
- Produces: the chip ("Auto-commit"/"Manual"/"Smart"; disabled with the reason "Commit or roll back first" while a tx is open; read-only sessions show a disabled Auto chip with the reason; production starts Manual per `default_mode`); the badge (pinned toolbar item; amber 7 px stamp; "Transaction · 3 statements · 2 min" repainted once a minute; Commit / Roll back; aborted form with only Roll back; shown only for `TxOwner::User`, never for `Cursor`/`Guard`/`Confirm`); the status tx segment as a button; the idle toast (sticky; Commit / Roll back / Keep; one per session, a newer one replaces it; Keep never rolls back); the single-session checkbox and the "Shared session" chip.
- [ ] **Step 1: Write the failing tests.**
```rust
#[gpui::test] fn chip_sets_mode_and_isolation(cx: &mut TestAppContext) {}
#[gpui::test] fn chip_disabled_with_reason_while_tx_open(cx: &mut TestAppContext) {}
#[gpui::test] fn badge_only_for_user_transactions(cx: &mut TestAppContext) {}
#[gpui::test] fn aborted_badge_offers_only_roll_back(cx: &mut TestAppContext) {}
#[gpui::test] fn idle_toast_keep_never_rolls_back(cx: &mut TestAppContext) {}
#[gpui::test] fn newer_idle_toast_replaces_the_older(cx: &mut TestAppContext) {}
#[gpui::test] fn single_session_checkbox_writes_through_the_model(cx: &mut TestAppContext) {}
#[gpui::test] fn badge_and_segment_are_never_clipped_at_960_tr_xt(cx: &mut TestAppContext) {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-app --features scene-check --test tx_ui`. Expected: FAIL.
- [ ] **Step 3: Implement** per the mock.
- [ ] **Step 4:** Run `cargo test -p hacivat-app --features scene-check,pseudo` (the M3a `status_follow`, `toolbar_narrow` suites green). Expected: PASS.
- [ ] **Step 5: Commit** `feat(app): transaction mode chip, badge with commit and roll back, idle-transaction toast, single-session toggle`.

**Review:** a trust signal clipped; Keep or a toast timeout ending a tx; the badge shown for Hacivat's own transactions.

### Task 23: Cancel escalation and the error-policy bar (HV-EXEC-08, -09 UI; R-M3-61) (W5, gpui)

**Files:** `crates/app/hacivat-app/src/exec_ui/{escalation,error_bar}.rs`; named lines in `exec_ui/cancel.rs` (`cancel` at `:67`: a second press while `Cancelling` opens the dialog); the body of Task 1b's `exec_ui::error_bar::on_choice` (Task 24's orchestrator calls it; P-M3b-17); tests `crates/app/hacivat-app/tests/escalation.rs`; FTL block `## m3b-t23-escalation`; shots fixtures for surfaces 6–7.

**Interfaces:**
- Consumes: `SessionEvent::SlowCancel`, `SessionHandle::escalate(seq, Escalation::{Terminate, Deactivate})`, `CancelOutcome`; Task 16's `RunTicket.choices` (`NeedsChoice`); `SessionHandle::choose`; Task 1b's `ExecMarkState::Escalated` (the gutter badge is painted by Task 34; this task sets the state).
- Produces: the slow-cancel toast at 10 s (Terminate on server — offered only when the engine supports it and the role may — / Deactivate); the second-press dialog (420 alertdialog, **Keep waiting focused**, Terminate on server, Deactivate as Danger with "the statement may continue on the server"; a failed terminate shows the server error verbatim in its own field); the error bar (inline 32 px between editor and results; "Statement 2 of 5 failed"; Retry / Ignore / Ignore all / Stop; **Roll back and continue** in place of Ignore, and no Retry, when `rollback_and_continue` (P-M3b-19); keyboard order Retry → Stop; Escape answers Stop — it never cancels anything running).
- [ ] **Step 1: Write the failing tests.**
```rust
#[gpui::test] fn slow_cancel_toast_after_ten_seconds(cx: &mut TestAppContext) {}
#[gpui::test] fn second_press_opens_the_dialog_with_keep_waiting_focused(cx: &mut TestAppContext) {}
#[gpui::test] fn escape_never_escalates(cx: &mut TestAppContext) {}
#[gpui::test] fn failed_terminate_shows_the_server_text_verbatim(cx: &mut TestAppContext) {}
#[gpui::test] fn error_bar_choice_reaches_the_session(cx: &mut TestAppContext) {}
#[gpui::test] fn error_bar_shows_roll_back_and_continue_in_a_failed_pg_tx(cx: &mut TestAppContext) {}
#[gpui::test] fn closing_the_console_answers_stop(cx: &mut TestAppContext) {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-app --features scene-check --test escalation`. Expected: FAIL.
- [ ] **Step 3: Implement** per the mock.
- [ ] **Step 4:** Run `cargo test -p hacivat-app --features scene-check,pseudo`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(app): slow-cancel toast, cancel escalation dialog, inline error-policy bar`.

**Review:** Escape or a close escalating; Deactivate without its warning; a choice sent to a finished run.

### Task 24: Execution wiring in the app: classify → params → gate → confirmations → run; Run as one; CHOOSE-06 (HV-EXEC-01/-03/-04 app side; R-M3-52, -60, -84) (W5, gpui)

**Files:** `crates/app/hacivat-app/src/exec_ui/{orchestrator,params_prompt,policy_popover,chooser}.rs`; **named exception** `crates/app/hacivat-sql-editor/src/preview.rs:113` (a verbatim selection previews as one unit when `SelectionPolicy::AsOne`); tests `crates/app/hacivat-app/tests/exec_ui_m3b.rs`; FTL block `## m3b-t24-exec-ui`; shots fixtures for surfaces 8–9.

**Interfaces:**
- Consumes: M3a `orchestrator::{start_run (:539), resolve_for_run (:249), agrees (:305)}`, `chooser::open`/`open_uncertain`; Task 14 (`ClassifyPool` on `ExecService::with_classifier`), Task 15 (`prepare_units`, `gate`, `SuppressSet`), Task 16 (`RunRequest.{prepared, plan, policy, run_as_one, history, confirm}`, `RunTicket.choices`), Tasks 21 and 23 (dialogs, bar), Task 17 (`HistoryStore` opened at boot; `HistoryMeta` from the console and `Profile.record_history`; private consoles: none in M3b), Task 18 (no read-only `Deny` for Unknown reaches Run before run A proves 0 false denials; P-M3b-36), `Profile.read_only_fn_allowlist` into `ClassifyCtx.fn_allowlist` (P-M3b-10), `ParamValueStore` (in-memory per app session, R-M3-79); Task 23's bar only through Task 1b's `error_bar::on_choice` and Task 25's limit only through `exec_ui::fetch_policy_for` (P-M3b-17).
- Produces: `start_run` becomes (P-M3b-2): units (M3a) → `prepare_units` (→ the params prompt when `NeedsValues`; Cancel sends nothing) → `classify(PreparedUnit.final_text)` on the pool (off the UI thread; `Expect::SingleStatement`, or `Script` for Run as one) → `gate` with `ConnSafety` from the console's live profile and session snapshot → **Deny**: log line + gutter Failed, nothing sent; **Confirm/Warn**: the confirmation dialogs in unit order (R-M3-84); **RunInTxThenConfirm**: carried in the plan; then one `ExecService::run`. The `preview_truth` rule holds: the units sent are the units `preview_of` showed (M3a gate); with params, the sent text is the rewrite of exactly those ranges. **Run as one** appears in the chooser's uncertain state and for `SelectionPolicy::AsOne`; **CHOOSE-06 "Customize…"** opens the per-console policy popover (Inside / Selection; remembered per console for the session). Until Task 26, production consoles stay read-only: the gate is reached with `read_only: true` there and denies non-Read units exactly like M3a's guard did (the guard still applies underneath).
- [ ] **Step 1: Write the failing tests.**
```rust
#[gpui::test] fn deny_sends_nothing_and_marks_the_gutter(cx: &mut TestAppContext) {}
#[gpui::test] fn params_prompt_cancel_sends_nothing(cx: &mut TestAppContext) {}
#[gpui::test] fn params_are_bound_and_remembered_for_the_session(cx: &mut TestAppContext) {}
#[gpui::test] fn turkish_grouped_number_is_flagged_ambiguous(cx: &mut TestAppContext) {}
#[gpui::test] fn confirmations_asked_in_unit_order_before_the_run(cx: &mut TestAppContext) {}
#[gpui::test] fn skip_all_skips_the_rest_and_logs_it(cx: &mut TestAppContext) {}
#[gpui::test] fn suppressed_fingerprint_runs_without_asking(cx: &mut TestAppContext) {}
#[gpui::test] fn run_as_one_from_the_uncertain_chooser(cx: &mut TestAppContext) {}
#[gpui::test] fn customize_sets_the_console_policy(cx: &mut TestAppContext) {}
#[gpui::test] fn production_stays_read_only_until_the_unlock(cx: &mut TestAppContext) {}
#[gpui::test] fn classification_never_runs_on_the_ui_thread(cx: &mut TestAppContext) {}
#[gpui::test] fn classifier_receives_final_text(cx: &mut TestAppContext) { /* a recording fake pool; PG console `SELECT :a` with a = 1 → the pool saw "SELECT $1" (P-M3b-2) */ }
// hacivat-sql-editor preview_truth property extended (named exception file's own tests): params and AsOne selections
```
- [ ] **Step 2:** Run `cargo test -p hacivat-app --features scene-check --test exec_ui_m3b` and `cargo test -p hacivat-sql-editor`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Run `cargo test -p hacivat-app --features scene-check,pseudo` (M3a `exec_ui` 33 tests green), `cargo test -p hacivat-sql-editor` (properties). Expected: PASS.
- [ ] **Step 5: Commit** `feat(app): classify, parameter prompt, gate and confirmations before Run; run as one; Customize in the chooser`.

**Review:** security-sensitive. Probes: anything sent after Deny, a prompt Cancel or a dialog close; the sent text differing from the previewed range; classification on the UI thread; a production console writing before Task 26.

### Task 25: Results: multi-result tabs, titles and pinning, Count rows, notices, eviction, value-size setting, log carries (HV-EXEC-07 UI; HV-RES-07/-10; R-M3-66, -68, -76) (W5, gpui)

**Files:** named items in `crates/app/hacivat-app/src/results/{tabs,strip,toolbar,output_view,view}.rs`; `results/{count,pin}.rs` (bodies); tests `crates/app/hacivat-app/tests/results_m3b.rs`; FTL block `## m3b-t25-results`; shots fixtures for surface 10.

**Interfaces:**
- Consumes: Task 16 (tabs with comment titles, `TabKey`, `Notice` log lines, `ResultSetLimit`, `count`), Task 8 (`set_visible`, `Evicted`, `cap_raised`), `SessionLog::lines_since`, M3a `ConsoleResults::apply` (`tabs.rs:248`), `ResultEntry`.
- Produces: one tab per result set with the comment title, `(2)` suffixes, a tooltip with the full title; pin from the tab context menu (a pinned tab is never reused; Close, Close others); the "untyped (run as one)" header note; Σ **Count rows** in the result toolbar (estimate vs exact, cancellable spinner, refusal reason); notices and warnings as log lines with severity glyphs and a code chip showing the vendor code beside SQLSTATE for MySQL (`1792 · 25006`, ledger 340); the eviction-closed strip ("Closed to free memory", Re-run); the fetch-all confirmation dialog when Fetch all would pass the per-result cap ("Fetch up to the memory limit" / Cancel); `set_visible` for every console's active tab; `FetchPolicy.cell_cap` from `results.max_value_bytes` through the body of Task 1b's `exec_ui::fetch_policy_for` (Task 24 calls it; P-M3b-17); the output view reads `lines_since` (O(new lines), ledger 336/342/362) and gains ↑/↓ line selection and Enter = Go to source (element-level keys); `results.next_tab`/`prev_tab` bodies; the toolbar `+` vs strip disagreement on `Truncated(FollowingResultSets)` resolved (one source: the strip's count; ledger 342).
- [ ] **Step 1: Write the failing tests.**
```rust
#[gpui::test] fn comment_titles_and_suffixes_on_tabs(cx: &mut TestAppContext) {}
#[gpui::test] fn pinned_tab_is_never_reused(cx: &mut TestAppContext) {}
#[gpui::test] fn count_rows_shows_exact_and_can_be_cancelled(cx: &mut TestAppContext) {}
#[gpui::test] fn notice_line_keeps_server_text_verbatim(cx: &mut TestAppContext) {}
#[gpui::test] fn mysql_chip_shows_vendor_code_and_sqlstate(cx: &mut TestAppContext) {}
#[gpui::test] fn evicted_result_offers_rerun(cx: &mut TestAppContext) {}
#[gpui::test] fn fetch_all_past_the_cap_asks_first(cx: &mut TestAppContext) {}
#[gpui::test] fn active_tabs_are_marked_visible(cx: &mut TestAppContext) {}
#[gpui::test] fn output_view_reads_only_new_lines(cx: &mut TestAppContext) {}
#[gpui::test] fn log_lines_navigate_with_arrows(cx: &mut TestAppContext) {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-app --features scene-check --test results_m3b`. Expected: FAIL.
- [ ] **Step 3: Implement** per the mock; never redefine an M3a id (`hv-results-*`, `hv-grid-*`, `hv-copy-*`, `hv-strip-*`).
- [ ] **Step 4:** Run `cargo test -p hacivat-app --features scene-check,pseudo` (M3a `results_view`, `results_model` green). Expected: PASS.
- [ ] **Step 5: Commit** `feat(app): result tabs per set with comment titles and pinning, Count rows, notices and warnings, eviction and fetch-all confirmation, incremental log view`.

**Review:** a visible result evicted; server text inside a translated sentence; the O(n) re-read surviving; a pinned tab overwritten.

### Task 28: Docker conformance for M3b on the full matrix (E-rows of the M3b gate) (W5)

**Files:** `crates/core/hacivat-exec/tests/docker_m3b_matrix.rs`; helpers block `// ## m3b-t28-matrix` in `crates/core/hacivat-exec/tests/common/mod.rs`; ADR 0006 `## m3b-t28 — conformance`.

**Interfaces:**
- Consumes: everything core of M3b (Tasks 11–19), through document → `resolve_units` → `prepare_units` → `gate` → `ExecService::run` → real session → typed driver, with an admin side-channel check after each case (M3a T31 pattern); lines built from `targets(Some(engine))` classified by properties (M3a T31 fix: no name-based lists).
- Produces (each case on pg14–18, mysql80/84, mariadb106/1011/114, SQLite; pg17-tr for the Turkish cases):
  1. `binds_round_trip_every_type_hint` (and `value_with_quote_is_bound_not_spliced`);
  2. `multi_result_call_and_run_as_one`;
  3. `error_policy_ask_ignore_stop_and_rollback_and_continue` (PG failed tx);
  4. `confirm_in_tx_both_forms_per_engine` (P-M3b-1): Auto idle → own transaction; Manual idle, a user `BEGIN` earlier in the run, and Smart → savepoint; MySQL/MariaDB never `START TRANSACTION` inside one; a failure inside the savepoint leaves the user's tx `InTx` (P-M3b-5); injected 2 s deadline; side channel shows the row only after Commit, or Keep plus the user's COMMIT; the user's tx intact after Undo;
  5. `smart_mode_opens_only_before_writes`;
  6. `explain_analyze_dml_is_rolled_back` (side channel unchanged);
  7. `read_only_classifier_denies_and_server_layers_hold` (a classifier-allow row still stopped);
  8. `notices_and_warnings_reach_the_log`;
  9. `count_rows_matches_count_star`;
  10. `type_goldens_match` (Task 7's tables through `Run`);
  11. `gate_end_to_end_over_real_classify` (Task 15's `#[ignore]` test turned on, P-M3b-16);
  12. `physical_memory_bytes_is_known` (runs in `build-test` on all three OSes, not Docker; ledger 347).
- [ ] **Step 1:** Write the tests; pre-build `cargo test -p hacivat-exec --no-run`.
- [ ] **Step 2:** Run `<main>/target/docker/run-docker-tests.sh --set full -p hacivat-exec --test docker_m3b_matrix` and `--set pr`. Expected: FAIL until every dependency task merged; then PASS. Report a mutation (e.g. the confirm path's `ROLLBACK TO` → `RELEASE` only) going red.
- [ ] **Step 3: Commit** `test(exec): M3b conformance over the Docker matrix`.

**Review:** security-sensitive. Probes: per-family assertions that no engine silently skips; the side channel read with an admin role; a mutation in the confirm path caught.

### Task 26: Production read-write unlock (R-M3-6 REVISED ends; R-M3-29, R-M3-52, R-M3-78, R-M3-81) (W6, gpui)

**Files:** every site that reads or sets the flag (inventory at `7e9209f`): `crates/core/hacivat-session/src/session/mod.rs:569` (`effective_read_only` loses its production arm: `profile.read_only` only; the `production_read_only` parameter is removed), `session/pool.rs:326, :336, :555` and callers `:613, :848, :892, :942, :1295` (`SessionPolicy.production_read_only` deleted; `SessionPolicy` kept empty-or-removed with `set_policy`'s call site), `manager.rs:155-156, :168, :182, :404, :633, :1130, :1337, :1343` (`ManagerConfig.production_read_only` deleted), `test_conn.rs:96, :129`, `lib.rs:38`; `crates/app/hacivat-app/src/connections/boot.rs:171-174`; `crates/app/hacivat-app/src/console/view.rs:162-169, :1247` (`production_read_only(cx)` and the retag-to-production reconnect path: retag now applies through the gate, R-M3-78; the read-only-toggle path of `pending_read_only` stays); `console/toolbar.rs` (the production read-only notice and its popover strings removed — `ToolbarModel.production` keeps the swatch/hairline; the notice item goes); `locales/{en,tr}/hacivat-ui.ftl` (the notice ids deleted from the `m3a-t32-console` block — named exception); every remaining use found by `grep -rn production_read_only crates` at the base, including the app's Test-connection callers of `test_connection_continuing` (`test_conn.rs:96`) (P-M3b-7); every M2b/M3a test that asserts production read-only (the implementer lists each file and assertion, updates it to the confirmation expectation, deletes none: `tests/{prod_read_only,manager}.rs`, the eight P21 app files, `console.rs`, `exec_ui.rs`, `status_follow.rs`, `cross_surface_m3a.rs`, `shots/m3a.rs` states `ConsoleProduction`, `ToolbarProductionWaiting`, `StatusProductionResult`); tests `crates/app/hacivat-app/tests/prod_unlock.rs`, Docker `crates/core/hacivat-exec/tests/docker_prod_unlock.rs`; ADR 0004 `## m3b-t26 — production unlock`.

**Interfaces:**
- Consumes: Tasks 18, 19, 21, 24, 28 merged (their Docker proofs green — the controller checks before dispatch).
- Produces: production consoles open read-write sessions in Manual mode (D-036); every non-Read unit passes the R-M3-55 gate and its confirmations; the hairline, swatch, capsule and env cell unchanged; a profile retagged to production while a console is open gets confirmations from the next Run (no forced reconnect); the read-only toggle keeps M3a's `pending_read_only` flow. The `pool.rs:335`/`manager.rs:155` "M3b removes it" comments go with the code. The M3a production shots change on purpose: listed PNG by PNG (R-M3-81).
- [ ] **Step 1: Write the failing tests.**
```rust
// hacivat-app/tests/prod_unlock.rs
#[gpui::test] fn production_console_is_read_write_in_manual_mode(cx: &mut TestAppContext) {}
#[gpui::test] fn production_delete_without_where_asks_before_anything_is_sent(cx: &mut TestAppContext) { let (vcx, f) = prod_console(cx, "DELETE FROM t;"); run_at(&vcx, 0); assert!(f.typed_log().is_empty()); assert!(confirm_dialog_open(&vcx)); }   // DmlUnsafe → Confirm before send (P-M3b-18)
#[gpui::test] fn production_update_runs_in_a_savepoint_and_asks_keep_or_undo(cx: &mut TestAppContext) { /* Manual default: call_log [Statement("BEGIN"), Statement("SAVEPOINT hv_confirm"), Typed(Query("UPDATE t SET a=1 WHERE id=3"))], the Keep/Undo prompt open */ }
#[gpui::test] fn production_delete_without_where_needs_the_typed_name(cx: &mut TestAppContext) {}
#[gpui::test] fn retag_to_production_applies_confirmations_on_the_next_run(cx: &mut TestAppContext) {}
#[gpui::test] fn read_only_toggle_still_reconnects_read_only(cx: &mut TestAppContext) {}
#[gpui::test] fn no_production_read_only_notice_remains(cx: &mut TestAppContext) {}
#[test] fn no_production_read_only_symbol_remains() { /* grep over crates/: no `production_read_only` identifier */ }
// Docker (#[ignore = "docker"]) on pg17, mysql84, mariadb114, SQLite
#[tokio::test] async fn production_dml_without_an_answer_leaves_nothing_visible() { /* DELETE: Cancel → nothing sent; UPDATE: no answer → side channel never sees the change, deadline undoes it; Keep + the user's COMMIT → row present */ }
#[tokio::test] async fn production_dml_rolls_back_at_the_deadline() {}
#[tokio::test] async fn production_select_runs_without_a_dialog() {}
```
- [ ] **Step 2:** Run the new tests. Expected: FAIL.
- [ ] **Step 3: Implement** the deletion and the test updates (none weakened: each production read-only assertion becomes "the write is confirmed before it is sent", with the same strength).
- [ ] **Step 4:** Run every line of the full test set that touches session, exec and app; the Docker test through the wrapper; the shots with `shot-diff` against both baselines (only the listed production shots move). Expected: PASS.
- [ ] **Step 5: Commit** `feat(safety): production consoles are writable behind confirmations; delete the production read-only flag`.

**Review:** security-sensitive, **two independent opus reviews** (one given the code only). Probes: any path where a production non-Read unit reaches the session without a gate decision (the Request::Exec user path included); a dialog close that sends; a retagged console writing without confirmation; a weakened test.

### Task 27: History dialog (HV-EXEC-12 UI; D-061) (W6, gpui)

**Files:** `crates/app/hacivat-app/src/history/**`; tests `crates/app/hacivat-app/tests/history_ui.rs`; FTL block `## m3b-t27-history`; shots fixtures for surface 11.

**Interfaces:**
- Consumes: Task 17's `HistoryStore::{search, pin}`; Task 1b's `history::open`, `history.open` chord; the focused console. **Named exception:** this task adds `pub fn insert_at_caret(console: ConsoleId, text: &str, cx: &mut App) -> bool` to `exec_ui/mod.rs`; it replaces the editor selection through `ImeCore::dispatch` + `apply_committed` (the M3a rule for programmatic edits) as one undo group, and returns `false` while a composition is active (deferred, M3a T27 rule).
- Produces: the 640 px dialog per the mock: search (Turkish-tolerant, debounced 150 ms, off the UI thread), filters (this connection / all, outcome), rows (time in locale medium time with ISO tooltip, env mark, connection mono, outcome glyph, duration, rows, first SQL line, pin), preview pane (masked SQL, error, redacted marker), Enter inserts at the caret, ⌘↵/Ctrl+↵ inserts and runs, Open in new console, Copy SQL, Pin.
- [ ] **Step 1: Write the failing tests.**
```rust
#[gpui::test] fn search_is_turkish_tolerant(cx: &mut TestAppContext) {}
#[gpui::test] fn enter_inserts_at_the_caret(cx: &mut TestAppContext) {}
#[gpui::test] fn cmd_enter_inserts_and_runs_through_the_gate(cx: &mut TestAppContext) {}
#[gpui::test] fn redacted_entry_shows_the_marker(cx: &mut TestAppContext) {}
#[gpui::test] fn empty_and_no_match_states(cx: &mut TestAppContext) {}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-app --features scene-check --test history_ui`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Run `cargo test -p hacivat-app --features scene-check,pseudo`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(app): query history dialog with Turkish-tolerant search, insert and run`.

**Review:** sonnet. The UI thread never awaits SQLite; run-from-history goes through the same gate.

### Task 29: Shots and cross-surface tests for M3b (W7, gpui)

**Files:** `crates/app/hacivat-app/tests/shots/m3b.rs` (new `M3b` enum of states; `Surface::M3b` arm and the `SURFACES` count in `shots.rs`, named lines), `crates/app/hacivat-app/tests/cross_surface_m3b.rs`.

**Interfaces:**
- Consumes: every M3b task's shots fixtures (surfaces 1–13); `ResultStore::from_rows` (no core runtime for grid states).
- Produces: shots of surfaces 1–13 at 1280 × 800 and 960 × 600 × `en`, `tr`, `en-XA`, `tr-XT` × dark/light, each state with a specific marker and model checks (M3a T35 rules: drawn-only markers; trust check for every state carrying production, transaction, read-only or cursor); cross-surface flows: production `UPDATE` → confirm dialog → Commit → log and gutter; production `DELETE FROM t` → type the name; confirm-in-tx deadline → rolled back line; error policy → Roll back and continue; params prompt → bound run; Run as one → untyped tabs; idle toast → Keep; slow cancel → dialog → Keep waiting; history → insert and run.
- [ ] **Step 1:** Write the cross-surface tests; run `cargo test -p hacivat-app --features scene-check --test cross_surface_m3b`. Expected: PASS.
- [ ] **Step 2:** Run the shots (sharded where useful). Expected: 0 violations; self-checks pass.
- [ ] **Step 3:** `cargo xtask shot-diff --baseline <main>/.superpowers/sdd/2026-11-14-m3-walking-skeleton/baseline-m3a --candidate target/shots` and the same against `baseline-m2b`. Expected: only Task 26's listed production shots differ from `baseline-m3a`; only the 8 known `menu-app-*` from `baseline-m2b`.
- [ ] **Step 4:** **Look at the PNGs** of every new surface at 960 `tr-XT` dark/light; record per-shot and total times. Copy the render to `baseline-m3b/` in the SDD folder for M3c (controller step).
- [ ] **Step 5: Commit** `test(app): M3b shots and cross-surface flows`.

**Review:** opus (design, a11y). Probes: keyboard-only completion of every dialog; Turkish overflow at 960; a marker satisfied by an unrelated label; a trust signal drawn but clipped.

### Task 30: M3b gate (W7)

**Files:** `docs/benchmarks/{m3b.md,po-demo-m3b.md}`.

**M3b exit criteria.** Each is PASS or FAIL in `m3b.md` with evidence:

| # | Criterion | Evidence |
|---|---|---|
| E1 | **SAFE-06 with the classifier on (all denied) and off (server-blocked rows blocked)** — the spec's second M3 exit criterion | Task 18 run A (PR), A′ and B (Docker full); MariaDB rows recorded |
| E2 | **Production consoles write only behind confirmations**: no change becomes visible without an answer (DmlUnsafe/DDL/Admin send nothing before it; DML runs inside a savepoint or Hacivat's own transaction, chosen from live state, P-M3b-1); the deadline rolls back only the confirmed statement; a failure inside the savepoint leaves the user's transaction intact (P-M3b-5) | Tasks 19, 26 (Docker), 28 |
| E3 | Classifier quality: CVE T12 shapes not Read; read corpus 0 false denials per engine; PG split vs `split_with_parser` ≥ 99.9 % on the regression corpus; fuzz targets clean for one nightly | Tasks 9, 10, 20 |
| E4 | Parameters: bound, never spliced, on all engines; classification sees the sent bytes; history stores masked text | Tasks 6, 13, 15, 17, 28 |
| E5 | Multiple result sets (100 cap), Run as one, error policy Ask with Roll back and continue | Tasks 11, 16, 28 |
| E6 | The preview equals what Run sends (properties extended to params and Run as one) | Task 24 |
| E7 | Transaction UI, idle toast, cancel escalation, confirmations: keyboard-complete, Escape never cancels, any close answers the safe value | Tasks 21–23, 29 |
| E8 | M2 carries C3, C6–C11, C13 green; every M2b/M3a test green | Tasks 11, 12, 18 |
| E9 | Incremental relex ≤ 2 ms p95 for typing edits on the 50k-line fixture (R-M3-9); B3 in the real app unchanged (re-run `b3 --bench typing`, `typing-with-results`) | Task 5 perf line; T36 harness on the PO's Mac |
| E10 | Shots: 0 violations; every M3a shot unchanged except Task 26's listed production shots; M1/M2 unchanged except the 8 known | Task 29 |
| E11 | Gates green: clippy, the full test set (perf lines included), check-deps (sqlparser arm), l10n, tokens, `gen-sql-tables --check`, `gen-fn-catalog --check`, `gen-pg-visitor --check`, `cargo deny`, `aws-lc-rs` absent | Step 2 |
| E12 | Budgets: first frame ≤ 500 ms, idle RSS ≤ 200 MB (`--smoke` ×3); release size delta of `sqlparser` recorded | Step 3 |

- [ ] **Step 1:** W7 merged; clean every workspace member in `<main>/target`.
- [ ] **Step 2:** Run clippy, every line of the full test set, both shot-diffs, `check-deps`, `l10n check`, `tokens --check`, the three generator `--check`s, `cargo deny`, the `aws-lc-rs` check. Expected: all green. The controller pushes `m3b` and records CI (`build-test` ×3, `turkey-test`, `docker-pr`, manual `docker-full`, `nightly-package`, `fuzz-sql`, `sql-differential`, `bench`).
- [ ] **Step 3:** On the PO's Mac: `--smoke` ×3; `b3 --bench typing` and `typing-with-results`; record with machine, build hash and captures. If the M3a gate's B3/B1-real numbers are still PENDING-PO, run them here too.
- [ ] **Step 4: Write `po-demo-m3b.md`** (pre-run by the gate agent, then by the PO with Docker up): 1. production pg17 console: `SELECT` runs without a dialog; 2. `UPDATE … WHERE id = 3` → "1 row changed — Commit / Roll back" with the countdown; Roll back; 3. `DELETE FROM orders` → type `orders`; Cancel; 4. Manual mode: `BEGIN; UPDATE …` → Keep / Undo; Undo; the transaction badge still shows the open tx; 5. a read-only profile: `INSERT` denied with the classifier's reason; 6. `SELECT * FROM t WHERE id = :id` → the prompt, `1.234` flagged in `tr`; 7. a MySQL `CALL` with three sets → three tabs; 8. a script with an error in statement 2 of 5 → the bar → Roll back and continue on PG; 9. `SELECT pg_sleep(60)` → Stop, Stop again → the escalation dialog → Keep waiting; 10. the history dialog → search `istanbul` → insert and run; 11. idle transaction toast after the injected interval (demo build flag); 12. EN ↔ TR on every new surface.
- [ ] **Step 5: Write `m3b.md`:** E1–E12; rulings R-M3-51..84 for PO confirmation (especially R-M3-51 split, -52 unlock order, -55 confirmation policy, -59 oracle stays out, -61 error policy, -69 Tab leaves the grid, -71 RTL, -78 retag, -83 injected classifier, -84 pre-run confirmations); the open questions; the carried items' status; the M3c plan pointer.
- [ ] **Step 6: Commit** `docs(m3b): gate record and PO demo checklist`.

---

## M3c "Complete" (W8–W13)

If the controller refuses the split, these tasks follow W7 directly and T30 is a checkpoint. Base branch: `m3c` from the M3b gate tip (or `m3b` if not split); every M3c Global Constraint is this plan's, with `m3c-<slug>` branches and `target-core/m3c-tNN`.

### Task 31: Multi-cursor, column selection, IME mirroring, clipboard metadata, Linux primary selection (UI-EDIT-05 multi, -12 mirroring, -14 cases 2–3; D-072; R-M3-64) (W8, gpui)

**Files:** `crates/ui/hacivat-text/src/{multi,selection,ime}.rs` (named items), `model.rs` (`undo_selection`/`redo_selection`, selection-only history entries); `crates/ui/hacivat-ui-editor/src/{commands/multi,ime,clipboard,mouse}.rs` (alt-drag column selection, named lines); tests `crates/ui/hacivat-text/tests/multi.rs`, `crates/ui/hacivat-ui-editor/tests/{multi_view,ime_mirror}.rs`; ADR 0007 `## m3b-t31 — multi-cursor`.

**Interfaces:**
- Consumes: Task 1b's `multi` signatures and command rows; M3a `SelectionSet` (already many ranges, `selection.rs:150`), `ImeCore` (`ime.rs:45`), `clipboard::{copy, cut, paste}`.
- Produces: the `multi` bodies; every M3a editing command applied per range (type, delete, tab, newline, word moves) with one undo group; the IME composition mirrored to every caret when all carets are in plain text (`TokenOracle` says not inside a string/comment boundary crossing), else the primary-only fallback that collapses the others first (R-M3-64); copy of N ranges writes clipboard metadata (piece lengths) plus the text joined by line breaks, and paste of N pieces into N carets distributes them (with the text-hash fallback when the clipboard text changed outside Hacivat); Linux primary selection written on selection change and pasted by middle click (`app.rs` primary APIs in the fork; Linux only, `#[cfg(target_os = "linux")]`); `undo_selection` (Ctrl/⌘+U) restores the previous selection set without touching text; `status().selected_chars` O(1) per frame (cached by version, ledger 207).
- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn ui_edit_05_cases_3_and_5() {}
#[test] fn typing_at_10k_carets_under_50ms() {}   // perf-gated
#[test] fn select_next_occurrence_is_case_and_turkish_exact() { /* "İ" never matches "i" */ }
#[test] fn column_selection_uses_display_columns_with_tabs_and_wide_chars() {}
#[test] fn undo_selection_restores_carets_without_text_change() {}
#[gpui::test] fn ime_mirrors_to_every_caret_in_plain_text(cx: &mut TestAppContext) {}   // UI-EDIT-12 test 5
#[gpui::test] fn ime_falls_back_to_the_primary_caret_inside_a_string(cx: &mut TestAppContext) {}
#[gpui::test] fn copy_and_paste_n_pieces_into_n_carets(cx: &mut TestAppContext) {}   // UI-EDIT-14 cases 2–3
#[gpui::test] fn foreign_clipboard_text_pastes_whole_at_each_caret(cx: &mut TestAppContext) {}
#[gpui::test] fn one_undo_removes_a_multi_caret_edit(cx: &mut TestAppContext) {}
```
- [ ] **Step 2:** Run the three targets. Expected: FAIL.
- [ ] **Step 3: Implement.** No binding fires during a composition; carets merge per the CodeMirror rule (M3a).
- [ ] **Step 4:** Run `cargo test -p hacivat-text`, `cargo test -p hacivat-ui-editor --features scene-check`, the editor perf line. Expected: PASS.
- [ ] **Step 5: Commit** `feat(editor): multiple carets and column selection, IME mirroring with primary fallback, clipboard pieces, Linux primary selection, undo selection`.

**Review:** IME traces (the macOS matrix rows for several carets join T45's checklist); a binding firing mid-composition; offsets reported to the OS for the primary caret only.

### Task 32: Folding with the chunked height map (UI-EDIT-10) (W8, gpui)

**Files:** `crates/ui/hacivat-text/src/{fold,display/height,display/compose}.rs` (named items); `crates/ui/hacivat-ui-editor/src/fold_gutter.rs`, named lines in `element.rs` (`:737`), `gutter.rs`, `a11y.rs` (folded lines left out of a11y runs), `marks.rs` (a mark inside a fold paints on the fold's header row); `crates/app/hacivat-sql-editor/src/folds.rs` (new, `folding_ranges` from the split: statements ≥ 3 lines, blocks, block comments, `-- region` markers); tests `crates/ui/hacivat-text/tests/fold.rs`, `crates/ui/hacivat-ui-editor/tests/fold_view.rs`, `crates/app/hacivat-sql-editor/tests/folds.rs`; ADR 0007 `## m3b-t32 — folding`.

**Interfaces:**
- Consumes: Task 1b's `FoldSet`, `HeightMap::chunked`, `LanguageService::folding_ranges`, `Decoration::Replace`/`PlaceholderId` (M3a seam), `compose_line`.
- Produces: `FoldSet` bodies (mapped through every change; a fold whose range an edit touches at its edges stays, an edit that deletes its header unfolds it); `HeightMap::Chunked` (O(log n) `y_of`/`line_at_y` over hidden-line runs; chunks of 1,024 lines); `compose_line` joins a folded range into one `VisualLine` with a `Placeholder` fragment (`⋯ 12 lines`); the gutter fold column (chevron on hover and focus; click toggles); folding commands; scroll anchors rebase across folds.
- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn ui_edit_10_cases_1_to_5() {}
#[test] fn chunked_height_map_matches_uniform_without_folds() {}
#[test] fn y_of_and_line_at_y_are_inverse_with_folds() {}
#[test] fn edit_inside_a_fold_keeps_it_edit_on_header_unfolds() {}
#[gpui::test] fn folded_lines_are_not_in_a11y_runs(cx: &mut TestAppContext) {}
#[gpui::test] fn exec_mark_inside_a_fold_paints_on_the_header(cx: &mut TestAppContext) {}
#[test] fn sql_folding_ranges_from_the_split() {}
#[test] #[ignore = "perf"] fn line_at_y_on_50k_lines_with_1000_folds_under_5us() {}
```
- [ ] **Step 2:** Run the targets. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Run `cargo test -p hacivat-text -p hacivat-sql-editor`, `cargo test -p hacivat-ui-editor --features scene-check`, perf lines. Expected: PASS; B3 unaffected (Task 44 re-measures).
- [ ] **Step 5: Commit** `feat(editor): folding over a chunked height map with a gutter fold column`.

**Review:** Run at a caret inside a folded statement runs the whole statement (the preview shows the fold's header band); a fold hiding an execution failure (must show on the header).

### Task 33: Editor find panel, Turkish-tolerant (UI-EDIT-22 without replace; R-M3-10; D-074) (W9, gpui)

**Files:** `crates/ui/hacivat-ui-editor/src/{find,commands/find,scrollbar}.rs` (named items; sole owner of `scrollbar.rs` in W9: the find-hit markers and the horizontal thumb drag moved from Task 34, ledger 207, P-M3b-17); `crates/app/hacivat-app/src/console/find_bar.rs` (the bar, built on `TextInput`, and the `regex`-backed `FindMatcher`); tests `crates/ui/hacivat-ui-editor/tests/find.rs`, `crates/app/hacivat-app/tests/find_bar.rs`; FTL block `## m3b-t33-find`.

**Interfaces:**
- Consumes: Task 1b's `FindMatcher`, `EditorView::{set_find, find_step, select_all_matches, find_status}`, `EditorEvent::FindRequested`; R-M3-10 (regex classes `[iIİı]` and accent classes built over the original text in the app; UI crates take the compiled matcher).
- Produces: the bar per the mock (query, Match case, Regex, Words, Turkish-tolerant toggle — on by default when the UI language or region is Turkish, remembered — "3 of 41", ↑↓, Select all occurrences capped at 10,000, close; Escape returns the caret to where find started; invalid regex turns the field red and searches nothing); counting runs on the background executor in batches; scrollbar hit markers; the macOS find pasteboard (seed from the selection, `cmd-e`) where the fork exposes it.
- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn turkish_tolerant_matches_dotted_and_dotless_i() { let m = matcher("istanbul", tolerant()); assert!(m.find("İSTANBUL", 0).is_some()); assert!(m.find("ıstanbul", 0).is_some()); }
#[test] fn exact_mode_does_not_fold_turkish_letters() {}
#[test] fn words_mode_uses_sql_word_boundaries() {}
#[gpui::test] fn escape_returns_the_caret(cx: &mut TestAppContext) {}
#[gpui::test] fn invalid_regex_searches_nothing(cx: &mut TestAppContext) {}
#[gpui::test] fn select_all_matches_caps_at_10000(cx: &mut TestAppContext) {}
#[gpui::test] fn counting_never_blocks_the_ui_thread(cx: &mut TestAppContext) {}
#[gpui::test] fn keyboard_loop_through_the_bar(cx: &mut TestAppContext) {}
#[gpui::test] fn horizontal_thumb_drags(cx: &mut TestAppContext) {}
```
- [ ] **Step 2:** Run the targets. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Run the editor and app suites. Expected: PASS.
- [ ] **Step 5: Commit** `feat(editor): find panel with Turkish-tolerant matching, select all occurrences and scrollbar markers`.

**Review:** the find field's IME; the Workspace "Focus filter" chord shadowed only while the editor has focus.

### Task 34: Gutter extras, line commands, history recall and editor carries (ED18) (W9, gpui)

**Files:** named items in `crates/ui/hacivat-ui-editor/src/{commands/lines,gutter,marks,mouse,element}.rs` (not `scrollbar.rs`: Task 33 owns it in W9, P-M3b-17); `crates/app/hacivat-app/src/console/commands.rs` (history recall wiring, named lines); tests `crates/ui/hacivat-ui-editor/tests/{lines,gutter_m3b}.rs`; FTL block `## m3b-t34-editor`.

**Interfaces:**
- Consumes: Task 1b's command rows, `ExecMarkState::Escalated`; Task 17's `HistoryStore::recall`; M3a marks/gutter.
- Produces: toggle comment (dialect line comment; block comment for a partial-line selection), duplicate line, delete line, move line up/down (one undo group each; across folds: the fold moves with its lines); mixed line endings detected (`EditorEvent::LineEndingsChanged`) with "Normalize…" to the dominant ending; ▶ on gutter hover runs the statement of that row; gutter focus as AccessKit buttons; the cancel-escalation badge (`Escalated`); the 3-outcome tooltip (last three runs of that statement); `editor.history_prev/next` recall from history at the last line of an empty-or-recalled console; carry: drag autoscroll timer (once per frame while the pointer is past the edge). (The horizontal thumb drag moved to Task 33.)
- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn toggle_comment_uses_the_dialect_line_comment() {}
#[test] fn move_line_moves_a_fold_with_it() {}
#[test] fn normalize_line_endings_to_the_dominant_one() {}
#[gpui::test] fn gutter_play_runs_the_rows_statement(cx: &mut TestAppContext) {}
#[gpui::test] fn escalated_mark_paints_the_badge(cx: &mut TestAppContext) {}   // closes tests/marks.rs:158
#[gpui::test] fn history_recall_at_the_last_line(cx: &mut TestAppContext) {}
#[gpui::test] fn drag_past_the_edge_autoscrolls(cx: &mut TestAppContext) {}
```
- [ ] **Step 2:** Run the targets. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Run the editor and app suites. Expected: PASS.
- [ ] **Step 5: Commit** `feat(editor): line commands, comment toggle, line-ending normalisation, gutter run and escalation badge, history recall, drag autoscroll`.

**Review:** gutter ▶ goes through the same resolve/preview/gate path as ⌘↵.

### Task 35: Grid model: permuted/filtered ViewMap, frozen columns, multi-rect, grow/shrink, select-column chord, auto-size slicing (HV-GRID-02 frozen, -05 multi-rect, -06) (W8, gpui)

**Files:** `crates/ui/hacivat-ui-grid/src/model/{selection,columns,frozen,filter,autosize}.rs`, `element/**` (the frozen layer, edge autoscroll), `view.rs` (named items); tests `crates/ui/hacivat-ui-grid/tests/{model_viewmap,model_frozen,model_multirect,element_frozen}.rs`; ADR 0007 `## m3b-t35 — grid model`.

**Interfaces:**
- Consumes: Task 1b's `ViewMap` variants (mapping working), `ColumnLayout::set_frozen`, `GridCommand::{GrowSelection, ShrinkSelection, Freeze, Unfreeze, AutoFit}`, `GridView::set_view_map`.
- Produces: selection under a permuted map (focus and row sets in model rows; rects in view coordinates cleared on re-sort, digest 04 R15); frozen-left columns painted in their own layer with a `seam-strong` edge, never scrolled horizontally; multi-rect selection (⌘/Ctrl-click and -drag) with the M3a copy refusal for non-stackable unions kept; grow/shrink selection (cell → row segment → row → all); `grid.select_column` bound (Task 1b's chord); auto-size time-sliced (≤ 240 cells per frame, M3a budget) with per-type width bounds; edge autoscroll while the pointer is still past the edge (ledger 213); fast-scroll thresholds unchanged.
- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn focus_follows_its_model_row_through_a_sort() {}
#[test] fn rects_clear_on_resort() {}
#[test] fn frozen_columns_never_scroll() {}
#[test] fn multi_rect_union_copy_refusal_is_unchanged() {}
#[test] fn grow_and_shrink_selection_steps() {}
#[test] fn filtered_map_counts_only_kept_rows() {}
#[gpui::test] fn edge_autoscroll_while_still(cx: &mut TestAppContext) {}
#[test] #[ignore = "perf"] fn permuted_map_lookup_under_50ns() {}
```
- [ ] **Step 2:** Run the targets. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Run `cargo test -p hacivat-ui-grid --features test-support,scene-check` and the grid perf line; the synthetic B1 (`b1_synthetic`) still passes. Expected: PASS.
- [ ] **Step 5: Commit** `feat(grid): permuted and filtered view maps, frozen columns, multi-rect selection, grow and shrink, edge autoscroll`.

**Review:** the paint path stays allocation-free on cache hits; a frozen layer that breaks fast-scroll.

### Task 36: Local sort with collation; copy through the ViewMap (HV-GRID-07; R-M3-70) (W9, gpui)

**Files:** `crates/ui/hacivat-ui-grid/src/model/sort.rs`; `crates/app/hacivat-app/src/results/{sort,copy}.rs`; `crates/core/hacivat-values/src/valfmt/compare.rs` (new module; module line in `valfmt/mod.rs`, named); tests `crates/app/hacivat-app/tests/results_sort.rs`, `crates/core/hacivat-values/tests/compare.rs`; FTL block `## m3b-t36-sort`.

**Interfaces:**
- Consumes: `GridEvent::SortRequested(SortSpec)`; `icu_collator` (Task 1a row); `ResultSource::snapshot`; M3a `copy_region` (`copy.rs:279`, identity assumption documented at `:42-46`).
- Produces: `valfmt::compare::{comparator_for(col: &ColumnFacts, collation: Option<&str>) -> Comparator, Comparator::cmp(a: ValueRef, b: ValueRef) -> Ordering}` (exact decimals, temporal by instant, text by the column's DB collation mapped to ICU — PG `tr-x-icu`/`tr_TR.utf8`, MySQL `utf8mb4_tr_0900_ai_ci` — else code point, never the UI locale; NULLs at the engine default: PG last ascending, MySQL/SQLite first); the sort job on the background executor (key extraction into a compact array, stable sort, parallel merge above 1M rows, cancellation every 64k comparisons) publishing `ViewMap::Permuted`; "Sorted locally — the database might order differently" when the result is not `Complete`; rows that arrive during or after the sort go to the unsorted tail with "N new rows (not sorted) — Re-sort"; progress after 200 ms; **copy translates view rows through the map** (ledger 253 I5) with a permuted-map test.
- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn turkish_collation_sorts_ilik_irmak_istek_izmir() { assert_eq!(sorted(&["İzmir", "ılık", "Irmak", "istek"], tr_collation()), ["ılık", "Irmak", "istek", "İzmir"]); }
#[test] fn code_point_order_without_a_mapped_collation() {}
#[test] fn decimals_sort_exactly_never_through_f64() { assert_eq!(sorted_dec(&["0.30000000000000000001", "0.3"]), ["0.3", "0.30000000000000000001"]); }
#[test] fn nulls_follow_the_engine_default() {}
#[gpui::test] fn partial_result_shows_sorted_locally(cx: &mut TestAppContext) {}
#[gpui::test] fn new_rows_go_to_the_unsorted_tail(cx: &mut TestAppContext) {}
#[gpui::test] fn copy_after_sort_copies_what_is_shown(cx: &mut TestAppContext) {}
#[gpui::test] fn sort_of_a_million_rows_is_cancellable_and_off_thread(cx: &mut TestAppContext) {}
```
- [ ] **Step 2:** Run the targets. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Run `cargo test -p hacivat-values`, `cargo test -p hacivat-app --features scene-check,pseudo`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(results): local sort by column collation with partial-result banners; copy follows the shown order`.

**Review:** a copy after sort that copies model order; the UI locale used as collation.

### Task 37: Grid find and go to row (HV-GRID-09, -14) (W10, gpui)

**Files:** `crates/ui/hacivat-ui-grid/src/model/goto.rs`; `crates/app/hacivat-app/src/results/{find,goto}.rs`; tests `crates/ui/hacivat-ui-grid/tests/goto.rs`, `crates/app/hacivat-app/tests/results_find.rs`; FTL block `## m3b-t37-find-goto`.

**Interfaces:**
- Consumes: Task 1b's `parse_goto`, `GotoTarget/GotoError`, `GridView::set_highlights`, `ViewMap::Filtered`; Task 33's matcher builder (shared in `console/find_bar.rs`, re-used, not duplicated); `ResultSource::ensure_rows`; the format locale's separators.
- Produces: the find bar replacing the result toolbar's left group (Match case, Regex, Words, Filter rows, Turkish-tolerant, "k of N"); the scan over **display strings** of visible columns in batches of 1,000 from the focus cell on the background executor, a new query cancelling the old; NULL never matches; truncated cells mark "may be incomplete"; Filter rows builds `ViewMap::Filtered`; Select all occurrences ≤ 10,000; go to row: 1-based **view** rows, locale digits and grouping accepted (`1.000` in `tr` is row 1000), the column by 1-based index, exact name, ASCII case-insensitive, then tolerant key; a row beyond `loaded` on a suspended cursor calls `ensure_rows` with a confirmation above 1M rows; beyond a known total → inline error.
- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn goto_row_reads_locale_grouping() { assert_eq!(parse_goto("1.000", &cols(), ',', '.'), Ok(GotoTarget::Row(1000))); }
#[test] fn goto_row_col_by_name_and_index() {}
#[test] fn goto_out_of_range_names_the_last_row() {}
#[gpui::test] fn find_over_truncated_marks_incomplete(cx: &mut TestAppContext) {}
#[gpui::test] fn null_never_matches(cx: &mut TestAppContext) {}
#[gpui::test] fn filter_rows_builds_a_filtered_map(cx: &mut TestAppContext) {}
#[gpui::test] fn goto_past_loaded_fetches_with_a_confirmation_above_1m(cx: &mut TestAppContext) {}
```
- [ ] **Step 2:** Run the targets. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Run the grid and app suites. Expected: PASS.
- [ ] **Step 5: Commit** `feat(results): grid find with filter rows and go to row`.

**Review:** the scan never runs on the UI thread; matches are over what the user sees, not raw values.

### Task 38: Transposed, record and text views; the value viewer (HV-GRID-12; R-M3-82) (W10, gpui)

**Files:** `crates/app/hacivat-app/src/results/{views,value_viewer}.rs`; `crates/ui/hacivat-ui-grid/src/element/transpose.rs` (Task 1b's hook: the axis swap at render; `GridView::set_transposed` is Task 1b's working setter, so this task edits neither `view.rs` nor `element/grid.rs`); `crates/core/hacivat-values/src/valfmt/tooltip.rs` (Task 1c's seam: the raw tooltip under a fraction cap; P-M3b-17); `crates/app/hacivat-app/src/shell/tiles.rs` (inspector visibility serde field, additive, named); tests `crates/app/hacivat-app/tests/results_views.rs`; FTL block `## m3b-t38-views`.

**Interfaces:**
- Consumes: `GridConfig.transposed`, `GridEvent::ValueRequested`, `TileLayout.inspector_w`, `VirtualList`, `valfmt::{format_cell, raw_text_in}` with `Purpose::Tooltip`.
- Produces: the Segmented control (Table / Transposed / Text), Transposed disabled above 100k rows with its reason, never auto-transposed; the record inspector (closed by default; 252 px; ▲▼ and Alt+↑/↓ in context `Inspector`; virtualized above 200 columns; duplicate names suffixed; a one-row result highlights the toggle); the text view (Pretty ASCII extractor, widths capped at 255 chars, line selection copy, > 100k rows asks first); the value viewer (480 px popover: column and type header, wrapped mono body, JSON/XML pretty toggle, hex for binary, the truncated value's total length and **"Re-run with a 64 MiB value limit"** which re-runs the unit with `cell_cap` raised, Copy raw value, Esc closes); the raw tooltip under a fraction cap (`valfmt/display.rs:68`).
- [ ] **Step 1: Write the failing tests.**
```rust
#[gpui::test] fn transposed_disabled_above_100k_rows(cx: &mut TestAppContext) {}
#[gpui::test] fn record_view_navigates_and_suffixes_duplicates(cx: &mut TestAppContext) {}
#[gpui::test] fn text_view_pretty_table_and_line_copy(cx: &mut TestAppContext) {}
#[gpui::test] fn value_viewer_pretty_json_is_display_only(cx: &mut TestAppContext) { /* Copy raw value copies the server text unchanged */ }
#[gpui::test] fn truncated_value_offers_a_rerun_with_the_raised_limit(cx: &mut TestAppContext) {}
#[gpui::test] fn rerun_with_raised_limit_goes_through_the_gate(cx: &mut TestAppContext) {}
```
- [ ] **Step 2:** Run the target. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Run the app and grid suites. Expected: PASS.
- [ ] **Step 5: Commit** `feat(results): transposed, record and text views; read-only value viewer`.

**Review:** the re-run path passes the gate and the preview rule; pretty-printing never changes copied bytes.

### Task 39: Grid context menu, raw display setting, auto-fit (HV-GRID-04 menu; L3 raw setting) (W11, gpui)

**Files:** `crates/app/hacivat-app/src/results/context_menu.rs`; tests `crates/app/hacivat-app/tests/results_menu.rs`; FTL block `## m3b-t39-grid-menu`.

**Interfaces:**
- Consumes: `GridEvent::ContextMenuRequested`; Tasks 35–38's commands; `GridSettings.raw_display` (global) and a per-console-per-column raw override.
- Produces: cell, header and row-number menus (Copy, Copy with headers, Sort ascending/descending, Clear sort, Freeze up to here, Unfreeze, Hide column, Show all, Auto-fit, Raw display, Open value viewer, Go to row), disabled items with reasons; double-click on a header edge auto-fits; raw display switches `DisplayMode::Raw` for the grid or a column (copy is raw already).
- [ ] **Step 1:** Tests: `menu_variants_and_disabled_reasons`, `double_click_header_edge_auto_fits`, `raw_display_per_column_and_global`, `menu_opens_from_the_keyboard_at_the_focus_cell`.
- [ ] **Step 2:** Run. Expected: FAIL. **Step 3:** Implement. **Step 4:** App suite. Expected: PASS.
- [ ] **Step 5: Commit** `feat(results): grid context menu, raw display setting, auto-fit`.

**Review:** sonnet.

### Task 40: Grid AccessKit subtree (HV-GRID-16; R-M3-69) (W10, gpui)

**Files:** `crates/ui/hacivat-ui-grid/src/element/a11y_rows.rs` (Task 1b's hook, called from `element/grid.rs` already); named lines in `view.rs` (`a11y_node` at `:406`) and `model/keys.rs` (Tab leaves from any cell); `crates/core/hacivat-values/src/valfmt/a11y_text.rs` (Task 1c's seam; P-M3b-17); tests `crates/ui/hacivat-ui-grid/tests/a11y_rows.rs`; FTL block `## m3b-t40-grid-a11y` in `hui.ftl` (`a11y-null`, `a11y-truncated`, `a11y-row-of`, `a11y-cell`, `grid-selection-summary`).

**Interfaces:**
- Consumes: the fork's `a11y_synthetic_children`, `window.is_a11y_active()`, `on_a11y_action` (leaf-only synthetic children); M3a's grid node on the focus root (T30 fix).
- Produces: `a11y_rows::visible_nodes(frame, rows, cols) -> Vec<RowNode>` (pure; overscan ±5; header row; ids from `(grid id, model row, col)`; names capped at 1,000 chars; NULL → `a11y-null`; truncated → `a11y-truncated`; placeholders "loading"); zero-paint row children only while AT is active; the active-descendant focus plus a polite live announcement on row/column change; large selection changes announce `grid-selection-summary`; actions Focus, ScrollIntoView, ShowContextMenu; `row_count` exact or `loaded` + "more rows are available"; **Tab leaves the grid from any cell** (R-M3-69).
- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn visible_nodes_for_a_40_by_20_viewport_has_861_nodes() { assert_eq!(count_nodes(visible_nodes(&frame_40x20(), 40, 20)), 861); }   // (40 rows + 1 header row) row nodes + 41 × 20 cell nodes; overscan off in this fixture
#[test] fn ids_are_stable_across_scroll() {}
#[test] fn null_and_truncated_names() {}
#[test] fn nothing_is_built_without_at() {}
#[test] fn tab_leaves_from_any_cell() {}
#[test] #[ignore = "perf"] fn node_build_under_1ms_per_frame() {}
```
  (Digest 04's "about 840" left out the header row; 861 is the fixed value.)
- [ ] **Step 2:** Run. Expected: FAIL. **Step 3:** Implement. **Step 4:** Grid suite + perf line. Expected: PASS.
- [ ] **Step 5: Commit** `feat(grid): AccessKit rows and cells while a screen reader is active; Tab leaves the grid`.

**Review:** zero cost without AT; the manual VoiceOver check (T45) decides whether a fork patch for nested synthetic nodes is needed (PO item).

### Task 41: `ToolbarItem::narrow`, F2 follow-ups and chrome carries (R-M3-75; F2 N1–N4, M8, M9 code) (W11, gpui)

**Files:** `crates/ui/hacivat-ui/src/toolbar.rs` (`narrow` laid out by `Measure`/`Plan`; doc fixes N3 `:14-16`, `:75-76`; ⋯ width reservation); `crates/app/hacivat-app/src/console/toolbar.rs` (narrow forms of the production chip — after Task 26: the read-only lock chip and the env label — and the waiting item); `connections/status.rs` (`GIVE_EDITOR`: the editor cell is `Yield::Never` while the tier is Reduced; a `status_right` slot placing the editor segment right of the version per the mock; Ln/Col leaves after the path); `commands/palette.rs` (editor commands run from the palette: focus restored to the editor before dispatch); `shell/hooks.rs` (`unregister_focus_context`, called on console close); `exec_ui/orchestrator.rs` ("Nothing to run here" through a status sentence instead of a toast; named lines); `console/view.rs` (`ConsoleResults::set_display` re-applied on settings change; the empty-console placeholder line drawn through the editor's placeholder; re-clicking the active console tab re-arms the follow, ledger 331); `console/toolbar.rs` Run uses `IconName::Play`; tests `crates/app/hacivat-app/tests/{toolbar_narrow,status_follow}.rs` (extended), `tests/shots/m3a.rs` (N1/N2 checks, named), `crates/ui/hacivat-ui/tests/toolbar.rs`.

**Interfaces:**
- Consumes: Task 1b's `ToolbarItem::narrow`, `IconName::Play`; M3a's `Toolbar::wrap_row`, `GiveWayRow`, the F2 minimum widths (en 126, tr 141, pseudo 219).
- Produces: every trust item whole down to the 120 px split floor in every locale; N1 (`status-everything` exemption only for pseudo-locales at 960), N2 (`#button-label-console-waiting-cancel` required in waiting states and in the narrow sweep's label filter), N3, N4 (the sweep covers running, cancelling, the pending read-only chip and the strip's "Cursor open"); M8; the carries above; the F2 M9 raw metrics become named theme tokens (`WRAP_LINE_PX`, `WRAP_PAD_PX`, `COMPACT_*`, the strip and menu gaps) consumed from `hacivat-ui` theme constants that Task 43 documents.
- [ ] **Step 1: Write the failing tests.**
```rust
#[gpui::test] fn trust_items_whole_down_to_120px_in_every_locale(cx: &mut TestAppContext) {}   // en, tr, en-XA, tr-XT × idle/running/cancelling/waiting/pending-RO
#[gpui::test] fn reduced_mode_never_gives_way(cx: &mut TestAppContext) {}
#[gpui::test] fn editor_segment_sits_right_of_the_version(cx: &mut TestAppContext) {}
#[gpui::test] fn palette_runs_editor_commands_in_the_editor(cx: &mut TestAppContext) {}
#[gpui::test] fn closing_a_console_unregisters_its_focus_context(cx: &mut TestAppContext) {}
#[gpui::test] fn nothing_to_run_is_a_status_sentence(cx: &mut TestAppContext) {}
#[gpui::test] fn display_settings_follow_a_settings_change(cx: &mut TestAppContext) {}
#[gpui::test] fn empty_console_draws_its_placeholder(cx: &mut TestAppContext) {}
```
- [ ] **Step 2:** Run. Expected: FAIL. **Step 3:** Implement. **Step 4:** `cargo test -p hacivat-ui`, the app suite, the shots (M3a states may change only where N1/N2/M8/the status order change them; listed PNG by PNG). Expected: PASS.
- [ ] **Step 5: Commit** `feat(ui): narrow toolbar forms keep every trust signal whole to the split floor; status-bar Reduced mode and order; chrome follow-ups`.

**Review:** opus (trust signals). Probes: a trust label clipped at any sweep width; a check weakened to pass.

### Task 42: Settings-UI rows (R-M3-44; the M3b settings) (W11, gpui)

**Files:** `crates/app/hacivat-app/src/settings_ui/**` (named rows); tests `crates/app/hacivat-app/tests/settings_ui.rs` (extended); FTL block `## m3b-t42-settings`.

**Interfaces:**
- Consumes: `EditorSettings` (`schema.rs:102`), Task 1c's `ExecSettings`, `ResultsSettings` (`ValueLimit`), `HistorySettings`, `GridSettings` (written through `store.rs`, P-M3b-9); `shell::update_settings` (single writer).
- Produces: rows for tab size, insert spaces, copy line without selection, caret blink; "Confirm UPDATE/DELETE without WHERE off production" (`exec.confirm_unsafe_where`); "Ask what to do when a statement fails" (`exec.error_policy_ask`); "Largest value kept per cell" (`results.max_value_bytes`, 64 KiB–64 MiB, locale-formatted); "Record query history" (`history.enabled`); "Show grid values raw" (`grid.raw_display`). Search finds them in EN and TR.
- [ ] **Step 1:** Tests: `each_row_writes_through_update_settings`, `search_finds_rows_in_tr`, `value_limit_clamps`. **Step 2:** FAIL. **Step 3:** Implement. **Step 4:** PASS.
- [ ] **Step 5: Commit** `feat(settings): editor, execution, results, history and grid rows`.

**Review:** sonnet.

### Task 43: M3c shots, cross-surface tests, DESIGN.md "Query, Results and Grid" (R-M3-30) (W12, gpui)

**Files:** `crates/app/hacivat-app/tests/shots/m3c.rs`, the `Surface::M3c` arm in `shots.rs` (named), `tests/cross_surface_m3c.rs`; `DESIGN.md` (new `### Query, Results and Grid` section under Components, plus the tokens of F2 M9).

**Interfaces:**
- Consumes: every M3c fixture; `design/m3a-query/NOTES.md` and `design/m3b-safe/NOTES.md`; the shipped code (the section records what shipped, not intentions).
- Produces: shots of surfaces 14–24 (rules as Task 29); cross-surface flows (multi-cursor type → undo; fold → run at caret in the fold; editor find → select all; sort → copy; grid find → filter rows → copy; go to row 1.000 in tr; value viewer → re-run with raised limit; VoiceOver-free a11y node checks; narrow pane at 120 px); DESIGN.md: editor (gutter, band, marks, chooser, find, folds), results (tile, tabs, strip, log, views, viewer, menus), grid (cells, header, selection, frozen, sort), dialogs (confirmation, commit prompt, escalation, params, history), the trust-signal rules, the Overflow Rule's `wrap_row` and `narrow` stages, the compact metrics as tokens.
- [ ] **Step 1:** Cross-surface tests PASS. **Step 2:** Shots: 0 violations. **Step 3:** shot-diff against `baseline-m3b` (Task 29's render) and `baseline-m3a`/`baseline-m2b`: only listed changes. **Step 4:** Look at the PNGs; DESIGN.md reviewed against the shots. **Step 5: Commit** `docs(design): Query, Results and Grid; test(app): M3c shots and cross-surface flows`.

**Review:** opus (design).

### Task 44: B3 and B1-real re-run with the M3b/M3c grid and editor; perf lines (R-M3-9, -11, -12) (W12, gpui)

**Files:** named modes in `crates/app/hacivat-app/src/bench/{b3,b1_real}.rs` (`b3 --bench typing-multi-caret`, `typing-with-folds`; `b1_real --sorted`); `docs/benchmarks/m3c-bench-notes.md`.

**Interfaces:**
- Consumes: the M3a harnesses (display validation, tr-TR grading, T0 fixture), Tasks 31, 32, 35, 36.
- Produces: B3 (typing, long line, with results, multi-caret, with 1,000 folds) and B1-real (1M × 50 from pg17 in `tr-TR`, 1440 × 900 at 2× ≥ 118 Hz and 2560 × 1440 at 1×; plus a non-gating sorted run) on the PO's Mac; the grid's own paint p50/p95/p99/max and counts > 3 ms / > 6 ms; the ≤ 2 ms relex figure re-measured in the real app's worker; the 19 MB typing perf in release (ledger 298).
- [ ] **Step 1:** Implement the modes (feature `bench` only). **Step 2:** Run locally on the PO's Mac when the display is awake and unlocked (else PENDING-PO, as M3a T36). **Step 3: Commit** `bench(app): M3c re-run of B3 and B1-real`.

**Review:** the probe point stays at handler entry; graded only on the reference panel and locale.

### Task 45: M3c gate (W13)

**Files:** `docs/benchmarks/{m3c.md,po-demo-m3c.md}`; `design/baselines/m3b/**` after PO sign-off.

| # | Criterion | Evidence |
|---|---|---|
| E1 | **The full §6 M3 exit:** read-heavy daily work on all three engines (sort, find, go to row, views, value viewer, history), B1 on real store data, D-011 goldens, SAFE-06 on/off (M3b E1 still green) | Tasks 29–44; demo |
| E2 | Editor: multi-cursor + IME mirroring (macOS matrix rows for several carets, the M3a checklist re-run), folding, find (Turkish-tolerant), line commands | Tasks 31–34; `m3a-ime-checklist.md` extended |
| E3 | Grid: frozen, multi-rect, sort with Turkish collation, find/filter, go to row in `tr`, views, value viewer, context menu, AccessKit subtree (manual VoiceOver smoke, non-gating, L4 at M9) | Tasks 35–40 |
| E4 | Trust signals whole to the 120 px split floor in every locale | Task 41 |
| E5 | **B1-real** ≥ 55 fps, no gap > 100 ms, paint figures recorded; **B3** ≤ 16 ms p95 in every mode; R-M3-9 ≤ 2 ms | Task 44 |
| E6 | Shots 0 violations; only listed changes against the three baselines; DESIGN.md section approved | Task 43; PO |
| E7 | Gates green (as M3b E11) | Step 2 |

- [ ] **Steps:** as Task 30 (clean, full set, shot-diffs, CI record, PO's Mac runs, `po-demo-m3c.md` covering E1–E4 by hand, `m3c.md` with the rulings for PO confirmation and the Deferred table), then **Commit** `docs(m3c): gate record, PO demo checklist, approved M3b/M3c baselines`.

---

## Open questions for the PO (batched; none blocks — the safer or simpler default is in force and logged)

1. **Q1. The unsafe-WHERE guard off production** (`DELETE FROM t` with no WHERE on staging asks once, calm variant, with a setting to turn it off). *Default:* on (R-M3-55). The binding M2a copy says "No extra confirmations" off production; this is the one exception.
2. **Q2. The split** into M3b "Safe" and M3c "Complete" (R-M3-51). *Default:* split, pending the controller's ruling.
3. **Q3. Production default mode Manual** (D-036) means every production DML is a savepoint Keep/Undo inside the session's open transaction (the form is chosen live, P-M3b-1), and the user commits the transaction separately; Commit/Roll back with Hacivat's own transaction appears only on a truly idle Auto session. *Default:* as D-036; the PO may prefer Auto.
4. **Q4. The 5-minute deadline** applies on production only, and in the savepoint form it undoes the statement only. *Default:* as R-M3-55.
5. **Q5. Tab leaves the grid from any cell** (R-M3-69), changing M3a's "only from the last loaded cell". *Default:* leave from any cell (no keyboard trap).
6. **Q6. "Run as one" on PostgreSQL gives untyped (text) columns.** *Default:* offered only where boundaries are uncertain or for an explicit As-one selection.
7. **Q7. Large values:** "Re-run with a 64 MiB value limit" instead of lazy fetch (R-M3-82). *Default:* re-run; lazy fetch arrives with row identity in M6.
8. **Q8. MySQL warnings text** is fetched with `SHOW WARNINGS` after each statement that reports warnings (not on read-only sessions). *Default:* on.
9. **Q9. Bind values are remembered for the app session only**, never written to disk (history keeps them masked). *Default:* session only.
10. **Q10. New chords** (Task 1b table), especially `ctrl-y` = delete line on Windows/Linux (spec) and `cmd-u`/`ctrl-u` = undo selection; and the shadowing they cause (P-M3b-21): the datagrip `ctrl-w` [Grid] hides `workspace.close_tab` while a grid has focus, and `cmd-f`/`ctrl-f` in the editor or grid hides the explorer's filter focus; ledger 336's Ctrl+Tab/⌘3 replaced by `ctrl-pagedown/pageup` in a new `Results` context. *Default:* as the table.
11. **Q11. RTL in the grid** stays v1.x (R-M3-71), overruling an M3a review note. *Default:* v1.x.
12. **Q12. DDL logs "Completed" on every engine** (not "0 rows affected" on MySQL/SQLite, ledger 340). *Default:* "Completed".
13. **Q13. The function allowlist** for read-only sessions lives in the profile TOML with no UI until M4/M5 (R-M3-56). *Default:* file only.
14. **Q14. Read-only `SET`** (P-M3b-6): a read-only console runs `SET` only for a per-dialect harmless list (PG `search_path`, `statement_timeout`, `lock_timeout`, `work_mem`, `DateStyle`, `IntervalStyle`, `TimeZone`, `extra_float_digits`, `application_name`; MySQL/MariaDB `time_zone`, `max_execution_time`, `div_precision_increment`); M3a's guard ran any `SET` harmlessly. *Default:* the list.
15. **Q15. Confirm-statement failure** (P-M3b-5): inside the savepoint form a failing statement is rolled back to `hv_confirm` and the user's transaction stays usable, unlike psql where it would stay aborted. *Default:* restore the transaction.
16. Carried from M3a and still open: the Turkish strings of every new surface; the 8 `menu-app-*` shots; the Windows/Linux IME hosts; the fork patch for nested a11y nodes if VoiceOver needs it.

## Self-review notes

- **Spec coverage (§6 M3, the M3a Deferred table and the ledger).** Every row of M3a's Deferred table marked M3b has an owner:

  | M3a Deferred row (M3b) | Tasks |
  |---|---|
  | Params and binds, template detection, HvNorm1 | T6, T13, T15, T24 |
  | Incremental relex/resplit, StreamSplitter, ≤ 2 ms budget | T5, T20, T30, T44 |
  | Exec-comment threshold views, `sql-ident`, `sql-safety` core, PG backend, interim backend, function data, orchestrator, SAFE-06 A/A′, MySQL `sql_mode` pin, fuzz, PG regression differential | T3, T4, T9, T10, T14, T18, T20 |
  | Gate and confirmations, RW on production, D-062 wrap, RunInTxThenConfirm + D-014 deadline, implicit-commit warning, Smart, `CALL` denial on read-only | T15, T18, T19, T21, T24, T26 |
  | Error policy Ask/IgnoreAll, roll back and continue, the bar | T11, T16, T23 |
  | Multiple result sets, Run as one | T11, T13, T16, T24, T25 |
  | History store, auditor chain, history dialog | T16, T17, T27 |
  | Notices, MySQL warnings, Count rows, eviction, fetch-all confirmation, large values and `results.max_value_bytes` | T8, T13, T16, T25, T38 |
  | Full type matrix | T7 |
  | M2 carries C3, C6–C11, C13 | T11, T12, T18 |
  | Multi-cursor, column selection, IME mirroring, clipboard metadata, Linux primary | T31 |
  | Folding + chunked height map, find (R-M3-10), gutter extras, Undo selection, line endings/Normalize, line commands, `[editor]` Settings rows | T31–T34, T42 |
  | Frozen, grow/shrink, multi-rect, sort with collation, grid find, go to row, three views, value viewer, context menu, auto-fit, AccessKit subtree | T35–T40 |
  | Result-tab comment titles and pinning; grid raw display | T16, T25, T39 |
  | Tx badge, mode/isolation chip, aborted banner, idle toast, single-session toggle; confirmation, confirm-in-tx and escalation dialogs; slow-cancel toast | T21–T23 |
  | DESIGN.md section; B1-real re-run | T43, T44 |
  | CHOOSE-06 | T24 |
  | Grid "select column" chord | T1b, T35 |

  The ledger's routings are in "Carried items" with owners. The §6 M3 scope items "Output log, history and transaction UI" and "production UX surfaces: … confirmations" land in M3b; the M3 exit's SAFE-06 criterion is M3b E1; "read-heavy daily work" is M3c E1.
- **Amend pass (2026-11-16).** The M3b pre-flight's Q1–Q38 are applied as P-M3b-1..38 ("Pre-flight rulings"; `amend-m3b-rulings.md`): the confirm-in-transaction form is the session's live choice; classification runs over `final_text`; out-of-band answers are `Command`s into a `RunGate` slot; `GateClass` is total with `Admin` and an `Unknown` default; T1a split into T1a + T1c and T2 into T2a + T2b (48 tasks, numbers stable); same-wave conflicts removed by seams; wrong test values corrected; two counter-rulings (P-M3b-33 `sql_mode` source, P-M3b-37 icon assets).
- **Pre-flight lessons.** Exact signatures from the code at `7e9209f` (inventoried read-only: `effective_read_only` sites, `RunUnit`/`ErrorPolicy`/`TxOwner`, `QuerySpec.binds`, `GIVE_EDITOR`, `ViewMap::Identity`, `HeightMap::Uniform`, `relex.rs:22`, `Analysis.checkpoints`); a manifest table claimed exhaustive for both halves; fixed test values (`$1, $2, $1`; `[0:2]={1,2,3}`; `1.000` → 1000; 861 a11y nodes; `"SELECT COUNT(*) FROM (\nSELECT * FROM t\n) hv_c"`); shared files controlled by seams and named exceptions; Task 1 split into 1a/1b with 1b on 1a's manifest commit.
- **Type consistency.** Names used across tasks as Tasks 1a/1b/1c define them: `Command::{Choose, CommitDecision}` and `RunGate` (no `Request` variants), `SessionHandle::{choose, commit_decision} -> oneshot::Receiver<bool>`, `TxOwner::is_hacivats`, `TxError::ImplicitCommitUnconfirmed`, `SessionEffects::DDL`, `Core::{step0b, classifier, observed_sql_mode}`, `track_statement(.., effects)`, `Profile::{startup_script, read_only_fn_allowlist, record_history}`, `ValueLimit`, `SafetyReport::{classified_len, classified_hash}`, `GateClass::Admin`, `set_allowed`, `denied_function_names`, `exec_ui::{error_bar::on_choice, fetch_policy_for}`, `GridView::set_transposed`, `valfmt::{tooltip, a11y_text}`; `UnitVerdict` (`TxThenConfirm` without a form flag), `SessionEffects`, `StatementClassifier`, `RoCheck`, `SafetyDenial`, `RunChoice`, `CommitAnswer`, `RunEvent::{NeedsChoice, NeedsCommit, CommitDecided, Notice}`, `TxOwner::Confirm`, `ManagerConfig.classifier`; `classify`, `decide`, `gate_class`, `opens_write`, `ClassifyCtx`, `SafetyReport`, `StmtReport`, `GateClass`; `scan_params`, `ParamScan`, `RewrittenSql`, `BindPlan`, `hv_norm1`, `Fingerprint`; `prepare_units`, `PreparedUnit`, `Prepared`, `OffsetMap`, `gate`, `GatePlan`, `GateDecision`, `ConnSafety`, `ConfirmPort`, `ConfirmQuestion`, `CommitQuestion`; `ClassifyPool`; `HistoryStore`, `HistoryRecord`, `HistoryMeta`; `SessionLog::lines_since`; `CloseReason::Evicted`, `ResultSource::{cap_raised, set_visible}`, `MemoryBudget::evict_for`; `ViewMap::{Identity, Permuted, Filtered}`, `SortSpec`, `parse_goto`, `FoldSet`, `HeightMap::chunked`, `FindMatcher`, `ToolbarItem::narrow`, `ExecMarkState::Escalated`.
- **Conflict control.** Same-wave pairs that share a crate were checked: W2 T11 (`run.rs`, `cursor.rs`, `actor.rs` named arms) vs T12 moved to W3 (env/liveness/pool/cancel; prewarm lines in `actor.rs` — disjoint from T11's arms); T13 (drivers' `typed.rs`) vs T7 (decoders) in different waves; W3 T15 (`gate/params/units`) vs T16 (`service/observer/status/log/titles/count/audit`) vs T17 (`history/**`) disjoint; W4 T18 (`readonly.rs`, `recovery.rs`) vs T19 (`confirm.rs`, `smart.rs`, `run.rs` named lines, `actor.rs:682`) disjoint; W5 T23 (`escalation`, `error_bar`, `cancel.rs` lines) vs T24 (`orchestrator`, `params_prompt`, `policy_popover`, `chooser`) vs T25 (`results/*`) disjoint; W8 T31 (`multi`, `selection`, `ime`) vs T32 (`fold`, `display/*`) disjoint in `hacivat-text`, both touching `hacivat-ui-editor` only in their own files plus named lines; W9 T33/T34 both in `hacivat-ui-editor/src/commands/` but in their own files (`find.rs`, `lines.rs`) whose rows Task 1b already registered, and `scrollbar.rs` is Task 33's alone (the pre-flight found the original claim false; P-M3b-17). W4 T18/T19 and W5 T23/T24/T25 and W10 T38/T40 are disjoint through Tasks 1b/1c's seams (the full list is in "Waves"). The FTL, ADR, `ci.yml`, `tests/support` and `tests/common` files are append-only blocks.
- **Proportion.** Signatures, test names and fixed values only; bodies appear only for values a ruling fixes (the policy table, the count wrap, the confirm-in-tx statement sequences).
- **Known risks** (also in the gate reports):
  - **Interim classifier false denials** on MySQL/MariaDB read-only connections (sqlparser gaps) — the read corpus bar is 0 and `known_gaps.toml` records each; users of read-only MySQL may meet denials until M5's parser.
  - **The libpg_query parser thread is a serial bottleneck** for classifying long PG scripts (R-M3-53); measured by `bench-sql`.
  - **The production unlock** (T26) changes the most safety-relevant behaviour of the product; it waits for five tasks' Docker proofs and gets two independent reviews.
  - **Confirm-in-transaction semantics on MySQL** (DDL implicit commits, `SAVEPOINT` inside a Manual transaction) are verified only in Docker (T19, T28).
  - **Task 1a/1b/1c size** — three interface commits for 48 tasks (split by the pre-flight, P-M3b-20); the M3a T1b needed two fix rounds for thin seams, so this plan names every shared function and the review checks every Consumes line.
  - **The live confirm-in-transaction choice** (P-M3b-1) is the most delicate session logic in M3b: one rule per engine, tested per engine in T19 (FakeDriver) and T28 (Docker), with the MySQL implicit-commit refusal as the backstop.
  - **Disk near the 60 GiB line**: the 2-slot schedule costs about three waves of wall time in M3c (P-M3b-31).
  - **gpui pressure in M3c**: W8–W12 are all-gpui with 3 slots; M3c is the long pole by wall time, not by risk.
  - **A11y nested nodes** may need a fork patch (R-M3-69) if VoiceOver ignores active-descendant on synthetic leaves.
