# M3c gate record

Milestone M3c, "Complete": the editor's multi-cursor, IME mirroring, folding, find and line
commands (Tasks 31–34); the grid's view map, frozen columns, multi-rect selection, local sort,
find, Filter rows, go to row, the Table/Transposed/Text views, the record inspector, the value
viewer with "Re-run with a larger limit", the grid context menu and the AccessKit subtree
(Tasks 35–40); the narrow toolbar forms and the trust-signal floor (Task 41); the Settings rows
(Task 42); the M3c shots, cross-surface flows and DESIGN.md "Query, Results and Grid" (Task 43);
and the B3/B1-real modes (Task 44).

This is the **draft** gate record (Task 45, docs part). It records the evidence the M3c tasks
produced and prepares what is left. Cells marked **PENDING (controller)** are the controller's to
fill (the local gate lines, CI, merges). Cells marked **PENDING-PO** need the PO at the Mac. The
PO's hand demo is `docs/benchmarks/po-demo-m3c.md` (Turkish).

- Gate branch for this record: `m3c-t45-docs`, from `m3c` at `b540c4c` (T31–T43 merged, the
  shot fixes, gutter-no-run, the PO fixes, the integration fixes and the m3b forward merge
  `9ee9f21`). <!-- ## m3c-t45 (controller, gate tip) --> **Gate tip: `m3c` at `83edd55`**:
  T44 `c19bbfc`, the narrow fixes `fe1c633` + fix round 1 `100e4e6`, this record `e1a14e5`, the
  shots thread fix `127d251`, PO P2-07 `ff7e4fe`, and a test-gating fix `83edd55` (section 2).
- SDD = `.superpowers/sdd/2026-11-14-m3-walking-skeleton/` (task reports, reviews, shot lists,
  baselines). No hostnames, accounts or personal names are recorded here.

<!-- ## m3c-t45 -->

## 0. PO-present work (everything the PO must do when back)

The controller runs everything else first (section 3 and the PENDING (controller) cells). Then,
with the PO at the Mac, display awake and session unlocked:

1. **T44 Step 2 measurements** (E5): the B3 loop and the B1-real runs in
   `docs/benchmarks/m3c-bench-notes.md` sections 2–4 (on `m3c-t44-bench` until T44 merges). Close
   other builds and agents first; keep the bench window frontmost; wrap the runs in
   `caffeinate -dis` so the display does not sleep mid-run. Then
   `cargo xtask bench-report --milestone m3c …` (section 6.2 of this file).
2. **The hand demo** (E1–E4): `docs/benchmarks/po-demo-m3c.md`, about 60–90 minutes, Docker stack
   `hacivat-m2a` as it is (no container is recreated). It includes the IME re-run
   (`docs/benchmarks/m3a-ime-checklist.md`, section "M3c: several carets").
3. **VoiceOver smoke** (optional, non-gating; PO answered "Sonra" to M3c-03): demo step 14, the
   nine T40 items.
4. **PNG sign-off** (E6): the changed approved shots in section 5, and PO page 2
   (https://claude.ai/artifact/R7xAfM1fWxoFe34aZuaAUD). P2-01..P2-13 were answered on
   2026-11-18 (P2-13, the earlier shot sets, approved). **Still open:** P2-14 (zero-width
   inactive tabs at 120 px), P2-15 (the narrow placeholder crosses the gutter), P2-16 (the
   176 new `narrow-*` PNGs) and P2-17 (the 16 `marks-denied` PNGs: the shorter several-targets
   denial, section 3). Also the DESIGN.md "Query, Results and Grid" section.
5. **Rulings confirmation**: section 7 (the "PO confirm" column), plus the open questions in
   section 8.
6. **Merge to `main`**: after 1–5, the controller opens the `m3c` → `main` PR (which also runs
   `docker-pr`), and the PO approves the merge. Nothing is merged before the PO says so.

## 1. Exit criteria E1 to E7

| # | Criterion | Status | Evidence |
|---|---|---|---|
| E1 | **The full §6 M3 exit:** read-heavy daily work on all three engines (sort, find, go to row, views, value viewer, history), B1 on real store data, D-011 goldens, SAFE-06 on/off (M3b E1 still green) | **PASS (automated)**; demo steps 1–9 **PENDING-PO**; B1-real **PENDING-PO** (E5); workspace line 5,467/0 and Docker full set 193/0 at the gate tip `83edd55` (section 2) | Sort T36 (`e307fd0`): `crates/app/hacivat-app/tests/results_sort.rs` `copy_after_sort_copies_what_is_shown`, `a_stale_sort_never_replaces_a_newer_one`, `sort_of_a_million_rows_is_cancellable_and_off_thread`; `hacivat-values/tests/compare.rs` `turkish_collation_sorts_ilik_irmak_istek_izmir`, `code_point_order_without_a_mapped_collation`. Find/Filter rows/go to row T37 (`8806031`): `results_find.rs` `find_over_truncated_marks_incomplete`, `null_never_matches`, `filter_rows_builds_a_filtered_map`, `goto_past_loaded_fetches_with_a_confirmation_above_1m`, `a_sort_while_finding_rescans_in_the_order_shown`; `hacivat-ui-grid/tests/goto.rs` `goto_row_reads_locale_grouping`. Views and viewer T38 (`8c1c767`): `results_views.rs` `transposed_disabled_above_100k_rows`, `value_viewer_pretty_json_is_display_only`, `truncated_value_offers_a_rerun_with_the_raised_limit`, `rerun_with_raised_limit_goes_through_the_gate`, `rerun_with_raised_limit_runs_only_a_read`, `rerun_with_raised_limit_refuses_a_write_on_production`. History: M3b T27 (`history_ui.rs`, `history_insert_and_run_meets_the_gate`) and T34 recall (`hacivat-ui-editor/tests/lines.rs::history_recall_at_the_last_line`). Integration (`909d625`, review `m3c-integration-review.md`, every SQL-sending path traced through the gate): `results_m3c_int.rs` 9 tests. Cross-surface (`cross_surface_m3c.rs`): `sort_then_copy_copies_the_shown_order`, `grid_find_filter_rows_then_copy_copies_the_kept_rows`, `go_to_row_1_000_in_turkish_reaches_view_row_1000_under_a_sort`, `value_viewer_rerun_after_a_sort_takes_runs_path_and_drops_the_sort`. D-011 goldens: `sql-split/tests/d011_goldens.rs` in the workspace line. SAFE-06: M3b E1 suites (`bypass_run_a.rs`, `docker_m3b_bypass.rs`, `docker_m2b_bypass.rs`) in the workspace and Docker lines. **Engines:** the grid features are engine-neutral above `ResultSource`; the per-engine part is the sort comparator (`compare.rs`: PG, MySQL/MariaDB zero dates and BIN/BIN2, SQLite int/float). **Known limit (PO-approved M3c-06):** the app sorts text by code point with the "collation unknown" tooltip on every engine; `collation_of` returns None (`results/sort.rs:105`); the ICU/tr_TR comparators are built and tested but reachable only from tests until M5's catalog lookup (section 4) |
| E2 | Editor: multi-cursor + IME mirroring (macOS rows for several carets, the M3a checklist re-run), folding, find (Turkish-tolerant), line commands | **PASS (automated)**; IME re-run and demo steps 10–12 **PENDING-PO** | T31 (`45c61f9`): `hacivat-text/tests/multi.rs` `typing_at_10k_carets_under_50ms` (8.5 ms), `select_next_occurrence_is_case_and_turkish_exact`, `a_platform_range_moves_every_mirror_by_the_same_distances`; `hacivat-ui-editor/tests/ime_mirror.rs` `ime_mirrors_to_every_caret_in_plain_text`, `ime_falls_back_to_the_primary_caret_inside_a_string`; `multi_view.rs` `run_follows_the_main_caret`, `one_undo_removes_a_multi_caret_edit`; `hacivat-app/tests/exec_ui.rs::several_carets_run_only_the_main_carets_statement`. T32 (`6e4cbdb`): `hacivat-text/tests/fold.rs` `ui_edit_10_cases_1_to_5`, `line_at_y_on_50k_lines_with_1000_folds_under_5us`, `fold_upkeep_with_3000_folds_stays_cheap`; `fold_view.rs` `folded_lines_are_not_in_a11y_runs`, `exec_mark_inside_a_fold_paints_on_the_header`. T33 (`0f09d78`): `hacivat-app/tests/find_bar.rs` `turkish_tolerant_matches_dotted_and_dotless_i`, `exact_mode_does_not_fold_turkish_letters`, `a_long_tolerant_literal_compiles_and_never_fails`, `typing_in_the_field_over_20_mb`; `hacivat-ui-editor/tests/find.rs` `counting_never_blocks_the_ui_thread`, `select_all_matches_caps_at_10000`. T34 (`2e81c7f`): `lines.rs` `toggle_comment_uses_the_dialect_line_comment`, `move_line_moves_a_fold_with_it`, `toggle_comment_on_a_folded_header_takes_the_whole_fold`; `gutter_m3b.rs` `gutter_play_runs_the_rows_statement`, `no_play_while_run_is_off_or_over_a_live_mark`, `each_gutter_column_is_one_tab_stop_walked_with_the_arrows` (PO M3c-04). PO chords (`6e81502`): `m3b_routing.rs::m3c_po_chords_collide_with_nothing_in_any_preset`. Cross-surface: `multi_caret_typing_is_one_undo_and_run_runs_the_main_caret`, `run_at_a_caret_inside_a_fold_runs_the_whole_statement`, `find_select_all_then_type_edits_every_match_as_one_undo`. **IME checklist:** extended by this task (`m3a-ime-checklist.md`, fenced section "M3c: several carets"); the rows are empty for the PO |
| E3 | Grid: frozen, multi-rect, sort with Turkish collation, find/filter, go to row in `tr`, views, value viewer, context menu, AccessKit subtree (manual VoiceOver smoke, non-gating, L4 at M9) | **PASS (automated)**, with the collation limit above (PO-approved M3c-06); demo steps 1–9 **PENDING-PO**; VoiceOver: non-gating, PO "Sonra" (section 9) | T35 (`90a0b8c`): `hacivat-ui-grid/tests/model_viewmap.rs` `permuted_map_lookup_under_50ns`, `shift_extends_from_a_gutter_row_take_view_rows_under_a_shuffled_sort`, `counting_10000_ranges_under_2ms`; `element_frozen.rs::deactivating_the_window_stops_the_autoscroll`; `results_view.rs::the_strip_counts_the_ranges_of_a_multi_range_selection`; `model_multirect.rs::contains_with_10000_rectangles_under_1ms_per_viewport` (integration I4). Go to row in `tr`: `goto_row_reads_locale_grouping`, cross-surface `go_to_row_1_000_in_turkish_reaches_view_row_1000_under_a_sort`. Context menu T39 (`5c9adf4`): `results_menu.rs` `menu_variants_and_disabled_reasons`, `raw_display_per_column_and_global`, `a_menu_over_an_old_result_closes_and_never_acts_on_the_new_one`, `right_press_under_sort_filter_and_transposed`; `hacivat-ui-grid/tests/m3b_seams.rs::a_right_press_asks_for_the_menu_over_what_it_hits`. AccessKit T40 (`5b74236`): `hacivat-ui-grid/tests/a11y_rows.rs` `visible_nodes_for_a_40_by_20_viewport_has_861_nodes`, `ids_are_stable_across_scroll`, `nothing_is_built_without_at`, `tab_leaves_from_any_cell`, `held_arrows_announce_once_after_the_last_move`, `node_build_under_1ms_per_frame` (0.108 ms); `transpose.rs::a_transposed_cell_is_spoken_as_its_table_cell`; cross-surface `grid_a11y_rows_follow_filter_and_sort` |
| E4 | Trust signals whole to the 120 px split floor in every locale | **PASS (automated)**; demo step 13 **PENDING-PO**; F10/F11/F12 fixed in the product (`m3c-narrow-fixes` `73bc4f1` + fix round 1 `8cf67e0`, incl. a narrow-and-short pane keeping the Reduced-mode title whole and clipped: `narrow_panes.rs`, 4 tests) | T41 (`e1254ff`, review: Approved 0 C / 0 I; 1 px sweeps 360→120 in en, tr, en-XA, tr-XT, every state, 0 failures): `hacivat-app/tests/toolbar_narrow.rs` `trust_items_whole_down_to_120px_in_every_locale`, `the_strips_cursor_open_stays_whole_in_a_narrow_pane`, `copy_gives_way_last_in_a_narrow_result_toolbar`, `the_narrow_lock_is_red_on_production`; `status_follow.rs::reduced_mode_never_gives_way`; `hacivat-ui/tests/toolbar.rs::a_narrow_form_is_taken_only_below_where_every_item_fits_whole`. Cross-surface (TR, production, 120 px pane): `a_production_console_in_a_120px_pane_runs_with_its_trust_signals_whole`. Shots: the T41 surface's 12 `narrow-*` states × 16 with the structural trust guard (a known finding can never cover a trust id). Capsule: `a_read_write_word_gives_way_last_before_the_name` (shot fixes `de24b57`) |
| E5 | **B1-real** ≥ 55 fps, no gap > 100 ms, paint figures recorded; **B3** ≤ 16 ms p95 in every mode; R-M3-9 ≤ 2 ms | **PENDING-PO** (every graded cell) | Section 6. Harness: T44 `m3c-t44-bench` (`a72845d`, `eaaac31`, `0734d98`, fix rounds 1–2), merged into `m3c` as `c19bbfc`. Commands and the PENDING-PO table: `docs/benchmarks/m3c-bench-notes.md`. Not gating, controller machine: relex p95 0.30 ms (`sql-lexer`), 0.59 ms (`sql-split`); 19 MiB typing p95 0.34 ms (editor level) |
| E6 | Shots 0 violations; only listed changes against the three baselines; DESIGN.md section approved | **PASS (harness)** at the gate tip: both known-findings lists empty, `HACIVAT_T45_GATE=1` 13/0; one unsharded render of all 4,860 PNGs, 0 violations; the three shot-diffs fully attributed (section 5.2). PO: earlier sets approved (P2-13); the 176 `narrow-*` PNGs (P2-16) and DESIGN.md **PENDING-PO** | Section 5. Self-checks: `crates/app/hacivat-app/tests/shots.rs` (11 + 1 ignored render) incl. `every_m3c_state_has_its_own_marker` and `known_findings_are_empty_at_the_t45_gate` (`HACIVAT_T45_GATE=1`). DESIGN.md "Query, Results and Grid" (T43a `223b71b`, T43b `b540c4c`) |
| E7 | Gates green (as M3b E11): clippy, the full test set with perf lines, check-deps, l10n, tokens, the three generators, `cargo deny`, `aws-lc-rs` absent | **PASS (local)** at the gate tip `83edd55` (section 2); CI on the gate tip: section 3 | Section 2: clippy plain and `--all-features` clean, workspace 5,467/0, app 1,002/0, perf lines 0 failed, check-deps, l10n, tokens, the three generators, `cargo deny`, `aws-lc-rs` absent, Docker full set 193/0, `--smoke` ×3 |

Summary at the gate tip `83edd55`: E1–E4 pass their automated halves; their demos are the PO's. E5
waits for the PO's Mac. E6 passes the harness and waits for the PO's sign-off of the narrow PNGs.
E7 passes locally; CI on the gate tip is in section 3. No criterion FAILS.

## 2. Step 2 gates (local)

<!-- ## m3c-t45 (controller, gate tip) --> Run on `m3c` 2026-11-18, each line on its own (`--no-fail-fast` where a
line has many binaries). Lines 1, 3, 5–7, 9–11 and 13–14 ran at `ff7e4fe`, after Step 1 cleaned every workspace member (25.7 GiB). Lines 2 and
4 failed to build there: `narrow_panes.rs` called `rendered_scene()` without `scene-check`. The
controller fixed that in `83edd55` (the file is `#![cfg(feature = "scene-check")]`; test code
only), then re-ran lines 2, 4 and 12 at `83edd55`. The full render (line 8) ran at `100e4e6`.
After it, only `ff7e4fe` (P2-07: the gate and classifier, no rendered state) and `83edd55`
(test gating) changed.

| # | Command | Result |
|---|---|---|
| 1 | Step 1: `cargo clean -p <every workspace member>` in the merge target | removed 10,566 files, 25.7 GiB |
| 2 | `cargo clippy --workspace --all-targets --locked -- -D warnings` | clean (at `83edd55`) |
| 3 | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | clean |
| 4 | `cargo test --workspace --locked --no-fail-fast` | **5,467 passed, 0 failed**, 238 ignored (Docker and perf), 394 result blocks (at `83edd55`) |
| 5 | `cargo test -p hacivat-app --features scene-check,pseudo --no-fail-fast` | **1,002/0**, 6 ignored |
| 6 | the other feature lines | ui-core `test-support,scene-check` 84/0; l10n `pseudo` 152/0; driver `fake` 178/0; testkit `ssh` 26/0 (1 ignored); ui-editor `scene-check` 272/0 (3 ignored); ui-grid `test-support,scene-check` 251/0 (4 ignored); release l10n `pseudo` 155/0; xtask `repo_catalogs_are_clean` 1/0 |
| 7 | `cargo test -p hacivat-app --features shots,scene-check,pseudo --test shots` with `HACIVAT_T45_GATE=1` | **13/0**, 1 ignored (the render): both known-findings lists empty |
| 8 | Full shot render (4,860 PNGs) + `shot-diff` vs `baseline-m3b`, `baseline-m3a`, `baseline-m2b` | 4,860 rendered in one process, 4,208 s, **0 violations**; live threads 4 → 5 over the run (the shots thread fix, section 3 note). Shot-diffs: section 5.2 |
| 9 | Perf lines (`HACIVAT_PERF=1 … --release … -- --ignored`), incl. `find_bar`, `results_sort`, `results_find`, `results_model`, `model_multirect`, `a11y_rows` | lexer/split/oracle/text/values/results/sql-editor/ui-editor/ui-grid 32/0; sql-safety `classify`+`read_corpus` 2/0; exec `classify_pool`+`audit`+`service_m3b` 2/0; driver-sqlite `typed` 1/0; app `results_model`+`find_bar`+`results_sort`+`results_find` 5/0 (machine not fully quiet: a shot-diff ran alongside) |
| 10 | `cargo xtask check-deps`, `l10n check`, `tokens --check`, `gen-sql-tables --check`, `gen-fn-catalog --check`, `gen-pg-visitor --check`, `docker check-pins` | all ok; l10n 0 errors (the known L025 warning) |
| 11 | `cargo deny --all-features check licenses bans sources`; `cargo tree -i aws-lc-rs --workspace --all-features` | bans ok, licenses ok, sources ok; `aws-lc-rs` matches no package |
| 12 | Docker full set through the lock wrapper (stack `hacivat-m2a` as it is) | **193 passed, 0 failed**, 430 s (at `83edd55`) |
| 13 | `cargo build --release -p hacivat`; `--smoke` ×3 (first frame ≤ 500 ms, idle RSS ≤ 200 MB) | 67,046,112 bytes (M3b 62,786,160: +4.1 MiB); first frame 302 / 281 / 229 ms, idle RSS 122 / 122 / 121 MiB; also `--features bench --examples` built, bench lib 49/0 |
| 14 | `cargo fmt --all -- --check` | only `spikes/spike-harman` (not ours, as M3b) |
| 15 | `git status --porcelain --ignored -- third_party/pg_query` | empty |

## 3. CI

| Field | Value |
|---|---|
| Run | **44333377438** (`workflow_dispatch` on `m3c` at `5535724`: the gate tip `83edd55` plus the three CI fixes below) |
| Status | **success** (every job green). `launch-smoke (macos-15)` and `switch (macos-15)` were first not picked up by a hosted macOS runner (GitHub capacity, "not acquired") and passed on the re-run of the same run |
| build-test ×3 (macOS, Ubuntu, Windows) | success |
| turkey-test | success (flake `exec_ui.rs:544` fixed, see below) |
| tokens, check-deps, l10n-check, cargo-deny | success |
| switch (ubuntu), launch-smoke (ubuntu, windows) | success |
| switch (macos), launch-smoke (macos) | success (re-run attempt 2 of the same run, same commit) |
| release-tests | success |
| bench (regression signal only) | success |
| nightly-package ×3 | success |
| pseudo-screens macOS (one process) | success: 4,860 states, 0 violations; live threads 4 after the first shot, 6 after the last; M1 shot-diff 36 compared, 0 breached, 32 superseded |
| pseudo-screens Linux (2 shards) | success: 1/2 and 2/2 (the first full Linux render of the M3c set) |
| docker-full | success |
| keyring-linux, keyring-windows, vault-linux | success |
| fuzz-sql, sql-differential | success |
| docker-pr | not in this run (runs with the `m3c` → `main` PR) |

Earlier runs, superseded:
- **44152258028** (`b540c4c`): before T44 and the narrow fixes.
- **44274413727** (`83edd55`): three reds, all fixed on `m3c`:
  1. **pseudo-screens macOS**, 12 violations in `marks-denied`. The P2-07 several-targets denial
     had grown to 152 characters, and an output-log sentence stays on one line. The sentence
     is back to the old length (98 characters, same meaning); fix `feca818`. The 16
     `marks-denied` PNGs change in text only (PO item P2-17).
     - This corrects the ledger note that P2-07 changes no shot: the local full render ran
       at `100e4e6`, before P2-07.
  2. **pseudo-screens Linux** cancelled at the 180-minute bound (about 2.5 s per shot). Linux
     now renders in two shards (`--shard k/n`); fix `feca818`.
  3. **turkey-test**, `exec_ui.rs:544`, the second gate failure, so no longer a flake:
     - `FOR UPDATE` is a lock, so the unit never suspends, and the 5 s fake stream could end
       before Escape on a slow runner.
     - The test-only fix (`0f963f5`, merged `c9e9c30`) uses a stream with no reachable end
       and asserts no cancel on Escape and no rows after the stop.
- **44306265834** (`c9e9c30`): all of the above green. The macOS render passed for the first time
  and reached the M1 pixel comparison:
  - 8 `settings-general` breaches, which T42 changed on purpose (PO P2-13). They were added to
    `superseded.txt` (`5535724`).
  - launch-smoke macOS was not acquired by a runner.

## 4. What the user sees that the plan wording does not say

- **Sort by Turkish collation.** E3 names "sort with Turkish collation". In the app every text sort
  is by **code point**, with the header tooltip "…The column's collation is not known here, so
  text compares by code point." / "…harmanlaması burada bilinmiyor; metin kod noktasına göre
  karşılaştırılır." (`hv-grid-sort-tip-unknown`). The reason: PostgreSQL's RowDescription carries
  no collation and MySQL's column charset is the result charset, so `collation_of` returns None.
  The comparators for `tr-x-icu`, glibc `tr_TR` and MySQL `_turkish_` exist and are tested
  (`turkish_collation_sorts_ilik_irmak_istek_izmir`; perf 1M rows `tr-x-icu` 805 ms) but are not
  reachable from the UI. PO approved this for M3c (M3c-06); the catalog lookup is M5 metadata.
  Turkish tolerance applies to **find** (T33, T37), not to sort.
- **A header click selects the column**, it does not sort (PO M3c-07). Sort is in the header/cell
  menu and the palette.
- **Re-run with a larger value limit** runs only statements that classify as Read (T38 I1, fail
  closed; plan deviation; PO M3c-01).

## 5. Shots (E6)

### 5.1 Matrix and known findings

| Stage | M3c states | M3c PNGs | Whole matrix |
|---|---|---|---|
| Before M3c shots (M1–M3b) | — | — | 3,244 |
| T43a (`223b71b`) | 67 | 1,072 | 4,316 |
| T43b + fix (`b540c4c`) | 101 | 1,616 | **4,860** (the test asserts `101 * 16`) |

- macOS renders: 0 violations outside the known-findings lists (T43b fix: one unsharded render of
  all 1,616 M3c shots; 14 stale entries removed; F12 found and listed).
- `KNOWN_FINDINGS` (part A, F1–F8): **empty** since the shot fixes (`de24b57`).
- `KNOWN_FINDINGS_B` at `b540c4c`: **4 entries**, none on a trust id (the guard refuses that):

  | # | State, scope, label | Fix (on `m3c-narrow-fixes`) | Status |
  |---|---|---|---|
  | F10 | `narrow-empty`, pseudo-locales, `#editor-placeholder` | wrap the empty console's placeholder in a narrow pane | **Fixed** (`73bc4f1`) |
  | F11 | `narrow-reduced`, `#editor-t1-title` (pseudo) | the Reduced-mode banner wraps/grows or scrolls at 120 px | **Fixed** (`73bc4f1`; narrow-and-short pane `8cf67e0`) |
  | F11 | `narrow-reduced`, `#editor-t1-body` (en, tr) | same | **Fixed** (`73bc4f1`) |
  | F12 | `narrow-reduced`, pseudo, `hacivat-ui/src/panes.rs:443` (tab title, no id) | ellipsize the pane tab title | **Fixed** (`73bc4f1`; title id `tab-title`) |

- `HACIVAT_T45_GATE=1` makes `known_findings_are_empty_at_the_t45_gate` fail while either list is
  non-empty. Gate rule (controller ruling C-26): **both lists empty at the gate.** Both lists are
  empty at the gate tip and the self-check passes with `HACIVAT_T45_GATE=1` (13/0).
- Linux: zero-violation claims above are macOS. Linux pseudo-screens on the new states run in CI
  (section 3). The first CI run on M3c died at shot 3,314: the shots harness built a GPUI platform
  per shot only for its text system, and on Linux that platform is never freed (a calloop `Rc`
  cycle in the fork), so about 5 dispatcher threads leaked per shot until thread creation failed.
  Fixed in the harness (`m3c-shots-threads` `14d4aee`: one platform text system per run; a
  live-thread guard fails a run whose threads keep growing). Product code is not affected (one
  platform per process).
- Determinism: the capsule's fit memo flake is gone (`settle_capsule` removed; 11 shots × 8 renders
  byte-identical).

### 5.2 PNG changes against the approved baselines (PO sign-off)

Baselines live in the SDD folder (`baseline-m3b/` 3,244, `baseline-m3a/`, `baseline-m2b/`); they
are not committed to git (controller ruling, section 7 C-37). Every set below is listed in a file
in the SDD folder.

| Set | PNGs | Cause | List |
|---|---|---|---|
| T41 chrome | **2,532** of 3,244 | Run play glyph, status-bar group and order, editor placeholder, disabled ink, menu active row; fix round 1 moved 38 (menu half-pixel snap and the M3c-18 order) | `task-m3c-41-shot-diff.txt` (2,532 lines + header), `task-m3c-41-fix1-shot-diff.txt` (38) |
| T38 views | **460** | the result toolbar's right group (view switch and record toggle), 29 surfaces | `task-m3c-38-report.md` § Shots (surface list) |
| T34 editor | **32** (16 `context-menu-*`, 16 `preview-as-one-*`) | line commands in the editor menu; "as one · n statements" label | `task-m3c-34-report.md` |
| Capsule fit (title bar, 960 wide) | **444** | the read-write word gives way last; "şifrelenmemiş" collapses to its glyph; deterministic | `m3b-capsule-fit-changed-shots.txt` = `m3c-shot-fixes-changed-approved.txt` (identical, 444 lines; en 14, tr 142, en-XA 144, tr-XT 144) |
| T42 settings | **8** (`settings-general-*`) | nav shows 7 sections; titles wrap | `task-m3c-42-report.md` |
| m3b PO fixes | **168** + **108 TR** | M3b-04 typed name, M3b-26 scope line (168); "rollback et-" wording and English ROLLBACK in TR (108) | `m3b-po-fixes-report.md` § Shots (both lists by surface) |
| tx-chip menus | **6** (`tx-chip-menu` en-XA ×2, `tx-chip-production` en-XA/tr-XT ×4, 960) | the transaction menu now ends 4 px inside the window (it was cut at the edge) | `m3b.md` section 12.4; `m3c-shot-fixes-report.md` |
| vs `baseline-m2b` | the known 24 + 12 rw capsules at 960 (M3b) | as M3b | `task-m3b-29-shot-diff.md` |
| Narrow fixes (new, P2-16) | **176** new PNGs (the 11 `narrow-*` states at 120 px × 16) | ellipsized active tab title, the mode chip's +1 px sides, the placeholder under line 1, the Reduced-mode banner's narrow form; `narrow-pseudo-floor` and settings unchanged | `m3c-narrow-fixes-report.md` |
| P2-07 denial (new, P2-17) | **16** (`marks-denied-*`, found by CI 44274413727) | the several-targets denial sentence: P2-07's one-name-one-target wording, cut back to the old length (`feca818`); text only; 0 violations | section 3 |
| Full diffs at the gate tip (default thresholds, max dE 2, share 0.001) | vs `baseline-m3b`: 3,244 compared, **1,883** breached, 1,616 new (the M3c states). vs `baseline-m3a`: 2,108 compared, 1,161 breached. vs `baseline-m2b`: 1,012 compared, 85 breached | vs m3b: 1,837 in the T41/capsule/shot-fix lists, the other 46 are 38 m3b-PO-fix and 8 T42 `settings-general` PNGs: **0 unattributed**; the narrow fixes' render showed the same 1,883 dE-0 identical to `b540c4c`. The 20 (m3a) and 12 (m2b) breaches not breached vs m3b are M3b changes attributed at the M3b gate | `t45-gate-shot-diff-vs-m3b.txt`, `-vs-m3a.txt`, `-vs-m2b.txt`, `t45-gate-m3b-breaches-outside-shot-lists.txt` |

The sets overlap (a capsule PNG can also carry the T41 glyph). The controller's full diff at the
gate tip is the record; any PNG not explained by a set above is a finding. After sign-off, the
controller renders `baseline-m3c/` in the SDD folder from the gate tip.

**Plan vs ruling:** the T45 plan lists `design/baselines/m3b/**` as a file of this task. The
controller's T37 Step 6 ruling keeps the M2–M3 baselines out of git (≈1.2 GB) until M9. This
record follows the ruling: nothing is committed under `design/baselines/` (PO confirm, C-37).

## 6. Performance (E5)

### 6.1 Graded figures — PENDING-PO

Commands, thresholds and refusal rules: `docs/benchmarks/m3c-bench-notes.md` (sections 1–4). A run
off the reference conditions is INVALID (exit 3, no JSON), never FAIL. B3 needs ≥ 118 Hz (any scale)
and the `tr-TR` format; B1-real needs 1440 × 900 on the built-in panel at 2× ≥ 118 Hz, or
2560 × 1440 on a 1× display.

Build: PENDING-PO · Date: PENDING-PO · Machine: the PO's Mac

| Bench | Display (Hz, scale) | p50 | p95 | p99 | max | Samples | Verdict |
|---|---|---|---|---|---|---|---|
| `b3 typing` | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO |
| `b3 typing-long-line` (Reduced) | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO |
| `b3 typing-with-results` (1M × 50 open) | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO |
| `b3 typing-multi-caret` (10 carets) | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO |
| `b3 typing-with-folds` (1,000 folds) | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO |
| `b3 typing-19mb` (Reduced; ledger 298) | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO |
| R-M3-9 worker relex (in `b3 typing`; budget 2 ms p95) | — | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO |

| Bench | Display | mean fps (≥ 55) | gaps > 100 ms (0) | frames | grid paint p50 / p95 / p99 / max | > 3 ms / > 6 ms | Verdict |
|---|---|---|---|---|---|---|---|
| `b1_real` 1440 × 900 | 2×, ≥ 118 Hz | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO |
| `b1_real --size 2560x1440` | 1× | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO |
| `b1_real --sorted` (not gating; sort time) | 2×, ≥ 118 Hz | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | n/a |

Ungraded, recorded for the PO (T44 review M7): `b3 open`, `b3 scroll`, `b3 ime-bounds` — PENDING-PO.
Known and ungraded: the long-line worker relex (expected 5–8 ms p95, outside R-M3-9).

Not gating (controller machine, under load, no display; from the bench notes): `results_sort` 1M
rows int 103 ms / `tr-x-icu` 947 ms / code point 185 ms; `results_find` scan 152 ms; `sql-lexer`
relex p95 0.30 ms; `sql-split` relex + resplit p95 0.59 ms; 19 MiB typing (editor level) p95 0.34 ms;
grid a11y node build 0.108 ms/frame; 10k carets 8.5 ms; 3,000-fold keystroke 40–64 µs.

### 6.2 How the PO's rows reach this file

`cargo xtask bench-report --milestone m3c target/bench/b1-fling-real-*.json target/bench/b3-typing*-app-*.json`
appends rows after the **last table row in this file**, so section "Bench rows" stays the last
table. Before the runs, move `target/bench/` aside so older M3a/M3b dumps are not picked up (T44
review M6). The generated rows carry a `host` column: replace it with "PO's Mac" before committing
(no hostnames in committed files).

## 7. Rulings for PO confirmation

### 7.1 Controller rulings taken during M3c

"PO confirm": **Onay (M3c-nn)** = approved in the PO's first list (`po-answers-m3-20261117.md`);
**P2-nn** = on PO page 2, awaiting; **PENDING-PO** = not yet asked; **Done (M3c-nn)** = the PO asked
for a change and it shipped.

| # | Task | Ruling | PO confirm |
|---|---|---|---|
| C-01 | controller | M3c proceeds while the M3b gate is PO-pending; `m3c` cut from `m3b` at `1894b7d` (M2b→M3 precedent); `m3b` merged forward repeatedly (last `9ee9f21`) | PENDING-PO |
| C-02 | controller | gpui slots: 2 while disk < 60 GiB (P-M3b-31), and 2 in practice even above (≈20 GiB per gpui target); later build targets moved to an external disk; new builds with `CARGO_PROFILE_*_DEBUG=0` | PENDING-PO (process) |
| C-03 | T31 | Implementer rulings 1–8 accepted: column box puts a caret at a short line's end (NOTES 14); executable comments count as comments; icons carry the repo NOTICE changes; selection history only on selection-only commits; N foreign lines into N carets one per caret; Option-click / Option-drag (Alt-click / Ctrl+Alt-drag elsewhere); no announcement on a next-occurrence miss (→ T33); split into lines gives a plain caret | Onay (M3c-12) for ruling 1; rest PENDING-PO |
| C-04 | T31 | Run follows the **main** caret only | Onay (M3c-02) |
| C-05 | T31 fix | column drag starts only after the pointer reaches another cell; middle-click text through background `prepare`, synchronous primary read kept (async read → M9 fork carry); all line-wise pieces paste line-wise; unknown/stale analysis = not plain (main-caret fallback); one history entry per gesture; restore carets on cancel; whole word only when started from a caret | PENDING-PO |
| C-06 | T32 | Implementer rulings 1–10 accepted (line model; deleting the header unfolds; text at a fold's start goes above it, at its end stays outside; 3-line minimum and `--` runs fold; vertical moves step over a fold; folding moves carets to the header end; "⋯" as the ellipsis icon) | Onay (M3c-13) for ruling 2; rest PENDING-PO |
| C-07 | T32 | T34 adds `ExecMarkState::{Awaiting, RolledBack}`; Awaiting ranks first on a fold header | PENDING-PO |
| C-08 | T32 review | INSERT VALUES rows and `DELIMITER … BEGIN…END` folding → M5 lexer | PENDING-PO |
| C-09 | T35 | Implementer rulings 1–13 accepted; ruling 2 changed: a "row segment" for grow/shrink is the pane holding the focus (frozen or scrolling), independent of scroll | Onay (M3c-22) |
| C-10 | T35 | Auto-fit keeps the outlier trim; strip says "N ranges · M cells"; resize tag has no middle dot | Onay (M3c-15, -27, -28) |
| C-11 | T35 | `set_view_map` generation guard → T36; B1 frame p99 re-measured on a quiet machine at T44 | PENDING-PO |
| C-12 | T33 | No rope-search crate (flatten per version off-thread); scrollbar track stays 8 px, no vertical thumb; Escape returns the caret only while find's selection stands; Select all from the bar focuses the editor and keeps the bar; ticks only when the document is taller than the view; three extra ids; `hacivat-settings` key `editor.find_turkish_tolerant` as a named exception | Onay (M3c-14, -26) |
| C-13 | T33 | No ⌘E (use the macOS find pasteboard) — **reversed by the PO**: ⌘E bound | Done (M3c-08) |
| C-14 | T33 fix | Touching matches merge into one selection (as T31) | Onay (M3c-21) |
| C-15 | T36 R1 | Text sorts by code point with the "collation unknown" tooltip on every engine in M3c; catalog collation lookup → M5 metadata | Onay (M3c-06) |
| C-16 | T36 | Header click still selects the column (spec: cycles sort); ties keep model order; 32 B per row per key (budget 16 B); a sort before the new rows are painted is refused; worker panic → `hv-grid-sort-failed` toast; PAD SPACE collations honoured | Onay (M3c-07, -25); rest PENDING-PO |
| C-17 | T37 | Loose digit grouping accepted (live line shows the target); column by position = server order | Onay (M3c-23) |
| C-18 | T37 | DataGrip grid/editor Select-all chord inconsistency → PO; **fixed**: both `ctrl-alt-shift-j` | Done (M3c-10) |
| C-19 | T34 | Implementer rulings 1–12 accepted (comment toggle at least indentation; duplicate onto the copy; ▶ glyph 12 px; recall newest 500, no prefix; EOL menu offers LF/CRLF/CR; …); move re-folds with `FoldSet::fold`; `SystemTime::now` for "Last runs" | Onay (M3c-24, -34) |
| C-20 | T34 | All ▶ as tab stops → PO; **changed**: ▶ column is one Tab stop walked with ↑/↓, Escape returns | Done (M3c-04) |
| C-21 | T34 | DataGrip ⌘⌫ deletes the line (preset follows DataGrip) | Onay (M3c-11) |
| C-22 | T40 | `hui-` prefixed ids; header row counted; a loading row has no cells; a multi-cell selection announces a summary; a re-sort is silent (m1); find/go-to keep their own nodes; `grid.next_cell`/`prev_cell` hidden from the palette (`PALETTE_HIDDEN`) | Onay (M3c-36); rest PENDING-PO |
| C-23 | T38 I1 | **Fail closed:** "Re-run with a larger value limit" runs only if every unit classifies as Read (plan deviation) | Onay (M3c-01) |
| C-24 | T38 | Find, Filter rows and go to row are off in Transposed and Text views; the hidden table grid renders at size 0 (a11y-hidden); record keys are element-level (no Inspector keymap row); transposed name column shows no type | Onay (M3c-20, -29) |
| C-25 | T39 | A right press selects first; the menu acts on the clicked column; raw display keyed by column name per console and overrides the setting until cleared (no Reset row); no sort/go-to in Transposed/Text; Show as record only opens | P2-10 |
| C-26 | T43a | `KNOWN_FINDINGS` allowed only as a temporary list; **empty at the T45 gate**, enforced by `HACIVAT_T45_GATE=1`; a finding may never cover a trust id | PENDING-PO |
| C-27 | T43a fix | `viewer-truncated` temporarily required `#value-viewer-sub` until F8 was fixed (restored since) | PENDING-PO (closed) |
| C-28 | T41 R1 / N1′ | Narrow Cancel draws its word wherever it fits (always in en/tr); only pseudo-locales at the 120 px floor draw × with the name as tooltip and accessible name | PENDING-PO |
| C-29 | T41 R2 / N5 | Narrow Running = marker + clock; "Running" only in tooltip and accessible name; "Cancelling…" keeps its word | PENDING-PO |
| C-30 | T41 | NOTES 24 N1–N4 (Run/Stop icon-only named "Run · ⌘↵"; connection name ellipsizes, swatch never; badge drops its age, keeps Commit/Roll back as icon buttons; mode chip keeps its word); lock red on production | Onay (M3c-16, -33); rest PENDING-PO |
| C-31 | T41 / PO | Copy gives way last in the narrow result toolbar (`COPY_PRIORITY` 200, view items 100) | Done (M3c-18) |
| C-32 | shot fixes | **Menu placement:** keep T41's `anchored().snap_to_window_with_margin(4 px)` + window-room width cap (hints ellipsize, label never); drop the F1 spacer and m3b's `KeepInWindow`; take m3b's window-side check through the forward merge, with the ruled case "wide menu anchored at width − 60" | PENDING-PO |
| C-33 | shot fixes | Capsule order: the palette field shrinks to 170 px first, then latency, path, "unencrypted" (to its glyph), engine; the read-write word goes last before the name; trust words never clip | P2-01, P2-03 |
| C-34 | T43b | Merge order shot fixes → gutter-no-run → T43b fix on merged `m3c`; `settle_capsule` removed; `narrow-reduced` restored with F11 listed | PENDING-PO |
| C-35 | T42 | Guard rows (`exec.confirm_unsafe_where`, `exec.error_policy_ask`, `history.enabled`) move focus with arrows, never flip; re-enabling history opens the store; strict 3-digit grouping; **unit switch converts the saved size**; five extra ids; value field left-aligned | P2-09; rest PENDING-PO |
| C-36 | PO answers | M3c-09 read as "VS Code defaults, remappable": fold ⌥⌘[ / unfold ⌥⌘] (Ctrl+Shift+[ / ] elsewhere); ⇧⌥F reserved for M5 reformat; **fold all / unfold all not bound** (⌘K is the palette chord); fold chords unreachable on AltGr layouts (DE, FR, TR-Q, TR-F, PL): palette or remap | Done (M3c-09); fold all → P2-08 |
| C-37 | controller (M3a T37 Step 6, kept) | M2–M3 shot baselines stay in the SDD folder, not in git, until M9 (curated subset or LFS) | PENDING-PO |
| C-38 | T44 | Manifest `bench` feature lines as fenced named exceptions (M3a T36 precedent); B3 graded at ≥ 118 Hz with no upper bound at any scale, Hz and scale recorded; activation failure = INVALID, not FAIL; `bench-report` accepts `m3a`/`m3b`/`m3c`; 10 carets, 1,000 folds (5 lines every 50), `--sorted` by `c01`; R-M3-9 graded only in `typing` | PENDING-PO |
| C-39 | T44 review | `--allow-inactive` is ungraded by construction (I1, fix round 2); temp-folder race relatives → notes (section 8.2) | PENDING-PO |
| C-40 | controller | PO-present work goes last (T44 Step 2, the hand demo, VoiceOver, PNG sign-off, the main merge); the stop line is the M3 closure | PENDING-PO (PO's own instruction) |
| C-41 | m3b (during M3c) | params fuzz: names under `CALL`/`EXEC`/`EXECUTE` are refused (Unbindable) on MySQL/MariaDB/SQLite — fail closed (ADR 0005) | PENDING-PO |
| C-42 | m3b (during M3c) | Gate EXPLAIN: the plain-EXPLAIN shortcut runs only after the production AND-check and D-062; routine evidence never runs unasked on production | PENDING-PO |
| C-43 | m3b PO fixes | `TRUNCATE`/`DROP TABLE` typed name on production; "rollback et-" in TR; transaction/commit/rollback stay English in TR; `DROP OWNED BY` typed role name → PO | Onay (M3b-04, -28); P2-07 Onay → C-46 |
| C-44 | narrow fixes | F10/F11/F12 fixed in the product. In a pane that is narrow **and** short, the Reduced-mode title keeps its height and its box clips (a cut is reported, never hidden). The editor text's 2 lines give way before the title. Inactive tabs shrink to 0 px at 120 px (Ctrl+Tab reaches them) | PENDING-PO (P2-14, P2-15, P2-16) |
| C-45 | shots thread fix | The shots harness builds one platform text system per run. A live-thread guard fails a run whose threads keep growing. The gpui fork's Linux calloop `Rc` cycle is carried to M9 (fork issue) | PENDING-PO |
| C-46 | P2-07 | On production, `DROP OWNED BY` asks for the role typed as written. **One typed name approves exactly one destructive-target occurrence:** two or more in a run (even the same name twice, e.g. `DELETE FROM t; DELETE FROM t` or `TRUNCATE a, A`) are denied. The spelling is read at the occurrence's own position. `TRUNCATE … CASCADE` is named in the dialog. Implicit effects (cascades, children, triggers, rules, views) are not counted, and a single unqualified name follows the session's resolution (ADR 0006 `## m3b-po`). Found while doing it, then closed: one typed name could approve two objects (`TRUNCATE "A", A`); 4 review rounds | PENDING-PO (stricter than M3b: repeat-target runs on production are now denied) |
| C-47 | controller | P2-08 rejected by the PO: fold all / unfold all stay without chords (no ⌘K second-key chord); palette only | Done (PO 2026-11-18) |

### 7.2 PO page 2

https://claude.ai/artifact/R7xAfM1fWxoFe34aZuaAUD, `po-items-m3-2.json`. Answered 2026-11-18:
P2-01 capsule order, P2-02 (M9), P2-03, P2-04 (small job after M3c), P2-05, P2-06, P2-07 (shipped,
C-46), P2-09, P2-10, P2-11, P2-12 (container recreate before M4 W3), P2-13 (shot sets) **Onay**;
P2-08 fold-all chords **Ret** (C-47). Awaiting: **P2-14** zero-width inactive tabs, **P2-15** the
narrow placeholder crossing the gutter, **P2-16** the 176 `narrow-*` PNGs, **P2-17** the 16
`marks-denied` PNGs.

## 8. Deferred

### 8.1 Deferred table

| Item | Source | Target |
|---|---|---|
| `hold_counts` is always compiled and `pub` in `hacivat-ui-editor`: gate it behind `test-support` at the next manifest window, or add a grep check that no `src/` path calls it | T43b fix m4 | next manifest window (M4 W0) |
| `settings_ui::fixtures::show_section` duplicates the navigation click handler: extract `SettingsPage::show_section` | T43b fix m5 | next touch of T42's file (M4/M5) |
| Shot states not yet rendered: "long name ellipsized, mark kept" (NOTES 24) and NOTES 22 "Re-run" over a live result with a truncated value (need new fixtures) | T43b fix m10 | M4 shots task |
| ActiveTab is app-wide: two workspace windows ping-pong `mirror_active_tab` → `note_active` → `refresh_windows` forever (test-only today: one `open_workspace` call site) | narrow fix round 1 | before any multi-window work |
| gpui fork: `LinuxPlatform` never frees (calloop `Rc` cycle), leaking its dispatcher threads per platform | shots thread fix | M9 (fork issue) |
| P2-14: a "»" overflow for zero-width inactive tabs | narrow review M8 | M9 polish |
| Temp-folder race relatives: `hacivat-session/tests/docker_readonly.rs:175` `unique()` (nanos only); `hacivat-bench` `write_dump` (seconds only); pid-only folders in `hacivat-bench/src/lib.rs:587`, `hacivat-ui-core` `text_rendering.rs:122`, `hacivat` bin `logging.rs:170`, `hacivat-driver/tests/sanitize.rs:314`, `hacivat-driver-mysql/tests/unit.rs:468`; fixed `hv-exec-test.db` in `hacivat-exec/tests/common/mod.rs:20` | T44 review (flake fix `eaaac31`) | M9 hardening (test hygiene) |
| P2-02: in en-XA/tr-XT production at 960 the engine drops though the read-write word still does not fit; drop the engine only when that makes the word fit | m3b-capsule-fit | M9 polish (PO rec. "Sonra") |
| P2-04: palette/search field collapses to its icon before the capsule drops a trust word | shot-fixes review m9 | small job after M3c closes (PO rec.) |
| Catalog collation for local sort (`pg_attribute.attcollation`, MySQL `COLUMNS.COLLATION_NAME`) | T36 R1 | M5 metadata |
| INSERT VALUES rows and `DELIMITER` procedure blocks as fold ranges | T32 M6 | M5 lexer |
| Async Linux primary-selection read (fork) | T31 I2 | M9 |
| Strip narrow forms ("Fetch more" as an icon) | T41 | M4 (results owner) |
| T41 m7/m8: connection dialog Sessions/Advanced labels at 960 pseudo; `toolbar-production-waiting` 3 lines at 960 tr-XT | T41, T29 | M9 polish |
| Find marks and Filter rows go stale after a raw-display toggle | T39 | M4 (grid owner) |
| Transposed build at 100k rows on the UI thread | T38 | T44 measure, then M6 |
| Capsule frame-flip rendering test; `CHROME_PX` hand-kept; menu cap ignores `client_inset` | shot fixes m4, m-e, m-h | M9 |
| `dev-run.sh` codesign race when shards sign at once | T32, T43 | M9 packaging |
| Unrowed setting `ui.time_zone` | T42 | PO decision, then M4 |
| Allowlisted builtins under EXPLAIN ask on production | gate EXPLAIN review M-1 | M5 |
| VoiceOver / NVDA walkthroughs that gate 1.0 (L4); a fork patch for nested synthetic nodes only if VoiceOver fails | T40, R-M3-69 | M9 |
| Windows and Linux IME rows; clipboard metadata on those hosts | R-M3-7 | M9 (hosts) |
| M3b deferred items still open (classifier 1 ms / 10 KB, `StreamSplitter`, …) | `m3b.md` § 9.2 | as listed there |

### 8.2 Open questions for the PO

1. Accept that E3's "sort with Turkish collation" ships as code-point order with an honest tooltip
   (M3c-06 says yes) — and is M5 the right home for the catalog lookup?
2. Baselines: keep them out of git until M9 (C-37), against the plan's `design/baselines/m3b/**`?
3. The fold chords cannot be typed on AltGr layouts (Turkish Q/F included): is the palette plus
   remapping enough for M3c?

## 9. VoiceOver smoke (non-gating)

| Check | Gating | Status |
|---|---|---|
| VoiceOver on the result grid: the nine T40 items (demo step 14), plus the gutter ▶/chevron buttons and the value viewer/inspector | no (L4 gates at M9) | **PO: "Sonra"** (M3c-03). Optional in this demo. The outcome decides whether R-M3-69 needs a fork patch; until then no patch is made |

## 10. Controller work before the PO arrives (done)

1. T44, the narrow fixes (+ fix round 1), the shots thread fix and P2-07 are merged into `m3c`
   (gate tip `83edd55`, pushed).
2. Section 2 lines ran on the gate tip, along with the full render and the three shot-diffs
   (sections 2 and 5.2).
3. CI on the gate tip is recorded in section 3.
4. The Docker stack `hacivat-m2a` stays up for the demo; no container was recreated.

## Bench rows

`cargo xtask bench-report --milestone m3c <dumps>` adds rows after the last table row anywhere in
this file, so this table stays the file's last table. Empty until the PO's panel runs. Replace the
`host` cell with "PO's Mac" before committing.

| label | OS | host | build | display | frames | mean fps | p95 ms | p99 ms | max gap ms | gaps > 100 ms | verdict |
|---|---|---|---|---|---|---|---|---|---|---|---|
