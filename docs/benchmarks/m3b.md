# M3b gate record (draft)

Milestone M3b, "Safe": the statement classifier (`sql-ident`, `sql-safety` with the PostgreSQL
backend over libpg_query and the interim `sqlparser` backend for MySQL, MariaDB and SQLite), the
classifier pool, the exec gate and the R-M3-55 confirmation policy; parameters and binds;
incremental relex; the full type matrix; eviction; SAFE-06 runs A, A′ and B; confirm-in-transaction
with the D-014 deadline, Smart mode and the implicit-commit backstop; multiple result sets, Run as
one and the error policy; the run service, history (SQLite + FTS5) and the history dialog; the
transaction UI, the confirmation dialogs and the cancel escalation; the M2 carries; and, last, the
**production unlock** (Task 26): production consoles write, but only behind confirmations.

This is the **draft** gate record, written by the gate agent (Task 30). Everything an agent can do
is done and recorded here. What only the PO can do is marked **PENDING-PO** and is prepared in
`docs/benchmarks/po-demo-m3b.md` (in Turkish, with EN/TR screen text) and in section 11.

- Gate branch: `m3b-t30-gate`, based on `m3b` at `19096e7` (W0–W7 merged: every M3b task, T1a to
  T29). Gate commits: `dc6d020` (rustfmt of `xtask/src/vf.rs` and `spikes/spike-probes`, section 2.1)
  and the docs commit that writes this record and the demo checklist.
- Machine: the PO's Mac (macOS, Apple silicon), with the `hacivat-m2a` Docker stack up for the whole
  run (not started, stopped or pruned by the gate). The screen was locked during the gate lines;
  the PO came to the Mac afterwards, and the B3 benches and smoke ran with the display awake, under
  other agents' concurrent builds (section 4). No hostnames, accounts or personal names are recorded
  here.
- Target dir: `<main>/target-core/merge` (shared, warm), `CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=4`,
  `--locked`. Step 1: every workspace member was cleaned in `<main>/target` (1,199 files, 616 MiB;
  `target/docker` untouched) and in `target-core/merge` (11,543 files, 24.0 GiB), so no artifact
  carried another checkout's `CARGO_MANIFEST_DIR`.

**Verdict: not yet a PASS, and no FAIL.** Every local gate line is green on this tree: clippy
(with and without `--all-features`), the workspace (5,048 passed, 0 failed), every feature line,
every perf line, the generators, `cargo deny`, and the Docker full set (192 passed, 0 failed). The
smoke budgets pass. B3 typing and typing-with-results were run with the PO at the Mac once the
display was awake. They pass, but other agents' builds were running at the same time, so the figures
are provisional (section 4.2).

Three things are still open:
- **CI has not run on M3b code since W0.** `ci.yml` runs on push only for `main`, so no run exists for
  the `m3b` head. That leaves the CI halves of E3 (one clean `fuzz-sql` nightly) and E11 (`build-test`
  ×3, `turkey-test`) open, along with `docker-pr`, `sql-differential`, `nightly-package` and `bench`.
  These are **PENDING (controller)**: dispatch `ci` on `m3b`, or open the `m3b` → `main` PR (section 3).
- **What only the PO can do:** the demo, signing off the changed shots, the TR copy, and the rulings
  (sections 6, 7 and 11).
- **The classifier misses its 1 ms per 10 KB budget** (R-M3-53), as Tasks 10, 14 and 20 predicted.
  Measured here: 1.29–1.62 ms best and up to 3.3 ms with unknown MySQL modes. The gate keeps the
  interim 2 ms assertion and carries the budget forward (section 9). The PO should accept or reject
  that.

## 1. Exit criteria E1 to E12

SDD = `.superpowers/sdd/2026-11-14-m3-walking-skeleton/` (the task reports and reviews cited below).

| # | Criterion | Status | Evidence |
|---|---|---|---|
| E1 | SAFE-06 with the classifier on (all denied) and off (server-blocked rows blocked) | **PASS** | Catalogue `sql-testkit/bypass/{pg,mysql,mariadb,sqlite}.toml`, 324 rows (139/75/**82 MariaDB**/28). Run A (offline, classifier on): `sql-safety/tests/bypass_run_a.rs` 5 tests in the workspace line; every denied row has reasons; the allowed rows are exactly the 14 documented ones; `the_row_set_equals_the_m2b_constants`. Run A′ (classifier on, real `ClassifyPool`, Docker): `hacivat-exec/tests/docker_m3b_bypass.rs` **2/2** here (pg14–18, mysql80/84, mariadb106/1011/114; ≈2,000 row runs, 0 false denials, `t18-bypass-a-prime.tsv`). Run B (classifier off, server layers): `hacivat-session/tests/docker_m2b_bypass.rs` **4/4**, MariaDB rows judged against their recorded outcome; `docker_m3a_readonly` **5/5** with MariaDB judged through Run. T18 merged `61d05bf` (fixes `64b19f0`, `1e8e6f7`); T28 `2e8273a`. Reports `task-m3b-18-report.md`, `-18-review.md`, `-28-report.md`. Note: MariaDB stays "guarded" (R-M3-58): run B records that `CALL` as the owner committed a row without the classifier; with it, step 0b denies |
| E2 | Production consoles write only behind confirmations; DML in a savepoint or Hacivat's own tx chosen from live state; the deadline undoes only the confirmed statement; a failure inside the savepoint keeps the user's tx | **PASS** (automated); demo steps 1–4, 3b PENDING-PO | Docker here: `docker_prod_unlock.rs` **9/9** (`production_dml_without_an_answer_leaves_nothing_visible`: `pg_stat_activity` shows the DELETE never reached the backend; `production_dml_rolls_back_at_the_deadline`; `production_select_runs_without_a_dialog`; `mysql_joined_and_comma_updates_name_the_table_that_changes`; three `run_context_*` legs; `production_myisam_dml_is_confirmed_before_it_is_sent`, `myisam_rollback_is_logged_as_not_undone`), `docker_m3b_confirm.rs` 1/1 (`confirm_in_tx_on_the_matrix`), `docker_m3b_matrix.rs` **14/14** (`confirm_in_tx_both_forms_per_engine` on 11 lines, with an Undo→RELEASE mutation proven red; `denied_plan_sends_nothing_on_production`). Session: `confirm_tx.rs` (`auto_idle_confirm_uses_hacivats_own_tx`, `manual_idle_confirm_opens_the_user_tx_then_a_savepoint`, `user_begin_earlier_in_the_run_forces_the_savepoint_form`, `an_already_committed_change_is_never_offered_keep_or_undo`, `deadline_rolls_back_only_the_confirmed_statement`, `failed_statement_in_savepoint_form_restores_the_user_tx`). App: `prod_unlock.rs` (`no_ungated_send_path_in_the_app`, `closing_a_dialog_never_sends_or_keeps`, `retag_to_production_applies_confirmations_on_the_next_run`, `production_run_refuses_when_its_context_read_times_out`), `cross_surface_m3b.rs` (D-043 `mysql_production_update_is_confirmed_before_it_is_sent`, `production_never_offers_dont_ask_again_nor_execute_all`, `production_delete_from_t_needs_the_name_typed`). T19 `04a0039`, T26 `1894b7d` (two independent reviews APPROVED: `task-m3b-26-review-a.md`, `-review-b.md`). Accepted deviation: a cancelled confirmed statement ends `Cancelled`, not `Failed` (T19 m9) |
| E3 | Classifier quality: CVE T12 shapes not Read; read corpus 0 false denials per engine; PG split vs the parser ≥ 99.9 %; fuzz clean for one nightly | **PASS** (local); **fuzz nightly PENDING (controller: CI)** | CVE T12 shapes: PG `f_write()` in LATERAL/CTE/window ORDER BY/ROLLUP/ARRAY()/ROWS FROM → Unknown (`sql-safety/tests/pg_backend.rs`, T9 ≈580 probes); MySQL CVE-2026-85620 shapes (`GET_LOCK` nested) caught (`tests/interim.rs`). Read corpus: `read_corpora_have_no_false_denial_through_classify` (`tests/classify.rs`, ≥1,200 entries, 4 engines; PG 304 entries with 5 **PO-ruled** denials, section 7.4), `read_corpus_has_no_false_denial` (MySQL 326, MariaDB 330, SQLite 312: 0), `…_on_explicit_server_versions` (MySQL 8.0.12/8.4.0, MariaDB 10.6.0: 0), `escaped_literals_are_no_false_denials_with_unknown_or_known_modes` (23 rows) — all in the workspace line here. PG split differential (T20, `sql-testkit/tests/pg_regress_differential.rs`, REL_17_2, 44,560 statements): **0 disagreements; 1000.00 ‰ of comparable, 988.17 ‰ of all** (279 not comparable, 248 uncertain — every uncertain one fails closed; both buckets pinned ±1 %); byte-identity of the patched fingerprint over 89,342 texts (`oracle_corpus.rs`). Fuzz: local smoke only (5 targets, 2–5 min each, 0 crashes) and Docker mutation fuzz (`docker_mutation_fuzz.rs` 1/1 here; 10k + 40k mutants, 0 read-only violations). **No `fuzz-sql` nightly has run on M3b code** (section 3). Reading of "≥ 99.9 %": met over comparable statements; the PO may want the all-statements reading (988 ‰) stated as accepted |
| E4 | Parameters bound, never spliced; classification sees the sent bytes; history stores masked text | **PASS** | T6 (`sql-split`): `rewrite_preserves_every_other_byte`, `bind_plan_covers_each_placeholder_once`, `a_replacement_never_merges_with_its_neighbours`, `a_unit_of_several_statements_never_binds` (400k-case stress in T6); perf `scan_params_10kb_unit_p95_under_1ms` here. T13 drivers: count mismatch refused before sending; `docker_typed_m3b` 5/5 + 5/5 here. T28: `binds_round_trip_every_type_hint`, `value_with_quote_is_bound_not_spliced` (all lines; pg17-tr Turkish binds byte-exact). Exact bytes: gate denies a stale report (`exec-gate-stale-classification`); `classifier_receives_final_text` (T24: PG `SELECT :a` → the pool sees `SELECT $1`). History: `unmasked_sql_never_reaches_history` (32 secrets absent from db/-wal/-shm), `review_probe_every_case_is_caught_or_a_documented_gap` (87 rows: 74 caught, 13 documented gaps → PO list 7.5) |
| E5 | Multiple result sets (100 cap), Run as one, error policy Ask with Roll back and continue | **PASS** | Session `run_m3b.rs` (150 sets → 100 + `MoreResultSets { skipped: 50 }`; 1,000 clamped), `error_policy.rs` (`ask_policy_waits_for_a_choice_and_serves_cancel`, `choice_reaches_a_run_that_is_waiting`, `close_while_waiting_answers_stop`, `a_cancel_that_races_the_answer_wins`), `queue.rs`. Docker `docker_m3b_matrix` `multi_result_call_and_run_as_one` and the error-policy legs (Auto and Manual on PG; Ignore in tx on MySQL family and SQLite), `docker_m3b_run` 1/1. App `run_as_one_lands_in_untyped_tabs`, `error_policy_roll_back_and_continue` (`cross_surface_m3b.rs`), `a_call_with_three_result_sets_keeps_each_in_its_own_tab` (`exec_ui.rs`). T11 `70b27e8`, T16 `d7975e4` |
| E6 | The preview equals what Run sends (params, Run as one) | **PASS** (by composition; one direct property carried) | `hacivat-sql-editor --test preview_truth` (in the workspace line): `preview_equals_execution_for_exact_items`, `exact_statement_spans_are_what_execution_runs`, `preview_equals_execution_under_the_console_policy` (512 cases), `a_selection_previews_as_one_unit_under_as_one`; `run_policy` shared by `preview_with` and `resolve_units_in` (T24 review). Parameters: the previewed span is the unit; what is sent is the byte-exact rewrite of that unit (T6 `rewrite_preserves_every_other_byte`) and the classifier sees exactly it (`classifier_receives_final_text`). **No single property ties preview → rewrite → sent bytes**; the gate carries one (section 9) |
| E7 | Transaction UI, idle toast, cancel escalation, confirmations: keyboard-complete, Escape never cancels, any close answers the safe value | **PASS** (automated); real window and keyboard PENDING-PO | App line here 846/0: `confirm.rs` 37 (`cancel_has_initial_focus_and_execute_is_danger_except_lock`, `tab_is_trapped_in_visual_order`, `enter_in_type_field_never_executes`, `enter_on_execute_waits_for_arming`, `a_double_click_never_answers_the_next_dialog`, `any_close_answers_cancel`, `commit_prompt_closes_answer_roll_back`, `quit_with_an_open_commit_prompt_answers_roll_back`, `the_deadline_answers_roll_back`), `tx_ui.rs` 18 (`idle_toast_keep_never_rolls_back`, `badge_only_for_user_transactions`, …), `escalation.rs` 12 (`escape_never_escalates`, `second_press_opens_the_dialog_with_keep_waiting_focused`, `f6_enters_the_bar_and_leaves_it_without_answering`, `closing_the_console_answers_stop`), `cross_surface_m3b.rs` 16 incl. keyboard-only `calm_confirmation_by_keyboard`, `multi_unit_confirmation_by_keyboard`, `staging_footer_tab_order_is_visual`, `chooser_by_keyboard`. **Gaps (carried):** no keyboard test of the idle toast's buttons; the idle toast is tested by showing it, not through the session's real threshold (no `IdleTransaction` test hook) |
| E8 | M2 carries C3, C6–C11, C13 green; every M2b/M3a test green | **PASS** | C3 startup scripts (`carries_m3b.rs`: `the_script_runs_before_the_schema_restore`, `a_script_that_leaves_a_transaction_open_is_refused`, `a_reconnect_replays_the_time_zone_and_the_script`, …); C6 `manual_mysql_turns_autocommit_off_after_the_script_on_connect_and_reconnect`; C7 + R-M3-73 `liveness_m3b.rs` (`idle_transaction_fires_per_session`, `every_console_session_pings_its_own_connection`, `a_console_that_loses_its_connection_in_a_transaction_tells_it`); C8 `lost_state_m3b.rs` (`lost_state_from_effects_and_scan_fallback`); C9 `queue_over_256_is_queue_full`; C10 WarmSide (`a_short_request_does_not_keep_the_warm_connection_open`, …); C11 `startup_fallback_m3b.rs` (3), `time_zone_m3b.rs`; C13 `stale_cancel_with_old_seq_does_not_cancel_the_next_request` (`cancel.rs`), `choose_is_served_while_a_request_runs` (`m3b_interfaces.rs`) and the question-id checks. Every M2b/M3a test runs in the lines of section 2: all green. M3a tests changed on purpose (T24 ruling): the DEMO-7 CTE test (now refused at step 0b), two stop tests (WHERE), 3 sets → 3 tabs. The ledger's "ws 4161/0" at the T29 merged check is not reproduced: this gate counts 5,048/0 on the same tree plus `dc6d020` (T26's merged check had 5,030/0) |
| E9 | Incremental relex ≤ 2 ms p95 on 50k lines; B3 in the real app unchanged | **PASS** (relex); **B3 PASS, provisional** (concurrent load) | `sql-split` `relex_typing_edit_p95_under_2ms` here: **p95 0.60 ms** over 1,000 edits (T5: 0.61–0.71 ms). B3 at `dc6d020` (section 4.2): typing p95 **5.10 ms** (M3a 5.45), typing-with-results p95 **5.44 ms** (M3a 9.36), typing-long-line (Reduced) p95 5.57 ms (M3a 9.39) — all PASS, measured on a 240 Hz external display at 1× while two other agents' test builds ran |
| E10 | Shots: 0 violations; changed shots attributed; M1/M2 unchanged except the known | **PASS** (harness); **PO sign-off PENDING** | Section 5: 3,244 rendered at the identical tree, 0 violations, self-checks 9/9 (re-run here: `--test shots` 9/0); vs m3a 520 breached, 0 unattributed; vs m2b the known 24 + 12 rw capsules (`task-m3b-29-shot-diff.md`) |
| E11 | Gates green: clippy, full test set with perf lines, check-deps, l10n, tokens, three generators, `cargo deny`, `aws-lc-rs` absent | **PASS** (local); **CI PENDING (controller)** | Section 2: every line green here. CI: no run on M3b code since W0 (section 3) |
| E12 | First frame ≤ 500 ms, idle RSS ≤ 200 MB (`--smoke` ×3); release size delta of `sqlparser` recorded | **PASS** | `--smoke` ×3: 186 / 192 / 181 ms, 120 / 120 / 120 MiB (section 4.1). Release binary 62,786,160 bytes (M3a gate 45,562,336: **+16.4 MiB**); `sqlparser` ≈ **6.5 MiB** of code by symbol attribution (section 4.3) |

Summary: E1, E2, E4, E5, E6, E8 and E12 PASS. E3 and E11 pass locally and wait for CI on `m3b`. E7,
E9 and E10 pass their automated halves; the PO's demo, the shot sign-off and a quiet B3 re-run (if
wanted) remain. No criterion FAILS.

## 2. Step 2 gates (local, on `m3b-t30-gate`)

Each line ran on its own (`--no-fail-fast` where a line has many binaries), so one failure could not
hide another. The Docker set ran only through the lock wrapper.

| # | Command | Result |
|---|---|---|
| 1 | Step 1: `cargo clean -p <every workspace member>` with `CARGO_TARGET_DIR=<main>/target`, then with `target-core/merge` | removed 1,199 files (616 MiB) and 11,543 files (24.0 GiB); `target/docker` untouched |
| 2 | `cargo clippy --workspace --all-targets --locked -- -D warnings` | clean |
| 3 | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | clean |
| 4 | `cargo test --workspace --locked --no-fail-fast` (line 1) | **5,048 passed, 0 failed**, 224 ignored (Docker and perf), 365 result blocks, 575 s |
| 5 | `cargo test -p hacivat-ui-core --features test-support,scene-check` (line 2) | 84/0 |
| 6 | `cargo test -p hacivat-l10n --features pseudo` (line 3) | 151/0 |
| 7 | `cargo test -p hacivat-app --features scene-check,pseudo --no-fail-fast` (line 4) | **846/0**, 1 ignored |
| 8 | `cargo test -p hacivat-driver --features fake` (line 5) | 178/0 |
| 9 | `cargo test -p hacivat-testkit --features ssh` (line 6) | 26/0, 1 ignored |
| 10 | `cargo test -p hacivat-ui-editor --features scene-check` (line 7) | 194/0, 2 ignored (perf) |
| 11 | `cargo test -p hacivat-ui-grid --features test-support,scene-check` (line 8) | 170/0 |
| 12 | `cargo test --release -p hacivat-l10n --features pseudo` (line 9) | 154/0 |
| 13 | `cargo test -p xtask l10n -- --ignored repo_catalogs_are_clean` (CI `l10n-check`) | 1/0 |
| 14 | `cargo test -p hacivat-app --features shots,scene-check,pseudo --test shots` (self-checks; the render is T29's, section 5) | 9/0, 1 ignored (`render_all_surfaces`) |
| 15 | `HACIVAT_PERF=1 cargo test --release -p sql-lexer -p sql-split -p sql-oracle-pg -p hacivat-text -p hacivat-values -p hacivat-results -p hacivat-sql-editor -p hacivat-ui-editor -p hacivat-ui-grid -- --ignored` (perf line 1) | **24/0** (incl. `relex_typing_edit_p95_under_2ms` in `sql-lexer` and `sql-split`, `scan_params_10kb_unit_p95_under_1ms`) |
| 16 | `HACIVAT_PERF=1 cargo test --release -p sql-safety --test classify --test read_corpus -- --ignored` | 2/0 (interim 2 ms assertion; figures in section 4.3) |
| 17 | `HACIVAT_PERF=1 cargo test --release -p hacivat-exec --test classify_pool --test audit --test service_m3b -- --ignored` | 2/0 (`classify_10kb_unit_under_1ms` — interim 2 ms on the best —, `sync_chain_under_5us_per_event`; `service_m3b` has no ignored perf test) |
| 18 | `HACIVAT_PERF=1 cargo test --release -p hacivat-driver-sqlite --test typed -- --ignored` | 1/0 |
| 19 | `HACIVAT_PERF=1 cargo test --release -p hacivat-app --test results_model -- --ignored` | 1/0 |
| 20 | `cargo xtask check-deps` (sqlparser arm) | `check-deps: ok` |
| 21 | `cargo xtask l10n check` | 0 errors, 1 warning (L025 `hv-conn-lost-tx` unused; append-only, ruled → M9 cleanup) |
| 22 | `cargo xtask tokens --check` | ok (exit 0) |
| 23 | `cargo xtask gen-sql-tables --check` | fresh |
| 24 | `cargo xtask gen-fn-catalog --check` | `functions.digest` fresh (`xxh3-64:caeb289e6edf2388`) |
| 25 | `cargo xtask gen-pg-visitor --check` | `pg_generated.rs` fresh |
| 26 | `cargo xtask docker check-pins` | ok |
| 27 | `cargo deny --all-features check licenses bans sources` (cargo-deny 0.20.2) | `bans ok, licenses ok, sources ok` |
| 28 | `cargo tree -i aws-lc-rs --workspace --all-features` | "did not match any packages"; `Cargo.lock` has no `aws-lc` entry |
| 29 | `git status --porcelain --ignored -- third_party/pg_query` after every build | empty |
| 30 | `<main>/target/docker/run-docker-tests.sh --set full` | **192 passed, 0 failed**, 35 Docker test binaries with tests, 470 s (`docker_m3b_matrix` 14, `docker_prod_unlock` 9, `docker_m3b_bypass` 2, `docker_m2b_bypass` 4, `docker_m3a_readonly` 5, `docker_m3a_matrix` 10, `docker_m3b_confirm` 1, `docker_m3b_run` 1, `docker_mutation_fuzz` 1, `gen_fn_catalog` 1, `docker_typed_m3b` 5 + 5, `docker_types_m3b` 2 + 1, the M2a/M2b/M3a session and driver suites) |
| 31 | `cargo build --release -p hacivat-app --features bench --examples` (CI `bench`) | ok |
| 32 | `cargo test --release -p hacivat-app --features bench --lib bench` (CI `bench`) | 25/0, 2 ignored |
| 33 | `cargo build --release -p hacivat` | ok, 62,786,160 bytes |
| 34 | `cargo fmt --all -- --check` | only `spikes/spike-harman` (not ours); see 2.1 |

### 2.1 Formatting debt (recorded, partly fixed)

`cargo fmt --all -- --check` failed at `19096e7` on three places: `xtask/src/vf.rs`,
`spikes/spike-probes/src/main.rs` and `spikes/spike-harman/**`. The first two are ours and were
formatting-only: fixed with `cargo fmt -p xtask -p spike-probes` in `dc6d020` (`git diff -w` shows
only layout; `vf.rs` +214/−46 lines of wrapping, `spike-probes` one `#![allow(...)]` wrapped).
`spikes/spike-harman` is not ours and is left as it is (M9 cleanup). After `dc6d020`,
`cargo fmt --all -- --check` reports only `spikes/spike-harman`.

## 3. CI

`gh run list --branch m3b` returns **no run**. `ci.yml` triggers on `push` only for `main` and
`m1-foundations`, plus `pull_request`, the nightly `schedule` (on `main`) and `workflow_dispatch`; the
controller pushed `m3b` without a dispatch. So none of the jobs the plan names has run on the `m3b`
head (`19096e7`) or on this gate's commits:

| Job | On `m3b` head | Last evidence for M3b code |
|---|---|---|
| build-test ×3 | **missing** | 43805144615 on `m3b-t01a-seams` (`e31936a`, W0): green ×3 |
| turkey-test | **missing** | same run: green |
| docker-pr | **missing** (runs on `pull_request` and `push` to `main` only) | none; locally the full set is green (section 2) |
| docker-full | **missing** | none on CI; locally 192/0 |
| nightly-package | **missing** | 43805144615: macOS/Windows green, Linux AppImage red (known, also on `main`) |
| fuzz-sql | **missing** (`schedule`/`workflow_dispatch`; the nightly runs on `main`, which has no M3b code) | never on M3b code — **E3 PENDING** |
| sql-differential | **missing** | never on CI; T20 ran it locally (REL_17_2, 0 disagreements) |
| bench | **missing** | locally: build and `--lib bench` green (section 2), B3 in the real app (section 4.2) |

Triage: not a failure of the code; a trigger gap. **Controller action:** `gh workflow run ci --ref m3b`
(runs everything except `docker-pr`, including `fuzz-sql`, `sql-differential`, `docker-full`,
`nightly-package`, `bench`; signing is never part of CI), and `docker-pr` with the `m3b` → `main` PR.
The gate did not dispatch (the controller pushes).

For context only (M3a code, not this gate): the last `main` runs are 43847902437 (`schedule`: known reds
`nightly-package (ubuntu-24.04)` AppImage icon and `pseudo-screens (ubuntu-24.04)`) and 43848594948
(`push` of the PR #4 merge: `docker-pr` red in "Docker tests (pr set)", `switch (macos-15)` and
`launch-smoke (macos-15)` cancelled). The failed-job log was not retrievable by the gate; the PR run
before it (43840188193) was green and the ledger records one known CI-timing flake
(`mysql_release_kills_on_the_side_connection_and_reuses_within_1s`, 1.18 s vs 1 s). **Controller:**
re-run or triage 43848594948's `docker-pr` before the `m3b` → `main` PR.

## 4. Smoke and benches on this Mac (Step 3)

### 4.1 Smoke (E12)

`target-core/merge/release/hacivat --smoke`, three runs right after the gate lines finished, display
awake, no connection; the shipped binary (`cargo build --release -p hacivat` at `dc6d020`,
62,786,160 bytes). Concurrent load: other agents' test binaries (load average 8.4–9.3).

| Run | first-frame-ms | idle-rss-mib | libpg-query |
|---|---|---|---|
| 1 | 186 | 120 | `pg_query 18.1.0 (PostgreSQL 18.6) statements=1` |
| 2 | 192 | 120 | same |
| 3 | 181 | 120 | same |

Budgets: first frame ≤ 500 ms, idle RSS ≤ 200 MB. **PASS** (M3a: 243/194/197 ms, 121/114/114 MiB).
Idle RSS with one connection and an open console: `--smoke` never connects; it stays the demo's
post-check (M3a E11 connected reading, still PENDING-PO, section 11).

### 4.2 B3 in the real app (E9), with the PO at the Mac

The screen was locked during the gate lines. The controller then reported the PO at the Mac with the
display awake, and the benches ran right after the gate lines. **Provisional:** other agents' builds
and tests ran at the same time (`m3c-t35` shots test binaries at 85–98 % CPU each, a VM process,
load average 8–11 on 14 cores). Load can only slow these figures, so a pass under load is
conservative; a quiet re-run is the PO's choice.

- Build `dc6d020`, release, `cargo build --release -p hacivat-app --features bench --examples`.
- Fixtures: `cargo xtask gen-sql --lines 50000 --out target/fixtures/b3-50k-t0.sql` (1,994,194 bytes,
  tier Full) and `--long-line 100000` (`b3-50k.sql`, Reduced).
- Display: main = external 5120 × 1440 at **240 Hz, scale 1×** (the M3a figures were on a 120 Hz 1×
  external display); viewport 1440 × 900.

| Bench | p50 | p95 | max | Samples | Verdict | M3a (7e9209f) |
|---|---|---|---|---|---|---|
| `b3 --bench typing` (50k lines, Full) | 4.71 ms | **5.10 ms** | 13.28 ms | 2000/2000, 0 inactive keys | PASS | p95 5.45 ms |
| `b3 --bench typing-with-results` (1M × 50 grid open; complete in 2.54 s, store 1,700.6 MiB, RSS 2,106 MiB) | 5.00 ms | **5.44 ms** | 15.26 ms | 2000/2000 | PASS | p95 9.36 ms |
| `b3 --bench typing-long-line` (Reduced; F4 re-measure carry) | 2.13 ms | 5.57 ms | 12.14 ms | 2000/2000 | PASS | p95 9.39 ms |

JSON: `target/bench/b3-{typing,typing-with-results,typing-long-line}-app-macos-*.json` in the gate
worktree (not committed). **E9 B3: PASS (provisional)** — no regression; the lower p95 than M3a is at
least partly the 240 Hz display (shorter frame interval).

The M3a gate's own B3 and B1-real numbers are **no longer pending**: the controller ran them with
the PO on 2026-11-16 (ledger, `bench-po-20261116-0643/`; E2 and E3 PASS in `m3a.md`). M3b changes
neither the grid paint path nor the B1-real loader, so B1-real was not re-run here (T44 re-measures
with the M3c grid).

### 4.3 Release size and the classifier budget

- **Size:** the shipped binary is 62,786,160 bytes against 45,562,336 at the M3a gate:
  **+17,223,824 bytes (+16.4 MiB)**. `sqlparser`'s share, attributed by symbol address ranges in the
  release binary (`nm -n`, text symbols whose mangled name contains the crate, generic instances
  included): **≈ 6.5 MiB** of the ≈ 30.8 MiB of attributed code; `sql_safety` ≈ 0.6 MiB,
  `hacivat_exec` ≈ 0.6 MiB, `pg_query` ≈ 0.9 MiB. This is an estimate (rodata and the
  `sql-safety` data tables are not attributed); `cargo bloat` is not installed. M3b has no size budget;
  M3a's was "≤ +10 MiB" for libpg_query. **PO item:** the total M3b delta is above what M3a allowed one
  dependency; whether to slim `sqlparser` (features, or its replacement in M5) is the PO's call.
- **Classifier time per 10 KB** (perf tests re-run with `--nocapture` after the gate lines; release):

  | Path | PG | MySQL | MariaDB | SQLite |
  |---|---|---|---|---|
  | `sql-safety` classify, best / p95 | 1.62 / 3.04 ms | 1.49 / 2.18 ms | 1.49 / 1.75 ms | 1.29 / 1.60 ms |
  | through `ClassifyPool`, best / p95 | 1.62 / 2.27 ms | 1.52 / 1.81 ms | — | 1.31 / 1.58 ms |
  | with `\` and `"`, modes unknown, best | 1.72 ms | **3.30 ms** | — | — |

  Spec budget (R-M3-53): ≤ 1 ms per 10 KB. **Not met**; interim assertion 2 ms on the best holds.
  Task 10 left the decision to this gate: **gate recommendation — accept the interim 2 ms for M3b**
  (classification runs on the pool, never on the UI thread; a 10 KB unit is rare; the unknown-modes
  MySQL path is multi-reading by design and ends once T18/T24 fill known modes, which they do on
  every connected console), and carry the 1 ms budget to M5's own parser with T44 re-measuring.
  PO to confirm.

## 5. Shots (E10)

Not re-rendered: Task 29 rendered the shots on the identical tree (`82fd723`, merged into `m3b` as
`19096e7` with an identical tree) and stored them as the SDD folder's `baseline-m3b/` (3,244 PNGs,
1.2 GB). Evidence: `task-m3b-29-shot-diff.md`, `task-m3b-29-report.md`, `task-m3b-29-review.md`.

- **Render:** 3,244 PNGs (1,020 M1/M2, 68 × 16 M3a, 71 × 16 M3b), **0 violations**, self-checks 9/9.
  The checks now include the T29 trust group (a truncated or clipped production name fails), the
  popup-menu card check and the per-state markers.
- **vs `baseline-m3a`** (2,108): **520 breached, 0 unattributed**, 1,136 new. By cause:

  | Cause | PNGs | What changed |
  |---|---|---|
  | T22 mode chip | 168 | the toolbar mode chip on every console, `marks-*`, `preview-*`, `context-*` state |
  | T22 badge | 128 | `prompt-close`, `-failed`, `-failed-quit`, `-quit`; `status-everything`, `status-tx-failed`, `status-tx-open`; `toolbar-waiting` |
  | T22 wording | 16 | `live-lost-tx` ("may have had an open transaction") |
  | T23 cancelling | 12 | `toolbar-cancelling` (Stop enabled while cancelling, NOTES ruling 29) |
  | T24 chooser | 32 | `chooser-candidates`, `chooser-hover` (the Customize… row) |
  | T24 uncertain | 16 | `chooser-uncertain` (Run as one) |
  | T25 count | 72 | Σ Count rows in result, strip, copy and grid states |
  | T26 production | 48 | `console-production`, `marks-denied`, `toolbar-production-waiting` (the read-only notice is gone) |
  | T29 result | 16 | `status-production-result` (the fixture now connects) |
  | T29 capsule | 12 | `live-ssh-rw` 8, `live-lost-reconnected` 4 (the production name no longer truncates) |

- **vs `baseline-m2b`** (1,012): **36 breached**: the 8 known `menu-app-*`, `live-lost-tx` 16 (T22
  wording) and the 12 read-write capsules at 960 from T29's I2 fit step. **PO item:** M2b baseline
  shots changed.
- E10's plan wording ("every M3a shot unchanged except Task 26's listed production shots") predates
  Tasks 21–25, which changed shots on purpose with their reviews; the attribution file is the record.
  The PO signs the changed shots off (section 7.2), then `baseline-m3b/` becomes the M3c baseline.

## 6. Rulings for PO confirmation

### 6.1 R-M3-51 to R-M3-84 (plan "New rulings")

The PO approved the batched M3 decisions on 2026-11-16 (ledger: R-M3-29, -23, -24, -27, P16, -37,
-55a/b/c, -73, Q1, Q14, Q15, chords, Tab leaves the grid, Run as one untyped, the 64 MiB re-run,
session-only binds, "failed", RTL v1.x). The rows below say which are covered by that approval and
which still need a word. **Bold** = the ones the plan asks the PO to confirm explicitly.

| Ruling | Gist | Status |
|---|---|---|
| **R-M3-51** | Split into M3b "Safe" (W0–W7, gate T30) and M3c "Complete" (W8–W13, gate T45) | Controller accepted 2026-11-16; **PO to confirm** (plan Q2). M3c already cut (section 10) |
| **R-M3-52** | No interim classifier or gate wired into Run; production unlocks in one task (T26) after T18, T19, T21, T24, T28 with Docker proofs | Held: the app calls the gate from T24; production writable only from T26 (`1894b7d`). **PO to confirm** |
| R-M3-53 | Classifier on `ClassifyPool` (2 × 16 MiB threads); libpg_query on its serial thread; the UI never classifies | Held. The 1 ms / 10 KB budget is **not met** (PG 1.58 ms, MySQL up to 3.4 ms with unknown modes); interim assertion 2 ms; see E3 note |
| R-M3-54 | `sqlparser = 0.64.0`, `sql-safety` only | Held (`check-deps` sqlparser arm) |
| **R-M3-55** | The confirmation policy table | PO-approved (55a/b/c, Q1, Q15). **Changed since, PO to confirm:** D-043 (MySQL-family production DML confirmed before send), no "don't ask again" and no Execute all on production, `BEGIN` on production asks, `TRUNCATE`/`DROP TABLE` confirm without a typed name |
| R-M3-56 | Human read-only profile `ReadOnlyPlusExplain`; Unknown denies; harmless-SET list | Approved (Q13, Q14) |
| R-M3-57 | MySQL `sql_mode` pinned on read-only sessions | Held; read at connect (T18 carry overrides P-M3b-33's lazy read) |
| R-M3-58 | MariaDB stays "read-only (guarded)" | Held; T18 recorded that `CALL` as owner without the classifier committed a row (PO list) |
| **R-M3-59** | The PG oracle stays out of the caret path; preview never consults it | Held (preview_truth). **PO to confirm** |
| R-M3-60 | Run as one: uncertain boundary or As-one selection; untyped PG columns | Approved (Q6). Production denies Run as one with an ambiguous boundary (T15 D3, stricter): PO list |
| **R-M3-61** | Error policy Ask; Roll back and continue in a failed PG tx; any close answers Stop | Held. **PO to confirm**; open sub-question: continue after an explicit Undo? (default: the error policy applies) |
| R-M3-62 | History in `hacivat-exec::history`, SQLite + FTS5, 0600, 10k / 90 days, never an audit trail | Held |
| R-M3-63 | Auditor chain wraps the observer; async sinks `try_send` 10,000 | Held; no hash chain (chain sinks are M8, T16 ruling m12) |
| R-M3-64 | Multi-cursor IME mirroring with primary fallback | M3c (T31, in review) |
| R-M3-65 | Incremental relex in the document worker, ≤ 2 ms | Held: p95 0.61–0.71 ms (E9) |
| R-M3-66 | ≤ 100 result sets per unit | Held (E5) |
| R-M3-67 | Type matrix | Held (T7) |
| R-M3-68 | Eviction of background results; fetch-all confirmation | Held (T8, T25); Fetch-all trigger rule is a PO item |
| **R-M3-69** | Grid AccessKit subtree; **Tab leaves the grid from any cell** | Tab rule approved (Q5); the subtree is M3c T40. **PO to confirm** the subtree approach after the VoiceOver check |
| R-M3-70 | Sort by DB collation, never the UI locale | M3c (T36) |
| **R-M3-71** | RTL in the grid stays v1.x | Approved (Q11). Listed for the record |
| R-M3-72 | Multi-window issues out of M3b/M3c | Deferred (no second window before M9) |
| R-M3-73 | Per-session liveness; non-default console deactivate breaks only its session | Approved; confirmed in code (T23) |
| R-M3-74 | `ABORT` = ROLLBACK on PG in a failed tx | Held (T11) |
| R-M3-75 | `ToolbarItem::narrow` | M3c (T41) |
| R-M3-76 | Count rows on a pooled session, classified again, Read only | Held (T16, T25) |
| R-M3-77 | Notices and warnings | Approved (Q8); held (T13, T16) |
| **R-M3-78** | Pending-read-only only for the read-only toggle; a retag to production applies confirmations from the next Run | Held and **tightened** in T26: the live tag is re-read at send, idle pooled sessions close on retag, startup scripts are checked on replay. **PO to confirm** |
| R-M3-79 | Bind values; Turkish `1.234` flagged; session-only memory | Approved (Q9); remembered values pre-fill only (T15 D8) |
| R-M3-80 | Fuzzing in CI only (`fuzz-sql` nightly) | Held; no nightly run on M3b code yet (section 3) |
| R-M3-81 | Baselines: zero tolerance vs m3a and m2b; changed shots listed | Superseded in practice by the T29 attribution (section 5); PO signs off |
| R-M3-82 | Large values: re-run with a 64 MiB limit | Approved; UI in M3c (T38) |
| **R-M3-83** | Read-only classifier hook injected (`StatementClassifier`), not linked | Held (T18: boot installs it; a boot test asserts it). **PO to confirm** |
| **R-M3-84** | Confirmations asked before the run is sent; only confirm-in-transaction mid-run | Held, with D-043 moving MySQL-family production DML entirely to before send. **PO to confirm** |

### 6.2 Controller rulings taken nonstop during M3b (for PO confirmation)

Taken while the PO was away (ledger `progress.md`, "## M3b"); the safer reading was chosen each time.

| Ruling | Task | Gist |
|---|---|---|
| **D-043** | T26 r1 | On production, MySQL/MariaDB/TiDB non-Read DML is **confirmed before it is sent** (no run-then-Keep/Undo; non-transactional engines make Keep/Undo meaningless). Warnings 1196/1751/1752 after a rollback → `exec-confirm-not-undone` ("Not undone: the table doesn't support transactions…") and the unit fails |
| T26 r2 | T26 | On production "don't ask again" is **never offered** and suppressions recorded anywhere never apply; **Execute all is hidden** on production (implementer's choice between "byte-identical units only" and "hidden") |
| T26 r2 | T26 | Fail-closed WHERE: a WHERE with no column of the target tables, a self-comparison, `X OR NOT X`, `IS NULL OR IS NOT NULL` counts as always true → typed name |
| T26 r1 | T26 | Typed name = the table that changes (joined/comma UPDATE, aliases, `WITH` modifiers; `LOW_PRIORITY`/`QUICK`/`IGNORE` no longer read as the table); two or more targets → deny on production |
| T26 r1 | T26 | Production confirm for `SET session_replication_role`, `sql_log_bin=0`, `FOREIGN_KEY_CHECKS=0`, `unique_checks=0`, `NOTIFY`, `sql_safe_updates`, `gtid_next`, `pseudo_replica_mode`/`pseudo_slave_mode`, `synchronous_commit` |
| T26 MUST 3 / T12 | T26 | On production a startup script runs only if the classifier rates it harmless Session/Read; otherwise the session refuses (`config.invalid`) |
| T24 I5 / R1-C1 | T24 | A literal `USE <db>` (MySQL) or `SET search_path` literal list (PG) sets later units' context; when such a switch is not the last unit the run is forced to StopOnError; a context generation in `RunSpec` refuses a send after the context moved |
| T24 I1 | T24 | The classifier context comes from the session (per session, stale-marked on context-changing statements; 10 s → unknown modes); a moved context sends nothing (`exec-run-context-changed`) |
| T19 C1/C2 | T19 | Confirm form from live autocommit (an already-committed change is never offered Keep/Undo); the deadline stops the run; an explicit Undo ends the unit not-succeeded so the error policy applies |
| T19 m4 | T19 | Smart mode skips statements that cannot run in a transaction (PG `VACUUM`, `CREATE DATABASE`, `CONCURRENTLY`, …; MySQL `SET TRANSACTION`, `LOCK/UNLOCK TABLES`) |
| T15 I5 / r2 | T15 | MySQL/MariaDB/TiDB implicit-commit list beyond DDL, a separate `SessionEffects::COMMITS_FIRST` bit; the T19 backstop keys on it |
| T15 D3, I1, I2, D8 | T15 | Production denies Run as one with an ambiguous boundary; pre-decided reports deny on production/read-only and get one calm Confirm elsewhere if they contain a write keyword; production `EXPLAIN ANALYZE` with an unknown inner confirms; remembered parameter values pre-fill and never skip the prompt |
| T14 C2 / r2–r3 | T14 | Classify under every lexing-mode reading when the modes are unknown and allow only if all allow; never drop a reading when the text holds an executable comment |
| T10 I3 / RR-I2 | T10 | Information-disclosure statements (`SHOW PROCESSLIST`, `ENGINE STATUS`, `REPLICA STATUS`, `GRANTS FOR` another user, MariaDB `SHOW EXPLAIN FOR`, `information_schema.PROCESSLIST`, `performance_schema.threads`, `sys.processlist`, …) are Unknown → denied on read-only; PG `pg_stat_activity`/`pg_stat_statements` likewise (T9) |
| T3 P-M3b-T3-1 | T3 | `SET search_path` is harmless on read-only only when `pg_catalog` is absent or first |
| T28 r1 | T28 | MariaDB `ANALYZE <dml>` is classified as the bare DML (it executes); production asks a typed name instead of denying |
| F5 | m3a | `CALL`/`DO` and `ANALYZE` refused on read-only consoles before sending |
| T29 r1 | T29 | The production name in the title bar never truncates (the rw segment yields first); the trust check fails on a cut name |
| T37 Step 6 | controller | The M3a baselines are not committed to git (2,108 PNGs ≈ 775 MB); they stay in the SDD folder until M9 (curated subset or LFS). Same for `baseline-m3b` |
| Account rule | controller | Stop dispatching at > 75 % weekly usage; nonstop between waves |

The full pre-flight set P-M3b-1..38 (with the controller's Q1 live-state variant and the Q19 task
split) is in the plan's "Pre-flight rulings" and in `amend-m3b-rulings.md`.

## 7. PO list (deduplicated, grouped)

### 7.1 Production safety behaviours to confirm

1. **D-043:** production MySQL-family changes ask **before** sending; PostgreSQL and SQLite run first and
   ask Commit/Roll back (own transaction, Auto idle) or Keep/Undo (savepoint, any open transaction).
2. **No "don't ask again" and no Execute all on production** (any class); off production both stay.
3. **Production starts in Manual** (D-036), so a production UPDATE is a savepoint Keep/Undo inside the
   session's open transaction, and the user commits separately (plan Q3/Q4; part of the approved
   R-M3-55a/b/c — confirm the lived experience in demo steps 2 and 4).
4. **`BEGIN` on production asks** (Tx class → Confirm).
5. **`TRUNCATE` and `DROP TABLE` confirm without a typed name**, while `DELETE FROM t` needs one
   (R-M3-55 as written; T26/T28 asked whether `TRUNCATE` should ask more).
6. Production denies **Run as one with an ambiguous boundary** (stricter than R-M3-60's confirm).
7. Production **`EXPLAIN ANALYZE`** of a non-Read or unknown inner confirms first; on MySQL-family
   engines the D-062 wrap cannot undo non-transactional writes. **MariaDB `ANALYZE <dml>` executes**
   (MySQL `EXPLAIN ANALYZE` is rolled back); production asks a typed name.
8. A multi-unit run with a context change (`USE`, `SET search_path`) is refused on read-only and
   production consoles unless the switch is the last unit; a busy shared session can make Run wait up
   to 10 s for its context.
9. On production a **startup script** runs only if harmless (Session/Read); otherwise the connection
   is refused with `config.invalid`.
10. A retag to production applies confirmations from the next Run; idle pooled sessions close.
11. **Continue after an explicit Undo?** Default: the error policy applies (T19).
12. Remaining production hardening (minor; owner "PO/M9"): `SET skip_replication`, `wsrep_on=OFF`,
    `sql_log_off`, `binlog_format`, PG `U&"…"` setting names do not confirm; `MERGE … ON true … UPDATE`
    gets no typed name; a cosmetic "suppressed" log line can appear on production
    (`orchestrator.rs:1761`); the internal name `confirm-suppress-prod` is historical.

### 7.2 Changed shots, by cause (sign-off)

- vs `baseline-m3a`: 520 PNGs in ten causes (section 5): **every console state** changed (T22 chip,
  some under the visible threshold), T22 badge 128, `live-lost-tx` wording 16, `toolbar-cancelling` 12,
  chooser 48, Count rows 72, production 48, `status-production-result` 16, rw capsules 12.
- vs `baseline-m2b`: 36 (8 known `menu-app-*`, 16 `live-lost-tx`, **12 rw capsules at 960**).
- Layout items: at 960 tr-XT the production notice collapses into More and the
  `toolbar-production-waiting` sentence takes 3 rows (→ T41); `conn-dialog-sessions` sub-label
  ellipsizes in the pseudo locales (→ T41); the Execute-all scope line says "production" on a staging
  dialog (copy).

### 7.3 Turkish copy

Every new TR string carries `# Review: PO` (blocks `m3b-t04-safety`, `-t15-gate`, `-t16-run`,
`-t18-safe06`, `-t19-confirm-tx`, `-t21-confirm`, `-t22-tx-ui`, `-t23-escalation`, `-t24-exec-ui`,
`-t25-results`, `-t26-prod-unlock`, `-t27-history`, `-t29-shots`). Points to look at first:

- **Terminology:** `exec-result-set-limit` says "Deyim … sonuç kümesi" while every other new string
  says "ifade" for statement (found by the gate).
- "Commit et" kept as an English verb; "İşlem" for transaction; "Koru / İfadeyi geri al" for
  Keep/Undo; "Tek parça çalıştır" for Run as one; "Beklemeyi sürdür", "Sunucuda sonlandır",
  "Bağlantıyı kes".
- T27: "Çalıştırılmadı", "Sabitlemeyi kaldır", "Silinmiş bağlantı", "Geçmiş şu anda okunamıyor.";
  T29 `hv-history-selection-a11y`; T22 `hv-tx-idle-waits`, `hv-conn-zone-refused`,
  `hv-conn-single-session-reconnect`; T23 `hv-cancel-*-hacivat`; T21 `hv-confirm-type-untypeable`.
- M3c mocks (T2b): "Türkçe esnek", "Devrik", "Biçimli/Ham", "harmanlama", "Satırları süz"; fold chords.

### 7.4 Classifier and read-only behaviours (minor SETs and denials)

- Read-only `SET` only for the harmless list (approved Q14); PG `search_path` only with `pg_catalog`
  first or absent.
- Denied on read-only: `current_setting()`, `version()`, PG `E'…'`/`U&'…'` with unknown
  `standard_conforming_strings`, user-type coercions, `json(b)_populate/to_record` (the 5 ruled PG
  corpus denials); `SELECT @v := 1`; MariaDB `sql_mode=ORACLE`; the information-disclosure set (6.2);
  `CALL`/`DO`, `ANALYZE`; `FETCH`/`MOVE` classified Session.
- A statement over 128 KiB with any `/*!` is TooComplex; the 256 KiB per-request readings budget can
  make a Run-as-one schema dump TooComplex.
- psql `:"name"` is refused, not bound. MariaDB `max_execution_time` SET refused (no such variable).
- CockroachDB `SHOW JOBS/QUERIES` read as Read (no parity with `pg_stat_activity`).
- MariaDB: `CALL` as the owner without the classifier committed a row (run B); with the classifier it
  is denied at step 0b.

### 7.5 History and secrets

- Secret forms not yet masked: `TOKEN`, `*_KEY`, `PRAGMA key`, `SET @pw`; URIs with unencoded `/#?` or
  spaces; `COPY PROGRAM 'curl -u …'`; truncated or adjacent-string error scrub (13 documented gaps of
  87 probe rows).
- Should enabling "do not record" purge existing entries? Default: no; "Clear history for this
  connection" needs a T17 delete API (not built).

### 7.6 UI details

- Escalation: no PID in the terminate line; no Stop tooltip naming the dialog.
- Fetch-all trigger rule (exact count past the 256 MiB cap); Count tooltip "separate session".
- The two duration exceptions (`SpanMs` whole seconds under a minute; the running clock).
- The mode chip stays enabled during an open transaction (NOTES ruling 10).

### 7.7 Budgets

- Classifier 1 ms / 10 KB not met (1.3–1.6 ms best, 3.3 ms MySQL with unknown modes): accept the
  interim 2 ms for M3b and carry to M5? (gate: yes; section 4.3).
- Release binary +16.4 MiB over M3a, `sqlparser` ≈ 6.5 MiB of it (section 4.3).
- PG split differential: 1000 ‰ of comparable statements, 988 ‰ of all (E3): accept the
  comparable-statements reading?

## 8. Open questions (plan list)

| Q | Topic | Status |
|---|---|---|
| Q1 | Unsafe-WHERE guard off production | Approved (on, with the amended M2a line) |
| Q2 | The split | Controller accepted; PO to confirm (R-M3-51) |
| Q3 | Production default Manual | Open in practice: confirm after demo step 4 |
| Q4 | 5-minute deadline on production only; savepoint form undoes the statement only | Covered by R-M3-55 approval; confirm in demo step 2 |
| Q5 | Tab leaves the grid from any cell | Approved |
| Q6 | Run as one untyped on PG | Approved |
| Q7 | Re-run with 64 MiB | Approved |
| Q8 | MySQL `SHOW WARNINGS` | Approved (not on read-only) |
| Q9 | Binds remembered for the app session only | Approved |
| Q10 | New chords and shadowing | Approved |
| Q11 | RTL v1.x | Approved |
| Q12 | DDL logs "Completed" | Approved |
| Q13 | Function allowlist in the profile TOML only | Approved (file only) |
| Q14 | Read-only harmless SET list | Approved |
| Q15 | Confirm-statement failure restores the user's tx | Approved |
| Q16 | Carried from M3a | TR strings: section 7.3; `menu-app-*`: still in the m2b diff; Windows/Linux IME hosts: open; nested a11y fork patch: M3c T40 after VoiceOver |

## 9. Carried items

### 9.1 Wave carry files

- **`carries-w4.md`** (T18–T22): done. T18: real `ClassifyCtx` (modes, `search_path`, version), classify
  on the pool only, `sql_mode` in the connect query, MariaDB ORACLE denied, refresh after FETCH/EXECUTE,
  `COPY … TO STDOUT` denied, one denial id. T19: backstop on `COMMITS_FIRST`, question ids. T20: all but
  the 1 ms budget (section 4.3). T21: all; the gutter FTL ids moved to T24 → **T34**. T22: all. **Not
  done:** a driver panic at request start breaks the session without `Lost` (optional; T18 → T23 →
  unowned) → proposed **M9 hardening**.
- **`carries-w5.md`** (T23–T25, T28): done, except the T25 result-tab menu keyboard opener → **M3c**
  (unassigned) and tokio `test-util` as an app dev-dependency (T23 m8) → **M9** (manifest).
- **`carries-w6-t26.md`** (T26): all four MUSTs done with tests and Docker legs (joined/comma UPDATE
  targets, no-generation refusal incl. Count rows, production startup scripts, Docker context read on
  read-write pg14–18 and MySQL `DATABASE()` NULL). Redshift/Cockroach context read inside an open
  transaction: not in the matrix; T24's "no read inside an open transaction" rule stands (ADR).

### 9.2 Open items and their owners

| Item | Source | Owner |
|---|---|---|
| A direct property preview → param rewrite → sent bytes (E6) | gate | M3c (T44 or the T45 gate), test only |
| Keyboard test of the idle toast's buttons; an `IdleTransaction` session test hook (E7) | T29 review I5, T22 dev. 5 | M3c (unassigned; T41 suggested) |
| Shared-session chip/badge/segment agreement test; T27 history-filter keyboard probe | T29 | M3c (unassigned) |
| "as one · n statements" band label, `hv-editor-mark-rolled-back`, gutter hourglass | T24, T29 | **T34** |
| `conn-dialog-sessions` pseudo overflow; `toolbar-production-waiting` 3 rows at 960 tr-XT; narrow toolbar | T29, T22, T26 | **T41** |
| Disabled-ink token, active-row tint (T2b system-wide rulings) | T2b | **T41** |
| hui: submenu arrow keys, `MenuItemSpec` icons, action-less toolbar items; a visible scroll cue for long in-window menus | T22, F5 | hui owner, M3c design (unassigned) |
| Grid AccessKit subtree; VoiceOver (M3a E14 still failing, non-gating) | F5 DEMO-12, R-M3-69 | **T40** / M9 |
| B3 and B1-real re-run with the M3c editor and grid; relex long-line/open-string figures next to R-M3-9 (T5: burst in an open string p95 2.2–2.7 ms, 100k-char line p95 5.6–8.3 ms) | T5, plan | **T44** |
| Classifier 1 ms / 10 KB budget | T10, T14, T20 | M5 (own parser); T44 re-measures |
| `StreamSplitter` holds a whole MySQL script without blocks (**MUST before "Run as script"**) | T5 | **M4b** |
| `DELIMITER`/`source`/`.cmd` arguments; BOM after a client command | T5 | M3c/M5 lexer |
| Oracle in the preview | R-M3-59 | M5 |
| `EnvStep::TimeZone` refresh after `SET time_zone` (stale metadata); session-state tracking (`CLIENT_SESSION_TRACK` would need a `mysql_async` patch) | T7 | settings model / later |
| Refused-zone flag persists until the setting changes | T22 | minor, M9 |
| Audit chain sinks | T16 m12 | M8 |
| Production hardening minors (section 7.1 item 12); driver panic → `Lost` | T26, T18 | M9 (PO may promote) |
| `hv-conn-lost-tx` unused FTL id; `spikes/spike-harman` rustfmt | T22, ledger | M9 cleanup |
| Windows IME pass; async Linux primary read (fork) | F4 M9, T31 I2 | M9 |
| Baselines in git (curated subset or LFS) | controller | M9 packaging |
| "Clear history for this connection" (needs a T17 delete API) | T27 | PO decision, then M3c/M4 |
| Multi-window issues | R-M3-72 | first milestone with a second window |

## 10. M3c pointer

- Branch `m3c` cut from `m3b` at `1894b7d` (2026-11-17), before T29; T29 and this gate reach it when
  `m3b` is merged forward. `carries-w8.md` binds W8.
- W8 in flight: **T31** (multi-cursor; DONE_WITH_CONCERNS `2b46802`, review "Needs fixes" 0 C / 4 I /
  11 M, fix round 1 sent) and **T35** (grid model) on the 2-slot gpui schedule (disk < 60 GiB); T32
  follows.
- Items this gate hands to M3c are in section 9; the M3c gate is Task 45.

## 11. Commands for the PO

1. **The demo:** `docs/benchmarks/po-demo-m3b.md` (13 steps incl. 3b; Docker up; release app
   `cargo run --release -p hacivat -- --lang en`). Every step is pre-verified by named tests; the screen,
   keyboard, focus and the TR copy are yours.
2. **Changed shots sign-off** (section 7.2): open `baseline-m3b/` in the SDD folder next to
   `baseline-m3a/` and `baseline-m2b/`, guided by `task-m3b-29-shot-diff.md`. After sign-off,
   `baseline-m3b/` is the M3c baseline (not committed to git, controller ruling).
3. **Rulings and PO list:** sections 6, 7 and 8 (Q3 especially), plus the classifier-budget
   recommendation (4.3) and the size delta (4.3).
4. **Optional quiet B3 re-run** (only the PO's Mac gives the real figure; close other builds first):

   ```sh
   cd <the m3b checkout>
   cargo xtask gen-sql --lines 50000 --out target/fixtures/b3-50k-t0.sql
   cargo xtask gen-sql --lines 50000 --long-line 100000 --out target/fixtures/b3-50k.sql
   cargo build --release -p hacivat-app --features bench --examples
   E=$CARGO_TARGET_DIR/release/examples      # or target/release/examples
   $E/b3 --file target/fixtures/b3-50k-t0.sql --bench typing
   $E/b3 --file target/fixtures/b3-50k-t0.sql --bench typing-with-results
   $E/b3 --file target/fixtures/b3-50k.sql --bench typing-long-line
   ```

   Keep the bench window frontmost and do not touch the mouse or keyboard; read each `RESULT:` line.
5. **Connected idle RSS** (M3a E11, still open): connect `pg17`, open a console, wait 30 s idle, then
   `ps -o rss= -p $(pgrep -x hacivat)` (KiB).
6. **Controller (not PO):** `gh workflow run ci --ref m3b`; open the `m3b` → `main` PR for `docker-pr`;
   triage `main` run 43848594948's `docker-pr`.
