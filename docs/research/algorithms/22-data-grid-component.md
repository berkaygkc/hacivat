# 22 — Data Grid Component (GPUI): Canonical Algorithm Catalog (HV-GRID)

> Status: canonical, implementation-ready. Date: 2026-11-11. Scope: Hacivat's own data grid, built on GPUI inside the published `hacivat-ui` library (ADR 0001): component boundary and data-source trait, 2-axis virtualization with row-index anchoring, rendering layers, column auto-size, selection and keyboard model, model↔view index mapping, local sort, local filter, find, cell display formatting and column display types, input parsing, alternate views (transposed, record, text, tree), the cell-editor lifecycle, go-to-row, column statistics, and accessibility.
>
> Synthesis document; conventions in `../README.md`. Companions: `20-query-execution.md`, `21-result-fetching-and-store.md` (the data the grid shows), `23-data-editing.md` (pending changes rendered by the grid), `24-clipboard-import-export.md` (copy/paste on top of the selection model).

## Sources and licenses

| Source document | Specs consumed | Underlying license |
|---|---|---|
| `clients/ui-02-code-editor-and-data-grid-components.md` | ALG-GRID-01 (virtualization), -02 (auto-size), -03 (selection), §3.3.6 (accessibility shadow), §3.3.9 (native grids) | Glide Data Grid MIT, TanStack Virtual MIT, egui MIT/Apache, gpui-component Apache-2.0 |
| `datagrip/08-data-grid.md` | ALG-GRID-11, -13, -15, -19, -20, -21; §3.1 (render cache, colour layers, fast-scroll low quality, column limit), §3.8 (editors), §3.11–3.16 (views, display types) | DataGrip proprietary (behaviour only) |
| `datagrip/11-settings-catalog.md` | DV-8…DV-17, custom number/date formats | DataGrip proprietary |
| `clients/dbeaver-04-results-viewer-data-editor.md` | ALG-DBV-GRID-09 (local filter/sort), §3.6 (display formats UI/EDIT/NATIVE), §3.7 (presentations, record mode), §3.12 (find/replace), Q1, Q5, Q10, Q12 defects | Apache-2.0 @ `f78ac45` |
| `clients/oss-clients-redisinsight-compass.md` | ALG-MONGO-02 (document → table), ALG-MONGO-03 (JSON tree virtualization) | Compass SSPL (behaviour only) |
| `clients/oss-clients-beekeeper-and-tui.md` | grid references, column width defaults | Beekeeper GPL (behaviour only) |
| `i18n/02-turkish-and-db-specific-pitfalls.md` | ALG-I18N-06 (search key), ALG-I18N-07 (number parse/format), §3.6.2 (sorting), §3.8–3.9 (display modes, dates), §3.15 (render hazards), §3.11 (shortcuts) | ICU4X Unicode-3.0, CLDR 49 |
| `decisions/0001-native-ui-on-gpui.md` | GPUI engine, own component library, benchmark B1 | gpui Apache-2.0 |
| `verification/01`, `verification/02` | Turkish collation orders (#39–#41), AG Grid/clipboard license facts (P34), GPUI license (P33) | — |
| GPUI source (`.refs/repos/zed@912159c`, Apache-2.0 crates only) | API names cited: `canvas`, `uniform_list`, `Window::paint_quad`, `with_content_mask`, `paint_layer`, `insert_hitbox`, `TextSystem::shape_line`, `Element::a11y_role`/`write_a11y_info`, `Window::on_a11y_action`; `Pixels(f32)` | Apache-2.0 |
| gpui-component (`.refs/repos/gpui-component@3c5320b`, Apache-2.0) | `crates/component/src/table/{state,delegate}.rs` as reference only | Apache-2.0 |

Clean-room: own pseudocode. Zed's GPL crates (`editor`, `ui`, `workspace`, `text`, `rope`) were not read. gpui-component's table is a reference: it uses `usize` row indices, single cell/row/column selection and string-typed `dump()` output [Observed: `table/state.rs:64-78, 523-713`] — not enough for a 10M-row typed DB grid, so we do not adopt it.

---

## 0. Dependency map and priorities

```
 HV-GRID-01 component + GridDataSource trait (hacivat-ui::grid, DB-agnostic)
      │
      ├──► HV-GRID-02 virtualization (index-anchored scroll, f64 math, custom scrollbars)
      │          │
      │          ▼
      ├──► HV-GRID-03 render pipeline (layers, frozen regions, text cache, fast-scroll mode)
      │          ▲
      │          │ widths
      ├──► HV-GRID-04 column auto-size
      ├──► HV-GRID-05 selection + keyboard/pointer model ──► HV-XFER-02/03 copy/paste (doc 24)
      ├──► HV-GRID-06 model↔view mapping ◄── HV-GRID-07 local sort, HV-GRID-08 local filter, HV-GRID-09 find
      ├──► HV-GRID-12 alternate views (transposed, record, text, tree)
      ├──► HV-GRID-13 cell editor lifecycle ──► HV-EDIT-02/03 (doc 23)
      ├──► HV-GRID-14 go to row      HV-GRID-15 column statistics (later)
      └──► HV-GRID-16 accessibility (AccessKit through GPUI hooks)

 hacivat-core::valfmt:  HV-GRID-10 display formatting & display types   HV-GRID-11 input parsing
 (used by the grid adapter, copy-as-displayed, export, MCP, import)
```

| ID | Name | Priority | Depends on | Consumed by |
|---|---|---|---|---|
| HV-GRID-01 | Component architecture and `GridDataSource` | **v1** | GPUI | everything in this file |
| HV-GRID-02 | Two-axis virtualization with row-index anchoring | **v1** | 01 | 03, 05, 16 |
| HV-GRID-03 | Render pipeline: layers, frozen regions, text cache | **v1** | 01, 02 | — |
| HV-GRID-04 | Column auto-size | **v1** | 03 (text measurement), 10 | 02 |
| HV-GRID-05 | Selection model and keyboard/pointer map | **v1** | 02, 06 | 13, HV-XFER-02/03, aggregators |
| HV-GRID-06 | Model ↔ view index mapping | **v1** | HV-RES-02 (tombstones) | 05, 07, 08, 09 |
| HV-GRID-07 | Local sort | **v1** | 06, 10 | — |
| HV-GRID-08 | Local filter (value lists) and quick filter | **v1.x** | 06, 10 | — |
| HV-GRID-09 | Find on page with "filter rows" | **v1** | 06, 10, ALG-I18N-06 | 05 |
| HV-GRID-10 | Cell display formatting and column display types | **v1** | HV-RES-01 | grid, copy, export, MCP |
| HV-GRID-11 | Cell input parsing (lenient, round-trip, ambiguity) | **v1** | HV-RES-01 | HV-EDIT-03, HV-XFER-03/09 |
| HV-GRID-12 | Alternate views: transposed, record, text, tree | **v1** (transpose, record, text); **v1.x** (tree, document flattening) | 02, 05, 06 | — |
| HV-GRID-13 | Cell editor lifecycle and value viewer | **v1** | 05, 10, 11 | HV-EDIT-02 |
| HV-GRID-14 | Go to row | **v1** | 06 | — |
| HV-GRID-15 | Column statistics header | **later** | 10 | — |
| HV-GRID-16 | Accessibility (AccessKit) | **v1.x** (release blocker for 1.0 GA per ui-02) | 02, 05 | — |

### Performance budgets (all specs in this file)

| Budget | Target | Source |
|---|---|---|
| Scroll / fling 10M rows × 50 columns | ≥ 55 fps macOS/Windows, ≥ 45 fps Linux reference; no blank frame > 100 ms | ADR 0001 benchmark B1 |
| Grid paint per frame (≈ 60 rows × 20 visible columns) | ≤ 6 ms CPU at 60 Hz, ≤ 3 ms at 120 Hz | derived from B1 |
| Input → visual response (selection move, typing into a cell) | ≤ 1 frame (16 ms) | — |
| Column auto-size of 200 columns × 60 samples | ≤ 30 ms, time-sliced at ≤ 4 ms per frame | ui-02 ALG-GRID-02 |
| Local sort of 1M loaded rows | numeric ≤ 300 ms, text with collation ≤ 2 s, off the UI thread, cancellable | — |
| Find over 1M loaded cells | first match ≤ 50 ms, full scan ≤ 500 ms, incremental | — |
| Memory of view mapping | 0 bytes when no local sort/filter; ≤ 4 bytes per loaded row otherwise | — |

## 1. Cross-cutting decisions for this catalog

1. **`hacivat-ui::grid` is database-agnostic** and publishable (Apache-2.0). It knows rows, columns, display content, selection and editing hooks — not SQL, sessions or `Value`. `hacivat-app::results::GridAdapter` binds it to `hacivat-core::results::ResultSource` (HV-RES-03). Value formatting and parsing live in `hacivat-core::valfmt`, because copy-as-displayed, export, MCP and import need them too.
2. **GPUI boundary.** The grid is a custom GPUI `Element` (implementing `request_layout` / `prepaint` / `paint`) that paints with `Window::paint_quad`, shaped text (`TextSystem::shape_line` → `ShapedLine::paint`), content masks (`with_content_mask`) for frozen regions, one body hitbox (`insert_hitbox`), and `request_animation_frame` for kinetic effects. All GPUI calls go through the thin adapter that ADR 0001 requires, so the engine can be swapped (fallback: Linebender stack).
3. **f32 precision.** GPUI `Pixels` is `f32` [Observed: `gpui/src/geometry.rs:2736`]. Absolute row positions are never computed in `f32`; only offsets inside the viewport are (HV-GRID-02).
4. **Data display defaults (i18n/02):** raw machine representation by default (no grouping, `.` decimal, ISO-8601 dates in the session time zone), localized display opt-in per user, connection or column; editing always pre-fills the raw value; NULL is a styled marker, not text.
5. **Shortcuts** keep DataGrip muscle memory where it does not collide with Turkish Q/F layouts or AltGr combinations (i18n/02 §3.11): Cmd/Ctrl+Enter submit, Ctrl+Alt+N set NULL (Cmd+Option+N on macOS), Ctrl+Alt+D set DEFAULT, Shift+Enter value viewer, Ctrl+G go to row (Cmd+L on macOS), Ctrl+F find, F4 related rows. Keymaps are data files with stable English action ids (`grid.submit`, `grid.setNull`, …); physical key codes for letter shortcuts.
6. **Crate versions to check:** `gpui 0.2.2` (git pin `zed@912159c`), `accesskit` (the version GPUI pins), ICU4X `2.4.0` (`icu_collator`, `icu_casemap`, `icu_segmenter`, `icu_decimal`, `fixed_decimal`), `jiff 0.2.17`, `regex 1.12.2`, `memchr 2.8.3`, `lru 0.12.5`, `roaring` (check), `unicode-bidi` (check) or ICU4X bidi.

---

## 2. Specifications

### HV-GRID-01: Component architecture and the `GridDataSource` trait

- **Priority / depends on:** v1. Depends on GPUI only.
- **Sources merged:** datagrip/08 §3.1 (`GridModel`, hook-up = loader + paging + sorting + filtering + mutator; surfaces with a capability matrix; render cache; colour layers), dbeaver-04 §2.1 and §7.9 (UI-free model; split the 5,000-line grid into small testable state machines), ui-02 §5.2 (`GridAdapter`), ADR 0001 (own library, thin GPUI adapter).
- **Problem:** Define the boundary between a reusable grid widget and Hacivat's data layer, so that the widget can render 10M-row results, Redis hashes, Mongo documents, CSV files and the Perde-like audit lists alike.
- **Inputs / outputs:** a `GridDataSource` implementation + `GridConfig` → a GPUI view `Grid` that emits `GridEvent`s (selection changed, edit requested, edit committed, column resized/moved, sort requested, context menu, find state).
- **Decision:**
  - The grid owns **view state** (scroll, column order/widths/hidden/frozen, selection, find state, local sort/filter permutation, editor state). The data source owns **data** (rows, display content, editability, pending-change decorations).
  - The data source returns **display content**, already formatted (HV-GRID-10 runs in the adapter with a cache), plus decoration flags. The grid never formats values.
  - Capabilities are data (`SourceCaps`): server sort, server filter, editable, insertable, deletable, has-more, can-count, transposable. Menus and MCP derive available actions from them (DataGrip capability-matrix idea).
  - Composition, not one giant class: `ScrollModel`, `ColumnModel`, `SelectionModel`, `ViewMap`, `FindModel`, `EditorModel`, `Renderer` — each unit-tested without GPUI.
  - Rejected: building on gpui-component `Table` (usize rows, single selection, string dumps); rejected: one grid type per surface.
- **Algorithm (frame lifecycle):**
  1. `prepaint`: `frame = source.begin_frame()` (one snapshot load). Compute the visible ranges (HV-GRID-02). Map view → model rows/cols (HV-GRID-06). Request display content for visible cells through the adapter cache; missing rows → placeholders + `source.ensure_rows(upto)`.
  2. `paint`: layers in HV-GRID-03 order.
  3. Input: one body hitbox; pointer → (view row, view col) by inverse mapping; keyboard → `SelectionModel` / `EditorModel` / actions.
  4. On `source.changed()` (generation/version), invalidate content caches for the changed generation and call `cx.notify()` at most once per frame.
- **Data structures and Rust placement** (`hacivat-ui::grid`):
  ```rust
  pub trait GridDataSource: 'static {
      type Frame: GridFrame;
      fn begin_frame(&self, cx: &App) -> Self::Frame;
      fn caps(&self) -> SourceCaps;
      fn columns(&self) -> &[ColumnSpec];                // stable ColId, header text, kind (Text/Number/Bool/Binary/Json/...), alignment, badges
      fn ensure_rows(&self, upto: u64);
      fn edit_requested(&self, cell: ModelCell, seed: EditSeed) -> EditAdmission;   // Allowed(EditorKind, initial_text) | ReadOnly(reason)
      fn edit_committed(&self, cell: ModelCell, text: String) -> EditResult;         // parsed by the adapter (HV-GRID-11) into the pending overlay
  }
  pub trait GridFrame {
      fn row_count(&self) -> RowCount;                   // { loaded: u64, more: bool, exact_total: Option<u64> }
      fn cell(&self, row: ModelRow, col: ColId) -> CellContent;
      fn row_state(&self, row: ModelRow) -> RowState;    // Normal | Inserted | Deleted | Error
      fn generation(&self) -> (u64, u64);
  }
  pub struct CellContent { pub text: SharedString, pub kind: CellKind /* Text | Number | Null | Bool(Option<bool>) | Placeholder | Error | Binary | Json | Image */,
                           pub flags: CellFlags /* MODIFIED | UNPARSED | TRUNCATED | LOSSY | READ_ONLY | RTL | HAS_CONTROLS */, pub align: Align }
  pub struct ModelRow(pub u64); pub struct ViewRow(pub u64); pub struct ColId(pub u32);
  ```
- **Complexity / performance budget:** see the budget table; the frame lifecycle does O(visible cells) work and O(1) snapshot loads.
- **Edge cases (exhaustive list):** zero columns (grid hidden; outcome panel instead); zero rows (header only); more than 2,000 columns (DataGrip refuses with "Too many columns"; we render with column virtualization up to 100,000 columns and show a hint above 2,000 suggesting the record or text view); data source dropped while painting (the frame holds its snapshot); source generation change mid-edit (the editor commits to the old generation's row identity or is cancelled with a notice); two grids on one source (independent view state); a source without editability (all edit requests return ReadOnly with a reason).
- **Test cases (input → expected):** a mock source with 10M rows × 50 columns renders the first frame within 16 ms; a source returning `Placeholder` for unloaded rows triggers exactly one `ensure_rows` per frame; `caps.editable = false` → the Edit context-menu item shows "View" (DataGrip convention).
- **i18n notes:** the grid has no hard-coded strings; menus and tooltips take Fluent ids from the host (`grid-*` messages in `hacivat-ui.ftl` for generic chrome: `grid-loading`, `grid-too-many-columns-hint`, `grid-read-only`).
- **Verification status:** design decision. GPUI API names [Observed: `zed@912159c`]. gpui-component limitations [Observed: `crates/component/src/table/state.rs`].
- **Prior art and evidence:** datagrip/08 §3.1–3.2; dbeaver-04 §5.
- **Used by:** Hacivat (result tabs, table editor, CSV-as-table, import preview, compare viewer, Redis/Mongo grids); any `hacivat-ui` consumer.

### HV-GRID-02: Two-axis virtualization with row-index anchoring

- **Priority / depends on:** v1. Depends on HV-GRID-01.
- **Sources merged:** ui-02 ALG-GRID-01 (index-anchored scroll state, native-toolkit custom scrollbar, `advanceRows`, prefix sums for columns, Fenwick tree for variable row heights, frozen regions, scroll compensation on height change, f64 arithmetic, edge cases), datagrip/08 §3.1 (fast-scroll low-quality rendering).
- **Problem:** Render up to 10^9 rows and 10^5 columns smoothly on GPUI, where coordinates are `f32` (exact only to 2^24 ≈ 16.7M px — 700k rows of 24 px), with frozen rows/columns, variable column widths and growing row counts.
- **Inputs / outputs:** row count (growing), column widths (view order), row height (fixed in v1), frozen sets, viewport size, scale factor, input events → `VisibleRegion { rows: Range<ViewRow>, cols: Range<usize>, row_offset_px: f32, col_offset_px: f32, frozen… }`.
- **Decision:**
  - **Scroll state is index-based:** `top_row: u64`, `top_row_offset: f32 ∈ [0, row_h)`; horizontally `left_x: f64` over prefix sums (column counts are small enough for f64).
  - **Custom scrollbars** (no pixel-height content element ever exists): thumb position = `(top_row + top_row_offset/row_h) / max(1, rows_scrollable − rows_per_viewport)`.
  - **Fixed row height in v1** (`row_h` from the font + padding; per-grid setting "compact/normal/comfortable"); multi-line values show a `↵` marker and open in the value viewer. Variable heights (Fenwick tree) are v1.x for the tree view and expanded rows.
  - **Fast-scroll mode** (DataGrip `database.grid.render.cache.fast.scroll` idea): while the scroll velocity exceeds `fast_rows_per_frame` (default 40 rows/frame), paint cells with cached shapes only; uncached cells paint a cheap truncated ASCII-safe prefix (≤ 24 chars, shaped as a single run) and decorations are reduced; the full quality frame follows within 100 ms of the velocity dropping.
- **Algorithm:**
  1. `advance_rows(dy_px: f64)`: `total = top_row_offset + dy`; `k = floor(total / row_h)`; `top_row = clamp(top_row + k, 0, max_top_row)`; `top_row_offset = total − k·row_h`, clamped to 0 at the bounds. `max_top_row = rows_scrollable.saturating_sub(rows_fully_visible)` so that the last row aligns with the bottom edge.
  2. Wheel and trackpad events (`ScrollDelta::Pixels` / `Lines`) → `advance_rows` (lines × row_h); momentum events from the OS are treated as small deltas; coalesce all scroll events per frame.
  3. Scrollbar thumb drag → `virtual_y = p × (rows_scrollable·row_h − H_body)` computed in f64 → `top_row = floor(virtual_y / row_h)`, `top_row_offset = virtual_y − top_row·row_h`. Thumb at either end → exactly row 0 / `max_top_row`.
  4. Programmatic navigation (keyboard, go-to-row, find): `scroll_into_view(row)`: if above → `top_row = row, offset 0`; if below → `top_row = row − rows_fully_visible + 1`, offset adjusted so that the row's bottom aligns.
  5. Visible rows: `r0 = top_row`, `n = ceil((H_body + top_row_offset) / row_h)`, `r1 = min(rows, r0 + n)`; overscan `±8` rows for prefetch hints (not painted).
  6. Columns: prefix sums `X[c]` (f64) over scrollable (non-frozen, non-hidden) columns in view order; visible `c0 = upper_bound(X, left_x) − 1`, `c1 = lower_bound(X, left_x + W_body)`; on width change of column k → update `X[k..]` (O(C); C ≤ 10^5 is fine) or a Fenwick tree when C > 10^4.
  7. Pixel positions inside the viewport: `y(r) = (r − r0)·row_h − top_row_offset` (small numbers, safe in f32).
  8. Growth while streaming: if the user was at the bottom before growth and "follow" is on → stick to the bottom; else keep `top_row`. Shrink (re-run, filter): clamp `top_row` to the new `max_top_row`.
  9. Frozen regions: `FL` left columns, `FR` right columns, `FT` top rows (pinned rows, v1.x), header and row-number gutter; they are excluded from the scrollable prefix sums.
  10. Emit `visible_region_changed(r0, r1, c0, c1)` debounced (50 ms) for the status bar and prefetch.
- **Data structures and Rust placement** (`hacivat-ui::grid::scroll`):
  ```rust
  pub struct ScrollState { pub top_row: u64, pub top_row_offset: f32, pub left_x: f64, pub follow_tail: bool, velocity: RowsPerFrame }
  pub struct ColumnLayout { order: Vec<ColId>, widths: Vec<f32>, prefix: Vec<f64>, frozen_left: u32, frozen_right: u32 }
  ```
- **Complexity / performance budget:** O(log C) column range, O(1) row range per frame; all row math in u64/f64 (exact to 2^53 → 10^9 rows × 100 px = 10^11 exact).
- **Edge cases (exhaustive list):** `R = 0`; `C = 0`; `R` grows while streaming (follow vs keep); `R` shrinks; viewport taller than all rows; frozen columns wider than the viewport (disable horizontal scrolling, hint, allow unfreeze); `FL + FR ≥ C`; scale-factor change between monitors (re-shape text, full repaint); zoom/font change (row_h changes, `top_row` preserved); column hidden/shown/reordered (rebuild prefix sums; keep `left_x` anchored to the first visible column's id); a column wider than the viewport; scroll event storms at 120 Hz (coalesce per frame); scrollbar at the extremes; keyboard navigation beyond loaded rows (scroll immediately, placeholders); RTL UI locale (chrome mirroring is a product decision; cell text direction is per cell, HV-GRID-03); window hidden (no painting; full repaint on show); f32 hosts (never compute `top_row × row_h` in f32); a row count of exactly `u32::MAX + 1` (u64 everywhere).
- **Test cases (input → expected):** R = 1,000,000,000, row_h = 24, H_body = 600 → initial r0 = 0, r1 = 25; End → r0 = 999,999,975 with the last row fully visible; thumb dragged to 50% → top_row = floor((R·24 − 600)/2/24) ± 1; 1,000 wheel events of 3 px → top_row advances exactly 125 rows; widths [100, 200, 50, 300], left_x = 250, W_body = 300 → visible cols [1, 4); FL = 2 with widths 400 + 400 and W = 700 → horizontal scrolling disabled; R shrinks from 10,000 to 10 while top_row = 9,000 → top_row = 0; fling at 200 rows/frame → fast-scroll mode, ≥ 55 fps on the B1 dataset.
- **i18n notes:** scrollbar tooltips (`grid-row-position = Row { $row } of { $total }`) use locale digits for display only.
- **Verification status:** algorithm from ui-02 [Observed: Glide, TanStack, egui sources]; GPUI `Pixels(f32)` [Observed]. B1 must be measured in the ADR 0001 spike.
- **Prior art and evidence:** ui-02 ALG-GRID-01; DataGrip fast-scroll registry key [Observed: datagrip/08 §3.1].
- **Used by:** the grid, the tree view (HV-GRID-12), the output log and history lists (same scroll model).

### HV-GRID-03: Render pipeline — layers, frozen regions, text cache

- **Priority / depends on:** v1. Depends on HV-GRID-01/02.
- **Sources merged:** datagrip/08 §3.1 (colour layers stripe → mutations → search → selection; render cache; fast-scroll), ui-02 ALG-GRID-01 steps 8–11 (frozen draw order, device-pixel snapping, ellipsis by measured prefixes, per-cell direction by first strong character), i18n/02 §3.15 (bidi controls visible, line breaks/tabs as glyphs, grapheme truncation, per-cell `dir=auto`).
- **Problem:** Paint ~1–3k visible cells per frame within ~6 ms, with correct stacking of states (pending edits, search, selection), frozen regions, crisp lines and correct text direction, including for data containing control characters.
- **Inputs / outputs:** visible region, view state, display content → GPUI scene primitives.
- **Decision:**
  - **Layer order (per region, bottom to top):** (1) region background; (2) zebra stripe (setting); (3) row state tint (inserted green, deleted red + strike-through line, error); (4) modified-cell tint; (5) search matches (all), current match stronger; (6) selection fill (translucent) and selected-row/column header tint; (7) cell content (text, NULL marker, checkbox glyph, type icons, truncation chip); (8) grid lines (hairlines snapped to device pixels); (9) cell-state borders (unparsed = red border, focus cell = 2 px accent border); (10) overlays: resize guide, drag ghost, drop indicator, skeleton shimmer for placeholders.
  - **Region order:** scrollable body → frozen-left body → frozen-right body → header row → row-number gutter → corner → scrollbars → in-cell editor (a real GPUI input element positioned over the cell) → tooltips/popovers. Each region paints inside `with_content_mask(region_bounds)`.
  - **Text cache:** an LRU (default 50k entries) of `ShapedLine`s keyed by `(content hash, font id, size, truncation width bucket, color class)`, independent of GPUI's per-frame line-layout cache. Ellipsis truncation is computed once per (string, width bucket) by binary search on cluster boundaries (ICU4X grapheme segmenter).
  - **Control characters are made visible:** `\n` → `↵`, `\t` → `→`, other C0/C1 → `␀`-style glyph; bidi controls (U+202A–U+202E, U+2066–U+2069, U+200E/F, U+061C) → a small chip `⟨RLO⟩` (Trojan Source defence); zero-width characters → a dotted marker when the cell is focused.
  - **Direction per cell:** text cells use the first strong character (UAX#9 P2–P3); numbers are always LTR and right-aligned; the column's alignment follows its kind (numbers right, text start, booleans center).
- **Algorithm:**
  1. For each region: compute the visible cell rects (integer device pixels after snapping).
  2. Batch quads by layer (backgrounds, tints, selection) → `paint_quad` per contiguous run (merge horizontally adjacent same-colour quads to reduce primitives).
  3. Text: for each cell, `shaped = text_cache.get_or_shape(content, width)`; paint at the baseline with clipping to the cell; for `Placeholder` paint a skeleton bar.
  4. NULL: italic, muted colour, the configured label (`NULL` by default); empty string: nothing (optionally a faint `∅` marker setting "show empty strings"); `Missing` (documents): faint `—`.
  5. Grid lines: one quad per visible row/column boundary, positioned at `round(x·scale)/scale + 0.5/scale`.
  6. Fast-scroll mode (HV-GRID-02): keep all tints (they are cheap quads); skip only the shaping of uncached strings (prefix fallback) and type icons.
  7. Damage: GPUI rebuilds the scene each frame; we avoid unnecessary frames by calling `cx.notify()` only when state changed (scroll, data version, selection, hover).
- **Data structures and Rust placement** (`hacivat-ui::grid::render`): `TextCache { lru: LruCache<TextKey, Arc<ShapedLine>> }`, `LayerBatch { quads: Vec<PaintQuad> }`, theme tokens `grid.*` (stripe, null, modified, inserted, deleted, unparsed, search, search.current, selection, focus, gridline).
- **Complexity / performance budget:** paint O(visible cells); ≤ 6 ms per frame at 60 Hz for 1,200 cells with a warm cache; shaping an uncached short string ≤ 15 µs (fast-scroll prefix fallback keeps the per-frame shaping count ≤ 150).
- **Edge cases (exhaustive list):** very long single-token strings (URLs, base64) → ellipsis; emoji/ZWJ sequences and combining marks (truncate on grapheme boundaries); CJK full-width text; RTL text mixed with numbers (UBA per cell); text with bidi controls (chip); multi-line values (first line + `↵`); tabs; NUL bytes; invalid UTF-8 rendered as `\xNN` escapes (lossy flag); huge cell content (> 64 KiB → only the first 1 KiB is shaped for display); theme switch (invalidate colour classes, keep shapes); scale change (invalidate the text cache); fonts missing glyphs (GPUI font fallback; Turkish glyphs `ğ ı İ ş` must render with the default font — checked in the spike); high-contrast theme (borders instead of tints); selection over modified cells (both visible: tint under, selection fill translucent over).
- **Test cases (input → expected):** a cell containing `a\tb\nc` renders `a→b↵c` on one line; a cell with U+202E renders the `⟨RLO⟩` chip and copy-raw keeps the character (HV-XFER-02); an Arabic text cell in a start-aligned text column keeps the column's alignment and gets RTL glyph order from its first strong character; a modified + selected + search-matched cell shows all three states distinguishably (screenshot test); paint time for the B1 dataset ≤ 6 ms p95.
- **i18n notes:** NULL label `value-null` (ASCII `NULL` by default, configurable); chips for bidi controls use the Unicode abbreviation (`RLO`, `LRI`, …) — not translated; tooltip `grid-bidi-control = Invisible direction control character { $name }`.
- **Verification status:** layer list [Observed: datagrip/08 §3.1 class names]; bidi/control rules [Inferred: CVE-2021-42574 research, i18n/02 §3.15]; GPUI text APIs [Observed].
- **Prior art and evidence:** Glide blit/ellipsis [Observed]; DataGrip colour layers.
- **Used by:** the grid and every list built on it.

### HV-GRID-04: Column auto-size

- **Priority / depends on:** v1. Depends on HV-GRID-03 (measurement), HV-GRID-10 (display strings).
- **Sources merged:** ui-02 ALG-GRID-02 (persisted widths first, head + tail + stratified samples, display transformation, outlier trimming at ≥ 2μ iterated, header width with icons, type bounds, fill, no auto-resize after layout, auto-fit actions), Beekeeper defaults, harlequin cap (half container width), datagrip/08 §3.15 (Expand/Narrow/Reset width actions).
- **Problem:** Pick initial widths that show typical content without huge JSON/base64 values blowing up the layout, using only already-loaded rows.
- **Inputs / outputs:** column specs, sampled display strings, fonts, viewport width, persisted widths → `width[c]` and `wrap_hint[c]`.
- **Decision:** ui-02 ALG-GRID-02 unchanged, with Hacivat defaults: min 48 px, max initial 480 px, absolute max 2,000 px; samples = first 50 + last 10 loaded + 20 stratified; persisted widths keyed by `(connection, schema, table | query fingerprint, column name)`; measurement time-sliced (≤ 4 ms per frame) so that the first frame never waits for sizing (columns start at type-based default widths and settle within ≤ 3 frames).
- **Algorithm:**
  1. Persisted width → use it.
  2. Initial frame: type-based default (bool 64, int 96, decimal 120, uuid 300, timestamp 200, text 160, json/binary 200).
  3. Background sizing (time-sliced on the UI thread because shaping needs the text system): for each column, take samples, convert with the same formatter as the cells, measure shaped widths (cached), `+ 2·padding + icon widths`.
  4. Outlier trimming: `n > 5` → μ = mean; drop `w ≥ 2μ`; repeat up to 3 passes when `n ≥ 20`; `cand = max(remaining)`; `n ≤ 5` → `cand = max`. Record `trimmed_count` for the hint.
  5. `hdr = measure(header) + sort icon + key/FK badge + type badge`; `w = max(cand, hdr)` (header-driven width limited to 320 px, header text then truncates with a tooltip).
  6. Clamp by type bounds and global min/max; snap to whole pixels.
  7. Fill: if Σw < viewport and C ≤ 8 → grow text-like columns proportionally (never numerics).
  8. Never resize automatically afterwards; "Auto-fit column" (double-click the header border) re-measures up to 10k loaded rows of that column; "Auto-fit all"; "Reset width".
- **Data structures and Rust placement:** `hacivat-ui::grid::autosize { SizerJob, WidthCache }`; persisted widths in `hacivat-app` workspace state.
- **Complexity / performance budget:** O(C × n) measurements, cached; ≤ 30 ms total for 200 × 60, spread over frames.
- **Edge cases (exhaustive list):** empty result (header widths); all-NULL column; single row; multi-line strings (first line; wrap hint); RTL and CJK (shaped widths); emoji/ZWJ; tabs (rendered glyphs); one huge token; headers longer than all values; duplicate column names (persist by position + name); columns arriving later (size on arrival); locale change for localized numbers (re-run for numeric/date columns not user-sized); zoom (scale proportionally); C > 1,000 (10 samples per column like Glide); `max < min` misconfiguration (swap); persisted width for a column whose type changed (still honoured).
- **Test cases (input → expected):** widths [40, 42, 45, 41, 300, 44], header 60 → cand 45 → w = 60; [40, 42, 300] → 300; boolean header `is_active` 70 px → 70 (within [48, 80]); all NULL, header `x` → 48; 20,000 columns → 10 samples each, total ≤ budget; persisted 333 px → 333 without measurement.
- **i18n notes:** NULL placeholder width uses the localized/configured NULL label; numbers measured in the active display mode (raw or localized grouping changes widths).
- **Verification status:** Glide algorithm [Observed: `use-column-sizer.ts:15-150`]; defaults are our decision.
- **Prior art and evidence:** ui-02 ALG-GRID-02.
- **Used by:** the grid.

### HV-GRID-05: Selection model and keyboard/pointer map

- **Priority / depends on:** v1. Depends on HV-GRID-02, -06.
- **Sources merged:** ui-02 ALG-GRID-03 (anchor/focus, rectangles, row/column interval sets, select-all flag, keyboard semantics, normalization for copy, u128 counts, behaviour on result change), datagrip/08 §3.15 (gutter row selection, Ctrl+W grow/shrink, occurrence multi-select), gpui-component selection modes [Observed], rainfrog/gobang selection modes.
- **Problem:** Spreadsheet-grade selection over a virtual grid where most cells are not loaded, robust to streaming, column reorder/hide, local sort and transposition.
- **Inputs / outputs:** pointer and keyboard events, grid dimensions, column order → `Selection` + change events; derived queries (`contains`, `bounding_box`, `cell_count`, `iter_rects`, `normalized_for_copy`).
- **Decision:** ui-02 ALG-GRID-03 with two clarifications. (1) Selection is stored in **model coordinates for rows (via stable row references) and column ids**, so local sort/filter does not move selected data; rectangles are re-expressed as view runs at render time. (2) Ctrl/Cmd+W grows the selection: cell → column (or row, per setting) → all; Shift+Ctrl/Cmd+W shrinks (DataGrip).
- **Algorithm:**
  1. `Selection { anchor, focus, rects: Vec<Rect>, rows: IntervalSet<ViewRow>, cols: IntervalSet<ColId-order>, all: bool }` — rects hold `(row_ref_range in view order at creation, Vec<ColId>)`; on view-mapping changes, rects are rebuilt from the stored row references (row selection by identity when the mapping is permuted).
  2. Click → single cell; Shift+click → extend the active rect; Ctrl/Cmd+click → add a rect (multi-range); drag → update focus with edge autoscroll (speed ∝ distance, capped at 40 rows/frame).
  3. Row-gutter click/Shift/Ctrl → row interval set; header click (with modifier, setting) → column set; corner → all.
  4. Keyboard (focus moves; selection collapses unless Shift):

     | Key (Win/Linux) | macOS | Action |
     |---|---|---|
     | Arrows | Arrows | move ±1 |
     | Ctrl+Arrow | Cmd+Arrow | jump to the first/last row or column (not "next non-empty": unloaded data) |
     | Home / End | Home / End (Fn+←/→) | first/last column in the row |
     | Ctrl+Home / Ctrl+End | Cmd+↑ / Cmd+↓ | first/last row (last *loaded* row while streaming, then follows) |
     | PageUp / PageDown | Fn+↑/↓ | ± rows per viewport |
     | Tab / Shift+Tab | same | next/previous cell, wrapping rows (when not editing) |
     | Enter / F2 | Enter / F2 | start editing (HV-GRID-13) |
     | Esc | Esc | cancel edit or collapse the selection |
     | Ctrl+A | Cmd+A | select all (`all = true`, O(1)) |
     | Shift+Space / Ctrl+Space | same | select row / select column |
     | Ctrl+W / Ctrl+Shift+W | Option+↑ / Option+↓ (DataGrip mac) | grow / shrink |
     | Alt+J (find occurrence) | Ctrl+G | add next occurrence of the focused value (HV-GRID-09) |
  5. Membership: `all || rows ∋ r || cols ∋ c || any rect ∋ (r, c)`; > 32 rects → interval tree on rows.
  6. Count: inclusion–exclusion in u128 (`|rows|·C + |cols|·R − |rows|·|cols|` + rect areas, overlaps by sweep line when needed).
  7. Normalization for copy (doc 24): `all` → the whole grid; row/col sets → whole rows/columns; several rects → stack if they share the same column span, concatenate if they share the same row span, else refuse with `grid-copy-multiple-selection` (Excel rule) or copy the bounding box with blanks (setting).
  8. On result changes: generation change → clear (keep focus row index clamped); append → keep; tombstones → shift; column hide → drop hidden ids from rects; column reorder → per-column runs.
  9. Emit `selection_changed` once per frame; screen reader announcement via HV-GRID-16.
- **Data structures and Rust placement:** `hacivat-ui::grid::selection { Selection, IntervalSet, Rect }`.
- **Complexity / performance budget:** all operations O(log k) or O(#rects); select-all O(1); never materializes per-cell state.
- **Edge cases (exhaustive list):** empty grid; header corner; frozen and scrollable cells in one rect; Shift+click without an anchor; selection spanning unloaded rows; unknown total rows with Ctrl+End; hidden column inside a rect; column reorder after selecting; RTL UI (arrow keys follow visual direction); IME composition in the editor (arrows belong to the editor); read-only grid (selection still allowed); Shift+arrows while editing (text selection in the editor); overlapping rects (counted once); Ctrl+click toggling the only selected row; key repeat with autoscroll; per-instance selection in multiple windows; AltGr combos on Turkish layouts never bound; transposed view (selection operates in model coordinates; arrows follow the visual axes).
- **Test cases (input → expected):** click (2,3), Shift+click (5,1) → rect r[2,6) c[1,4), anchor (2,3), focus (5,1); then Ctrl+click (10,10) → two rects, the second active; row gutter 7, Shift 9, Ctrl 20 → rows {[7,10), [20,21)}; Ctrl+A on 10^9 × 10^4 → `all`, count 10^13 in u128; re-run → cleared; streaming append → unchanged; hide column 2 in rect c[1,4) → columns {1,3}; local sort after selecting row with id 42 → the selection follows id 42 to its new position.
- **i18n notes:** status bar `grid-selection-summary = { $rows -> [one] 1 row *[other] { $rows } rows }, { $cells -> [one] 1 cell *[other] { $cells } cells }` (Turkish forms share one plural category but still use selectors); copy refusal message `grid-copy-multiple-selection`.
- **Verification status:** design from ui-02 [Observed: Glide `CompactSelection`, gpui-component selection modes]. DataGrip grow/shrink [Observed: keymap]. macOS bindings for grow/shrink are [Inferred: DataGrip mac keymap] — verify.
- **Prior art and evidence:** ui-02 ALG-GRID-03.
- **Used by:** copy/paste, aggregators, delete rows, filter-by, record view, MCP "selected cells" context.

### HV-GRID-06: Model ↔ view index mapping

- **Priority / depends on:** v1. Depends on HV-RES-02 (tombstones, local appends) and HV-EDIT-02 (pending inserted rows).
- **Sources merged:** datagrip/08 ALG-GRID-13 (`ModelRow`/`ViewRow` newtypes, view permutation after local filter and sort, transposition at render, selection in model coordinates), dbeaver-04 ALG-DBV-GRID-09 recommendation (permutation vector, never discard edits).
- **Problem:** Translate between the order the user sees (after local sort, local filter, find-filter, deleted-row hiding and pending inserts) and the store's order, cheaply for 10M rows.
- **Inputs / outputs:** row count, tombstones, permutation (optional), filter mask (optional), pending inserts with anchors → `ViewMap` with `view_to_model(v)`, `model_to_view(m)`, `len()`.
- **Decision:**
  - **Identity fast path** (no local sort/filter): `view_to_model` = k-th row not in `tombstones` (roaring `select` on the complement → O(log n)), plus pending inserted rows spliced after their anchor rows. Zero extra memory per row.
  - **Permuted path** (local sort and/or filter active): `rows: Vec<RowRef>` where `RowRef = Model(u32 | u64) | Pending(TempId)`; `u32` storage when the loaded row count < 2^32. The inverse map (`model_to_view`) is built lazily, only when needed (find navigation, selection restore).
  - Pending inserted rows stay at their insertion position in both paths (they are not sorted or filtered until submitted) so that they never jump away while the user edits them.
  - Columns: `view_cols: Vec<ColId>` (order without hidden), frozen counts.
- **Algorithm:**
  1. `rebuild(inputs)` whenever tombstones, the permutation, the filter mask or pending inserts change (off the UI thread when > 100k rows; the old map stays active until the new one is ready).
  2. Permuted: `rows = sorted_model_rows.filter(mask).filter(!tombstoned)`, then splice pending rows after their anchors (or at the top/bottom per "add row" placement).
  3. Streaming appends with an active local sort: new rows are appended **unsorted** below a separator row "N new rows (not sorted) — Re-sort" (never re-sort automatically under the user's eyes); with an active filter, new rows are filtered incrementally.
  4. Transposed view: same map; axes swap at render (HV-GRID-12).
- **Data structures and Rust placement** (`hacivat-ui::grid::viewmap`):
  ```rust
  pub enum ViewMap { Identity { loaded: u64, holes: Arc<RoaringTreemap>, inserts: Arc<BTreeMap<u64, Vec<TempId>>> },
                     Permuted { rows: Arc<RowVec>, inverse: OnceCell<Arc<InverseMap>>, unsorted_tail: u64 } }
  pub enum RowRef { Model(u64), Pending(TempId), Separator }
  ```
- **Complexity / performance budget:** `view_to_model` O(log n) (identity) / O(1) (permuted); rebuild O(n) (filter) or O(n log n) (sort, done in HV-GRID-07); memory 4–8 bytes per row only when permuted.
- **Edge cases (exhaustive list):** tombstoned rows inside a permutation; pending insert anchored to a row that gets filtered out (keep the pending row visible right after the nearest visible predecessor); pending insert anchored to a deleted row; streaming appends with sort/filter active (unsorted tail); row counts ≥ 2^32 (u64 path); clearing sort restores identity; generation change (map reset); hidden columns plus frozen columns; transposition with 10^6 rows (becomes 10^6 columns → column virtualization handles it; transpose refused above 100k rows with a hint).
- **Test cases (input → expected):** 10 rows, tombstone {3} → view 3 → model 4; pending insert anchored after model 5 → view order …, 5, P1, 6…; sort desc on loaded 1M rows → `Permuted`, model_to_view built only when find navigates; filter hides model 5 with pending P1 anchored after it → P1 appears after model 4; append 1,000 rows during sort → unsorted tail separator with count 1,000.
- **i18n notes:** separator row `grid-unsorted-tail = { $count } new rows (not sorted) — Re-sort`.
- **Verification status:** DataGrip newtypes [Observed: `ModelIndex`, `ViewIndex`, `RawIndexConverter`]; design decisions ours.
- **Prior art and evidence:** datagrip/08 ALG-GRID-13.
- **Used by:** every grid feature that maps positions.

### HV-GRID-07: Local sort

- **Priority / depends on:** v1. Depends on HV-GRID-06, HV-GRID-10; the server-sort alternative is HV-RES-11.
- **Sources merged:** dbeaver-04 ALG-DBV-GRID-09 and Q1/Q12 (per-key comparators, never drop edits), datagrip/08 §3.14 (header click cycles, stacked sort with Alt+Click, level numbers, clear sorting), i18n/02 §3.6.2 (client-side sort uses the column's collation when known, else code point; hint "sorted locally"), verification/01 #39–#41 (Turkish orders).
- **Problem:** Sort already-loaded rows without a server round trip, matching database semantics as closely as possible, never losing pending edits.
- **Inputs / outputs:** sort keys `[(ColId, Asc|Desc, NullsPos)]`, loaded rows, column logical types and collations → a permutation for HV-GRID-06.
- **Decision:**
  - **When local sort is used:** console results that are `Complete` (or non-Read results), CSV/file grids, Redis/Mongo pages; tables use server ORDER BY by default (HV-RES-11). A banner "Sorted locally — the database might order differently" appears when the result is not complete.
  - **Comparators per key** (fixes DBeaver Q1): numbers numerically (exact decimals by `(mantissa, scale)` normalization; floats with NaN after +∞ like PG); booleans false < true; temporal by instant (TimestampTz by UTC); UUID by bytes; binary by bytes; text by **collation**: if the column's DB collation maps to an ICU locale (PG `tr-x-icu`, `tr_TR.utf8`, ICU `tr-TR`; MySQL `utf8mb4_tr_0900_ai_ci`; MSSQL `Turkish_CI_AS`, `Turkish_100_*`; etc.) use ICU4X `Collator` for that locale at the matching strength (CI → secondary, CS → tertiary; AI → primary); else **code-point order** (never the UI locale — data must not be ordered by UI-language rules); `Opaque`/JSON by text.
  - **NULL placement** defaults to the engine's default for the result's source (PG/Oracle: NULLs last ascending; MySQL/SQLite/MSSQL: first), switchable per key.
  - **Pending edits are kept**; edited cells sort by their **new** value; pending inserted rows stay at their anchors (HV-GRID-06). Rejected: DBeaver's `rejectChanges()` before sorting (Q12).
  - Header click cycles none → asc → desc → none; Alt+Click (configurable gesture) adds a key; level numbers shown.
- **Algorithm:**
  1. Extract keys for loaded rows into a compact array (`(model_row: u32/u64, key0 normalized, …)`) on a worker thread; text keys keep `Arc<str>` (no copies).
  2. Stable sort (`sort_by` / parallel merge sort when > 1M rows) with the comparator chain; ties by model order (stable).
  3. Publish the permutation to HV-GRID-06 atomically; preserve the selection by row reference and the focus cell; scroll to keep the focused row in view.
  4. Cancel when a new sort request arrives (generation token checked every 64k comparisons).
- **Data structures and Rust placement:** `hacivat-core::valfmt::collation::{collator_for(db_collation) -> Option<CollatorBorrowed>}` (core, so that exports and MCP agree), `hacivat-app::results::local_sort` (job), permutation consumed by `hacivat-ui::grid::viewmap`.
- **Complexity / performance budget:** O(n log n); numeric 1M rows ≤ 300 ms; text with collation 1M rows ≤ 2 s (progress indicator after 200 ms); the UI keeps the old order until done.
- **Edge cases (exhaustive list):** mixed types in a column (documents: order by type rank then value — Mongo BSON order: MinKey < Null < Numbers < Symbol/String < Object < Array < BinData < ObjectId < Boolean < Date < Timestamp < Regex < MaxKey); `Special` values (−infinity first, +infinity last); MySQL zero dates (before all dates); numeric strings in text columns (text order, not numeric — natural sort is a separate opt-in); collation unknown (code point + hint); case-insensitive collations (secondary strength; ties by tertiary then code point for determinism); Turkish `I/ı/İ/i` (tailored order verified: `ılık Irmak istek İzmir`); German `ß`; truncated values (compare the prefix; flag "approximate order"); `Error` cells (last); pending edits with `Unparsed` values (sorted as text at the end); very large selections of keys (memory ≤ 16 bytes per row per key); sort while streaming (unsorted tail, HV-GRID-06); transposed view (sorting by a row is not offered).
- **Test cases (input → expected):** `["b", "A", NULL, "a"]` ASC, code point, NULLs first (MySQL source) → `[NULL, "A", "a", "b"]`; the same with a PG source → `["A", "a", "b", NULL]`; Turkish collation column `[İzmir, ılık, Irmak, istek]` ASC → `[ılık, Irmak, istek, İzmir]` (verified tr order); decimals `[1.50, 1.5, 10]` → `[1.50, 1.5, 10]` (equal values keep model order); a cell edited from 5 to 50 sorts as 50; DBeaver Q1 regression: two keys of different types sort each by its own comparator.
- **i18n notes:** `sort-local-hint`, `sort-level = Sort level { $level }`; collation names are shown verbatim in the header tooltip.
- **Verification status:** [Observed: verification/01 #39–#40] tr orders for PG ICU and MySQL tr collations equal the CLDR tailoring — ICU4X `tr` collator must reproduce them (golden test from those word lists). MSSQL `Turkish_CI_AS` order not verified [Unknown].
- **Prior art and evidence:** dbeaver-04 Q1/Q12; i18n/02 §3.6.
- **Used by:** the grid; exports of "as displayed" order.

### HV-GRID-08: Local filter (value lists) and quick filter

- **Priority / depends on:** v1.x. Depends on HV-GRID-06, -10.
- **Sources merged:** datagrip/08 ALG-GRID-11 and §3.13 (Excel-like per-column distinct values with counts, AND across columns, filter by cell value, NULL vs empty distinct, toggle keeps state, loaded rows only), dbeaver-04 ALG-DBV-GRID-09 and Q10 (quick filter over `toString()` → use display strings; DBeaver blocks next-page fetches while active).
- **Problem:** Narrow the loaded rows by column values or a quick text search without touching the server.
- **Inputs / outputs:** per-column allowed value sets and/or a quick text predicate → a filter mask for HV-GRID-06 and a match count.
- **Decision:** DataGrip's value-list popup per column plus a global quick filter that matches **display strings** with the Turkish-tolerant search key (ALG-I18N-06). Value identity uses typed values (`ValueKey`), not display strings. While a local filter is active, the grid does **not** auto-fetch more rows (banner: "Filtering N loaded rows — Load more"), because rows appearing and disappearing while scrolling are confusing (DBeaver blocks next-page for the same reason).
- **Algorithm:**
  1. Opening the popup for column c: compute distinct `ValueKey`s with counts over loaded rows (worker thread; cap 10,000 distinct, sorted by count desc then value; a search box filters the list; "(NULL)" and "(empty)" entries separate).
  2. `allowed[c] = Some(set)`; mask = rows where every filtered column's value ∈ allowed and the quick predicate (if any) matches some visible column's display string.
  3. "Filter by this cell" sets `allowed[c] = {cell value}`.
  4. Pending edits: filter on the displayed (new) value; pending inserted rows always visible.
  5. Status: `grid-local-filter-matches = Number of matches: { $count }`.
  6. Turning the filter off keeps its conditions for later re-enabling.
- **Data structures and Rust placement:** `hacivat-app::results::local_filter { LocalFilter { allowed: HashMap<ColId, HashSet<ValueKey>>, quick: Option<SearchKey> } }`; `ValueKey` = normalized typed value (floats by value with ±0 merged and all NaNs merged, decimals normalized, text exact).
- **Complexity / performance budget:** distinct computation O(n) ≤ 200 ms for 1M rows; mask O(n × filtered columns) ≤ 100 ms for 1M rows.
- **Edge cases (exhaustive list):** only loaded rows (banner); 100k distinct values (cap + search); floats compared exactly by value (`-0.0` and `0.0` form one key; all NaNs form one key); case sensitivity (exact by default in value lists; quick filter is tolerant); values changed by pending edits; local filter + server WHERE (local applies after); filter on a truncated value (prefix key; flagged); JSON/array values (by canonical text); Missing vs NULL in document grids (separate entries).
- **Test cases (input → expected):** 3 rows `a, a, b` → popup `a (2), b (1)`; check `b` → 1 row, "Number of matches: 1"; NULL and empty string rows → two separate entries; quick filter `istanbul` matches `İSTANBUL` cells.
- **i18n notes:** `grid-local-filter-title = Local filter for { $column }`, `grid-local-filter-null = (NULL)`, `grid-local-filter-empty = (empty)`, `grid-local-filter-loaded-only`.
- **Verification status:** DataGrip popup behaviour [Observed: help, bundle]; FILTER-09/10 (doc 13) open.
- **Prior art and evidence:** datagrip/08 ALG-GRID-11; dbeaver-04 Q10.
- **Used by:** Hacivat.

### HV-GRID-09: Find on page with "filter rows"

- **Priority / depends on:** v1. Depends on HV-GRID-06, -10, ALG-I18N-06.
- **Sources merged:** datagrip/08 ALG-GRID-21 and §3.13 (Match case, Regex, Words, multi-line, Prev/Next, Filter rows, select all occurrences), dbeaver-04 §3.12 (find/replace over display strings; replace becomes pending edits), i18n/02 ALG-I18N-06 (Turkish-tolerant key with highlight mapping), datagrip/13 FILTER-10.
- **Problem:** Search loaded cells the way users see them, tolerant to Turkish `I/ı/İ/i` and accents, with exact highlighting, optionally hiding non-matching rows.
- **Inputs / outputs:** query, options `{match_case, regex, whole_words, accent_insensitive (default on), filter_rows}`, visible columns → matches `Vec<(ViewRow, ColId, Range<usize>)>` (byte ranges in display strings), current match, optional mask.
- **Decision:** search the **display strings** of visible columns (what the user sees; DBeaver's raw `toString()` is wrong, Q10). Default mode = Contains with ALG-I18N-06 keys (Turkish-tolerant, accent-insensitive). Match case = exact NFC substring. Regex = `regex` crate on display strings, case-insensitive flag uses default folding plus explicit `[iıİI]` expansion for `i`/`I`. Replace (v1.x) turns each replacement into a pending edit through the cell parser (HV-GRID-11). NULL cells never match (the NULL marker is not data).
- **Algorithm:**
  1. Build/refresh the search key per cell lazily (cache keyed by `(generation, version, row, col)`), only for cells visited.
  2. Incremental scan on a worker thread in view order starting from the focus cell, yielding matches in batches of 1,000; the UI highlights visible matches immediately and updates "k of N" as the count grows; a new query cancels the old scan.
  3. Next/Prev cycle with wrap; navigation scrolls into view (HV-GRID-02).
  4. Filter rows: mask = rows with ≥ 1 match → HV-GRID-06.
  5. Select all occurrences → the selection becomes one rect per matched cell (capped at 10,000 cells, then a warning).
  6. Highlight ranges map from the folded key back to the original display string (ALG-I18N-06 step 5), extended to grapheme boundaries.
- **Data structures and Rust placement:** `hacivat-core::valfmt::search::SearchKey` (shared with history and object-tree search), `hacivat-ui::grid::find::FindModel` (matches, current, mask) — the generic grid accepts a match provider from the adapter.
- **Complexity / performance budget:** first match ≤ 50 ms; 1M cells scanned ≤ 500 ms; memory for keys bounded by an LRU (100k cells).
- **Edge cases (exhaustive list):** NULL cells (no match); truncated values (prefix only; result marked "may be incomplete"); multi-line values (multi-line toggle allows `\n` in the query); 1M+ cells (incremental, cancel); hidden columns ignored; localized display mode (search what is displayed: `1.234,5` in tr); invalid regex (red field, no scan); regex catastrophic patterns (linear engine, none); RTL text with bidi controls (dropped from keys, highlight includes them); emoji; Turkish dotted/dotless I (tolerant by default); whole-word with Turkish letters (word boundaries via ICU4X segmenter, not ASCII `\b`); search during streaming (new rows scanned as they arrive).
- **Test cases (input → expected):** "john" (case-insensitive) on the screenshot data → 4 rows with Filter rows on; `istanbul` matches `İSTANBUL` and highlights the whole `İSTANBUL`; `musteri` matches `müşteri` with accent-insensitive on, not with it off; regex `^\d{3}$` matches `123` cells only; NULL cells with query `null` → no match.
- **i18n notes:** find bar strings `find-placeholder`, `find-match-case`, `find-regex`, `find-words`, `find-filter-rows`, `find-count = { $current } of { $total }`; folding rules are locale-independent by design (always tolerant), so a Turkish and an English UI find the same matches.
- **Verification status:** DataGrip toggles [Observed: help screenshot]; ALG-I18N-06 test vectors [Observed: Python checks of Unicode folding in i18n/02].
- **Prior art and evidence:** as merged.
- **Used by:** the grid, the text view, the record view.

### HV-GRID-10: Cell display formatting and column display types

- **Priority / depends on:** v1. Depends on HV-RES-01. Lives in `hacivat-core::valfmt`.
- **Sources merged:** datagrip/08 ALG-GRID-15 (format step), §3.12 (binary display types Detect/Hex/Hex+ASCII/UUID/UUID swapped/Text; numeric display types Number/Timestamp s/ms; per-column highlighting language; boolean text/checkbox; NULL rendering), dbeaver-04 §3.6 (display modes UI/EDIT/NATIVE; formatter profiles; ID columns excluded from grouping; epoch transformer tests), i18n/02 ALG-I18N-07 (format algorithm), §3.8.2 (raw default, never round silently, ID columns never grouped, scale significant), §3.9 (dates, session time zone, specials), §3.15 (render hazards).
- **Problem:** Turn a `Value` into the exact text a cell shows, consistently across grid, record view, copy-as-displayed, exports and MCP, never losing precision silently, honouring per-column display types.
- **Inputs / outputs:** `ValueRef`, column logical/native type, `DisplaySettings { mode: Raw | Localized | Mixed, format_locale, time_zone, number_overrides, date_patterns, bool_mode, binary_display, number_display, null_label, max_display_chars }`, purpose (`Cell | Tooltip | CopyDisplayed`) → `Display { text, kind, flags (truncated_indicator, lossy, rtl, has_controls), align }`.
- **Decision:**
  - Three display modes: **Raw** (default: canonical server-like text — `1234567.50`, ISO dates `2026-11-11 14:05:09.123456+03`), **Localized** (CLDR for the *format locale*, which may differ from the UI language: tr `1.234.567,50`, `11.11.2026 14:05:09`), **Mixed** (grouping on, `.` decimal, ISO dates). Choice per user, overridable per connection and per column.
  - **Never round silently:** decimals show their full scale; if a localized display limits fraction digits (user setting), the cell shows a `≈` indicator and the tooltip shows the raw value.
  - **ID-like columns are never grouped** (PK columns, `*_id`, integer columns with sequence/identity defaults).
  - **Column display types** (per column, persisted like widths): binary → Detect (16-byte → UUID, image magic → image chip, valid UTF-8 → text, else hex), Hex (`\x…`), Hex + ASCII, UUID, UUID (time-low/high swapped, MySQL `UUID_TO_BIN(x,1)`), Text (choose encoding); integer → Number, Timestamp from epoch seconds, ms, µs, ns (UTC or display zone); text → plain, JSON (single-line preview), XML, SQL (highlighting language in the value viewer).
  - DBeaver's three purposes (UI/EDIT/NATIVE) become: grid **display** (this spec), **edit seed** = raw canonical text (HV-GRID-13), **SQL literal** (HV-EDIT-09 renderer).
- **Algorithm (per logical type):**
  1. `Null` → `null_label` styled (kind Null); `Missing` → `—` faint; `Default` → `DEFAULT` styled; `Error` → `⚠ <message>` (kind Error); `Unparsed` → the raw user text with the unparsed flag; `Truncated` → format the prefix and set `truncated_indicator`; `Opaque` → server text verbatim.
  2. Integers: raw → decimal digits; localized → ICU4X `DecimalFormatter` (grouping off for ID columns); UInt/i128 exact.
  3. Decimals: raw → `mantissa/10^scale` exact with the server scale; localized → `fixed_decimal::Decimal::try_from_str(canonical)` → `DecimalFormatter` (exact, arbitrary precision); never via f64.
  4. Floats: shortest round-trip representation (Rust `{}` / ryu semantics) → raw; localized via the same exact path on that string; `NaN`, `Infinity`, `-Infinity` as the server spells them (raw) or CLDR `∞` (localized), `-0` kept for floats.
  5. Booleans: text `true/false` (raw), or checkbox glyph (setting); localized mode does not translate `true/false` (they are SQL literals) unless the user enables "translated booleans".
  6. Temporal: Date `YYYY-MM-DD` (raw) / CLDR short date (localized, tr `d.MM.y`); Timestamp with the server precision (trailing zeros of fractional seconds trimmed to the declared precision, not beyond); TimestampTz converted to the connection's display zone with the offset shown (`+03` raw style); Time/TimeTz; Interval in PG style (`1 day 02:00:00`) raw, localized as a duration phrase only in tooltips; Specials (`infinity`, `-infinity`, `0000-00-00`) verbatim; years outside 1–9999 in ISO extended form with a sign.
  7. UUID: lowercase canonical hyphenated (MSSQL uniqueidentifier converted from mixed endian).
  8. Binary: per display type; Text display decodes with the chosen encoding and marks lossy bytes `\xNN`.
  9. JSON: compact single-line preview (whitespace collapsed, preserving the original key order and number spelling) truncated for the cell; pretty in the value viewer.
  10. Arrays/composites: PG literal style raw (`{1,2,3}`, `(a,b)`) or JSON style (setting); nested truncated.
  11. Geometry: `SRID=4326;POINT(…)` WKT prefix if decodable, else hex.
  12. Text: as is, with the control/bidi visualization done by the renderer (HV-GRID-03); `max_display_chars` (default 1,024) cap for the cell string.
- **Data structures and Rust placement** (`hacivat-core::valfmt::display`):
  ```rust
  pub enum DisplayMode { Raw, Localized, Mixed }
  pub struct DisplaySettings { pub mode: DisplayMode, pub format_locale: icu_locale::Locale, pub tz: jiff::tz::TimeZone,
      pub null_label: Arc<str>, pub bool_mode: BoolMode, pub max_display_chars: u32, pub max_fraction_digits: Option<u8> }
  pub enum BinaryDisplay { Detect, Hex, HexAscii, Uuid, UuidSwapped, Text(EncodingRef) }
  pub enum NumberDisplay { Number, EpochSeconds, EpochMillis, EpochMicros, EpochNanos }
  pub fn format_cell(v: ValueRef<'_>, col: &ColumnMeta, s: &DisplaySettings, purpose: Purpose) -> Display;
  ```
  The grid adapter caches `Display` per `(generation, version, row, col, settings_version)`.
- **Per-DBMS matrix (display specifics):** PG `money` → server text (lc_monetary); PG `interval` → PG style; MySQL `TIME` beyond 24 h (`838:59:59`) → as is; MySQL `YEAR` → 4 digits; MSSQL `datetime` 3.33 ms precision → 3 fractional digits; SQLite dates stored as text/real/int → Text by default, display type "SQLite date (ISO / Julian / Unix)" opt-in; ClickHouse `DateTime64(p, tz)` → the column's zone, not the session's.
- **Complexity / performance budget:** ≤ 1 µs per scalar cell in raw mode; ≤ 5 µs localized (ICU4X formatter instances cached per settings version); cached per visible cell.
- **Edge cases (exhaustive list):** negative zero; very small/large doubles (scientific in raw form as the shortest repr gives); DECIMAL(38,10) exact; PG `infinity` timestamps; MySQL zero dates; Oracle DATE with time (later); time 24:00:00; leap seconds (`:60` from servers that allow it → verbatim); DST gaps and overlaps in the display zone (show the offset); Turkish locale (`,` decimal, `.` grouping, `d.MM.y`); Arabic-Indic digits in localized mode for `ar` format locales (CLDR default numbering system); ID columns; `BIGINT UNSIGNED` max; JSON with numbers beyond f64 (verbatim text); invalid UTF-8 text (lossy); binary that looks like UTF-8 but is not intended (Detect picks text only when valid UTF-8 and ≥ 90% printable); 16-byte binary that is not a UUID (Detect shows UUID by DataGrip convention — user can switch per column); epoch values out of range for timestamps (show the number with a warning flag); Truncated JSON (no pretty-print); booleans in MySQL `TINYINT(1)` (shown as numbers unless PD-9); NULL label customised to Turkish `BOŞ` (allowed but copy-raw still writes the NULL text policy of the clipboard spec).
- **Test cases (input → expected):** raw `Decimal{123456750, 2}` → `1234567.50`; localized tr → `1.234.567,50`; localized tr on an ID column `Int(1234567)` → `1234567`; float `0.1f32` → `0.1`; `-0.0f64` → `-0`; PG `timestamptz '2026-11-11 07:00:00Z'` with display zone Europe/Istanbul → `2026-11-11 10:00:00+03` (raw) / `11.11.2026 10:00:00` (tr localized; tooltip ISO + zone); 16-byte `bytea` → UUID text; same column display type Hex → `\x…`; epoch-ms column `0` → `1970-01-01 00:00:00.000` (UTC), `-1` → `1969-12-31 23:59:59.999` (DBeaver epoch tests); `'infinity'::date` → `infinity`; JSON `{"a": 1.000}` preview → `{"a": 1.000}` (number spelling kept).
- **i18n notes:** format locale ≠ UI language (a Turkish UI may use en-US formats and vice versa, i18n/02 §3.6.2); all patterns come from CLDR via ICU4X — no per-language code; `value-null` label default ASCII `NULL`.
- **Verification status:** CLDR 49 Turkish symbols/patterns [Observed: verification/02 S34]; DBeaver epoch and number tests [Observed: `EpochTimeAttributeTransformerTest.java:93-134`, `NumberDataFormatterTest.java:44-133`] reused as vectors; fluent-rs `NUMBER()` is locale-insensitive [Observed: verification/02 P31] → numbers are never formatted through Fluent.
- **Prior art and evidence:** datagrip/08 ALG-GRID-15; i18n/02 ALG-I18N-07.
- **Used by:** grid, record view, value viewer, copy-as-displayed (HV-XFER-02), export "as displayed", MCP (raw mode only), aggregators' output formatting.

### HV-GRID-11: Cell input parsing (lenient, round-trip, ambiguity)

- **Priority / depends on:** v1. Depends on HV-RES-01. Lives in `hacivat-core::valfmt::parse`.
- **Sources merged:** i18n/02 ALG-I18N-07 (number parse: machine grammar, regional grammar from CLDR symbols, ambiguity, batch evidence, canonicalization, decimal scale rounding preview, specials), datagrip/08 ALG-GRID-15 parse step (booleans, temporal patterns then ISO then epoch, TZ offset range, UUID, binary hex, JSON) and edge cases, datagrip/08 §3.8 (typed error messages, boundary value prompt), dbeaver-04 Q5 (no-change detection by string forms is wrong), i18n/02 §3.9.2 (date input rules: ISO first, regional short pattern, never US `M/d/y` unless en-US, two-digit years rejected).
- **Problem:** Accept what users type or paste in machine or regional format, produce an exact canonical value for the column type, and flag — never guess — ambiguous input such as Turkish `1.234`.
- **Inputs / outputs:** text, column type (logical + native), `ParseContext { format_locale, time_zone, source: Typed | PasteInternal | PasteExternal, column_evidence }` → `ParseOutcome::Ok{value, notes} | Ambiguous{machine, regional} | Err(ParseError{kind, hint})`.
- **Decision:** ALG-I18N-07 for numbers verbatim; the other types follow the rules below. Parsing is mode-independent (raw/localized display only affects prompts and previews). Equality for "no-op edit" detection compares **typed values**, not strings (fixes DBeaver Q5). For an integer column, typing `1.0` over `1` is no change. For a decimal column, `1.0` over `1` is a change, because the scale differs and the server would store the new scale.
- **Algorithm (non-numeric types):**
  1. Trim Unicode whitespace; strip bidi/format characters (ALG-I18N-07 step 2) except inside Text columns (text is stored verbatim, including leading/trailing spaces; trimming of text is a paste option, off by default).
  2. Boolean: `true/false/t/f/1/0/yes/no/y/n/on/off` (ASCII case-insensitive) + localized words when enabled for the UI language (`evet/hayır/doğru/yanlış`); otherwise `Err(ExpectedBoolean)`.
  3. Date: ISO `YYYY-MM-DD` (and extended years) → then the format locale's short pattern (tr `d.M.y` accepting 1–2 digit day/month, 4-digit year) → else error; two-digit years rejected; US `M/d/y` only if the format locale is en-US.
  4. Timestamp: ISO-8601 with `T` or space, optional fraction (up to 9 digits; more than the column precision → note `will_round`), optional offset `Z/±HH[:MM]`; offsets must lie in −12:00…+14:00 (DataGrip rule) else `Err(OffsetOutOfRange)`; for `timestamp` (naive) an offset is an error unless "convert" is chosen; for `timestamptz` a missing offset means the display zone; then the regional pattern `d.M.y H:mm[:ss]`; then epoch digits **only** when the column display type is epoch.
  5. Time: `HH:MM[:SS[.f]]`, `24:00:00` allowed for PG; TimeTz offset as above.
  6. Interval (PG): PG input syntax and ISO 8601 durations (`P1DT2H`) → passed to the server as text with a client-side syntax check only.
  7. UUID: 32 hex or 36-char hyphenated, braces allowed, case-insensitive → canonical lowercase.
  8. Binary: per display type: Hex (`\x…`, `0x…`, plain hex), Base64 (opt-in), Text (encode with the chosen encoding), UUID display → 16 bytes (swapped variant honoured).
  9. JSON: validated with a strict parser; keep the user's text (preserve spacing for `json`, server normalizes `jsonb`).
  10. Enum: must equal one of the labels (exact, case-sensitive); suggestions via edit distance.
  11. Arrays: PG literal `{…}` or JSON array (setting); element parsing recursive.
  12. Text/varchar(n): length check against `max_chars` → note `will_truncate` or error per DB mode (MySQL strict mode errors, non-strict truncates).
  13. Out-of-range (numbers, dates): `Err(OutOfRange{boundary})` with the boundary value suggestion (DataGrip "Value is out of range. Set …?").
  14. `PasteInternal` (Hacivat's private clipboard flavour) → skip parsing, use the typed value (HV-XFER-03).
- **Data structures and Rust placement:** `hacivat-core::valfmt::parse { parse(text, &ColumnMeta, &ParseContext) -> ParseOutcome; enum ParseErrorKind { Empty, InvalidNumber, OutOfRange, Overflow, ExpectedBoolean, ExpectedDate, ExpectedTimestamp, OffsetOutOfRange, InvalidUuid, InvalidHex, InvalidJson, UnknownEnumLabel, TooLong, ExponentNotAllowedForInt, NotFinite, AmbiguousYear } }`.
- **Per-DBMS matrix (parse-relevant):** PG numeric NaN/±Infinity allowed (≥ 14 for infinity); MySQL zero dates allowed only when `sql_mode` lacks `NO_ZERO_DATE` (read at connect); MSSQL `datetime` range 1753–9999 (out-of-range error with boundary); SQLite accepts anything (parse by declared affinity, warn on mismatch rather than error).
- **Complexity / performance budget:** O(n) per cell; 100k pasted cells ≤ 100 ms.
- **Edge cases (exhaustive list):** all 30 numeric edge cases of ALG-I18N-07 (Turkish `1.234` ambiguous for decimals, `1.234` → 1234 for ints, `1,5`, `12.34.56` invalid, fr narrow-NBSP grouping, Arabic-Indic digits, U+2212 minus, `+5`, `-0`, `007`, `.5`, `5.`, exponents rejected for ints, overflow, scale rounding preview, `%25` rejected, currency for money only); dates `1.2.2026` and `01.02.2026` in tr; `3.` in Turkish is an ordinal, never `3.0` (number grammar requires digits after `.` in regional mode, and machine mode accepts `5.` → `5` only for numeric columns — documented); DST gap local times for `timestamptz` input (`2026-03-29 03:30` in a zone where that time does not exist → `Err` with a hint, never shifted silently); leap second `23:59:60` (rejected unless the engine accepts it); text with trailing spaces (kept); NUL characters in text (rejected for PG); JSON with duplicate keys (accepted; warning); booleans `d`/`g`/`c` letters (DataGrip text-mode special values → only via explicit actions in Hacivat, not parse).
- **Test cases (input → expected):** tr Decimal `1.234` → `Ambiguous{1.234, 1234}`; tr Int `1.234` → `Ok(1234)` + regional note; tr Decimal `1.234,5` → `Ok(1234.5)`; en Decimal `1,234.5` → `Ok(1234.5)`; `2026-11-11T10:00:00+03:00` into timestamptz → `Ok` (displays `2026-11-11 10:00:00+03` in Europe/Istanbul); `+15:00` offset → `Err(OffsetOutOfRange)`; tr date `1.2.2026` → `2026-02-01`; en-US date `2/1/2026` → `2026-02-01`, but in tr → `Err(ExpectedDate)`; UUID `{6F9619FF-8B86-D011-B42D-00C04FC964FF}` → canonical lowercase; `evet` into boolean with Turkish words enabled → `true`; enum `Active` vs label `active` → `Err(UnknownEnumLabel)` with suggestion `active`; Decimal(10,2) `0,125` in tr → `Ok(0.125)` + `will_round(2)` preview `0.13` (PG rounds half away from zero [Inferred]).
- **i18n notes:** error hints show examples in both machine and regional form (`parse-hint-number = Use 1234.5 or { $regional }`); the ambiguity prompt `parse-ambiguous = Is "{ $input }" { $machine } or { $regional }?`; batch evidence prompt per column for pastes (HV-XFER-03).
- **Verification status:** ALG-I18N-07 vectors [Observed: CLDR 49 data via ICU4X]; PG rounding mode [Inferred — verify in Docker]; DataGrip error messages [Observed: bundle].
- **Prior art and evidence:** as merged.
- **Used by:** cell editing (HV-EDIT-03), paste (HV-XFER-03), import conversion (HV-XFER-09), parameter dialog (HV-EXEC-03), filter chips (HV-RES-12).

### HV-GRID-12: Alternate views — transposed, record, text, tree

- **Priority / depends on:** v1 for transposed, record and text views; v1.x for the tree view and document flattening. Depends on HV-GRID-02, -05, -06.
- **Sources merged:** datagrip/08 §3.11 (value editor with Value/Record/Aggregates tabs; record view 1/2-column layout), §3.16 (Table/Tree/Text + Transpose; auto-transpose Never/If one record/Always, not for query results), dbeaver-04 §3.7 (record mode toggle; auto-switch when ≤ 1 row; plain-text presentation with max column width 255), ALG-MONGO-02 (document → table flattening), ALG-MONGO-03 (JSON tree virtualization with Fenwick sizes and chunking), datagrip/08 ALG-GRID-23 (JSON loader semantics for file grids, referenced by doc 24).
- **Problem:** Offer views better suited than a wide table for one record, very wide rows, nested documents and plain-text sharing — with the same data, selection and editing semantics.
- **Inputs / outputs:** the same `GridDataSource` and view state → alternate renderers.
- **Decision:**
  - **Transposed:** the same grid element with axes swapped at render; selection and editing stay in model coordinates; offered for results up to 100k rows (beyond that the transposed width is unusable → refused with a hint). Auto-transpose setting for *table editors* (DataGrip: Never / If one record / Always); query results never auto-transpose (DataGrip rule) — but a one-row result offers a "Record view" button.
  - **Record view:** side panel (dock right or bottom) for the focused row: Field | Value (two-column) or label-above-value (single column) layouts; editable through the same cell parser; NULL/DEFAULT actions; navigation ▲▼ between rows.
  - **Text view:** read-only, virtualized monospace text of the current page or selection rendered by the selected extractor (default "Pretty" ASCII table, HV-XFER-06), with find; never for > 100k rows without confirmation.
  - **Tree view (v1.x):** for JSON/JSONB/array/composite cells and for document collections; ALG-MONGO-03 algorithm (lazy node tape, visible-size Fenwick tree per expanded large container, synthetic chunks of 100 children, bounded "expand all", background search); rows rendered through HV-GRID-02's single-column mode.
  - **Document flattening (v1.x):** Mongo collections and JSON columns as tables per ALG-MONGO-02 (columns from schema + seen keys, `_id` first, nested documents expanded to depth 1 when ≤ 10 sub-fields, arrays never expanded into columns, Missing ≠ NULL, drill-down breadcrumb).
  - **Value viewer** (Shift+Enter): Value / Record / Aggregates tabs; text with soft wrap and formatting toggle (pretty JSON/XML); image preview for image bytes; hex view for binary; highlighting language per column; Enter = newline, Ctrl/Cmd+Enter = apply, Esc = revert and leave.
- **Algorithm:** transposed rendering = swap the scroll model's axes and the column spec provider (column headers become row headers); the record view reads `frame.cell(focus_row, col)` for all columns (virtualized list when > 200 columns); the text view streams extractor output into a rope-backed read-only buffer; the tree view per ALG-MONGO-03 steps 1–9.
- **Data structures and Rust placement:** `hacivat-ui::grid` (transpose flag), `hacivat-app::results::{record_view, text_view, value_viewer}`, `hacivat-ui::tree::VirtualTree` (generic tree with Fenwick visible sizes), `hacivat-core::results::doc_flatten` (column model for documents).
- **Complexity / performance budget:** record view switch ≤ 1 frame; text view of a 500-row page rendered ≤ 50 ms; tree view: 50 MB JSON tape built in the background, first paint after the root rows; row lookup O(depth log n).
- **Edge cases (exhaustive list):** transposing a result with 10k columns (rows of the transposed view = 10k → fine); transposed + local sort (sort by a row not offered); record view on a pending inserted row (editable); record view on a deleted row (read-only, struck through); record view with duplicate column names (show ordinal suffix); text view with binary cells (display type rules); tree view: very deep nesting (iterative traversal), 1M-element arrays (two-level chunking), duplicate JSON keys (all shown, warning), invalid JSON in a text column (raw with error position), numbers beyond f64 (source text), expand state preserved by path across refresh; document flattening: keys with `.`/`$`, sparse fields hidden under "+N sparse fields", fields appearing in later pages appended at the end (never reordered), Mixed type columns.
- **Test cases (input → expected):** 1-row table with auto-transpose "If one record" → opens transposed; a 1-row query result → not transposed, "Record view" button shown; record view edit of `city` → the same pending change as editing the cell; text view of `[[1,"a"],[2,"b"]]` with the Pretty extractor → an ASCII box table; tree view of an array of 1,000,000 numbers → 1 + 100 group rows, instant scroll; Mongo docs `{_id:1,a:{b:1,c:2}}`, `{_id:2,a:{b:3}}` at depth 1 → columns `_id, a.b, a.c`, doc 2 `a.c` Missing.
- **i18n notes:** `view-table`, `view-transposed`, `view-record`, `view-text`, `view-tree`, `record-layout-one-column`, `record-layout-two-columns`, `tree-chunk = [{ $from }…{ $to }]`, `doc-sparse-fields = +{ $count } sparse fields`; field names are data (never translated).
- **Verification status:** DataGrip view modes [Observed: help, bundle]; Compass behaviours [Observed: SSPL code read for behaviour only]; DBeaver record auto-switch [Observed: `ResultSetViewer.java:2295-2330`].
- **Prior art and evidence:** as merged.
- **Used by:** Hacivat.

### HV-GRID-13: Cell editor lifecycle and value viewer editing

- **Priority / depends on:** v1. Depends on HV-GRID-05, -10, -11; feeds HV-EDIT-02/03.
- **Sources merged:** datagrip/08 §3.8 (start: Enter/F2/double-click/typing; confirm Enter, cancel Esc; multi-cell typing writes into every selected cell; value completion from loaded values; editors per type; Set NULL / Set DEFAULT / Generate UUID / Load File; read-only cells offer View), ui-02 ALG-GRID-03 (Enter starts editing or moves down; IME composition), dbeaver-04 §3.7 (INLINE / PANEL / EDITOR edit types; Q5), i18n/02 §3.8.3 (editor pre-fills the raw value; live preview of the interpretation).
- **Problem:** Edit cells naturally with keyboard and mouse, with IME support, typed editors, and a live preview of how the input will be interpreted — then hand the result to the pending-changes overlay.
- **Inputs / outputs:** focus cell, key/mouse events, editability from the source → `EditCommitted { cells: Vec<ModelCell>, text | special: Null | Default | GeneratedUuid | File(path) }` to the adapter (which parses via HV-GRID-11 and writes HV-EDIT-02).
- **Decision:**
  - **Activation:** Enter or F2 (edit with the current raw value, caret at the end), double-click, or typing a printable character (replaces the content with that character — spreadsheet behaviour). Read-only cells open the value viewer in view mode with the read-only reason.
  - **In-cell editor** = a GPUI text input positioned over the cell (supports IME composition through GPUI's input handler); multi-line values open the value viewer instead (in-cell editing of values containing `\n` would hide content).
  - **Keys inside the editor:** Enter commits and moves down (setting: stay); Tab commits and moves right; Shift+Enter / Shift+Tab move up/left; Alt/Option+Enter inserts a newline (switches to the value viewer); Esc cancels; Cmd/Ctrl+Enter commits and **submits** pending changes (DataGrip Submit).
  - **Live preview line** under the editor: shows the parsed interpretation for numbers/dates in the user's display mode (`= 1.234,50` in tr), ambiguity choices, and errors; committing an invalid value is allowed and kept as `Unparsed` (HV-EDIT-03) — the user is not trapped in the editor.
  - **Typed editors:** boolean (checkbox toggle with Space; three-state with NULL when nullable), date/time (text with a calendar popover), enum (dropdown filtered as you type), JSON/XML (value viewer with validation), binary (hex editor or Load File), UUID (text + Generate).
  - **Special actions:** Set NULL (Ctrl+Alt+N / Cmd+Option+N), Set DEFAULT (Ctrl+Alt+D), Generate UUID, Load File…, Clear field (empty string for text, NULL otherwise — explicit menu wording).
  - **Multi-cell edit:** typing with several cells selected writes the same parsed value to every editable selected cell in one undo step (read-only cells skipped and counted).
  - **Value completion** (v1.x): Ctrl+Space offers distinct values loaded in that column (local), FK target values (when the column is an FK, via a side query, capped at 100).
  - **Truncated values:** entering edit mode first loads the full value (HV-RES-08); the editor shows a spinner until then.
- **Algorithm:**
  1. `begin_edit(cell, seed)`: `source.edit_requested(cell, seed)` → `Allowed(kind, initial_raw_text)` or `ReadOnly(reason)`; create the editor element with the initial text (raw canonical, HV-GRID-10 purpose EditSeed).
  2. While typing: debounce 50 ms → `parse(text)` → update the preview line.
  3. Commit: send `text` (or the special) → the adapter parses → overlay update → `EditResult::{Ok, Unparsed(err), Ambiguous(choices)}`; on `Ambiguous` keep the editor open with the two choices as buttons (keyboard 1/2).
  4. Cancel: discard; restore focus.
  5. Undo/redo (Cmd/Ctrl+Z / Shift+Cmd/Ctrl+Z) operate on the overlay's change stack (HV-EDIT-10), not on the editor once committed.
- **Data structures and Rust placement:** `hacivat-ui::grid::editor::{EditorModel, EditorKind}`; parsing and overlay in the adapter (`hacivat-app::results::edit_bridge`).
- **Complexity / performance budget:** editor opens ≤ 1 frame; preview update ≤ 1 ms per keystroke; IME composition latency per GPUI (spike IME test of ADR 0001).
- **Edge cases (exhaustive list):** IME composition (Japanese/Chinese; Turkish dead keys) — Enter during composition commits the composition, not the cell; editing a key column (allowed; WHERE uses the original, HV-EDIT-04); editing a row that got deleted on refresh (edit cancelled with a notice); generation change during editing (commit goes to the row identity if it still exists, else the edit is kept as an orphan pending change, HV-EDIT-02); editing a pending inserted row; editing a truncated LOB (full load first); editing in the transposed view; editing with a local sort active (the row may move after commit — focus follows the row); paste into an open editor goes into the editor text (not a grid paste); a very long value in the in-cell editor (switch to the value viewer above 1,000 chars); NUL characters; RTL text input; read-only reasons from the source (expression column, no identity, read-only connection, Karagöz policy).
- **Test cases (input → expected):** type `5` on a numeric cell → editor with `5`; Enter → value 5 committed, focus moves down; F2 on `1.50` (decimal) → editor shows `1.50` raw even in tr localized mode; type `1.234` in a tr decimal cell → preview offers `1.234` / `1234`, pressing 2 commits 1234; Esc → no change; Ctrl+Alt+N on 3 selected cells → 3 NULL edits in one undo step; a read-only expression column → the value viewer opens with "Column is an expression".
- **i18n notes:** `edit-preview = = { $value }`, `edit-ambiguous-choice`, `edit-set-null`, `edit-set-default`, `edit-generate-uuid`, `edit-load-file`, `edit-read-only = Read-only: { $reason }`; boolean three-state labels; calendar uses CLDR first day of week (Monday for tr).
- **Verification status:** DataGrip behaviours [Observed: help, bundle]; GPUI IME support must pass the ADR 0001 IME spike.
- **Prior art and evidence:** as merged.
- **Used by:** Hacivat grid, record view, value viewer.

### HV-GRID-14: Go to row

- **Priority / depends on:** v1. Depends on HV-GRID-06.
- **Sources merged:** datagrip/08 ALG-GRID-19 (`<row>`, `<row>:<column>`, `<row>,<column>`; 1-based; column by index or name).
- **Problem:** Jump to a row (and column) by number or name.
- **Inputs / outputs:** input text → `(ViewRow, Option<ColId>)` or an inline error.
- **Decision:** DataGrip syntax; rows are 1-based **view** positions (what the user sees, after local sort/filter); a row beyond the loaded count in a suspended result triggers fetching up to that row (bounded by the memory budget, with confirmation above 1M rows); a row beyond the known total → error.
- **Algorithm:** trim; split at the first `:` or `,` not inside double quotes; row = parse an integer (locale digits allowed, grouping separators allowed — `1.000` in tr = 1000, since rows are integers); column = integer (1-based visible order) or name (exact, then case-insensitive ASCII, then Turkish-tolerant key; quoted names allowed); validate ranges; scroll and focus.
- **Data structures and Rust placement:** `hacivat-ui::grid::goto::parse_goto(text, &ColumnModel, locale) -> Result<GotoTarget, GotoError>`.
- **Complexity / performance budget:** O(columns) for name lookup.
- **Edge cases (exhaustive list):** `0` or negative; beyond the total; a column name containing `:` or `,` (quoted); hidden columns (error with "column is hidden — show it?"); transposed view (row/column refer to model axes); filtered view (visible rows); duplicate column names (first match + warning); Turkish `İD` vs `id` (tolerant match).
- **Test cases (input → expected):** `12` → row 12, current column; `12:3` → row 12, column 3; `12,name` → column `name`; `abc` → error; `1.000` in tr → row 1000; `2:"a:b"` → row 2, column `a:b`.
- **i18n notes:** `goto-placeholder = <row>[:<column>]`, `goto-error-row`, `goto-error-column`.
- **Verification status:** syntax [Observed: bundle `row.column.or.row.column`].
- **Prior art and evidence:** datagrip/08 ALG-GRID-19.
- **Used by:** Hacivat.

### HV-GRID-15: Column statistics header

- **Priority / depends on:** later. Depends on HV-GRID-10.
- **Sources merged:** datagrip/08 ALG-GRID-20 (Off/Compact/Detailed; null count, distinct (exact then HyperLogLog), min/max, top-K (Space-Saving), mean; incremental; skip LOB/binary).
- **Problem:** Summarize loaded values per column above the header.
- **Inputs / outputs:** loaded rows per visible column → per-column stats → compact or detailed header panels.
- **Decision:** as ALG-GRID-20, computed lazily per **visible** column on a worker thread, debounced 200 ms after page loads; "whole table" statistics only on explicit request (server-side `SELECT count(*), count(DISTINCT c), min(c), max(c)` with a timeout).
- **Algorithm:** per column: `nulls`, `distinct` (exact HashSet up to 100k then HyperLogLog++ precision 14), `min/max` (comparable types, collation-aware for text via HV-GRID-07 comparators), top-5 values (Space-Saving with 50 counters), mean/stddev for numerics (Welford, exact decimals via rust_decimal fast path, else f64 with an "approx" flag); Compact = "distinct · nulls"; Detailed = mini histogram + top values.
- **Data structures and Rust placement:** `hacivat-app::results::col_stats`.
- **Complexity / performance budget:** O(n) per column; ≤ 100 ms for 1M rows per column; never on the UI thread.
- **Edge cases (exhaustive list):** mixed types (document grids); very wide tables (visible columns only); pending edits (stats on displayed values); transposed view (disabled); LOB/binary (skipped); streaming (recompute per published version, debounced).
- **Test cases (input → expected):** `[a, a, NULL]` → distinct 1, nulls 1, top `a × 2`.
- **i18n notes:** `stats-distinct`, `stats-nulls`, `stats-min`, `stats-max`, `stats-approx`.
- **Verification status:** DataGrip metrics [Inferred]; ours.
- **Prior art and evidence:** datagrip/08 ALG-GRID-20.
- **Used by:** Hacivat.

### HV-GRID-16: Accessibility (AccessKit through GPUI hooks)

- **Priority / depends on:** v1.x (must be done before 1.0 GA; ui-02 calls the accessibility shadow a release blocker). Depends on HV-GRID-02, -05.
- **Sources merged:** ui-02 §3.3.6 (grid role, absolute row/column counts and indices for the visible window only, roving focus, selection state, NULL announced as a localized phrase, truncated announced), ADR 0001 (AccessKit needs our own work), GPUI source.
- **Problem:** Make a virtualized, custom-painted grid usable with VoiceOver, NVDA/Narrator and Orca.
- **Inputs / outputs:** visible region, focus, selection → an AccessKit subtree updated per frame; AccessKit actions → grid actions.
- **Decision:** use GPUI's built-in AccessKit integration [Observed: `Element::a11y_role()` and `write_a11y_info(&self, node: &mut accesskit::Node)` at `gpui/src/element.rs:113-121`; `Window::on_a11y_action`, `is_a11y_active` at `window.rs:7286-7328`]. Only when `is_a11y_active()` do we build the subtree (zero cost otherwise). The grid node has role `Grid` with `row_count`/`column_count` = totals (unknown → loaded count + "more" description); children = header row + the **visible** rows (with overscan of 5) as `Row` nodes containing `Cell`/`ColumnHeader` nodes with absolute `row_index`/`column_index`; the focus cell is the focused node; selection via `selected` state; actions `Focus`, `ScrollIntoView`, `SetValue` (edit), `ShowContextMenu`.
- **Algorithm:**
  1. Each frame with a11y active: emit nodes for the visible window with stable ids derived from `(grid id, model row ref, col id)` so that screen readers keep focus across scrolling.
  2. Cell name = the display text (HV-GRID-10) with special states spelled: NULL → `a11y-null` ("null"), truncated → "… truncated, 5 megabytes", unparsed → "invalid value", modified → "modified".
  3. Focus moves (keyboard) → set AccessKit focus to the cell node and post an announcement of row/column header + value when the column changes.
  4. Large selection changes → announce the summary (`grid-selection-summary`) rather than each cell.
  5. Handle actions: `ScrollIntoView(row)` → HV-GRID-02 navigation; `Focus` → move focus; `SetValue` → begin edit.
- **Data structures and Rust placement:** `hacivat-ui::grid::a11y` (behind the adapter so that an engine swap keeps the logic).
- **Complexity / performance budget:** ≤ 1 ms per frame for ~1,200 cell nodes when active; nothing when inactive.
- **Edge cases (exhaustive list):** unknown total rows; frozen columns (indices absolute, emitted once); transposed view (roles swap); edit mode (the editor node takes focus); very wide cells (name truncated at 1,000 chars with "…"); RTL text (AccessKit carries text; direction from the platform); placeholder rows (name "loading"); screen-reader table navigation commands that jump to unloaded rows (`ScrollIntoView` triggers fetch); platform differences (Orca on X11 vs Wayland support in GPUI — check).
- **Test cases (input → expected):** VoiceOver on macOS reads "row 1,234 of 10,000,000, column email, alice@example.com"; a NULL cell reads "null"; moving focus across 1M rows keeps the announced indices absolute; `debug_a11y_tree_json()` snapshot test of a 3×3 grid.
- **i18n notes:** announcements are Fluent messages (`a11y-cell = { $column }: { $value }`, `a11y-null`, `a11y-truncated = truncated, { $size }`, `a11y-row-of = row { $row } of { $total }`); numbers in announcements are localized.
- **Verification status:** [Observed: GPUI AccessKit hooks at `zed@912159c`] — this partly supersedes the ADR's "accessibility needs our own work": the platform bridge exists; the grid subtree is still ours. Screen-reader behaviour must be tested per OS.
- **Prior art and evidence:** ui-02 §3.3.6.
- **Used by:** Hacivat; every `hacivat-ui` consumer.

---

## 3. Conflicts resolved in this catalog

| Topic | Source A | Source B | Decision |
|---|---|---|---|
| Grid implementation | ui-02: Glide in a webview | ADR 0001: own GPUI grid | Own GPUI element; Glide/TanStack/egui used as algorithm references only |
| gpui-component Table | ADR: may be a reference/temporary dependency | 10M typed rows, range selection needed | Reference only (usize rows, single selection, string dumps) |
| Number display default | DataGrip: `.` decimal custom format | DBeaver: locale formatting (UI) | Raw by default; localized opt-in; ID columns never grouped |
| Local sort and pending edits | DBeaver drops edits | DataGrip keeps | Keep; inserted rows pinned at anchors |
| Quick filter target | DBeaver raw `toString()` | DataGrip display | Display strings with the Turkish-tolerant key |
| Local sort collation | UI locale (common bug) | i18n/02: column collation, else code point | Column collation via ICU4X, else code point |
| Accessibility | ADR: AccessKit is our work | GPUI source has AccessKit hooks | Use GPUI hooks; build the grid subtree ourselves |
| Fixed vs variable row height | ui-02 open question | — | Fixed in v1 (+ value viewer); variable in v1.x for tree/expanded rows |

## 4. Open questions that need a product decision

- **PD-10 (display mode default).** Raw machine format by default for every locale (recommended by i18n/02) vs localized by default for Turkish users.
- **PD-11 (RTL chrome mirroring).** Mirror the grid chrome (row numbers on the right) in RTL UI locales — not needed for v1 (EN + TR), decide before adding RTL languages.
- **PD-12 (NULL label).** ASCII `NULL` everywhere (recommended; copy-friendly) vs a localized label in Turkish UI (`BOŞ`/`NULL`).
- **PD-13 (local filter blocks auto-fetch).** Keep (recommended) vs keep fetching while filtering.
