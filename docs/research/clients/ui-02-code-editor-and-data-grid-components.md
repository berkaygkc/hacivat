# UI-02 — Code Editor and Data Grid Components

Status: research input. Date: 2026-11-11. Depends on `ui-01-stack-decision.md` (recommended stack: Tauri v2 + TypeScript UI + Rust core).
This document is the home of the grid algorithm specifications **ALG-GRID-01..04** (virtualization, auto-sizing, selection, clipboard), which other client documents reference.

---

## 1. Scope and sources

| Component | Version / commit | License | Location / evidence |
|---|---|---|---|
| CodeMirror 6 `@codemirror/view` | 6.41.0 (npm latest 6.43.15), commit `fbff59ba004d80d8c914f64c42586387b08706ac` | MIT | `.refs/repos/codemirror-view` |
| CodeMirror 6 `@codemirror/state` | 6.6.0, commit `9c801279cb83011e6f92af778f4443406e8f1200` | MIT | `.refs/repos/codemirror-state` |
| CodeMirror 6 `@codemirror/autocomplete` | 6.20.1, commit `9a01794def6f9f12468805dce2d687b7548ff469` | MIT | `.refs/repos/codemirror-autocomplete` |
| `@codemirror/lang-sql` | commit `f5b0b0ba1811f87db2d1733f005585bd43322980` | MIT | `.refs/repos/lang-sql` (cloned by another agent; referenced only) |
| Monaco Editor | 0.57.0 (npm, file listing via unpkg) | MIT | unpkg metadata |
| Glide Data Grid | 6.0.4-alpha25 (npm stable 6.0.3), commit `0875d78cc41535a39f8b417ff88ce1ebe129c363` | MIT | `.refs/repos/glide-data-grid` |
| TanStack Virtual (`@tanstack/virtual-core`) | 3.19.11, commit `54a3f7d3cc32b373187b3802d81cb4b24ae08b0b` | MIT | `.refs/repos/tanstack-virtual` |
| TanStack Table | (GitHub metadata only) | MIT | `gh api repos/TanStack/table` |
| AG Grid Community | 36.3.0 (npm) | MIT (Community); Enterprise is commercial | npm metadata |
| Handsontable | 18.1.2 (npm) | "SEE LICENSE IN LICENSE.txt" (proprietary/commercial; free non-commercial key) | npm metadata |
| Perspective (FINOS) | (GitHub metadata) | Apache-2.0 | `gh api repos/finos/perspective` |
| Tabulator | (GitHub metadata; Beekeeper uses `tabulator-tables` ^6.7.1) | MIT | beekeeper `apps/studio/package.json:158` |
| egui / egui_extras `TableBuilder` | 0.37.0, commit `38f6bfa3…` | MIT OR Apache-2.0 | `.refs/repos/egui` |
| gpui-component `Table`, `Editor` | commit `3c5320b7…` | Apache-2.0 | `.refs/repos/gpui-component` |
| Floem editor-core / Lapce | floem `1351ffb1…`, lapce `f66ffaa9…` | MIT / Apache-2.0 | `.refs/repos/floem`, `.refs/repos/lapce` |
| Zed editor | commit `912159c7…` | **GPL-3.0-or-later** (editor, rope, text) | not studied beyond license (clean-room) |
| Helix | GitHub metadata | MPL-2.0 | — |
| ropey | GitHub metadata | MIT | — |
| Outerbase Studio "OptimizeTable" | GitHub API fetch of `src/components/gui/table-optimized/use-visibility-calculation.ts` | **AGPL-3.0** (prose only) | `gh api` |

No builds/scripts executed. AG Grid / Handsontable / Perspective statements not backed by source in this workspace are tagged **[Inferred]**.

---

## 2. Executive summary

1. **Editor: CodeMirror 6** is the right default for Hacivat: modular, MIT, small, excellent IME/bidi handling via the browser, first-class i18n hook (`EditorState.phrases`) **[Observed: codemirror-state/src/state.ts:305-330]**, height-map virtualization that already handles documents taller than browser DOM limits (`MaxDOMHeight = 7e6`, `BigScaler`) **[Observed: codemirror-view/src/viewstate.ts:52-60, 695-720]**.
2. CodeMirror autocompletion defaults: activate on typing with 100 ms delay, max 100 rendered options, 75 ms interaction delay; fuzzy matcher with penalties for case-folding, non-start matches, gaps **[Observed: codemirror-autocomplete/src/config.ts:98-114; filter.ts:5-133]**.
3. **Monaco** is heavier, VS Code-coupled, but ships localized UI strings including **Turkish** (`esm/vs/nls/lang/tr.js` in 0.57.0) **[Observed: unpkg file listing]** and has strong built-in a11y. Keep as fallback, not default.
4. Native Rust editors: Zed's is GPL (unusable); **gpui-component Editor** (Apache, ropey, LSP features) and **Floem/Lapce editor-core** (MIT/Apache, xi-rope) are the only permissive candidates; neither matches CM6 in breadth today.
5. Text buffers in Rust (for core-side features like formatting, splitting, MCP edits): **ropey** (MIT) is the default choice; Zed's `rope` crate is GPL; `sum_tree` is Apache.
6. **Grid: Glide Data Grid** (MIT, canvas) is the strongest off-the-shelf grid for a Tauri UI: canvas rendering, async `getCellsForSelection`, column auto-size from samples with outlier removal, TSV+HTML clipboard, frozen columns/trailing rows, and an **offset-based scroller that defeats the 33,554,400 px browser height limit** **[Observed: glide-data-grid/packages/core/src/internal/scrolling-data-grid/infinite-scroller.tsx:74-82, 234-270]**.
7. Glide's TSV paste parser iterates code points but slices by UTF-16 index — a latent bug for astral characters (emoji) **[Observed: data-editor-fns.ts:107-163]**; Hacivat's spec (ALG-GRID-04) fixes this.
8. **AG Grid Community** is MIT but range selection and clipboard are Enterprise features **[Observed: https://www.ag-grid.com/javascript-data-grid/community-vs-enterprise/ — "Range selection" and enhanced "Clipboard" listed as Enterprise features; Community is MIT]** — disqualifying for a spreadsheet-like DB grid without a license.
9. **Handsontable** is proprietary since v7 (non-commercial key) — not usable in an open-source permissive product.
10. **TanStack Table + Virtual** is headless and excellent for medium datasets, but DOM-based; 2-axis DOM virtualization costs layout per scroll. Its virtual-core algorithms (binary search over measurements, overscan, lanes, scroll adjustment on size change) are a good reference **[Observed: tanstack-virtual/packages/virtual-core/src/index.ts:45-105, 258-290, 646]**.
11. **Perspective** (Apache-2.0, WASM engine on Arrow) is a pivot/analytics viewer, not an editable DB grid; useful later for "analyze result" views.
12. **Tabulator** (MIT, used by Beekeeper) is DOM-virtualized with remote pagination; Beekeeper drives it with an `ajaxRequestFunc` that queries the DB **[Observed: beekeeper TableTable.vue:1243-1256, 2013-2100]**.
13. Native grids: egui `TableBuilder` virtualizes rows only (fixed-height fast path) **[Observed: egui_extras/src/table.rs:1027-1081]**; gpui-component `Table` virtualizes rows and columns and supports fixed columns and load-more **[Observed: table/state.rs:129-134; delegate.rs:219-238]**.
14. ALG-GRID-01 specifies **row-index-anchored 2-axis virtualization** with a scroll-space scaler (works for 10^9 rows, both browser and f32 native toolkits), frozen rows/columns, variable column widths via prefix sums, and overscan.
15. ALG-GRID-02 specifies **column auto-sizing** from head+tail samples with outlier trimming (generalizing Glide's approach) plus type-aware min/max and header width.
16. ALG-GRID-03 specifies a **selection model** (current cell + list of rectangles + whole-row/whole-column sets compacted as interval sets), with keyboard semantics (Shift/Ctrl/Cmd, Home/End, PageUp/Down, Ctrl+A) and edge cases (hidden/frozen columns, shrinking results).
17. ALG-GRID-04 specifies **clipboard**: TSV (RFC-4180-style quoting), CSV, HTML table, and a private JSON MIME for lossless round-trips; NULL vs empty string; paste-shape broadcasting; type coercion; size limits.
18. Accessibility for a canvas grid needs an **ARIA shadow** (`role=grid`, `aria-rowcount`, `aria-rowindex`, `aria-colindex`, live focus cell element) — specified in §3.3.6.

---

## 3. Detailed findings

### 3.1 CodeMirror 6

#### 3.1.1 State/view architecture

- `EditorState` is immutable; changes are `Transaction`s carrying `ChangeSet`s, selection updates and `StateEffect`s. Extensions configure the state through **facets** (multi-provider configuration values with combine functions) and **state fields** (derived persistent values updated per transaction) **[Observed: codemirror-state/src/state.ts; facet.ts]**.
- `EditorView` owns the DOM. Updates flow `dispatch(tr) → state = tr.state → view.update([tr])`; view plugins react to `ViewUpdate`s. Measurement is batched into a read phase and a write phase (`requestMeasure`) to avoid layout thrash **[Observed: codemirror-view/src/editorview.ts (MeasureRequest import line 17)]**.

#### 3.1.2 Viewport, height map, very large documents

- Only the **viewport** (visible lines plus a margin `VP.Margin = 1000` px) is rendered; the rest is represented by a **height map** (tree of line heights with an estimating `HeightOracle`) **[Observed: viewstate.ts:52-60; heightmap.ts]**.
- Beyond `MaxDOMHeight = 7e6` px "DOM layout starts to break down in browsers because they use fixed-precision numbers", so the view switches to a `BigScaler` that compresses the non-viewport parts of the document into the 7M budget while keeping viewport regions 1:1 **[Observed: viewstate.ts:58, 207-211, 695-720]**. This is the same idea Hacivat's grid needs (ALG-GRID-01 step 2).
- Very long lines are handled by **line gaps** (`LG.Margin = 2000`, `MarginWrap = 10000`) that skip rendering off-screen horizontal content **[Observed: viewstate.ts:106-110, 451-460]**.
- Bidi: per-line direction support (`perLineTextDirection` facet), `bidiSpans`, `textDirectionAt` **[Observed: extension.ts:36; editorview.ts:667-813]**; `EditContext` used where the browser supports it **[Observed: docview.ts:153-160]**.

#### 3.1.3 Decorations

- Four kinds: mark (styling a range), widget (inline/block element at a position), replace (hide/replace a range, e.g., folding), line (attributes on a line). Provided as `RangeSet<Decoration>` from state fields or view plugins; `MatchDecorator` builds decorations from regex matches incrementally **[Observed: codemirror-view/src/decoration.ts, matchdecorator.ts]**.
- Gutters (`gutter.ts`), layers for selection/cursor drawing (`layer.ts`, `draw-selection.ts`), rectangular selection (`rectangular-selection.ts`), tooltips (`tooltip.ts`), panels (`panel.ts`) **[Observed: file list]**.
- Inlay hints = widget decorations (`side` controls stickiness) — e.g., showing inferred column types after `SELECT` items.

#### 3.1.4 Autocompletion

- Config defaults: `activateOnTyping: true`, `activateOnTypingDelay: 100`, `closeOnBlur: true`, `maxRenderedOptions: 100`, `filterStrict: false`, `interactionDelay: 75`, `updateSyncTime` (sync window for async sources) **[Observed: codemirror-autocomplete/src/config.ts:9-114]**.
- `CompletionSource` = function of `CompletionContext` returning `{from, to?, options, validFor?, filter?}`; async sources supported; multiple sources merged and ranked.
- `FuzzyMatcher`: pattern code points + case-folded variants; penalties (`Penalty.CaseFold`, `NotStart`, `Gap`, `ByWord`, `NotFull`); single-character queries only match at word starts; scores more negative for worse matches **[Observed: filter.ts:5-160]**. Case folding uses `toUpperCase()/toLowerCase()` without a locale **[Observed: filter.ts:33-39]** — correct for SQL identifiers, but means Turkish users typing `ı` won't match `I`-spelled identifiers (and that's desirable for identifier semantics).
- Phrase key used by the completion list: `"Completions"` **[Observed: grep of `phrase("…")` in codemirror-autocomplete]**.

#### 3.1.5 Lint, language support, Lezer

- `@codemirror/lint`: diagnostics with severity, actions, gutter markers, panel; linter function may be async **[Inferred: package API; Compass uses it — compass-editor/package.json:72]**.
- Language support via **Lezer** incremental LR parsers; `@codemirror/lang-sql` provides dialects (StandardSQL, PostgreSQL, MySQL, MariaDB, MSSQL, SQLite, Cassandra, PL/SQL) and schema-based completion (`schema: {table: [columns]}`) **[Inferred: lang-sql API; repository in .refs/repos/lang-sql]**. For Hacivat the dialect tokenizer is fine for highlighting; *semantic* completion should come from the Rust core (schema cache + parser such as sqlparser-rs/pg_query) via a custom `CompletionSource`.

#### 3.1.6 Collaboration

- `@codemirror/collab` implements a central-authority rebase protocol (clients send updates with version numbers; the authority orders them; clients rebase pending changes) **[Inferred: package design]**. Relevant only if Perde later supports shared query tabs.

#### 3.1.7 i18n (phrases API)

- `EditorState.phrases` facet registers translation maps; `state.phrase(text, ...insert)` looks them up, with `$1`-style insertion **[Observed: state.ts:305-330]**.
- Phrase keys found in the studied packages: `"close"`, `"Control character"` (view), `"Completions"` (autocomplete) **[Observed: grep]**. Other packages (search: "Find", "Replace", "next", "previous", "all", "match case", "regexp", "by word", "replace all", "Go to line"; lint: "Diagnostics", "No diagnostics"; fold: "Folded lines", "Unfolded lines", "unfold") **[Inferred: package sources not cloned here]**.
- Hacivat: generate a phrase map from Fluent: messages `cm-find = Bul`, etc., mapped by an explicit table from CM English phrase → Fluent id; CI test asserts every CM phrase used by enabled extensions has a mapping (extract by grepping `phrase("` in `node_modules/@codemirror/*`).

#### 3.1.8 Accessibility

- The editor is a `contenteditable` with ARIA attributes; completion list is `role=listbox` with `aria-activedescendant` (screen readers announce options) **[Inferred: CM6 a11y docs]**. Tab-trapping is opt-in (`indentWithTab` keymap) — Hacivat must expose an "Escape then Tab leaves editor" behavior.

### 3.2 Monaco

- VS Code's editor extracted: model (piece tree text buffer), view (DOM, virtualized lines), services (instantiation, keybinding, contextview), language features via providers; tokenization via Monarch or TextMate grammars (with extra setup) **[Inferred: Monaco architecture]**.
- Size: multiple MB of JS + CSS **[Observed: unpkg listing shows a large CSS/JS tree]**; workers for languages.
- i18n: ships NLS bundles for cs, de, es, fr, it, ja, ko, pl, pt-br, ru, **tr**, zh-cn, zh-tw **[Observed: unpkg `esm/vs/nls/lang/*.js` for 0.57.0]**. Adding a language not in that list requires generating NLS from VS Code translations — non-trivial (conflicts with "new languages must be trivial").
- Strong built-in accessibility (accessible diff viewer, screen-reader optimized mode) **[Observed: `accessibleDiffViewer.css` in listing]**.
- Used by RedisInsight Workbench (`monaco-editor` ^0.48) **[Observed: RedisInsight package.json:313]**.
- Verdict: heavier, less modular, CSP/worker setup friction in Tauri; Turkish strings are a plus but Hacivat must translate its own UI anyway.

### 3.3 Data grids

#### 3.3.1 Glide Data Grid (MIT)

Rendering model: one `<canvas>` for cells/headers; overlays (DOM) for the active editor; scroll handled by an invisible native scroller ("padders") with the canvas positioned on top **[Observed: infinite-scroller.tsx:73-82]**.

- **Virtualization**: `walkColumns`, `walkRowsInCol` iterate only visible items; `blitLastFrame` copies the previous frame's pixels when scrolling and redraws only the newly exposed strip; `blitResizedCol` optimizes column resize **[Observed: render/data-grid-render.walk.ts:21,66; data-grid-render.blit.ts:21,193]**.
- **Huge heights**: browser max div height `33_554_400`; padders split into ≤ 5,000,000 px segments; when the virtual content height exceeds the DOM limit, the scroller uses a hybrid mapping — small deltas (`|Δ| ≤ 2000`) move the virtual position 1:1, large jumps or edges recompute from the scrollbar percentage **[Observed: infinite-scroller.tsx:74-82, 234-270, 317]**.
- **Direction lock**: when scrolling starts, the dominant axis is locked to reduce diagonal jitter until 200 ms idle **[Observed: infinite-scroller.tsx:177-232, 271-275]**.
- **Freeze**: `freezeColumns` (left), `freezeTrailingRows` (bottom), `smoothScrollX/Y` options **[Observed: scrolling-data-grid.tsx:80-94, 133-185]**.
- **Selection**: `CompactSelection` stores sorted, merged ranges of indices (rows or columns) **[Observed: data-grid-types.ts:590-595]**; GridSelection = current cell + range + additional ranges (`rangeStack`) + row/column CompactSelections **[Inferred: API]**.
- **Auto-size**: see ALG-GRID-02; observed algorithm: sample `computeRows = max(1, 10 − floor(cols/10000))` rows from the top, swap one for the last row (tail sample), measure each cell with its renderer's `measure` (default 150), drop outliers ≥ 2× average when > 5 samples, include header title + 2×padding + 28 px icon, clamp to `[ceil(min), floor(max)]` **[Observed: use-column-sizer.ts:15-150]**.
- **Clipboard copy**: builds both TSV (`text/plain`) and HTML (`text/html`) buffers. TSV escaping: quote when cell contains `\t`, `\n` or `"` (for array cells also `,`), doubling quotes **[Observed: copy-paste.ts:130-153]**. HTML: `<table><tbody>` with a `<style>` making `<br>` stay in the same cell in Excel (`mso-data-placement:same-cell`), raw values in a `gdg-raw-value` attribute, URIs as `<a href>`, arrays as `<ol>` **[Observed: copy-paste.ts:155-215]**. HTML decode on paste restores `<br>` → `\n`, links, lists, raw values **[Observed: copy-paste.ts:239-310]**.
- **Paste**: `onPaste` may be boolean or callback receiving the parsed 2-D string array; `coercePasteValue(val, cell)` converts per cell; TSV parsed by a 3-state machine (`None`, `inString`, `inStringPostQuote`), CRLF normalized **[Observed: data-editor.tsx:486, 629-637, 3586-3720; data-editor-fns.ts:100-163]**.
  - Bug: `for (const char of str)` iterates **code points** while `index` is used with `str.slice` in **UTF-16 units** → offsets drift after any astral character (emoji, some CJK extension chars) **[Observed: data-editor-fns.ts:121-158]**.
- Editing: overlay editors per cell kind; custom cells via renderers with `draw`, `measure`, `onPaste`, `provideEditor` **[Inferred: API; observed `onPaste` on renderer at data-editor.tsx:3611-3624]**.
- Performance: Glide advertises smooth scrolling of millions of rows **[Inferred: project README]**; real constraint is the data callback (`getCellContent`) which must be synchronous per cell — pair with ALG-UI-01 page cache and render placeholders ("loading" cells) for missing pages.
- A11y: Glide renders an accessible table shadow for focused region **[Inferred]**; verify with screen readers in the spike.

#### 3.3.2 AG Grid Community

- DOM-based; row virtualization with `rowBuffer`, column virtualization (`suppressColumnVirtualisation` to disable); row models: client-side, infinite, server-side (Enterprise), viewport (Enterprise) **[Inferred: AG Grid docs]**.
- Cell selection (range), clipboard, Excel export, pivoting are **Enterprise** **[Observed: https://www.ag-grid.com/javascript-data-grid/community-vs-enterprise/ — "Range selection" and enhanced "Clipboard" listed as Enterprise features; Community is MIT]**. Community includes sorting, filtering, editing, column resize/reorder/pin.
- Used by Compass table view (`ag-grid-community` ^36.2) **[Observed: compass/packages/compass-crud/package.json:92-93]**.
- Verdict: excellent but the features a DB client needs most (rectangular copy) are paywalled.

#### 3.3.3 TanStack Table + TanStack Virtual (MIT)

- Table: headless state machine (column defs, sorting, filtering, grouping, column sizing/pinning/ordering, row selection); rendering is the app's job.
- Virtual core: `estimateSize(index)`, measured sizes cached by key (`itemSizeCache`), `measureElement` via ResizeObserver, visible range computed by binary search over cumulative measurements, `overscan` (default 1), `lanes` for masonry/multi-column, and `shouldAdjustScrollPositionOnItemSizeChange` to prevent jumps when items above the viewport resize **[Observed: virtual-core/src/index.ts:45-105, 258-290, 351-392, 646, 793-808]**.
- 2-axis: instantiate two virtualizers (rows + columns) **[Inferred]**. DOM cells = visible rows × visible cols elements; with 50 × 20 = 1000 elements re-positioned per frame — acceptable, slower than canvas on WebKitGTK **[Inferred]**.
- Great for Perde panel lists (audit log, users) and Hacivat side trees.

#### 3.3.4 Perspective (FINOS, Apache-2.0)

- C++ engine compiled to WASM; data ingested as Arrow/CSV/JSON; supports pivots, aggregates, filters, expressions; `perspective-viewer` web component with datagrid plugin (`regular-table`, virtual DOM table) and charts **[Inferred: Perspective docs]**.
- Not a row editor; ideal for an optional "Analyze" view on a result (pivot/aggregate locally) fed directly with Arrow IPC from ALG-UI-01 (no conversion).

#### 3.3.5 Handsontable (license caveat)

- Spreadsheet-like (formulas, merged cells, rich editors). Since v7.0.0 (2019) distributed under a proprietary license; free only for non-commercial/evaluation with a key `non-commercial-and-evaluation` **[Inferred: Handsontable licensing docs; npm `license` field "SEE LICENSE IN LICENSE.txt" observed]**. **Do not use** in Hacivat (open source, may be used commercially).

#### 3.3.6 Accessibility shadow for canvas grids (Hacivat requirement)

- Keep a DOM element `role="grid"` with `aria-rowcount` = total rows (or `-1` when unknown), `aria-colcount`; render a small set of `role="row"`/`role="gridcell"` elements for the **visible** window with `aria-rowindex`/`aria-colindex` (1-based, absolute), header cells `role="columnheader"`; the focused cell element receives DOM focus (`tabindex=0`, roving tabindex) so screen readers announce it; selection reflected with `aria-selected`; NULL announced as localized "null" phrase; truncated values announce "truncated".
- Live region (polite) for "Copied 120 cells" / "Row 1,234 of 10,000,000".

#### 3.3.7 Tabulator (MIT) — as used by Beekeeper

- Virtual DOM rows (`renderVertical: "virtual"`), optional horizontal virtual rendering; remote pagination via `ajaxRequestFunc`; range selection and clipboard (`copyRanges`) in v5.5+/v6 **[Observed: Beekeeper calls `this.tabulator.copyRanges({ type: 'plain' })` TableTable.vue:1124]**.

#### 3.3.8 Outerbase Studio OptimizeTable (AGPL-3.0 — prose only)

- DOM table with fixed row height; visible rows = `floor(scrollTop / rowHeight) − 1 − renderAhead` to `+ ceil(height/rowHeight) + renderAhead`; visible columns found by a **linear scan** accumulating column widths from a gutter offset **[Observed: use-visibility-calculation.ts via `gh api`]**. Uses fake padding rows/cells for off-screen space. No height scaling → limited by the browser's max element height at ~1M rows × 35 px **[Inferred]**. Linear column scan is O(C) per scroll event — fine below a few hundred columns.

#### 3.3.9 Native Rust grid approaches

- **egui `TableBuilder`**: fixed-height rows → O(1) visible range; heterogeneous rows → O(n) scan per frame; column widths resizable with `Column::auto/remainder/initial`; rows-only virtualization **[Observed: table.rs:30-381, 976-1225]**.
- **gpui uniform_list / list (SumTree)** as building blocks; **gpui-component Table** provides 2-axis virtualization, fixed columns left/right, selection modes, loop selection, load-more threshold **[Observed: state.rs:57-335; delegate.rs:219-274]**.
- All native options use f32 coordinates → must apply ALG-GRID-01 row-index anchoring.

### 3.4 Native editor/buffer options summary

| Option | License | Buffer | Features | Usable? |
|---|---|---|---|---|
| Zed editor | GPL-3.0+ | GPL rope over Apache sum_tree | Full IDE | **No** |
| gpui-component Editor | Apache-2.0 | ropey | highlighting (tree-sitter optional), search, diagnostics, LSP completions/hover/code actions/semantic tokens | Yes (if gpui path chosen) |
| Floem `editor-core` / Lapce | MIT / Apache-2.0 | lapce-xi-rope | cursor/selection/movement/registers/vim mode | Yes |
| Helix (`helix-core`) | MPL-2.0 (file-level copyleft) | ropey | modal editing, tree-sitter | Possible as dependency (MPL obligations on modified files) |
| ropey | MIT | rope of UTF-8 chunks, char/line/UTF-16 indexing | — | Yes (core-side text ops) |

---

## 4. Algorithm specifications

### ALG-GRID-01: Two-axis grid virtualization with row-index anchoring

- **Problem:** Render a grid of R rows (up to 10^9) and C columns (up to 10^4) with variable column widths, fixed (or few-valued) row heights, frozen leading/trailing rows and columns, smooth scrolling, and no dependence on pixel coordinate precision (browser element height limits ~7M–33.5M px; f32 exactness 2^24 px).
- **Inputs / outputs:**
  - Inputs: `R` (rows; may grow while streaming), `C`, `colWidth[c]` (logical px), `rowHeight` (logical px; optionally per-row overrides map), `frozenLeftCols FL`, `frozenRightCols FR`, `frozenTopRows FT` (header rows are separate), `frozenBottomRows FB`, viewport size `(W, H)`, device pixel ratio, overscan `(oxRows, oyCols)`, scroll input events (wheel deltas, scrollbar drags, keyboard navigation targets).
  - Outputs per frame: `visibleRows: [r0, r1)`, `visibleCols: [c0, c1)` for the scrollable region, the frozen sets, the pixel offset of the first visible row/col (`subRowOffsetY`, `subColOffsetX`), and draw lists.
- **Algorithm:**
  1. **Scroll state is index-based.** Maintain `topRow: u64` and `topRowOffset: f32 ∈ [0, rowHeight)` (pixels the top row is scrolled out), and analogously `leftCol`, `leftColOffset`. Never store absolute Y pixels for the data region.
  2. **Native scrollbar mapping (browser/scroll-container hosts).** Let `contentH = (R − FT − FB) × rowHeight` (logical). If `contentH ≤ LIMIT` (choose `LIMIT = 7,000,000` px for cross-browser safety; Glide uses 33,554,400 **[Observed]**, CodeMirror 7e6 **[Observed]**), set the scroller's inner height to `contentH` and map `scrollTop ↔ (topRow, topRowOffset)` exactly. Else set inner height to `LIMIT` and use a **hybrid mapping**:
     a. On a scroll event with `Δ = scrollTop − lastScrollTop`: if `|Δ| ≤ JUMP` (e.g., 2000 px) and the scroller is not at either end, advance the virtual position by `Δ` pixels (1:1 feel): `advanceRows(Δ)`.
     b. Otherwise (thumb drag, page jump, Home/End, at edges) compute `p = scrollTop / (LIMIT − H)`, set `virtualY = p × (contentH − H)`, and derive `topRow = floor(virtualY / rowHeight)`, `topRowOffset = virtualY − topRow × rowHeight` using **f64/u64 arithmetic**.
     c. After programmatic navigation (keyboard to row k), compute the target virtual position, then set `scrollTop = round(virtualY / (contentH − H) × (LIMIT − H))` while suppressing the resulting scroll event's re-mapping (one-shot flag).
  3. **Native-toolkit hosts (gpui/egui):** draw a custom scrollbar whose thumb position is `topRowFraction = (topRow + topRowOffset/rowHeight) / max(1, R_scrollable − rowsPerViewport)`; wheel deltas use `advanceRows`; thumb drags use 2b. No pixel-height content element is ever created.
  4. `advanceRows(Δpx)`: `total = topRowOffset + Δpx`; `rows = floor(total / rowHeight)`; `topRow = clamp(topRow + rows, 0, maxTopRow)`; `topRowOffset = total − rows × rowHeight` (then clamp to 0 at boundaries). `maxTopRow` is the row index such that the last row aligns with the bottom edge.
  5. **Visible rows**: `r0 = topRow`, `rowsVisible = ceil((Hs + topRowOffset) / rowHeight)` where `Hs = H − headerH − FT×rh − FB×rh`; `r1 = min(R_scrollable, r0 + rowsVisible)`. Overscan: `r0' = max(0, r0 − oxRows)`, `r1' = min(R, r1 + oxRows)` (overscan used for prefetch, not necessarily drawing).
  6. **Variable row heights (optional, e.g., expanded JSON rows):** store overrides in a Fenwick tree (BIT) over row index with `delta = h_i − rowHeight`; position of row i = `i × rowHeight + prefixDelta(i)`; find row at virtual y by binary-lifting on the BIT (O(log R)). Unmeasured rows use `rowHeight`. When a row above the viewport changes height, adjust `topRowOffset` so the visible content does not jump (TanStack's `shouldAdjustScrollPositionOnItemSizeChange` idea **[Observed]**).
  7. **Columns**: maintain prefix sums `X[c] = Σ_{k<c} colWidth[k]` for scrollable columns (f64). Visible columns by binary search: `c0 = upperBound(X, leftScrollX) − 1`, `c1 = lowerBound(X, leftScrollX + Ws)`. Horizontal content width is rarely > LIMIT, but apply the same hybrid mapping if `X[C] > LIMIT` (10^4 cols × 300 px = 3M px, safe). On width change of column k, update prefix sums from k (O(C)) or use a Fenwick tree for O(log C).
  8. **Frozen regions**: draw in this order — scrollable body, frozen-left body, frozen-right body, frozen-top/bottom rows, header row(s), corner cells; each with a clip rect. Frozen columns do not participate in horizontal scrolling and are excluded from `X`.
  9. **Painting (canvas):** if only the scroll position changed by `(dx, dy)` and `|dx| < Ws`, `|dy| < Hs`, blit the previous frame by `(−dx, −dy)` and paint only the exposed strips (Glide `blitLastFrame` **[Observed]**). Otherwise full repaint. Paint at device pixels: `canvas.width = W × dpr`; snap grid lines to `0.5/dpr` offsets.
  10. **Data availability:** for each visible cell, ask the page cache (ALG-UI-01); if missing, paint a skeleton placeholder and enqueue the page; when it arrives, invalidate only cells in that page that are still visible.
  11. **Text drawing:** truncate to the cell width with an ellipsis computed by binary search on measured prefix widths, cached per `(string, font, width-bucket)`; set `direction` per cell from the first strong character (UAX#9 P2-P3) so RTL data renders correctly regardless of UI locale.
  12. Emit `visibleRegionChanged(r0, r1, c0, c1)` debounced (e.g., 50 ms) for consumers (status bar, prefetch).
- **Data structures:** `ScrollState {topRow: u64, topRowOffset: f32, leftScrollX: f64}`; `colPrefix: Vec<f64>`/Fenwick; `rowHeightDeltas: Fenwick<f64>` (sparse, optional); frame cache (offscreen canvas); text measurement LRU.
- **Complexity / performance notes:** O(log C) column range, O(1) or O(log R) row range per frame; paint O(visible cells) (~1–3k). Blit reduces typical scroll frame to O(exposed strip). All arithmetic on row positions in f64/u64 — f64 integers are exact to 2^53, so 10^9 rows × 100 px = 10^11 is exact.
- **Edge cases (exhaustive list):**
  1. `R = 0` → draw header only; no selection beyond header; scrollbar hidden.
  2. `C = 0` (statement without result columns) → grid not shown.
  3. `R` grows during streaming while user is at bottom → optionally "stick to bottom" if the user was at the bottom before growth; otherwise keep `topRow`.
  4. `R` shrinks (re-run, filter) → clamp `topRow` to new `maxTopRow`; clamp selection (ALG-GRID-03).
  5. Viewport taller than all rows → `maxTopRow = 0`, `topRowOffset = 0`.
  6. Frozen columns wider than viewport → scrollable region width ≤ 0 → disable horizontal scrolling, show a hint, allow unfreezing.
  7. `FL + FR ≥ C` → no scrollable columns.
  8. Device pixel ratio change (moving between monitors) → resize backing store, full repaint, invalidate text cache.
  9. Zoom/font-size change → `rowHeight` changes; preserve `topRow` (index anchoring makes this trivial).
  10. Column hidden/shown/reordered → rebuild prefix sums; keep `leftScrollX` anchored to the first visible column's index if possible.
  11. Extremely wide column (> viewport) → visible range contains one column; text clipped; horizontal blit still valid.
  12. Scroll event storms (trackpads at 120 Hz) → coalesce in `requestAnimationFrame`.
  13. Scrollbar thumb at extreme ends in scaled mode → exact `topRow = 0` or `maxTopRow` (Glide special-cases `newY === 0 || newY === scrollableHeight` **[Observed]**).
  14. Keyboard navigation beyond loaded pages → scroll immediately, placeholders until data arrives.
  15. RTL UI locale → mirror chrome (row header on the right) only if product decides; cell text direction independent (per step 11).
  16. Touch/inertial scrolling (macOS trackpad momentum) → treat as small deltas (1:1).
  17. Background tab (hidden) → stop painting; resume with full repaint.
  18. Variable row height row above viewport expands → compensation per step 6.
  19. Rows with height overrides when sorting changes → overrides keyed by stable row id, not index; clear on generation change.
  20. f32 hosts: never compute `topRow × rowHeight` in f32.
- **Test cases (input → expected):**
  1. R = 1,000,000,000, rh = 24, H = 600 → initial r0 = 0, r1 = 25; End key → r0 = 999,999,975 (with header-less body height 600), last row fully visible, no precision artifacts.
  2. Scaled mode, scrollbar dragged to 50% → topRow ≈ (R·rh − H)/2 / rh within ±1 row.
  3. Wheel Δ = 3 px repeatedly 1000 times in scaled mode → topRow advances exactly 125 rows (3000/24), not jumping.
  4. Column widths [100, 200, 50, 300], scrollX = 250, Ws = 300 → visible cols [1, 4) (col1 spans 100–300, col2 300–350, col3 350–650).
  5. FL = 2 with widths 400+400 and W = 700 → horizontal scrolling disabled, hint shown.
  6. R shrinks from 10,000 to 10 while topRow = 9,000 → topRow = 0.
  7. dpr changes 1 → 2 → backing store doubles; no blurry text.
  8. Row 5 (above viewport, topRow = 100) height changes 24 → 240 → visible content does not move.
- **Prior art and evidence:** Glide infinite scroller + blit **[Observed: infinite-scroller.tsx; data-grid-render.blit.ts]**; CodeMirror BigScaler **[Observed: viewstate.ts:695-720]**; TanStack virtual-core **[Observed]**; egui TableBuilder fixed-height math **[Observed: table.rs:1027-1081]**; Outerbase linear column scan (AGPL, prose) **[Observed]**.
- **Used by:** Hacivat (result grid, table data editor, Redis value grids, Mongo table view); Perde (audit log grid).

### ALG-GRID-02: Column auto-sizing from sampled content

- **Problem:** Choose initial column widths that show typical content without letting a few huge values (long JSON, base64) blow up the layout, using only a sample of rows (data may be remote/streaming).
- **Inputs / outputs:** Inputs: column metadata (name, declared type, nullable, is_pk/fk), header font & cell font metrics, per-column samples (strings as they will be displayed), theme paddings, `minW`, `maxW` (global and per-type), viewport width `W`, user overrides (persisted widths). Output: `width[c]` (logical px), plus `wrapHint[c]`.
- **Algorithm:**
  1. If a persisted user width exists for `(connection, schema, table|query-fingerprint, column)`, use it and stop.
  2. **Sample selection:** take the first `h` rows available (h = 50 default; reduce to `max(1, 10 − floor(C/10000))` style scaling for extremely wide results, as Glide does with 10 rows **[Observed: use-column-sizer.ts:118-123]**) plus a tail sample (last `t = min(10, R − h)` rows if known) plus up to `s` stratified random rows from already-loaded pages. Never block on fetching more rows than the first page.
  3. **Display transformation:** convert each sampled value to its display string exactly as the cell renderer will (NULL → localized placeholder width, booleans → checkbox width constant, numbers with locale grouping, dates formatted, JSON single-line preview truncated at `maxPreviewChars`, binary → hex preview).
  4. **Measure** each string with the cell font (`measureText` / shaped width), add `2 × cellPadding` + type icon width if present. Cache widths by `(string hash, font)`.
  5. **Outlier trimming:** if `n > 5`, compute the mean `μ`; values `≥ 2μ` are outliers (Glide's rule **[Observed: use-column-sizer.ts:48-63]**); `cand = max(width_i | width_i < 2μ)`. Because a single huge value inflates μ, apply the rule iteratively (recompute μ over the remaining values, at most 3 passes) when `n ≥ 20`. If `n ≤ 5`, `cand = max(width_i)`. Record `trimmedCount` so the UI can show a "some values are wider" hint on hover.
  6. **Header width:** `hdr = measure(header text, header font) + 2 × padding + sortIcon + typeBadge + (pk/fk icon)`. Header text is the column name (data; not translated) plus optional type label.
  7. `w = max(cand, hdr)`.
  8. **Type-specific bounds:** booleans `[48, 80]`; integers `[48, 160]`; UUID `[~36ch]` fixed; timestamps `[~19–29ch]`; text/JSON `[80, maxW]`; binary `[80, 240]`. Clamp: `w = clamp(w, max(minW, typeMin), min(maxW, typeMax))` with `ceil/floor` (Glide clamps to `[ceil(min), floor(max)]` **[Observed]**). Beekeeper defaults: min 100, max 1000, max initial 500, default 125, large-field 300 **[Observed: beekeeper default.config.ini tableTable]** — Hacivat's suggested defaults: min 48, max initial 480, absolute max 2000.
  9. **Fill:** if `Σ w < W` and fewer than ~8 columns, optionally grow text-like columns proportionally to fill (`grow` factor), never numerics.
  10. **Re-measure policy:** do not resize automatically after initial layout (avoids jitter as pages stream). Offer "Auto-fit column" (double-click header border) which re-measures the *currently loaded* rows of that column (bounded to e.g. 10k rows) and "Auto-fit all".
  11. Run sizing off the main paint path (Web Worker or idle callback); measurement for 200 columns × 60 samples = 12k strings must complete < 30 ms (spike target).
- **Data structures:** per-column sample arrays; measurement cache LRU; persisted widths keyed by stable identifiers.
- **Complexity / performance notes:** O(C × n) measurements; with caching, repeated values are cheap. Percentile via nth_element O(n).
- **Edge cases (exhaustive list):** empty result (header-only widths); all NULL column (width = max(header, NULL placeholder)); single row; multiline strings (measure first line; set wrapHint); RTL strings (shaped width differs; still measure shaped); CJK full-width characters; emoji/ZWJ sequences (measure shaped grapheme clusters); combining marks; tabs in data (render as visible markers or spaces consistently); extremely long single token (URL) → clamp; header longer than all values (e.g., `customer_lifetime_value_estimate`) → may exceed type max for numerics: allow header-driven width up to `maxHeaderW` then truncate header with tooltip; duplicate column names (from joins) → widths keyed by position + name; column added later (schema change) → size on arrival; font not yet loaded (web fonts) → re-run sizing on `document.fonts.ready`; locale change (number grouping changes widths) → re-run for numeric/date columns only if not user-overridden; zoom change → scale widths proportionally rather than re-measure; very wide results (C > 1000) → reduce sample to 10 rows like Glide; `maxW < minW` misconfiguration → swap/normalize.
- **Test cases (input → expected):**
  1. Samples widths [40, 42, 45, 41, 300, 44] px, header 60 → μ=85.3, 2μ=170.7, 300 is outlier → cand = 45 → w = 60 (header).
  2. Samples [40, 42, 300] (n ≤ 5) → cand = 300 → clamped to max initial 480 → 300.
  3. Boolean column header `is_active` (70 px) → w = clamp(70, 48, 80) = 70.
  4. All NULL, header `x` (12 px), NULL placeholder 36 px → w = max(48, 36+padding).
  5. 20,000 columns → only ~8 sample rows measured (10 − 2 = 8, minus tail swap) and completes under budget.
  6. User resized `email` to 333 px previously → 333 used, no measurement.
- **Prior art and evidence:** Glide `measureColumn`/`useColumnSizer` **[Observed: use-column-sizer.ts:15-150]**; harlequin caps column width at half the container (`max(20, width//2 − 2)`) **[Observed: harlequin/src/harlequin/components/results_viewer.py:323-327]**; Beekeeper INI defaults **[Observed]**; egui `Column::auto()` measures content "this frame" **[Observed: table.rs:42-47, 146]**.
- **Used by:** Hacivat; Perde web grids.

### ALG-GRID-03: Selection model (ranges, multi-ranges, row/column selection, keyboard)

- **Problem:** Spreadsheet-grade selection over a virtual grid where most cells are not loaded, supporting single cell, rectangles, multiple disjoint rectangles, whole rows, whole columns, select-all, and keyboard extension — and remaining valid as rows stream in or the result shrinks.
- **Inputs / outputs:** Inputs: pointer events (cell hit, header hit, modifiers), keyboard events, grid dimensions `(R, C)`, column order mapping (visual ↔ model), hidden columns. Output: `Selection` value and change events; derived queries (`contains(r,c)`, `boundingBox`, `cellCount` (u128-safe), `iterateRectangles()`).
- **Algorithm (model):**
  1. `Selection = { anchor: (r,c)?, focus: (r,c)?, rects: Vec<Rect>, rows: IntervalSet<u64>, cols: IntervalSet<u32>, all: bool }`, where `Rect = {r0, r1 (exclusive), c0, c1}` in **visual column order** at the time of selection but stored as model column ids for robustness (see edge cases), `IntervalSet` = sorted disjoint merged half-open intervals (like Glide's `CompactSelection` **[Observed: data-grid-types.ts:590-595]**).
  2. The **active rectangle** is the last element of `rects`; it is always the rectangle spanned by `anchor` and `focus`.
  3. **Click** on cell (no modifier): `rects = [cell]`, `anchor = focus = cell`, clear `rows/cols/all`.
  4. **Shift+click**: replace active rect with `span(anchor, clicked)`; `focus = clicked`.
  5. **Ctrl/Cmd+click**: push a new rect `[clicked]`, `anchor = focus = clicked` (multi-range). Ctrl/Cmd+click on a cell already inside a non-active rect: optional deselect (split rect into up to 4 rects) — Hacivat: support deselect for row/col sets; for rects, keep simple (no hole punching) in v1.
  6. **Drag**: update `focus` as pointer moves; autoscroll when near edges (speed proportional to distance, capped).
  7. **Row header click**: `rows = {r}`; Shift extends interval from anchor row; Ctrl toggles row in set. Same for column headers with `cols`.
  8. **Select all** (Ctrl/Cmd+A when grid focused): `all = true` (do not materialize R).
  9. **Keyboard navigation** (focus moves, selection collapses unless Shift): arrows ±1; Ctrl/Cmd+arrow → jump to data edge (in DB grids: first/last row/column, not "next non-empty" since emptiness is unknown for unloaded pages); Home/End → first/last column in row; Ctrl/Cmd+Home/End → (0,0)/(R−1,C−1); PageUp/PageDown → ± rows-per-viewport; Tab/Shift+Tab → next/previous cell wrapping rows (when not editing); Enter → start editing (or move down after commit, configurable); Escape → cancel edit / collapse selection to focus.
  10. **Shift+navigation** extends the active rect by moving `focus`.
  11. **Membership** `contains(r,c)`: `all` || `rows.contains(r)` || `cols.contains(c)` || any rect contains (r,c). Iterate rects in reverse; for many rects (>32) use an interval tree on rows.
  12. **Normalization for copy** (ALG-GRID-04): if `all` → whole grid; if rows/cols sets non-empty → whole rows/columns; if multiple rects → copy only if they share the same column span (stack vertically) or same row span (concatenate horizontally); otherwise report "multiple selections cannot be copied" (Excel rule) or copy the bounding box with blanks (configurable).
  13. **Count**: `|rows| × C + |cols| × R − |rows|×|cols|` + rect areas (inclusion-exclusion for overlaps approximated by computing union area via sweep line when needed); use u128 for safety (10^9 × 10^4).
  14. **Selection on result change:** on generation change (re-run) → clear; on append (streaming) → keep; on row deletion (edits) → shift intervals; on column hide → drop hidden columns from rects (by id) and recompute visual spans; on column reorder → rects stored by ids become possibly non-contiguous → convert to per-column runs.
  15. Emit `selectionChanged` coalesced per frame; screen-reader announcement of new focus cell and selection size.
- **Data structures:** `IntervalSet` (Vec of `(start,end)` with binary search insert/merge), `Rect` list, column id ↔ visual index maps.
- **Complexity / performance notes:** All operations O(log k) or O(#rects); no per-cell materialization; select-all O(1).
- **Edge cases (exhaustive list):** empty grid (no focus; Ctrl+A no-op); click on header corner (select all); click on frozen column cells (same coordinates space); drag from frozen into scrollable region; Shift+click with no anchor (treat as click); selection spanning unloaded pages; R unknown (streaming) with Ctrl+End (go to last known row, then follow); hidden column inside a rect; column reorder after selection; RTL UI (arrow key direction follows visual layout: Left arrow moves to visually-left cell); IME composition active in cell editor (arrow keys belong to editor); read-only grid (selection still allowed); editing cell + Shift+arrow (extends text selection in editor, not grid); overlapping rects (counted once); Ctrl+click toggling the only selected row (empty selection → focus retained); keyboard repeat at 30+ Hz with autoscroll; multi-window: selection is per grid instance; macOS uses Cmd where Windows/Linux use Ctrl; Turkish keyboard layouts where Ctrl+Alt combos produce characters (AltGr) — never bind AltGr combos.
- **Test cases (input → expected):**
  1. Click (2,3), Shift+click (5,1) → rect r[2,6) c[1,4), focus (5,1), anchor (2,3).
  2. Then Ctrl+click (10,10) → rects = [r2-5×c1-3, r10×c10], active = second.
  3. Row header click 7, Shift+click row 9, Ctrl+click row 20 → rows = {[7,10), [20,21)}.
  4. Ctrl+A on 10^9 × 10^4 grid → `all = true`, count = 10^13 (u128), O(1).
  5. Re-run query → selection cleared; append 1000 rows during streaming → selection unchanged.
  6. Hide column 2 while rect c[1,4) selected → rect columns {1,3}; copy produces 2 columns.
  7. Ctrl+Right in row with unloaded pages → focus at column C−1 immediately.
- **Prior art and evidence:** Glide `GridSelection`/`CompactSelection` **[Observed]**; gpui-component table selection modes and events **[Observed: table/state.rs:57-243]**; rainfrog `SelectionMode` transitions **[Observed: rainfrog/src/components/scroll_table.rs:185-192]**; gobang column-range copy **[Observed: gobang/src/components/table.rs:200-215]**.
- **Used by:** Hacivat; Perde (lighter variant).

### ALG-GRID-04: Copy/paste of rectangular ranges (TSV/CSV/HTML/private JSON)

- **Problem:** Copy selected cells to the system clipboard in formats that paste correctly into spreadsheets, text editors, other DB clients and back into Hacivat (lossless), and paste clipboard content into the grid as edits with correct shape handling, type coercion and NULL semantics.
- **Inputs / outputs:**
  - Copy inputs: normalized selection (ALG-GRID-03 step 12), column metadata, cell values (fetched on demand — may require loading pages), user options (`includeHeaders`, `nullAs`, `format`). Outputs: clipboard items `text/plain` (TSV), `text/html`, `application/x-hacivat-cells+json` (private), optionally `text/csv` for "Copy as CSV".
  - Paste inputs: clipboard items, target anchor/selection, column types, editability (PK presence, read-only result). Output: pending change set (inserts/updates) or a rejection with reason.
- **Algorithm — copy:**
  1. Resolve the copy region: list of row ranges × ordered column ids. Compute `cellCount`; if > `COPY_CONFIRM` (e.g., 1,000,000 cells or estimated > 100 MB), ask for confirmation and offer "Export…" instead.
  2. Fetch values page by page from the core (ALG-UI-01) with `max_cell_bytes = ∞` for copy (full values), streaming into builders; show progress and allow cancellation.
  3. **TSV (`text/plain`)**: for each row join cells with `\t`, rows joined with `\n` (use `\r\n` on Windows? — Excel accepts `\n`; choose `\n` everywhere, but parsers must accept both). Cell encoding:
     - NULL → `nullAs` (default empty string; option "NULL" literal) — note loss: empty string and NULL are indistinguishable in plain TSV.
     - If the value contains `\t`, `\n`, `\r` or `"`, or starts/ends with whitespace that matters, wrap in `"` and double inner `"` (Glide rule: quote on `[\t\n"]` **[Observed: copy-paste.ts:130-135]**; add `\r`).
     - Binary → hex (`\x…`) or base64 per user setting.
     - Numbers → canonical, locale-independent form (`1234.5`, no grouping) for data fidelity; option "copy as displayed".
     - Dates/times → ISO 8601.
  4. **CSV (`text/csv`, explicit command)**: RFC 4180 — delimiter `,` (or `;` for locales whose decimal separator is `,` when "copy as displayed"); quote when containing delimiter, `"`, `\r`, `\n`, or leading/trailing space; CRLF line endings; optional header row.
  5. **HTML (`text/html`)**: `<meta charset="utf-8"><table><thead>…</thead><tbody><tr><td>…</td></tr></tbody></table>`; escape `& < > "`; newlines → `<br>` with `<style>br{mso-data-placement:same-cell}</style>` so Excel keeps them in-cell **[Observed: Glide copy-paste.ts:182]**; NULL → empty `<td>` with `data-null="1"`; raw value in `data-raw` attribute when display differs **[Observed: Glide `gdg-raw-value`]**; numeric cells `style="mso-number-format:'\@'"` for long digit strings to prevent Excel from mangling (e.g., IDs > 15 digits, leading zeros).
  6. **Private JSON**: `{version:1, columns:[{name,type}], rows:[[{v, t, null:bool}]]}` (typed, lossless, includes NULL vs empty distinction) under a custom MIME; in browsers, custom MIME may be restricted (web custom formats use `web ` prefix in `ClipboardItem`) **[Inferred]** — in Tauri, write it via Rust clipboard (arboard/clipboard-rs) or keep an in-app clipboard keyed by a hash of the TSV text to recover typed data on paste from Hacivat.
  7. Write all formats atomically in one clipboard write.
- **Algorithm — paste:**
  1. Read formats in priority: private JSON (if hash matches) → HTML table (parse cells, `<br>` → `\n`, `data-null` → NULL) → TSV/plain.
  2. **TSV parsing** (fixing the Glide astral bug): iterate by UTF-16 index *or* by code-point with a byte/UTF-16 offset tracked consistently; state machine:
     - `Start` (beginning of field): `"` → `Quoted`; `\t` → emit empty field; `\n`/`\r\n` → emit field + end row; else → `Unquoted`.
     - `Unquoted`: until `\t` or newline; `"` inside unquoted is literal.
     - `Quoted`: `""` → literal `"`; `"` followed by `\t`/newline/EOF → end field; `"` followed by other char → treat preceding as literal (lenient: Glide returns to `None` **[Observed: data-editor-fns.ts:140-152]**).
     - Normalize `\r\n` and lone `\r` to `\n` outside quotes; keep inside quotes as-is.
     - Trailing newline at end of clipboard does **not** create an extra empty row (spreadsheets append one).
  3. Result: `src` matrix `m × n` (ragged rows padded with "missing" markers, not empty strings).
  4. **Shape broadcasting** onto target:
     - Single cell source (1×1) + multi-cell target selection → fill every selected cell (Excel behavior).
     - Source `m×n` + target selection exactly a multiple (`k·m × l·n`) → tile.
     - Source `m×n` + single target cell → paste into `m×n` starting at focus; if it exceeds `C` columns → truncate with warning; if it exceeds loaded/existing rows → optionally create new rows (insert) only in table-edit mode with explicit confirmation.
     - Otherwise → paste at selection anchor using source shape (ignore target size) and warn.
  5. **Header detection:** if the first source row equals target column names (case-insensitive, trimmed) → offer "skip header row".
  6. **Coercion per target column type:** empty field → configurable: NULL for nullable non-text columns, empty string for text columns (default), or always NULL; literal `NULL` (case-insensitive) → NULL only if option enabled; numbers parsed with locale-independent grammar first, then user-locale grammar (Turkish `1.234,5`) if that fails and the column is numeric; booleans `true/false/t/f/1/0/yes/no/evet/hayır` (locale-aware list); dates ISO first, then user locale patterns; JSON validated; binary from hex/base64; enum validated against allowed values. Each failure marks the cell as invalid (red) in pending changes; no partial silent conversions.
  7. Skip read-only targets (computed columns, PK in some modes, result-only grids) and report counts.
  8. Produce a single undoable change-set entry (one undo step for the whole paste).
  9. Limit: paste > `PASTE_MAX_CELLS` (e.g., 100k) → offer "Import…" flow.
- **Data structures:** streaming string builders; parsed matrix `Vec<Vec<Option<String>>>` with missing markers; change-set entries `(row_id, col_id, old, new)`.
- **Complexity / performance notes:** O(total chars). Copy of 1M cells with average 20 chars ≈ 20 MB text + HTML (larger) — HTML generation should be skipped above a threshold (e.g., > 200k cells) with only TSV written, to keep clipboard writes fast.
- **Edge cases (exhaustive list):** NULL vs empty string (plain TSV can't distinguish; private JSON/HTML can); values containing tabs/newlines/quotes; values starting with `=`, `+`, `-`, `@` (CSV/formula injection when pasted into spreadsheets — for "Copy as CSV for Excel" prefix with `'` optionally); leading zeros (`00123`); long numeric IDs > 15 digits; `NaN`/`Infinity`; negative zero; decimals with many digits; timestamps with tz; intervals; arrays (`{1,2,3}` PG literal vs JSON); geometry/WKB; binary; very large cells (MB); emoji and astral characters (UTF-16 surrogate correctness); RTL text with bidi controls (preserve); BOM at start of pasted text (strip U+FEFF); trailing empty line; ragged rows; Windows CRLF, old Mac CR; HTML from Google Sheets/Excel/LibreOffice (colspan/rowspan → expand or reject); HTML with nested tables; `&nbsp;` → space (Glide does this **[Observed: copy-paste.ts:242]**); clipboard permission denied (web) → fall back to native Rust clipboard; paste while a cell editor is open (paste into editor text, not grid); paste into filtered/sorted view (target rows by row id); paste into a result with no PK (reject updates, Beekeeper similarly marks tables without PK read-only **[Observed: TableTable.vue:557]**); paste extending beyond last row; pasting into hidden columns (skip, by visual mapping); Turkish locale numbers `1.234,56`; boolean words in Turkish; copy of `all` selection on 10^9 rows (must refuse/redirect to export).
- **Test cases (input → expected):**
  1. Copy 2×2 [["a\tb", null], ["", "x\"y"]] → TSV `"a\tb"\t\n\t"x""y"` (NULL and empty both empty) ; private JSON keeps null vs "".
  2. Paste `1\t2\n3\t4\n` at (0,0) → 2×2 (trailing newline ignored).
  3. Paste `"line1\nline2"\tz` → 1 row, 2 cells, first contains newline.
  4. Paste `😀\t"a""b"` → cells ["😀", "a\"b"] (astral-safe; Glide's parser would misalign).
  5. Single value `0` pasted onto 3×3 selection → 9 cells set to 0.
  6. Source 2×2 onto 4×4 selection → tiled 4 times.
  7. `1.234,5` into numeric column with tr-TR locale → 1234.5; with en-US → parse error flagged.
  8. HTML `<td>a<br>b</td>` → "a\nb".
  9. Copy 300k cells → TSV only (HTML skipped), confirmation shown above 1M.
- **Prior art and evidence:** Glide copy/paste implementation **[Observed: copy-paste.ts:26-310; data-editor-fns.ts:100-163]**; Beekeeper `copyRanges({type:'plain'})` and paste mapping "positionally onto the visible data columns" **[Observed: TableTable.vue:1124, 1702]**; gobang copies a row slice with `join(",")` and **no escaping** (pitfall) **[Observed: gobang/src/components/table.rs:210]**; harlequin copy formats (csv/tsv via extension, parquet, json, orc, feather for export) **[Observed: harlequin/src/harlequin/copy_formats.py:36-343]**.
- **Used by:** Hacivat; Perde (copy-only).

---

## 5. Implications for Hacivat / Karagöz / Perde

1. **Editor**: CodeMirror 6 with: `lang-sql` dialect highlighting; Rust-backed completion source (`CompletionSource` calling a Tauri command with `(doc snapshot hash, cursor, dialect, connection id)`; core returns ranked candidates computed from the schema cache and a parser); lint from core (syntax errors with ranges); inlay widgets for parameter placeholders/types; gutter run-markers per statement (statement splitting from the core); phrases from Fluent.
2. **Grid**: Phase 1 — Glide Data Grid behind `GridAdapter` (ALG-UI-01 cache, ALG-GRID-02 sizing replacing Glide's defaults where possible, ALG-GRID-04 clipboard implemented in Hacivat code rather than Glide's to fix the astral bug and add NULL fidelity). Phase 2 — in-house canvas grid if needed, implementing ALG-GRID-01..04 directly (~3–5k LOC TS).
3. **A11y**: ARIA shadow grid (§3.3.6) is a release blocker.
4. **Perde**: reuse the same grid/editor packages; Perde's audit log viewer benefits from ALG-GRID-01 (millions of audit rows).
5. **Karagöz**: none directly; but audit events should include result-shape metadata (rows, columns) for Perde display.
6. **MCP**: the clipboard private JSON format can double as the MCP "cells" payload format for agents (typed, NULL-preserving).

---

## 6. i18n considerations

- CodeMirror phrases mapped from Fluent; Glide/other grid UI strings (menus, "Loading…", "NULL") from Fluent.
- **Data is not translated**: column names, values, NULL marker style is localized only as a *label* (e.g., "NULL" stays "NULL" by convention in DB tools; make it configurable and keep it ASCII by default to avoid confusion when copying).
- Number/date formatting in cells uses the user's chosen *data locale* (may differ from UI language; e.g., Turkish UI, en-US number format).
- Clipboard: canonical formats by default; "copy as displayed" uses locale formatting.
- Paste parsing accepts Turkish number formats and boolean words only when the column is typed and the option is on.
- Sorting UI lists with `Intl.Collator(uiLocale)`; sorting data is done by the database.
- Keyboard shortcuts must not rely on characters that differ on Turkish Q/F layouts (e.g., `Ctrl+/` is `Ctrl+Shift+7` on Turkish Q) — use `KeyboardEvent.code` for physical shortcuts where appropriate and show the localized key label.
- Glide/Monaco/AG Grid have their own i18n coverage gaps; Hacivat's Fluent layer must override every visible string.

---

## 7. Risks, pitfalls, open questions

- **[Unknown]** Glide Data Grid accessibility quality with NVDA/VoiceOver/Orca — test in spike; if poor, the ARIA shadow must be ours.
- **[Unknown]** Clipboard custom MIME support in WKWebView/WebKitGTK via `ClipboardItem` — plan the Rust-side clipboard path.
- Pitfall: Glide's paste parser astral-character bug **[Observed]** — do not rely on `unquote`.
- Pitfall: AG Grid Community lacks range copy — easy to discover late.
- Pitfall: Handsontable license.
- Pitfall: canvas text measurement differs slightly across webviews → widths differ per OS (acceptable) but snapshots tests must be OS-specific.
- Pitfall: f32 coordinate precision in native toolkits and DOM height limits — always index-anchored scroll.
- Open question: whether to support variable row heights in v1 (expanded multi-line cells) — recommended v1: fixed height + "expand cell" side panel/popup.
- Open question: Monaco only if a future "script" mode (JS/Python notebooks) needs VS Code-level features.
