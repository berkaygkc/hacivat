# M3a gate record

Task 37 writes this record. Task 36 started the file with the notes the gate needs about the real-app benches (`hacivat-app` examples `b3` and `b1_real`, feature `bench`).

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
