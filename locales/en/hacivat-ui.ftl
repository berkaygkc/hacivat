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

## m2a-t18-explorer

# Command titles of the connection commands (palette rows and menu items).
hv-cmd-connection-new = New connection
hv-cmd-connection-paste-url = New connection from a copied URL
hv-cmd-connection-toggle-connect = Connect or disconnect
hv-cmd-connection-edit = Edit connection
hv-cmd-connection-duplicate = Duplicate connection
hv-cmd-connection-copy-url = Copy connection as URL
hv-cmd-connection-delete = Delete connection
hv-cmd-connection-context-menu = Show the connection menu
hv-cmd-connection-import-pg-service = Import pg_service.conf
hv-cmd-connection-retry = Retry the connection
hv-cmd-connection-explain-unencrypted = Explain the unencrypted connection
hv-cmd-workspace-focus-notification = Go to the newest notification
# The explorer with no saved connection: the sentence under the title and its three buttons.
hv-explorer-empty-body = Add a PostgreSQL, MySQL, MariaDB or SQLite database. If you paste a connection URL, { -brand-hacivat } fills the form for you.
hv-explorer-new = New connection
hv-explorer-paste = Paste URL
hv-explorer-import = Import pg_service.conf…
# Explorer group headers, in small caps: pre-cased here, never cased in code.
hv-explorer-group-production = PRODUCTION
hv-explorer-group-staging = STAGING
hv-explorer-group-test = TEST
hv-explorer-group-dev = DEV
hv-explorer-group-local = LOCAL
# The group of connections without an environment (hand-edited files only).
hv-explorer-group-unset = NO ENVIRONMENT
# An environment tag as a word: the connection sheet and the capsule's tag of a non-production connection.
# Variables:
#   $env (Enum(production, staging, test, dev, local, unset)) - the connection's environment tag.
hv-explorer-env = { $env ->
        [production] production
        [staging] staging
        [test] test
        [dev] dev
        [local] local
       *[unset] no environment
    }
# Toast when a connection change could not be written to connections.toml.
hv-explorer-write-failed = The change couldn't be saved to your connections file.
# Toast when a saved password could not be removed from or copied in the system keyring.
hv-explorer-secret-failed = A saved password couldn't be updated in the system keyring.
# Toast after Copy as URL from the explorer.
hv-explorer-copied = Copied as URL
# The line under the copy toast when the connection signs in with a password.
hv-explorer-copied-nopw = The password isn't included.
# The connection menu (explorer row, Shift-F10).
hv-conn-menu-connect = Connect
hv-conn-menu-disconnect = Disconnect
hv-conn-menu-edit = Edit…
hv-conn-menu-dup = Duplicate
hv-conn-menu-copy-url = Copy as URL
hv-conn-menu-delete = Delete…
# The delete confirmation.
# Variables:
#   $name (String) - the connection's name, e.g. "orders-prod".
hv-conn-del-title = Delete { $name }?
hv-conn-del-body = Its saved password is removed from the Keychain too. Scripts and query history stay. This can't be undone.
hv-conn-del-btn = Delete
hv-conn-del-cancel = Cancel
# Toast after a connection was deleted.
# Variables:
#   $name (String) - the deleted connection's name, e.g. "orders-prod".
hv-conn-toast-deleted = Deleted { $name }
# Toast after Escape cancelled a connect in progress.
# Variables:
#   $name (String) - the connection's name, e.g. "orders-prod".
hv-conn-toast-cancelled = Connecting to { $name } was cancelled.
# The connection sheet under the explorer.
hv-conn-sheet-none = Select a connection to see its details.
hv-conn-kv-env = Environment
hv-conn-kv-engine = Engine
hv-conn-kv-host = Host
hv-conn-kv-file = File
hv-conn-kv-user = User
hv-conn-kv-access = Access
hv-conn-kv-path = Path
hv-conn-kv-enc = Encryption
hv-conn-kv-secret = Password
# A read-only connection: M2a sets the session read-only but cannot enforce it (R10).
hv-conn-access-ro-guarded = read-only (guarded)
# The Encryption row of a live connection.
# Variables:
#   $version (String) - the TLS version the server negotiated, e.g. "TLSv1.3".
hv-conn-enc-verified = { $version } · verified
# Variables:
#   $version (String) - the TLS version the server negotiated, e.g. "TLSv1.3".
hv-conn-enc-unverified = { $version } · not verified
hv-conn-enc-unencrypted = unencrypted
hv-conn-enc-none-file = not applicable (file)
# Where the connection's password comes from (the sheet's Password row).
hv-conn-secret-forever = Keychain · forever
hv-conn-secret-never = asks every time
hv-conn-secret-session = for this session
hv-conn-secret-restart = until restart
hv-conn-secret-pgpass = from ~/.pgpass
hv-conn-secret-none = none
# The popover of the capsule's "unencrypted" segment.
hv-conn-unenc-title = Connected without encryption
# Variables:
#   $host (String) - the server's host name or address, e.g. "10.0.4.12".
hv-conn-unenc-body = { $host } doesn't offer TLS. { -brand-hacivat } allowed plaintext because the address is on a private network. This badge stays for as long as the connection is unencrypted.
hv-conn-unenc-fix = To remove it, turn on TLS on the server or connect through SSH.
# The capsule while connecting, after a failure, and its Retry segment.
hv-capsule-connecting = Connecting
hv-capsule-failed = Couldn't connect
hv-capsule-retry = Retry
# The capsule's running clock while connecting.
# Variables:
#   $seconds (Decimal) - seconds since the connect started, e.g. 1.25.
hv-capsule-clock = { NUMBER($seconds, minimumFractionDigits: 2) } s
# The step the capsule shows while connecting.
# Variables:
#   $step (Enum(pgpass, resolve, connect, tls, sign-in, check-server, open-file)) - the running step.
hv-capsule-step = { $step ->
        [pgpass] pgpass
        [resolve] Resolve
        [connect] Connect
        [tls] TLS
        [sign-in] Sign in
        [check-server] Check server
       *[open-file] Open file
    }
# The capsule's mark of a server below the supported versions (PO Q3).
hv-capsule-unsupported = unsupported
# The capsule's access cell for a read-write connection.
hv-capsule-access-read-write = read-write
# A connection's latency (the capsule, the status bar and an explorer row).
# Variables:
#   $ms (Number) - the latency in whole milliseconds, e.g. 12.
hv-capsule-latency = { $ms } ms
# The status bar's environment segment: the current connection's environment and access.
# Variables:
#   $env (Enum(production, staging, test, dev, local, unset)) - the connection's environment tag.
#   $access (Enum(read-only, read-write)) - whether the connection is read-only.
hv-status-env = { $env ->
        [production] production
        [staging] staging
        [test] test
        [dev] dev
        [local] local
       *[unset] no environment
    } · { $access ->
        [read-only] read-only
       *[read-write] read-write
    }
# The editor area's home while no query is open: the title, the hint and the entry points.
hv-home-title = No query open
hv-home-hint = Pick a connection in the Explorer, or add one.
hv-home-new = New connection
hv-home-paste = New connection from a copied URL
hv-home-connect = Connect to the selected connection
# The pg_service.conf import dialog (R24).
hv-conn-import-title = Import from pg_service.conf
hv-conn-import-body = Choose the services to import. Every remote service needs an environment.
# An environment choice of an imported service.
# Variables:
#   $env (Enum(production, staging, test, dev, local, unset)) - the environment tag, or none chosen yet.
hv-conn-import-env = { $env ->
        [production] Production
        [staging] Staging
        [test] Test
        [dev] Dev
        [local] Local
       *[unset] Choose…
    }
hv-conn-import-confirm = Import
hv-conn-import-cancel = Cancel
# A service row of the import dialog: its name and the server it points to (connection data).
# Variables:
#   $service (String) - the service name, e.g. "orders-prod".
#   $target (String) - its host, socket or database, e.g. "db-prod".
hv-conn-import-row = { $service } · { $target }
# Toast when no pg_service.conf, or no service in it, was found.
hv-conn-import-empty = No services were found in pg_service.conf.
# Toast when pg_service.conf exists but could not be read.
hv-conn-import-unreadable = pg_service.conf couldn't be read.
# A note in the import dialog: a password was left out because the file is readable by others.
# Variables:
#   $service (String) - the service name, e.g. "orders-prod".
hv-conn-import-password-ignored = The password of { $service } was ignored because other users can read the file.
# Toast after an import.
# Variables:
#   $count (Number) - how many connections were imported, e.g. 2.
hv-conn-import-done = { $count ->
        [one] Imported one connection
       *[other] Imported { $count } connections
    }
