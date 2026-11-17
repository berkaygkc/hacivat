# M3c benchmarks: how the PO runs them (Task 44)

Task 44 Step 1 adds the M3c modes to the real-app benches (feature `bench`, never in a product
build). Step 2 is the PO's: the graded runs happen on the PO's Mac with the display awake and the
session unlocked. Until then every graded cell below is **PENDING-PO**. The figures in section 5
come from the controller machine and are **not gating**.

## 1. What each figure gates (M3c gate, Task 45 E5)

| Figure | Gate | Threshold | Where it is printed |
|---|---|---|---|
| B1-real, 1M × 50 from `pg17`, `tr-TR`, 1440 × 900 at 2× (≥ 118 Hz) | E5 | mean ≥ 55 fps, no frame gap > 100 ms, grid painted on ≥ 95 % of frames | `RESULT:` line, `verdict=` line |
| B1-real, the same at 2560 × 1440 on a 1× display | E5 | the same | the same |
| B1-real grid paint p50/p95/p99/max, frames > 3 ms and > 6 ms | E5 ("paint figures recorded") | recorded, not gating | `GRID PAINT (the gate's paint figure)` line; `target/bench/load/` |
| B1-real `--sorted` (whole result sorted by `c01`, then the fling) | none | not gating (`b1-sorted-real-*`) | the same lines, plus `sort:` |
| B3 `typing`, `typing-long-line`, `typing-with-results`, `typing-multi-caret`, `typing-with-folds`, `typing-19mb` | E5 | p95 ≤ 16 ms in **every** mode | `RESULT:` line; `p50/p95/p99/max` line |
| R-M3-9: incremental relex in the real app's worker | E5 | p95 ≤ 2 ms (graded in `b3 --bench typing`) | `R-M3-9 worker relex` line; `target/bench/relex/` |
| 19 MB typing in release (ledger 298) | E5 (B3 in every mode) | p95 ≤ 16 ms | `b3 --bench typing-19mb`; also the editor-level perf test (section 5) |

A run is graded only under the reference conditions. Otherwise it is **INVALID** (exit 3, no
JSON written) with a `PENDING-PO` note. INVALID never means FAIL: it means the run could not
measure.

- **B3** needs a refresh of at least 118 Hz, with no upper bound and at any scale, and the `tr-TR`
  format. This is B1-real's `PANEL_MIN_HZ`. The PO's graded M3a B3 ran on a 120 Hz 1× display
  and the M3b gate's on 240 Hz 1×; both count. A 60 Hz display, or one that reports no refresh
  (asleep), is refused. Every result line prints `display_hz` and `display_scale`, and the
  results table records them, because a faster display lowers latency (the wait for the next
  frame is shorter).
- **B1-real** keeps the M3a rule: 1440 × 900 needs the built-in panel at 2× and ≥ 118 Hz,
  2560 × 1440 needs a 1× display, and the viewport must cover ≥ 90 % of the requested size. The
  check runs before the load and again at the end.
- **Both** refuse a window that cannot become frontmost (display asleep, session locked).
  `--any-display` runs anywhere under an ungraded label (`b3-offpanel-…`, `b1-offpanel-real-…`);
  a format other than `tr-TR` gives `…-offlocale-…`. `--allow-inactive` is only the CI harness
  check; it never gives a graded number.

## 2. Before running

- The `m3c` checkout at the commit under test. Write down `git rev-parse --short HEAD`; the
  printed `build=` must not end in `-dirty`.
- Display awake, session unlocked. Close other builds and agents, because load slows the figures.
  Keep the bench window frontmost and do not touch the mouse or keyboard while a bench runs.
- `pg17` from the Docker matrix must be healthy for B1-real (the controller owns the compose
  project). The bench reads `pg17` only through `HACIVAT_DOCKER_SET=pr` and creates nothing on
  the server: the rows come from `generate_series`.

```sh
cd <the m3c checkout>
cargo xtask gen-sql --lines 50000 --out target/fixtures/b3-50k-t0.sql                    # T0: ~2 MB, full tier
cargo xtask gen-sql --lines 50000 --long-line 100000 --out target/fixtures/b3-50k.sql    # T1: Reduced
cargo xtask gen-sql --lines 520000 --out target/fixtures/b3-19mb.sql                     # 20,765,805 bytes (>= 19 MiB), Reduced
cargo build --release -p hacivat-app --features bench --examples
E=target/release/examples      # or $CARGO_TARGET_DIR/release/examples
```

## 3. B3 in the real app (display: >= 118 Hz, any scale)

```sh
# Each run ends with `RESULT: PASS|FAIL|INVALID|n/a <label>`.
for m in typing typing-with-results typing-multi-caret typing-with-folds open scroll; do
  $E/b3 --file target/fixtures/b3-50k-t0.sql --bench $m
done
for m in typing-long-line ime-bounds; do
  $E/b3 --file target/fixtures/b3-50k.sql --bench $m
done
$E/b3 --file target/fixtures/b3-19mb.sql --bench typing-19mb
```

What the modes do. Every typing mode sends 2,000 keystrokes at 60 Hz after an unmeasured
warm-up. The probe is marked at the editor's handler entry and recorded once the frame that shows
the input has been presented (as in M3a).

- `typing`: the end of line 25,000 of the T0 fixture. This run also grades **R-M3-9**.
- `typing-multi-caret`: 10 carets, at the end of line 25,000 and the 9 lines under it (placed
  with the editor's own `editor.add_caret_below`). Every keystroke types at all 10. The run is
  INVALID if any caret is lost.
- `typing-with-folds`: 1,000 folds, each hiding 5 lines, one every 50 lines (the shape of the
  `hacivat-text` perf test). The typing line is in none of them. The run is INVALID unless all
  1,000 folds are still there at the end.
- `typing-19mb`: the middle line of the 19 MiB file, which opens in Reduced mode (the product's
  behaviour at that size). The run refuses a file smaller than 19 MiB or one that opens in another
  tier.
- `typing-long-line`, `typing-with-results`, `open`, `scroll`, `ime-bounds`: unchanged from M3a.

**R-M3-9.** While each typing mode measures, it also arms the language worker's own timer
(`hacivat_sql_editor::bench`, feature `bench` only). For each analysis the timer records the
one-edit diff of the rope plus `SqlDocument::apply` (the incremental relex and resplit) on the
GPUI background executor, where the app really runs them. It records the whole analysis next to
it. Fresh builds and analyses with nothing to relex are counted but kept out of the figure. The
line reads:

```
b3-typing-app R-M3-9 worker relex (one-edit diff + SqlDocument::apply, incremental analyses): n=… p50=…ms p95=…ms p99=…ms max=…ms (budget 2 ms p95: PASS|FAIL); whole analysis …
```

Only `typing` grades it (the 50k-line fixture R-M3-9 names). A FAIL there makes the run's RESULT
a FAIL. The other typing modes print it ungraded.

**Known and ungraded: the long-line relex.** `typing-long-line` types in the middle of a
100,000-character line. Its worker relex is expected at about **5–8 ms p95**: M3b Task 5
measured 5.6–8.3 ms on a 100k-character line, and 2.2–2.7 ms for a burst inside an open string.
That is above R-M3-9's 2 ms, and by design R-M3-9 does not cover it (the requirement names the
50k-line fixture of short lines). The figure is recorded, never graded.

## 4. B1-real (pg17, 1M × 50, `tr-TR`)

```sh
HACIVAT_DOCKER_SET=pr $E/b1_real                        # 1440 x 900 on the built-in panel (2x, >= 118 Hz)  [gating]
HACIVAT_DOCKER_SET=pr $E/b1_real --size 2560x1440       # a 1x external display set as the main display      [gating]
HACIVAT_DOCKER_SET=pr $E/b1_real --sorted               # 1440 x 900, sorted by c01 first                    [not gating]
HACIVAT_DOCKER_SET=pr $E/b1_real --mode streaming       # not gating (M3a)
```

- For the 2560 × 1440 run the window opens on the main display, so the PO makes the 1× external
  display the main display. The printed `display_scale` must read `Some(1.0)`.
- `--sorted` loads the whole result ("Fetch all", then "Fetch more anyway"). It then sorts all
  1,000,000 rows locally by `c01` (Turkish words mixed with an md5) through
  `GridView::request_sort`, as a header click does. It waits until the grid shows that sort over
  every row and prints how long the sort took. The fling then scrolls over the sorted order, so
  the grid reads scattered model rows while it paints.
- The paint figure for the gate is `GRID PAINT (the gate's paint figure)`: the grid's own paint
  timer, giving `n`, p50/p95/p99/max and the frames over 3 ms and over 6 ms. The window's CPU per
  frame is printed after it as an upper bound. The sidecar `target/bench/load/<dump>.json` holds
  the same figures and the load figures.

To add the graded dumps as rows of the M3c gate report (`docs/benchmarks/m3c.md`):

```sh
cargo xtask bench-report --milestone m3c target/bench/b1-fling-real-*.json target/bench/b3-typing*-app-*.json
```

`bench-report` accepts a milestone with a letter suffix (`m3a`, `m3b`, `m3c`) since M3c Task 44.
Before that it accepted only `m<number>`. The rows in section 5.1 come from the same dumps. The
R-M3-9 figures (`target/bench/relex/`) and the paint and load figures (`target/bench/load/`) are
sidecars: copy them from the printed lines.

## 5. Results

### 5.1 PO's Mac (graded): PENDING-PO

Build: PENDING-PO · Date: PENDING-PO · Machine: the PO's Mac

| Bench | Display | p50 | p95 | p99 | max | Samples | Verdict |
|---|---|---|---|---|---|---|---|
| `b3 typing` | PENDING-PO (Hz, scale) | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO |
| `b3 typing-long-line` (Reduced) | PENDING-PO (Hz, scale) | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO |
| `b3 typing-with-results` (1M × 50 open) | PENDING-PO (Hz, scale) | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO |
| `b3 typing-multi-caret` (10 carets) | PENDING-PO (Hz, scale) | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO |
| `b3 typing-with-folds` (1,000 folds) | PENDING-PO (Hz, scale) | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO |
| `b3 typing-19mb` (Reduced, release; ledger 298) | PENDING-PO (Hz, scale) | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO |
| R-M3-9 worker relex (`b3 typing`) | — | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO |

| Bench | Display | mean fps | gaps > 100 ms | frames | grid paint p50 / p95 / p99 / max | > 3 ms / > 6 ms | Verdict |
|---|---|---|---|---|---|---|---|
| `b1_real` 1440 × 900 | 2×, ≥ 118 Hz | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO |
| `b1_real --size 2560x1440` | 1× | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO |
| `b1_real --sorted` (not gating) | 2×, ≥ 118 Hz | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | PENDING-PO | n/a |

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
