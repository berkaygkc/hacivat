### Widget texts of the `hacivat-ui` crate (prefix `hui-`).
### Append-only: each task adds a block headed `## <task slug>`.

## ui-text-input

# Shown in the command palette when no command matches the query.
hui-palette-no-results = No matching commands
# Accessible name of the button that clears a text field.
hui-input-clear = Clear
# Key hints in the command palette footer, each after its key (↑↓, ↩, ⎋).
hui-palette-hint-navigate = Navigate
hui-palette-hint-run = Run
hui-palette-hint-close = Close

## task-12-chrome
hui-dialog-close = Close
hui-toast-dismiss = Dismiss
hui-menu-more = More
hui-tab-close = Close tab
hui-tabs = Tabs
hui-window-minimize = Minimize
hui-window-maximize = Maximize
hui-window-close = Close window
hui-menu-app = Application menu

## m3a-t29-a11y-tiers

# The SQL editor's accessible name, read when it takes the focus.
# Variables:
#   $name (String) - the console's name, e.g. "Console 1".
hui-editor-label = SQL editor, { $name }
# A document size in the editor's reduced-mode notice.
# Variables:
#   $n (Decimal) - the megabytes, e.g. 18.4.
hui-editor-t1-size = { $n } MB
# The first line of the notice above a large document's text (reduced mode, 4–20 MB).
# Variables:
#   $size (String) - the document's size (an hui-editor-t1-size message), e.g. "18.4 MB".
hui-editor-t1-title = This document is large ({ $size }), so the editor is in reduced mode.
# The notice's second line.
# Variables:
#   $size (String) - the size reduced mode ends below (an hui-editor-t1-size message), e.g. "3.2 MB".
hui-editor-t1-body = Reduced mode keeps typing fast. It ends when the document is smaller than { $size }.
# The first line of the notice when a line longer than 64 KB put the editor in reduced mode.
hui-editor-t1-long-title = This document has a very long line, so the editor is in reduced mode.
# The notice's second line in that case.
hui-editor-t1-long-body = Reduced mode keeps typing fast.
# Announced to screen readers when a statement starts running.
# Variables:
#   $n (Number) - the statement's position in the run, e.g. 2.
hui-editor-exec-running = Statement { $n } is running
# Announced when a statement finished without rows or a row count.
# Variables:
#   $n (Number) - the statement's position in the run, e.g. 2.
hui-editor-exec-done = Statement { $n } executed
# Announced when a statement returned rows.
# Variables:
#   $n (Number) - the statement's position in the run, e.g. 2.
#   $rows (Number) - the rows it returned, e.g. 40.
hui-editor-exec-rows = { $rows ->
        [one] Statement { $n } executed: { $rows } row
       *[other] Statement { $n } executed: { $rows } rows
    }
# Announced when a statement returned rows and more are left to fetch.
# Variables:
#   $n (Number) - the statement's position in the run, e.g. 2.
#   $rows (Number) - the rows fetched so far, e.g. 1000.
hui-editor-exec-rows-more = Statement { $n } executed: { $rows }+ rows
# Announced when a statement changed rows.
# Variables:
#   $n (Number) - the statement's position in the run, e.g. 2.
#   $rows (Number) - the rows it changed, e.g. 7.
hui-editor-exec-affected = { $rows ->
        [one] Statement { $n } executed: { $rows } row affected
       *[other] Statement { $n } executed: { $rows } rows affected
    }
# Announced when a statement failed and the server gave no SQLSTATE.
# Variables:
#   $n (Number) - the statement's position in the run, e.g. 2.
hui-editor-exec-failed = Statement { $n } failed
# Announced when a statement failed.
# Variables:
#   $n (Number) - the statement's position in the run, e.g. 2.
#   $sqlstate (String) - the server's SQLSTATE code, verbatim, e.g. "42P01".
hui-editor-exec-failed-code = Statement { $n } failed: { $sqlstate }
# Announced when a statement was cancelled.
# Variables:
#   $n (Number) - the statement's position in the run, e.g. 2.
hui-editor-exec-cancelled = Statement { $n } cancelled
# Announced when a statement was not run because an earlier one failed.
# Variables:
#   $n (Number) - the statement's position in the run, e.g. 2.
hui-editor-exec-skipped = Statement { $n } skipped: an earlier statement failed
