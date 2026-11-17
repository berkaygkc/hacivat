# M3c benchmarks: how the PO runs them (Task 44)

Task 44 Step 1 adds the M3c modes to the real-app benches (feature `bench`, never in a product
build). Step 2 is the PO's: the graded runs happen on the PO's Mac with the display awake and the
session unlocked. Until then every graded cell below is **PENDING-PO**. The figures in section 5.2
come from the controller machine and are **not gating**.

## 1. What each figure gates (M3c gate, Task 45 E5)

| Figure | Gate | Threshold | Where it is printed |
|---|---|---|---|
| B1-real, 1M × 50 from `pg17`, `tr-TR`, 1440 × 900 at 2× (≥ 118 Hz) | E5 | mean ≥ 55 fps, no frame gap > 100 ms, at least 600 frames recorded (`B1_MIN_FRAMES`), grid painted on ≥ 95 % of them | `RESULT:` line, `verdict=` line |
| B1-real, the same at 2560 × 1440 on a 1× display | E5 | the same | the same |
| B1-real grid paint p50/p95/p99/max, frames > 3 ms and > 6 ms | E5 ("paint figures recorded") | recorded, not gating | `GRID PAINT (the gate's paint figure)` line; `target/bench/load/` |
| B1-real `--sorted` (whole result sorted by `c01`, then the fling) | none | not gating (`b1-sorted-real-*`) | the same lines, plus the `sort:` line (the sort's time) |
| B3 `typing`, `typing-long-line`, `typing-with-results`, `typing-multi-caret`, `typing-with-folds`, `typing-19mb` | E5 | p95 ≤ 16 ms in **every** mode, with ≥ 95 % of the 2,000 keystrokes measured | `RESULT:` line; `p50/p95/p99/max` line |
| R-M3-9: incremental relex in the real app's worker | E5 | p95 ≤ 2 ms over at least 500 incremental analyses (graded in `b3 --bench typing`) | `R-M3-9 worker relex` line; `target/bench/relex/` |
| 19 MB typing in release (ledger 298) | E5 (B3 in every mode) | p95 ≤ 16 ms | `b3 --bench typing-19mb`; also the editor-level perf test (section 5.2) |
| B3 `open`, `scroll`, `ime-bounds` | none | their own budgets, printed (`open`: first paint ≤ 150 ms, analysis ≤ 600 ms; `ime-bounds`: ≤ 1 ms p95) | `RESULT:` line; the mode's line |

A run is graded only under the reference conditions. When it cannot measure, it is **INVALID**
(exit 3, no JSON written) with a `PENDING-PO` note. INVALID never means FAIL.

| Condition | B3 | B1-real |
|---|---|---|
| Display | ≥ 118 Hz, any scale, no upper bound (B1-real's `PANEL_MIN_HZ`; the PO's M3a B3 ran on a 120 Hz 1× display, the M3b gate's on 240 Hz 1×) | 1440 × 900: the built-in panel at 2×, ≥ 118 Hz; 2560 × 1440: a 1× display; the viewport ≥ 90 % of the request |
| Display under the rule, or reporting no refresh (asleep) | INVALID; with `--any-display`, ungraded (`b3-offpanel-…`) | INVALID; with `--any-display`, ungraded (`b1-offpanel-real-…`) |
| Display checked again at the end of the run | yes (graded runs) | yes |
| Format the app applied is not `tr-TR` | runs **ungraded** (`b3-offlocale-…`, JSON written) | runs ungraded (`b1-offlocale-real-…`) |
| The window cannot become frontmost within 5 s (display asleep, session locked) | INVALID; `--any-display` does not change this | INVALID |
| Keystrokes / fling steps reach an inactive window mid-run (focus lost, display slept) | INVALID | INVALID |
| R-M3-9 with fewer than 500 incremental analyses (a quarter of the 2,000 keystrokes: the worker coalesced most jobs) | INVALID | — |
| `--allow-inactive` (the CI harness check under xvfb) | goes on, **always ungraded** (`b3-offpanel-…`) | goes on, always ungraded (`b1-offpanel-real-…`) |

Every result line prints the actual `display_hz` and `display_scale`. Record them in the tables:
a faster display lowers B3's latency (the wait for the next frame is shorter).

## 2. Before running

- Use the `m3c` checkout at the commit under test. Write down `git rev-parse --short HEAD`; the
  printed `build=` must not end in `-dirty`.
- **Empty `target/bench` first.** Older dumps (M3a, M3b, CI-like `--any-display` runs) would
  otherwise match the same file names.
- The display must be awake and the session unlocked. Run every bench under `caffeinate -d`
  (as below): synthetic keystrokes do not reset the macOS idle timer, and the whole set takes
  many minutes. Close other builds and agents, because load slows the figures. Keep the bench
  window frontmost and do not touch the mouse or keyboard while a bench runs.
- `pg17` from the Docker matrix must be healthy for B1-real (the controller owns the compose
  project). The bench reads `pg17` only through `HACIVAT_DOCKER_SET=pr` and creates nothing on
  the server: the rows come from `generate_series`.

```sh
cd <the m3c checkout>
rm -rf target/bench                                                                       # no stale dumps
cargo xtask gen-sql --lines 50000 --out target/fixtures/b3-50k-t0.sql                    # T0: ~2 MB, full tier
cargo xtask gen-sql --lines 50000 --long-line 100000 --out target/fixtures/b3-50k.sql    # T1: Reduced
cargo xtask gen-sql --lines 520000 --out target/fixtures/b3-19mb.sql                     # 20,765,805 bytes (19-20 MiB), Reduced
cargo build --release -p hacivat-app --features bench --examples
E=target/release/examples      # or $CARGO_TARGET_DIR/release/examples
```

## 3. B3 in the real app (display: ≥ 118 Hz, any scale)

```sh
# Each run ends with `RESULT: PASS|FAIL|INVALID|n/a <label>`.
for m in typing typing-with-results typing-multi-caret typing-with-folds open scroll; do
  caffeinate -d $E/b3 --file target/fixtures/b3-50k-t0.sql --bench $m
done
for m in typing-long-line ime-bounds; do
  caffeinate -d $E/b3 --file target/fixtures/b3-50k.sql --bench $m
done
caffeinate -d $E/b3 --file target/fixtures/b3-19mb.sql --bench typing-19mb
```

What the modes do. Every typing mode sends 2,000 keystrokes at 60 Hz after an unmeasured
warm-up. The probe is marked at the editor's handler entry and recorded once the frame that shows
the input has been presented (as in M3a).

- `typing`: the end of line 25,000 of the T0 fixture. This run also grades **R-M3-9**.
- `typing-multi-caret`: 10 carets. The first is at the end of line 25,000. The other 9 are
  placed with the editor's own `editor.add_caret_below`, one on each line under it, at the same
  column clamped to that line's length. So a caret sits at a line's end only where that line is
  no longer than line 25,000. Every keystroke types at all 10. The run is INVALID if any caret
  is lost.
- `typing-with-folds`: 1,000 folds, each hiding 5 lines, one every 50 lines (the shape of the
  `hacivat-text` perf test). The typing line is in none of them. The run is INVALID unless all
  1,000 folds are still there at the end.
- `typing-19mb`: the middle line of the 19 MiB file, which opens in Reduced mode (the product's
  behaviour at that size). The run refuses a file under 19 MiB, or at or over 20 MiB: that is
  the Reduced ceiling and the largest console file the app restores. It also refuses a file
  that opens in another tier.
- `typing-long-line`, `typing-with-results`, `open`, `scroll`, `ime-bounds`: unchanged from M3a.

**R-M3-9.** While each typing mode measures, it also arms the language worker's own timer
(`hacivat_sql_editor::bench`, feature `bench` only). It runs on the GPUI background executor,
where the app really does this work. For each analysis it records two parts:

- the **scan**: `change_between`, which finds the one edit between the kept text and the rope.
  This grows with the document.
- the **apply**: `SqlDocument::apply`, the incremental relex and resplit.

It also records the whole analysis. R-M3-9 grades the scan and the apply together, and the line
prints both parts so that a figure near 2 ms says which part is large. Fresh builds and analyses
with nothing to relex are counted but kept out of the figure. A figure needs at least **500**
incremental analyses, a quarter of the 2,000 keystrokes; with fewer, the run is INVALID. The line
reads:

```
b3-typing-app R-M3-9 worker relex (scan + SqlDocument::apply, incremental analyses, at least 500): n=… p50=…ms p95=…ms p99=…ms max=…ms (budget 2 ms p95: PASS|FAIL|INVALID); scan n=… p50=… …; apply n=… …; whole analysis …; unchanged=… fresh=…
```

Only `typing` grades it (the 50k-line fixture R-M3-9 names). A FAIL or INVALID there makes the
run's RESULT the same. The other typing modes print it ungraded.

**Known and ungraded: the long-line relex.** `typing-long-line` types in the middle of a
100,000-character line. Its worker relex is expected at about **5–8 ms p95**: M3b Task 5
measured 5.6–8.3 ms on a 100k-character line, and 2.2–2.7 ms for a burst inside an open string.
That is above R-M3-9's 2 ms, and by design R-M3-9 does not cover it (the requirement names the
50k-line fixture of short lines). The figure is recorded, never graded.

## 4. B1-real (pg17, 1M × 50, `tr-TR`)

```sh
HACIVAT_DOCKER_SET=pr caffeinate -d $E/b1_real                   # 1440 x 900 on the built-in panel (2x, >= 118 Hz)  [gating]
HACIVAT_DOCKER_SET=pr caffeinate -d $E/b1_real --size 2560x1440  # a 1x external display set as the main display      [gating]
HACIVAT_DOCKER_SET=pr caffeinate -d $E/b1_real --sorted          # 1440 x 900, sorted by c01 first                    [not gating]
HACIVAT_DOCKER_SET=pr caffeinate -d $E/b1_real --mode streaming  # not gating (M3a)
```

- For the 2560 × 1440 run the window opens on the main display, so the PO makes the 1× external
  display the main display. The printed `display_scale` must read `Some(1.0)`.
- `--sorted` loads the whole result ("Fetch all", then "Fetch more anyway"). It then sorts all
  1,000,000 rows locally by `c01` (Turkish words mixed with an md5) through
  `GridView::request_sort`, as a header click does. It waits until the grid shows that sort over
  every row and prints how long the sort took (`… sort: c01 ascending over 1000000 rows shown
  after … ms`). The fling then scrolls over the sorted order, so the grid reads scattered model
  rows while it paints.
- The paint figure for the gate is `GRID PAINT (the gate's paint figure)`: the grid's own paint
  timer, giving `n`, p50/p95/p99/max and the frames over 3 ms and over 6 ms. The window's CPU per
  frame is printed after it as an upper bound. The sidecar `target/bench/load/<dump>.json` holds
  the same figures and the load figures.

### After the runs: hand the dumps over, do not run `bench-report` yet

Dump names now carry the seconds, the sub-second nanoseconds and a sequence number, so no two
runs overwrite each other. Do **not** run `bench-report` now: `--milestone m3c` writes
`docs/benchmarks/m3c.md`, which is Task 45's gate report, and running it early would create that
file with a bare header. Instead:

1. Copy the whole `target/bench` folder, with its `relex/` and `load/` sidecars, to a dated
   folder.
2. Give that folder and the printed logs to the controller.
3. The controller fills in section 5.1 from them.
4. Task 45 runs the report once its `m3c.md` exists, keeping the bench table as that file's last
   table (`bench-report` appends after the last table row in the file):

   ```sh
   cargo xtask bench-report --milestone m3c <the folder>/b1-fling-real-*.json <the folder>/b3-typing*-app-*.json
   ```

`bench-report` has accepted a milestone with a letter suffix (`m3a`, `m3b`, `m3c`) since M3c
Task 44; before that it accepted only `m<number>`.

## 5. Results

### 5.1 PO's Mac (graded): PENDING-PO

Build: PENDING-PO · Date: PENDING-PO · Machine: the PO's Mac

| Bench | Display (Hz, scale) | p50 | p95 | p99 | max | Samples | Verdict |
|---|---|---|---|---|---|---|---|
| `b3 typing` | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO |
| `b3 typing-long-line` (Reduced) | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO |
| `b3 typing-with-results` (1M × 50 open) | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO |
| `b3 typing-multi-caret` (10 carets) | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO |
| `b3 typing-with-folds` (1,000 folds) | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO |
| `b3 typing-19mb` (Reduced, release; ledger 298) | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO |
| R-M3-9 worker relex, scan + apply (`b3 typing`) | — | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO (≥ 500) | PENDING-PO |
| ... scan part (not graded on its own) | — | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | — | — |
| ... apply part (not graded on its own) | — | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | — | — |
| R-M3-9 worker relex in `typing-long-line` (known 5–8 ms, not graded) | — | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | n/a |

B3 modes that E5 does not gate (their own budgets):

| Bench | Display (Hz, scale) | Figures | Budget | Result |
|---|---|---|---|---|
| `b3 open` (5 opens, T0) | PENDING-PO | first paint p50 / max: PENDING-PO; full analysis p50 / max: PENDING-PO | 150 ms / 600 ms | PENDING-PO |
| `b3 scroll` (1,000 PageDown/PageUp) | PENDING-PO | p50 / p95 / p99 / max: PENDING-PO | none (n/a) | n/a |
| `b3 ime-bounds` (200 questions at line 49,000) | PENDING-PO | p50 / p95 / max: PENDING-PO | 1 ms p95 | PENDING-PO |

| Bench | Display | mean fps | gaps > 100 ms | frames (≥ 600) | grid paint p50 / p95 / p99 / max | > 3 ms / > 6 ms | sort time | Verdict |
|---|---|---|---|---|---|---|---|---|
| `b1_real` 1440 × 900 | PENDING-PO (2×, ≥ 118 Hz) | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | — | PENDING-PO |
| `b1_real --size 2560x1440` | PENDING-PO (1×) | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | — | PENDING-PO |
| `b1_real --sorted` (not gating) | PENDING-PO (2×, ≥ 118 Hz) | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | n/a |

### 5.2 Controller machine (not gating)

These ran on 2026-11-17 on the controller machine (a 14-core laptop on a 65 W adapter) while
other agents' builds were running (load average about 7). They are release builds with
`HACIVAT_PERF=1` and no window. No display is involved, so none of them stands in for a 5.1 cell.

| Perf line | Figure | Budget | Result |
|---|---|---|---|
| `hacivat-app --test results_sort` `sort_of_a_million_rows_perf` (1,048,576 rows) | int 103 ms; text `tr-x-icu` 947 ms; text code point 185 ms | int ≤ 300 ms, collated ≤ 2 s | pass |
| `hacivat-app --test results_find` `find_scan_and_filter_of_a_million_rows_perf` | scan 152 ms (131,072 matches); filter map 0 ms | test's own | pass |
| `hacivat-app --test results_model` `cell_through_the_adapter_hit_under_200_ns` | under 200 ns (the test prints no figure) | 200 ns | pass |
| `sql-lexer --test incremental` `relex_typing_edit_p95_under_2ms` (50k lines, 1,000 edits) | relex p95 0.30 ms | 2 ms | pass |
| `sql-split --test incremental` `relex_typing_edit_p95_under_2ms` | relex + resplit p95 0.59 ms | 2 ms | pass |
| `hacivat-ui-editor --test large_file` `typing_in_reduced_mode_stays_within_budget` (19 MiB, Reduced; ledger 298) | commit + draw p50 0.26 ms, p95 0.34 ms, max 0.45 ms over 200 keystrokes | 16 ms p95 | pass |

Commands:

```sh
HACIVAT_PERF=1 cargo test --release -p hacivat-app --test results_model --test results_sort --test results_find -- --ignored
HACIVAT_PERF=1 cargo test --release -p sql-lexer --test incremental -- --ignored relex_typing
HACIVAT_PERF=1 cargo test --release -p sql-split --test incremental -- --ignored relex_typing
HACIVAT_PERF=1 cargo test --release -p hacivat-ui-editor --test large_file -- --ignored typing_in_reduced --nocapture
```

The library relex figures (0.30 ms and 0.59 ms) come from the crates' own tests. The R-M3-9
figure that E5 reads is the one measured **in the real app's worker** (section 3), and it is
PENDING-PO. The worker figure also includes the one-edit diff over the rope, which the library
test does not have.

The new modes have headless smoke tests on GPUI's test platform (no frames, no figures). They
run with `cargo test --release -p hacivat-app --features bench --lib bench`, as CI's `bench` job
already does. The tests cover: 10 carets typing at once (`b3_multi_caret_types_at_ten_carets`),
1,000 folds kept while typing (`b3_folds_mode_keeps_a_thousand_folds_while_typing`), the caret at
the middle line in Reduced mode (`b3_large_mode_types_in_reduced_mode_at_the_middle_line`), the
worker timer recording incremental relexes (`the_language_worker_times_its_incremental_relex`),
and the whole result sorted through the grid
(`the_sorted_run_sorts_the_whole_result_through_the_grid`). A real-window run needs the PO's Mac.
