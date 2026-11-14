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
