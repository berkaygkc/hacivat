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

## m3a-t28 — the preview rule

**The rule.** The Run preview band draws only what the language service said Run would execute,
for the exact text on screen (R-M3-8, UI-EDIT-21). `preview::plan` returns a band only when the
service's last answer was asked at the frame's text version; an answer of another version, a
`None` (no analysis of this version), a `Nothing`, a range past the text's end or off a
character boundary all draw no band. The band asks `exec_preview(Execute1)` 50 ms after the last
change or selection move (the selections when there are any, else the carets, as Run asks),
and at once when an analysis lands or the document context changes (unless the timer is
pending). Between an edit and the next answer there is no band: a band rebased through the edit
would be a range the service never vouched for. The two exceptions: the chooser's hover override
(`set_preview_override`, D-011) wins over the service, and while an IME composition is open the
band is frozen at its last answer, mapped through the composition's edits as a container range
(it asks again after the commit; a mapping that fails draws nothing).

**Fix round 1 (review: Critical 1 and 2, Important 3).** An answer is keyed to where it was asked
as well as to the version: `plan`, `status_message` and `current_statement` use it only while
`asked_at == exec_at(snap)` (the carets and selections, built by the one `exec_at` that
`editor.run` also uses). A selection-only commit keeps the version, so without this a caret move,
or a key repeat faster than 50 ms, kept the old statement's band while ⌘↵ would send the new
one; now nothing is drawn until the new position's answer. A context change drops the answer. A
mapped (composition) answer is drawn only while frozen and is dropped when the composition
ends; the freeze keeps an answer only when it was current when the composition began (no query
pending), whichever comes first, `composition_changed(true)` or the first marked text. A text
change clears the chooser's override. **Rejected:** keeping the old band until the new answer
(it is about another place) and asking synchronously on every move (the 50 ms debounce stays).
**Held by:** `a_caret_move_hides_the_old_band_until_the_new_answer` (looks before the timer, also
through three moves 30 ms apart), `a_context_change_drops_the_old_answer`,
`a_text_change_clears_the_chooser_override`, `chooser_override_wins_and_composition_freezes`
(no band right after the composition ends) and
`a_composition_begun_while_a_query_is_pending_shows_no_band`; the mutation "ignore `asked_at`"
fails the first.

**Rejected:** rebasing the last answer through every edit until the next one arrives (fewer
frames without a band, but a range nobody resolved: the §2.3 anti-goal); drawing from the
service's statement spans instead of its preview (the editor's split is not what runs, D-06);
drawing a choices count for `AskPreselected` from a guess (`ExecPreview` has no count; the hint
is left out until the seam carries it).

**Geometry and hues** (design/m3a-query/NOTES.md surfaces 3 and 5, binding over the plan): a
line a unit owns (only blanks and its terminator outside it) gets the full-row band, `blue_tint`
at .55 (dark) or .8 (light) of its alpha, from the gutter's edge to the right edge; a shared line
gets the exact tint over the unit's characters, without the terminator. Uncertain and
no-connection previews take the neutral `hover` fill. Gutter bars are 1 px at x = 4 (`blue`;
dashed `text_2` uncertain; `text_3` no connection; x = 2 beside an executed bar; a 2 px gap
between the units of a separate-statements selection). Marks: Failed is the ink alert glyph,
Cancelling a hollow amber square, Running the amber run marker after 50 ms (no spinner, no
ambient motion); no state is red.

**Marks.** Container ranges rebased from the submitting version through the `ChangeLog`
(`TooOld` and `Deleted` drop the mark); a mark is stale when an edit landed inside it or at its
edges, also between submission and `add_exec_marks`. A mark one edit deleted is a tombstone for
64 text transactions, keyed by where its text was and the text's XXH3 hash; an edit that puts
the same text back at that place (undo) restores it with its state. The 1 Hz repaint is
scheduled from paint, so a window that is not painted schedules nothing, and it stops when no
mark is running or cancelling.

**Held by:** `tests/preview.rs` (`every_preview_state_renders`,
`stale_service_answer_draws_no_band`, `the_chooser_override_replaces_the_band`),
`preview::tests::chooser_override_wins_and_composition_freezes`, `tests/marks.rs` (cases 1–5,
staleness, rebasing, tombstones, the timer, the end-of-line collision) and `tests/gutter_scene.rs`
(bars, band alphas, the exact tint, glyph geometry, no red, the scrollbar track); seven
mutations (the version check, staleness, tombstones, the timer stop, the collision rule, the
track's hit target, the 50 ms grace) each fail their test.

## m3a-t24 — off-thread analysis and the preview rule

**Choice (R-M3-22).** `hacivat-sql-editor`'s `SqlLanguageService` never lexes on the UI thread.
For each open document, `open` starts two tasks:

- **The worker** runs on the GPUI background executor. It is fed by a `tokio::sync::watch` that
  holds only the latest job: the version, an O(1) clone of the snapshot's rope, and the console's
  SQL context.
  - It builds `SqlDocument::from_chunks(&RopeChunks(buffer), ctx, flavor)`. That is the one copy
    of the text, and it is made here.
  - It computes the execution split, the client and `COPY` regions, the tokens inside running
    executable comments, and the statement spans.
  - It publishes the result on a second `watch`.
  - One analysis runs at a time. Jobs that arrive meanwhile collapse into the newest
    (latest wins).
- **A foreground task** stores each published analysis and calls the editor's `on_update`
  notifier from its own task, so the notifier never re-enters a call of the trait.

`change` does two things: it records the `ChangeDesc` (at most 256 are kept) and replaces the
job. It lexes nothing and copies no text.

**Which analysis answers what.**

| Reader | Uses | When the analysis is older |
|---|---|---|
| Highlighting | the latest analysis | Rebased through the editor's `ChangeLog` as `RangeKind::Content`. The visible range is mapped back conservatively, then each mark is mapped forward. A token an edit touched paints plain until the next analysis. An analysis older than the log paints nothing. |
| Statement spans | the latest analysis | Mapped through the recorded changes as containers. A statement an edit touched is `Heuristic` until the next analysis. An unknown chain gives no spans. |
| Token oracle, Run preview, `doc_snapshot` | only an analysis of exactly the editor's version, made under the current SQL context | — |

A new `set_sql_context` bumps a generation and re-analyses. Until the new analysis lands, no
preview, token words or `doc_snapshot` come from the old dialect. A connected console whose SQL
context was never set has no preview, and Run analyses its own snapshot. `ClientFlavor::None`
takes the dialect's own client (`effective_flavor`): psql for PostgreSQL, mysql for MySQL and
MariaDB, sqlite3 for SQLite. So a PostgreSQL console never splits without psql's rules.

**The preview rule (R-M3-8, R-M3-24).** `exec_preview` asks `exec_range::resolve` the same
question Run asks, **in execution mode**, with Run's policy for the action and the console's
remembered chooser kind (`set_remembered_kind`).

| Execution's answer | Preview |
|---|---|
| Runs units M3a sends | `Units`, with exactly those ranges |
| Asks | `AskPreselected`, with the ranges of the candidate the chooser opens on |
| Nothing at the caret | `Nothing` |
| `Ambiguous` | `Uncertain` |
| An unsupported client command | `Uncertain` |
| A `COPY … FROM STDIN` or verbatim unit | `Uncertain` |

For an `Uncertain` preview, editor mode is asked only to find the ranges to draw in neutral. A
disconnected console gets `NoConnection` with the same ranges.

So the preview draws a range as runnable only when it is the range Run sends, by construction.
In the common case this costs one resolve on the UI thread.

A statement span is `Exact` only when the editor split is certain there and the execution split
holds the same certain statement.

**Rejected.**
- **Mapping editor mode's answer, with a certainty check per unit** (the plan's wording). It is
  correct only through Task 13's agreement of the two modes, and it costs a lookup per unit of a
  whole-script candidate. The rule above is true without that agreement. The tests still check
  the agreement for every runnable preview.
- **Lexing on the keystroke frame.** It breaks B3 at 2 MB (R-M3-22).
- **Highlight spans precomputed for the whole document.** That is O(document) memory per
  analysis; marks are built per visible range from `Lexed::tokens_in`.

**Costs.**
- The preview never calls the PostgreSQL parser oracle, because a blocking parse is not UI work.
  A region Task 23 resolves through the oracle therefore previews `Uncertain`.
- Colours come from the whole-text lexing. Past a client command, `COPY` rows or a client
  delimiter they can differ from the split, for example when a quote in `COPY` rows colours the
  SQL after them. This is cosmetic: Run uses the split.

**Held by.** `tests/preview_truth.rs`:
- `preview_equals_execution_for_exact_items` and `preview_follows_the_remembered_kind` (512
  cases by default, 1,000,000 each in the task's run): every runnable preview equals execution's
  answer and editor mode's answer, and offline previews keep the geometry.
- `exact_statement_spans_are_what_execution_runs`.
- `heuristic_boundary_previews_uncertain`, `unsupported_commands_and_copy_never_preview_as_runnable`
  and `stale_snapshot_never_yields_a_preview`.

`tests/worker.rs`:
- `worker_result_equals_a_fresh_document_after_random_edits` (10⁴ edits on PostgreSQL, 2,000 each
  on MySQL and SQLite).
- `change_never_lexes_on_the_calling_thread`.
- `doc_snapshot_is_exact_and_under_the_consoles_context` and
  `postgres_never_analyses_without_its_client_flavor`.

`tests/highlight.rs`: `touched_range_paints_plain_until_the_next_analysis` and
`an_analysis_older_than_the_log_paints_nothing`.

`tests/budget.rs` (perf): `change()` p95 ≤ 0.5 ms, the preview p95 ≤ 2 ms, and one viewport's
decorations p95 ≤ 1 ms on 50,000 lines.

**Fix round 1 (review of m3a-t24).**
- **Selections off the UI thread (I1).** Resolving a selection lexes and splits the selected
  text, about 23 ms for 1.2 MB in release; 64 KiB took 1.3 ms. A selection whose total length
  exceeds 32 KiB (`SYNC_SELECTION_LIMIT`, chosen so the UI-thread work stays under the 1 ms rule
  of §5.4) is therefore not resolved on the UI thread:
  - `exec_preview` answers `None`, so no band is drawn, and posts the question to the
    document's preview worker, which is latest-wins like the analysis worker;
  - the answer is cached under the key (version, context generation, `ExecAt`, action,
    remembered kind), served only for an equal key, and the editor is notified when it lands;
  - shorter selections and every caret still resolve synchronously, within 2 ms;
  - a refused selection is drawn from execution's own answer (both modes answer a selection
    alike), so it is never lexed twice.
- **Setters notify (I2).** `set_sql_context` and `set_remembered_kind` take `cx`. When the value
  changes they call `cx.defer` with the editor's notifier, so a band drawn under the old context
  or preselection is asked again. They never call it synchronously.
- **A worker panic is contained (M1).** `catch_unwind` wraps each analysis and each preview. A
  panic publishes nothing for that version, so there is no preview and no `doc_snapshot`
  (fail closed), and the worker keeps running.
- **Gutter statements (M2).** Every statement from the earliest change onwards is `Heuristic`
  until the next analysis, because lexer state flows forward.
- **Client flavor (M3).** `effective_flavor` always applies the dialect's own client. An explicit
  mismatched flavor, such as PostgreSQL with mysql's rules, is replaced.
- **Closed documents (M6).** A closed document ignores the setters and `on_update` until it is
  opened again.
- **Perf fixture.** The perf fixture is now 2.8 MB: 50,000 lines, 12,500 statements.

## m3a-t29 — editor accessibility node and large-file tiers

**The node is the focusable column, not the text element.** GPUI gives the platform focus to the
node of the element that tracks the focus (`div.rs`: `set_focusable`/`set_focus`), and the
editor's focus is tracked by the view's root column. A node on the inner `EditorElement` (the
round-2 seam above) would never be focused, so assistive technology would announce the whole
window instead of the text. Task 29 moves the declaration to the column: `render` calls
`a11y::declare`, which (only while `window.is_a11y_active()`) gives it role
`MultilineTextInput` and an `a11y_synthetic_children` closure that writes the label, the T1
description, read-only, the text selection, the window's `TextRun`s and the polite live region.
`EditorElement` no longer has accessibility hooks.

**Cached embedding.** The fork's `reuse_prepaint`/`reuse_paint` carry no accessibility state, so
a reused cached view drops out of the tree. `a11y::embed(editor, style, window)` caches the
editor only while no assistive technology is attached; the console host (Task 32) embeds with
it. A fork carry that skips view reuse while accessibility is active would fix every cached view
(the grid included); it is a controller decision.

**Keys and the window.** Line keys follow line-start anchors (`Bias::After`, `MapMode::TrackDel`)
rebased through the change log at build time only (nothing per keystroke); a line takes the
first anchor inside it; keys are never reused; the log running out starts the keys over. A
window is rebuilt only when the text version, its lines or a long caret line's exposed part
change. A line longer than 64 KB exposes 64 KB (around the caret on the caret's line; the end
of the text is a position on its last line). The default window exposes at most 256 KB in all,
the lines nearest the caret first; its AccessKit run nodes are built with it and reused while it
is. The live region is named and valued (Windows and Linux announce the name, macOS the value)
and leaves the tree after a 3 s hold on the executor clock, or at the first frame without
assistive technology, so a subtree re-entering the tree never reads an old status again.

**Tiers.** T0 up to 4 MB and a 64 KB longest line; T1 above, never T2 in M3a. The tier state
records which limit caused T1 (accumulated while in T1); back to T0 once each caused limit is
below 80 % of itself and the other is within itself (exact integer comparison), and the banner's
copy follows the cause. Sizes in the banner come from the host's `large_file::SizeText` (format
locale); the fallback is an unformatted decimal.

## m3b-t31 — multi-cursor

**One selection set, every edit per range.** The M3a `SelectionSet` already held many ranges;
Task 31 makes every command act on all of them in one transaction (one undo group): typing
(`ImeCore::commit` replaces every range outside a composition), Enter, Tab, the deletes, the
moves and the clipboard. Edits that would overlap (two deletions reaching into each other)
merge. Ranges merge by the M3a rule after every change. The main range is the last one added
(a caret added, the next occurrence, the head's line of a box): Escape collapses to it, the
platform's selection and the IME's marked range are its, and **Run follows it** (controller
ruling 2): `preview::exec_at` asks for the main range only, so the band and Run agree.

**Occurrences are byte-exact.** Case- and Turkish-exact matching needs no casing at all
(INV-3): `İ`, `I`, `ı` and `i` are four letters. From a caret the word under it is selected
first (the language service's words in the view), and a search that began that way matches
whole words only (UI-EDIT-05 step 5); a range selected by hand matches as a substring. The bit
is the view's (`CommandState::whole_word_at`, the model's commit count after the search's own
selection: any other selection change ends it). The search reads the rope in 64 KB windows,
never the whole document at once. Select all occurrences stops at 10,000 ranges, the asked-from
one always kept and main, and counts every occurrence (`select_all_occurrences_with`) for Task
33's "first 10,000 of N".

**Column selection counts display cells.** `column_selection` uses the display model's widths
(a tab to the next stop, wide characters and emoji 2, a visible control 1). A tab or a wide
character the box cuts is taken whole; a line shorter than the box gets a caret at its end
(NOTES surface 14; the spec's "skip short rows" is superseded by the approved mock). The mouse:
Option-drag on macOS, Ctrl+Alt-drag elsewhere; Option/Alt-click adds a caret.

**Selection history beside the text's.** `EditorModel` keeps the selection before every
selection-only commit (at most 100 sets and 100,000 ranges), each with the version it is for;
`undo_selection` restores the newest, mapped through the change log (an entry the log no longer
reaches is dropped), as a selection-only commit that records nothing. Text commits add no
entry, so typing is never many steps, and the stacks survive it. One gesture is one entry: a
pointer press opens a gesture (`begin_selection_gesture`) that its drag's changes join, ended by
the release, a move without the button or a blur; select all occurrences is one commit.

**IME: mirror only in plain text (R-M3-64).** A composition opened with several carets is
mirrored: every update and the commit replace every marked range in one transaction, the
platform's range moving the mirrors by the same distances (a mirror that would overlap another
target, or that an outside edit deleted, leaves). The platform sees the main caret's marked
text only; the underline is painted at every caret. Before the first mark the view asks the
token oracle (`TokenOracle::in_string_or_comment`; the SQL adapter answers from its tokens:
strictly inside a string or a comment, or at the end of a line comment or of an unterminated
one). Only a known "outside" for every caret mirrors: when any caret is inside one, or the
oracle does not know (no analysis at this version, as right after typing a quote), the other
carets collapse first and the composition goes to the main caret, said once as a transient
status sentence that holds while the composition is open. A fallback composition the platform
cancels (an empty mark) gives the collapsed carets back.

**Clipboard pieces and the hash fallback.** Copy writes one metadata piece per range, the
pieces joined with the dominant line ending (line-wise pieces carry their own break). Paste
cuts the text into pieces only when the metadata describes it exactly; when the platform
dropped the metadata, an app-wide copy record (the XXH3-128 of the text and its pieces) still
applies while the clipboard holds exactly the copied text, whoever put it there: the same bytes
copied in another app are taken for Hacivat's copy (accepted, review M9). N pieces into N carets
go one each; otherwise text of exactly N lines (a trailing break aside) goes one line per caret;
anything else goes whole to every caret, line-wise (above the caret's line) when every piece
is a whole line.

**Linux primary selection.** 100 ms after a selection change a non-empty main selection is
written to the primary selection (a generation counter debounces; nothing while composing); a
middle click pastes it at the pointer without moving the selections, its text through the
paste's background half (the size refusal before anything is built, then normalising). The
primary read itself is synchronous: the fork has no asynchronous one (fork carry). Both are
`#[cfg(target_os = "linux")]` at the platform calls; the rest compiles everywhere.

**Status.** `EditorStatus::carets` is new; `selected_chars` covers every range, counted with the
rope's char index (O(k log n)) and cached by the model's commit count, so a frame reads it in
O(1) (ledger 207).

## m3b-t32 — folding

**Lines, not bytes.** A fold keeps its first line (the header) and hides the lines after it to
the line its range ends on; when only blanks stand before the range's end on that line (a
block's closing `)` or `END` at the start of its line) that line stays visible. The folded
header composes into one visual line (`compose_folded`: the header's text, then a placeholder
the element draws as the "⋯ n lines" inlay). UI-EDIT-07 step 2's byte model (the last line's
tail after the placeholder) is not needed by NOTES surface 15, which draws the header line and
the inlay only. Every range spans at least three lines (NOTES M3c-43, `MIN_FOLD_LINES`), so the
spec's two-line `--` comment folds no more.

**Through edits.** A folded range maps with its start biased after and its end before (text
typed at the start goes above the fold, whose header stays its header; text typed at the end
stays outside it, so Enter or a key before a block's visible closing `)` or `END` never joins
the fold: fix round 1); an edit inside it keeps it; one edit deleting all of it drops it. An edit that deletes the header drops it: the
range's first character (a block's `(`, a statement's first token) or the header line's break
(joining the first hidden line, deleting the header line). `prune` then drops a fold left on
fewer than three lines. This replaces UI-EDIT-10 step 4's reconciliation after a reparse: the
fold goes at the edit, not at the next edit inside it, and needs no candidate list (the
language service's answer may be mapped from an older analysis and could not tell a fresh
"none" from a late one). **Undo does not restore a dropped fold:** the fold set is view state,
not text, and is not in the history.

**Cost per commit** (fix round 1). Each fold keeps its header and last line. `map` leaves the
folds before the change alone, shifts the ones after it, and maps those around it in one sorted
pass of the change's mapper; `prune` reads the text only for the folds the change reached (the
others shift by its line delta) and checks nesting with a stack; `reveal_selection` binary-
searches the sorted caret lines. With 3,000 folds a one-caret keystroke costs about 40 µs
(map, prune and reveal), a 1,000-caret keystroke about 3.5 ms, a 1,000-caret reveal about
0.1 ms (release). The height map is cached by version, fold generation, line count and row
height; one decoration request spans rows whose hidden gaps are under 64 KB, so Fold all asks
once per frame.

**The chunked height map.** `HeightMap::Chunked` keeps the hidden runs sorted and merged with a
prefix sum of hidden lines; `y_of`, `line_at_y` (through the visible row) and `row_of` are one
binary search over the runs (28 ns a pair at 50,000 lines and 1,000 folds, budget 5 µs).
Without a fold the editor builds the uniform map, so the M3a paths are unchanged. The element
walks visible lines only (`next_visible`), a row over a fold is its header, and decorations are
asked per run of rows between folds, never for hidden bytes. The scroll anchor a new fold hid
re-pins onto the fold's header (`ScrollAnchor::snap_visible`).

**A caret never hides.** Folding moves a range end on a hidden line to its header's end; a
selection end that lands on a hidden line (go to line, an error position, undo) opens the folds
hiding it unless the range holds the fold whole (select all keeps the folds); a vertical move
counts a fold as one row. Run at a caret on a folded statement's header resolves the statement
as before; the band and the gutter bar collapse onto the header's row.

**Marks in a fold.** A mark whose glyph line is hidden paints on the fold's header row; the
row shows the one that most needs the user (NOTES M3c-7): a cancel not answered > failed >
running > cancelling > queued > cancelled > skipped > done > stale. The app encodes "awaiting
an answer" as a running mark and "rolled back" through its label (NOTES ruling 31), so they
rank as those states until `ExecMarkState` grows the arms. A failure inside says where: "failed
inside the fold · line 9" with its SQLSTATE chip. Range bars collapse onto the header's row.

**Candidates** (`hacivat-sql-editor::folds`, worker side, once per analysis): statements from
their first token to their terminator, parenthesised groups (to before the `)`), `BEGIN … END`,
`CASE … END`, `IF … END IF` and `LOOP … END LOOP` (a `BEGIN` before `;` or
`TRANSACTION`/`WORK`/… is a transaction command; an `IF` is a block only once its `THEN` comes;
a `CASE` open at a `;` is closed only by `END CASE`), dollar-quoted bodies (their text lexed
again, so the PL/pgSQL blocks inside fold), block comments, runs of `--` comment lines (a blank
line ends one), and `-- region`/`-- endregion` pairs. Not yet: `INSERT … VALUES` rows and a
MySQL procedure's `BEGIN … END` under a client `DELIMITER` (M5 lexer work). The service maps the
latest analysis's ranges through the edits since with the fold's own mapping (`map_fold`); the
editor asks once per version and again when an analysis lands (`fold_gutter::on_analysis`).
Folding and unfolding are said in the editor's polite live region, and a folded header's text
run carries "n lines folded" as its description; the chevrons' AccessKit buttons and keyboard
focus are Task 34's. No default chord: the four commands are palette
rows (controller ruling 7).

## m3b-t35 — grid model

M3c Task 35 (HV-GRID-02 frozen, -05 multi-rect, -06): the view map the grid paints, keys and
hit-tests through; frozen columns; grow and shrink; edge autoscroll; auto-fit and per-type
widths.

**`ViewMap` (the API Task 36 sorts through).** The three variants of Task 1b stay; `Permuted`
and `Filtered` gain an `index: ViewIndex` field, the inverse (model row → view row) that makes
`model_to_view` O(1) (1M-row permutation: model→view→model under 50 ns, the perf line). Build
the maps with `ViewMap::permuted(order, tail_from, loaded)` and `ViewMap::filtered(rows,
source_loaded)`: they build the index (O(n), so Task 36 calls `permuted` on the background
executor with its sort and publishes the result with `GridView::set_view_map`). Match variants
with `..`. The index is a cache: it never decides `==` or `Hash`; it has no public
constructor (fix round 1), so the two builders are the only way to make one and no map builds
its index on the UI thread by accident; every hit is checked against the order, and a miss is
trusted only when the index was built from the same `Arc` (it keeps a `Weak` to it, so the
allocation cannot be reused), so an index moved into another map scans, never answers wrong. A
sparse filter (few kept rows of a huge source; a dense table only up to 16 entries per kept row
or 64 Ki) gets a sorted `(model, view)` table instead (O(log n)), and a filter keeps its kept
rows sorted for counts. `ViewMap` is `Send + Sync` (a compile-time check). Other helpers:
`ViewMap::with_loaded` (rows appended after a sort join its unsorted tail), `is_identity`,
`shows`, `count_shown` (how many of a model range it shows, without mapping).

**A late publish (T36 carry).** `set_view_map` has no generation guard: a sort that finishes
after a re-run would put the old order on the new rows. Task 36 checks the result's generation
before it calls `set_view_map` (or adds a checked variant).

**Selection under a map (digest 04 R15).** The focus, the anchor and the row set are model
rows; rectangles are view rows. `Selection::on_view_map_changed` (called by `set_view_map`)
clears the rectangles, keeps the focus and the row and column sets, and selects the focus cell
alone where the new map shows it when nothing else is left (a focus a filter hides selects
nothing and stays for the next key). Copy (`normalize_for_copy`) takes a row set as the view
rows that show it (rows a filter hides drop out); the copy region is in view rows, which Task
36's copy translates back. The counts never map rows (fix round 1, I5): `cell_count_in` counts
a row set with `count_shown` (a sort's is its length clipped to the loaded rows, a filter's a
binary search in its sorted kept rows) and builds view runs only when rectangles share the
selection with a row set; `size_in` gives the strip's `rows × cols` of one piece the same way.
The rectangles are counted by a sweep down the rows with a segment tree over the compressed
column edges, O(k log k) for k ranges (T37's 10,000 one-cell ranges count in well under a
millisecond). Every Shift-extend of rows under a sort or a filter takes the rows **as
shown**: `extend_rows_to_in` (the gutter, the row drag, Select row) and `extend_to` from a row
piece (Shift+arrow, Page, ⌘↓, Shift+click on a cell after a gutter click; fix round 1, I1) make
a whole-width rectangle of view rows (`cols: 0..u32::MAX`, which later extends keep whole),
because a model row range would select other rows than the user extended over; one row stays a
model row piece. An anchor the map hides makes the extend a click (M1).

**The grid and the map.** `GridView::set_view_map` stores the map (a sort's follows the loaded
rows), updates the selection and asks for a frame. **A re-run drops the map** (new rows have no
order); Task 36 publishes a new one. The frame paints view row `r` with the cells of
`view_to_model(r)`; the gutter numbers view rows; a filter's kept rows are all the grid
scrolls over; clicks, drags and keys turn view rows into model rows through the map; the
selection boxes of a row set under a map are the runs of view rows near the view that hold it.

**Frozen columns.** `ColumnLayout` keeps the count the user set (`frozen`) and the count the
frame shows (`frozen_shown`: at most those leaving `MIN_WIDTH` of the body to scroll,
`fit_frozen` per frame), so a narrow pane keeps scrolling columns and widening it brings the
frozen ones back. `left_x` keeps its meaning, but `ScrollState` gets a viewport of the body less
the frozen width and a content of `scroll_width()` (the columns after the frozen ones), so a
column scrolled into view comes out from under the frozen ones and a frozen column never
scrolls. The element paints the body and the header in two segments, each with the same layers
and its own (disjoint) clip: the frozen columns (unshifted, clipped to the frozen width), then
the scrolling ones (shifted by `left_x`, clipped right of it), with a 1 px `seam-strong` edge on
the last frozen column (NOTES M3c-21: no glyph). Nothing frozen is one segment, pixel for pixel
M3a's frame. Both segments share the fast-scroll budget, frozen cells first (the columns kept in
view are the last to fall back to skeleton bars), and the kept column buffer, so a frame of
cache hits still allocates nothing.
`grid.freeze` freezes up to the focus column and emits `FrozenChanged`; when that would leave
less than `MIN_WIDTH` to scroll it emits `FreezeRefused` and the app says why in a notice
(`hv-grid-freeze-refused`; fix round 1, M3). `grid.unfreeze` unfreezes; the host sets a count
with `GridView::set_frozen` (no event). A re-run with the same columns keeps the count.
`frozen_shown` is O(frozen) (nothing frozen: O(1)).

**Grow and shrink.** Cell → row segment → row → all, and back, at the focus. The row segment is
the focus row across the **pane** holding the focus: the frozen columns, or every column after
them (ruling 2 as the controller changed it in fix round 1: it never depends on the horizontal
scroll). It is skipped when it is the cell or the whole row, so with nothing frozen the ladder
is cell → row → all. The row step is a model row piece, so it follows the data through a
re-sort. Any other shape grows to everything and shrinks to the focus cell.

**Multi-rect.** ⌘/Ctrl-click and -drag add a range (M3a's pieces); the Excel copy rule and its
refusal are unchanged. `Selection::ranges` counts pieces; the app's strip says "Selection 2
ranges · 10 cells" (`hv-strip-ranges`) for more than one.

**Edge autoscroll while still (ledger 213).** While a cell or row drag rests past the body's
edge, a 16 ms timer on the GPUI executor (in the grid's window) does what a move to the resting
pointer would: scroll by the distance, capped at 40 rows (and, for a cell drag only, 120 px
sideways; a row drag in the gutter scrolls only up and down), then extend. It stops (fix round
1, I2, I3):
- when the pointer comes back inside, or the mouse button goes up;
- when the grid loses the focus (an `on_blur` hook, and a focus check on every tick) or the
  window is deactivated (`observe_window_activation`); the drag ends there, a resize reporting
  the width it reached;
- when a tick changes neither the scroll nor the selection (the result's end, or a side that
  cannot scroll). This bounds a lost mouse-up: the timer runs at most to the end of the loaded
  rows, then stops; a later move with the button still down starts it again.

A tick that changes nothing repaints nothing, and a tick allocates only when the selection
moves (it extends only when the cell under the pointer is not already the focus).

**Auto-size.** The initial size bounds a column's data width by its kind (`width_bounds`: flags
≤ 96 px, numbers ≤ 240, timestamps, UUIDs and binary ≤ 320, text and JSON ≤ 480; the header
still widens past them, up to 320). `grid.auto_fit` (selected whole columns, else the focus
column) and a double-click on a header's right edge queue an auto-fit: measured from the same
samples inside the same 240-cell frame budget, up to 2,000 px, outliers trimmed as M3a's
`fit_width` does; the width counts as the user's (even when it did not change, so a pending
initial size never overrides it) and a change is reported as `ColumnResized`. The fast-scroll
thresholds are unchanged.

**Keys (fix round 1, I4).** The datagrip preset binds Extend / Shrink selection in a grid,
`ctrl-w` / `ctrl-shift-w` [Grid] on Windows and Linux, as the Task 1b note deferred to Task 35:
with a grid innermost ctrl-w grows the selection instead of closing the tab (P-M3b-21);
elsewhere it still closes the tab. Ctrl-only letter chords: no AltGr collision on any CI layout.

**Resize.** The hovered header edge shows a 1 px `text-2` handle; while dragging, the guide is
dashed; the view draws the width tag under the header ("136 px · Drag to resize ·
double-click to fit", `hv-grid-width` + `hv-grid-resize-tip`) while hovering or dragging an
edge, on the side of the edge with room (right of an edge in the grid's left half, else left of
it), so it never hangs over a neighbouring pane (fix round 1, M4). Frozen headers carry the
tooltip `hv-grid-frozen-tip`. The labels are `GridLabels`'
`frozen_tip`, `resize_tip` and `width`.

## m3b-t36 — local sort

M3c Task 36 (HV-GRID-07, R-M3-70; NOTES surface 18, M3c-19, M3c-20): a result grid sorts its
fetched rows on this computer, by the column's database collation, off the UI thread; the copy
follows the order shown.

**Comparisons (`hacivat-values::valfmt::compare`).** `comparator_for(col, collation)` gives a
column's ascending order. Text compares by the database collation mapped to ICU4X
(`resolve_collation`: PostgreSQL ICU `tr-x-icu` / `und-u-ks-level2`, glibc `tr_TR.utf8`, MySQL
and MariaDB UCA `utf8mb4_tr_0900_ai_ci`, `utf8mb4_turkish_ci`, `…_uca1400_turkish_ai_ci`, SQL
Server `Turkish_CI_AS`; accent-insensitive → primary, case-insensitive → secondary, else
tertiary), by code point for a binary collation (`C`, `*_bin`, SQLite `BINARY`), and by code
point **as a fallback that the tooltip names** for anything else (`utf8mb4_general_ci`,
`NOCASE`, `default`, an unknown name, or no collation at all). The UI locale and the process
locale are never read. Exact decimals compare exactly (the 38-digit form by its words, longer
ones by their digits), never through a float; floats by IEEE order with NaN after +∞; temporal
values by instant; enums by declared label order; bytes and UUIDs by bytes; decode errors after
every value; NULLs where the engine puts them ascending (`nulls_at`: last on PostgreSQL and its
forks, Oracle, ClickHouse; first elsewhere). A descending key reverses the whole order, NULLs
included, as `ORDER BY … DESC` does. Ties are equal: the sort is stable.

**No column carries its collation today.** PostgreSQL's row description has none; MySQL's column
definition carries the *connection's* result character set (`character_set_results`), not the
column's collation. So `results::sort::collation_of` answers `None`, every text column sorts by
code point, and its tooltip says the collation is not known (`hv-grid-sort-tip-unknown`). The
mapping is ready for a catalog lookup through `OriginHint` (ruling request in the Task 36
report).

**The order (`hacivat-ui-grid::model::stable_order`).** A bottom-up stable merge sort of row
numbers under the job's comparator (runs of 32 by insertion), which looks at a cancellation flag
every 65,536 comparisons and before each merge pass; above 1,000,000 rows it splits the rows over
up to four scoped threads and merges the runs pairwise, left first. It knows no values.

**The job (`hacivat-app::results::sort`).** On a request the UI thread takes one snapshot and
builds each key's comparator; the background executor extracts each key column into one
`ValueRef` per row (borrowed from the snapshot), sorts, and builds `ViewMap::permuted(order,
rows, rows)` with its O(n) index. The UI thread only publishes: `GridView::set_view_map` and
`GridView::set_sort` (the header indicator and its tooltips). **Generation guard:** each grid's
request number is bumped by every request, Cancel and Clear sort; a finished job publishes only
when its number is current, the adapter is at the generation it sorted, and it is still the
running job. A newer request raises the older job's flag. A re-run drops the map and the
indicator (the grid does both at the new generation's first frame); sorting again is the
user's call. Rows that arrive during or after a sort join the unsorted tail
(`ViewMap::with_loaded`), and the grid draws a 1 px dashed `text-3` line above the tail's first
row (`GridView::tail_boundary`, derived from the map: `order.len()` while `loaded > tail_from`).

**The header.** A sorted column keeps room after its name (6 px gap) for a 12 px blue
`arrow-up` / `arrow-down` and, with two or more keys only, its level number in Fragment Mono 11
blue (a `Roles::sort_level` role); the name ellipsizes first. An end-aligned header puts the
indicator at its end. Its tooltip (`HeaderTip::sort`) adds the key's text: "Sort level 1,
ascending. Text compares by the column's collation, tr-x-icu." (`hv-grid-sort-tip`),
`hv-grid-sort-tip-cp` for a collation with no equivalent, `hv-grid-sort-tip-unknown` when none
is known, `hv-grid-sort-level` for a column that holds no text.

**The note row** (under the result toolbar, M3c-20): "Sorting N rows…" with Cancel once a job has
run 200 ms (the static amber marker); on a partial result (anything but `Complete` and
`Truncated(FollowingResultSets)`) "Sorted locally: the database might order differently. Only
the N fetched rows are sorted." with Clear sort; a tail on a partial result says both facts in
one row with Re-sort and Clear sort; a tail on a complete result "N new rows at the end (not
sorted)" with Re-sort.

**Copy follows the view (ledger 253 I5).** `CopyRegion::rows` are view rows. The tile passes
the grid's map to `copy::copy_region_in`, which reads each view row's model row, in view order;
a view row the map does not show is left out. `copy_region` / `tsv` are the identity's.

**Commands.** `grid.sort_asc` / `grid.sort_desc` sort by the focus column, `grid.clear_sort` goes
back (palette-only; Task 39's menus add "Add as sort level N" through
`GridView::request_sort`). A header click still selects the column, as in M3a.

**Fix round 1.**
- *Comparisons.* PostgreSQL 17's interval ±infinity sort around the finite intervals (I1). An
  integer against a float compares exactly (the float's floor as an integer, then its
  fraction), and an exact decimal against a float compares exactly through the float's exact
  decimal expansion, so a SQLite column of mixed integers and reals has a total order (m1; the
  earlier "no engine types a column that way" was wrong for SQLite). SQLite booleans are the
  integers 0 and 1. `timetz` ties order by the zone in seconds west, as `timetz_cmp`. MySQL's
  partial zero dates compare by their parts (`2020-00-15` between 2019 and `2020-01-00`).
- *Collation names.* `resolve_collation(engine, name)` reads only the engine's spellings (a
  PostgreSQL collation named like a MySQL one is not taken for MySQL's). SQL Server's `*_BIN`
  / `*_BIN2` compare UTF-16 code units. MariaDB's `nopad` names map like their PAD SPACE twins.
  PAD SPACE collations (MySQL's and MariaDB's except `_0900_`, `nopad` and `binary`; every SQL
  Server collation) ignore trailing spaces (`pads_trailing_spaces`); PostgreSQL and SQLite do
  not.
- *Jobs.* A job stops when its state goes: a newer sort, Cancel, Clear sort, a re-run (the tile
  drops the old run's state when it shows the grid at the new generation) and a closed tab or
  console (`prune`). The `Running` value raises the flag when it drops. A request is refused,
  claiming nothing, while the grid has not painted the adapter's current generation, and a
  job publishes only to a grid showing the generation it sorted. A worker thread's panic is
  re-raised (`stable_order`), never a cancel; the job catches it, keeps the order shown, and
  says so in a notice (`hv-grid-sort-failed`).
- *Memory.* Key extraction holds one `ValueRef` (32 B) per row and key: about 32 MB per key at
  one million rows, freed when the job ends. No budget check (accepted at review).
- *Copy.* Every copy entry point (`copy_region`, `tsv`, `estimated_bytes`) takes the `ViewMap`.

## m3b-t37 — grid find and go to row

- *What is searched.* The grid find runs Task 33's matcher (`console::find_bar::build_matcher`)
  over each visible column's **display string** (`valfmt::format_cell`, `Purpose::Cell`), the
  text the grid paints: grouped numbers, the display zone, the grapheme cap. NULL and a decode
  error never match. A value the store cut at its cell cap is searched up to the cut and the
  count says "may be incomplete". A match is a **cell**; every matching range in it is marked.
- *Off the UI thread.* The scan runs on the background executor in batches of 1,000 view rows
  from the focus row (to the end, then from the top), in slices of about 8 ms that each report
  to the bar; a generation number and a stop flag drop a stale scan. The UI thread merges a
  slice's hits (at most 100,000 kept; the count goes on) and moves the focus once. The rows with
  a match are concatenated, and the filter's maps built, on the background executor too.
- *Marks.* `GridView::set_highlights` takes a closure that answers each painted cell's
  `CellMarks` (byte ranges of its display text, the current one); layer 5 outlines each range's
  glyphs (`text-3`), the current one filled (`blue-tint-strong`, blue outline). A text shown
  with control chips is marked whole (its display bytes are not the value's).
- *One owner of the view map.* `results::find::show_base` composes the sort's order with the
  filter: the sort publishes its `Permuted` map there, Clear sort hands it `None`; with a filter
  on, the grid shows the filter in the sort's order (composed on the background executor), the
  filter in the source order after Clear sort, and the sort again once the filter is off.
  (Task 36 review m6.) The sort still sorts every fetched row, so turning the filter off needs
  no new sort.
- *Go to row.* `parse_goto` reads the format locale's digits and grouping (the separators are
  read from how the result's display writes a number) and a column by position, exact name,
  ASCII case-insensitive name, then a tolerant key (ASCII case, Turkish i, Latin accents).
  `GridView::go_to` takes **view** rows and focuses the cell (`focus_cell`, the public "focus
  this cell"); past the fetched rows of an open cursor the app fetches in steps of 100,000 rows
  (so Stop fetching can stop between them), asking first above a million rows to fetch.

## m3b-t40 — grid AccessKit subtree

- *Only while a screen reader is attached.* `element::a11y_rows::prepaint` (the Task 1b hook)
  returns at once while `Window::is_a11y_active` is false, and the view's render adds nothing;
  no node, name or announcement is built (R-M3-69, HV-GRID-16's "zero cost").
- *Shape.* The grid's node stays on the view's focused root (T30): role `Grid`, its label, the
  column count, and a row count that includes the header row (row 0; view row `v` is row
  `v + 1`); an open result counts its loaded rows and says "More rows are available", also
  under a filter (it ran over the fetched rows; fix round 1, m5). Its
  children are one zero-paint `Row` element per row (the header, then the view rows on screen
  ± 5), each at its row's place, whose cells are **synthetic leaves** (`ColumnHeader`/`Cell`)
  with their bounds, column index (place among the visible columns, frozen ones first), name,
  selected state and, on a header, the sort direction with the sort tooltip as description.
  The fork's synthetic children are leaves only, so rows must be elements; no fork patch.
- *Stable ids.* A row element is keyed by its model row under the grid's own element, a cell by
  its column under its row: ids follow `(grid, model row, column)` through scrolls, sorts and
  filters. Rows past the loaded ones are one "loading" row each, without cells.
- *Focus.* The focus cell is the grid node's **active descendant** (the AccessKit node
  property, written by the root's synthetic-children hook once the rows have declared their
  cells; AccessKit's consumer moves the platform focus to it). A find step and go to row move
  the focus through `focus_cell`, so the current match is the active descendant.
- *Names.* The cell's `GridFrame::a11y_cell` (the app: `Purpose::A11y`, controls spoken as one
  space, 1,000 graphemes, cached per data generation and display settings) capped at 1,000
  graphemes, never splitting one; NULL, a truncated value (with its whole size) and a
  placeholder have their own words (`hui-grid-a11y-*`).
- *Announcements* (fix round 1). A polite `Status` leaf of the grid's node, labelled and
  valued. Only what the focused cell's node does not say, once the selection has been still for
  200 ms (a held arrow key says one thing): a focus that settled in another row says "Row 3 of
  1,000"; a move within the row, or the same cell moved by a re-sort, says nothing (the active
  descendant is read); a selection of more than one cell says its size
  (`hui-grid-selection-summary`). A timer drops it after 3 s, and once a painted frame carried
  it, a render after a 250 ms pause in the grid's painting drops it: a grid shown again (a tab
  switch) never reads it twice. The find's count and the go-to dialog keep their own nodes; the
  grid never says a count of matches.
- *Actions.* Each data cell answers Focus (focus the grid and the cell), ScrollIntoView and
  ShowContextMenu (focus the cell, then the menu as Shift+F10); a canvas registers them once the
  leaves have their ids.
- *Tab leaves the grid from any cell* (R-M3-69; M3a left only from the last loaded cell). The
  arrows move between cells. `grid.next_cell` / `grid.prev_cell` keep their keys and leave the
  command palette (`commands::PALETTE_HIDDEN`): their titles no longer say what they do.
- *Cost.* The perf test (`node_build_under_1ms_per_frame`) times the hook's pure half for about
  1,200 cells: the nodes, the compare with the frame before, and the AccessKit nodes the
  elements write (0.15 ms). GPUI's half (the row elements' layout, the leaves' pushes, about
  3,600 action listeners per paint) runs only with a screen reader attached; Task 45 checks it.

## m3b-t38 — result views and the value viewer

- *The axis swap is a source.* A grid's columns, layout, selection and events all come from its
  source, so the Transposed view is a grid of its own over
  `hacivat_ui_grid::TransposedSource` (`element/transpose.rs`): the first column holds each
  shown table column's name, then one column per table row in the order the table shows them
  (the sort's and the filter's view map, read when the source is built), titled by its row
  number; one row per shown table column. `TransposedSource::table_cell` maps a cell back. The
  hook in `transpose.rs` keeps a transposed grid's name column frozen and shows no sort
  indicator (sorting by a row is not offered, alg/22). The table grid keeps its own selection,
  scroll, sort and frozen columns, and keeps taking its frames at no size while another view is
  shown, so its state (a re-run's rows, the map that follows them) is the one every view is
  built from. Transposed is disabled above 100,000 shown rows; find, Filter rows and go to row
  are the table's and are off while transposed.
- *The text view* is a Pretty ASCII table of the shown rows and columns in the raw channel
  (control characters escaped, NULL as `NULL`, columns capped at 255 characters), built on the
  background executor in batches (widths, then lines) with progress and Cancel; above 100,000
  rows it asks first. Lines are what copy copies.
- *Model cells.* The record inspector and the value viewer read the focused **model** cell, so a
  sort or a filter never changes which value they show; the inspector steps through the rows as
  shown.
- *The value viewer copies raw bytes.* It shows the raw channel; JSON Pretty only re-indents
  (key order and every token kept) and XML Pretty only breaks lines; Copy raw value copies the
  raw text whatever is shown. Its well wraps the text itself (one mono advance) and builds only
  its visible lines.
- *Re-run with a 64 MiB value limit* is `ResultsEvent::RerunWithLimit`: the run UI handles it as
  Re-run (the tab's unit, only while the editor still holds the text that ran, through Run's
  pipeline: parameters, classification of the bytes sent, the gate, the confirmations); only the
  run's `FetchPolicy::cell_cap` differs (`orchestrator::start_run_capped`).
- *The tooltip* (`valfmt::tooltip`, `Purpose::Tooltip`) is the raw channel under a quarter of the
  cell's grapheme cap (`TOOLTIP_FRACTION`), marked and chipped when cut.

## m4-t08 — tree widget

`hacivat_ui::tree` (plan-m4 Task 8): the explorer's virtualized tree, with no database
knowledge. Bodies from `m4-spike-tree` @ `2a31b8b` (R-M4-40) on Task 1b's seam; the spike's view
is split into `view/{mod,anchor,keyboard,pointer,element}.rs`.

- *Flat-vector model.* The expanded tree is one `Vec<FlatRow>` (32 bytes a row: key, depth,
  kind, position and set size; about 5 MB at 160,000 rows), built by an iterative DFS (no
  recursion limit; a 50,000-deep tree is tested) on a model generation change, and spliced on
  expand/collapse (`expand_at`/`collapse_at` cost the subtree, not the tree). A frame reads only
  the rows on screen ± `TREE_OVERSCAN_ROWS` (2). Measured (release, this Mac, 162,041 rows): the
  full flatten 1.8 ms (gate 50 ms), row build p95 0.37 ms, 0.36 ms with assistive technology
  (gate 2 ms). `index_of` stays linear (about 0.1 ms at 160,000 rows per key press); a key →
  index map would cost about 5 ms per rebuild.
- *Fibonacci key hasher.* `KeySet` (expansion, selection) hashes a `TreeKey`, already a stable
  128-bit hash, with one fold and multiply instead of SipHash: the expanded-set lookup is the
  flattening's hot loop. The high half is spread before the fold (keys whose halves are swapped
  or equal stay apart) and the product's high bits are folded into the low ones the table
  indexes by.
- *Anchoring by key.* Expansion is a set of keys owned by the view, so it survives generations,
  and a key that disappears and returns comes back expanded. On a generation change the first
  visible node keeps its screen position (relocated by key after the rebuild); expand/collapse
  above the viewport shifts the offset by the spliced rows; collapsing the subtree that holds the
  top row makes the collapsed node the top row.
- *Lazy children.* `NotLoaded` draws a chevron and emits `Expand(key)` once per expansion: also
  for a node restored expanded, a node revealed by an ancestor's expansion (collapsing an
  ancestor forgets the announcement of every expanded node it hides), and a node whose children
  the host drops back to `NotLoaded` while it is expanded, so a cancelled load never strands a
  node. `Loading`/`Failed` draw one child row with the model's label at `text-2` (`Failed` adds
  `CircleAlert` in the chevron column, never red; no motion); neither takes the cursor.
- *R-M4-49: group headers take the cursor, never the selection.* A group header that can expand
  (a kind group) is a cursor stop for ↑/↓/Home/End/Page keys, and ← / → / Enter / a click
  collapse or expand it; Enter never activates it; `SelectionChanged` never names it; type-ahead
  skips it. The cursor (`TreeView::cursor`) may therefore be a header while the selection
  (`TreeView::selection`) stays on the last row. **Ruling (fix round 1, C1):** row-scoped host
  commands (View DDL, Copy qualified name, Delete…, Open data) act on the selection only while
  the cursor is not a group header; with the cursor on a header they do nothing, except Refresh
  and Expand/Collapse all children, which act on the header. A header that cannot expand (the
  M2a environment labels, NOTES M4 surface 1) is a label the keys and the pointer pass over.
  **Ruling (accepted in review):** a header still asks for its menu (right click, Shift-F10, the
  menu key: `ContextMenu` with the header's key), because the mocks give kind groups a menu
  (NOTES M4 surface 5); the selection does not move, and the menu acts on the event's key.
- *R-M4-50: defaults the host sets.* The type-ahead fold defaults to `Locale { turkic: false }`
  (the access-key fold); the explorer sets `Fold::Tolerant` for Turkish users (Task 14, D-074).
  Neither fold calls a casing function: the tolerant fold is `I i İ ı` as one letter plus the
  access-key fold's table. The buffer resets after 1 s of the GPUI executor's clock. Single
  selection by default (M4's explorer); with single selection a collapse (by the user, or the
  host's `set_expanded`/`set_expanded_keys`) that hides the selected row clears the selection
  (`SelectionChanged(None)`). A row that leaves the model in a new generation stays selected by
  key (a refresh that drops and brings back a namespace keeps it), so "never a hidden row" holds
  after a collapse, not across generations. Whether a multi-select keeps hidden rows is left to
  the milestone that adds a bulk action (multi-select keeps them today).
- *Lag-free accessibility hand-off.* While `Window::is_a11y_active()`, the root declares `Tree`
  (named by `set_a11y_label`) and its synthetic children are one `TreeItem` per row on screen
  ± 5: level, expanded state, position and size in set, selected (never on a header), bounds,
  and a description joined from the status text, the host's `a11y_description` (the
  environment, read-only, the badge: the marks carry no words of their own), the row tooltip
  and the pill's tooltip; a loading row's description is its count, a failed row's its retry
  word. The element computes the items in its prepaint and hands
  them to the root's synthetic-children callback through a shared `Rc`; GPUI runs that callback
  after the children's prepaint, so the items describe the frame being drawn (no one-frame lag,
  unlike the grid's T40 subtree). Ids derive from `(key, row kind)`. The cursor item is the
  active descendant (a header too). Items that take the cursor answer Focus, Click (activate a
  row, toggle a header), Expand, Collapse (no-ops on a leaf), ScrollIntoView and
  ShowContextMenu. Inactive: no role, no item, no string.
- *The border-strip finding.* GPUI paints a border-only quad as four strips, so the selection's
  1 px blue left edge is a child fill, and the keyboard-focus ring is an absolutely placed
  overlay (no layout shift), drawn only after keyboard input.
- *Row order and give-way* (fix round 1, I2; NOTES M4 surface 1 and cross-cutting #5): chevron ·
  kind icon · name · type text · badge · flags · status · pill · spacer · meta mark + meta ·
  row action. The row is laid out by the `give_way` primitive over the shaped widths of its
  texts: the meta ellipsizes and then its mark leaves, the pill leaves whole, the badge
  collapses to its glyph (the text becomes the glyph's tooltip; a badge without a glyph leaves
  whole), the type text ellipsizes, and the name ellipsizes down to about **seven** characters
  (the mock's number, ruled over M2a's six) with its full text as the tooltip. The chevron, the
  icon, the flags, the status glyph and the row action never give way. Group headers draw no
  flags: their one glyph is the filter glyph (`filter_glyph`, NOTES M4 surface 5).
- *Seam fields (fix round 1, R-W1-1, additive).* `ChildState::Loading { label, running, count }`
  (the static amber run marker in the chevron column only while running; the label at 12 px
  `caption`, `text-2`; the count in mono), `ChildState::Failed { label, retry }` (the alert glyph
  and the label in `text` ink: **ruling**, the reviewed mock supersedes the plan's `text-2`
  because the user must read the error; a ghost retry button emits `TreeEvent::Retry(parent)`),
  and on `TreeRowData`: `meta_mark` (`Running`, `Ok`, `Glyph`), `type_text` (mono 11 px
  `text-2`), `strong` (`body_strong`), `pill` (16 px, outlined, `inlay`; blue when explicit;
  emits `TreeEvent::Pill(key, its bottom-left)`), `badge_glyph`, `tooltip`, `row_action`
  (drawn while the row is hovered or selected; emits `TreeEvent::RowAction`), `filter_glyph`
  and `a11y_description`. The pill and the buttons are pointer only and change neither the
  cursor nor the selection.
- *Events resolve rows by key* (fix round 1, I1). A frame's pointer listeners capture the row's
  key, not its index, and find its row when the event arrives: GPUI dispatches input to the
  last drawn frame's listeners, and a publish re-flattens before the next frame, so an index
  would name another node. A key that is gone does nothing.
- *The cursor never strands* (fix round 1, M3). When the cursor's row leaves the shown rows (a
  new generation, the host's expansion), the nearest row above it that is still shown takes it
  (a hidden row's nearest shown ancestor, a dropped row's neighbour; the rebuild remembers up to
  256 rows above), without touching the selection. PageUp/PageDown move a viewport less one
  *row* and snap to the nearest cursor stop between (M6).
- *Defensive flattening* (fix round 1, M5, M7). Each `FlatRow` carries its header, selectable
  and focusable bits, read from the model once per flatten (the generation contract re-flattens
  when they change), so key presses and type-ahead scan bits and ask the model only for the
  text they read. A node already on the walk's path is shown but not expanded again, and
  nothing is expanded past depth `u16::MAX`, so a host cycle cannot loop the UI thread. A
  chevron double click toggles once, like a header's.
- *Named exceptions.* `tests/tree_seam.rs` (T1b's) checks the seam against the widget
  (ratified by the controller in fix round 1); `hacivat-ui/src/lib.rs` line 38 (the module
  line's comment).
- *Keys.* The widget reads key-downs while focused and binds nothing; every chord with ⌘, Ctrl,
  Alt or fn is left to the host's keymap; nothing fires during an IME composition.

## m4-t14 — explorer tree

The M2a connection list became the database explorer tree in place (R-M4-18): `hacivat-app`'s
`explorer::adapter::ExplorerModel` implements Task 8's `TreeModel` over the profiles and one
`Option<Arc<CatalogSnapshot>>` per profile; `connections::explorer` hosts a `TreeView` of it.

- *Shape.* Roots: the M2a environment headers (labels, 23 px, no chevron, skipped by the keys;
  NOTES ruling 1) and the profile rows. PostgreSQL: databases (the profile's own first, others
  by collation; out of scope dim with "Add to scope", R-M4-14) → contributed nodes → schemas →
  kind groups → objects → child groups → leaves, and a collapsed Built-in group last when the
  catalog holds system schemas of that database. MySQL/MariaDB: schemas called "database"
  (`model-kind-schema-mysql`). SQLite: `main`, `temp`, attached; no Built-in (R-M4-6). The kind
  groups are one fixed order (`rows::DisplayGroup`); a kind without a group (an extension, a
  partition, an `Unknown` object with its raw server kind as type text) follows the groups.
- *Laziness.* `children` is computed the first time the tree asks (only expanded nodes are
  asked) and kept for the generation; the sorted members of a kind group (ICU4X collation of the
  UI language through `hacivat-values`' collator, byte order breaking ties; columns by ordinal)
  are kept across generations in `ExplorerCache` while their arena is the same `Arc`. The model
  never does I/O and reads the snapshot synchronously (spec §5.4 rule 2).
- *Keys.* `xxh3_128("hv-object\0" ‖ profile ‖ kind code ‖ raw kind ‖ (database ‖ server id) or
  (path ‖ overload signature))`, every part length-prefixed; groups, profiles, environments and
  Built-in hash their own domains. Namespace roots (databases, schemas) are keyed by path only,
  so a schema keeps its key when it is first loaded with an OID. A PostgreSQL rename keeps the
  key (the OID); a name-only engine's rename hint (`CatalogDelta::renamed`) moves the expansion
  (`adapter::set_catalog`).
- *Fail-closed.* No catalog or a generation-0 catalog shows the child row "Not loaded", never an
  empty database (T1c m-B5); a namespace known only by its load state shows that state; a
  relation of a `Names`-level namespace shows "Not loaded" under it, never zero columns.
- *Filter* (D-074). `FilterJob` matches profile names and hosts (M2a) and every loaded name with
  `hacivat-l10n`'s search key (Contains; Turkish-tolerant while the toggle in the field is on, its
  default on for a Turkish UI language or region). Matches reveal their ancestors (the explorer
  sets the tree's expansion to them and restores the previous one when the filter clears).
  More than 2,000 names run on the background executor, latest wins.
- *Profile commands.* The `Explorer` bindings (Enter, ⌘E, ⌘D, ⌘⇧C, ⌫, Shift-F10) run only while
  the tree's keyboard row is a profile root (`connections::explorer::is_visible`); elsewhere the
  chord passes through to the tree (carries-w2 T14). Any row selects its profile in
  `ConnectionsModel` (the sheet follows it). A context menu acts on the event's key: a profile
  root opens its connection menu; the other nodes' menus are Task 24's.
- *Seams for Task 24.* `adapter::set_catalog` / `catalog_of` (the snapshot each root shows) and
  `ExplorerContributor::child_node` (additive, default `None`: the rows of lazily listed
  contributed children). `connections::explorer::shown_rows` lists what the tree shows.
- *Named exceptions.* `tests/shots.rs` (the explorer surfaces' marker is the tree's row file);
  `tests/connections_explorer.rs`, `tests/connections_live_m2b.rs`, `tests/cross_surface_m2b.rs`
  (the M2a `explorer-row-*` selectors became the tree's; the filter toggle is a Tab stop);
  `tests/support/mod.rs` block `// ## m4-t14-explorer` (catalog builders: `hacivat-meta`'s
  `fixture` is behind `test-support`, which `hacivat-app`'s tests do not enable).
- *Fix round 1.*
  - *Ruling R-W2-12 (I1).* The tree's focus carries the `Explorer` key context only while its
    cursor is on a profile root, so the Explorer keymap rows act only there and every other row
    gets its keys. `ConnectionsModel::selected` follows the profile that owns the cursor row, so
    the sheet and the palette's and welcome page's connection commands (enabled wherever the
    cursor is) name the same profile; `is_visible` keeps its M2a meaning. The M4 rows bound in
    the `Explorer` context (F12, F5, ⌘←, ⌥⌘C, ⌥⌘F) follow the same rule: Task 24 decides
    their context when it fills them.
  - *Fail-closed filter (I2).* `FilterResult` counts what it could not search (`unsearched`:
    connections with no catalog or generation 0, databases with nothing read, state-only
    namespaces; `unloaded`: relations of a `Names` namespace). "No objects match" only when
    nothing matched and both are 0; with no match those rows stay in sight (the roots are never
    hidden), and "Searches loaded names only…" says why. With matches the filter narrows as
    M2a's did.
  - *Bounded apply, cancellable jobs (I3).* A filter reveals the ancestors of its first 200
    matches (`REVEAL_HITS`); the rest are kept, behind "n of total". Applying the 150,000-match
    `c0` result over 15,000 tables (15,225 rows) takes 2.9 ms at p95 on the UI thread, measured
    in `explorer_perf`. A job checks a cancel token every 1,024 names and stops when a newer
    query took over (1.2 ms vs a 100 ms whole run).
  - *Minor.* An active filter runs again when a profile is added or renamed, or the language
    changes, and not again on a rename hint's extra notification; the cache prunes with a
    `HashSet`; accessible descriptions join their words with ", " and dim rows say "out of
    scope"; key and index parts show their expressions; a foreign key's arrow is Fluent
    (`hv-explorer-fk-target`); Escape clears the filter field (on an empty field it is the
    shell's Escape and returns to the tree) and ↓ moves into the tree.
  - *Named exceptions (accepted).* The M2a profile-root debug selectors are replaced by the
    tree's own; `connections/explorer.rs` is a near-rewrite; `tests/m4_routing.rs`'s copy-chord
    test gets a profile so the list has a root for its cursor.
- *Fix round 2.*
  - *Ruling R-W2-12b (key contexts split).* The focused tree carries `ExplorerTree` on every
    row and adds `Explorer` after it while its cursor is on a profile root
    (`shell::hooks::add_focus_context` / `remove_focus_context`: one handle, several contexts,
    a named exception in `shell/hooks.rs`). `Explorer` keeps the connection rows (Enter,
    Shift-F10, Backspace/Delete, ⌘E, ⌘D, ⌘⇧C); `ExplorerTree` takes T1b's M4 rows (F12 View
    DDL, F5 Refresh, ⌘←/Ctrl+← Collapse all, ⌥⌘C/Shift+Alt+C Copy qualified name, ⌥⌘F the
    filter) in all three default keymaps, chords unchanged (named exception in Task 1b's rows),
    and the M4 explorer commands are registered in it. Each command decides whether it applies
    to the cursor's row (R-W1-5).
  - *Ruling R-W2-13 (I3 accepted).* A broad filter expands the ancestors of its first 200
    matches only; the others are kept behind their groups' "n of total" counts. The mock
    (surface 4) shows no "N more matches" hint, so none is drawn: a T32 item.

## m4-t24 — live explorer

- *Decision.* `explorer::live::CatalogHost` is a thin UI wrapper over `CatalogHub` (R-M4-31):
  per profile it keeps the hub's `ProfileCatalog`, what it last told the engine (scope, scope
  filters, level pills, the current database and schema) and the engine's progress, held
  deletions and the namespaces the user asked to refresh. Hub calls go through a
  `CatalogSource` trait (the hub in the app; an engine over Task 15's fakes in tests) and run
  on the `CoreRuntime`; the UI awaits `tokio::sync::oneshot`/`mpsc` only.
- *Coalesced notify (spec §5.4 rule 3).* Per open profile, forwarders on the runtime put the
  catalog's events, the engine's events and its progress into an inbox and wake the UI only
  when the inbox was clean. The UI drains it, reads `Catalog::current()` once and calls
  `set_catalog` only when the generation moved: one explorer update per batch. Progress is
  stored only when it differs (no repaint without a change).
- *App start (R-M4-15, PO-Q19).* The workspace calls `live::start` after `workspace.json` was
  restored. Cache files are looked for on the blocking pool; plaintext profiles with one open
  at once; an encrypted profile opens (the keyring call) only when its root is expanded in the
  restored workspace, else its root shows the key glyph and the first keyring call is the
  user's expansion (a user `Expand` event) or a connect.
- *Connect and disconnect.* The connections mirror (`connections::live::apply_state`, named
  lines) tells the host: Connected opens the catalog if needed (cache only, no server query)
  and sends `on_connect` once the cached namespaces were published **and the profile's root is
  expanded** (at once when it already is; else at its first expansion). Deviation D-1 from the
  plan's "on connect": reading the catalog of a subtree nobody looks at opened metadata
  connections on every connect of the M2/M3 tests and consumed their scripted driver answers
  (eight failures in four files); the condition is one line in `live::connect_engine`, to drop
  if the controller rules for "always on connect" (with those tests adapted); any non-connected state stops the engine through
  `CatalogHub::shutdown` (the cache stays, the explorer keeps the last snapshot and the
  offline strip "Showing cached metadata from {date}" names its newest `Loaded.at`). The
  current schema comes from the user's session (the focused console of the profile,
  `SessionSnapshot::schema`), never from the metadata connection.
- *Deltas.* Renames move expansion (Task 14); dropped and `out_of_scope` objects lose their
  expansion; an out-of-scope object is a removal, never a deletion notice. Engine events map to
  Fluent toasts (`hv-metadata-ns-dropped`, `intro-hint-level-lowered`,
  `model-cache-write-failed`); `DatabaseUnlisted` is kept per profile (log only).
- *Mass deletion (D-039, R-M4-22).* A `PendingConfirmation` for a namespace the user asked to
  refresh opens the confirmation (Remove from cache, Default; Keep cached, Primary and initial
  focus; Escape is Keep); any other one keeps the namespace's stale badge with the held count
  in its tooltip. `PendingWithdrawn` closes an open question.
- *Actions (mock 5).* Commands act on the cursor row; a right click moves the cursor first.
  Qualified names quote through `sql_ident::quote` (re-exported as `hacivat_meta::name::quote`,
  a one-line named exception: the app has no `sql-ident` dependency). Open data opens a console
  over `SELECT * FROM <qualified name>` and runs it with the editor's own Run once the console's
  session is idle and its SQL context read (`console::commands::open_with_text`). "Hide objects
  like this" appends `kind:-prefix*` to the namespace's view filter in the profile data file;
  the adapter hides those rows and marks the group header; the catalog is unchanged. The
  profile root keeps its M2a menu (its tests pin it); F5 refreshes a root.
- *Expanded state (PO-Q15).* `workspace.json` gains `explorer.expanded: { uuid: [hex] }`,
  written next to the layout; restored keys are applied only when present in the cached
  snapshot (walked from the root through restored keys), and their `Expand` events load
  nothing.
- *Pending until other branches merge.* The hub's SQLite readers (R-W3-10,
  `with_readers`) and the per-pass progress counts (R-W3-12) arrive with Tasks 18 and 17; the
  user's attached SQLite files are not copied onto the reader (no `attached_files` seam exists).
- *Fix round 1 (opus review).*
  - *I-1.* Opens and closes of one profile run one at a time in a lifecycle task that reads
    the latest wish at every step (`want_open`): an open answered after a close is closed
    by the same task, so no stale shutdown can stop a newer engine. `release_cache` sets
    `deleting`: nothing opens the store until `reopen`, which Save calls on every path
    after the release, with the profile as stored then (an "encrypt on" Save never
    recreates a plaintext cache).
  - *I-2, I-3.* Open data's queued run is of exactly the text it generated (it is dropped
    when the editor changed, the session broke or the tab closed). On a database-level
    engine `open_with_text` refuses, with a notice, a database other than the one the
    console's session uses (the user's session context, else the profile's database), and
    one it cannot tell.
  - *R-W3-10.* The hub builds SQLite's introspector with its per-schema readers
    (`hub::introspector_with_readers`, a named exception in Task 15's file).
  - *m-1.* Task 12's filter keeps an escaped trailing whitespace (`trim_item`, a named
    exception in `filter.rs`). The adapter compiles a view filter per item when the whole
    does not parse, so a bad item hides nothing and never undoes the others; "Hide objects
    like this" refuses, with a notice, an item that does not parse.

## m4-t25 — DDL view

- *Decision.* The DDL view tab (`ddl_view::view::DdlView`, design/m4-explorer surfaces 10
  and 11) shows exactly `hacivat_ddl::produce::create_script_with` for one object, the strip's
  options (`GenOptions` from `ProfileUserData::ddl_view`, remembered per profile through
  `explorer::userdata`) and the profile's current schema (the user's focused console's
  session; none qualifies every name). The text sits in a read-only `EditorView`
  (`EditorConfig::read_only`); nothing localizes or edits it. Copy puts the raw text (`\n`) on
  the clipboard; Open in console opens a console on the profile with the text inserted, not
  run (`console::open_with_text(…, run: false)`; a database-level engine passes the object's
  database, which it refuses with a notice when the console is on another).
- *The catalog seam.* `ddl_view::source::DdlCatalog` is what the view reads and asks: the
  snapshot (synchronously, as the explorer reads it) and `elevate`, `check_state`, `refresh`,
  `load_facet` as tickets awaited on a GPUI foreground task (spec §5.4 rule 2; the I/O runs on
  the `CoreRuntime`). The app's is `EngineCatalog` over the live host's `IntroHandle` (a
  profile whose catalog is not open answers at once with `NotImplemented`); tests and Task
  15b install their own with `set_catalog_provider` (plan Task 25: "a fake `IntroHandle`").
  At this base no source text and no facet value has a home in the model (`SourceRef` is a
  hash; `load_facet` is Task 15b's stub), so `EngineCatalog::sources` answers "not loaded":
  views, routines, triggers and server text show "couldn't read the definition", grants carry
  `ddl-grants-not-loaded` (F31).
- *Loading.* An object whose definition is the server's text (R-M4-24) is read at `Sources`
  first when its namespace holds less (one fragment, `elevate`); a definition still missing is
  asked for once as the `Sources` facet; the grants facet is asked for once while Grants is
  on. Each wait shows "Reading the definition of …".
- *Freshness (surface 11; round 1 on Task 22).* When a script is first shown, and after every
  Refresh or Reload (F5 in the text, `ddl_view.reload` in the `DdlViewCode` focus context), the
  view takes Task 22's per-key verdict through `IntroHandle::check_state_verdicts` (a named
  exception in Task 15's `engine/{mod,scheduler}.rs`: the state check's jobs report
  `Vec<(ObjectKey, Staleness)>` on a oneshot before any re-read; a failed or cancelled check
  drops the sender). `Current` → current; `Changed` → the engine re-reads it, and a script
  that differs from the shown one is "changed on the server" (Reload), an equal one current;
  gone from a loaded namespace → "no longer exists" (Close tab); `Unknown`, a `Dropped` the
  catalog does not confirm, or no verdict → "couldn't check". An `Unknown` never shows as
  current. A later catalog whose script differs marks the view changed (never an automatic
  reload); a namespace marked stale turns current into "couldn't check". Server text from the
  `Sources` facet is current only while that facet is `Loaded` and not stale in the snapshot
  (T19 review m6: `DdlSources::server_text` has no load state of its own).
- *Fix round (review I-1 to I-3, m-1 to m-8).* Freshness never carries over to a new text: a
  first text and every replacement (an option, the server toggle, a facet, Refresh, Reload)
  is checked again, with a "Checking whether … changed" caption until the verdict lands. A
  `Current` verdict (or `Changed` with a re-read) reads current only when the catalog still
  gives the shown text (else "changed": a publish landed during the check) and vouches for it:
  the namespace not stale and, for `Sources`-facet server text, that facet `Loaded` and not
  stale. The same rule demotes current on a newer catalog with the same text. A stale
  namespace now reads "couldn't check" even under a `Current` verdict (stricter than round 1).
  Comparisons regenerate with the schema the shown text was made with. Names on single-line
  chrome draw their control characters visibly (`ddl_view::visible_name`: `\n` → `␊`). Run is
  disabled in read-only editors (`console/commands.rs`, named exception). MySQL/MariaDB show no
  Owner option and their server text keeps `DEFINER=` verbatim.
- *Options per engine.* "Definitions provided by server" is hidden on PostgreSQL, shown on
  MySQL/MariaDB, and shown checked and fixed on SQLite (its tables always show
  `sqlite_schema.sql`, F10; Task 19 ignores the flag there, the T7 carry M8; ruling R-W4-12),
  with a tooltip saying why (`hv-ddl-view-server-fixed-tip`). Qualification
  and Constraints hide while the shown text is the server's (NOTES ruling 16). Grants: PG and
  MySQL/MariaDB; Owner: PostgreSQL only (fix round m-8).
- *Tabs.* The object table keeps a handle per open object and drops it when the view is
  released (the tab closed, T1b carry m3); an object with a server id renamed on the server
  keeps its handle and the tab title follows the new name. `ddl_view.copy` copies the active
  DDL view's text (palette only, R-W0-6). `explorer.view_ddl` (F12, the node menu) opens the
  cursor object's view, a column's table's. Provided toolbar actions
  (`ActionSurface::DdlViewToolbar`) are buttons in the toolbar.

## m4-t27 — metadata UI

- *Decision.* Refresh after DDL reaches the explorer through one `RefreshCoordinator` per open
  catalog (`explorer::refresh_prompt`), made when the live host opens a profile's catalog and
  dropped with its engine (named lines in `explorer/live.rs`: `opened`, the lifecycle task's
  close step, `teardown`). What a closed coordinator owed for want of a connection is kept and
  run by the next one once the profile is connected again; `reconnected()` is called on every
  `Connected` of an open catalog, also when the engine survived a reconnect (T21 m6).
- *Runs.* The orchestrator hands the classifier's reports of each unit and the coordinator's
  inbox to `ExecService::run_with_refresh` (named lines in `exec_ui/orchestrator.rs`, a named
  exception in `hacivat-exec` `service.rs`, ADR 0006 `m4-t27`), which installs
  `CatalogRefreshSink::for_run` in the run's auditor chain: DDL run in any console reaches the
  explorer without the UI waiting on anything. The toolbar's Commit and Roll back (and every
  prompt that ends a transaction through `exec_ui::tx::end_tx`) tell the coordinator
  (`tx_ended`; a commit whose answer was lost is `Unknown`, refresh more); a session no console
  holds any more is `session_closed`.
- *What the user sees* (mock 9). Ask: a wide sticky toast with the coordinator's count
  (Refresh, Always refresh — which sets the preference through `update_settings` — and Not
  now); it never takes the focus, so a run is never blocked (NOTES ruling 9). A newer question
  replaces the older toast (the coordinator folded its targets). Dismissing the toast with its
  ✕ answers nothing: the namespaces stay "out of date" and the next question carries the
  targets. Never: nothing refreshes; the engine publishes the namespaces stale ("out of date",
  T14's badge) until a refresh. `CoordEvent::Stale { scope: true }`: the stale schema list is
  badged "out of date" on its database row (the root where schemas are top level) until it is
  read again. Inside a transaction: the namespace row carries the 7 px amber stamp
  (`MetaMark::Running`) with "{count} changes … wait for COMMIT" in its tooltip, and the
  console's output logs `hv-metadata-log-pending`; the counts are read again on the core
  runtime after each run and each transaction end (the coordinator's inbox drained first).
  Dynamic SQL (`EXECUTE`, `DO`, `CALL` with an opaque catalog effect) under Always and outside a
  transaction: "Refreshing all of {schema}" and why.
- *R-W4-8.* A deletion held for a namespace badges it "out of date" with the held count
  whatever its load state; a `Partial` namespace (no stale bit) keeps the held count until the
  engine withdraws it.
- *Stale objects* (mock 11). A state check that found an object changed and could not read it
  again, or gone while the catalog still holds it (the DDL view's check, one named line in
  `ddl_view/view.rs`), marks the object's explorer row with the DDL view's sentence as its
  tooltip and accessible description, for the namespace generation the check saw: a later
  publish of the namespace (a refresh, a pass) or a check that finds it current clears it
  (`explorer::stale`).
- *T25 m-9.* The DDL view's Reload of an object whose namespace is stale reads the namespace
  (a pass publishes it current); otherwise the object alone. A decided verdict then shows the
  text as current.
- *Settings › Metadata* (mock 13): refresh after DDL (Segmented), the background check (a
  150 px Select: Automatic, Off, 5, 10, 30 min), the cache per connection and in all (a whole
  number and MB/GB, saved in MiB, set to the nearest bound outside 1 MiB–1 TiB) and "Clear
  cached metadata…" for the active console's connection with its size on this computer (read on
  the blocking pool). Every write goes through `shell::update_settings`. The background check
  and the cache caps are read by the hub when a catalog opens: a change applies from the next
  connect. `metadata.clear_cache` (the explorer) and the settings button ask first (Cancel
  focused); Clear closes the store (`live::release_cache`), deletes the profile's cache files on
  the blocking pool, shows no cached tree and reopens the catalog when connected (a full read).
- *Carries done.* `SettingsPage::show_section` is the navigation's handler and the shots'
  fixture (m3c.md §8.1, P-M4-18). The view filter's per-item fallback and "Hide objects like
  this" split items as the grammar does (`filter::split_items`, T24 nit). A queued Open-data
  run is dropped when the connect it waited for ends without a connection (T24 nit). The
  "Introspect live" choice is kept per profile (`ProfileUserData::live_system`, sorted; T26
  carry); handing it to the overlays waits for T26's merge.
