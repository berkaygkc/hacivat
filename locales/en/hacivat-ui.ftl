### The Hacivat desktop shell (prefix `hv-`).
### Append-only: each task adds a block headed `## <task slug>`.

## task-13-shell

# The title-bar field that opens the command palette.
hv-titlebar-palette = Search or run a command
# The title-bar connection capsule before any connection is open.
hv-capsule-no-connection = No connection
# The capsule's production block, in small caps: pre-cased here, never cased in code.
hv-capsule-env-production = PRODUCTION
# The capsule's access cell for a read-only connection.
hv-capsule-access-read-only = read-only
# The capsule's path cell for a direct (peer-to-peer) connection.
hv-capsule-path-direct = Direct
# Panel titles, in small caps: pre-cased here, never cased in code.
hv-panel-explorer = EXPLORER
hv-panel-connection = CONNECTION
# Placeholder of the explorer's filter field.
hv-explorer-filter = Filter connections
# The explorer before any connection exists.
hv-explorer-empty = No connections yet
# The title of a new SQL console tab.
# Variables:
#   $n (Number) - the console's sequence number in this workspace, e.g. 1.
hv-tab-console-untitled = Console { $n }
# The editor area of a console with no connection.
hv-editor-empty = Connect to a database to start writing SQL
# The results pane before a query has run.
hv-results-empty = Run a query to see results here
# The status bar's connection segment when nothing is connected.
hv-status-no-connection = No connection
# The status bar while a multi-key shortcut waits for its next key.
# Variables:
#   $chord (String) - the keys pressed so far, as the keyboard shows them, e.g. "⌃J".
hv-status-pending-chord = Waiting for the next key: { $chord }
# Tooltips and accessible names of the title-bar theme switch.
hv-theme-dark = Dark
hv-theme-light = Light
# Error toast at startup when settings.toml could not be parsed and was set aside.
# Variables:
#   $file (String) - the name the unreadable file was saved under, e.g. "settings.toml.corrupt-1731400000".
hv-settings-file-quarantined = Your settings file could not be read. It was saved as { $file } and defaults are in use.
# Warning toast at startup when some settings had invalid values.
# Variables:
#   $keys (List) - the settings keys that fell back to their defaults, e.g. "ui.theme".
hv-settings-invalid-value = Some settings could not be read and use their defaults: { $keys }
# Warning toast at startup when settings.toml was written by a newer version.
hv-settings-newer-version = Your settings file is from a newer Hacivat. Settings this version does not know are kept.

## Task 16

# Command titles (palette rows and menu items): `hv-cmd-<action id with . and _ as ->`.
hv-cmd-workspace-command-palette = Command palette
hv-cmd-workspace-open-settings = Open settings
hv-cmd-workspace-new-console-tab = New console tab
hv-cmd-workspace-close-tab = Close tab
hv-cmd-workspace-next-tab = Next tab
hv-cmd-workspace-prev-tab = Previous tab
hv-cmd-workspace-split-right = Split right
hv-cmd-workspace-split-down = Split down
hv-cmd-workspace-focus-next-pane = Focus next pane
hv-cmd-workspace-toggle-explorer = Show or hide the explorer
hv-cmd-workspace-toggle-theme = Switch between dark and light theme
hv-cmd-workspace-next-language = Switch to the next language
hv-cmd-workspace-reload-translations = Reload translations
hv-cmd-app-about = About { -brand-hacivat }
hv-cmd-app-show-menu = Show app menu
hv-cmd-app-restart = Restart { -brand-hacivat }
hv-cmd-app-quit = Quit { -brand-hacivat }
hv-cmd-explorer-focus-filter = Focus connection filter
# Application menu titles (the macOS menu bar and the Windows/Linux in-window menu).
hv-menu-app = { -brand-hacivat }
hv-menu-view = View
hv-menu-window = Window
hv-menu-help = Help
# The About dialog's title.
hv-about-title = About { -brand-hacivat }
# The About dialog's version line.
# Variables:
#   $version (String) - the app version, e.g. "0.1.0".
hv-about-version = Version { $version }
# Info toast after the translation overlay was read again.
hv-toast-translations-reloaded = Translations reloaded

## Task 17

# The settings tab's title, and the title of its tab.
hv-settings-title = Settings
# Left navigation of the settings tab.
hv-settings-nav-general = General
hv-settings-nav-appearance = Appearance
hv-settings-nav-keyboard = Keyboard
# Placeholder of the settings search field.
hv-settings-search = Search settings
# Shown when the search matches no setting.
hv-settings-no-results = No settings match
# Setting titles.
hv-settings-language = Language
hv-settings-regional-format = Regional format
hv-settings-theme = Theme
hv-settings-text-rendering = Text rendering
hv-settings-keymap = Keyboard shortcuts
# The choice that follows the operating system (language, regional format, theme).
hv-settings-system = System
# The two text rendering choices; "macOS standard" is the system's own font smoothing.
hv-settings-text-rendering-crisp = Crisp
hv-settings-text-rendering-standard = macOS standard
# Shown under a setting that only applies once the app restarts.
hv-settings-restart-note = Applies after restart
# The button next to the restart note, and the dialog it opens.
hv-settings-restart-now = Restart now
hv-settings-restart-title = Restart Hacivat?
hv-settings-restart-body = Hacivat closes and opens again to apply the new text rendering. Your settings and layout are saved first.
hv-settings-restart-later = Later
# A sample of the regional format: a number and a date with time, as the chosen format shows them.
# Variables:
#   $number (Decimal) - a sample number, 1234.5.
#   $when (DateTime) - a sample wall-clock time, 2026-11-11 14:05.
hv-settings-regional-preview = { NUMBER($number) } · { DATETIME($when, dateStyle: "short", timeStyle: "short") }
# Warning toast after choosing a keymap preset whose shortcuts type characters on this keyboard layout.
# Variables:
#   $chords (List) - the shortcuts and the character each types, e.g. "Ctrl+Alt+S → ś".
hv-settings-keymap-collisions = On this keyboard layout these shortcuts type a character instead: { $chords }
# Section headings of the settings page, in small caps: pre-cased here, never cased in code.
hv-settings-section-general = GENERAL
hv-settings-section-appearance = APPEARANCE
hv-settings-section-keyboard = KEYBOARD
# The two keymap presets: the brand, and the product whose shortcuts the preset follows.
hv-settings-keymap-hacivat = { -brand-hacivat }
hv-settings-keymap-datagrip = DataGrip
# One shortcut in the collision warning: the shortcut and the character it types instead.
# Variables:
#   $chord (String) - the shortcut as the keyboard shows it, e.g. "Ctrl+Alt+S".
#   $char (String) - the character it types on this layout, e.g. "ś".
hv-settings-keymap-collision = { $chord } → { $char }

## m2a-t01-menu

# The File menu title (the macOS menu bar and the Windows/Linux in-window menu).
hv-menu-file = File

## m2a-t12-profiles

# Toast when a password could not be saved to the system keyring; it is kept in memory until quit.
hv-conn-keyring-unavailable = Passwords can't be saved to the system keyring right now. { -brand-hacivat } keeps them until it quits.
# Toast at startup when connections.toml was written by a newer Hacivat: connections can't be changed.
hv-conn-store-read-only = Your connections file is from a newer { -brand-hacivat }. Connections can't be added or changed in this version.
# Error toast at startup when connections.toml could not be parsed and was set aside.
# Variables:
#   $file (String) - the name the unreadable file was saved under, e.g. "connections.toml.corrupt-1731400000".
hv-conn-file-quarantined = Your connections file could not be read. It was saved as { $file } and no connections are loaded.
# Toast at startup when some entries of connections.toml could not be read (invalid, duplicate or for an unknown engine).
# Variables:
#   $count (Number) - how many entries were skipped, e.g. 2.
hv-conn-file-entries-skipped = { $count ->
        [one] One connection in your connections file couldn't be read and was skipped.
       *[other] { $count } connections in your connections file couldn't be read and were skipped.
    }
# Toast at startup when connections.toml contained a password; the value is never shown.
hv-conn-file-password-ignored = A password in your connections file was ignored. { -brand-hacivat } keeps passwords in the system keyring, never in that file.
# Toast after duplicating a connection.
# Variables:
#   $name (String) - the new connection's name, e.g. "orders-prod-2".
hv-conn-toast-dup = Duplicated as { $name }
# Error toast at startup when connections.toml exists but could not be read (permissions, disk error) or could not be set aside; the file is left untouched and connections can't be changed.
hv-conn-file-unreadable = { -brand-hacivat } can't read your connections file. It was left as it is, and connections can't be changed until it can be read.
# Toast at startup when some values in connections.toml were not understood and were ignored (an unknown environment, an organization tag on a connection the organization does not manage).
# Variables:
#   $count (Number) - how many values were ignored, e.g. 2.
hv-conn-file-values-ignored = { $count ->
        [one] One setting in your connections file couldn't be read and was ignored.
       *[other] { $count } settings in your connections file couldn't be read and were ignored.
    }
