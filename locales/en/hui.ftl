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
