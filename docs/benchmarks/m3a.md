# M3a gate record (draft)

Milestone M3a, "Run a query": the walking skeleton. libpg_query vendored and proven on three
operating systems; the SQL lexer, splitter, `SqlDocument` and the D-011 caret/selection chooser;
the single-cursor console editor (IME, clipboard, lexical highlight, undo, gutter marks, the Run
preview band, a11y baseline, large-file tiers 1–2, autosave); typed execution on PostgreSQL,
MySQL/MariaDB and SQLite with cursors, stop-fetching and cancel; the result store with caps and idle
bounds; the typed read-only guard; the grid with fast-scroll and every M3a cell type; result tabs,
raw-TSV copy, the results tile, the cursor strip and the output log; consoles bound to
connections, the production hairline and read-only production consoles.

This is the **draft** gate record, written by the gate agent (Task 37). Everything an agent can do
is done and recorded here. What only the PO can do is marked **PENDING-PO** and is prepared in
`docs/benchmarks/po-demo-m3a.md` (in Turkish) and in section 9. Baselines are **not** copied: the PO
signs them off first (Step 6).

- Gate branch: `m3a-t37-gate`, based on `m3a` at `7e9209f` (every M3a task and follow-ups F1–F3
  merged). Gate commits: `2ecec22` (a test fix, section 3.2) and the docs commit that writes this
  record.
- Machine: the PO's Mac (macOS, Apple silicon, 14 cores, 48 GB), with the 19 containers of
  `hacivat-m2a` up for the whole run. The display was locked for most of the run; the benches that
  need a frontmost window are the controller's, with the PO (section 4.2). No hostnames, accounts or
  personal names are recorded here.

**Verdict: not yet a PASS.** Every local gate line is green, the Docker full set is green
locally and on CI, and the bench harness now runs to the end under xvfb. One CI defect was found and
fixed here: a Task 35 test looked a menu label up as one drawn run, which fails wherever access keys
split it (Linux, Windows); it had been masked in the previous CI run (section 3.2). The fix
`2ecec22` is not pushed yet, so CI `build-test` (Linux, Windows) and `turkey-test` must be re-run on
it (pushed with the gate merge; CI run 43800394929). E2 and E3 PASS on the controller's panel run with
the PO; E7, E8, E14 and the connected half of E11 wait for the PO.

## 1. Exit criteria E1 to E14

| # | Criterion | Status | Evidence |
|---|---|---|---|
| E1 | Typed SELECTs on PostgreSQL, MySQL/MariaDB and SQLite with the rows browsed; DML and DDL on non-production consoles | **PASS** (automated); demo steps 2–6 PENDING-PO | Docker full set 147/0 locally and on CI `docker-full` (sections 2, 3): `typed_golden_tables_per_engine_and_version` (PG 14–18, MySQL 8.0/8.4, MariaDB 10.6/10.11/11.4, SQLite), `dml_and_ddl_report_update_counts_on_a_dev_console` (P27), `select_streams_suspends_fetches_more_and_closes`, `turkish_collation_text_round_trips_on_pg17_tr` (`hacivat-exec/tests/docker_m3a_matrix.rs`); plain `dml_reports_rows_affected_and_ddl_completed` (`hacivat-exec/tests/service.rs`) |
| E2 | B1-real: 1M × 50 from PostgreSQL in `tr-TR`, fling ≥ 55 fps on macOS, no blank frame > 100 ms; paint p99, load time, store bytes and RSS recorded | **PASS** (controller run with the PO, display awake) | Release `b1_real` at `a824840`/`7e9209f`, pg17, `tr-TR`, 1M × 50, Fetch all. **1440 × 900 on the built-in panel (2×, 120 Hz):** 2,400 frames, mean 120.0 fps, max gap 9.6 ms, 0 gaps > 100 ms; grid paint p50 1.27 / p95 1.99 / **p99 2.46 ms**; load 10.80 s, first rows 247 ms, store 836.1 MiB, RSS 119 → 1,086 MiB. **2560 × 1440 (viewport 2560 × 1344) on a 1× 120 Hz external display:** 2,400 frames, mean 120.0 fps, max gap 11.3 ms, 0 gaps > 100 ms; grid paint p50 2.25 / p95 2.93 / **p99 3.33 ms**; load 10.80 s, first rows 230 ms, store 836.1 MiB, RSS 119 → 1,087 MiB. A first 1440 × 900 attempt ran with the window inactive (throttled to 30 fps) and is discarded as invalid. Raw JSON and logs kept with the gate record (section 4.2) |
| E3 | B3 in the real app: typing ≤ 16 ms p95 on the 50k-line file, on the 100k-character line and with a 1M-row grid open; open 2 MB first paint ≤ 150 ms, full analysis ≤ 600 ms | **PASS** (controller run with the PO; component budgets also PASS) | Release `b3` at `7e9209f` on a 120 Hz display: typing p95 **5.45 ms** (50k lines, tier Full); typing with a 1M-row grid open p95 **9.36 ms**; typing on the 100k-character line (Reduced) p95 **9.39 ms**; open 2 MB first paint p50 16.7 / max **22.9 ms** (≤ 150), full analysis p50 44.9 / max **52.1 ms** (≤ 600); IME bounds p95 0.008 ms (≤ 1); scroll page-down p95 7.28 ms (ungraded). Note: run on the 1× external 120 Hz display; B1-real at 1440 × 900 confirms the 2× panel path. The component budgets passed in release through the three perf lines (P9, P27): 21 perf tests, among them `change_p95_under_half_a_millisecond_on_50k_lines`, `preview_p95_within_the_resolve_budget_on_50k_lines`, `selection_preview_p95_within_the_budget_on_2_mb`, `typing_in_reduced_mode_stays_within_budget`, `bounds_for_range_at_line_49000_under_1ms`, `one_megabyte_lexes_within_10_ms_release`, `one_megabyte_splits_within_30_ms_release`, `resolve_on_50k_lines_within_2_ms_release`, `first_chunk_of_a_million_rows_within_20_ms_release`, `cell_through_the_adapter_hit_under_200_ns` |
| E4 | D-011 goldens CARET/SEL/CHOOSE with the ID-coverage meta-test; the preview/execution agreement property | **PASS** | `caret_sel_choose_goldens` (CARET 30, SEL 8, CHOOSE 7) and `every_corpus_id_is_present_or_skipped_with_a_reason` (`sql-split/tests/d011_goldens.rs`); `preview_equals_execution_for_exact_items`, `exact_statement_spans_are_what_execution_runs`, `preview_follows_the_remembered_kind`, `statements_are_exact_only_where_run_sends_them`, `stale_snapshot_never_yields_a_preview` (`hacivat-sql-editor/tests/preview_truth.rs`); `preview_and_run_agree_on_generated_scripts` (`hacivat-app/tests/exec_ui.rs`). All in the workspace line |
| E5 | Docker matrix for typed execution on all three engines: `docker-pr` green on the PR, `docker-full` green | **PASS** (`docker-full`); `docker-pr` PENDING-PO (runs on the PR) | `docker-full` green on CI run 43794695339 (147 passed, 0 failed) and on every earlier M3a dispatch except 43779216264 (a test race, fixed in F3, section 3.3). Locally `run-docker-tests.sh --set full`: 147/0 in 278 s. `docker-pr` runs only on `pull_request`; M3a has had no PR yet (all runs are `workflow_dispatch`), so it first runs on the `m3a` → `main` PR, which is the PO's merge step. Its two halves are covered meanwhile: the `pr` Docker set is a subset of the full set, and line 5 (`hacivat-driver --features fake`, 176/0) ran locally |
| E6 | Read-only on the typed path: SAFE-06 server-blocked rows blocked with the classifier off through `Run`; production consoles cannot write on any engine | **PASS** | Docker: `read_only_bypass_rows_blocked_through_run_without_classifier`, `production_console_rejects_insert_on_every_engine` (`docker_m3a_matrix.rs`), `server_blocked_bypass_rows_are_blocked_through_run`, `neutralised_and_privilege_only_rows_through_run` (`hacivat-session/tests/docker_m3a_readonly.rs`), `a_production_profile_connects_with_a_bound_console_on_every_server_line` (`docker_m3a_f1.rs`). Plain: `prod_read_only.rs` (7), `a_production_insert_is_refused_logged_and_marked_failed` |
| E7 | macOS IME matrix re-run on the real editor; scripted traces green; Windows/Linux rows open | **PENDING-PO** (scripted traces **PASS**) | `hacivat-text/tests/ime_traces.rs` (16) and `hacivat-ui-editor/tests/ime_view.rs` (22 + 1 perf) green, including UI-EDIT-12 test 7 `app_edits_wait_for_the_composition_to_end` and test 8 `bounds_for_range_at_line_49000_under_1ms` (perf line 1). The manual matrix is `m3a-ime-checklist.md` (PO). Windows/Linux rows are open items (section 10) |
| E8 | Shots: 0 violations for every M3a surface; every M1/M2 shot unchanged at zero tolerance; PO sign-off of the M3a surfaces and Turkish strings | **PENDING-PO** (harness half **PASS**) | Section 5: 2,100 shots rendered at the gate tip, **0 violations** (checks a–f, the T35 markers and trust checks), self-checks 8/8. `shot-diff` against `baseline-m2b`: 1,012 compared, the 1,004 unchanged ones at exactly 0.0000 %, **8 breached = exactly the known `menu-app-*`** (the Edit group, ruled legitimate), 1,088 new. Against `baseline-m3a` (as refreshed by F2): **2,100 compared, 0 breached**, every shot at 0.0000 %. CI `pseudo-screens (macos-15)` also rendered 2,100 with 0 violations (3.1). The sign-off of the M3a surfaces and the Turkish strings is the PO's |
| E9 | libpg_query on macOS, Windows and Linux CI and the universal macOS bundle; `cargo deny` with the PostgreSQL clarify; `THIRD_PARTY_NOTICES.md` in every bundle; size delta ≤ +10 MiB per architecture | **PASS** (Linux bundle: see note) | CI 43794695339: build-test, launch-smoke and the release build compile and link the vendored `pg_query` on all three OSes; `--smoke` prints `libpg-query=pg_query 18.1.0 (PostgreSQL 18.6) statements=1` on all three; `nightly-package` macOS (universal dmg, 92,552,192-byte universal binary) and Windows (NSIS) green. `cargo deny --all-features check licenses bans sources` ok locally and on CI. The notices ship through `Packager.toml`'s `resources` key (unchanged since Task 1a, whose review probe listed them in the dmg, NSIS and AppImage). Size: the shipped macOS arm64 binary is 45,562,336 bytes locally (`cargo build --release -p hacivat`) and 45,545,824 bytes on CI `launch-smoke (macos-15)`, against the M2b gate's 37,138,656: **+8.03 MiB** (CI: +8.02 MiB), within +10 MiB. Only arm64 has a recorded baseline (Task 1a); Linux x86_64 is 86,209,632 bytes, recorded here as the first baseline. Note: the Linux AppImage/deb are not produced because of the known AppImage icon red (pre-existing, also on `main`) |
| E10 | Gates green: clippy `-D warnings`, the full test set with the perf lines, `check-deps`, `l10n check`, `tokens --check`, `gen-sql-tables --check`, `cargo deny`, `cargo tree -i aws-lc-rs` empty | **PASS** locally; CI re-run on `2ecec22` due | Section 2: every line green on this Mac. CI 43794695339 at `7e9209f`: the macOS `build-test`, `check-deps`, `l10n-check`, `tokens`, `cargo-deny`, `release-tests` and `switch` ×2 are green. `build-test` (Linux, Windows) and `turkey-test` are red on one test-code defect, fixed in `2ecec22` (section 3.2). Lines 2–8 of those jobs (and the rest of `cargo test --workspace` after `hacivat-ui`) have not run on Linux or Windows since `326ee12`, because an earlier failing line stops the job; the re-run is the first full Linux/Windows pass of the later M3a tests |
| E11 | First frame ≤ 500 ms and idle RSS ≤ 200 MB with no connection; idle RSS with one connection and an open console recorded | **PASS** (no connection); connected reading PENDING-PO | `--smoke` ×3: 243, 194, 197 ms and 121, 114, 114 MiB (section 4.1). `--smoke` never connects, so the connected reading is the demo's post-check 1 (PO) |
| E12 | Cancel within 1 s on every engine on the typed path; stop fetching keeps rows; incomplete cursors leave no transaction open | **PASS** | Docker: `cancel_within_one_second_on_every_engine`, `stop_fetching_keeps_rows_and_frees_the_session`, `incomplete_cursor_leaves_no_transaction_in_pg_stat_activity` (`docker_m3a_matrix.rs`), `pg_suspended_million_rows_released_by_select_1`, `pg_injected_idle_bound_rolls_back_hacivats_transaction`, `pg_stop_fetching_keeps_a_users_transaction_and_a_server_cancel_fails_it`, `mysql_release_kills_on_the_side_connection_and_reuses_within_1s` (`docker_m3a_cursor.rs`). Plain: `incomplete_cursor_rolls_back_hacivats_own_transaction`, `cancel_in_a_user_transaction_marks_it_failed` (P16), `cancel_reaches_the_session_and_stop_fetching_keeps_rows` |
| E13 | The TestScheduler teardown deadlock does not recur: the 50-cycle stress test and every shots run green | **PASS** | `fifty_core_boot_and_quit_cycles_never_hang`, `timed_blocks_while_another_thread_schedules_never_hang`, `watchdog_fails_a_hung_cycle` (`hacivat-app/tests/teardown_stress.rs`) in the workspace and app lines; every test line and the shots render finished without a hang (section 5) |
| E14 | VoiceOver reads the editor by line and announces the selection; the grid is announced as a grid with its title (manual, non-gating; L4 gates at M9) | **PENDING-PO** | Demo step 12. Automated support: `editor_declares_multiline_text_input_with_its_label`, `children_are_text_runs_with_their_text`, `window_is_plus_minus_100_lines`, `execution_status_is_announced_politely` (`hacivat-ui-editor/tests/a11y.rs`); `grid_declares_role_and_label` (`Role::Grid`, "Result 1"), `grid_node_is_the_focused_element`, `the_visible_region_is_announced_once_it_is_still` (`hacivat-ui-grid/tests/view.rs`) |

Summary: E1, E2, E3, E4, E6, E9, E12 and E13 PASS. E5 passes `docker-full`; `docker-pr` first runs on the PR.
E10 passes locally and needs the CI re-run on `2ecec22`. E11 passes its no-connection half. E7, E8 and E14 wait for the PO. Their automated
halves pass.

## 2. Step 2 gates (local, on `m3a-t37-gate`)

Every command ran with `CARGO_TARGET_DIR=<main>/target-core/m3a-t37` (this gate's own target;
Plan Step 1's shared-target clean does not apply), `CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=6` and
`--locked`, from `7e9209f` plus the `M3A_PENDING` removal. Each line ran on its own, so one failure
could not hide another. Docker was not started or stopped; the Docker tests ran through the lock
wrapper.

| # | Command | Result |
|---|---|---|
| 0 | Gate housekeeping: `M3A_PENDING` deleted from `hacivat-app/tests/commands.rs` (it was empty: Tasks 30, 32 and 33 registered every command), together with `every_m3a_pending_id_is_bound`; `keymap_commands_are_registered` now requires every binding registered. `cargo test -p hacivat-app --test commands` | **39/0**, `keymap_commands_are_registered ... ok` |
| 1 | `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| 2 | `cargo build --workspace --release` | ok |
| 3 | `cargo test --workspace --no-fail-fast` (line 1) | **3,946 passed, 0 failed**, 170 ignored (Docker and perf), 280 binaries |
| 4 | `cargo test -p hacivat-ui-core --features test-support,scene-check` (line 2) | 84/0 |
| 5 | `cargo test -p hacivat-l10n --features pseudo` (line 3) | 151/0 |
| 6 | `cargo test -p hacivat-app --features scene-check,pseudo --no-fail-fast` (line 4) | **655/0**, 1 ignored |
| 7 | `cargo test -p hacivat-driver --features fake` (line 5) | 176/0 |
| 8 | `cargo test -p hacivat-testkit --features ssh` (line 6) | 26/0, 1 ignored |
| 9 | `cargo test -p hacivat-ui-editor --features scene-check` (line 7) | 176/0, 2 ignored (perf) |
| 10 | `cargo test -p hacivat-ui-grid --features test-support,scene-check` (line 8) | 161/0 |
| 11 | `cargo test --release -p hacivat-l10n --features pseudo` (line 9) | 154/0 |
| 12 | `cargo test -p xtask l10n -- --ignored repo_catalogs_are_clean` (CI `l10n-check`) | 1/0 |
| 13 | `HACIVAT_PERF=1 cargo test --release -p sql-lexer -p sql-split -p sql-oracle-pg -p hacivat-text -p hacivat-values -p hacivat-results -p hacivat-sql-editor -p hacivat-ui-editor -- --ignored` (perf line 1, with `sql-split` per the T8 ruling) | **19/0** (every perf test of those crates) |
| 14 | `HACIVAT_PERF=1 cargo test --release -p hacivat-driver-sqlite --test typed -- --ignored` (perf line 2) | 1/0 (`first_chunk_of_a_million_rows_within_20_ms_release`) |
| 15 | `HACIVAT_PERF=1 cargo test --release -p hacivat-app --test results_model -- --ignored` (perf line 3) | 1/0 (`cell_through_the_adapter_hit_under_200_ns`) |
| 16 | `cargo build --release -p hacivat-app --features bench --examples` (CI `bench`) | ok |
| 17 | `cargo test --release -p hacivat-app --features bench --lib bench` (CI `bench`) | 25/0, 2 ignored |
| 18 | `cargo xtask check-deps` | `check-deps: ok` |
| 19 | `cargo xtask l10n check` | 0 errors, 0 warnings |
| 20 | `cargo xtask tokens --check` | ok (exit 0, no drift) |
| 21 | `cargo run -p xtask -- gen-sql-tables --check` | fresh |
| 22 | `cargo deny --all-features check licenses bans sources` | `bans ok, licenses ok, sources ok` |
| 23 | `test -z "$(cargo tree -i aws-lc-rs --workspace 2>/dev/null)"` | empty (`aws-lc-rs` is not in the lock graph at all) |
| 24 | `cargo xtask docker check-pins` | ok |
| 25 | `<main>/target/docker/run-docker-tests.sh --set full` | **147 passed, 0 failed**, 19 Docker test binaries, 278 s |
| 26 | Shots: `cargo test -p hacivat-app --features shots,scene-check,pseudo --test shots` self-checks, then `-- --ignored --shard k/3` ×3 | self-checks 8/8; **2,100 rendered, 0 violations**, 3 shards all exit 0, 561 s wall |
| 27 | `cargo xtask shot-diff --baseline <sdd>/baseline-m2b --candidate <target>/shots` | 1,012 compared, **8 breached** (`menu-app-{dark,light}-{en,en-XA,tr,tr-XT}-1280x800`, changed 2.9–7.7 %), 1,088 new; every other compared shot 0.0000 % |
| 28 | `cargo xtask shot-diff --baseline <sdd>/baseline-m3a --candidate <target>/shots` | **2,100 compared, 0 breached**, 0 new; every shot 0.0000 % |
| 29 | After the fix: `cargo test -p hacivat-ui --features scene-check`; clippy for `hacivat-ui` (`--all-targets --features scene-check`) | 225/0; clean |

`cargo fmt --all -- --check` reports only the pre-existing `spikes/**` and `xtask/src/vf.rs`.

**Harness note (shots).** The first render attempt started three `cargo test … --shard k/3` at
once. Each goes through `scripts/dev-run.sh`, which re-signs the same test binary with the
"Hacivat Dev" identity; with the keychain locked the concurrent re-signs failed and left the binary
without any signature, and macOS killed every shard (SIGKILL, no PNGs). The render was redone from
a fresh link (the linker's ad-hoc signature) with the shards run on the binary directly. Nothing in
the repository was changed for this; a later gate that renders in parallel should build with
`--no-run` and start the shards on the binary, or start them one at a time through cargo.

## 3. CI

### 3.1 Run 43794695339 (`workflow_dispatch` on `m3a` at `7e9209f`)

Every job is listed (`gh run view 43794695339`). A red that is not one of the three known reds is a
finding.

| Job | Result | Note |
|---|---|---|
| build-test (macos-15) | success | lines 1–4, 6–8, clippy, release build, the `pg_query` tree unchanged |
| build-test (ubuntu-24.04) | **failure** | **finding, fixed in `2ecec22`** (3.2): `hacivat-ui` `chrome` `a_menu_at_its_max_width_ellipsizes_the_hint_not_the_label` |
| build-test (windows-2022) | **failure** | the same test, the same cause (3.2) |
| turkey-test | **failure** | the same test, the same cause (3.2) |
| switch (macos-15), switch (ubuntu-24.04) | success | language switching tests |
| launch-smoke (macos-15) | success | shipped binary 45,545,824 bytes, within Task 1a's budget; `first-frame-ms=1842`, `idle-rss-mib=78` (shared runner, not the budget machine); `libpg-query=pg_query 18.1.0 (PostgreSQL 18.6) statements=1` |
| launch-smoke (ubuntu-24.04) | success | 86,209,632 bytes (x86_64); `first-frame-ms=2254`, `idle-rss-mib=170` under xvfb; the libpg-query line |
| launch-smoke (windows-2022) | success | the libpg-query line |
| nightly-package (macos-15) | success | unsigned universal dmg; universal binary 92,552,192 bytes |
| nightly-package (windows-2022) | success | unsigned NSIS |
| nightly-package (ubuntu-24.04) | failure | **known**: "Could not find a square icon to use as AppImage icon" (also on `main`) |
| pseudo-screens (ubuntu-24.04) | failure | **known class**: 2,100 rendered, 744 violations. All are (a) "Fluent label not bracketed" on access-key run fragments from `hacivat-ui/src/button.rs` (menus and dialog buttons, 696) or (b) the same 32 `#explorer-filter` clips as before. At `326ee12` it was 360 of the same two kinds over 1,012 shots; it grew because M3a adds the close/quit/failed prompts and the editor context menu, which use access keys off macOS. The harness's check (a) does not join access-key runs; that is the pre-existing Linux item |
| pseudo-screens (macos-15) | failure | **known**: the render passes (2,100 rendered, 0 violations); the shot-diff step against the M1 baselines (`design/baselines/m1/macos`) breaches exactly the 24 M1 rebaselines that wait for the PO (`dialog-about`, `menu-app`, `toast-info`, 8 each); 2,032 new |
| docker-full | success | 147 passed, 0 failed (83 test binaries, 19 Docker targets); `a_parenthesis_before_copy_is_a_syntax_error_on_postgresql` (red in 43779216264) passes |
| docker-pr | skipped | runs only on `pull_request` |
| bench (regression signal only) | success | first run with F3's `send_key`: `b3 --bench typing` runs to the end under xvfb (no reentrancy panic), `RESULT: FAIL b3-typing-app` because the xvfb window is never key (2,000 inactive keys, 392/2,000 samples, p95 49.38 ms; ungraded by design). `b1_real --engine sqlite --any-display`: `RESULT: n/a b1-offpanel-real-sqlite-1440x900`, 417 frames, 0 gaps > 100 ms, grid paint p99 4.72 ms. Spikes: `b1_synthetic` grid paint p99 2.87 ms; spike-editor typing p95 16.78 ms (inactive window) |
| release-tests | success | line 9 |
| check-deps, tokens, l10n-check, cargo-deny | success | |
| keyring-linux, keyring-windows, vault-linux | success | |

### 3.2 CI finding fixed in this gate: `a_menu_at_its_max_width_ellipsizes_the_hint_not_the_label`

- **Symptom.** `build-test (ubuntu-24.04)`, `build-test (windows-2022)` and `turkey-test` fail in
  `cargo test --workspace`. One test, `hacivat-ui/tests/chrome.rs:430`, panics with "the label":
  the menu label "Whole script · 2 statements" is not found as a drawn run.
- **Root cause.** The test (Task 35) found the label as one run with that exact text. `PopupMenu`
  enables access keys by default off macOS (`menu.rs`: `access_keys: !cfg!(target_os = "macos")`),
  so on Linux and Windows the label is drawn as runs around its key letter. This is the M2b P15
  lesson, and the shared `find_label_runs` helper exists for it. The test passed on macOS only.
- **Why it was not seen before.** In run 43779216264 (`bfd3557`) the same jobs stopped earlier, at
  hacivat-app's `exec_ui` failures (F3), so `cargo test --workspace` never reached `hacivat-ui`.
  The last run where these jobs passed is at `326ee12`, before Task 35.
- **Fix (`2ecec22`, test only, TDD).** The test now renders the menu with access keys off and on,
  finds the label with `find_label_runs`, requires more than one run when the keys are on, and
  requires every run to be untruncated and unclipped. RED first: with access keys on and the old
  exact lookup it failed on macOS with "access keys true: the label is not drawn", which is the CI
  failure. GREEN: 15/15 in `chrome`, 225/0 in `hacivat-ui --features scene-check`. No check was
  weakened; macOS now also covers the split-run form.
- **Residual risk.** On Linux and Windows, everything after `hacivat-ui` in line 1 and lines 2–8
  have not run since `326ee12`. A static scan of the later test files found no other menu or dialog
  label looked up as a single run, and no platform-specific chords. The CI re-run on `2ecec22`
  (pushed by the controller) is the evidence.

### 3.3 Earlier M3a runs and E5

| Run | Commit | `docker-full` | Other reds |
|---|---|---|---|
| 43535015009 | `263bf15` (T1a) | success | known only |
| 43538697404 | `677db71` (T1a fix) | success | known only |
| 43545681877 | `051697e` (T1b) | success | build-test ×3 + turkey-test (`u2028_is_not_a_line_break`, fixed in T1b round 1) |
| 43552401456 | `da85ffa` (T1b) | success | known only |
| 43582003477 | `3d61171` (W1) | success | known only |
| 43606952032 | `326ee12` | success | known only |
| 43779216264 | `bfd3557` | **failure** (`a_parenthesis_before_copy_is_a_syntax_error_on_postgresql`: a test race, fixed in F3 by waiting on the state watch) | build-test Linux/Windows + turkey-test (`exec_ui` labels split by access keys, fixed in F3), bench (b3 reentrant `Workspace` update, harness fixed in F3) |
| 43794695339 | `7e9209f` | **success** (147/0) | see 3.1 |

`docker-pr` was skipped in every M3a run: the job runs only on `pull_request`, and every M3a run is
a `workflow_dispatch`. "Known" reds are `pseudo-screens` ×2 (the M1 rebaselines and Linux
violations, also on `main`) and `nightly-package (ubuntu-24.04)` (AppImage: "Could not find a square
icon", also on `main`).

## 4. Smoke and benches on this Mac (Step 3)

### 4.1 Smoke (E11)

`cargo run -p hacivat --release -- --smoke`, three runs after the heavy runs finished, display awake,
no connection (the shipped binary, `cargo build --release -p hacivat`, 45,562,336 bytes):

| Run | first-frame-ms | idle-rss-mib | libpg-query |
|---|---|---|---|
| 1 | 243 | 121 | `pg_query 18.1.0 (PostgreSQL 18.6) statements=1` |
| 2 | 194 | 114 | same |
| 3 | 197 | 114 | same |

Median: first frame 197 ms, idle RSS 114 MiB (M2b: 208 ms, 108 MiB). Budgets: first frame at most
500 ms, idle RSS at most 200 MB. **PASS** for the no-connection case (worst run 243 ms, 121 MiB).
Idle RSS with one connection and an open console: `--smoke` never connects, so it is the demo's
post-check 1 (PO).

### 4.2 B3 and B1-real: attempted, not measurable here

Not run by the gate agent. The display was locked for most of the gate run, which makes any
`b3` or `b1_real` figure INVALID (the window cannot become frontmost; Task 36 saw the same). When the
PO came to the Mac, the controller took over: **E2 and E3 are PENDING: controller run with the PO**,
on a quiet machine, with the commands in 4.3. The controller fills in the numbers and the bench rows
at the end of this file.

### 4.3 Commands for the PO (display awake, unlocked, built-in 120 Hz panel at 2×)

Before running: the session unlocked and the display awake; the first B1-real run on the built-in
120 Hz panel at its default 2× scale (anything else is INVALID: exit 3, no JSON); `pg17` healthy;
keep the bench window frontmost and do not touch the mouse or keyboard while a bench runs.

```sh
cd <the m3a checkout>
cargo xtask gen-sql --lines 50000 --out target/fixtures/b3-50k-t0.sql                    # T0, about 2 MB, full tier
cargo xtask gen-sql --lines 50000 --long-line 100000 --out target/fixtures/b3-50k.sql    # T1, Reduced mode
cargo build --release -p hacivat-app --features bench --examples
E=target/release/examples      # or $CARGO_TARGET_DIR/release/examples

# B3 in the real app (E3)
for m in typing typing-with-results open scroll; do
  $E/b3 --file target/fixtures/b3-50k-t0.sql --bench $m
done
for m in typing-long-line ime-bounds; do
  $E/b3 --file target/fixtures/b3-50k.sql --bench $m
done

# B1-real (E2): 1M × 50 from pg17, tr-TR
HACIVAT_DOCKER_SET=pr $E/b1_real                         # 1440 × 900 on the built-in panel (2×, ≥ 118 Hz)
HACIVAT_DOCKER_SET=pr $E/b1_real --size 2560x1440        # a 1× external display set as the main display
HACIVAT_DOCKER_SET=pr $E/b1_real --mode streaming        # not gating

# Rows for this file's bench table
cargo xtask bench-report --milestone m3a target/bench/b1-fling-real-*.json target/bench/b3-*-app-*.json
```

Read each run's `RESULT:` line (PASS, FAIL, INVALID or n/a). The B1-real paint figure for the gate
is the `GRID PAINT (the gate's paint figure)` line; the load figures are in `target/bench/load/`.
Record the machine, the build hash (`git rev-parse --short HEAD`) and the raw dumps with the rows.

## 5. Shots (E8, E13; Step 6 waits for the PO)

**Render** (from `7e9209f` plus the `M3A_PENDING` removal; the `chrome.rs` fix does not touch the
render): 3 shards, side by side, on the linker-signed test binary. Self-checks 8/8. **2,100
rendered, 0 violations**; every shard exited 0; 561 s wall. The PNGs are in
`<main>/target-core/m3a-t37/shots/` (the controller removes that target after the gate). Since the
render equals `<sdd>/baseline-m3a/` pixel for pixel, the PO reviews that folder, and Step 6 copies
its macOS PNGs.

**Against `baseline-m2b`** (`<sdd>/baseline-m2b`, the M2b render at `5d5832f`, P17), the shot-diff
defaults (ΔE2000 2, share 0.1 %): 1,012 compared, the 1,004 unchanged ones at exactly 0.0000 %, and
**8 breached, exactly the known `menu-app-*`**:
`menu-app-{dark,light}-{en,en-XA,tr,tr-XT}-1280x800@2x.png` (2.9–7.7 % changed: the Edit group,
greyed without editor focus). 1,088 new (the M3a states).

**Against `baseline-m3a`** (`<sdd>/baseline-m3a`, 2,100 PNGs, refreshed by F2): **2,100 compared,
0 breached, 0 new**; every shot at 0.0000 %. The render is reproducible from the F2 baseline.

E13: the render booted and stopped the core runtime for every live surface without a hang, and so
did every test line in section 2.

What the PO signs off (with `po-demo-m3a.md`, "Ekran görüntüleri"):
- the M3a surfaces (68 states × `en`, `tr`, `en-XA`, `tr-XT` × dark/light × 1280 × 800 and
  960 × 600) and their Turkish strings;
- the 8 `menu-app-*` changes against M2b: the Edit group is shown, greyed without editor focus
  (ruled legitimate during T32/T33);
- F2's three layout decisions: the two-line results strip at 960 in the pseudo locales (the grid
  loses one row), the wrapping toolbar (the waiting sentence may move into ⋯), and the status
  bar's give-way order (cells leave whole; the message ellipsizes only after rows · timing gives
  way).

**Step 6 (PO-gated, not done):** after sign-off, copy the macOS PNGs of the render to
`design/baselines/m3a/macos/` and commit them.

## 6. Windows and Linux (Step 5)

No Windows or Linux host was provided, so demo steps 2, 4 and 10 and the IME rows were not run
there. CI covers them as far as it can: `build-test` and `launch-smoke` on Windows and Linux,
`turkey-test` under `tr_TR`, `switch` on Linux, `pseudo-screens` on Linux, `nightly-package`
Windows (green) and Linux (known icon red). Open items: section 10.

## 7. Rulings for PO confirmation

The PO's mandate (2026-11-13/14) let rulings take effect immediately; each is listed here for
confirmation. The most important first.

### 7.1 Key rulings (the plan's Step 7 list)

- **R-M3-22, off-thread lexing.** The SQL adapter lexes and splits each committed snapshot on a
  background worker; highlights may lag one frame on very large files and are rebased through the
  change log; the preview shows only at the exact version. Selection previews over 32 KiB also
  run on the worker (T24 I1).
- **R-M3-23, cursor eligibility and rollback (amended by P26).** Without a classifier a unit may stay
  suspended only with a read head (`SELECT`, `VALUES`, `TABLE`, MySQL `SHOW`, or `WITH` without DML,
  no `INTO`, no locking clause); a parenthesised `(SELECT …)` is never eligible. A suspended cursor
  that does not complete ends Hacivat's own transaction with `ROLLBACK`: when another statement
  needs the session, at the idle bound, or on close. The log says "Hacivat's transaction was
  rolled back". Cost: functions with side effects in a read head are rolled back too (P26).
- **R-M3-24, ambiguity (changed in effect by the "preview vs Run" ruling).** Ambiguous boundaries send
  nothing. Run resolves **without** the PostgreSQL oracle and runs only when the preview on the same
  snapshot shows the same units or candidate; T23's oracle code stays unused by the app. Cost:
  regions only the oracle could settle cannot run at the caret in M3a.
- **R-M3-27, the transaction prompt.** Closing a console, quitting or restarting with a user
  transaction asks: Commit / Roll back / Cancel, Cancel focused; a Failed transaction offers only
  Roll back and Cancel (P16). Commit on a failed transaction is refused before it is sent.
- **R-M3-29, the production mechanism.** `effective_read_only(profile, production_read_only)` at
  every session read-only site; the app sets `production_read_only: true`, so production consoles
  are read-only on every engine until M3b's confirmations (R-M3-6 REVISED).
- **R-M3-37, the unbound console.** An unbound console keeps the M2 home; the first successful connect
  binds the focused unbound console; there is no "New console" on the home.
- **P16, psql semantics.** A server cancel inside a user transaction leaves it Failed; the
  transaction segment says so; the close prompt offers only Roll back and Cancel; per-unit savepoints
  are not used. Cost: a late cancel of a non-suspendable statement in a user transaction loses the
  earlier work unless the user rolls back, as in psql.
- **P26, rollback of what read-head functions wrote** (with R-M3-23 and R-M3-49): stated in the log
  line; a restored console over 20 MB is refused and kept on disk.

### 7.2 Planner rulings R-M3-22..49 (plan "New rulings")

| Ruling | Summary | Later change |
|---|---|---|
| R-M3-22 | Off-thread lexing; snapshots tagged with the editor version | Selection previews > 32 KiB on the worker (T24) |
| R-M3-23 | Cursor eligibility without a classifier; incomplete cursors roll back | Amended by P26; `(SELECT …)` never eligible; guard holds report `rolled_back: false` |
| R-M3-24 | Ambiguous boundaries send nothing; PG oracle for exact cover | Oracle only with `standard_conforming_strings` on (T23 C1); then Run resolves without the oracle (preview-vs-Run ruling) |
| R-M3-25 | One result set per unit; more sets drained, counted, logged, noted | — |
| R-M3-26 | Error policy StopOnError | — |
| R-M3-27 | User transactions: segment, tx commands, close prompt | Commit on Failed refused before send; restart guarded (T33 C3) |
| R-M3-28 | ADRs 0005 (SQL), 0006 (execution), 0007 (editor and grid) | — |
| R-M3-29 | `effective_read_only` at every read-only site | — |
| R-M3-30 | Shots for every M3a surface; DESIGN.md section in M3b; mock NOTES binding | — |
| R-M3-31 | Typed decoders for the listed types; the rest `Value::Opaque` | xml and citext also Opaque |
| R-M3-32 | Grid/results scope cuts to M3b; grid role Grid with its label | Grid a11y node on the focused root (T30) |
| R-M3-33 | Fetch all to the 256 MiB cap; memory banner is the confirmation | B1-real uses `raise_cap` |
| R-M3-34 | No `sql-ident`/`sql-safety`/`sqlparser` in M3a; the check-deps rules land | — |
| R-M3-35 | PG oracle on a 16 MiB-stack thread, nesting ≤ 256 | One serial parser thread; 8 MiB per-call cap; the app no longer calls it |
| R-M3-36 | The D-011 chooser at the caret, no access keys | Bidi and default-ignorable code points shown as `<U+XXXX>`; F2 tint and key-hint footer |
| R-M3-37 | Unbound console keeps the M2 home | — |
| R-M3-38 | Chords: cancel `cmd-.`; Escape never cancels | Amended: GPUI has no Break key → `ctrl-f2` on Windows/Linux and in the DataGrip preset |
| R-M3-39 | `hacivat-exec::sqlctx` builds the dialect context from one probe | Re-probe after RESET/DISCARD/ROLLBACK undoing a SET; the session owns the probe, which runs outside the guard; failure → `Unknown` (F1-c) |
| R-M3-40 | Benches are examples behind `bench` | `bench` also enables `hacivat-ui-grid/bench` (named manifest exception) |
| R-M3-41 | The results tile follows the focused pane's active console | — |
| R-M3-42 | Shared type shapes (`TextEdit`, `TokenClass`, `StmtKey` XXH3-128) | — |
| R-M3-43 | "Waiting for a connection" derived in the app | — |
| R-M3-44 | `[editor]` settings in the file; Settings UI rows in M3b | — |
| R-M3-45 | Run on a disconnected bound console starts the connect, runs nothing | — |
| R-M3-46 | PG `BEGIN`/`SAVEPOINT` before a cursor is its own round trip | — |
| R-M3-47 | The M2b debug cancel demo goes away (T33) | Done |
| R-M3-48 | The result store publishes on every append | — |
| R-M3-49 | Nothing over 20 MB enters the editor; restored consoles over 20 MB refused | Amended by P26; the refusal runs off the UI thread |

Also for confirmation: the digest rulings R-M3-1..21 and R-M3-50 (ledger "Planning" and "SPLIT
DECISION"), with these changes: R-M3-3's classifier pool is one serial parser thread in M3a; R-M3-9's
≤ 2 ms incremental budget moves to the M3b gate; R-M3-16's "never aborts the user's transaction" is
replaced by P16.

### 7.3 Pre-flight rulings P1–P28

| | Ruling |
|---|---|
| P1 | Vendor `pg_query` 18.1.0 as `third_party/pg_query`; the build never regenerates protobuf; bindgen kept |
| P2 | `m3a` cut from the green M2b tip `5d5832f` |
| P3 | Rejected (stale shared rate-limit file); pause at weekly ≥ 98 % |
| P4 | `effective_read_only(profile, production_read_only)`; `SessionManager::set_policy`; seven sites |
| P5 | `Client::query_with_formats` for non-suspendable units; named portals only inside a transaction; PG suspendable needs a transaction |
| P6 | Manifest rows fixed in T1b; later tasks may not edit manifests |
| P7 | The bench examples exist from T1b as exit-2 stubs |
| P8 | `Value` wide payloads behind `Arc` (32 B), ADR 0006 |
| P9 | `xtask docker test --test`; perf tests self-gate on `HACIVAT_PERF=1`; three perf lines; the wrapper fails when its wait expires |
| P10 | Test lines 6–8 in `build-test` and `turkey-test`; one libclang action |
| P11 | Exact editor seams; T19 owns the call sites |
| P12 | Every results/console/exec_ui submodule declared in T1b |
| P13 | The fork commit prepared in T1a, pushed fast-forward by the controller |
| P14 | gpui build caps (4/3/2) and `CARGO_BUILD_JOBS` |
| P15 | Chooser without access keys; shared run-joining label finder; `Path` comparisons; CRLF generated at test time |
| P16 | psql semantics for a cancel inside a user transaction (section 7.1) |
| P17 | `baseline-m2b` preserved in the SDD folder; every shot-diff names it |
| P18 | Wrong test values in the plan corrected |
| P19 | Docker stack up from W1 |
| P20 | `stop_core_runtime` for both test contexts |
| P21 | The eight M2b app test files flipped by `production_read_only` updated without weakening |
| P22 | `PRODUCT_CRATES` extended; M5 exemptions named in ADR 0005 |
| P23 | `Packager.toml` resources key for the notices |
| P24 | `LogMsg::fluent_id` returns a literal per arm |
| P25 | Grid select-column unbound in M3a; `exec.cancel` gains `ctrl-f2`; cost: `ctrl-f2` switches desktops on KDE |
| P26 | R-M3-23/R-M3-49 costs restated; `rolled_back` in the log line |
| P27 | E1 gets `dml_and_ddl_report_update_counts_on_a_dev_console`; T27's 1 ms assert is a perf test |
| P28 | Rules copied before T1a; `b1_real` documents `HACIVAT_DOCKER_SET`; misc pre-flight findings |

Plus the split ruling: Task 1 became T1a "Proofs" and T1b "Seams".

### 7.4 Later controller rulings worth the PO's look (from the ledger)

- **Preview vs Run** (T33): Run resolves without the PG oracle and must agree with the preview;
  property-tested at 9,700 places (one documented divergence).
- **Selections** (T13): inside `$$` bodies refused; inside a psql `\if` false branch they run as
  selected while a caret there is ambiguous; a selection inside a `--` comment runs (explicit
  intent); a selection after odd-quote COPY data, or after a client command with an odd quote, is
  refused; a selection equal to a later whole statement still runs.
- **Undo grouping** (T5): runs of spaces and `;` stay in the previous word's undo group.
- **Copy** (T26): error cells copy empty; truncated prefixes are kept with a warning toast; copy
  carries the result generation (a stale copy is refused).
- **Close-cursor reason** (T7): Close cursor from the memory banner closes with `MemoryLimit`; the
  reason follows the state, not the button.
- **Cancel** (T18/T31): stopping a non-suspendable PG unit sends a protocol cancel (the transaction
  becomes Failed); inside a user transaction a non-suspendable PG unit is cancelled only when
  overdue; MySQL KILL makes `SLEEP()` return 1, shown as already finished.
- **IME** (T10/T27): app edits are deferred or refused while composing (no public GPUI call ends a
  composition); paste and cut are refused during a composition, copy always runs.
- **A11y fork carry** (T29/T30): GPUI view reuse is skipped while a screen reader is active
  (fork commit on `hacivat/912159c`).
- **Status bar and toolbar** (F2): trust signals are never truncated or dropped down to the minimum
  split pane; cells give way whole; the message ellipsizes only after rows · timing gives way;
  `ToolbarItem::narrow` for panes under the per-locale minimum is deferred to M3b.
- **Default session** (F1): another session's in-flight user request counts as a possible open
  transaction before the default session idles; the loss of the default session ends every
  console session (rollback reported).
- **Fluent and copy** (T25, T26, T30, T34): new strings without a mock entry (describe items, copy
  refusals, `hv-copy-done-incomplete`, size units) wait for the PO's Turkish review.

### 7.5 PO notes from the ledger (decisions or looks)

- One duration format: `exec-dur` "131 ms" vs the status bar's "0.13 s" (T30).
- DDL logs "Completed" on PostgreSQL vs "0 rows affected" on MySQL/SQLite (T31).
- The editor status segment sits left of the version (plan order); the mock wants it right
  (T32/T34; needs a `status_right` hook).
- The toolbar `+` on `Truncated(FollowingResultSets)` vs the strip's note (T30 M13).
- Chooser and prompt layouts differ from the mock: no chooser footer before F2, the prompt's role
  is Dialog, not alertdialog, all prompt buttons are on the right; "Nothing to run here" as a toast
  with an a11y announcement (T33).
- A mark's end edge can swallow the next typed statement (T28 M10, plan-mandated ruling 6).
- Toast wording in a rare idle race ("lost" vs idled out) and F1-b having no bound of its own
  (M3b per-session liveness).
- Whether `give_way_row` should round (seams < 1 px apart are treated as one).
- The T1a concern: no notices yet for the Rust crate graph (M9 packaging).
- 162 mock copy strings marked for PO review (T2), and the TR of `exec-tx-committed`.

## 8. Deferred work, with owners

### 8.1 The plan's "Deferred work" table

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

### 8.2 M3b carries from the ledger and the reviews

| Item | Owner | Source |
|---|---|---|
| `ToolbarItem::narrow`: split panes between the 120 px floor and the per-locale minimum (en 126, tr 141, en-XA/tr-XT 219 px) keep every trust item whole; today the production chip is capped at the bar width and runs past it. Plan: a per-item narrow form with measured heights | M3b (also on the PO list) | F2 report; controller ruling |
| F2 review N1: the `status-everything` rows exemption at 960 should apply only to the pseudo locales (harness only; `tests/shots/m3a.rs:1026` still exempts every locale) | M3b | F2 review |
| F2 review N2: require `#button-label-console-waiting-cancel` in both waiting states and in the `toolbar_narrow.rs` label filter | M3b | F2 review |
| F2 review N3: stale `toolbar.rs` docs after M7; `stage_fit`/`staged_width` still reserve the overflow button and `Plan::Lines` pushes a button slot (a possible empty line, not reachable today) | M3b | F2 review |
| F2 review N4: the narrow-pane sweep covers only idle/waiting; add running/cancelling, the pending-read-only chip and the strip's "Cursor open" | M3b (with `narrow`) | F2 review |
| T34 M6: Reduced mode never gives way in the status bar; routed to T32 and left by F2 M8. `connections/status.rs` keeps `Yield::Whole(40)` for the editor cell and `EditorSegment::reduced()` is not read | M3b follow-up | T34 review, T32 routing |
| F2 M8: Ln/Col leaves before the path (the editor cell is one entity, rank 40 over the path's 30); NOTES wants the selected count before the path and Ln/Col after. Known deviation | M3b (with the editor-segment position, PO note) | F2 review |
| F2 M9: no tokens yet for `WRAP_LINE_PX` (16), `WRAP_PAD_PX` (4), the compact button metrics (20/7/5/12) and the menu footer's 5 px gap | M3b DESIGN.md section | F2 review |
| Multi-window: `ActiveTab`, `FocusedEditor` and `StatusFollow` are app-wide; two workspace windows repaint each other; `ResultsEvent` handled twice | Multi-window milestone | T34 M9/N4 |
| Per-session liveness: only the default session has liveness (F1-b has no bound of its own) | M3b | F1 review |
| T32 follow-ups: editor commands do not run from the palette (focus restore order); `FocusContexts` has no unregister (one handle per closed console); the empty-console placeholder line is not drawn; Run has no play icon (`play.svg` has no `IconName`); re-clicking the active console tab after an explorer pick keeps the M2 home until the editor is focused | M3b follow-up | T32 review/confirmation |
| Session: publish the transaction and drop the cursor on close or break; ABORT refused in a failed transaction | M3b | T34/T23 routing |
| MySQL: `CALL` check reads the text tag; release reads a result to its end with no final OK; the chip shows SQLSTATE 25006, not vendor 1792; `time_zone` in `NativeType::MySql`; generic type labels; the run service drops the command tag | M3b | T22, T14, T30, T26, T23 |
| PG RO startup option `idle_in_transaction_session_timeout=30s` can be refused by a pooler (pgbouncer) | M3b | T22, T31 |
| MariaDB bypass rows (C19) | M3b | T22 |
| RO guard holds report `rolled_back: false` (the log is silent there) | M3b | T22, T23 |
| libpg_query: the stderr pipe captures other threads' writes; prost depth limit (47 nested `ARRAY[` or 100 UNIONs) fails a full decode, the classifier must fail closed; child processes inherit the pipe; deparse hazards | M3b | T9 |
| Grid: frozen columns, edge autoscroll while the pointer is still, RTL; copy through `ViewMap` (HV-GRID-06); `cap_raised` onto `ResultSource`; Tab exits a large grid only from the last loaded cell | M3b | T17, T20, T26 |
| Lexer: `x'…'`/`b'…'` inside comment bodies read as strings; a client command's argument should end at end of line (removes the odd-quote cost); MySQL `"…"`/`_charset'…'` double-click falls back to the whole token | M3b follow-up | T4, T13, T24 |
| Output log O(n) re-read and per-change strip-fact rebuild → `lines_since`; keymap rows for log line selection, Ctrl+Tab/⌘3 | M3b | T30, T33 |
| A cancel answered "already finished" after Hacivat's own kill must not read "had already finished" | M3b | T31 M7 |
| Editor minors: `selected_chars` is O(selection); no drag autoscroll timer; the horizontal thumb is not draggable; f32 x on extreme lines; size-only advance caches | M3b | T19, T20 |
| Idle bound stale after `SET SESSION net_write_timeout` | M3b | T14 review |
| Unused `answers_probe` typed answer | Next touch | F1 M6 |
| rustdoc: ~20 ambiguous-link warnings under `connections/` and 2 in the grid docs | Cleanup (not a gate line) | T26 note |

## 9. PO-only checklist

1. **B1-real (E2) and B3 (E3)** on the built-in 120 Hz panel, display awake and unlocked (the
   controller runs them with the PO): section 4.3. Then `cargo xtask bench-report --milestone m3a …`
   fills the table at the end of this file.
2. **macOS IME matrix (E7):** `docs/benchmarks/m3a-ime-checklist.md`, every macOS row and the T37 list,
   including **UI-EDIT-12 test 7** (row 1a: a "go to error" while composing).
3. **The 12 demo steps** and the connected RSS reading (E11): `docs/benchmarks/po-demo-m3a.md`.
4. **Turkish string sign-off:** every new M3a surface in `tr` (shots and the demo), the strings marked
   for review in section 7.4–7.5, and the TR of `exec-tx-committed`.
5. **Shot sign-off (E8):** the M3a surfaces, the 8 `menu-app-*` changes, F2's two-line strip, the
   wrapping toolbar and the status bar's give-way order (section 5).
6. **VoiceOver (E14):** demo step 12.
7. **Rulings:** confirm or reverse section 7 (reversals become follow-ups).
8. **After sign-off:** copy the macOS PNGs to `design/baselines/m3a/macos/` (Step 6); the controller
   pushes `2ecec22` and re-runs CI; then the merge of `m3a` to `main` (its PR also gives `docker-pr`).

## 10. Open items

- **Windows and Linux IME** (R-M3-7): the rows in `m3a-ime-checklist.md` stay open until the PO
  provides hosts, and so does whether the clipboard's line-wise metadata survives there.
- **NVDA** (Windows screen reader): not checked; the 1.0 walkthroughs gate at M9 (L4).
- **The M3b plan is to be written now** (the controller has started it; rulings are numbered from
  R-M3-51).
- **CI re-run on `2ecec22`:** the first full Linux/Windows pass of the later M3a tests (section 3.2).
- **Known reds** (not M3a): `pseudo-screens` ×2 and the Linux AppImage icon.

## Bench notes (Task 36)

- **What the bench binaries link.** Cargo builds examples with the package's dev-dependencies, so `b3` and `b1_real` also enable the dev-dependency features: `gpui/test-support`, `hacivat-ui-core/test-support`, `hacivat-ui-grid/test-support` and `hacivat-app/test-support`. The runtime cost is negligible. In particular, GPUI's per-handle leak detector is behind `cfg(any(test, gpui_leak_detection))`, not `test-support` (pinned gpui, `crates/gpui/src/app/entity_map.rs:1058`). Nothing in the bench path changes product behaviour: the `bench` module exists only with the feature.
- **The grid's paint timer.** The app's `bench` feature also enables `hacivat-ui-grid/bench`. This is a named manifest exception from the controller in the Task 36 fix round. `b1_real`'s gate paint figure is the grid's own paint (p50/p95/p99/max, frames over 3 ms and over 6 ms). The window's frame CPU is printed after it as an upper bound.
- **Graded conditions.** `b1_real` grades only `b1-fling-real-pg-<viewport>`:
  - 1440 × 900 on the built-in panel (scale 2, at least 118 Hz), or 2560 × 1440 on a 1× display;
  - the viewport at least 90 % of the requested size;
  - the `tr-TR` format locale.

  Any other display is INVALID: exit 3, no JSON. `--any-display` runs ungraded as `b1-offpanel-real-…`. Another format locale runs as `b1-offlocale-real-…`, also ungraded.
- **B3 fixtures.**
  - `typing`, `typing-with-results`, `open` and `scroll` use the T0 fixture, `cargo xtask gen-sql --lines 50000 --out target/fixtures/b3-50k-t0.sql` (about 2 MB, full tier).
  - `typing-long-line` and `ime-bounds` use the T1 fixture, `--lines 50000 --long-line 100000`. Its 101,491-character line opens the file in Reduced mode.
  - Each result line prints the tier. `typing`, `typing-with-results` and `open` refuse anything but the full tier.
- **Results.** Every bench ends with `RESULT: PASS|FAIL|INVALID|n/a <label>`. The exit code is 0 for PASS or ungraded, 1 for FAIL or aborted, and 3 for INVALID.

## Bench rows

`cargo xtask bench-report --milestone m3a <dumps>` adds rows after the last table row anywhere in
the file, so this table stays the file's last table. Empty until the PO's panel runs.

| label | OS | host | build | display | frames | mean fps | p95 ms | p99 ms | max gap ms | gaps > 100 ms | verdict |
|---|---|---|---|---|---|---|---|---|---|---|---|
