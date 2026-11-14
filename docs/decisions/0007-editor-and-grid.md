# ADR 0007 — Editor and grid

- Status: Accepted
- Date: 2026-11-14
- Scope: M3a (UI-EDIT-01 to -23, HV-GRID-01 to -05, -10, -16; R-M3-10, -11, -22, -36 to -38, -49)

## Context

M3a adds the SQL console's editor and the result grid, both written for Hacivat on GPUI (Zed's
GPL editor crates are never read or used):

- `hacivat-text`: a headless text core (rope buffer, change sets, anchors, selections, the
  editing model, IME state, display layout), with no GPUI dependency;
- `hacivat-ui-editor`: the editor view and its element, over a `LanguageService` the app
  provides (`hacivat-sql-editor`);
- `hacivat-ui-grid`: a database-agnostic grid model, one custom element with layered regions,
  the view, and a virtual list, over a `GridDataSource` (the app's result adapter).

Several tasks paint into the same editor element in the same wave, so the seams between them
are fixed before the work starts.

## Rule

Each M3a task that makes a choice in this area appends one section headed
`## m3a-tNN — <topic>` (append-only; a choice made in a fix round goes into the same task's
section before re-review). A section states the choice, the alternatives it rejected, and the
tests or bench numbers that hold it. Sections: Task 1b (seams), Task 19 (the shape-cache key),
Task 20 (the fast-scroll thresholds), Task 24 (the off-thread analysis, R-M3-22), Task 28 (the
preview rule).

## m3a-t01b — seams

**Editor seams with exact signatures and module-owned state (P11).** The element calls each
painting module through one function whose signature Task 1b fixed, and each module owns a state
struct that `EditorView` holds:

| Module | Seam (called from) | State | Fills it |
|---|---|---|---|
| `gutter` | `paint(&GutterCtx, ..)` (`element::paint_frame`); `GutterCtx` carries the frame's `&Snapshot` and `current_statement: Option<Range<u32>>` | — | Task 28 |
| `marks` | `add(.., &ChangeLog, &Snapshot)` (`add_exec_marks`), `set_state`, `clear`, `on_commit(.., &Committed, &ChangeLog, &Snapshot)` (`apply_committed`), `prepaint(&MarkStore, &GutterCtx, ..) -> MarkHitboxes` (`prepaint_frame`: hit targets need prepaint), `paint(&MarkStore, &GutterCtx, &FrameLayout, &MarkHitboxes, ..)` (`paint_frame`) | `MarkStore` | Task 28 |
| `preview` | `on_update(&mut EditorView, Refresh, ..)` (`apply_committed`, a landed analysis through `LanguageService::on_update`, `set_doc_context`), `on_composition(&mut BandState, bool)` (`EditorView::composition_changed`), `current_statement(&BandState, &Snapshot)`, `paint_band(&BandState, &VisibleRows, &Snapshot, ..)` (`paint_frame`), `set_override` | `BandState` | Task 28 |
| `scrollbar` | `paint(&ScrollbarCtx, &MarkStore, ..)` (`paint_frame`); `ScrollbarCtx` carries the `&Snapshot` and the current statement | — | Task 28 |
| `a11y` | `role(&A11yState, &A11yFrame) -> Option<Role>`, `write_info(.., &mut accesskit::Node)`, `synthetic_children(.., &mut A11ySubtreeBuilder)` (the `EditorElement`'s `a11y_role`, `write_a11y_info`, `a11y_synthetic_children` hooks; `A11yFrame` = snapshot, label, config, tier, focus, built in `request_layout` only while `window.is_a11y_active()`, the label re-read only when the UI-text generation changes); `announce(&A11yState, text)` (`EditorView::announce`) queues for the live-region child | `A11yState` (a shared handle; `A11yNodes { pending, .. }`) | Task 29 |
| `large_file` | `tier_for(..)`, `update` (`apply_committed`), `banner(&TierState, ..) -> Option<AnyElement>` (`render`) | `TierState` | Task 29 |
| `ime` | `on_focus`, `on_blur` (the view's focus subscriptions), `after_commit(.., &Committed, UserEvent, ..)` (`apply_committed`), `on_frame` (`prepaint_frame`); the view's `composition_changed` for mark/unmark, and `apply_committed` for the `Committed` of `ImeCore::mark`/`commit` | `InputState` | Task 27 |
| `clipboard` | paste and copy entry points | `PasteState` | Task 27 |
| `commands` | the 45-command table; `overlay(&EditorView, ..) -> Option<AnyElement>` (`render`: the context menu); undo/redo through `apply_committed(c, UserEvent::Undo / Redo, cx)`; `EditorEvent::ContextMenuOpened { at }` | `CommandState` (`tab_moves_focus`, the open menu), `EditorView::command_state() -> &CommandState` | Task 25 |

Fix round 1 of Task 1b added the snapshot, layout, label/config/tier and current-statement
parameters, the refresh, banner, focus/blur/after-commit/frame seams and the two new state
structs, and wired every call site in `view.rs` and `element.rs` (bodies are no-ops), because
the first cut gave the modules no way to map a byte to a line and no call point for the preview
query, the focus rule or the banner. The view also sets `FocusedEditor` on focus.

Round 2 added three things. **`EditorView::apply_committed(c, event, cx)` is the one entry for
every `Committed`**: `commit(Transaction)` calls it after `EditorModel::dispatch`, and undo/redo
and the IME's mark/commit, which produce a `Committed` without a view transaction, call it
directly, so the marks, the service, the tier, the preview and the input handler always hear of
an edit. **Accessibility goes through element hooks**: the GPUI fork builds nodes only from
`Element::a11y_role`/`write_a11y_info` (before the element's prepaint) and
`a11y_synthetic_children` (after it), so the text area is a custom `EditorElement` whose hooks
call the `a11y` seams; `announce` queues its text for a polite live-region child. **The context
menu has a host**: `commands::overlay`, drawn by the view, and `CommandState` read by reference
(it may hold an entity). The element also gained the prepaint seam for the gutter's hit
targets.

`EditorView`'s public methods delegate in one line (`add_exec_marks` → `marks::add(&mut
self.marks, ..)`). Task 19 owns every call site in `view.rs` and `element.rs`; Tasks 27 to 29
own only their modules, so the parallel tasks never edit the same file. The band's state is
`BandState` because `PreviewState` is already the service's verdict enum. Rejected: methods on
`EditorView` filled by each task (three tasks editing `view.rs` in one wave). Task 1b's bodies
paint nothing and change nothing (`MarkStore`'s bookkeeping works, so the app can be written
against it).

**The language service fails closed.** `LanguageService::exec_preview` returns
`Option<ExecPreview>`; the editor's default service (`FailClosedService`) and Task 1b's
`SqlLanguageService` never preview or offer a range (`fail_closed_service_never_previews_a_range`),
so Run cannot run text the user was not shown.

**`hacivat-text` stays headless.** `check-deps` rule `TextCoreNotHeadless` fails the build when
`hacivat-text` reaches `hacivat-ui-core` or `gpui`, so the text core's tests run without a
window and the editing model can be fuzzed.

**Commands and keys.** The editor, grid and console key rows are bound in Task 1b's keymap
blocks before their commands exist (a binding to an unregistered command does nothing); the
app's `keymap_commands_are_registered` test accepts exactly the ids in its `M3A_PENDING` list (68
ids; `every_m3a_pending_id_is_bound` keeps it honest), and the M3a gate (Task 37) requires that
list empty (controller ruling). Every keymap `grid.*` id must be in `hacivat_ui_grid::GRID_COMMANDS`
and every `editor.*` id in `hacivat_ui_editor::commands()`, so a typo in a row fails a test. GPUI's key parser has no Break key, so `exec.cancel` is `cmd-.` on macOS (French
layout `cmd-shift-.`) and `ctrl-f2` on Windows and Linux (and in the DataGrip preset); the
`ctrl-break` row of R-M3-38 is not bound. The Edit menu group shows only once `editor.*`
commands are registered, and its clipboard items carry no key equivalent, so the menu never
takes `cmd-c`/`cmd-v` from a focused text input.

**App seams.** A console tab's view comes from the `console_view` hook (built once per tab, its
root takes the content area as the M2 home does), its decoration (title, environment mark, the
1 px production hairline) from `tab_decor`, the results tile from `results_view`; closing a tab
and quitting ask `tab_close_guard` and `quit_guard`, which may defer (the guard later calls
`Workspace::force_close_tab` or quits). Every empty or stub hook keeps today's rendering, so the
M1 and M2 shots do not move.

## m3a-t19 — the shape-cache key

**Choice.** The editor element shapes each visible row with `WindowTextSystem::shape_line` and
keeps the result in its own LRU (`layout_cache.rs`, at most 4,096 shaped lines or 32 MB
estimated), keyed `(text_hash, runs_hash, font_size_bits, column_window)`:

- `text_hash` is XXH3-64 (`twox-hash`, R-M3-4) of the text that was shaped: the row's text with
  tabs expanded to spaces at their stops, or a long line's window. The key is the **content**,
  never a line index or a version, so an edit reshapes only the rows whose text changed (one per
  typed character), a row moved by an edit above it is still a hit, and scrolling back finds the
  rows shaped on the way down.
- `runs_hash` covers every run's length, font (italic comments, ligatures off), colour,
  background and underline. **Colours are in the key** because the pin's `ShapedLine` keeps its
  decoration runs private: a cached shape cannot be repainted in another colour. A theme switch
  therefore reshapes the visible rows only (about 35 at 600 px); GPUI's frame cache still holds
  the font layout underneath.
- `column_window` is the 512-column window a long line (more than 10,000 columns or 64 KB) is
  shaped through, its start stepping by 128 columns (the M0 spike's `column_window`, ported with
  its tests); columns are UTF-16 units from the line's start, so finding the window's bytes is
  O(log n). x outside the window is the column × the monospace advance.
- A control character is a chip of its own (its `control_name` label, shaped at 11 px under the
  same cache), so a row is a short list of pieces; the ten gutter digits are shaped once per
  font size and ink (`DigitCache`, an app global: shapes belong to the text system).

**Rejected.** The spike's cache keyed by line index with a per-line version table (the table is
spliced in O(n) on every edit, and a row moved by an edit misses); GPUI's two-frame
`LineLayoutCache` alone (scrolling back past two frames reshapes everything); a colour-free key
with recolouring at paint (not possible through the pin's public API).

**Held by.** `one_line_reshaped_per_typed_character` (50k lines: exactly one shape per typed
character), `scrolling_back_hits_the_cache_95_percent`, `theme_switch_reshapes_only_visible_lines`
(shapes = painted rows), `long_line_shapes_a_window` (100k characters: never more than 640
characters shaped, and the window follows the caret), `the_key_changes_with_text_colour_size_and_window_only`,
`digits_are_shaped_once_per_size_and_ink`, `column_window_*` (`tests/element.rs`,
`src/layout_cache.rs`).

**Also decided here.** Rows are painted relative to a `ScrollAnchor` (line and offset into it;
the 6 px top padding is a negative offset at the top), so a million rows stay pixel-exact, and
an edit above the view re-pins the anchor; the view reveals the main caret only after a commit
that changed the selection or edited at the caret. The code area and the gutter share one `panel`
ground (NOTES surface 1 and the approved mock; the plan's `ground` plus a `gutter` fill and a
1 px seam predate the mocks). The caret blinks at 530 ms (GPUI exposes no platform rate at the
pin), restarts on every input, and stops solid after 10 s idle and under reduced motion.

**Review round 1.**

- **A hit is verified, never trusted on the hash** (a safety choice). XXH3-64 is not
  collision-resistant: a birthday search over a free comment suffix is about 2^32 work. Two
  lines with one key would show the first line's glyphs for the second, and a pasted
  `DROP TABLE users; -- …` could be drawn over `DROP TABLE trash; -- …`: the line shown would
  not be the line that runs (the wrong-range anti-goal). A hit counts only when the stored text
  *and* runs equal the ones asked for; anything else is a miss that replaces the entry
  (`LayoutStats::collisions`). The cost is one comparison over bytes that were just hashed.
  Held by `a_key_collision_is_a_miss_never_the_other_lines_glyphs`, which forces one key for
  two lines.
- **The executable comment's inlay well is the element's, not a run background.** At the pin,
  run backgrounds paint only through `ShapedLine::paint_background`. A class with `well`
  (`ExecComment`) now opens a well over its bytes, kept apart from the token classes: it inks
  only the bytes no token claims, so code inside keeps its colours whatever the marks' order
  or priority (`resolve_classes`). The well is painted before the band and the selection, the
  row's full height, in `seam_soft`. Task 24 marks the whole `/*! … */` as `ExecComment` and
  the tokens inside with their own classes. Held by
  `an_executable_comment_sits_on_its_well_under_the_selection` (scene) and
  `a_well_never_erases_the_ink_of_the_code_inside_it`.
- **A long line's window always covers the viewport.** It is 512 columns, or the viewport
  plus one 128-column step when the viewport is wider (`column_window_covers_a_viewport_wider_than_the_cap`).
- **Accessibility in a cached embedding is unverified.** The fork's
  `reuse_prepaint`/`reuse_paint` carry no accessibility state, so on a frame where the cached
  editor is not prepainted again, its node may be missing. Task 29 (the node) and Task 32 (the
  console host's `cached()` embedding) own the test.
- **A right click's pointer** is readable through `EditorView::context_menu_point` only while
  its `editor.context_menu` runs: Task 25 opens the menu there, and at the caret for the
  keyboard.

## m3a-t20 — fast-scroll thresholds and the grid's text cache

**Fast-scroll (R-M3-12).** The grid paints in fast-scroll mode while the scroll moves more than
**40 rows per frame** (`element::FAST_SCROLL_ROWS_PER_FRAME`; a thumb jump or drag step counts as
infinite for one frame). The velocity is `ScrollState::velocity_rows_per_frame`: the rows the user
moved the view between two frames, from the frame's coalesced deltas, with follow-tail snaps,
shrink clamps, resizes and re-runs left out, so a stream never trips it on a still view. In a
fast-scroll frame:
- cached shapes paint as usual;
- an uncached cell whose text is all printable ASCII within 24 characters is shaped at full
  quality;
- any other uncached cell paints its **ASCII-safe prefix of at most 24 characters followed by
  `…`** (the prefix stops before the first character that is not printable ASCII, so it shapes in
  the primary font with no fallback, and the ellipsis says the value goes on); the prefix and its
  ellipsis are cut further to the cell when needed;
- a text with no such prefix, or whose prefix would end at a number's group separator (a digit
  before NBSP, NNBSP or `’`: `12 345` in `fr` or `sv` would read `12…`), paints the **skeleton
  bar** instead;
- chips are skipped;
- the body shapes at most **150 layouts** (`FAST_SHAPES_PER_FRAME`; a miss needs room for two),
  spent **row by row from the top** so whole rows read, and a cell past the budget paints the
  **skeleton bar**: no body cell is ever blank;
- the header is never in fast-scroll: it paints its full-quality shape (cached, or shaped on its
  own budget), so a column that scrolls into view during a drag shows its real name.

(Fix round 1 of the review: the first cut spent the budget column by column and left skipped
cells blank, so a thumb drag blanked most of the grid for its whole length; a prefix without an
ellipsis could read as a different value, `12 345` as `12`.) **Recovery:** a frame that painted anything below full
quality asks for the next frame (`Window::on_next_frame`); that frame is full quality as soon as
the velocity is back under the threshold, so the full-quality frame follows one frame (≈ 8 ms at
120 Hz) after the scroll slows, inside the plan's 100 ms. No timer and no hysteresis.
Rejected: a 100 ms quality timer (a still view would show prefixes for up to 100 ms); hysteresis
on the threshold (no measured flicker to cure). The B1 fling peaks near 5 rows per frame at
120 Hz, so B1 itself never enters fast-scroll; thumb drags and very fast flings do. Tests:
`fast_scroll_caps_shapes_per_frame`, `a_thumb_jump_is_fast_for_one_frame`,
`quality_frame_within_100ms_of_slowing`, `slow_scrolling_is_full_quality`,
`a_sustained_drag_leaves_no_blank_cell_and_no_degraded_header`, `the_fast_budget_is_spent_row_major`,
`the_header_is_never_fast_scroll`, `fast_prefixes_end_in_an_ellipsis_and_grouped_numbers_are_skeletons`.

**The text cache.** The M0 spike's frame-LRU, keyed `(content hash, font id, size bits, colour
class, width bucket)`, 50,000 entries, eviction by frame generation down to 7/8 of the capacity,
never evicting the current frame. It holds bare `LineLayout`s with a colour class per piece and
paints them through `LineLayout::paint` with a one-run decoration built on the stack: a cached
`ShapedLine` would carry a 32-run inline decoration buffer (about 3 KB each, 150 MB at capacity),
and the colour applied at paint time means a theme switch recolours every cached shape and
reshapes none (this needed `DecorationRun` and `LineLayout` in `gpui_surface`, a two-name
re-export). The content hash is a word-at-a-time multiplicative hash; each entry keeps its source
text and a hit compares it, so a collision is a miss, never the wrong string
(`a_content_hash_collision_is_a_miss_never_the_wrong_text` forces one under each of the three
keys). Width buckets are 4 px: an untruncated shape is cached once for every width it fits (`FULL_WIDTH`), a truncated one
is cut for its bucket's narrowest width, so it fits every column width in the bucket. The lookup
order is untruncated, this bucket, then (fast-scroll) the prefix. A scale change clears the cache.
Before shaping, a cell's text is capped at `ceil(col_px / min_advance_px) + 2` grapheme clusters
(the narrowest advance of ten narrow characters in the role's font); a shape wider than its cell
is ellipsized at the last cluster boundary that fits with `…` (binary search). Row numbers paint
digit by digit from ten cached digit layouts at the tabular advance, so scrolling shapes no row
number. **Allocations:** a cache hit allocates nothing; the paint path counts its own allocation
sites (`FrameStats::cell_allocs`: one per shaped layout, one per new entry, one per debug record),
and a frame of hits counts 0 (`cache_hits_allocate_nothing`). A counting global allocator would
also see GPUI's internals, but it needs `unsafe`, which the workspace denies outside
`hacivat-ui-core/src/platform`.

**Visible controls.** `\n` → `↵`, `\t` → `→`, other C0 controls and DEL → their control pictures,
C1 controls → `⟨U+0085⟩`, the bidi controls → named chips (`⟨RLO⟩`, the Trojan Source defence),
each painted as a chip (Fragment Mono 11 `text-2` on a `seam-soft` well, radius 3); the cell's text
is unchanged, so a copy keeps the real characters.
