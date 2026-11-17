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

## m3a-t27-clipboard

# A size in whole megabytes, rounded up (the paste refusal; the T1 banner may reuse it).
# Variables:
#   $n (Number) - megabytes, e.g. 31.
hui-editor-size-mb = { $n } MB
# Toast title: a paste would take the document past the editor's size limit.
hui-editor-paste-refused-title = Couldn't paste
# Variables:
#   $size (String) - the pasted text's size, e.g. "31 MB".
#   $limit (String) - the largest document the editor takes, e.g. "20 MB".
hui-editor-paste-refused-body = The pasted text is { $size }, which would take this document past { $limit }. Paste a smaller part.
# Toast title: the pasted text had NUL characters, which the editor shows as chips.
hui-editor-nul-title = The pasted text contains NUL characters
# Variables:
#   $n (Number) - how many NUL characters were pasted, e.g. 2.
hui-editor-nul-body = { $n ->
        [one] { $n } NUL character is marked in the text so you can find it. Most servers reject it.
       *[other] { $n } NUL characters are marked in the text so you can find them. Most servers reject them.
    }
# Toast title: the clipboard could not be read for a paste.
hui-editor-clipboard-title = Couldn't read the clipboard
hui-editor-clipboard-body = Another app may be holding it, or the system didn't allow access.
# Toast button: read the clipboard again and paste.
hui-editor-try-again = Try again

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
# The notice's second line in that case (m3a-f4: it says when reduced mode ends).
# Variables:
#   $size (String) - the line length reduced mode ends below (an hui-editor-t1-line-size message), e.g. "51 KB".
hui-editor-t1-long-body = Reduced mode keeps typing fast. It ends when every line is shorter than { $size }.
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

## m3a-f4-ime
# A line length in whole kilobytes (the reduced-mode notice).
# Variables:
#   $n (Number) - kilobytes, e.g. 51.
hui-editor-t1-line-size = { $n } KB
# The reduced-mode notice's second line when both the size and a very long line put the editor in reduced mode.
# Variables:
#   $size (String) - the size reduced mode ends below (an hui-editor-t1-size message), e.g. "3.2 MB".
#   $line (String) - the line length it ends below (an hui-editor-t1-line-size message), e.g. "51 KB".
hui-editor-t1-both-body = Reduced mode keeps typing fast. It ends when the document is smaller than { $size } and every line is shorter than { $line }.

## m3b-t40-grid-a11y

# What a screen reader reads for a NULL cell in a result grid.
hui-grid-a11y-null = null
# What a screen reader reads for a cell whose value was cut at the value-size limit.
# Variables:
#   $value (String) - the part of the value that was read, e.g. "Lorem ipsum".
#   $size (String) - the whole value's size, with its unit, e.g. "5 MB".
hui-grid-a11y-truncated = { $value }, truncated, { $size }
# Announced when the grid's focus has settled on a cell in another row (the cell itself is read as the focus).
# Variables:
#   $row (Number) - the row's position as shown, from 1, e.g. 1234.
#   $total (Number) - the rows shown, e.g. 10000.
hui-grid-a11y-row-of = Row { $row } of { $total }
# As hui-grid-a11y-row-of, while the result may have more rows than were fetched.
# Variables:
#   $row (Number) - the row's position as shown, from 1, e.g. 1234.
#   $total (Number) - the rows fetched (or kept by a filter) so far, e.g. 10000.
hui-grid-a11y-row-of-more = Row { $row } of { $total } or more
# Announced when a grid selection holding more than one cell has settled.
# Variables:
#   $cells (Number) - the selected cells, e.g. 160.
hui-grid-selection-summary = { $cells ->
        [one] { $cells } cell selected
       *[other] { $cells } cells selected
    }
# The grid's accessible description while the result may have more rows than were fetched.
hui-grid-a11y-more-rows = More rows are available
