### Shared texts.

# A fixed label.
cm-ok = OK

# Rows read so far.
# Variables:
#   $count (Decimal) - a fraction of rows.
cm-rows =
    { $count ->
        [one] One row
       *[other] { $count } rows
    }

# Rich text.
cm-hint = Press <code>F5</code> to refresh.

# Saved.
# Variables:
#   $name (String) - connection name.
cm-saved = Connection { $name } saved.
    .title = Saved
