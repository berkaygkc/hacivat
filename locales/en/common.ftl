### Terms and messages shared by every domain (prefix `cm-`).
### Append-only: each task adds a block headed `## <task slug>`.

## interface-commit

# The brand. Never translated and never Turkish-cased.
-brand-hacivat = Hacivat
-brand-hacivat-caps = HACIVAT

# The title-bar brand word (a term has no message ID, so the title bar renders this message).
cm-brand-hacivat-caps = { -brand-hacivat-caps }

## l10n-core

# Shown in place of a text whose message is missing from every language (a defect). Release
# builds only; debug builds show the ID in ⟦ ⟧ instead.
# Variables:
#   $code (String) - the missing message ID, e.g. "hv-tab-console".
cm-error-unknown = Text not found: { $code }
