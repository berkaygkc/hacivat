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
#   $version (String) - the TLS version the server negotiated, e.g. "TLS 1.3".
hv-conn-enc-verified = { $version } · verified
# Variables:
#   $version (String) - the TLS version the server negotiated, e.g. "TLS 1.3".
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
#   $env (String) - the environment word (an hv-conn-env-* message), e.g. "production".
#   $access (String) - the access word (an hv-capsule-access-* message), e.g. "read-write".
hv-status-env = { $env } · { $access }
# The editor area's home while no query is open: the title, the hint and the entry points.
hv-home-title = No query open
hv-home-hint = Pick a connection in the Explorer, or add one.
hv-home-new = New connection
hv-home-paste = New connection from a copied URL
hv-home-connect = Connect to the selected connection
# The pg_service.conf import dialog (R24).
hv-conn-import-title = Import from pg_service.conf
hv-conn-import-body = Choose the services to import. Every remote service needs an environment.
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
## m2a-t20-errors

# The connection dialog's test strip while a test runs (NOTES surface 1).
hv-conn-test-running = Testing
# The running clock of a test, in seconds with two decimals.
# Variables:
#   $seconds (Decimal) - seconds since the test started, e.g. 0.06.
hv-conn-test-clock = { NUMBER($seconds, minimumFractionDigits: 2) } s
# The test strip's heading after a successful test.
# Variables:
#   $ms (Number) - the whole test in whole milliseconds, e.g. 54.
hv-conn-test-ok = Connected in { $ms } ms
# The line under a successful test's heading.
hv-conn-test-ok-sub = Nothing was saved. The test opened its own connection and closed it.
# The button that copies the test report (always in English).
hv-conn-test-copy = Copy report

# Diagnostics of a successful test: row labels and values.
hv-conn-diag-server = Server
hv-conn-diag-latency = Latency
hv-conn-diag-tls = Encryption
hv-conn-diag-cipher = Cipher
hv-conn-diag-auth = Signed in as
hv-conn-diag-warn = Warnings
hv-conn-diag-none = None
# The server row's value.
# Variables:
#   $product (String) - the server's product name, e.g. "PostgreSQL".
#   $version (String) - the server's version, e.g. "16.4".
hv-conn-diag-server-v = { $product } { $version }
# The latency row's value.
# Variables:
#   $ms (Number) - the median round trip in whole milliseconds, e.g. 12.
hv-conn-diag-latency-v = { $ms } ms · median of 3 pings
# The sign-in row's value: the user and the sign-in method.
# Variables:
#   $user (String) - the database user, e.g. "deploy".
#   $method (Enum(password, none, pgpass, client_cert)) - how the user signed in.
hv-conn-diag-auth-v = { $user } · { $method ->
        [password] Password
        [pgpass] pgpass
        [client_cert] Client certificate
       *[none] no password
    }
# The sign-in row's value when the user is unknown: the sign-in method alone.
# Variables:
#   $method (Enum(password, none, pgpass, client_cert)) - how the user signed in.
hv-conn-diag-auth-method = { $method ->
        [password] Password
        [pgpass] pgpass
        [client_cert] Client certificate
       *[none] No password
    }
# The encryption row's value for an encrypted connection.
# Variables:
#   $version (String) - the TLS version number, e.g. "1.3".
#   $state (Enum(full, ca_only, no)) - how far the server's certificate was verified.
hv-conn-diag-tls-v = TLS { $version } · { $state ->
        [full] verified
        [ca_only] CA verified
       *[no] not verified
    }
# The encryption row's value when the TLS version is unknown.
# Variables:
#   $state (Enum(full, ca_only, no)) - how far the server's certificate was verified.
hv-conn-diag-tls-nov = TLS · { $state ->
        [full] verified
        [ca_only] CA verified
       *[no] not verified
    }
# The encryption row's value for an unencrypted network connection.
hv-conn-diag-tls-none = unencrypted
# The encryption and sign-in rows of a database file (SQLite).
hv-conn-diag-file = not applicable (file)
# One line of a copied report (English): a row's label and its value.
# Variables:
#   $label (String) - the row's label, e.g. "Latency".
#   $value (String) - the row's value, e.g. "12 ms · median of 3 pings".
hv-conn-diag-row = { $label }: { $value }

# The connect steps of the step trail (HV-CONN-01).
hv-conn-step-resolve = Resolve
hv-conn-step-tcp = Connect
hv-conn-step-tls = TLS
hv-conn-step-auth = Sign in
hv-conn-step-check = Check server
hv-conn-step-pgpass = pgpass
hv-conn-step-open = Open file
# A finished step's duration.
# Variables:
#   $ms (Number) - the step's time in whole milliseconds, e.g. 14.
hv-conn-step-ms = { $ms } ms
# The step trail's accessible name, and the state each step announces.
hv-conn-step-list = Connection steps
hv-conn-step-state-pending = not started
hv-conn-step-state-running = in progress
hv-conn-step-state-done = done
hv-conn-step-state-failed = failed
hv-conn-step-state-skipped = skipped

# Warnings a successful test still reports.
hv-conn-warn-no-tls = The connection isn't encrypted.
hv-conn-warn-tls-not-verified = The connection is encrypted, but the server's certificate wasn't verified.
# Variables:
#   $version (String) - the server's version, e.g. "18.1".
hv-conn-warn-not-tested = { -brand-hacivat } hasn't been tested with version { $version } yet.
# Variables:
#   $version (String) - the server's version, e.g. "9.6.24".
hv-conn-warn-unsupported = Version { $version } is older than { -brand-hacivat } supports. You chose to connect anyway.
hv-conn-warn-replica = The server is a read-only replica.
hv-conn-warn-server-read-only = The server is in read-only mode.
hv-conn-warn-super-read-only = The server is in super-read-only mode: nobody can write, not even administrators.
# Variables:
#   $seconds (Number) - the server's idle timeout in seconds, e.g. 60.
hv-conn-warn-low-wait-timeout = The server closes idle connections after { $seconds } s.
hv-conn-warn-local-infile-on = The server allows LOAD DATA LOCAL INFILE, so it can ask for files from this Mac.
hv-conn-warn-sqlite-file-not-writable = The database file can't be written, so it opens read-only.
# Variables:
#   $mode (String) - the SQLite journal mode, e.g. "delete".
hv-conn-warn-sqlite-journal = The database uses the { $mode } journal mode.

# The connect-error dialog's title (NOTES surface 3).
# Variables:
#   $name (String) - the connection's name, e.g. "orders-prod".
hv-conn-err-title = Couldn't connect to { $name }
# Under the re-entered password field: which of the 3 attempts this is.
# Variables:
#   $n (Number) - the attempt, 1 to 3.
hv-conn-err-attempt = Attempt { $n } of 3
# Placeholder of the re-entered password field.
# Variables:
#   $user (String) - the database user, e.g. "deploy".
hv-conn-err-new-pw = Password for { $user }
# The technical details disclosure and its rows.
hv-conn-err-tech = Technical details
hv-conn-err-stage = Failed at
hv-conn-err-class = Class
hv-conn-err-code = Code
hv-conn-err-server = Server message
hv-conn-err-elapsed-label = Elapsed
# The elapsed row's value, in seconds with three decimals.
# Variables:
#   $seconds (Decimal) - the connect's time in seconds, e.g. 0.214.
hv-conn-err-elapsed = { NUMBER($seconds, minimumFractionDigits: 3) } s
# The button that copies the technical details (always in English).
hv-conn-err-copy = Copy details (English)

# Quick fixes (HV-CONN-03).
hv-conn-fix-reenter = Enter password again
hv-conn-fix-edit = Edit connection
hv-conn-fix-retry = Try again
# Variables:
#   $db (String) - a database the user can open, e.g. "orders".
hv-conn-fix-use-db = Use { $db }
hv-conn-fix-versions = Supported versions
hv-conn-fix-anyway = Connect anyway, unsupported
hv-conn-fix-connect = Connect
hv-conn-fix-close = Close
# Shown when the connections file is read-only: fixes that change the connection are off.
hv-conn-fix-read-only = Your connections file can't be changed, so fixes that edit this connection are off.
# Shown when a fix's change to the connection could not be saved.
hv-conn-fix-save-failed = The change couldn't be saved to your connections file.

# The production-tag prompt (NOTES surface 4, D-004).
# Variables:
#   $name (String) - the connection's name, e.g. "orders-prod-us".
hv-conn-prod-title = How careful should { -brand-hacivat } be with { $name }?
# Variables:
#   $host (String) - the connection's host, e.g. "db-prod-us.acme.io".
hv-conn-prod-body = { $host } isn't on this Mac. Pick the kind of database it is. You can change this later in the connection settings.
# Why Production is preselected: a word of the host name.
# Variables:
#   $token (String) - the word that matched, e.g. "prod".
hv-conn-prod-suggest = Suggested because the host name contains “{ $token }”.
# Why Production is preselected: a word of the connection's name.
# Variables:
#   $token (String) - the word that matched, e.g. "prd".
hv-conn-prod-suggest-name = Suggested because the name contains “{ $token }”.
# The environment choices and their descriptions.
hv-conn-prod-opt-production = Production
hv-conn-prod-opt-production-d = Live data that people or services rely on
hv-conn-prod-opt-staging = Staging
hv-conn-prod-opt-staging-d = A copy that mirrors production
hv-conn-prod-opt-test = Test
hv-conn-prod-opt-test-d = Shared test or CI data
hv-conn-prod-opt-dev = Dev
hv-conn-prod-opt-dev-d = A development database you can break
# The choice list's accessible name.
hv-conn-prod-choices = Environment
# What changes with Production, as a list.
hv-conn-prod-changes = With production, { -brand-hacivat }:
# Variables:
#   $word (String) - the capsule's word, hv-conn-prod-c1-word, drawn in red small caps.
hv-conn-prod-c1 = shows the { $word } capsule in the title bar while you work on it,
# The capsule's word in the list, in small caps: pre-cased here, never cased in code.
hv-conn-prod-c1-word = PRODUCTION
hv-conn-prod-c2 = asks you to confirm statements that change data or schema,
hv-conn-prod-c3 = uses manual commit: nothing is written until you commit.
# Shown instead of the list for any other environment.
hv-conn-prod-other = No extra confirmations. Statements commit as they run.
# The primary button: it names the result.
# Variables:
#   $env (Enum(production, staging, test, dev)) - the chosen tag.
hv-conn-prod-save = { $env ->
        [production] Save as production
        [staging] Save as staging
        [test] Save as test
       *[dev] Save as dev
    }
# The primary button while nothing is chosen (disabled).
hv-conn-prod-pick = Pick an environment
hv-conn-prod-cancel = Cancel

# The password prompt a connect asks through (R14).
# Variables:
#   $name (String) - the connection's name, e.g. "orders-prod".
hv-conn-pw-title = Sign in to { $name }
# Variables:
#   $user (String) - the database user, e.g. "deploy".
hv-conn-pw-body = Enter the password for { $user }. It's kept only if the connection works.
hv-conn-pw-cancel = Cancel
## m2a-t17-dialog

# Palette titles of the connection dialog's commands (they run only while the dialog is open).
hv-cmd-connection-dialog-test = Test connection
hv-cmd-connection-dialog-save = Save connection
hv-cmd-connection-dialog-save-and-connect = Save and connect
hv-cmd-connection-dialog-copy-url = Copy connection as URL
# Connection dialog title for a new connection.
hv-conn-dlg-title-new = New connection
# Connection dialog title while editing a saved connection.
# Variables:
#   $name (String) - the connection's name, e.g. "orders-prod".
hv-conn-dlg-title-edit = Edit { $name }
# Line under the dialog title for a new connection.
hv-conn-dlg-sub-new = Not saved yet
# Line under the dialog title for a saved connection.
hv-conn-dlg-sub-edit = Saved · changes apply on the next connect
# Form row labels and placeholders of the connection dialog.
hv-conn-dlg-url = URL
hv-conn-dlg-url-ph = Paste a connection URL to fill the form
hv-conn-dlg-copy-url = Copy as URL
hv-conn-dlg-engine = Engine
hv-conn-dlg-name = Name
hv-conn-dlg-env = Environment
# Hint under the environment choice when the host is not this machine.
hv-conn-dlg-env-ask = Asked when you save, because this host isn't on this Mac.
# Hint under the environment choice when the host is this machine.
hv-conn-dlg-env-local = Set to local: the server runs on this Mac.
hv-conn-dlg-ro = Read-only
# Dimmer text after the Read-only checkbox, starting with its separator dot.
hv-conn-dlg-ro-hint = · Block statements that change data or schema
# Section headings of the connection form, in small caps: pre-cased here, never cased in code.
hv-conn-section-server = SERVER
hv-conn-section-auth = AUTHENTICATION
# Titles of the folding sections of the connection form.
hv-conn-section-tls = Encryption (TLS)
hv-conn-section-advanced = Advanced
hv-conn-dlg-host = Host
hv-conn-dlg-port = Port
hv-conn-dlg-db = Database
hv-conn-dlg-user = User
hv-conn-dlg-file = File
# Button that opens a file picker.
hv-conn-dlg-choose = Choose…
hv-conn-dlg-sqlite-ro = Open the file read-only
hv-conn-dlg-sqlite-create = Create the file if it doesn't exist
# Row label of the sign-in method choice.
hv-conn-dlg-auth = Authentication
# The sign-in methods.
hv-conn-auth-password = Password
hv-conn-auth-pgpass = pgpass
hv-conn-auth-cert = Client certificate
hv-conn-auth-none = None
hv-conn-dlg-password = Password
# Label of the choice of how long a password is kept.
hv-conn-dlg-remember = Remember
# Placeholder of the password field when a password is already stored.
hv-conn-dlg-pw-saved = saved in the Keychain
# Next to a password that came from a pasted URL.
hv-conn-dlg-pw-from-url = from the pasted URL
# How long a password is kept, each with a hint.
hv-conn-ret-never = Never
hv-conn-ret-never-h = ask on every connect
hv-conn-ret-restart = Until restart
hv-conn-ret-restart-h = kept in memory
hv-conn-ret-session = For this session
hv-conn-ret-session-h = forgotten on disconnect
hv-conn-ret-forever = Forever (Keychain)
hv-conn-ret-forever-h = stored in the macOS Keychain
# Hint of the disabled "Forever" choice when the system keyring can't be used.
hv-conn-ret-forever-off-h = the system keyring isn't available
# The pgpass line that matches the connection.
# Variables:
#   $n (Number) - the 1-based line number, e.g. 3.
hv-conn-pgpass-match = Matches line { $n } of ~/.pgpass
# Shown when no pgpass line matches the connection.
hv-conn-pgpass-none = No line of ~/.pgpass matches this connection yet.
hv-conn-pgpass-note = Read on every connect. { -brand-hacivat } stores no password for this connection.
hv-conn-dlg-cert = Certificate
hv-conn-dlg-key = Private key
hv-conn-auth-none-note = Connects without a password. Use only for servers that trust this machine.
# Summary of the folded encryption section in Auto mode.
hv-conn-tls-sum-auto = Auto: verify when the server offers TLS
hv-conn-tls-mode = Mode
# The encryption modes, each with a hint.
hv-conn-tls-auto = Auto
hv-conn-tls-auto-h = verify if offered; plaintext only on private networks
# Hint of Auto for MySQL and MariaDB, which send credentials in plain text only to this machine.
hv-conn-tls-auto-loopback-h = verify if offered; plaintext only to this Mac
hv-conn-tls-require = Require
hv-conn-tls-require-h = encrypt, don't verify
hv-conn-tls-verify-ca = Verify CA
hv-conn-tls-verify-ca-h = check the chain
hv-conn-tls-verify-full = Verify full
hv-conn-tls-verify-full-h = check the chain and host name
hv-conn-tls-disable = Disable
hv-conn-tls-disable-h = never encrypt
# Warning when encryption is turned off for MySQL or MariaDB.
hv-conn-tls-disable-warn = With encryption off, the password crosses the network in plain text, where anyone on the network path can read it.
hv-conn-tls-ca = CA certificate
hv-conn-tls-ca-ph = System trust store
# Summary of the folded Advanced section.
# Variables:
#   $timeout (Number) - the connect timeout in seconds, e.g. 15.
#   $keepalive (Enum(on, off)) - whether keep-alive is on.
#   $seconds (Number) - the keep-alive interval in seconds, e.g. 60.
hv-conn-adv-sum = Timeout { $timeout } s · { $keepalive ->
        [on] keep-alive every { $seconds } s
       *[off] keep-alive off
    }
hv-conn-adv-timeout = Connect timeout
# Unit after the connect timeout field.
hv-conn-adv-timeout-unit = seconds
hv-conn-adv-keepalive = Keep-alive every
hv-conn-adv-off = Off
# A keep-alive interval choice.
# Variables:
#   $seconds (Number) - the interval in seconds, e.g. 60.
hv-conn-adv-every = { $seconds } s
hv-conn-adv-app = Application name
hv-conn-adv-extra = Extra properties
hv-conn-adv-extra-add = Add property
# Placeholders of an extra property's name and value fields.
hv-conn-adv-extra-key = key
hv-conn-adv-extra-value = value
hv-conn-adv-extra-remove = Remove property
hv-conn-adv-budget = Connections
# The connection budget, stated as a fact.
# Variables:
#   $max (Number) - how many connections may be open at once, e.g. 4.
hv-conn-adv-budget-v = Up to { $max } open at once; more consoles wait their turn
# Footer buttons of the connection dialog.
hv-conn-dlg-test = Test connection
hv-conn-dlg-test-stop = Stop
hv-conn-dlg-cancel = Cancel
hv-conn-dlg-save = Save
hv-conn-dlg-save-connect = Save and connect
# Notice in the dialog when connections.toml can't be written.
hv-conn-dlg-store-read-only = Connections can't be saved now, because the connections file can't be written.
# Notice in the dialog when saving failed.
hv-conn-dlg-save-failed = The connection couldn't be saved. Try again.
# Notice in the dialog when the connection being edited was deleted meanwhile.
hv-conn-dlg-save-gone = This connection was deleted, so the changes weren't saved.
# Notice in the dialog when the connection was saved but its password was not.
hv-conn-dlg-save-secret = The connection was saved, but its password couldn't be stored.
# Environment names in the connection form, lowercase as the marks' labels.
hv-conn-env-local = local
hv-conn-env-dev = dev
hv-conn-env-test = test
hv-conn-env-staging = staging
hv-conn-env-production = production
# Notice after a connection URL filled the form.
hv-conn-url-filled = Filled from the pasted URL.
hv-conn-url-secret = The password was taken out of the URL. It goes to the Keychain when you save.
# The password line of the notice when it is not kept in the Keychain.
# Variables:
#   $ret (Enum(never, until_restart, for_session)) - how long the password is kept.
hv-conn-url-secret-other = { $ret ->
        [never] The password was taken out of the URL. { -brand-hacivat } doesn't keep it and asks on every connect.
        [until_restart] The password was taken out of the URL. { -brand-hacivat } keeps it in memory until restart.
       *[for_session] The password was taken out of the URL. { -brand-hacivat } keeps it for this session.
    }
# After a URL parameter, what it became in the form.
# Variables:
#   $setting (String) - the form setting, e.g. "Encryption: Verify full".
hv-conn-url-mapped = became { $setting }
# The encryption setting a URL parameter became.
# Variables:
#   $mode (String) - the encryption mode, e.g. "Verify full".
hv-conn-url-setting-tls = Encryption: { $mode }
# After an unknown URL parameter.
hv-conn-url-extra = kept as an extra property
# Variables:
#   $engine (String) - the database engine, e.g. "MySQL".
hv-conn-url-engine-set = Engine set to { $engine } from the URL.
hv-conn-url-undo = Undo
# A pasted URL is for another engine than the form.
# Variables:
#   $engine (String) - the URL's engine, e.g. "MySQL".
#   $current (String) - the form's engine, e.g. "PostgreSQL".
hv-conn-url-switch = This is a { $engine } URL, and the form is set to { $current }.
hv-conn-url-switch-l2 = Nothing was changed yet.
# Variables:
#   $engine (String) - the URL's engine, e.g. "MySQL".
hv-conn-url-switch-btn = Switch to { $engine } and fill
# Variables:
#   $current (String) - the form's engine, e.g. "PostgreSQL".
hv-conn-url-keep = Keep { $current }
hv-conn-url-unrec = { -brand-hacivat } can't read this as a connection URL. Nothing was changed.
hv-conn-url-formats = It reads postgres://, mysql://, mariadb://, jdbc:… URLs, key=value strings and SQLite file paths.
# A pasted URL named several hosts.
# Variables:
#   $count (Number) - how many hosts the URL names, e.g. 2.
#   $host (String) - the host that is used, e.g. "db-1.acme.io".
hv-conn-url-multihost = The URL names { $count } hosts. { -brand-hacivat } connects to the first, { $host }.
# Closes the URL notice.
hv-conn-url-dismiss = Dismiss
# Toast after saving a connection.
# Variables:
#   $name (String) - the connection's name, e.g. "orders-prod".
hv-conn-toast-saved = Saved { $name }
# Toast title after "Copy as URL"; the URL follows on its own line.
hv-conn-toast-copied = Copied as URL
hv-conn-toast-nopw = The password isn't included.
# Problems that stop a save, shown under the field they belong to.
hv-conn-issue-name-missing = Enter a name.
hv-conn-issue-host-missing = Enter a host.
hv-conn-issue-host-invalid = This host can't be used. Remove spaces and URL characters.
hv-conn-issue-port-invalid = Enter a port from 1 to 65535.
hv-conn-issue-file-missing = Choose a database file, or let { -brand-hacivat } create it.
hv-conn-issue-path-refused = { -brand-hacivat } won't open this path. Choose a local file.
hv-conn-issue-create-and-read-only = A new file can't be opened read-only. Turn one of the two off.
hv-conn-issue-transport-unavailable = This connection uses a tunnel this version can't open yet.
hv-conn-issue-tls-ca-unreadable = The CA certificate can't be read. Verify CA needs one.
hv-conn-issue-tls-cert-unreadable = The client certificate or its key can't be read.
# Variables:
#   $mode (String) - the encryption mode, e.g. "Verify CA".
hv-conn-issue-tls-mode-unsupported = { $mode } isn't available for this engine yet.
# Variables:
#   $key (String) - the property's name, e.g. "sslkey".
hv-conn-issue-option-rejected = The property { $key } isn't allowed here.

## m2a-t21-shots

# A connection without an environment tag, as a word (the sheet and the capsule; hand-edited files only).
hv-conn-env-unset = no environment
# The import dialog's environment choice before one is picked.
hv-conn-import-choose = Choose…
# Toast when a deleted connection's saved password could not be removed from the system keyring.
hv-conn-toast-delete-secret = The connection was deleted, but its saved password couldn't be removed from the system keyring.

## m2a-finalfix

# The Encryption row of a live connection whose certificate chain was checked but whose host name was not (Verify CA).
# Variables:
#   $version (String) - the TLS version the server negotiated, e.g. "TLS 1.3".
hv-conn-enc-ca-only = { $version } · CA verified
# Under Advanced, when an extra property's key holds a password (db_password, x-pwd, ...).
# Variables:
#   $key (String) - the property's name, e.g. "db_password".
hv-conn-issue-option-secret = The property { $key } holds a password. Remove it and type the password in the Password field.

## m2b-t01-interfaces

# Placeholder issue texts for the SSH section and the TLS key (Task 1); Task 20 rewrites the
# SSH ones and Task 19 the TLS one.
# An SSH hop has no host.
# Review: PO
hv-conn-issue-ssh-host-missing = Enter the SSH host.
# An SSH hop has no user.
# Review: PO
hv-conn-issue-ssh-user-missing = Enter the user to sign in as.
# The tunnel has more jump hosts than Hacivat supports.
# Review: PO
hv-conn-issue-ssh-too-many-hops = A tunnel can go through at most 8 jump hosts.
# The SSH key file is missing or can't be read.
# Review: PO
hv-conn-issue-ssh-key-file-missing = This key file can't be found. Choose the private key, such as ~/.ssh/id_ed25519.
# The TLS client key uses the old OpenSSL encryption, which can't be decrypted.
# m2b-t19-tls (named exception): the placeholder rewritten.
# Review: PO
hv-conn-issue-tls-key-legacy-encrypted = This key is encrypted in an old format { -brand-hacivat } can't read. Convert it to an encrypted PKCS #8 key (openssl pkcs8 -topk8), then choose the new file.

## m2b-t20-ssh

# The SSH disclosure of the connection dialog (NOTES M2a surface 6).
hv-conn-section-ssh = SSH tunnel
# Summary of the folded SSH section when the connection uses no tunnel.
hv-conn-ssh-sum-off = Off
# Summary of the folded SSH section: a tunnel without jump hosts.
# Variables:
#   $host (String) - the SSH host, e.g. "bastion.acme.io".
hv-conn-ssh-sum-on = via { $host }
# Summary of the folded SSH section: a tunnel through jump hosts.
# Variables:
#   $host (String) - the SSH host, e.g. "bastion.acme.io".
#   $n (Number) - how many jump hosts lead to it, e.g. 1.
hv-conn-ssh-sum-hops = via { $host } · { $n ->
        [one] { $n } jump host
       *[other] { $n } jump hosts
    }
hv-conn-ssh-use = Connect through an SSH tunnel
hv-conn-ssh-import = Import from ~/.ssh/config…
hv-conn-ssh-host = SSH host
hv-conn-ssh-user = SSH user
# The SSH sign-in methods (a segmented control).
hv-conn-ssh-auth-agent = Agent
hv-conn-ssh-auth-key = Key file
hv-conn-ssh-auth-password = Password
hv-conn-ssh-auth-kbd = Keyboard-interactive
hv-conn-ssh-agent-note = Uses the keys loaded in ssh-agent.
hv-conn-ssh-key = Key file
hv-conn-ssh-passphrase = Passphrase
hv-conn-ssh-kbd-note = The server's questions, such as a one-time code, appear when you connect.
hv-conn-ssh-jumps = Jump hosts
hv-conn-ssh-add-jump = Add jump host
# The remove button of a jump host row (its tooltip and accessible name).
# Variables:
#   $n (Number) - the row's 1-based position, e.g. 2.
hv-conn-ssh-remove-jump = Remove jump host { $n }
# Under a jump host row without a host.
hv-conn-ssh-jump-host-missing = Enter the jump host.
# Under the jump host list when a jump host's key file (from ~/.ssh/config) is missing.
hv-conn-ssh-jump-key-missing = A jump host's key file can't be found. Fix its IdentityFile in ~/.ssh/config, then import the host again.
hv-conn-ssh-route = Route
# The first stop of the route row: this computer.
# Variables:
#   $os (Enum(mac, other)) - whether this computer is a Mac.
hv-conn-ssh-this-mac = { $os ->
        [mac] this Mac
       *[other] this computer
    }
hv-conn-ssh-hostkeys = Host keys
# The host key policies (a select; there is never an "accept everything" choice).
hv-conn-ssh-hk-ask = Ask for new hosts
hv-conn-ssh-hk-accept = Accept new, stop on change
hv-conn-ssh-hk-strict = Known hosts only
# The accessible name of the import menu.
hv-conn-ssh-import-title = Hosts in ~/.ssh/config
# Why an import entry is disabled.
hv-conn-ssh-proxycmd = uses ProxyCommand, not supported
# Why an import entry is disabled: one of its values looked like a password (never shown).
hv-conn-ssh-unusable-secret = holds a value that looks like a password, not imported
# Why an import entry is disabled.
# Variables:
#   $token (String) - the unsupported token, e.g. "%C".
hv-conn-ssh-unusable-token = uses { $token }, not supported
# Why an import entry is disabled: a cycle, or a chain longer than a tunnel allows.
hv-conn-ssh-unusable-depth = its jump hosts nest too deep
# Why an import entry is disabled: a ProxyJump names a host the file doesn't define.
hv-conn-ssh-unusable-unknown = names a jump host that isn't in the file
# What an import entry resolves to, through jump hosts.
# Variables:
#   $target (String) - the user and host it signs in to, e.g. "deploy@bastion.acme.io".
#   $jumps (String) - its ProxyJump hosts, e.g. "jump-eu".
hv-conn-ssh-import-via = { $target } · ProxyJump { $jumps }
# What an import entry resolves to, with a key file.
# Variables:
#   $target (String) - the user and host it signs in to, e.g. "ci@10.0.4.7".
#   $key (String) - its first IdentityFile, e.g. "~/.ssh/ci_ed25519".
hv-conn-ssh-import-key = { $target } · IdentityFile { $key }
# The import menu's only row when the file names no host.
hv-conn-ssh-import-none = No hosts in ~/.ssh/config
# The import menu's only row when the file can't be read.
hv-conn-ssh-import-unreadable = ~/.ssh/config can't be read
# The notice after an import.
# Variables:
#   $alias (String) - the imported Host alias, e.g. "bastion".
hv-conn-ssh-imported = Imported { $alias } from ~/.ssh/config: host, user, key and jump host.
# Lines of the import notice: what the import skipped or changed.
# Variables:
#   $line (Number) - the line in ~/.ssh/config, e.g. 12.
hv-conn-ssh-notice-match = Line { $line }: Match blocks were skipped.
# Variables:
#   $line (Number) - the line in ~/.ssh/config, e.g. 3.
hv-conn-ssh-notice-include = Line { $line }: Include nests too deep, so the files it names were skipped.
# Variables:
#   $alias (String) - the Host alias, e.g. "bastion".
hv-conn-ssh-notice-known-hosts = { $alias }: UserKnownHostsFile was ignored. { -brand-hacivat } checks its own known hosts and ~/.ssh/known_hosts.
# Variables:
#   $alias (String) - the Host alias, e.g. "bastion".
hv-conn-ssh-notice-strict-no = { $alias }: StrictHostKeyChecking no is read as Ask for new hosts.
# Variables:
#   $alias (String) - the Host alias, e.g. "bastion".
hv-conn-ssh-notice-agent = { $alias }: IdentityAgent was ignored. { -brand-hacivat } uses the agent in SSH_AUTH_SOCK.
# Variables:
#   $alias (String) - the Host alias, e.g. "bastion".
hv-conn-ssh-notice-jump-password = { $alias }: its ProxyJump held a password, which was removed and not kept.
# The SSH step of the connect trail.
hv-conn-step-ssh = SSH

## m2a-demo-extra

# After a URL parameter the connection may not use (the paste notice): it is not added.
# Variables:
#   $reason (Enum(unknown, refused)) - unknown: not an option Hacivat allows for this engine when the connection comes from a pasted or imported text; refused: never allowed here.
#   $engine (String) - the database engine, e.g. "PostgreSQL".
hv-conn-url-left-out = { $reason ->
        [unknown] isn't a connection option for { $engine }, so it was left out
       *[refused] isn't allowed, so it was left out
    }
# Technical details rows of a setting that stopped the connect before anything was sent.
hv-conn-err-property = Property
hv-conn-err-path = Path
hv-conn-err-problem = Problem
# The value of the Property or Path row: what was refused, then why.
# Variables:
#   $name (String) - the property's name (e.g. "foo") or the file's path.
#   $reason (Enum(unknown, encoding, script, secret, path, denied)) - why it was refused.
hv-conn-err-refused = { $name } — { $reason ->
        [unknown] not allowed for imported connections
        [encoding] encoded twice
        [script] runs commands, not allowed for imported connections
        [secret] holds a password; type it in the Password field
        [path] { -brand-hacivat } won't open this path
       *[denied] never allowed
    }

## m2a-dialog-retries

# Under a rejected password's error, once the server rejected 3 passwords in a row: no more are sent.
hv-conn-err-attempts-used = All 3 attempts are used. { -brand-hacivat } won't send another password, so the account isn't locked.
# Under the connection dialog's Password field after 3 rejected passwords: how to try again.
hv-conn-dlg-attempts-used = No attempts left for this server. To try again, change the host, port or user, or reopen this dialog.

## m2b-t22-live

# The capsule while a lost connection reconnects (B19): the attempt under way.
# Variables:
#   $n (Number) - the attempt, from 1, e.g. 3.
hv-capsule-reconnecting = Reconnecting · attempt { $n }
# The capsule between two reconnect attempts: when the next one starts.
# Variables:
#   $seconds (Number) - whole seconds until the next attempt, e.g. 4.
hv-capsule-next = in { $seconds } s
# The path of a connection through an SSH tunnel (the capsule, the status bar and the sheet).
# Variables:
#   $host (String) - the SSH host the database is reached from (the last hop), e.g. "bastion.acme.io".
hv-conn-path-ssh = SSH via { $host }
# The capsule, the status bar and the explorer after an idle disconnect (B18).
hv-conn-idle-cap = Disconnected (idle)
# Toast after an idle disconnect whose threshold is a whole number of hours.
# Variables:
#   $name (String) - the connection's name, e.g. "orders-dev".
#   $hours (Number) - the idle threshold in hours, e.g. 4.
hv-conn-idle-disconnected = Disconnected { $name } after { $hours } h idle. It reconnects when you use it.
# Toast after an idle disconnect whose threshold is not a whole number of hours.
# Variables:
#   $name (String) - the connection's name, e.g. "orders-dev".
#   $minutes (Number) - the idle threshold in minutes, e.g. 90.
hv-conn-idle-disconnected-minutes = Disconnected { $name } after { $minutes } min idle. It reconnects when you use it.
# Toast when a transaction has stayed open and idle for the connection's idle threshold.
# Variables:
#   $name (String) - the connection's name, e.g. "orders-prod".
#   $minutes (Number) - how long it has been idle, in minutes, e.g. 60.
hv-conn-idle-tx = A transaction on { $name } has been open and idle for { $minutes } min. It holds its locks until you commit it or roll it back.
# The sticky toast after a lost server session (D-085, B19): its title.
# Variables:
#   $name (String) - the connection's name, e.g. "orders-prod".
#   $state (Enum(reconnected, lost)) - reconnected: Hacivat opened a new session on its own; lost: it did not, because work was in progress.
hv-conn-lost-title = { $state ->
        [reconnected] Reconnected to { $name }
       *[lost] The session on { $name } was lost
    }
# The toast's first line.
# Variables:
#   $state (Enum(reconnected, lost)) - as in hv-conn-lost-title.
hv-conn-lost-body = { $state ->
        [reconnected] The server session ended, so { -brand-hacivat } opened a new one.
       *[lost] The server session ended while work was in progress, so { -brand-hacivat } didn't reconnect on its own. Retry connects again.
    }
# A line of the toast, only when a transaction was open.
hv-conn-lost-tx = Your open transaction ended, and the server rolled back its uncommitted changes.
# A line of the toast: what else the old session held.
# Variables:
#   $items (List) - what was lost, each an hv-conn-lost-item-* message, e.g. "temporary tables".
hv-conn-lost-other = Also gone: { $items }
hv-conn-lost-item-temp = temporary tables
hv-conn-lost-item-vars = session variables
# Variables:
#   $count (Number) - how many prepared statements were lost, e.g. 2.
hv-conn-lost-item-prepared = { $count ->
        [one] a prepared statement
       *[other] { $count } prepared statements
    }
hv-conn-lost-item-locks = session locks
hv-conn-lost-details = Show details
hv-conn-lost-dismiss = Dismiss
# The sticky alert after the read-only guard stopped a statement (Q35): its title.
# Variables:
#   $name (String) - the connection's name, e.g. "orders-prod".
hv-conn-gap-title = Read-only guard on { $name }
# The alert's line when the statement ended the guard's read-only transaction.
hv-conn-gap-tx-ended = { -brand-hacivat } stopped a statement that ended the read-only transaction on a read-only connection and closed that session.
# The alert's line when the statement changed data.
hv-conn-gap-changed-data = { -brand-hacivat } stopped a statement that changed data on a read-only connection and closed that session.
# A read-only connection whose role the server limits to reading (B21; the capsule, the status bar and the popover).
hv-conn-access-ro-enforced = read-only (enforced by server role)
# The sheet's short forms of the access labels (they fit its narrow value column).
hv-conn-ro-sheet-enforced = read-only · enforced
hv-conn-ro-sheet-guarded = read-only · guarded
# The read-only explanation popover (NOTES M2b surface 6).
hv-conn-ro-pop-enf-title = Read-only, enforced by the server
# Variables:
#   $user (String) - the role the connection signs in as, e.g. "app_ro".
hv-conn-ro-pop-enf-body = The role { $user } can only read here. The server itself refuses writes, whatever runs in { -brand-hacivat }.
hv-conn-ro-pop-enf-temp = TEMP alone doesn't weaken enforcement, so the label stays enforced.
hv-conn-ro-pop-g-title = Read-only is guarded, not enforced
# Variables:
#   $user (String) - the role the connection signs in as, e.g. "qa".
hv-conn-ro-pop-g-body = The role { $user } isn't limited to reading here, so { -brand-hacivat } stops writes itself: it checks every statement and runs it read-only.
hv-conn-ro-pop-g-limit = That catches mistakes. Only a read-only role on the server is a hard guarantee.
hv-conn-ro-pop-g-fix = To enforce it, ask your DBA for a role like this, then make it this connection's user:
hv-conn-ro-pop-maria-body = { -brand-hacivat } can't yet confirm that a MariaDB role is read-only, so it always guards MariaDB connections itself.
hv-conn-ro-pop-no-fix = No role change makes this label enforced, so there is no SQL to copy.
hv-conn-ro-pop-sqlite-body = SQLite has no server roles. { -brand-hacivat } opens the file read-only and lets through only statements that read.
hv-conn-ro-copy-sql = Copy SQL
# Toast after Copy SQL.
hv-conn-ro-copied = Copied the SQL. Replace the values in angle brackets before you run it.
# The lead of the role probe's findings, then one line each.
hv-conn-ro-warn-head = Found on this role:
hv-conn-ro-warn-superuser = It is a superuser.
hv-conn-ro-warn-bypass-rls = It bypasses row-level security.
hv-conn-ro-warn-replication = It can start replication.
hv-conn-ro-warn-create-role = It can create and change roles.
hv-conn-ro-warn-server-file-roles = It can read or write files on the database server.
hv-conn-ro-warn-execute-server-program = It can run programs on the database server.
hv-conn-ro-warn-signal-backend = It can cancel or end other sessions.
hv-conn-ro-warn-write-privileges = It can change data in some tables.
hv-conn-ro-warn-temp-privilege = It can create temporary tables, which read-only transactions block.
# Variables:
#   $name (String) - the extension's name, from the probe's fixed list, e.g. "dblink".
hv-conn-ro-warn-egress-extension = The { $name } extension can reach other servers.
hv-conn-ro-warn-role-grant-not-expanded = Other roles are granted to it, and { -brand-hacivat } can't see what they allow.
hv-conn-ro-warn-unparsed = { -brand-hacivat } couldn't read its privileges.
# The palette command that opens the read-only explanation of the current connection.
hv-cmd-connection-explain-access = Explain read-only access

## m2b-t21-prompts

# The new-host-key prompt (M2a NOTES surface 6). Its title.
# Variables:
#   $host (String) - the SSH host Hacivat dialled, e.g. "bastion.acme.io".
# Review: PO
hv-conn-hk-title = First connection to { $host }
# Variables:
#   $os (Enum(mac, other)) - whether this computer is a Mac.
# Review: PO
hv-conn-hk-body = { $os ->
        [mac] This Mac hasn't connected to this SSH server before. Compare the fingerprint with the one your administrator gave you.
       *[other] This computer hasn't connected to this SSH server before. Compare the fingerprint with the one your administrator gave you.
    }
# The fingerprint inset's label.
# Review: PO
hv-conn-hk-fp = Host key
# Review: PO
hv-conn-hk-store = { -brand-hacivat } saves it in its own known-hosts list. ~/.ssh/known_hosts isn't changed.
# The primary button.
# Review: PO
hv-conn-hk-trust = Trust and connect
# The tooltip and accessible name of the Copy button of a fingerprint inset.
# Review: PO
hv-conn-hk-copy-fp = Copy fingerprint

# The changed-host-key review (NOTES surface 1): a hard stop whose default is Cancel.
# Variables:
#   $host (String) - the SSH host Hacivat dialled, e.g. "bastion.acme.io".
# Review: PO
hv-conn-hk-changed-title = The host key of { $host } changed
# Review: PO
hv-conn-hk-changed-body = The server answered with a different key from the one stored for it. That happens when a server is rebuilt or its keys are rotated, and also when someone is intercepting the connection. { -brand-hacivat } stopped before signing in.
# The two fingerprint insets.
# Review: PO
hv-conn-hk-changed-old = Stored key
# Review: PO
hv-conn-hk-changed-new = Offered now
# Under the stored key: where it is stored.
# Review: PO
hv-conn-hk-changed-stored-in = Stored in
# Review: PO
hv-conn-hk-changed-src-hacivat = { -brand-hacivat }'s known hosts
# Variables:
#   $file (String) - the OpenSSH file, e.g. "~/.ssh/known_hosts".
#   $n (Number) - the line in that file, e.g. 14.
# Review: PO
hv-conn-hk-changed-src-openssh = { $file } line { NUMBER($n, useGrouping: "false") }
# Variables:
#   $host (String) - the SSH host, e.g. "bastion.acme.io".
# Review: PO
hv-conn-hk-changed-ask = Ask whoever runs { $host } whether its key changed, and compare the new fingerprint before you trust it.
# The trust-widening button: never the default.
# Review: PO
hv-conn-hk-changed-replace = Remove the old key and trust the new one
# Above the ssh-keygen command when the old key is in an OpenSSH file.
# Variables:
#   $file (String) - the OpenSSH file, e.g. "~/.ssh/known_hosts".
# Review: PO
hv-conn-hk-changed-openssh-note = { -brand-hacivat } doesn't change { $file }. If the new key is genuine, remove the old line in a terminal, then try again:
# Review: PO
hv-conn-hk-changed-copy = Copy command
# Toast after Copy command.
# Review: PO
hv-conn-hk-changed-copied = Copied the command
# Toast after the old key was replaced.
# Variables:
#   $host (String) - the SSH host, e.g. "bastion.acme.io".
# Review: PO
hv-conn-hk-changed-trusted = Trusted the new key of { $host }

# The keyboard-interactive prompt (NOTES surface 2). Server text is never placed in these.
# Variables:
#   $host (String) - the SSH host Hacivat dialled, e.g. "bastion.acme.io".
# Review: PO
hv-conn-kbd-title = Sign in to { $host }
# Variables:
#   $host (String) - the SSH host Hacivat dialled, e.g. "bastion.acme.io".
# Review: PO
hv-conn-kbd-asks = { $host } asks:
# The caption of the inset holding the server's own text.
# Review: PO
hv-conn-kbd-from-server = Sent by the server, shown as received
# Review: PO
hv-conn-kbd-no-text = The server sent no name or instructions.
# Variables:
#   $n (Number) - the attempt, 1 to 3.
# Review: PO
hv-conn-kbd-attempt = Attempt { $n } of 3
# Review: PO
hv-conn-kbd-not-kept = Answers go to this server only and aren't kept.
# Review: PO
hv-conn-kbd-rejected = The server didn't accept the answers and asked again.
# Review: PO
hv-conn-kbd-continue = Continue

# The SSH password and key passphrase prompts (NOTES surface 3).
# Variables:
#   $host (String) - the asking hop's host, e.g. "jump-eu.acme.io".
#   $n (Number) - the hop's 1-based position, e.g. 1.
#   $total (Number) - how many hops the route has, e.g. 2.
# Review: PO
hv-conn-ssh-secret-hop = { $host }, hop { $n } of { $total }
# Variables:
#   $host (String) - the asking hop's host, e.g. "jump-eu.acme.io".
# Review: PO
hv-conn-ssh-secret-pw-title = SSH password for { $host }
# Variables:
#   $user (String) - the SSH user and host, e.g. "deploy@jump-eu.acme.io".
# Review: PO
hv-conn-ssh-secret-pw-body = Enter the SSH password for { $user }. It's kept only if the connection works.
# Variables:
#   $key (String) - the key file, e.g. "~/.ssh/id_ed25519".
# Review: PO
hv-conn-ssh-secret-pass-title = Passphrase for { $key }
# Review: PO
hv-conn-ssh-secret-pass-body = This key file is encrypted. Enter its passphrase. It's kept only if the connection works.
# Variables:
#   $host (String) - the asking hop's host, e.g. "jump-eu.acme.io".
# Review: PO
hv-conn-ssh-secret-wrong = { $host } didn't accept the password.
# After a passphrase that didn't decrypt the key (new: the mock shows only the password's line).
# Review: PO
hv-conn-ssh-secret-pass-wrong = That passphrase didn't unlock the key.
# Variables:
#   $n (Number) - the attempt, 1 to 3.
# Review: PO
hv-conn-ssh-secret-attempt = Attempt { $n } of 3
# Review: PO
hv-conn-ssh-secret-continue = Continue

# The vault (NOTES surface 4): create, unlock, reset, damaged.
# Review: PO
hv-conn-vault-create-title = Create a vault for saved passwords
# Review: PO
hv-conn-vault-create-why = This computer has no system keyring.
# Review: PO
hv-conn-vault-create-what = { -brand-hacivat } keeps the passwords you save Forever in an encrypted file, locked with a master password.
# Review: PO
hv-conn-vault-master = Master password
# Review: PO
hv-conn-vault-repeat = Repeat
# The create dialog's two live rules.
# Review: PO
hv-conn-vault-min = At least 8 characters
# Review: PO
hv-conn-vault-match = Both entries match
# Review: PO
hv-conn-vault-mismatch = The two entries don't match
# Review: PO
hv-conn-vault-norecover = { -brand-hacivat } can't recover it. If you forget it, you reset the vault and enter your saved passwords again.
# Review: PO
hv-conn-vault-notnow-note = Not now keeps this password until { -brand-hacivat } restarts.
# Review: PO
hv-conn-vault-notnow = Not now
# Review: PO
hv-conn-vault-create-btn = Create vault
# Toast after the vault was created.
# Review: PO
hv-conn-vault-created = Vault created. Passwords you save Forever go there.
# Review: PO
hv-conn-vault-unlock-title = Unlock the vault
# Variables:
#   $name (String) - the connection's name, e.g. "orders-dev".
# Review: PO
hv-conn-vault-unlock-body = { -brand-hacivat } needs the master password to read the saved password for { $name }. The vault stays unlocked until you quit.
# The unlock body when no single connection asks (new: the pipeline's question names none).
# Review: PO
hv-conn-vault-unlock-body-any = { -brand-hacivat } needs the master password to read your saved passwords. The vault stays unlocked until you quit.
# Review: PO
hv-conn-vault-unlock-wrong = That isn't the master password.
# Variables:
#   $n (Number) - the attempt, 1 to 3.
# Review: PO
hv-conn-vault-attempt = Attempt { $n } of 3
# Review: PO
hv-conn-vault-unlock-btn = Unlock
# Review: PO
hv-conn-vault-reset-link = Reset vault…
# Review: PO
hv-conn-vault-reset-title = Reset the vault?
# The mock counts the passwords; a locked or damaged vault can't be counted, so the body names none.
# Review: PO
hv-conn-vault-reset-body = This deletes the vault and every password saved in it. Your connections stay, and { -brand-hacivat } asks for each password the next time you connect. This can't be undone.
# Variables:
#   $word (String) - the word to type, hv-conn-vault-reset-word, e.g. "reset".
# Review: PO
hv-conn-vault-reset-type = Type { $word } to confirm
# The word typed to confirm a reset: the same in every language on purpose (B13).
# Review: PO
hv-conn-vault-reset-word = reset
# Review: PO
hv-conn-vault-reset-btn = Reset vault
# Toast after a reset.
# Review: PO
hv-conn-vault-reset-done = The vault was reset. Its saved passwords are gone.
# Review: PO
hv-conn-vault-damaged-title = The vault can't be read
# Review: PO
hv-conn-vault-damaged-body = The vault file failed its integrity check, so { -brand-hacivat } didn't use it and didn't change it. Restore it from a backup, or reset the vault.
# Review: PO
hv-conn-vault-damaged-meanwhile = Until then, passwords you enter are kept until { -brand-hacivat } restarts.
# The Remember choice "Forever" when the vault holds the secrets, with its hint.
# Review: PO
hv-conn-ret-forever-vault = Forever (encrypted file)
# Review: PO
hv-conn-ret-forever-vault-h = in { -brand-hacivat }'s vault, behind a master password

# The quick fix of ssh.host_key_changed: opens the review (NOTES surface 1).
# Review: PO
hv-conn-fix-review-host-key = Review host key…

# Fix round 1. Under the SSH secret prompt's Remember: the choice is the connection's.
# Review: PO
hv-conn-ssh-secret-remember-all = Applies to the saved SSH passwords and passphrases of every hop of this connection.
# Fix round 1. A save or delete that changed nothing because the stored passwords could not be
# moved or deleted (a locked vault whose unlock was declined, a refused write).
# Review: PO
hv-conn-save-unchanged = Nothing was changed: the saved passwords couldn't be moved or removed. If { -brand-hacivat } asks, unlock the vault, then try again.
# Fix round 1. The vault file is a link or not a regular file, so it can't be used.
# Review: PO
hv-conn-vault-denied = { -brand-hacivat } can't use the vault file: it isn't a regular file. Passwords you enter are kept until { -brand-hacivat } restarts.

## m2b-t19-tls

# Quick fixes of an untrusted certificate (B6): open the certificate review, or choose its CA file.
hv-conn-fix-review-cert = Review certificate…
hv-conn-fix-import-ca = Import CA…
# The certificate review's title (NOTES M2a surface 5).
hv-conn-tls-title = Certificate not trusted
# Variables:
#   $host (String) - the server's host as the connection names it, e.g. "db-qa.acme.io".
#   $port (String) - its port, e.g. "5432".
hv-conn-tls-body = { $host }:{ $port } offered a certificate that no CA trusted by this Mac has signed. Compare the fingerprint with whoever runs the server before you continue.
# The certificate's facts.
hv-conn-tls-subject = Issued to
hv-conn-tls-issuer = Issued by
hv-conn-tls-valid = Valid until
# The certificate's expiry date, in the regional format.
# Variables:
#   $when (DateTime) - the moment the certificate expires, e.g. 2027-03-14.
hv-conn-tls-valid-date = { DATETIME($when, dateStyle: "long") }
hv-conn-tls-fp = SHA-256 fingerprint
# The tooltip of the button that copies a fingerprint.
hv-conn-tls-copy-fp = Copy fingerprint
hv-conn-tls-chain = Chain
# A certificate's place in the chain, after its name.
hv-conn-tls-leaf = server
hv-conn-tls-inter = intermediate
hv-conn-tls-root = root · not trusted by this Mac
# Variables:
#   $host (String) - the server's host, e.g. "db-qa.acme.io".
#   $port (String) - its port, e.g. "5432".
hv-conn-tls-pin-note = Pinning trusts exactly this certificate for { $host }:{ $port }. If it ever changes, { -brand-hacivat } stops and asks again. A pin doesn't extend the expiry date.
hv-conn-tls-pin-btn = Pin this certificate
hv-conn-tls-import = Import CA…
# Under the certificate's facts when today is outside its dates: why nothing can trust it.
# Variables:
#   $state (Enum(expired, early)) - expired: past its expiry date; early: not valid yet.
hv-conn-tls-dates-note = { $state ->
        [expired] This certificate has expired. A pin doesn't extend the expiry date, so it can't be pinned. Ask whoever runs the server to renew it.
       *[early] This certificate isn't valid yet. Check the date and time of this Mac, or ask whoever runs the server.
    }
# The TLS section of the connection dialog: the endpoint's pinned certificate.
hv-conn-tls-pin = Pinned certificate
hv-conn-tls-pin-none = None
hv-conn-tls-remove-pin = Remove pin
# In the TLS section when Encryption is Disable and the endpoint has a pinned certificate.
hv-conn-issue-tls-disable-ignores-pin = With encryption off, the pinned certificate isn't checked. Choose another mode to use the pin.
# The client certificate's key passphrase field.
hv-conn-dlg-key-pass = Key passphrase
# Under the key passphrase field when the key is encrypted and the passphrase is missing or wrong.
hv-conn-issue-tls-key-passphrase = This key is encrypted. Enter its passphrase.
# The certificate of a pinned endpoint changed (NOTES M2b surface 5).
# Variables:
#   $host (String) - the server's host, e.g. "db-qa.acme.io".
#   $port (String) - its port, e.g. "5432".
hv-conn-tls-pin-changed-title = The certificate of { $host }:{ $port } changed
hv-conn-tls-pin-changed-body = This connection is pinned to one certificate, and the server now presents a different one. { -brand-hacivat } stopped before signing in.
# The two fingerprints: the pinned one, and the one the server presents now.
hv-conn-tls-pin-changed-old = Pinned
hv-conn-tls-pin-changed-new = Now
hv-conn-tls-pin-changed-ask = A renewed certificate looks like this too. Ask whoever runs the server before you replace the pin.
# Variables:
#   $host (String) - the server's host, e.g. "db-qa.acme.io".
#   $port (String) - its port, e.g. "5432".
hv-conn-tls-pin-changed-note = Replacing the pin trusts exactly the new certificate for { $host }:{ $port } and removes the old pin. A pin doesn't extend the expiry date.
hv-conn-tls-pin-changed-replace = Replace pin
# Toast after Replace pin.
# Variables:
#   $host (String) - the server's host, e.g. "db-qa.acme.io".
hv-conn-tls-pin-changed-done = Pinned the new certificate of { $host }
# The public-plaintext question (NOTES M2a surface 5): a public host offered no TLS.
# Variables:
#   $host (String) - the server's host, e.g. "db.acme.io".
hv-conn-plain-title = { $host } doesn't offer encryption
# Variables:
#   $ip (String) - the server's address, e.g. "203.0.113.10".
hv-conn-plain-body = { $ip } accepts only unencrypted connections and isn't on a private network. Your password and query results would cross the network in plain text.
hv-conn-plain-ssh = Use an SSH tunnel…
hv-conn-plain-connect = Connect without encryption
hv-conn-plain-remember = Don't ask again for this connection

## m2b-t23-f1

# A line of the read-only popover's findings list (the hv-conn-ro-warn-* family of Task 22):
# the role may run a PostgreSQL function that writes something a rollback can't undo, such as
# pg_logical_emit_message (RoleWarning::NonTransactionalWrite).
hv-conn-ro-warn-non-transactional-write = It can run functions whose effects a rollback can't undo.

## m2b-t24-shots

# The Encryption value of a live connection whose certificate is pinned (M2a NOTES enc-pinned).
# Variables:
#   $version (String) - the TLS version the server negotiated, e.g. "TLS 1.3".
hv-conn-enc-pinned = { $version } · pinned
# The test report's encryption row for a pinned certificate.
# Variables:
#   $version (String) - the TLS version number, e.g. "1.3".
hv-conn-diag-tls-pinned = TLS { $version } · pinned
# The same row for a pinned certificate when the TLS version is unknown.
hv-conn-diag-tls-nov-pinned = TLS · pinned
# The quick fix of tls.not_supported_by_server: the connection's SSH section (M2a NOTES fix-ssh).
hv-conn-fix-ssh = Connect through SSH…
# The sheet's Password row when Forever passwords live in Hacivat's vault (NOTES M2b vault-secret).
hv-conn-secret-vault = vault · forever

## m3a-t01b-seams

# The Edit menu title (the macOS menu bar and the Windows/Linux in-window menu); the group shows
# only while the editor commands are registered.
hv-menu-edit = Edit

## m3a-t25-editor-commands

# Command titles of the editor's commands (palette rows, the Edit menu and the editor's context menu).
hv-cmd-editor-move-left = Move left
hv-cmd-editor-move-right = Move right
hv-cmd-editor-move-up = Move up
hv-cmd-editor-move-down = Move down
hv-cmd-editor-select-left = Extend selection left
hv-cmd-editor-select-right = Extend selection right
hv-cmd-editor-select-up = Extend selection up
hv-cmd-editor-select-down = Extend selection down
hv-cmd-editor-move-word-left = Move to previous word
hv-cmd-editor-move-word-right = Move to next word
hv-cmd-editor-select-word-left = Extend selection to previous word
hv-cmd-editor-select-word-right = Extend selection to next word
hv-cmd-editor-move-line-start = Move to line start
hv-cmd-editor-move-line-end = Move to line end
hv-cmd-editor-select-line-start = Extend selection to line start
hv-cmd-editor-select-line-end = Extend selection to line end
hv-cmd-editor-move-document-start = Move to document start
hv-cmd-editor-move-document-end = Move to document end
hv-cmd-editor-select-document-start = Extend selection to document start
hv-cmd-editor-select-document-end = Extend selection to document end
hv-cmd-editor-page-up = Page up
hv-cmd-editor-page-down = Page down
hv-cmd-editor-select-page-up = Extend selection a page up
hv-cmd-editor-select-page-down = Extend selection a page down
hv-cmd-editor-newline = Insert line break
hv-cmd-editor-tab = Indent
hv-cmd-editor-outdent = Outdent
hv-cmd-editor-delete-backward = Delete backward
hv-cmd-editor-delete-forward = Delete forward
hv-cmd-editor-delete-word-backward = Delete previous word
hv-cmd-editor-delete-word-forward = Delete next word
hv-cmd-editor-delete-to-line-start = Delete to line start
hv-cmd-editor-select-all = Select all
hv-cmd-editor-undo = Undo
hv-cmd-editor-redo = Redo
hv-cmd-editor-copy = Copy
hv-cmd-editor-cut = Cut
hv-cmd-editor-paste = Paste
hv-cmd-editor-escape = Collapse selection
# Turns a mode on or off in which the Tab key moves the keyboard focus out of the editor instead of indenting.
hv-cmd-editor-toggle-tab-moves-focus = Toggle Tab moves focus
hv-cmd-editor-context-menu = Show editor menu
# Runs the statement at the caret, or the selection.
hv-cmd-editor-run = Run
# The second and third configurable variants of Run.
hv-cmd-editor-run-2 = Run (2)
hv-cmd-editor-run-3 = Run (3)
# Says in one sentence which lines Run would execute (the status bar and screen readers).
hv-cmd-editor-describe-exec-range = Describe execution range
# The sentence of "Describe execution range" when Run would execute one statement that spans several lines.
# Variables:
#   $from (Number) - the statement's first line, e.g. 12.
#   $to (Number) - the statement's last line, e.g. 14.
#   $label (String) - the start of the statement's text, e.g. "SELECT o.id, c.full_name, …".
hv-editor-describe-one = Run executes lines { $from }–{ $to }: { $label }
# The same when the statement is on one line.
# Variables:
#   $line (Number) - the statement's line, e.g. 12.
#   $label (String) - the start of the statement's text, e.g. "SELECT 1;".
hv-editor-describe-one-line = Run executes line { $line }: { $label }
# The sentence when Run would execute several statements (always two or more).
# Variables:
#   $n (Number) - how many statements, e.g. 3.
#   $from (Number) - the first statement's first line, e.g. 2.
#   $to (Number) - the last statement's last line, e.g. 4.
hv-editor-describe-many = Run executes { $n } statements on lines { $from }–{ $to }
# The same when the statements are all on one line.
# Variables:
#   $n (Number) - how many statements, e.g. 2.
#   $line (Number) - their line, e.g. 7.
hv-editor-describe-many-line = Run executes { $n } statements on line { $line }
# The sentence when Run would first ask which of several statements to execute.
hv-editor-describe-ask = Run asks which statement to execute
# The sentence when there is no statement at the caret.
hv-editor-describe-nothing = Nothing to run here
# The sentence when the editor can't tell where the statement at the caret ends.
hv-editor-describe-uncertain = Boundary uncertain: select the statement to run it
# The sentence when the console has no open connection.
hv-editor-describe-offline = Not connected: Run connects first and runs nothing
# The sentence when the editor has not finished reading the text just typed.
hv-editor-describe-pending = The editor is still reading this text. Try again in a moment.
# Status bar message and screen reader announcement after turning "Tab moves focus" on.
hv-editor-tab-moves-focus-on = Tab moves focus
# The same after turning it off.
hv-editor-tab-moves-focus-off = Tab indents

## m3a-t26-results-model

# A result tab's title (NOTES surface 9).
# Variables:
#   $n (Number) - the result's number in its console, from 1.
hv-results-result-n = Result { $n }
# The chip after a cell whose value was cut (NOTES surface 10, "… 5.2 MB").
# Variables:
#   $size (String) - the value's whole size (an hv-grid-size-* message), e.g. "5.2 MB".
hv-grid-truncated = … { $size }
# A value's size in bytes, inside the truncation chip.
# Variables:
#   $n (Number) - the bytes, e.g. 512.
hv-grid-size-bytes = { $n } B
# A value's size in kilobytes, inside the truncation chip and the copy limit.
# Variables:
#   $n (Decimal) - the kilobytes, e.g. 12.5.
hv-grid-size-kb = { $n } KB
# A value's size in megabytes.
# Variables:
#   $n (Decimal) - the megabytes, e.g. 5.2.
hv-grid-size-mb = { $n } MB
# A value's size in gigabytes.
# Variables:
#   $n (Decimal) - the gigabytes, e.g. 1.5.
hv-grid-size-gb = { $n } GB
# A value's length in characters, inside the truncation chip.
# Variables:
#   $n (Number) - the characters, e.g. 1999.
hv-grid-size-chars = { $n ->
        [one] { $n } character
       *[other] { $n } characters
    }
# A cell whose value could not be decoded (NOTES surface 10).
hv-grid-decode-error = Can't decode
# Toast after a copy from the grid (NOTES surface 14).
# Variables:
#   $rows (Number) - the rows copied, e.g. 3.
#   $cols (Number) - the columns copied, e.g. 8.
hv-copy-done = Copied { $rows } × { $cols } cells
# The second line of the copy toast.
hv-copy-done-line = Tab-separated, raw values
# The second line of the copy toast when some copied values are not whole: values the result
# cut short (their kept beginning was copied) or could not decode (copied as empty cells).
# Variables:
#   $n (Number) - how many of the copied values are incomplete, e.g. 3.
hv-copy-done-incomplete = { $n ->
        [one] { $n } of the copied values is incomplete: cut short or not decodable.
       *[other] { $n } of the copied values are incomplete: cut short or not decodable.
    }
# Toast while a large copy is prepared in the background.
# Variables:
#   $cells (Number) - the cells being copied, e.g. 48000.
hv-copy-working = Copying { $cells } cells…
# Toast title when a selection is too large to copy.
hv-copy-refused-title = Too much to copy
# Toast body when a selection is too large to copy.
# Variables:
#   $cells (Number) - the cells selected, e.g. 8000000.
#   $max (Number) - the most cells a copy takes, 1000000.
#   $size (String) - the most text a copy takes (an hv-grid-size-* message), e.g. "100 MB".
hv-copy-refused-body = The selection has { $cells } cells. Copy takes at most { $max } cells or { $size }; select fewer rows or columns.
# Toast when separate selections neither stack nor sit side by side, so they have no table shape.
hv-copy-refused-multiple = These selections can't be copied together. Select one block, or blocks that share their rows or their columns.

## m3a-t34-status-follows

# The status bar's transaction segment: the user's open transaction (NOTES M3a surface 6).
# Variables:
#   $n (Number) - the statements run since BEGIN, e.g. 3.
hv-tx-open = { $n ->
        [one] Transaction open · { $n } statement
       *[other] Transaction open · { $n } statements
    }
# The tooltip of the transaction segment.
hv-tx-open-tip = The statements since BEGIN aren't committed yet. Commit or roll back from the command palette.
# The transaction segment after an error or a cancel stopped the transaction.
hv-tx-failed = Transaction failed · roll back to end it
# The tooltip of the failed transaction segment.
hv-tx-failed-tip = An error or a cancel stopped the transaction. The server now accepts only a rollback.
# The active result's rows in the status bar, before its time is known.
# Variables:
#   $rows (Number) - the rows loaded, e.g. 100.
hv-results-rows = { $rows ->
        [one] { $rows } row
       *[other] { $rows } rows
    }
# The active result's rows while more may come (a cursor is open, rows are still arriving, or a
# limit cut it short).
# Variables:
#   $rows (Number) - the rows loaded so far, e.g. 10000.
hv-results-rows-more = { $rows }+ rows
# The active result's rows and time in the status bar.
# Variables:
#   $rows (Number) - the rows loaded, e.g. 100.
#   $duration (String) - the time (an hv-status-duration-* message), e.g. "131 ms".
hv-results-rows-timing = { $rows ->
        [one] { $rows } row · { $duration }
       *[other] { $rows } rows · { $duration }
    }
# The active result's rows and time while more may come.
# Variables:
#   $rows (Number) - the rows loaded so far, e.g. 10000.
#   $duration (String) - the time (an hv-status-duration-* message), e.g. "131 ms".
hv-results-rows-more-timing = { $rows }+ rows · { $duration }
# A time under a second in the status bar.
# Variables:
#   $ms (Number) - whole milliseconds, e.g. 131.
hv-status-duration-ms = { $ms } ms
# A time of a second or more in the status bar.
# Variables:
#   $seconds (Decimal) - seconds with two decimals, e.g. 2.45.
hv-status-duration-s = { NUMBER($seconds, minimumFractionDigits: 2) } s
# The status bar's button that closes the open cursor.
hv-results-status-cursor = Cursor open
# The tooltip of the cursor button.
hv-results-status-cursor-tip = Holding a database snapshot. Click to close the cursor.

## m3a-t28-gutter

# The status bar's sentence when the caret is where Run would run nothing (a blank line, a comment).
hv-editor-nothing-to-run = Nothing to run here
# The status bar's sentence when the statement at the caret has an uncertain boundary, so Run sends nothing.
hv-editor-uncertain = Boundary uncertain: select the statement to run it
# At the end of a statement's first line when a selection runs as separate statements.
# Variables:
#   $i (Number) - the statement's position, e.g. 2.
#   $n (Number) - the statements the selection runs, e.g. 3.
hv-editor-preview-unit = { $i } of { $n }
# At the end of the statement's first line when the console is not connected (Run connects first and runs nothing).
hv-editor-preview-offline = not connected
# At the end of the statement's first line when its boundary is uncertain.
hv-editor-preview-uncertain = boundary uncertain
# At the end of an executed statement's first line once it was edited.
hv-editor-mark-stale = edited since execution
# Tooltip of an executed statement's gutter mark once the statement was edited.
hv-editor-tip-stale = Edited since execution. The marks show the last run.
# Tooltip title of a gutter mark when two executed statements share its line; each statement's own outcome follows.
hv-editor-tip-two = Two statements on this line

## m3a-t32-console

# A bound console's tab title from the second console of a connection on; the first shows the connection's name alone.
# Variables:
#   $name (String) - the connection's name, e.g. "orders-dev".
#   $n (Number) - the console's number among that connection's consoles, e.g. 2.
hv-console-tab-title = { $name } ({ $n })
# The console toolbar's Run button.
hv-console-run = Run
# The console toolbar's Stop button, in place of Run while a statement runs.
hv-console-stop = Stop
# The console toolbar's run state while a statement runs; the elapsed time follows it.
hv-console-running = Running
# The elapsed time of the running statement in the console toolbar, repainted once a second.
# Variables:
#   $s (Number) - whole seconds since the run started, e.g. 3.
hv-console-clock = { $s } s
# The console toolbar's run state after Stop, until the server answers.
hv-console-cancelling = Cancelling…
# The console toolbar while the console waits for a session of its connection (every connection the profile allows is in use).
# Variables:
#   $name (String) - the connection's name, e.g. "orders-dev".
#   $inUse (Number) - connections in use, e.g. 4.
#   $max (Number) - connections the profile allows, e.g. 4.
hv-console-waiting = Waiting for a free connection to { $name } ({ $inUse } of { $max } in use)
# The short form of the waiting sentence when the toolbar is narrow (the connection's name is already in the toolbar).
# Variables:
#   $inUse (Number) - connections in use, e.g. 4.
#   $max (Number) - connections the profile allows, e.g. 4.
hv-console-waiting-short = Waiting for a connection ({ $inUse } of { $max } in use)
# Stops waiting for a free connection.
hv-console-waiting-cancel = Cancel
# The status bar's caret position in the console's editor (raw numbers, like the line numbers).
# Variables:
#   $line (Number) - the caret's line, from 1, e.g. 14.
#   $col (Number) - the caret's column, from 1, e.g. 49.
hv-editor-lncol = Ln { NUMBER($line, useGrouping: "false") }, Col { NUMBER($col, useGrouping: "false") }
# The status bar's count of selected characters in the console's editor.
# Variables:
#   $n (Number) - selected characters, e.g. 186.
hv-editor-selected = { $n } selected
# The status bar's mark while the console's editor is in reduced mode (a large document).
hv-editor-reduced = Reduced mode
# A document size in megabytes, in the regional format (the reduced-mode banner, the restore notice).
# Variables:
#   $mb (Decimal) - the size in megabytes, e.g. 18.4.
hv-console-size-mb = { NUMBER($mb, minimumFractionDigits: 1, maximumFractionDigits: 1) } MB
# A notice when a console's saved text is too large to open at launch.
# Variables:
#   $name (String) - the console's tab title, e.g. "orders-dev (2)".
hv-console-restore-refused-title = Couldn't reopen { $name }
# The notice's text.
# Variables:
#   $size (String) - the saved text's size (an hv-console-size-mb message), e.g. "31.4 MB".
#   $limit (String) - the largest size the editor opens (an hv-console-size-mb message), e.g. "20.0 MB".
#   $file (String) - the name the unchanged file was kept under, next to the console's own, e.g. "2.sql.too-large-1794755669".
hv-console-restore-refused-body = Its saved text is { $size }, more than the { $limit } this version opens. The file was kept unchanged as { $file }.
# The notice's text when the console's saved text could not be read.
# Variables:
#   $file (String) - the name the unchanged file was kept under, e.g. "2.sql.unreadable-1794755669".
hv-console-restore-unreadable-body = Its saved text couldn't be read. The file was kept unchanged as { $file }.
# The Edit menu's Undo when it would take back typing.
hv-edit-undo-typing = Undo Typing
# The Edit menu's Undo when it would take back a paste.
hv-edit-undo-paste = Undo Paste
# The Edit menu's Undo when it would take back a drop.
hv-edit-undo-drop = Undo Drop
# The Edit menu's Undo when it would take back a deletion.
hv-edit-undo-delete = Undo Delete
# The Edit menu's Undo when it would take back a cut.
hv-edit-undo-cut = Undo Cut
# The Edit menu's Undo when it would take back an indent.
hv-edit-undo-indent = Undo Indent
# The Edit menu's Undo when it would take back an outdent.
hv-edit-undo-outdent = Undo Outdent
# The Edit menu's Redo when it would apply typing again.
hv-edit-redo-typing = Redo Typing
# The Edit menu's Redo when it would apply a paste again.
hv-edit-redo-paste = Redo Paste
# The Edit menu's Redo when it would apply a drop again.
hv-edit-redo-drop = Redo Drop
# The Edit menu's Redo when it would apply a deletion again.
hv-edit-redo-delete = Redo Delete
# The Edit menu's Redo when it would apply a cut again.
hv-edit-redo-cut = Redo Cut
# The Edit menu's Redo when it would apply an indent again.
hv-edit-redo-indent = Redo Indent
# The Edit menu's Redo when it would apply an outdent again.
hv-edit-redo-outdent = Redo Outdent
# A notice when a connection was made read-only (tagged production) while a console holds its open transaction: the console turns read-only once that transaction ends.
# Variables:
#   $name (String) - the connection's name, e.g. "orders".
hv-console-ro-pending = { $name } turns read-only once its open transaction is committed or rolled back.
# The short form of the pending read-only chip in a narrow console toolbar.
hv-console-ro-pending-short = Read-only after this transaction
# A notice when a console's own session could not be opened.
# Variables:
#   $name (String) - the console's tab title, e.g. "orders-dev (2)".
hv-console-session-failed = Couldn't open a connection for { $name }. Run tries again.
# A notice when Disconnect or Delete must ask about an open transaction while another such question is still open.
hv-console-release-busy = Answer the open question first, then try again.
## m3a-t30-results-view

# The results tile's first tab: the console's output log (NOTES surface 9).
hv-results-output = Output
# The accessible name of the dot on the Output tab: a line arrived while another tab was active.
hv-results-unread = New output
# The accessible name of the results tile's tab list.
hv-results-tabs = Result tabs
# After a result tab's title when its connection is not production (production has the red swatch).
# Variables:
#   $env (String) - the environment word (an hv-conn-env-* message), e.g. "staging".
hv-results-env-suffix = ({ $env })
# The result toolbar's copy button, and the Output toolbar's.
hv-results-copy = Copy
# The Output toolbar: empty the log.
hv-results-log-clear = Clear
# The Output toolbar: show the newest line and follow new ones.
hv-results-log-end = Scroll to end
# The Output tab before anything ran in this console.
hv-results-log-empty = No output yet. Messages from the statements you run appear here.
# The accessible name of the output log.
hv-results-log-label = Output log
# The body of a finished result that has no rows; its header stays.
hv-grid-no-rows = No rows
# The info row under the result toolbar when a result has more than 2,000 columns.
# Variables:
#   $n (Number) - the result's columns, e.g. 2412.
hv-grid-too-many-columns = { $n } columns. Wide results scroll sideways; select fewer columns to read them more easily.
# A grid row that is not loaded yet.
hv-grid-loading = Loading
# The cursor strip while rows arrive.
# Variables:
#   $rows (Number) - the rows so far, e.g. 3500.
hv-strip-fetching = { $rows ->
        [one] Fetching… { $rows } row
       *[other] Fetching… { $rows } rows
    }
# The cursor strip's count while a cursor holds more rows.
# Variables:
#   $rows (Number) - the rows loaded, e.g. 10000.
hv-strip-rows-more = { $rows }+ rows
# The cursor strip while a cursor is open.
hv-strip-cursor-open = Cursor open — holding a database snapshot
# hv-strip-cursor-open when the strip is short; the whole sentence stays in its tooltip.
hv-strip-cursor-open-short = Cursor open
# Cursor strip button: close the open cursor.
hv-strip-close-cursor = Close cursor
# Cursor strip button: fetch the next rows.
hv-strip-fetch-more = Fetch more
# Cursor strip button: fetch every row, up to the memory limit.
hv-strip-fetch-all = Fetch all
# The cursor strip of a complete result.
# Variables:
#   $rows (Number) - the result's rows, e.g. 1000000.
hv-strip-complete = { $rows ->
        [one] { $rows } row
       *[other] { $rows } rows
    }
# Cursor strip button: run the statement behind a closed or cancelled result again.
hv-strip-rerun = Re-run
# The cursor strip of a cancelled result.
# Variables:
#   $rows (Number) - the rows it had, e.g. 3500.
hv-strip-cancelled = { $rows ->
        [one] Cancelled after { $rows } row
       *[other] Cancelled after { $rows } rows
    }
# The memory banner in the cursor strip (R-M3-33).
# Variables:
#   $rows (Number) - the rows kept, e.g. 412880.
#   $size (String) - the memory they take (an hv-grid-size-* message), e.g. "256 MB".
hv-strip-memory = Memory limit for this result reached ({ $rows } rows, { $size })
# Memory banner button: raise the limit to the shared budget and fetch on.
hv-strip-fetch-anyway = Fetch more anyway
# The cursor strip of a result whose rows past the memory limit were counted, not kept.
# Variables:
#   $rows (Number) - the rows kept, e.g. 412880.
hv-strip-truncated = { $rows ->
        [one] { $rows } row kept: the memory limit for this result was reached
       *[other] { $rows } rows kept: the memory limit for this result was reached
    }
# The cursor strip of a cursor closed for being idle when its idle bound is not known (the log
# line that said it is gone); never says a duration.
# Variables:
#   $rows (Number) - the rows the result has, e.g. 100.
hv-strip-closed-idle = { $rows ->
        [one] Cursor closed after { $rows } row: it was idle too long.
       *[other] Cursor closed after { $rows } rows: it was idle too long.
    }
# The cursor strip after a statement returned more than one result set (R-M3-25).
# Variables:
#   $n (Number) - the result sets read and not shown, e.g. 2.
hv-strip-more-sets = { $n ->
        [one] { $n } more result set not shown
       *[other] { $n } more result sets not shown
    }
# The word before the selection's size on the right of the cursor strip.
hv-strip-selection = Selection
# The selection's size.
# Variables:
#   $rows (Number) - the selected rows, e.g. 3.
#   $cols (Number) - the selected columns, e.g. 2.
hv-strip-dims = { $rows } × { $cols }
# Command palette titles of the results tile's commands.
hv-cmd-results-copy = Copy the selected result cells
hv-cmd-results-fetch-more = Fetch more rows of the result
hv-cmd-results-fetch-all = Fetch all rows of the result
hv-cmd-results-close-cursor = Close the result's cursor
hv-cmd-results-rerun = Re-run the result's statement
# Command palette titles of the grid's commands.
hv-cmd-grid-move-left = Grid: move left
hv-cmd-grid-move-right = Grid: move right
hv-cmd-grid-move-up = Grid: move up
hv-cmd-grid-move-down = Grid: move down
hv-cmd-grid-select-left = Grid: extend the selection left
hv-cmd-grid-select-right = Grid: extend the selection right
hv-cmd-grid-select-up = Grid: extend the selection up
hv-cmd-grid-select-down = Grid: extend the selection down
hv-cmd-grid-jump-left = Grid: jump to the first column
hv-cmd-grid-jump-right = Grid: jump to the last column
hv-cmd-grid-jump-up = Grid: jump to the first row
hv-cmd-grid-jump-down = Grid: jump to the last loaded row
hv-cmd-grid-home = Grid: go to the row's first cell
hv-cmd-grid-end = Grid: go to the row's last cell
hv-cmd-grid-first-row = Grid: go to the first row
hv-cmd-grid-last-row = Grid: go to the last row
hv-cmd-grid-page-up = Grid: page up
hv-cmd-grid-page-down = Grid: page down
hv-cmd-grid-next-cell = Grid: next cell
hv-cmd-grid-prev-cell = Grid: previous cell
hv-cmd-grid-select-all = Grid: select all
hv-cmd-grid-select-row = Grid: select the row
hv-cmd-grid-select-column = Grid: select the column
hv-cmd-grid-copy = Grid: copy
hv-cmd-grid-copy-with-headers = Grid: copy with headers
hv-cmd-grid-escape = Grid: clear the selection
## m3a-t33-exec-ui

# Command: stop the running statement of the focused console (or stop fetching its rows).
hv-cmd-exec-cancel = Stop the running statement
# Command: commit the user's open transaction on the focused console.
hv-cmd-tx-commit = Commit transaction
# Command: roll back the user's open (or failed) transaction on the focused console.
hv-cmd-tx-rollback = Roll back transaction
# Run on a console whose connection is not connected: Hacivat connects and runs nothing.
# Variables:
#   $name (String) - the connection's name, e.g. "orders-dev".
hv-console-connect-first = Connecting to { $name }. Run again once it's connected.
# At the end of a statement's first line while it waits to run.
hv-editor-mark-queued = queued
# At the end of a statement's first line while it runs; the clock repaints once a second.
# Variables:
#   $duration (String) - how long it has run (an exec-dur-* message), e.g. "3 s".
hv-editor-mark-running = running · { $duration }
# At the end of a statement's first line when it returned rows.
# Variables:
#   $duration (String) - how long it took (an exec-dur-* message), e.g. "42 ms".
#   $rows (Number) - the rows it returned, e.g. 100.
hv-editor-mark-rows = { $rows ->
        [one] executed · { $duration } · { $rows } row
       *[other] executed · { $duration } · { $rows } rows
    }
# At the end of a statement's first line when it returned rows and more are available (its cursor is open).
# Variables:
#   $duration (String) - how long it took (an exec-dur-* message), e.g. "42 ms".
#   $rows (Number) - the rows fetched so far, e.g. 10000.
hv-editor-mark-rows-more = executed · { $duration } · { $rows }+ rows
# At the end of a statement's first line when it changed rows.
# Variables:
#   $rows (Number) - the rows affected, e.g. 12.
#   $duration (String) - how long it took (an exec-dur-* message), e.g. "38 ms".
hv-editor-mark-affected = { $rows ->
        [one] { $rows } row affected · { $duration }
       *[other] { $rows } rows affected · { $duration }
    }
# At the end of a statement's first line when it finished without rows or a count (DDL, SET).
# Variables:
#   $duration (String) - how long it took (an exec-dur-* message), e.g. "12 ms".
hv-editor-mark-done = executed · { $duration }
# At the end of a statement's first line when it failed (the SQLSTATE chip follows).
hv-editor-mark-failed = failed
# At the end of a statement's first line when a read-only console did not send it.
hv-editor-mark-denied = not sent: read-only
# At the end of a statement's first line while its cancel is on its way.
hv-editor-mark-cancelling = cancelling…
# At the end of a statement's first line when it was cancelled.
# Variables:
#   $duration (String) - how long it ran (an exec-dur-* message), e.g. "12.40 s".
hv-editor-mark-cancelled = cancelled after { $duration }
# At the end of a statement's first line when an earlier statement of its run failed.
hv-editor-mark-skipped = skipped: an earlier statement failed
# The short form of a status on a long line, after the green check: how long the statement took.
# Variables:
#   $duration (String) - how long it took (an exec-dur-* message), e.g. "42 ms".
hv-editor-mark-short-done = { $duration }
# The short form of a running status on a long line: the clock, in amber.
# Variables:
#   $duration (String) - how long it has run (an exec-dur-* message), e.g. "3 s".
hv-editor-mark-short-running = { $duration }
# At the end of the first line of the statement Run would open the chooser on, with the chevrons-up-down glyph.
# Variables:
#   $n (Number) - the candidates the chooser offers, e.g. 2.
hv-editor-preview-choices = { $n } choices
# The statement chooser's title (D-011): Run has more than one candidate.
hv-chooser-title = Choose what to run
# Chooser kind: the statement at the caret.
hv-chooser-statement = Statement
# Chooser kind: the subquery at the caret.
hv-chooser-subquery = Subquery
# Chooser kind: the largest statement (or batch) at the caret.
hv-chooser-largest = Largest statement
# Chooser kind: from the statement at the caret to the end of the script.
hv-chooser-from-caret = From caret
# Chooser kind: every statement of the script.
hv-chooser-whole = Whole script
# A chooser row: the candidate's kind, and its count of statements when it runs several.
# Variables:
#   $kind (String) - the kind (an hv-chooser-statement, -largest, -from-caret, -whole or -subquery message), e.g. "Whole script".
#   $n (Number) - the statements the candidate runs, e.g. 5.
hv-chooser-row = { $n ->
        [one] { $kind }
       *[other] { $kind } · { $n } statements
    }
# The chooser's state when Run sent nothing because a statement boundary is uncertain.
hv-chooser-uncertain-title = Boundary uncertain
# The first line under "Boundary uncertain"; the reason follows on its own line.
hv-chooser-uncertain-body = Hacivat can't tell exactly where this statement ends, so it sent nothing.
# The last line under "Boundary uncertain": what to do instead.
# Variables:
#   $key (String) - the Run shortcut, e.g. "⌘↵".
hv-chooser-uncertain-fix = Select exactly the text to run, then press { $key }.
# The last line under "Boundary uncertain" when no key runs the statement.
hv-chooser-uncertain-fix-unbound = Select exactly the text to run, then run it.
# The open-transaction prompt's title when a console or the app closes.
# Variables:
#   $name (String) - the console's title (the connection's name), e.g. "orders-dev".
#   $n (Number) - the statements in the open transaction, e.g. 3.
hv-tx-close-title = { $n ->
        [one] { $name } has an open transaction with { $n } statement
       *[other] { $name } has an open transaction with { $n } statements
    }
# The open-transaction prompt's body when a console closes (its transaction ends with it, on whichever session it runs).
hv-tx-close-body = Closing this console ends its transaction. Commit keeps the changes; Roll back discards them.
# The open-transaction prompt's body when the app restarts.
hv-tx-restart-body = Restarting ends the session. Commit keeps the changes; Roll back discards them.
# Button: commit the transaction, then restart.
hv-tx-commit-restart = Commit and restart
# Button: roll back the transaction, then restart.
hv-tx-rollback-restart = Roll back and restart
# The open-transaction prompt's body when the console's connection is about to change (released or replaced).
hv-tx-release-body = The console's connection is about to change, which ends this transaction. Commit keeps the changes; Roll back discards them.
# Button: commit the transaction, then go on (the connection changes).
hv-tx-commit-continue = Commit and continue
# Button: roll back the transaction, then go on (the connection changes).
hv-tx-rollback-continue = Roll back and continue
# The open-transaction prompt's body when the app quits.
hv-tx-quit-body = Quitting ends the session. Commit keeps the changes; Roll back discards them.
# Button: commit the transaction, then close the console.
hv-tx-commit-close = Commit and close
# Button: roll back the transaction, then close the console.
hv-tx-rollback-close = Roll back and close
# Button: commit the transaction, then quit.
hv-tx-commit-quit = Commit and quit
# Button: roll back the transaction, then quit.
hv-tx-rollback-quit = Roll back and quit
# Button: keep the console (or the app) open; the safe default.
hv-tx-cancel = Cancel
# The prompt's title when the console's transaction failed (only a rollback ends it).
# Variables:
#   $name (String) - the console's title (the connection's name), e.g. "orders-dev".
hv-tx-failed-title = The transaction on { $name } failed
# The prompt's body when the console's transaction failed.
# Variables:
#   $n (Number) - the statements in the failed transaction, e.g. 3.
hv-tx-failed-body = { $n ->
        [one] An error or a cancel stopped it, so it can only be rolled back. Its { $n } statement won't be kept.
       *[other] An error or a cancel stopped it, so it can only be rolled back. Its { $n } statements won't be kept.
    }
# Toast when Commit or Roll back did not end the transaction (the console or the app stays open).
# Variables:
#   $name (String) - the console's title (the connection's name), e.g. "orders-dev".
hv-tx-end-failed = The transaction on { $name } didn't end, so nothing was closed.
# Toast when stopping a statement inside the user's transaction failed that transaction (R-M3-16 amended).
# Variables:
#   $name (String) - the console's title (the connection's name), e.g. "orders-dev".
hv-tx-failed-by-stop = Stopping the statement failed the transaction on { $name }. Only a rollback can end it now.
# Run on a production console whose open transaction must end before it reconnects read-only: nothing is sent.
hv-exec-pending-read-only = Nothing was sent: this console becomes read-only once its open transaction ends. Commit or roll back first.
# Toast when Re-run is refused because the statement was edited since it ran.
hv-exec-rerun-edited = The statement was edited since it ran. Run it from the editor.

## m3b-t01b-seams

# Command titles of the M3b and M3c commands (palette rows, key hints). Task 1b registers them;
# each command's task fills its body and its surface strings.
hv-cmd-editor-add-caret-above = Add a caret above
hv-cmd-editor-add-caret-below = Add a caret below
hv-cmd-editor-select-next-occurrence = Select the next occurrence
hv-cmd-editor-select-all-occurrences = Select all occurrences
hv-cmd-editor-split-selection-into-lines = Split the selection into lines
hv-cmd-editor-undo-selection = Undo the selection change
hv-cmd-editor-redo-selection = Redo the selection change
hv-cmd-editor-fold = Fold
hv-cmd-editor-unfold = Unfold
hv-cmd-editor-fold-all = Fold all
hv-cmd-editor-unfold-all = Unfold all
hv-cmd-editor-find = Find
hv-cmd-editor-find-next = Find next
hv-cmd-editor-find-prev = Find previous
hv-cmd-editor-find-close = Close find
hv-cmd-editor-toggle-comment = Comment or uncomment
hv-cmd-editor-duplicate-line = Duplicate line
hv-cmd-editor-delete-line = Delete line
hv-cmd-editor-move-line-up = Move line up
hv-cmd-editor-move-line-down = Move line down
hv-cmd-editor-normalize-line-endings = Normalize line endings
hv-cmd-editor-history-prev = Previous statement from history
hv-cmd-editor-history-next = Next statement from history
hv-cmd-grid-grow-selection = Grid: expand the selection
hv-cmd-grid-shrink-selection = Grid: shrink the selection
hv-cmd-grid-find = Grid: find
hv-cmd-grid-goto-row = Grid: go to row
hv-cmd-grid-open-value = Grid: open the value viewer
hv-cmd-grid-context-menu = Grid: show the menu
hv-cmd-grid-auto-fit = Grid: fit the column widths
hv-cmd-grid-freeze = Grid: freeze the columns up to here
hv-cmd-grid-unfreeze = Grid: unfreeze the columns
hv-cmd-grid-sort-asc = Grid: sort ascending
hv-cmd-grid-sort-desc = Grid: sort descending
hv-cmd-grid-clear-sort = Grid: clear the sort
hv-cmd-grid-view-table = Grid: table view
hv-cmd-grid-view-transposed = Grid: transposed view
hv-cmd-grid-view-text = Grid: text view
hv-cmd-grid-toggle-record = Grid: show or hide the record view
hv-cmd-results-next-tab = Next result tab
hv-cmd-results-prev-tab = Previous result tab
hv-cmd-history-open = Open query history

## m3a-f4-ime
# The Edit menu item (macOS) that opens the system's Emoji & Symbols palette; AppKit's own name for it.
hv-cmd-app-character-palette = Emoji & Symbols

## m3b-t07-types

# A result column's type line for a MySQL/MariaDB column type code the app has no name for
# (rare: replication-only codes). The code is the protocol's number, e.g. 19.
# Variables:
#   $code (Number) - the column type code, from 0 to 255.
hv-results-type-mysql-code = MySQL type { $code }

## m3b-t22-tx-ui

# The transaction mode chip in the console toolbar (D-014, D-036; design/m3b-safe surface 3).
hv-tx-chip-auto = Auto-commit
hv-tx-chip-manual = Manual
hv-tx-chip-smart = Smart
# The chip's tooltip.
# Variables:
#   $mode (String) - the mode's name (an hv-tx-chip-* message), e.g. "Manual".
hv-tx-chip-tip = Transaction mode: { $mode }
# The chip's menu: its title row, the mode hints, the Isolation row.
hv-tx-menu-title = Transaction mode
hv-tx-mode-auto-hint = each statement commits as it runs
hv-tx-mode-manual-hint = changes wait for Commit
hv-tx-mode-smart-hint = Manual from the first change
hv-tx-menu-isolation = Isolation
# The isolation level the server picks itself; its SQL name follows in mono.
hv-tx-iso-default = Server default
# The Isolation row's hint while the server picks the level: its SQL name follows.
# Variables:
#   $level (String) - the engine's default level (SQL, untranslated), e.g. "READ COMMITTED".
hv-tx-iso-default-hint = Server default · { $level }
# Why a mode or the isolation can't change now (a transaction is open).
hv-tx-menu-reason-open = Commit or roll back first
# The note under the menu on a production console.
hv-tx-menu-prod-note = Production consoles start in Manual.
# The disabled chip's tooltip on a read-only session.
hv-tx-chip-ro-reason = A read-only session runs each statement on its own, so its mode is always Auto-commit.
# The transaction badge in the console toolbar (surface 4).
# Variables:
#   $n (Number) - statements in the open transaction, e.g. 3.
#   $age (String) - how long it has been open (an exec-dur-* message), e.g. "2 min".
hv-tx-badge = { $n ->
        [one] Transaction · { $n } statement · { $age }
       *[other] Transaction · { $n } statements · { $age }
    }
# The badge without its age (narrow toolbars).
# Variables:
#   $n (Number) - statements in the open transaction, e.g. 3.
hv-tx-badge-short = { $n ->
        [one] Transaction · { $n } statement
       *[other] Transaction · { $n } statements
    }
hv-tx-badge-failed = Transaction failed
hv-tx-badge-tip = Not committed yet: other sessions don't see these changes, and the rows they touch stay locked.
hv-tx-badge-failed-tip = An error or a cancel stopped the transaction. The server now accepts only a rollback.
hv-tx-badge-commit = Commit
hv-tx-badge-rollback = Roll back
# The status bar's transaction segment is a button that opens the chip's menu.
hv-tx-segment-tip = Click for Commit, Roll back and the transaction mode.
# The idle-transaction toast (surface 5).
# Variables:
#   $age (String) - how long the transaction has been open (an exec-dur-* message), e.g. "10 min".
#   $name (String) - the connection's name, e.g. "orders-prod".
hv-tx-idle-title = Transaction open for { $age } on { $name }
hv-tx-idle-body = It may hold locks that block other sessions.
hv-tx-idle-commit = Commit
hv-tx-idle-rollback = Roll back
hv-tx-idle-keep = Keep open
# The session-lost toast's transaction line: a session counts as maybe in a transaction while
# any request of the user's ran (T12), so the line never claims one was open.
hv-tx-lost-maybe = It may have had an open transaction; if so, the server rolled back its uncommitted changes.
# The connection dialog's single-session setting (Advanced; surface 12).
hv-conn-sessions = Sessions
hv-conn-single-session = Use one session for every console
hv-conn-single-session-sub = Consoles of this connection share one session: its transaction, variables and temporary tables.
# The Connections row when every console shares one session.
hv-conn-budget-single = One session, shared by every console
# The console toolbar's chip when the console shares its connection's one session.
hv-console-shared = Shared session
# Its tooltip.
# Variables:
#   $name (String) - the connection's name, e.g. "orders-dev".
hv-console-shared-tip = Every console of { $name } uses this session: a transaction opened here is open in all of them.

## m3b-t22-tx-ui-r1

# The idle-transaction toast's Commit or Roll back while the console's run waits for an answer
# (or still runs): nothing was sent.
hv-tx-idle-waits = The console is waiting for an answer to its run. Commit or roll back once it's answered.
# A connected profile's single-session setting changed: it applies to the sessions opened after
# the next reconnect.
# Variables:
#   $name (String) - the connection's name, e.g. "orders-dev".
hv-conn-single-session-reconnect = The session setting of { $name } applies after it reconnects.
# The capsule of a connection whose server refused the time zone in Settings. Retry connects
# without it.
# Variables:
#   $zone (String) - the time zone name (an IANA name, untranslated), e.g. "Europe/Istanbul".
hv-conn-zone-refused = The server refused time zone { $zone }

## m3b-t21-confirm

# The gate's confirmation dialog (R-M3-55, R-M3-84; design/m3b-safe surface 1): asked before a
# statement is sent. Nothing is sent until the user chooses.
# Variables:
#   $name (String) - the connection's name, e.g. "orders-prod".
hv-confirm-title = Run this statement on { $name }?
# The title for a statement that only locks rows (SELECT … FOR UPDATE).
# Variables:
#   $name (String) - the connection's name, e.g. "orders-prod".
hv-confirm-title-warn = This statement locks rows on { $name }
# The subtitle's position of the statement in a run of several.
# Variables:
#   $i (Number) - the statement's position, from 1.
#   $n (Number) - the statements in the run.
hv-confirm-sub-unit = statement { $i } of { $n }
# The caption above the statement's text.
hv-confirm-sql = Statement
# The label of the row that says what the statement does.
hv-confirm-effect = Effect
# The dialog's last line on production.
hv-confirm-prod-line = Nothing is sent until you choose.
# The dialog's last line off production (the no-WHERE guard, which a setting turns off).
hv-confirm-calm-line = Off production, Hacivat asks only before statements like this one. Settings › Execution can turn it off.
# What the statement does (the Effect row), one per kind.
hv-confirm-class-lock = Reads rows and locks them
hv-confirm-class-dml = Changes data
hv-confirm-class-dml-unsafe = Changes data with no usable filter
hv-confirm-class-ddl-destructive = Removes schema objects or their data
hv-confirm-class-ddl-other = Changes the schema
hv-confirm-class-routine = Runs a procedure
hv-confirm-class-admin = Administers the server or reaches outside the database
hv-confirm-class-tx = Controls the transaction
hv-confirm-class-unknown = Unknown: Hacivat can't tell what it does
# MySQL and MariaDB commit a schema change at once.
hv-confirm-mysql-ddl = MySQL commits DDL at once, so this can't be rolled back.
# Above the field the name is typed into.
# Variables:
#   $name (String) - the table or schema name exactly as written in the statement, e.g. "müşteriler".
hv-confirm-type-label = Type { $name } to run it
# Under the field after a typed name that doesn't match.
hv-confirm-type-hint = Type it exactly as shown, letter case included.
# The small button that copies the name to type (for a name the keyboard can't reproduce).
hv-confirm-copy-name = Copy the name
hv-confirm-execute = Execute
hv-confirm-execute-all = Execute all
hv-confirm-execute-suppress = Execute and don't ask again
# On production the label names where "don't ask again" applies.
hv-confirm-execute-suppress-prod = Execute; don't ask again in this console
# Variables:
#   $console (String) - the console's title (the connection's name), e.g. "orders-prod".
hv-confirm-suppress-scope = Not asked again for statements of the same shape in { $console } until it closes.
hv-confirm-skip = Skip
hv-confirm-skip-all = Skip all
hv-confirm-cancel = Cancel
# What Execute all also answers (RC-1). Never shown on production. (m3b-po, PO M3b-26: edited
# in place to name the connection's real environment instead of "production".)
# Variables:
#   $n (Number) - the later statements Execute all runs without asking.
#   $env (String) - the connection's environment word (an hv-conn-env-* message), e.g. "staging".
hv-confirm-execute-all-scope = { $n ->
        [one] Execute all also runs, without asking, the remaining { $n } statement that would ask on this connection ({ $env }). A statement that needs its name typed still asks.
       *[other] Execute all also runs, without asking, the remaining { $n } statements that would ask on this connection ({ $env }). A statement that needs its name typed still asks.
    }

# The confirm-in-transaction prompt (R-M3-55, D-014; design/m3b-safe surface 2): a production
# change ran inside a transaction and waits for an answer.
# Hacivat's own transaction (Auto-commit, nothing open before).
# Variables:
#   $rows (Number) - the rows the statement changed.
#   $name (String) - the connection's name, e.g. "orders-prod".
hv-commit-title = { $rows ->
        [one] { $rows } row changed on { $name }
       *[other] { $rows } rows changed on { $name }
    }
# As hv-commit-title when the server reported no row count.
# Variables:
#   $name (String) - the connection's name, e.g. "orders-prod".
hv-commit-title-unknown = The statement changed data on { $name }
hv-commit-body-own = Hacivat ran this statement in a transaction of its own. Other sessions don't see the change until you commit.
hv-commit-locks = Its rows stay locked while this waits. With no answer, Hacivat rolls back after 5 minutes.
hv-commit-commit = Commit
hv-commit-rollback = Roll back
# The countdown; the time is minutes and seconds, e.g. 4:32.
# Variables:
#   $time (String) - the time left, e.g. "4:32".
hv-commit-countdown = Rolls back in { $time }
# Inside the user's open transaction (a savepoint).
# Variables:
#   $rows (Number) - the rows the statement changed.
hv-commit-sp-title = { $rows ->
        [one] { $rows } row changed inside your transaction
       *[other] { $rows } rows changed inside your transaction
    }
# As hv-commit-sp-title when the server reported no row count.
hv-commit-sp-title-unknown = The statement changed data inside your transaction
hv-commit-sp-keep-line = Keep: the change joins your open transaction. Nothing is committed yet.
hv-commit-sp-undo-line = Undo: removes this statement's change; your transaction stays open.
hv-commit-sp-locks = With no answer, Hacivat undoes the statement after 5 minutes and leaves your transaction open.
hv-commit-keep = Keep
hv-commit-undo = Undo
# Variables:
#   $time (String) - the time left, e.g. "4:51".
hv-commit-sp-countdown = Undoes this statement in { $time }
# The connection was lost while the prompt waited.
# Variables:
#   $name (String) - the connection's name, e.g. "orders-prod".
hv-commit-dropped-title = The connection to { $name } was lost
hv-commit-dropped-body = The server rolled back these changes. Nothing was committed.
# Variables:
#   $n (Number) - the statements of the user's transaction before this one.
hv-commit-dropped-sp-body = The server rolled back your open transaction: this statement and the { $n } before it. Nothing was committed.
# As hv-commit-dropped-sp-body when the count is not known.
hv-commit-dropped-sp-body-unknown = The server rolled back your open transaction, this statement included. Nothing was committed.
hv-commit-close = Close
# Fix round 1 (m8): the name to type holds a control character (a tab or a line break in a
# quoted name), which the field can't take: the statement can't be confirmed here.
hv-confirm-type-untypeable = This name contains a character that can't be typed or pasted, so it can't be confirmed here. Run this statement from another client.

## m3b-t25-results

# A result tab whose title repeats an earlier tab's (a statement's further result set, or two
# statements under the same comment). The title is the statement's comment, as written, or
# hv-results-result-n.
# Variables:
#   $title (String) - the tab's title, e.g. "orders by city".
#   $n (Number) - which repeat this is, from 2.
hv-results-tab-dup = { $title } ({ $n })
# The tooltip of a pinned result tab's pin glyph.
hv-results-tab-pinned = Pinned: the next run opens a new tab
# The result tab menu (right-click on a result tab).
hv-results-tab-pin = Pin tab
hv-results-tab-unpin = Unpin tab
hv-results-tab-close-others = Close other tabs
# After a result tab's title: the statements ran as one, so the server sent every column as
# text (R-M3-60).
hv-results-untyped = untyped (run as one)
# The short form of hv-results-untyped when the tab strip is short.
hv-results-untyped-short = untyped
# The tooltip of hv-results-untyped.
hv-results-untyped-tip = Run as one sends the statements together, so the server returns every column as text. Numbers sort as text.
# The info row under the result toolbar when a statement returned more result sets than a run
# keeps (R-M3-66).
# Variables:
#   $kept (Number) - the result sets shown, e.g. 100.
#   $skipped (Number) - the result sets read and not kept, e.g. 37.
hv-results-set-limit = The first { $kept } result sets are shown; { $skipped } more were read and not kept.
# The result toolbar's Count rows button (R-M3-76).
hv-results-count = Count rows
# The tooltip of hv-results-count.
hv-results-count-tip = Runs SELECT COUNT(*) over this query on a separate session.
# Beside Count rows: the count the server answered.
# Variables:
#   $rows (Number) - the rows the query returns, e.g. 1048213.
hv-results-count-exact = { $rows ->
        [one] { $rows } row (exact)
       *[other] { $rows } rows (exact)
    }
# Beside Count rows while the count runs.
hv-results-counting = Counting…
# Stops a count that is running.
hv-results-count-cancel = Cancel
# Beside Count rows: why this result's query can't be counted (only one SELECT, WITH, VALUES or
# TABLE statement without parameters or locking clauses is counted).
hv-results-count-unavailable = This query can't be counted.
# Beside Count rows: the read-only check refused the count statement; nothing was sent.
hv-results-count-refused = Not counted: Hacivat could not confirm the count only reads.
# Beside Count rows: the count ran for 30 seconds and was stopped.
hv-results-count-timeout = Stopped after 30 seconds.
# Beside Count rows: the server or the session failed the count.
hv-results-count-failed = The count failed.
# Beside Count rows: the console has no connection to count on.
hv-results-count-offline = Not counted: the console isn't connected.
# The cursor strip of a background result closed to free memory (R-M3-68); Re-run brings the
# rest back.
# Variables:
#   $rows (Number) - the rows the result kept, e.g. 4096.
hv-strip-evicted = { $rows ->
        [one] Closed to free memory; the first row is kept.
       *[other] Closed to free memory; the first { $rows } rows are kept.
    }
# A copy whose selection reaches past the rows a result kept after it was closed to free
# memory: nothing was copied.
hv-results-copy-evicted = Nothing was copied: this result was closed to free memory and keeps fewer rows than you selected. Select again, or re-run it.
# The fetch-all question (R-M3-68): asked when fetching every row would pass the result's
# memory limit.
hv-fetchall-title = Fetching every row would pass this result's memory limit
# Variables:
#   $size (String) - the result's memory limit, e.g. "256.0 MB".
hv-fetchall-body = The query has more rows than fit in { $size }. Hacivat can fetch up to the limit and keep the cursor open for the rest.
hv-fetchall-ok = Fetch up to the memory limit
hv-fetchall-cancel = Cancel

## m3b-t25-results-r1

# A repeated result tab title's number, drawn after the title so a cut title keeps it.
# Variables:
#   $n (Number) - which repeat this is, from 2.
hv-results-tab-repeat = ({ $n })
# The button after the result tabs that fit: how many more there are (a menu lists them).
# Variables:
#   $n (Number) - the result tabs not drawn, e.g. 12.
hv-results-tabs-more = +{ $n }
# The tooltip and accessible name of hv-results-tabs-more.
# Variables:
#   $n (Number) - the result tabs not drawn, e.g. 12.
hv-results-tabs-more-tip = { $n ->
        [one] { $n } more result tab
       *[other] { $n } more result tabs
    }
# Beside Count rows: the tab's statement is no longer known to the console (an older run).
hv-results-count-rerun = Re-run the statement to count its rows.
# Beside Count rows: the console has not read its SQL settings, so the statement can't be
# read safely yet.
hv-results-count-context = Not counted: the console's SQL settings aren't known yet.

## m3b-t23-escalation

# The slow-cancel toast (design/m3b-safe surface 6): a cancel not answered after 10 s.
hv-cancel-slow-title = Cancelling is taking long
# Variables:
#   $name (String) - the connection's name, e.g. "orders-dev".
#   $duration (String) - how long it has been cancelling (an exec-dur-* message), e.g. "11 s".
hv-cancel-slow-body = The server hasn't stopped the statement on { $name } after { $duration }.
# The toast's one button: it opens the escalation dialog (it never ends anything itself).
hv-cancel-options = Show options…
# The escalation dialog (a second Stop while cancelling, or Show options…).
hv-cancel-dlg-title = The statement isn't answering the cancel
# Variables:
#   $duration (String) - how long it has been cancelling (an exec-dur-* message), e.g. "14 s".
hv-cancel-dlg-body = It has been cancelling for { $duration }. Hacivat keeps waiting unless you choose otherwise.
hv-cancel-keep-waiting = Keep waiting
hv-cancel-terminate = Terminate on server
# Variables:
#   $fn (String) - the server function or statement that ends the session (SQL, untranslated), e.g. "pg_terminate_backend".
hv-cancel-terminate-line = Terminate on server ends this session on the server with { $fn }.
# Variables:
#   $n (Number) - statements in this console's open transaction, e.g. 3.
hv-cancel-own-tx = { $n ->
        [one] This console's open transaction ({ $n } statement) rolls back.
       *[other] This console's open transaction ({ $n } statements) rolls back.
    }
# This console's running statement may have opened a transaction (Manual or Smart mode).
hv-cancel-own-tx-maybe = This console may have an open transaction; if so, it rolls back.
# A transaction Hacivat opened for this console (a confirmed change waiting for its answer, a
# cursor) rolls back (fix round 1).
hv-cancel-own-tx-hacivat = The transaction Hacivat opened on this console (a change waiting for your answer, or an open cursor) rolls back.
# Another console holds a transaction Hacivat opened for it (fix round 1).
# Variables:
#   $console (String) - the other console's title, e.g. "orders-dev (2)".
hv-cancel-other-tx-hacivat = { $console } also has a transaction Hacivat opened; the server rolls it back.
# The app's existing verb (hv-conn-menu-disconnect); the engine call stays "deactivate".
hv-cancel-disconnect = Disconnect
# From the profile's default console: the whole profile disconnects (R-M3-73).
# Variables:
#   $name (String) - the connection's name, e.g. "orders-dev".
hv-cancel-disconnect-line-all = Disconnect closes every connection of { $name } here; the statement may continue on the server.
# From any other console: only its session.
# Variables:
#   $console (String) - the console's title, e.g. "orders-dev (2)".
hv-cancel-disconnect-line-one = Disconnect closes only this console's session ({ $console }); the statement may continue on the server.
# SQLite: a local file, nothing runs "on the server"; the profile's consoles share its one
# connection (single-session; fix round 1, m4).
# Variables:
#   $name (String) - the database file's name, e.g. "app.db".
hv-cancel-disconnect-line-local = Disconnect closes the connection to the file { $name } that this connection's consoles share; SQLite stops the statement when its connection closes.
# Another console of the profile holds a user transaction that the default console's
# Disconnect rolls back.
# Variables:
#   $console (String) - the other console's title, e.g. "orders-dev (2)".
#   $n (Number) - statements in its open transaction, e.g. 3.
hv-cancel-other-tx = { $n ->
        [one] { $console } also has an open transaction ({ $n } statement); the server rolls it back.
       *[other] { $console } also has an open transaction ({ $n } statements); the server rolls it back.
    }
# Another console runs a statement that may have opened a transaction (T12: a running request).
# Variables:
#   $console (String) - the other console's title, e.g. "orders-dev (2)".
hv-cancel-other-tx-maybe = { $console } may also have an open transaction; if so, the server rolls it back.
# A terminate the server refused; the server's own text follows in its own box, verbatim.
hv-cancel-terminate-failed = Couldn't terminate the session. Hacivat is still waiting for the cancel.
# The gutter's end-of-line status of a statement whose cancel was not answered (amber hourglass).
# Variables:
#   $duration (String) - how long it has been cancelling (an exec-dur-* message), e.g. "14 s".
hv-editor-mark-escalated = cancel not answered · { $duration }
# The error-policy bar (surface 7): a statement of a multi-statement run failed (R-M3-61).
# Variables:
#   $i (String) - the failed statement's number in the run, e.g. "2".
#   $n (String) - the statements in the run, e.g. "5".
hv-errbar-failed = Statement { $i } of { $n } failed
# Inside a failed PostgreSQL transaction (P-M3b-19).
# Variables:
#   $i (String) - the failed statement's number in the run, e.g. "3".
#   $n (String) - the statements in the run, e.g. "6".
hv-errbar-aborted = Statement { $i } of { $n } failed, and the transaction failed with it
# Variables:
#   $n (Number) - the statements after the failed one, e.g. 3.
hv-errbar-waiting = { $n ->
        [one] { $n } more waits
       *[other] { $n } more wait
    }
hv-errbar-retry = Retry
hv-errbar-ignore = Ignore
hv-errbar-ignore-all = Ignore all
hv-errbar-stop = Stop
hv-errbar-rollback-continue = Roll back and continue
# Roll back and continue's tooltip in Manual or Smart mode: the rest runs in a new transaction.
# Variables:
#   $n (Number) - statements in the user's open transaction, e.g. 2.
#   $next (String) - the number of the statement that runs next, e.g. "4".
hv-errbar-rollback-continue-tip = { $n ->
        [one] Rolls back your open transaction ({ $n } statement), then continues with statement { $next } in a new transaction.
       *[other] Rolls back your open transaction ({ $n } statements), then continues with statement { $next } in a new transaction.
    }
# As hv-errbar-rollback-continue-tip in Auto-commit mode (the user's own BEGIN).
# Variables:
#   $n (Number) - statements in the user's open transaction, e.g. 2.
#   $next (String) - the number of the statement that runs next, e.g. "4".
hv-errbar-rollback-continue-tip-auto = { $n ->
        [one] Rolls back your open transaction ({ $n } statement), then continues with statement { $next }, committing each statement as it runs.
       *[other] Rolls back your open transaction ({ $n } statements), then continues with statement { $next }, committing each statement as it runs.
    }

## m3b-t24-exec-ui

# The parameter prompt (design/m3b-safe surface 8): the values of a statement's parameters,
# asked before the statement is checked and sent. Values are data: never translated.
hv-params-title = Values for this statement
# The subtitle's count of parameters asked for.
# Variables:
#   $n (Number) - how many parameters the prompt asks for.
hv-params-sub = { $n ->
        [one] { $n } parameter
       *[other] { $n } parameters
    }
# Column headings of the prompt's rows.
hv-params-col-name = Name
hv-params-col-value = Value
hv-params-col-type = Type
# The tooltip of a row's NULL toggle.
hv-params-null-tip = Send NULL
# Under a value filled in from an earlier run of this app session.
hv-params-remembered = remembered
# The tooltip of "remembered".
hv-params-remembered-tip = From an earlier run in this app session; never saved to disk.
# A number grouped with dots where the region groups that way: it is read as a decimal unless
# the user writes it without the dots. Run stays off until the user decides.
# Variables:
#   $value (String) - the value as typed, e.g. "1.234".
#   $plain (String) - the same digits without dots, e.g. "1234".
hv-params-ambiguous = Read as the decimal { $value }: a dot is the decimal point here. If you meant { $plain }, write it without the dot.
# The button that accepts the decimal reading.
# Variables:
#   $value (String) - the value as typed, e.g. "1.234".
hv-params-ambiguous-use = Use { $value }
# A value written as bytes (\x…) that isn't pairs of hex digits.
hv-params-bad-hex = Bytes written as \x must be pairs of hex digits (0–9, a–f).
# A template variable can't be filled in as a parameter: the statement can't run as written.
# Variables:
#   $var (String) - the template variable as written, e.g. "${tablo}".
hv-params-template = { $var } is a template variable, not a parameter. Hacivat doesn't fill it in, so this statement can't run as written.
# PostgreSQL: a ? in the statement is the jsonb operator, not a parameter.
hv-params-pg-q = On PostgreSQL ? isn't a parameter here, because jsonb operators use it. Settings › Parameters can turn it on.
# The note at the end of the prompt.
hv-params-note = Values are kept for this app session only and are masked in history.
# The prompt's buttons.
hv-params-run = Run
hv-params-cancel = Cancel
# The tooltip of a disabled Run.
hv-params-resolve-first = Resolve the flagged value first
# The type column when the statement says nothing of the parameter's type.
hv-params-type-unknown = unknown
# The chooser's uncertain state (R-M3-60): sends the uncertain region in one request.
hv-chooser-run-as-one = Run as one
# The line under "Run as one".
hv-chooser-run-as-one-hint = Run as one sends the whole region in one request; its results come back untyped.
# The line under a disabled "Run as one" on a read-only session.
hv-chooser-run-as-one-ro = Not on a read-only session: its guard checks one statement at a time.
# The chooser's last row: opens how Run picks statements in this console.
hv-chooser-customize = Customize…
# The policy popover (CHOOSE-06): how Run picks statements in this console.
hv-policy-title = How Run picks statements in this console
hv-policy-inside = Caret inside a statement
hv-policy-inside-ask = Ask
hv-policy-inside-smallest = Smallest statement
hv-policy-inside-largest = Largest statement
hv-policy-inside-whole = Whole script
hv-policy-selection = With a selection
hv-policy-sel-separate = As separate statements
hv-policy-sel-as-one = As one
hv-policy-note = Kept for this console until it closes.
# A statement's gutter mark while its change waits for the user's answer (confirm in
# transaction; NOTES ruling 31).
hv-editor-mark-awaiting = waiting for Commit or Roll back
# The same in the savepoint form (the user's own transaction stays open).
hv-editor-mark-awaiting-sp = waiting for Keep or Undo
# A statement Run refused before sending it (its tooltip says why), on a read-write console.
hv-editor-mark-not-sent = not sent
# Once, when the query history file was damaged and set aside at startup.
hv-exec-history-recovered = The query history file was damaged, so Hacivat set it aside and started a new one.

## m3b-t27-history

# The query history dialog (design/m3b-safe surface 11).
hv-history-title = Query history
# The search field's placeholder.
hv-history-search = Search history
hv-history-search-tip = Search ignores letter case and accents: musteri finds müşteri.
# The scope: this console's connection, or every connection.
hv-history-filter-this = This connection
hv-history-filter-all = All connections
# The outcome filter.
hv-history-outcome-any = Any outcome
hv-history-outcome-ok = Succeeded
hv-history-outcome-failed = Failed
hv-history-outcome-cancelled = Cancelled
# An entry the safety gate refused (nothing was sent).
hv-history-outcome-denied = Not run
# Day groups of the list.
hv-history-today = Today
hv-history-yesterday = Yesterday
# The header's subtitle.
# Variables:
#   $n (Number) - the entries the list shows, e.g. 5.
hv-history-count = { $n ->
        [one] { $n } entry
       *[other] { $n } entries
    }
# A row's and the preview's row count.
# Variables:
#   $rows (Number) - rows fetched or affected, e.g. 12.
hv-history-rows = { $rows ->
        [one] { $rows } row
       *[other] { $rows } rows
    }
# The chip on an entry from which a secret was removed before it was saved.
hv-history-redacted = redacted
hv-history-redacted-line = A password or secret was removed before this was saved.
hv-history-pin = Pin
hv-history-unpin = Unpin
# The pin glyph's tooltip on a pinned row.
hv-history-pinned = Pinned: never removed by retention
hv-history-insert = Insert
hv-history-insert-run = Insert and run
hv-history-copy = Copy SQL
hv-history-open-new = Open in new console
hv-history-empty = No history yet. Statements you run are kept here for 90 days.
# Variables:
#   $q (String) - what the user searched for, e.g. "faturalar".
hv-history-nomatch = Nothing matches “{ $q }”.
# Variables:
#   $n (Number) - how many times the same statement ran, e.g. 4.
hv-history-ran = { $n ->
        [one] Ran once
       *[other] Ran { $n } times
    }
# Which console Insert and Insert and run go into (always shown; fix round 1).
# Variables:
#   $name (String) - that console's title, e.g. "orders-dev" or "orders-dev (2)".
hv-history-target = Goes into the console { $name }; Run checks it there.
# A row's time, in the regional format.
# Variables:
#   $t (DateTime) - when the statement last started, e.g. 2026-11-16T14:32:08.
hv-history-time = { DATETIME($t, timeStyle: "medium") }
# A day group's date, in the regional format (days before yesterday).
# Variables:
#   $t (DateTime) - a moment of that day, e.g. 2026-11-14.
hv-history-day = { DATETIME($t, dateStyle: "medium") }
# The entry's connection was deleted since.
hv-history-conn-gone = Deleted connection
# The history store could not answer (closed or damaged).
hv-history-failed = The history can't be read right now.
# Fix round 1: an entry with masked values (or one cut at the size limit) never runs as stored.
hv-history-masked-line = Masked values: fill them in before running.
hv-history-truncated-line = This statement was cut at 64 KiB when it was saved; it can't run as it is.
# The header's subtitle when the search shows its most entries (there may be more).
# Variables:
#   $n (Number) - the entries the list shows, e.g. 200.
hv-history-count-capped = { $n }+ entries
# Toasts when an action did nothing.
hv-history-console-gone = The console to insert into is closed.
hv-history-insert-refused = The statement couldn't be inserted: the editor is read-only, composing text, or would get too large.
hv-history-pin-failed = The pin couldn't be changed.
hv-history-open-failed = The new console didn't open in time; nothing was inserted.

## m3b-t31-multi

# The status bar's count of carets in the console's editor, before the selected count (shown from two carets on; NOTES surface 14).
# Variables:
#   $n (Number) - the carets, e.g. 3.
hv-editor-carets = { $n } carets
# Said once, politely, when an input method composition goes to the main caret only (R-M3-64).
hv-editor-ime-primary-only = Input method: typing at the main caret only, because another caret was inside a string or comment

## m3b-t29-shots
# The history list's selected entry, announced to assistive technology when ↑/↓ moves it:
# its first SQL line (data) and its place in the list.
# Variables:
#   $sql (String) - the entry's first SQL line, e.g. SELECT 1.
#   $pos (Number) - its place in the list, e.g. 3.
#   $n (Number) - the entries the list shows, e.g. 5.
hv-history-selection-a11y = { $sql }, { $pos } of { $n }

## m3b-t32-fold

# A folded range's inlay after its header line, behind an ellipsis glyph (NOTES surface 15).
# Variables:
#   $n (Number) - the lines the fold hides, e.g. 5.
hv-editor-fold-lines = { $n ->
        [one] { $n } line
       *[other] { $n } lines
    }
# The folded inlay's tooltip.
# Variables:
#   $from (Number) - the fold's first line, e.g. 2.
#   $to (Number) - its last line, e.g. 7.
hv-editor-fold-tip = Lines { $from }–{ $to } are folded. Click to unfold.
# The gutter's chevron on an open range (its name and tooltip).
# Variables:
#   $from (Number) - the range's first line, e.g. 2.
#   $to (Number) - its last line, e.g. 7.
hv-editor-fold-name = Fold lines { $from }–{ $to }
# The gutter's chevron on a folded range (its name and tooltip).
# Variables:
#   $from (Number) - the fold's first line, e.g. 2.
#   $to (Number) - its last line, e.g. 7.
hv-editor-unfold-name = Unfold lines { $from }–{ $to }
# A folded header's end-of-line status when a statement inside the fold failed (the SQLSTATE chip follows).
# Variables:
#   $line (Number) - the failed statement's first line, e.g. 9.
hv-editor-fold-failed = failed inside the fold · line { $line }
# A folded header's accessible description (screen readers), after its text.
# Variables:
#   $n (Number) - the lines the fold hides, e.g. 5.
hv-editor-fold-hidden = { $n ->
        [one] { $n } line folded
       *[other] { $n } lines folded
    }
# Said politely after a fold (the gutter, the inlay or the palette).
# Variables:
#   $from (Number) - the fold's first line, e.g. 2.
#   $to (Number) - its last line, e.g. 7.
hv-editor-fold-done = Folded lines { $from }–{ $to }
# Said politely after an unfold.
# Variables:
#   $from (Number) - the fold's first line, e.g. 2.
#   $to (Number) - its last line, e.g. 7.
hv-editor-unfold-done = Unfolded lines { $from }–{ $to }

## m3b-t35-grid

# The header tooltip of a frozen column (frozen columns keep their place when the grid scrolls
# sideways).
hv-grid-frozen-tip = Frozen: stays in place when you scroll sideways
# The column-resize tag's hint, after the width (hv-grid-width), while the pointer is on or
# drags a column header's edge.
hv-grid-resize-tip = Drag to resize · double-click to fit
# A column's width in the column-resize tag.
# Variables:
#   $px (Number) - the width in logical pixels, e.g. 136.
hv-grid-width = { $px } px
# The selection's size on the right of the cursor strip when it holds several ranges
# (Ctrl/Cmd-click adds one).
# Variables:
#   $n (Number) - the ranges, e.g. 2.
#   $cells (Number) - the cells they cover together, e.g. 10.
hv-strip-ranges = { $n } ranges · { $cells ->
        [one] { $cells } cell
       *[other] { $cells } cells
    }
# Fix round 1: the notice when Freeze columns is refused because the frozen columns would
# leave too little of the grid to scroll.
hv-grid-freeze-refused = These columns can't be frozen: there would be no room left to scroll. Widen the pane or freeze fewer columns.

## m3b-t36-sort

# The sort note under the result toolbar when the rows were sorted on this computer and the
# database may hold more rows (a partial result): its order might differ from ORDER BY.
hv-grid-sorted-locally = Sorted locally: the database might order differently.
# After hv-grid-sorted-locally on a partial result.
# Variables:
#   $rows (Number) - the fetched rows the sort ordered, e.g. 10,000.
hv-grid-sorted-locally-more = Only the { $rows } fetched rows are sorted.
# A button in the sort note: back to the order the database sent.
hv-grid-clear-sort = Clear sort
# The sort note when rows arrived after the sort of a complete result: they wait, unsorted,
# after a dashed line at the end of the grid.
# Variables:
#   $rows (Number) - the rows that arrived after the sort, e.g. 1,500.
hv-grid-unsorted-tail = { $rows ->
        [one] { $rows } new row at the end (not sorted)
       *[other] { $rows } new rows at the end (not sorted)
    }
# A button in the sort note: sort again, the new rows included.
hv-grid-resort = Re-sort
# The sort note while a long sort runs (shown after 200 ms; the grid keeps its old order
# meanwhile).
# Variables:
#   $rows (Number) - the rows being sorted, e.g. 1,048,576.
hv-grid-sorting = { $rows ->
        [one] Sorting { $rows } row…
       *[other] Sorting { $rows } rows…
    }
# After hv-grid-sorted-locally on a partial result whose new rows arrived after the sort: both
# facts in one note.
# Variables:
#   $sorted (Number) - the fetched rows the sort ordered, e.g. 10,000.
#   $rows (Number) - the rows that arrived after the sort, e.g. 1,500.
hv-grid-sorted-tail = { $rows ->
        [one] Only the { $sorted } fetched rows are sorted; { $rows } new row at the end is not.
       *[other] Only the { $sorted } fetched rows are sorted; { $rows } new rows at the end are not.
    }
# A button in the sort note while a sort runs: stop it and keep the order shown.
hv-grid-sort-cancel = Cancel
# The tooltip of a sorted column's header.
# Variables:
#   $lvl (Number) - the sort level, 1 for the first key, e.g. 1.
#   $dir (String) - hv-grid-dir-asc or hv-grid-dir-desc, e.g. "ascending".
#   $collation (String) - the column's collation as the database names it, e.g. "tr-x-icu".
hv-grid-sort-tip = Sort level { $lvl }, { $dir }. Text compares by the column's collation, { $collation }.
# The tooltip of a sorted text column whose collation this app cannot follow.
# Variables:
#   $lvl (Number) - the sort level, 1 for the first key, e.g. 1.
#   $dir (String) - hv-grid-dir-asc or hv-grid-dir-desc, e.g. "ascending".
#   $collation (String) - the column's collation as the database names it, e.g. "utf8mb4_general_ci".
hv-grid-sort-tip-cp = Sort level { $lvl }, { $dir }. The column's collation { $collation } has no equivalent here, so text compares by code point.
# The sort direction inside the sort tooltips.
hv-grid-dir-asc = ascending
hv-grid-dir-desc = descending
# The tooltip of a sorted text column when the result does not say which collation the column
# uses (the database sends no collation with a result's columns).
# Variables:
#   $lvl (Number) - the sort level, 1 for the first key, e.g. 1.
#   $dir (String) - hv-grid-dir-asc or hv-grid-dir-desc, e.g. "ascending".
hv-grid-sort-tip-unknown = Sort level { $lvl }, { $dir }. The column's collation is not known here, so text compares by code point.
# The tooltip of a sorted column that holds no text (numbers, dates…).
# Variables:
#   $lvl (Number) - the sort level, 1 for the first key, e.g. 1.
#   $dir (String) - hv-grid-dir-asc or hv-grid-dir-desc, e.g. "ascending".
hv-grid-sort-level = Sort level { $lvl }, { $dir }.
# Fix round 1: the notice when a local sort stops on an internal error; the grid keeps the
# order it showed.
hv-grid-sort-failed = The sort stopped on an internal error. The grid keeps the order it showed.

## m3b-t33-find

# The editor find bar's query field: placeholder and accessible name, also the bar's (role=search) name (NOTES surface 16).
hv-find-placeholder = Find
# The find field's toggle: match upper and lower case exactly (tooltip and accessible name).
hv-find-match-case = Match case
# The find field's toggle: match whole words only (SQL word boundaries).
hv-find-words = Whole words
# The find field's toggle: the query is a regular expression.
hv-find-regex = Regular expression
# The find field's toggle: Turkish-tolerant matching (D-074).
hv-find-tolerant = Turkish-tolerant
# The Turkish-tolerant toggle's tooltip and accessible description.
hv-find-tolerant-tip = Turkish-tolerant: İ, I, ı and i find each other, and accents are ignored.
# The find count: the current match and the total.
# Variables:
#   $i (Number) - the current match, 1-based, e.g. 3.
#   $n (Number) - the matches, e.g. 41.
hv-find-count = { $i } of { $n }
# The find count while counting goes on (the static amber marker beside it).
# Variables:
#   $i (Number) - the current match, 1-based, e.g. 3.
#   $n (Number) - the matches counted so far, e.g. 1,240.
hv-find-count-more = { $i } of { $n }+
# The tooltip of a count still running.
hv-find-counting-tip = Still counting; the number grows as the rest of the document is searched.
# The find count when nothing matches.
hv-find-none = No matches
# The find count when the regular expression cannot be read (ink glyph, never red; NOTES M3c-11).
hv-find-invalid = Invalid pattern
# The tooltip of an invalid pattern, with the regular expression engine's message.
# Variables:
#   $error (String) - the engine's message (English, data), e.g. unclosed group.
hv-find-invalid-tip = The regular expression can't be read, so nothing is searched: { $error }
# The find bar's previous-match button (tooltip and accessible name).
hv-find-prev = Previous match
# The find bar's next-match button (tooltip and accessible name).
hv-find-next = Next match
# Said in the status bar and politely to assistive technology when Select all occurrences stops at its cap.
# Variables:
#   $max (Number) - the occurrences selected, e.g. 10,000.
#   $n (Number) - every occurrence, e.g. 41,208.
hv-find-capped = Selected the first { $max } of { $n } matches
# The find count when the caret is not on a match (the total only).
# Variables:
#   $n (Number) - the matches, e.g. 41.
hv-find-total = { $n ->
        [one] { $n } match
       *[other] { $n } matches
    }
# The find count when the caret is not on a match, while counting goes on.
# Variables:
#   $n (Number) - the matches counted so far, e.g. 1,240.
hv-find-total-more = { $n }+ matches
# Said politely when Select next occurrence finds nothing more (every occurrence is selected).
hv-find-no-more = No further occurrence

## m3b-t37-find-goto

# The grid find bar's query field: placeholder and accessible name, also the bar's (role=search) name (NOTES surface 19).
hv-gfind-placeholder = Find in the shown values
# The grid find field's toggle: show only the rows with a match (tooltip and accessible name).
hv-gfind-filter = Filter rows
# The Filter rows toggle's longer tooltip and accessible description.
hv-gfind-filter-tip = Filter rows: show only the rows with a match
# After the grid find count when some searched values were cut at the value-size limit.
hv-gfind-incomplete = may be incomplete
# The tooltip of "may be incomplete".
# Variables:
#   $n (Number) - the values that were cut, e.g. 1.
#   $size (String) - the value-size limit, already formatted with its unit, e.g. 1 MB.
hv-gfind-incomplete-tip = { $n ->
        [one] { $n } value was cut at { $size }; text past the cut isn't searched.
       *[other] { $n } values were cut at { $size }; text past the cut isn't searched.
    }
# The note row under the result toolbar while Filter rows is on (NOTES M3c-26).
# Variables:
#   $shown (Number) - the rows with a match, e.g. 41.
#   $rows (Number) - the rows fetched, e.g. 10,000.
hv-gfind-filter-note = Showing the { $shown } rows that match, of the { $rows } fetched. Rows not fetched yet aren't filtered.
# The filter note's button: turns Filter rows off.
hv-gfind-show-all = Show all rows
# The grid find field's tooltip: what it searches.
hv-gfind-null-tip = Searches what the grid shows, row by row from the focused cell. NULL never matches.
# The grid find bar's close button (tooltip and accessible name).
hv-gfind-close = Close find
# The grid find bar's Select all occurrences button (the command's words without "Grid:").
hv-gfind-select-all = Select all occurrences
# The command palette's title of the grid's Select all occurrences (⌃⌘G in a grid).
hv-cmd-grid-select-all-occurrences = Grid: select all occurrences
# The go-to popover's title (NOTES surface 20).
hv-goto-title = Go to row
# The go-to field's placeholder.
hv-goto-placeholder = row or row:column
# The line under the go-to field before anything is typed.
hv-goto-hint = A row number, then an optional column by name or position: 1500:city or 1500:3
# The line under the go-to field: where Enter goes, with a column.
# Variables:
#   $row (Number) - the row, e.g. 1,000.
#   $col (String) - the column's name (catalog text), e.g. city.
hv-goto-target = Row { $row }, column { $col }
# The line under the go-to field: where Enter goes, a row only.
# Variables:
#   $row (Number) - the row, e.g. 1,000.
hv-goto-target-row = Row { $row }
# The go-to error past the last row.
# Variables:
#   $row (Number) - the row asked for, e.g. 40.
#   $total (Number) - the result's rows, e.g. 12.
hv-goto-past-end = { $total ->
        [one] There's no row { $row }: the result has { $total } row.
       *[other] There's no row { $row }: the result has { $total } rows.
    }
# The go-to error for a column name no column has.
# Variables:
#   $name (String) - what was typed (data), e.g. ilce.
hv-goto-unknown-col = No column is named { $name }.
# The go-to error for a hidden column.
# Variables:
#   $name (String) - the column's name (catalog text), e.g. meta.
hv-goto-hidden-col = Column { $name } is hidden.
# The button after the hidden-column error: shows the column and goes.
hv-goto-show-col = Show it and go
# The go-to question above a million rows to fetch.
# Variables:
#   $row (Number) - the row asked for, e.g. 1,500,000.
#   $more (Number) - the rows still to fetch, e.g. 1,490,000.
hv-goto-fetch = Row { $row } isn't fetched yet. Fetching { $more } more rows takes a while, and the result's memory limit still applies.
# While go to row fetches toward its row.
# Variables:
#   $row (Number) - the row asked for, e.g. 1,500,000.
#   $done (Number) - the rows fetched so far, e.g. 214,000.
#   $more (Number) - the rows to fetch, e.g. 1,490,000.
hv-goto-fetching = Fetching to row { $row }: { $done } of { $more } rows
# Under the fetching line: Escape hides the popover, the fetch goes on.
hv-goto-fetching-note = Escape hides this; fetching goes on in the strip.
# The go-to fetching's button: ends the fetching, the cursor stays open.
hv-goto-stop = Stop fetching
# The go-to question's primary button.
hv-goto-fetch-ok = Fetch and go
# The go-to question's cancel button.
hv-goto-cancel = Cancel
# The go-to key hint's word after the return glyph ("↵ Go").
hv-goto-key-go = Go
# The go-to error when the row part is not a row number (or too large for one).
hv-goto-not-a-row = That isn't a row number.
# The go-to error past the last row while Filter rows shows only the matching rows.
# Variables:
#   $row (Number) - the row asked for, e.g. 40.
#   $total (Number) - the rows the filter shows, e.g. 12.
hv-goto-past-end-filter = { $total ->
        [one] There's no row { $row }: the filter shows { $total } row.
       *[other] There's no row { $row }: the filter shows { $total } rows.
    }
# The go-to line when fetching toward a row ended because the cursor closed (an error, a
# cancel, a lost connection).
# Variables:
#   $row (Number) - the rows fetched, e.g. 214,000.
hv-goto-fetch-closed = Fetching stopped at row { $row }: the cursor is closed.
# The go-to line when fetching toward a row ended at the result's memory limit.
# Variables:
#   $row (Number) - the rows fetched, e.g. 214,000.
hv-goto-fetch-paused = Fetching stopped at row { $row }: the result reached its memory limit.

## m3b-t34-editor

# The gutter's ▶ button on a statement's first row (accessible name) and its tooltip's title (NOTES surface 17).
hv-gutter-run = Run this statement
# The ▶ tooltip's caption above the statement's last three runs.
hv-gutter-last-runs = Last runs
# The ▶ tooltip of a statement that has not run in this editor.
hv-gutter-never-run = Not run yet.
# The ▶ tooltip's last line: the gutter runs through the same resolve, preview and gate as Run.
hv-gutter-same-path = Runs through the same checks as Run.
# The status bar's button when the editor's lines end in more than one way.
hv-eol-mixed = Mixed line endings
# The tooltip of the mixed line endings button: the endings the text has.
# Variables:
#   $mix (String) - the endings' names, data, e.g. LF, CRLF.
hv-eol-mixed-tip = Lines end in more than one way ({ $mix }). Click to normalize them.
# The header of the line endings menu: how many lines end each way.
# Variables:
#   $lf (Number) - the lines that end in LF, e.g. 214.
#   $crlf (Number) - the lines that end in CRLF, e.g. 12.
hv-eol-counts = Line endings: LF { $lf }, CRLF { $crlf }
# A row of the line endings menu: rewrite every line ending as this one (one undo group).
# Variables:
#   $ending (String) - the ending's name, LF or CRLF (data), e.g. LF.
hv-eol-to = Normalize to { $ending }
# The hint beside the dominant ending's row.
hv-eol-most = most lines
# The status bar's sentence after a history recall (⌃⌥↑ / ⌃⌥↓ at the last line of an empty or recalled console).
# Variables:
#   $i (Number) - the recalled statement's place, newest first, e.g. 2.
#   $n (Number) - the statements recalled, e.g. 48.
#   $older (String) - the key for an older statement, e.g. ⌃⌥↑.
#   $newer (String) - the key for a newer statement, e.g. ⌃⌥↓.
hv-history-recall = From history: statement { $i } of { $n } · { $older } older · { $newer } newer
# A statement's end-of-line status after its change was rolled back (Roll back, Undo, the deadline or a close; the gutter's undo glyph).
hv-editor-mark-rolled-back = rolled back
# The end of a selection's first line when Run sends it as one unit (the layers glyph; NOTES surface 9).
# Variables:
#   $n (Number) - the statements the selection holds, e.g. 2.
hv-editor-preview-as-one = { $n ->
        [one] as one · { $n } statement
       *[other] as one · { $n } statements
    }
# The line endings menu's header when some lines end in CR (old Mac files).
# Variables:
#   $lf (Number) - the lines that end in LF, e.g. 214.
#   $crlf (Number) - the lines that end in CRLF, e.g. 12.
#   $cr (Number) - the lines that end in CR, e.g. 3.
hv-eol-counts-cr = Line endings: LF { $lf }, CRLF { $crlf }, CR { $cr }
# The gutter's ▶ button's accessible name: which statement it runs.
# Variables:
#   $line (Number) - the statement's first line, 1-based, e.g. 12.
hv-gutter-run-line = Run the statement at line { $line }

## m3b-t38-views

# The result view switch's segment: the grid as a table (NOTES surface 21).
hv-view-table = Table
# The result view switch's segment: rows as columns.
hv-view-transposed = Transposed
# The result view switch's segment: the result as a text table.
hv-view-text = Text
# The result view switch's accessible name.
hv-view-label = View
# The tooltip of the disabled Transposed segment.
# Variables:
#   $max (Number) - the most rows the Transposed view shows, e.g. 100,000.
#   $rows (Number) - the rows the result has, e.g. 1,048,576.
hv-view-transposed-off = Transposed shows up to { $max } rows; this result has { $rows }.
# The Transposed view's first column: each column's name.
hv-transposed-col = Column
# The record toggle's word (a one-row result shows it) and the inspector's small-caps title.
hv-record = Record
# The record toggle's tooltip and accessible name.
hv-record-tip = Show the focused row as a record
# The record inspector's position.
# Variables:
#   $i (Number) - the row shown (as the grid numbers it), e.g. 4.
#   $n (Number) - the rows the grid shows, e.g. 184.
hv-record-pos = Row { $i } of { $n }
# The record inspector's previous-row button (tooltip and accessible name).
hv-record-prev = Previous row
# The record inspector's next-row button (tooltip and accessible name).
hv-record-next = Next row
# Under a value the record inspector shows in part: opens the value viewer.
hv-record-show-more = Show more
# After a column name the result has more than once.
# Variables:
#   $i (Number) - which of the columns with that name this is, e.g. 2.
hv-record-dup = ({ $i })
# The tooltip of a duplicate column name's suffix.
# Variables:
#   $n (Number) - the columns with that name, e.g. 2.
#   $name (String) - the column name, e.g. "id".
#   $i (Number) - which of them this is, e.g. 2.
hv-record-dup-tip = The result has { $n } columns named { $name }; this is number { $i }.
# The record inspector's accessible name.
hv-record-label = Record of the focused row
# The text view's question above 100,000 rows.
# Variables:
#   $rows (Number) - the rows the result has, e.g. 1,048,576.
hv-text-big-title = { $rows ->
        [one] Show { $rows } row as text?
       *[other] Show { $rows } rows as text?
    }
# Under the text view's question.
# Variables:
#   $size (String) - the memory the text takes (an hv-grid-size-* message), e.g. "180 MB".
hv-text-big-body = Building the text view for this many rows takes a while and uses about { $size } of memory.
# The text view's build in progress.
# Variables:
#   $done (Number) - the rows built so far, e.g. 412,000.
#   $rows (Number) - the rows to build, e.g. 1,048,576.
hv-text-building = Building the text view: { $done } of { $rows } rows
# The text view build's cancel button (back to the table).
hv-text-cancel = Cancel
# The text view question's primary button.
hv-text-big-ok = Show as text
# The text view question's default button.
hv-text-big-back = Back to the table
# The text view's accessible name.
hv-text-label = The result as a text table
# The toast after copying lines of the text view.
# Variables:
#   $n (Number) - the lines copied, e.g. 3.
hv-text-copied = { $n ->
        [one] Copied { $n } line
       *[other] Copied { $n } lines
    }
# The value viewer's subtitle: the column's type and the row.
# Variables:
#   $type (String) - the column's type, e.g. "jsonb".
#   $row (Number) - the row (as the grid numbers it), e.g. 4.
hv-vv-sub = { $type } · row { $row }
# The value viewer's switch: the value as the server sent it.
hv-vv-raw = Raw
# The value viewer's switch: JSON or XML indented.
hv-vv-pretty = Pretty
# The value viewer's single segment for a binary value.
hv-vv-hex = Hex
# The Pretty segment's tooltip.
hv-vv-pretty-tip = Display only: Copy raw value copies the server's text unchanged.
# The value viewer's footer for a text value.
# Variables:
#   $size (String) - its size (an hv-grid-size-* message), e.g. "48 KB".
#   $n (Number) - its characters, e.g. 48,213.
hv-vv-chars = { $n ->
        [one] { $size } · { $n } character
       *[other] { $size } · { $n } characters
    }
# The value viewer's footer for a binary value.
# Variables:
#   $n (Number) - its bytes, e.g. 48,213.
hv-vv-bytes = { $n ->
        [one] { $n } byte
       *[other] { $n } bytes
    }
# The value viewer's notice for a value cut at the value-size limit.
# Variables:
#   $shown (String) - what the result kept (an hv-grid-size-* message), e.g. "1 MB".
#   $size (String) - the value's whole size, e.g. "5.2 MB".
hv-vv-truncated = Showing the first { $shown } of { $size }.
# The value viewer's notice for a value cut at the value-size limit whose whole size is unknown.
# Variables:
#   $shown (String) - what the result kept (an hv-grid-size-* message), e.g. "1 MB".
hv-vv-truncated-unknown = Showing the first { $shown } of the value.
# The value viewer's re-run button.
# Variables:
#   $limit (String) - the raised limit (an hv-grid-size-* message), e.g. "64 MB".
hv-vv-rerun = Re-run with a { $limit } value limit
# The re-run button's tooltip.
hv-vv-rerun-tip = Runs this statement again with a larger limit for every value. It goes through the same checks as Run.
# The value viewer's copy button.
hv-vv-copy = Copy raw value
# Beside the switch when a JSON value does not parse.
hv-vv-not-json = Not valid JSON, so Pretty is off and the value shows as the server sent it.
# The value viewer's accessible name.
hv-vv-label = Value viewer
# The toast after Copy raw value.
hv-vv-copied = Copied the value
# The toast after Copy raw value of a value cut at the value-size limit.
# Variables:
#   $shown (String) - what was copied (an hv-grid-size-* message), e.g. "1 MB".
hv-vv-copied-cut = Copied the first { $shown } of the value: the rest wasn't fetched.
# The value viewer's line (and the console's log line and gutter) when its re-run was refused:
# only a read runs again with a raised value limit (fix round 1, controller ruling).
hv-vv-rerun-not-read = Only a query that reads data is run again with a larger value limit. Run this statement again from the editor.
# The re-run button's tooltip while the console runs a statement.
hv-vv-rerun-busy = A statement is running in this console. Re-run once it ends.

## m3b-t42-settings

# The settings navigation's four M3b sections (NOTES surface 25, ruling M3c-37).
hv-settings-nav-editor = Editor
hv-settings-nav-execution = Execution
hv-settings-nav-results = Results
hv-settings-nav-history = History
# The headings of those sections, pre-cased for small caps.
hv-settings-section-editor = EDITOR
hv-settings-section-execution = EXECUTION
hv-settings-section-results = RESULTS
hv-settings-section-history = HISTORY
# The two choices of an on/off setting.
hv-settings-on = On
hv-settings-off = Off
# The choice that decides by itself (the Turkish-tolerant find: on for Turkish users).
hv-settings-auto = Auto
# Setting titles (the Editor section).
hv-settings-tab-size = Tab size
hv-settings-insert-spaces = Indent with spaces
hv-settings-copy-line = Copy the line when nothing is selected
hv-settings-caret-blink = Blinking caret
hv-settings-find-tolerant = Turkish-tolerant find
# The note under the Turkish-tolerant find row.
hv-settings-find-tolerant-note = Auto: on when the language or the regional format is Turkish. The find bar's toggle changes this setting too.
# Setting titles and notes (the Execution section).
hv-settings-confirm-unsafe-where = Confirm UPDATE or DELETE without WHERE off production
hv-settings-confirm-unsafe-where-note = Production always asks, with the table's name typed.
hv-settings-error-policy = Ask what to do when a statement fails
hv-settings-error-policy-note = Off: a run of several statements stops at the first failure.
# Setting title and note (the Results section): the most bytes a cell keeps of one value.
hv-settings-max-value = Largest value kept per cell
hv-settings-max-value-note = From 64 KB to 64 MB. Larger values are cut; the value viewer can re-run with the largest limit.
# The number in the largest-value field, in the regional format (the unit is the select beside it).
# Variables:
#   $n (Decimal) - the size in the chosen unit, e.g. 0.5.
hv-settings-max-value-amount = { $n }
# Under the largest-value field after a size above the largest allowed was typed (the info glyph).
# Variables:
#   $typed (String) - the size typed (an exec-size-* message), e.g. "128 MB".
#   $max (String) - the largest size allowed (an exec-size-* message), e.g. "64 MB".
hv-settings-max-value-clamped = { $typed } is above the largest allowed, so it was set to { $max }.
# Under the largest-value field after a size below the smallest allowed was typed (the info glyph).
# Variables:
#   $typed (String) - the size typed (an exec-size-* message), e.g. "10 KB".
#   $min (String) - the smallest size allowed (an exec-size-* message), e.g. "64 KB".
hv-settings-max-value-clamped-low = { $typed } is below the smallest allowed, so it was set to { $min }.
# The two units of the largest-value select.
hv-settings-unit-kb = KB
hv-settings-unit-mb = MB
hv-settings-raw-display = Show grid values raw
hv-settings-raw-display-note = Numbers and dates as the server sent them, without regional formatting. Copy is always raw.
# Setting title and note (the History section).
hv-settings-history = Record query history
hv-settings-history-note = Up to 10,000 statements per connection are kept for 90 days; recognised passwords and secrets are masked.

## m3c-int-fixes

# The go-to line when fetching toward a row (or its question before a large fetch) ended
# because the result was run again: no row of the new result is fetched or gone to.
hv-goto-fetch-rerun = Fetching stopped: the result was run again.

## m3b-t41-chrome

# The transaction mode chip's word for Auto-commit in a narrow pane (design/m3b-safe surface 24, N4): the chip keeps its mode word.
hv-tx-chip-auto-short = Auto
# The waiting item's first line in a narrow pane (NOTES surface 24): the sentence keeps all its words on two lines.
hv-console-waiting-l1 = Waiting for a connection
# The waiting item's second line in a narrow pane.
# Variables:
#   $inUse (Number) - connections in use, e.g. 4.
#   $max (Number) - connections the profile allows, e.g. 4.
hv-console-waiting-l2 = { $inUse } of { $max } in use
# The tooltip and accessible name of an icon-only toolbar button in a narrow pane (N1): its word and its keys, e.g. "Run · ⌘↵".
# Variables:
#   $label (String) - the button's word, e.g. Run.
#   $keys (String) - its shortcut, e.g. ⌘↵.
hv-console-button-keys = { $label } · { $keys }
# The first line of an empty console, in text-3 (design/m3a-query surface 1; m3b-safe surface 24: it wraps in a narrow pane).
# Variables:
#   $key (String) - the Run shortcut, e.g. ⌘↵.
hv-editor-placeholder = Write SQL here. { $key } runs the statement at the caret.

## m3b-t39-grid-menu

# The grid's context menu (right press on a cell, a column header or a row number; Shift+F10).
# Copies the selected cells.
hv-gmenu-copy = Copy
# Copies the selected cells with the column names as the first line.
hv-gmenu-copy-headers = Copy with headers
# The row-number menu's Copy: copies the selected rows.
hv-gmenu-copy-rows = Copy rows
# The header menu: copies the column's name.
hv-gmenu-copy-name = Copy column name
# Opens the value viewer on the cell.
hv-gmenu-open-value = Open value viewer
# Sorts the rows by the column, smallest first.
hv-gmenu-sort-asc = Sort ascending
# Sorts the rows by the column, largest first.
hv-gmenu-sort-desc = Sort descending
# The header menu, while another column is sorted: sorts by this column too, after the others.
# Variables:
#   $lvl (Number) - the level this column would take, e.g. 2.
hv-gmenu-add-sort = Add as sort level { $lvl }
# The header menu: keeps the columns up to this one in place when the grid scrolls sideways.
hv-gmenu-freeze = Freeze up to here
# The header menu while columns are frozen.
hv-gmenu-unfreeze = Unfreeze columns
# The header menu: hides the column from the grid (Show all columns brings it back).
hv-gmenu-hide = Hide column
# The header menu: shows every hidden column again.
hv-gmenu-show-all = Show all columns
# The header menu: sizes the column to its values.
hv-gmenu-autofit = Fit width
# A check item: shows the column's values as the server sent them (no digit grouping).
hv-gmenu-raw = Raw display
# The hint after hv-gmenu-raw: the item applies to this column only (the setting applies to
# every grid).
hv-gmenu-raw-hint = this column
# Opens the go-to-row field.
hv-gmenu-goto = Go to row…
# The row-number menu: opens the record inspector on the row.
hv-gmenu-record = Show as record
# Why Open value viewer is unavailable: several cells are selected.
hv-gmenu-why-one-cell = Select one cell
# Why Clear sort is unavailable.
hv-gmenu-why-not-sorted = Not sorted
# Why Show all columns is unavailable: no column is hidden.
hv-gmenu-why-no-hidden = None hidden
# Why Copy is unavailable: the selected ranges share neither their rows nor their columns.
hv-gmenu-why-ranges = Ranges don't line up
# Why Hide column is unavailable: it is the only column still shown.
hv-gmenu-why-last = Last shown column
# The menu's name for screen readers.
hv-gmenu-label = Grid menu

## m3c-po

# The command (palette; macOS ⌘E) that makes the selection the find query without opening the
# find bar (PO M3c-08; macOS "Use Selection for Find").
hv-cmd-editor-use-selection-for-find = Use selection for find

## m4-t01b-ui

# Surface 1 (Explorer tree).
hv-explorer-tree-label = Database explorer
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-tables = TABLES
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-views = VIEWS
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-matviews = MATERIALIZED VIEWS
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-functions = FUNCTIONS
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-procedures = PROCEDURES
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-sequences = SEQUENCES
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-types = TYPES
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-events = EVENTS
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-columns = COLUMNS
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-keys = KEYS
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-foreign-keys = FOREIGN KEYS
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-indexes = INDEXES
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-checks = CHECKS
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
hv-explorer-kind-triggers = TRIGGERS
# Surface 1 (Explorer tree). The Built-in group of a database.
hv-explorer-builtin = Built-in
# Surface 1 (Explorer tree). The Built-in group of a database.
# Variables:
#   $version (String) - the bundled catalog's server version, e.g. "17".
hv-explorer-builtin-badge = version { $version }
# Surface 1 (Explorer tree). The Built-in group of a database.
# Variables:
#   $version (String) - the bundled catalog's server version, e.g. "17".
hv-explorer-builtin-tip = Built-in catalog (version { $version }): the server's own objects, shipped with Hacivat and read without a connection.
# Surface 1 (Explorer tree). The database row's schema scope count (N of M).
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $total (Number) - the total, e.g. 15000.
hv-explorer-scope-count = { $n } of { $total }
# Surface 1 (Explorer tree). The database row's schema scope count (N of M).
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $total (Number) - the total, e.g. 15000.
hv-explorer-scope-count-tip = { $total ->
        [one] { $n } of { $total } schema is in scope. Edit scope… changes which ones are read.
       *[other] { $n } of { $total } schemas are in scope. Edit scope… changes which ones are read.
    }
# Surface 1 (Explorer tree).
hv-explorer-add-scope = Add to scope
# Surface 1 (Explorer tree).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-explorer-out-of-scope-tip = { $name } isn't in this connection's scope, so it isn't read.
# Surface 1 (Explorer tree).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-explorer-out-of-scope-db-tip = { $name } isn't in this connection's scope, so it isn't read. Reading it opens one more connection.
# Surface 1 (Explorer tree).
# Variables:
#   $rows (Number) - the server's row estimate, e.g. 48213.
hv-explorer-row-estimate-tip = { $rows ->
        [one] About { $rows } row (the server's estimate)
       *[other] About { $rows } rows (the server's estimate)
    }
# Surface 1 (Explorer tree).
hv-explorer-loads-on-open = Loads when you open it
# Surface 2 (Loading states).
hv-explorer-loading = Loading…
# Surface 2 (Loading states). A phase row's label; the count follows in mono (hv-explorer-progress-count).
hv-explorer-phase-names = Reading names…
# Surface 2 (Loading states). A phase row's label; the count follows in mono (hv-explorer-progress-count).
hv-explorer-phase-columns = Reading columns…
# Surface 2 (Loading states). A phase row's label; the count follows in mono (hv-explorer-progress-count).
hv-explorer-phase-constraints = Reading keys and indexes…
# Surface 2 (Loading states). A phase row's label; the count follows in mono (hv-explorer-progress-count).
hv-explorer-phase-sources = Reading definitions…
# Surface 2 (Loading states). Progress of a pass in the explorer (surface 14: explorer only).
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $total (Number) - the total, e.g. 15000.
hv-explorer-progress-count = { $n }/{ $total }
# Surface 2 (Loading states).
hv-explorer-waiting-pass = Waiting for columns
# Surface 2 (Loading states). The partial-load badge and its tooltip.
hv-explorer-partial = partial
# Surface 2 (Loading states). The partial-load badge and its tooltip.
hv-explorer-partial-title = Some objects could not be read
# Surface 2 (Loading states). The partial-load badge and its tooltip.
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
hv-explorer-partial-body = { $n ->
        [one] This login can't read the definition of { $n } function in { $schema }. It is listed by name; its definition is missing.
       *[other] This login can't read the definitions of { $n } functions in { $schema }. They are listed by name; their definitions are missing.
    }
# Surface 2 (Loading states). A namespace whose catalog query was refused.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-explorer-failed = Couldn't read { $name }
# Surface 2 (Loading states). A namespace whose catalog query was refused.
hv-explorer-failed-body = The server refused the catalog query. Retry reads it again.
# Surface 2 (Loading states).
hv-explorer-retry = Retry
# Surface 3 (Cached and offline). A profile shown from its cache while offline.
# Variables:
#   $date (DateTime) - when the cache was saved, e.g. 2026-11-17 14:05.
hv-explorer-cached = Showing cached metadata from { DATETIME($date, dateStyle: "medium", timeStyle: "short") }
# Surface 3 (Cached and offline). A profile shown from its cache while offline.
hv-explorer-cached-meta = cached
# Surface 3 (Cached and offline). A profile shown from its cache while offline.
# Variables:
#   $date (DateTime) - when the cache was saved, e.g. 2026-11-17 14:05.
hv-explorer-cached-tip = Not connected. The tree is the copy saved on { DATETIME($date, dateStyle: "medium", timeStyle: "short") }; connect to check it against the server.
# Surface 3 (Cached and offline).
hv-explorer-stale = out of date
# Surface 3 (Cached and offline).
# Variables:
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
hv-explorer-stale-tip = The server changed { $schema } after it was read. Refresh to load the changes.
# Surface 3 (Cached and offline).
hv-explorer-encrypted-tip = The cached metadata is encrypted. Opening this connection reads its key from the Keychain.
# Surface 3 (Cached and offline).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-explorer-memory-only = The Keychain isn't available, so { $name }'s metadata is kept in memory until you quit.
# Surface 4 (Filter field). The explorer filter field.
hv-explorer-filter-objects = Filter objects
# Surface 4 (Filter field). The explorer filter field.
# Variables:
#   $n (Number) - a count, e.g. 3400.
hv-explorer-filter-count = { $n ->
        [one] { $n } match
       *[other] { $n } matches
    }
# Surface 4 (Filter field). The explorer filter field.
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $total (Number) - the total, e.g. 15000.
hv-explorer-filter-group-count = { $n } of { $total }
# Surface 4 (Filter field). The explorer filter field.
# Variables:
#   $n (Number) - a count, e.g. 3400.
hv-explorer-filter-unloaded = { $n ->
        [one] Searches loaded names only. The columns of { $n } table aren't loaded yet.
       *[other] Searches loaded names only. The columns of { $n } tables aren't loaded yet.
    }
# Surface 4 (Filter field). The explorer filter field.
hv-explorer-filter-none = No objects match
# Surface 4 (Filter field). The explorer filter field.
# Variables:
#   $query (String) - the filter text as typed, e.g. "müş".
hv-explorer-filter-none-body = Nothing loaded matches “{ $query }”.
# Surface 4 (Filter field). The explorer filter field.
hv-explorer-filter-clear = Clear filter
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-refresh = Refresh
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-edit-scope = Edit scope…
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-level = Introspection level
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-clear-cache = Clear cached metadata…
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-open-data = Open data
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-view-ddl = View DDL
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-view-table-ddl = View table DDL
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-copy-name = Copy qualified name
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-copy-plain = Copy name
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-hide-like = Hide objects like this
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-show-hidden = Show hidden objects
# Surface 5 (Context menus).
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $pattern (String) - the view filter's pattern, e.g. "yedek_fatura_*".
hv-explorer-hidden-tip = { $n ->
        [one] { $n } object is hidden by a view filter ({ $pattern }). Show hidden objects, in this group's menu, removes it.
       *[other] { $n } objects are hidden by a view filter ({ $pattern }). Show hidden objects, in this group's menu, removes it.
    }
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-copy-ddl = Copy DDL
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-modify-table = Modify table…
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-modify-view = Modify view…
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-modify-column = Modify column…
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-modify-tip = Arrives in a later version
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-remove-scope = Remove from scope
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-expand-all = Expand all children
# Surface 5 (Context menus). The tree-wide command (palette, ⌘←); node menus use hv-explorer-menu-collapse-children.
hv-explorer-menu-collapse-all = Collapse all
# Surface 5 (Context menus).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-explorer-copied = Copied { $name }
# Surface 5 (Context menus).
# Variables:
#   $conn (String) - the connection name, e.g. "orders-dev".
hv-explorer-open-data-note = Opens a console on { $conn } and runs:
# Surface 5 (Context menus). An explorer context-menu item.
hv-explorer-menu-collapse-children = Collapse all children
# Surface 6 (Introspection level). The introspection level list (menu and pill).
hv-explorer-level-auto = Automatic
# Surface 6 (Introspection level). The introspection level list (menu and pill).
hv-explorer-level-names = Names only
# Surface 6 (Introspection level). The introspection level list (menu and pill).
hv-explorer-level-structure = Structure
# Surface 6 (Introspection level). The introspection level list (menu and pill).
hv-explorer-level-sources = Structure and sources
# Surface 6 (Introspection level). The introspection level list (menu and pill).
# Variables:
#   $level (String) - the level code, a selector: "names", "structure" or "sources".
hv-explorer-level-auto-hint = { $level ->
        [names] now Names only
        [sources] now Structure and sources
       *[structure] now Structure
    }
# Surface 6 (Introspection level). The introspection level list (menu and pill).
hv-explorer-level-names-hint = fastest, no columns
# Surface 6 (Introspection level). The introspection level list (menu and pill).
hv-explorer-level-structure-hint = columns, keys, indexes
# Surface 6 (Introspection level). The introspection level list (menu and pill).
hv-explorer-level-sources-hint = and view and function text
# Surface 6 (Introspection level). The level pill's word.
hv-explorer-pill-names = Names
# Surface 6 (Introspection level). The level pill's word.
hv-explorer-pill-structure = Structure
# Surface 6 (Introspection level). The level pill's word.
hv-explorer-pill-sources = Sources
# Surface 6 (Introspection level). The level pill's tooltip and accessible name: one whole sentence per level.
# Variables:
#   $level (String) - the level code, a selector: "names", "structure" or "sources".
hv-explorer-pill-tip-explicit = { $level ->
        [names] Introspection level: Names only, set here. Schemas below inherit it.
        [sources] Introspection level: Structure and sources, set here. Schemas below inherit it.
       *[structure] Introspection level: Structure, set here. Schemas below inherit it.
    }
# Surface 6 (Introspection level). The level pill's tooltip and accessible name: one whole sentence per level.
# Variables:
#   $parent (String) - the database or schema the level comes from, e.g. "orders".
#   $level (String) - the level code, a selector: "names", "structure" or "sources".
hv-explorer-pill-tip-inherited = { $level ->
        [names] Introspection level: Names only, inherited from { $parent }.
        [sources] Introspection level: Structure and sources, inherited from { $parent }.
       *[structure] Introspection level: Structure, inherited from { $parent }.
    }
# Surface 6 (Introspection level). The level pill's tooltip and accessible name: one whole sentence per level.
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $level (String) - the level code, a selector: "names", "structure" or "sources".
hv-explorer-pill-tip-auto = { $level ->
        [names] Introspection level: Names only, chosen automatically for { $n ->
                [one] { $n } object
               *[other] { $n } objects
            }.
        [sources] Introspection level: Structure and sources, chosen automatically for { $n ->
                [one] { $n } object
               *[other] { $n } objects
            }.
       *[structure] Introspection level: Structure, chosen automatically for { $n ->
                [one] { $n } object
               *[other] { $n } objects
            }.
    }
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-section = Schemas
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $total (Number) - the total, e.g. 15000.
hv-scope-summary = { $total ->
        [one] { $n } of { $total } schema
       *[other] { $n } of { $total } schemas
    }
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $total (Number) - the total, e.g. 15000.
hv-scope-summary-mysql = { $total ->
        [one] { $total } database
       *[other] All { $total } databases
    }
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-current-db = current database
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-other-db-hint = needs its own connection
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-pattern = Pattern
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-pattern-hint = @ is the current database or schema, * is all, {"{"}…{"}"} holds a regular expression, | separates clauses.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $pos (Number) - the 1-based character position of the error, e.g. 20.
hv-scope-pattern-error = Expected a schema name after “:” (character { NUMBER($pos, useGrouping: "false") }).
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-builtin-note = Built-in schemas come from the bundled catalog and are not listed here.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-mysql-note = Every database you can see is listed. Each one loads the first time you open it.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-all-dbs = All databases
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-all-dbs-title = Read every database on this server?
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $n (Number) - a count, e.g. 3400.
hv-scope-all-dbs-body = { $n ->
        [one] This server has { $n } database. Reading it opens one more connection.
       *[other] This server has { $n } databases. Reading them opens one connection per database and can take a while on a large server.
    }
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $n (Number) - a count, e.g. 3400.
hv-scope-all-dbs-yes = { $n ->
        [one] Read { $n } database
       *[other] Read all { $n } databases
    }
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-all-dbs-no = Keep the current database
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-encrypt = Encrypt cached metadata
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-encrypt-sub = The saved catalog is encrypted with a key kept in the Keychain. On by default for production.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-scope-encrypt-confirm-on = Turning this on deletes the cached metadata of { $name }. It is read again, encrypted, on the next connect.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-scope-encrypt-confirm-off = Turning this off deletes the encrypted cache of { $name }. From the next connect it is saved as a plain file only your account can read.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-encrypt-apply = Delete the cache and encrypt
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-encrypt-apply-off = Delete the cache and stop encrypting
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-encrypt-keep = Keep it as it is
# Surface 7 (Scope editor). The connection dialog's Schemas section.
hv-scope-connect-to-list = Connect to list the schemas. The pattern below still decides what is read.
# Surface 8 (Mass-deletion confirmation). The mass-deletion confirmation (D-039).
# Variables:
#   $count (Number) - a count of objects, e.g. 312.
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
hv-metadata-mass-title = { $count ->
        [one] { $count } object in { $schema } is no longer visible
       *[other] { $count } objects in { $schema } are no longer visible
    }
# Surface 8 (Mass-deletion confirmation). The mass-deletion confirmation (D-039).
hv-metadata-mass-body = They may have been dropped, or this login may have lost access to them. Removing them from the cache changes nothing on the server.
# Surface 8 (Mass-deletion confirmation). The mass-deletion confirmation (D-039).
# Variables:
#   $n (Number) - a count, e.g. 3400.
hv-metadata-mass-more = and { $n } more
# Surface 8 (Mass-deletion confirmation). The mass-deletion confirmation (D-039).
hv-metadata-mass-keep = Keep cached
# Surface 8 (Mass-deletion confirmation). The mass-deletion confirmation (D-039).
hv-metadata-mass-remove = Remove from cache
# Surface 8 (Mass-deletion confirmation). The mass-deletion confirmation (D-039).
# Variables:
#   $count (Number) - a count of objects, e.g. 312.
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
hv-metadata-mass-pending-tip = { $count ->
        [one] { $count } object disappeared in the last background check. Refresh { $schema } to review it; nothing was removed.
       *[other] { $count } objects disappeared in the last background check. Refresh { $schema } to review them; nothing was removed.
    }
# Surface 8 (Mass-deletion confirmation).
# Variables:
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
hv-metadata-ns-dropped = { $schema } is no longer on the server, so it was removed from the explorer.
# Surface 9 (Refresh after DDL). Refresh after DDL.
# Variables:
#   $count (Number) - a count of objects, e.g. 312.
hv-metadata-refresh-ask = { $count ->
        [one] Refresh the database explorer to reflect { $count } changed object?
       *[other] Refresh the database explorer to reflect { $count } changed objects?
    }
# Surface 9 (Refresh after DDL). Refresh after DDL.
# Variables:
#   $console (String) - the console's title, e.g. "orders-dev".
hv-metadata-refresh-ask-body = Your script in { $console } changed them.
# Surface 9 (Refresh after DDL). Refresh after DDL.
hv-metadata-refresh = Refresh
# Surface 9 (Refresh after DDL). Refresh after DDL.
hv-metadata-refresh-always = Always refresh
# Surface 9 (Refresh after DDL). Refresh after DDL.
hv-metadata-refresh-not-now = Not now
# Surface 9 (Refresh after DDL). Refresh after DDL inside a transaction (the pending stamp).
hv-metadata-after-commit = Refreshes after COMMIT
# Surface 9 (Refresh after DDL). Refresh after DDL inside a transaction (the pending stamp).
# Variables:
#   $count (Number) - a count of objects, e.g. 312.
hv-metadata-after-commit-tip = { $count ->
        [one] { $count } change from this transaction waits for COMMIT. ROLLBACK discards it and the explorer stays as it is.
       *[other] { $count } changes from this transaction wait for COMMIT. ROLLBACK discards them and the explorer stays as it is.
    }
# Surface 9 (Refresh after DDL). Refresh after DDL: dynamic SQL.
# Variables:
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
hv-metadata-broad = Refreshing all of { $schema }
# Surface 9 (Refresh after DDL). Refresh after DDL: dynamic SQL.
# Variables:
#   $what (String) - the SQL keyword that made the change opaque, e.g. "EXECUTE".
hv-metadata-broad-body = The script ran dynamic SQL ({ $what }), so Hacivat can't tell which objects changed.
# Surface 9 (Refresh after DDL). Refresh after DDL.
# Variables:
#   $count (Number) - a count of objects, e.g. 312.
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
hv-metadata-refreshed = { $count ->
        [one] Explorer refreshed { $count } object in { $schema }
       *[other] Explorer refreshed { $count } objects in { $schema }
    }
# Surface 9 (Refresh after DDL).
# Variables:
#   $count (Number) - a count of objects, e.g. 312.
hv-metadata-log-pending = { $count ->
        [one] Explorer refresh waits for COMMIT ({ $count } object)
       *[other] Explorer refresh waits for COMMIT ({ $count } objects)
    }
# Surface 10 (DDL view). The DDL view tab.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-ddl-view-tab-title = { $name } · DDL
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-qualification = Qualification
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-qual-auto = Auto
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-qual-never = Never
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-qual-always = Always
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-constraints = Constraints
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-constraints-inside-column = Inside column
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-constraints-inside-table = Inside table
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-constraints-after-table = After table
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-server = Definitions provided by server
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-grants = Grants
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-owner = Owner
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-options = Options
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-copy = Copy
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-open-console = Open in console
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-refresh = Refresh
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-source-generated = Generated from the catalog
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-source-server = Text from the server
# Surface 10 (DDL view). The DDL view tab.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-ddl-view-copied = Copied the DDL of { $name }
# Surface 10 (DDL view). One whole sentence per kind ($kind selector); never a kind word inserted.
# Variables:
#   $kind (String) - the object kind code, a selector, e.g. "index".
hv-ddl-view-unsupported = { $kind ->
        [domain] Hacivat can't show DDL for a domain yet.
        [extension] Hacivat can't show DDL for an extension yet.
        [operator] Hacivat can't show DDL for an operator yet.
        [policy] Hacivat can't show DDL for a row security policy yet.
        [rule] Hacivat can't show DDL for a rule yet.
        [type] Hacivat can't show DDL for a type yet.
       *[other] Hacivat can't show DDL for this kind of object yet.
    }
# Surface 10 (DDL view). One whole sentence per kind ($kind selector); never a kind word inserted.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-ddl-view-unsupported-body = { $name } stays in the explorer, and Copy qualified name works.
# Surface 10 (DDL view). The DDL view tab.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-ddl-view-loading = Reading the definition of { $name }…
# Surface 10 (DDL view). The DDL view tab.
hv-ddl-view-grants-partial = Some grants could not be read, so the list may be incomplete.
# Surface 11 (Stale object). The DDL view tab.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-ddl-view-stale = { $name } changed on the server since it was loaded
# Surface 11 (Stale object). The DDL view tab.
hv-ddl-view-reload = Reload
# Surface 11 (Stale object). The DDL view tab.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-ddl-view-gone = { $name } no longer exists on the server
# Surface 11 (Stale object). The DDL view tab.
hv-ddl-view-gone-body = The text below is the last one loaded.
# Surface 11 (Stale object). The DDL view tab.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-ddl-view-unknown = Hacivat couldn't check whether { $name } changed on the server since it was loaded
# Surface 12 (Invisible characters and broken objects). The invisible-characters badge and its tooltip.
hv-explorer-invisible = invisible characters
# Surface 12 (Invisible characters and broken objects). The invisible-characters badge and its tooltip.
# Variables:
#   $what (String) - the SQL keyword that made the change opaque, e.g. "EXECUTE".
hv-explorer-invisible-tip = This name contains an invisible character: { $what }. Copy and DDL keep it exactly.
# Surface 12 (Invisible characters and broken objects). One whole sentence per kind ($kind selector); the name stays an isolated argument.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
#   $kind (String) - the object kind code, a selector, e.g. "index".
hv-explorer-broken = { $kind ->
        [check] Check constraint { $name } references a column that could not be resolved
        [foreign-key] Foreign key { $name } references a column that could not be resolved
        [index] Index { $name } references a column that could not be resolved
        [trigger] Trigger { $name } references a column that could not be resolved
       *[other] { $name } references a column that could not be resolved
    }
# Surface 12 (Invisible characters and broken objects). One whole sentence per kind ($kind selector); the name stays an isolated argument.
hv-explorer-broken-body = The column may have been dropped or renamed after the last read. Refresh the table to check.
# Surface 13 (Settings › Metadata).
hv-settings-nav-metadata = Metadata
# Surface 13 (Settings › Metadata).
hv-settings-section-metadata = METADATA
# Surface 13 (Settings › Metadata). Settings › Metadata row.
hv-metadata-set-refresh = Refresh the explorer after DDL
# Surface 13 (Settings › Metadata). Settings › Metadata row.
hv-metadata-set-always = Always
# Surface 13 (Settings › Metadata). Settings › Metadata row.
hv-metadata-set-ask = Ask
# Surface 13 (Settings › Metadata). Settings › Metadata row.
hv-metadata-set-never = Never
# Surface 13 (Settings › Metadata). Settings › Metadata row.
hv-metadata-set-refresh-note = After a console runs a statement that changes the schema. Inside a transaction it waits for COMMIT.
# Surface 13 (Settings › Metadata). Settings › Metadata row.
hv-metadata-set-check = Check for changes in the background
# Surface 13 (Settings › Metadata).
hv-metadata-check-auto = Automatic
# Surface 13 (Settings › Metadata). Settings › Metadata row.
hv-metadata-set-check-note = Automatic checks local and dev connections every 10 minutes, and the others when you connect or press Refresh.
# Surface 13 (Settings › Metadata).
# Variables:
#   $n (Number) - a count, e.g. 3400.
hv-metadata-every-min = Every { $n } min
# Surface 13 (Settings › Metadata). Settings › Metadata row.
hv-metadata-set-cache-conn = Cache per connection
# Surface 13 (Settings › Metadata). Settings › Metadata row.
hv-metadata-set-cache-total = Cache for all connections
# Surface 13 (Settings › Metadata). Settings › Metadata row.
hv-metadata-set-cache-note = Past the limit, the schemas loaded longest ago leave first. The current database always stays.
# Surface 13 (Settings › Metadata).
hv-metadata-unit-mb = MB
# Surface 13 (Settings › Metadata).
hv-metadata-unit-gb = GB
# Surface 13 (Settings › Metadata). Settings › Metadata row.
hv-metadata-set-clear = Cached metadata
# Surface 13 (Settings › Metadata). Settings › Metadata: clearing a connection's cached metadata.
hv-metadata-clear = Clear cached metadata…
# Surface 13 (Settings › Metadata). Settings › Metadata: clearing a connection's cached metadata.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
#   $size (String) - a size (an hv-metadata-size-mb message), e.g. "38.4 MB".
hv-metadata-clear-note = { $name } is the active console's connection. Its cached metadata uses { $size } on this computer.
# Surface 13 (Settings › Metadata). Settings › Metadata: clearing a connection's cached metadata.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-metadata-clear-title = Clear cached metadata for { $name }?
# Surface 13 (Settings › Metadata). Settings › Metadata: clearing a connection's cached metadata.
hv-metadata-clear-body = The explorer reads it again from the server on the next connect. Nothing on the server changes.
# Surface 13 (Settings › Metadata). Settings › Metadata: clearing a connection's cached metadata.
hv-metadata-clear-btn = Clear
# Surface 13 (Settings › Metadata). Settings › Metadata: clearing a connection's cached metadata.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-metadata-cleared = Cleared the cached metadata of { $name }
# Surface 13 (Settings › Metadata).
# Variables:
#   $n (Number) - a count, e.g. 3400.
hv-metadata-size-mb = { $n } MB
# Surface 14 (Long introspection). Progress of a pass in the explorer (surface 14: explorer only).
# Variables:
#   $fraction (Number) - the share read so far, from 0 to 1, e.g. 0.23.
hv-explorer-progress-meta = { NUMBER($fraction, style: "percent") }
# Surface 14 (Long introspection). Progress of a pass in the explorer (surface 14: explorer only).
# Variables:
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
#   $n (Number) - a count, e.g. 3400.
#   $total (Number) - the total, e.g. 15000.
hv-explorer-progress-tip = { $total ->
        [one] Reading { $schema }: the columns of { $n } of { $total } table so far. The explorer stays usable meanwhile.
       *[other] Reading { $schema }: the columns of { $n } of { $total } tables so far. The explorer stays usable meanwhile.
    }

# M4 command titles (palette, menus, keymap; `hv-cmd-<action id with . and _ as ->`).
hv-cmd-explorer-view-ddl = View DDL
hv-cmd-explorer-copy-qualified-name = Copy qualified name
hv-cmd-explorer-collapse-all = Collapse all in the explorer
hv-cmd-explorer-open-data = Open data
hv-cmd-explorer-refresh = Refresh metadata
hv-cmd-explorer-expand-all-children = Expand all children
hv-cmd-explorer-edit-scope = Edit scope
hv-cmd-explorer-set-level-auto = Introspection level: Automatic
hv-cmd-explorer-set-level-names = Introspection level: Names only
hv-cmd-explorer-set-level-structure = Introspection level: Structure
hv-cmd-explorer-set-level-sources = Introspection level: Structure and sources
hv-cmd-metadata-clear-cache = Clear cached metadata
hv-cmd-ddl-view-copy = Copy DDL

## m4-t14-explorer

# Surface 1 (Explorer tree). The child row of a node whose catalog has not been read yet (no
# cached or live catalog): never an empty database.
hv-explorer-not-loaded = Not loaded
# Surface 1 (Explorer tree). A count shown alone: a kind group's or an environment group's.
# Variables:
#   $n (Number) - a count, e.g. 1204.
hv-explorer-count = { $n }
# Surface 1 (Explorer tree). A foreign key's type text after its name (mono): the table and
# columns it references.
# Variables:
#   $target (String) - the referenced table, verbatim, e.g. "İSTANBUL_ŞUBE".
#   $columns (String) - the referenced columns, verbatim, e.g. "id".
hv-explorer-fk-target = → { $target } ({ $columns })
# Surface 1 (Explorer tree). The accessible word of a database or schema row outside the
# connection's scope (the row is drawn dim).
hv-explorer-out-of-scope = out of scope
# Surface 4 (Filter field). Under the field while connections, databases or schemas have no
# loaded catalog: their names could not be searched.
# Variables:
#   $n (Number) - a count, e.g. 3.
hv-explorer-filter-not-loaded = { $n ->
        [one] Searches loaded names only. { $n } connection or schema isn't loaded yet.
       *[other] Searches loaded names only. { $n } connections or schemas aren't loaded yet.
    }

## m4-t20-scope

# Surface 7 (Scope editor). The folded Schemas section's summary while no schema list was read
# (not connected, nothing cached): the pattern that decides what is read.
# Variables:
#   $pattern (String) - the scope pattern as typed, e.g. "@:*".
hv-scope-summary-pattern = Pattern { $pattern }
# Surface 7 (Scope editor). Said by assistive technology for a database whose schemas are
# partly checked (the box's mixed state).
hv-scope-tree-mixed = partly selected
# Surface 7 (Scope editor). Asked in place when a click in the list cannot be made without
# writing the pattern again from the checked names (fix round 1, I-3).
hv-scope-rewrite-title = Write the pattern again from the checked names?
# Surface 7 (Scope editor).
hv-scope-rewrite-body = Parts of it that the list can't show (a regular expression, an exclusion, a name that isn't listed) would be removed.
# Surface 7 (Scope editor). The safe answer (focused).
hv-scope-rewrite-keep = Keep the pattern
# Surface 7 (Scope editor).
hv-scope-rewrite-apply = Write it again
# Surface 7 (Scope editor). The saved scope could not be read (the connection's data file).
hv-scope-stored-unreadable = The saved schema scope couldn't be read. The pattern shown is the default, and it can't be saved until the file can be read.
# Surface 7 (Scope editor). Save failed at the top of the form: nothing was saved.
hv-scope-save-cache-failed = The cached metadata of this connection couldn't be deleted, so nothing was saved.
# Surface 7 (Scope editor). Save failed at the top of the form: nothing was saved.
hv-scope-save-data-unreadable = This connection's explorer settings file can't be read, so nothing was saved.
# Surface 7 (Scope editor). Save failed at the top of the form: nothing was saved.
hv-scope-save-data-failed = The schema scope couldn't be saved, so nothing was saved.
# Surface 6 (Introspection level). A toast when a level could not be saved.
hv-explorer-level-not-saved = The introspection level couldn't be saved.

## m4-t24-live
# Surface 5 (Context menus). The palette title of the node menu's "Copy name".
hv-cmd-explorer-copy-name = Copy name
# Surface 5 (Context menus). The palette title of "Hide objects like this".
hv-cmd-explorer-hide-like-this = Hide objects like this
# Surface 5 (Context menus). The palette title of "Show hidden objects".
hv-cmd-explorer-show-hidden = Show hidden objects
# Surface 5 (Context menus). The palette title of "Collapse all children".
hv-cmd-explorer-collapse-children = Collapse all children
# Surface 5 (Context menus). The palette title of "Introspection level" (opens the level list).
hv-cmd-explorer-level-menu = Introspection level
# Surface 5 (Context menus). The title of the disabled Modify… placeholder (never in the palette).
hv-cmd-explorer-modify = Modify
# Surface 5 (Context menus). Open data refused: the object is in another database than the one
# this connection's consoles use.
# Variables:
#   $database (String) - the object's database, verbatim, e.g. "reports".
#   $current (String) - the database the consoles use, verbatim, e.g. "app".
hv-explorer-open-data-other-db = This connection's consoles use { $current }, so Open data can't show the data of { $database }. Connect to { $database } to see it.
# Surface 5 (Context menus). Open data refused: which database the consoles use is not known.
# Variables:
#   $database (String) - the object's database, verbatim, e.g. "reports".
hv-explorer-open-data-db-unknown = Hacivat can't tell which database this connection's consoles use, so Open data didn't open { $database }. Set the connection's database, or open a console on it first.
# Surface 5 (Context menus). Hide objects like this refused: the name can't be written as a view filter.
# Variables:
#   $name (String) - the object's name, verbatim, e.g. "yedek_2019".
hv-explorer-hide-refused = Objects like { $name } can't be hidden: the name can't be written as a view filter.

## m4-t25-ddl-view
# Surface 10 (DDL view). The toolbar's read-only chip (never hides).
hv-ddl-view-read-only = read-only
# Surface 10 (DDL view). The definition could not be read (not loaded, or the login can't read it).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-ddl-view-not-loaded = Hacivat couldn't read the definition of { $name }.
# Surface 10 (DDL view). Under hv-ddl-view-not-loaded.
hv-ddl-view-not-loaded-body = Refresh reads it from the server again.
# Surface 11 (Stale object). The "no longer exists" notice's button.
hv-ddl-view-close-tab = Close tab
# Surface 10 (DDL view). The tooltip of "Definitions provided by server" on SQLite, where it is
# checked and can't be changed (ruling R-W4-12).
hv-ddl-view-server-fixed-tip = SQLite always shows the server's text: CHECK constraints and generated columns exist only there.
# The palette title of the DDL view's Reload (F5 in its text).
hv-cmd-ddl-view-reload = Reload DDL
# Surface 11 (Stale object). While the state check runs (review m-2): a quiet caption.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-ddl-view-checking = Checking whether { $name } changed on the server…

## m4-t27-refresh-ui
# Surface 13 (Settings › Metadata). Clearing a connection's cached metadata failed (its files could not be deleted).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-metadata-clear-failed = The cached metadata of { $name } couldn't be deleted. Nothing changed.
# Surface 9 (Refresh after DDL). Ruling R-W5-10: the Ask when a whole schema changed (dynamic SQL, a database-wide change): no count.
# Variables:
#   $schema (String) - a schema (or database) name, verbatim, e.g. "public".
hv-metadata-refresh-ask-schema = Refresh the database explorer to reflect the changes in { $schema }?
# Surface 9 (Refresh after DDL). Ruling R-W5-10: the namespace stamp's tooltip when the whole schema waits for COMMIT.
# Variables:
#   $schema (String) - a schema (or database) name, verbatim, e.g. "public".
hv-metadata-after-commit-tip-schema = Changes to { $schema } from this transaction wait for COMMIT. ROLLBACK discards them and the explorer stays as it is.
# Surface 9 (Refresh after DDL). Ruling R-W5-10: the output line when the whole schema waits for COMMIT.
# Variables:
#   $schema (String) - a schema (or database) name, verbatim, e.g. "public".
hv-metadata-log-pending-schema = Explorer refresh waits for COMMIT (changes in { $schema })

## m4-t32-shots
# Settings › General (PO ruling R-W5-14 a). The time zone row's title.
hv-settings-time-zone = Time zone
# Settings › General (PO ruling R-W5-14 a). The choice that follows the operating system, with the zone it reports.
# Variables:
#   $zone (String) - an IANA time zone, verbatim, e.g. "Europe/Istanbul".
hv-settings-time-zone-system = System ({ $zone })
# Settings › General (PO ruling R-W5-14 a). Coordinated Universal Time.
hv-settings-time-zone-utc = UTC
# Settings › General (PO ruling R-W5-14 a). The note under the time zone row.
hv-settings-time-zone-note = Dates and times in results show in this zone. Connections opened from now on use it; open ones keep theirs.
# Command palette (PO ruling R-W5-14 b): clears the selected connection's query history.
hv-cmd-connection-clear-history = Clear query history of connection
# The connection context menu (PO ruling R-W5-14 b).
hv-conn-menu-clear-history = Clear query history…
# The query history dialog (PO ruling R-W5-14 b). The ⋯ button's name and tooltip.
hv-history-more = More actions
# The query history dialog's ⋯ menu (PO ruling R-W5-14 b).
hv-history-clear-item = Clear history of this connection…
# Command palette (PO ruling R-W5-14 b): clears the history of the console's connection, from the history dialog.
hv-cmd-history-clear = Clear query history of this connection
# The clear-history confirmation (PO ruling R-W5-14 b). Cancel is the default button.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-history-clear-title = Clear the query history of { $name }?
# The clear-history confirmation (PO ruling R-W5-14 b).
hv-history-clear-body = Every statement saved for this connection is deleted from this computer, pinned ones too. This can't be undone. Nothing on the server changes.
# The clear-history confirmation (PO ruling R-W5-14 b). The button that clears.
hv-history-clear-btn = Clear history
# A toast after the history of a connection was cleared (PO ruling R-W5-14 b).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-history-cleared = The query history of { $name } was cleared.
# A toast when clearing the history of a connection failed (PO ruling R-W5-14 b).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-history-clear-failed = The query history of { $name } couldn't be cleared. Nothing changed.
# Settings › General (fix round 1, review M-3). A saved zone the time zone database does not know, as a choice.
# Variables:
#   $zone (String) - an IANA time zone, verbatim, e.g. "Europe/Istanbul".
hv-settings-time-zone-unknown-choice = { $zone } (unknown)
# Settings › General (fix round 1, review M-3). The notice under the row while the saved zone is unknown.
# Variables:
#   $zone (String) - an IANA time zone, verbatim, e.g. "Europe/Istanbul".
#   $system (String) - the system choice as the select shows it, e.g. "System (Europe/Istanbul)".
hv-settings-time-zone-unknown = { $zone } isn't a time zone Hacivat knows, so { $system } is used.

## m4-t33b-carries

# The clear-history confirmation when damaged backups exist (T32 carry, ruling 2026-11-20).
# Variables:
#   $count (Number) - how many damaged backups of the history exist, e.g. 2.
hv-history-clear-body-backups = { $count ->
        [one] Every statement saved for this connection is deleted from this computer, pinned ones too. This can't be undone. Nothing on the server changes. Hacivat also keeps { $count } damaged backup of the history, set aside when the file was repaired. It may still hold this connection's statements, and every other connection's.
       *[other] Every statement saved for this connection is deleted from this computer, pinned ones too. This can't be undone. Nothing on the server changes. Hacivat also keeps { $count } damaged backups of the history, set aside when the file was repaired. They may still hold this connection's statements, and every other connection's.
    }
# The clear-history confirmation: the checkbox (off by default) that also deletes the damaged backups.
hv-history-clear-backups = Also delete damaged history backups
# The clear-history confirmation: the checkbox's dimmer note.
# Variables:
#   $count (Number) - how many damaged backups of the history exist, e.g. 2.
hv-history-clear-backups-note = { $count ->
        [one] { $count } file, with every connection's old statements
       *[other] { $count } files, with every connection's old statements
    }
# A toast after the history of a connection was cleared and the damaged backups deleted.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
#   $count (Number) - how many damaged backups were deleted, e.g. 2.
hv-history-cleared-backups = { $count ->
        [one] The query history of { $name } was cleared, and { $count } damaged backup was deleted.
       *[other] The query history of { $name } was cleared, and { $count } damaged backups were deleted.
    }
# A toast when the history file turned out damaged while clearing with the history off (T32 re-review RR-2).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-history-clear-damaged = The history file was damaged, so Hacivat set it aside as a backup. The query history of { $name } wasn't cleared from it; clear again to delete the damaged backup.
# Surface 1 (Explorer tree). The Built-in group's tooltip when only some of its schemas come from the shipped catalog (T32 re-review RR-3).
# Variables:
#   $version (String) - the bundled catalog's server version, e.g. "17".
#   $schemas (String) - the schema names the bundled catalog answers for, a list, e.g. "information_schema".
hv-explorer-builtin-tip-mixed = Built-in catalog (version { $version }) for { $schemas }: shipped with Hacivat and read without a connection. The other built-in schemas were read from the server.
# Surface 4 (Filter field). Under the field when more objects match than the filter opens (T14 carry, R-W2-13).
# Variables:
#   $n (Number) - how many matches the filter did not open, e.g. 50.
hv-explorer-filter-more = { $n ->
        [one] { $n } more match isn't opened. Open its group, or type more to narrow the filter.
       *[other] { $n } more matches aren't opened. Open their groups, or type more to narrow the filter.
    }
# Surface 14 (Long introspection). The meta of a loaded schema row while it is read again: the share read, in whole percents (T24 carry).
# Variables:
#   $fraction (Number) - the share read so far, from 0 to 1, e.g. 0.23.
hv-explorer-progress-percent = { NUMBER($fraction, style: "percent", minimumFractionDigits: 0, maximumFractionDigits: 0) }
# Surface 13 (Settings › Metadata). The clear-cache question's subtitle: the size of the cached metadata (T27 carry).
# Variables:
#   $size (String) - a preformatted size, e.g. "38.4 MB".
hv-metadata-clear-subtitle = { $size } on this computer
# A toast when the history of a connection was cleared but its damaged backups could not be deleted (T33b review 6).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
hv-history-cleared-backups-failed = The query history of { $name } was cleared, but its damaged backups couldn't be deleted.

## m4b-t01b-seams

# Surface 1 (Scripts node).
hv-scripts-node = Scripts
# Surface 1 (Scripts node).
hv-scripts-node-shared = shared
# Surface 1 (Scripts node).
# Variables:
#   $conns (String) - the other connections, a locale list of names (data), e.g. "orders-staging and orders-dev".
hv-scripts-node-shared-tip = Also under { $conns }: every connection with a database of this name shows the same files.
# Surface 1 (Scripts node).
# Variables:
#   $path (String) - a folder or file path, verbatim (data), e.g. "~/work/orders-scripts".
#   $database (String) - a database name, verbatim (data), e.g. "orders".
hv-scripts-lib-tip = Folder { $path }. Binds to the database { $database }.
# Surface 1 (Scripts node).
# Variables:
#   $env (String) - the environment word (an env-* message), e.g. "production".
hv-scripts-env-folder-tip = Only connections tagged { $env } see this folder.
# Surface 1 (Scripts node).
hv-scripts-git-modified = M
# Surface 1 (Scripts node).
hv-scripts-git-modified-tip = Modified: differs from the last commit
# Surface 1 (Scripts node).
hv-scripts-git-new = N
# Surface 1 (Scripts node).
hv-scripts-git-new-tip = New: not in the last commit
# Surface 1 (Scripts node).
hv-scripts-git-conflicted = C
# Surface 1 (Scripts node).
hv-scripts-git-conflicted-tip = Conflicted: the merge isn't resolved
# Surface 1 (Scripts node).
hv-scripts-git-unknown = ?
# Surface 1 (Scripts node).
# Variables:
#   $reason (String) - selector: the reason; one of dataless, filter (anything else: the default).
hv-scripts-git-unknown-tip =
    { $reason ->
        [dataless] Git status unknown: the file isn't downloaded, and Hacivat doesn't download it to compare.
       *[filter] Git status unknown: a .gitattributes filter applies to it, and Hacivat never runs filters.
    }
# Surface 1 (Scripts node).
hv-scripts-git-contains = •
# Surface 1 (Scripts node).
hv-scripts-git-contains-tip = Contains changed files
# Surface 1 (Scripts node).
hv-scripts-outside-link-tip = Points outside the library, so Hacivat never opens it.
# Surface 1 (Scripts node).
hv-scripts-dataless = not downloaded
# Surface 1 (Scripts node).
hv-scripts-downloading = Downloading…
# Surface 1 (Scripts node).
# Variables:
#   $n (Number) - a count, e.g. 3.
hv-scripts-diag-row =
    { $n ->
        [one] { $n } problem
       *[other] { $n } problems
    }
# Surface 1 (Scripts node).
hv-scripts-missing-row = Folder not found
# Surface 1 (Scripts node).
hv-scripts-loading = Listing scripts…
# Surface 2 (Context menus).
hv-explorer-menu-attach-library = Attach script library…
# Surface 2 (Context menus).
hv-explorer-menu-map-library = Map to a script library…
# Surface 2 (Context menus).
hv-explorer-menu-map-library-none = No other library is attached
# Surface 2 (Context menus).
hv-scripts-menu-new-script = New script
# Surface 2 (Context menus).
hv-scripts-menu-new-folder = New folder
# Surface 2 (Context menus).
hv-scripts-menu-open = Open
# Surface 2 (Context menus).
hv-scripts-menu-open-under = Open under another connection…
# Surface 2 (Context menus).
hv-scripts-menu-rename = Rename
# Surface 2 (Context menus).
# Variables:
#   $os (String) - selector: the host (macos, windows, linux); one of other, windows (anything else: the default).
hv-scripts-menu-trash =
    { $os ->
        [windows] Move to Recycle Bin
       *[other] Move to Trash
    }
# Surface 2 (Context menus).
# Variables:
#   $os (String) - selector: the host (macos, windows, linux); one of linux, macos, windows (anything else: the default).
hv-scripts-menu-reveal =
    { $os ->
        [windows] Reveal in File Explorer
        [linux] Open Containing Folder
       *[macos] Reveal in Finder
    }
# Surface 2 (Context menus).
hv-scripts-menu-copy-path = Copy path
# Surface 2 (Context menus).
hv-scripts-menu-copy-rel = Copy relative path
# Surface 2 (Context menus).
hv-scripts-menu-settings = Script library settings…
# Surface 2 (Context menus).
hv-scripts-menu-outside-tip = Points outside the library
# Surface 2 (Context menus).
hv-scripts-new-name = untitled.sql
# Surface 2 (Context menus).
hv-scripts-new-folder-name = new folder
# Surface 2 (Context menus).
# Variables:
#   $reason (String) - selector: the reason; one of dots, empty, exists, ext, reserved, slash (anything else: the default).
#   $name (String) - a file or folder name, verbatim (data), e.g. "archive_old_orders.sql".
hv-scripts-name-err =
    { $reason ->
        [slash] A name can't contain / or \.
        [reserved] Windows keeps this name for devices (CON, PRN, AUX, NUL, COM1…).
        [exists] { $name } is already in this folder.
        [dots] A name can't be . or .., and can't end with a dot or a space.
        [ext] A script's name ends in .sql or .md.
       *[empty] Type a name.
    }
# Surface 2 (Context menus).
# Variables:
#   $os (String) - selector: the host (macos, windows, linux); one of other, windows (anything else: the default).
#   $name (String) - a file or folder name, verbatim (data), e.g. "archive_old_orders.sql".
hv-scripts-trashed =
    { $os ->
        [windows] Moved { $name } to the Recycle Bin. You can restore it from there.
       *[other] Moved { $name } to the Trash. You can put it back from there.
    }
# Surface 3 (Attach dialog).
hv-scripts-attach-choose = Choose a folder of scripts
# Surface 3 (Attach dialog).
hv-scripts-attach-title = Attach a script library
# Surface 3 (Attach dialog).
hv-scripts-attach-folder = Folder
# Surface 3 (Attach dialog).
hv-scripts-attach-change = Change…
# Surface 3 (Attach dialog).
hv-scripts-attach-database = Database
# Surface 3 (Attach dialog).
hv-scripts-attach-database-hint = The database these scripts are for. Every connection with a database of this name shows them.
# Surface 3 (Attach dialog).
hv-scripts-attach-aliases = Aliases
# Surface 3 (Attach dialog).
hv-scripts-attach-aliases-ph = Other names for it, separated by commas
# Surface 3 (Attach dialog).
hv-scripts-attach-shows-under = Shows under
# Surface 3 (Attach dialog).
# Variables:
#   $database (String) - a database name, verbatim (data), e.g. "orders".
hv-scripts-attach-shows-none = No connection has a database named { $database } yet.
# Surface 3 (Attach dialog).
hv-scripts-attach-will-write = Hacivat creates this file in the folder:
# Surface 3 (Attach dialog).
hv-scripts-attach-file-note = It holds only the database name and its aliases: never a connection, a host or a password. Commit it so your team gets the same binding.
# Surface 3 (Attach dialog).
# Variables:
#   $database (String) - a database name, verbatim (data), e.g. "orders".
hv-scripts-attach-found = Found .hacivat/workspace.toml — binds to { $database }
# Surface 3 (Attach dialog).
hv-scripts-attach-found-note = Hacivat reads this file and never changes it.
# Surface 3 (Attach dialog).
hv-scripts-attach-btn = Attach
# Surface 3 (Attach dialog).
# Variables:
#   $reason (String) - selector: the reason; one of already, control, empty, long, many (anything else: the default).
hv-scripts-attach-err =
    { $reason ->
        [control] A database name can't contain control characters.
        [long] A database name can be at most 128 bytes.
        [already] This folder is already attached.
        [many] Hacivat holds up to 32 libraries. Remove one in Settings › Script libraries first.
       *[empty] Type the database name.
    }
# Surface 3 (Attach dialog).
# Variables:
#   $database (String) - a database name, verbatim (data), e.g. "orders".
hv-scripts-map-title = Show a script library under { $database }
# Surface 3 (Attach dialog).
hv-scripts-map-library = Library
# Surface 3 (Attach dialog).
# Variables:
#   $database (String) - a database name, verbatim (data), e.g. "orders".
hv-scripts-map-binds = binds to { $database }
# Surface 3 (Attach dialog).
hv-scripts-map-note = The mapping is kept on this computer only. Your teammates don't see it.
# Surface 3 (Attach dialog).
# Variables:
#   $database (String) - a database name, verbatim (data), e.g. "orders".
hv-scripts-map-alias = Also add { $database } as an alias for everyone who uses this folder
# Surface 3 (Attach dialog).
hv-scripts-map-alias-sub = Adds it to .hacivat/workspace.toml and keeps the file's comments
# Surface 3 (Attach dialog).
hv-scripts-map-btn = Show here
# Surface 3 (Attach dialog).
hv-scripts-map-other = Attach another folder…
# Surface 4 (Settings › Script libraries).
hv-settings-nav-scripts = Script libraries
# Surface 4 (Settings › Script libraries).
hv-settings-section-scripts = Script libraries
# Surface 4 (Settings › Script libraries).
hv-scripts-set-attach = Attach a folder…
# Surface 4 (Settings › Script libraries).
hv-scripts-set-note = A library is a folder of .sql and .md files. Removing one never changes the folder or its files.
# Surface 4 (Settings › Script libraries).
# Variables:
#   $state (String) - selector: the watcher state; one of limit, polling, unavailable, watching (anything else: the default).
#   $n (Number) - a count, e.g. 3.
hv-scripts-set-status =
    { $state ->
        [polling] Checking every 2 seconds
        [unavailable] Folder not found
        [limit] Stopped at { $n } files
       *[watching] Watching for changes
    }
# Surface 4 (Settings › Script libraries).
# Variables:
#   $reason (String) - selector: the reason; one of network, watches (anything else: the default).
hv-scripts-set-status-tip =
    { $reason ->
        [watches] The system's file-watch limit is low, so Hacivat checks this folder every 2 seconds instead of being told about changes.
       *[network] This folder is on a network drive, so Hacivat checks it every 2 seconds instead of being told about changes.
    }
# Surface 4 (Settings › Script libraries).
# Variables:
#   $n (Number) - a count, e.g. 3.
hv-scripts-set-files =
    { $n ->
        [one] { $n } script
       *[other] { $n } scripts
    }
# Surface 4 (Settings › Script libraries).
# Variables:
#   $aliases (String) - the aliases, a locale list (data), e.g. "orders_p".
hv-scripts-set-aliases = aliases { $aliases }
# Surface 4 (Settings › Script libraries).
hv-scripts-set-git = git repository
# Surface 4 (Settings › Script libraries).
hv-scripts-set-git-off = git status off
# Surface 4 (Settings › Script libraries).
hv-scripts-set-remove = Remove…
# Surface 4 (Settings › Script libraries).
hv-scripts-set-mappings = Local mappings
# Surface 4 (Settings › Script libraries).
hv-scripts-set-mapping-shows = shows
# Surface 4 (Settings › Script libraries).
hv-scripts-set-mapping-remove = Remove
# Surface 4 (Settings › Script libraries).
hv-scripts-set-empty = No script libraries yet. Attach a folder of .sql and .md files to see its scripts under the databases they are for.
# Surface 4 (Settings › Script libraries).
# Variables:
#   $library (String) - a library's folder name, verbatim (data), e.g. "orders-scripts".
hv-scripts-remove-title = Remove { $library } from Hacivat?
# Surface 4 (Settings › Script libraries).
# Variables:
#   $n (Number) - a count, e.g. 3.
hv-scripts-remove-body =
    { $n ->
        [one] The folder and its files stay as they are, including .hacivat/workspace.toml. Its Scripts nodes go away, and so does its { $n } local mapping.
       *[other] The folder and its files stay as they are, including .hacivat/workspace.toml. Its Scripts nodes go away, and so do its { $n } local mappings.
    }
# Surface 4 (Settings › Script libraries).
hv-scripts-remove-btn = Remove
# Surface 5 (Script tab).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
#   $conn (String) - a connection name, verbatim (data), e.g. "orders-prod".
hv-scripts-tab-title = { $file } · { $conn }
# Surface 5 (Script tab).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
#   $conn (String) - a connection name, verbatim (data), e.g. "orders-prod".
#   $database (String) - a database name, verbatim (data), e.g. "orders".
hv-scripts-tab-title-db = { $file } · { $conn } · { $database }
# Surface 5 (Script tab).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
hv-scripts-tab-title-unbound = { $file } · no connection
# Surface 5 (Script tab).
# Variables:
#   $library (String) - a library's folder name, verbatim (data), e.g. "orders-scripts".
hv-scripts-tab-tip-library = From the library { $library }
# Surface 5 (Script tab).
# Variables:
#   $conn (String) - a connection name, verbatim (data), e.g. "orders-prod".
#   $database (String) - a database name, verbatim (data), e.g. "orders".
hv-scripts-tab-tip-runs = Runs on { $conn }, database { $database }
# Surface 5 (Script tab).
hv-scripts-tab-tip-unbound = Runs nowhere: its connection was deleted
# Surface 5 (Script tab).
hv-scripts-tab-tip-unsaved = Unsaved changes
# Surface 5 (Script tab).
hv-scripts-tab-save = Save
# Surface 5 (Script tab).
hv-scripts-tab-save-all = Save all
# Surface 5 (Script tab).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
hv-scripts-tab-saved = Saved { $file }
# Surface 5 (Script tab).
hv-scripts-run-md = Markdown notes do not run
# Surface 5 (Script tab).
hv-scripts-run-unbound = This tab's connection was deleted, so it runs nothing. Open it under another connection.
# Surface 5 (Script tab).
# Variables:
#   $database (String) - a database name, verbatim (data), e.g. "orders".
hv-scripts-run-switching = Switching to { $database }…
# Surface 5 (Script tab).
hv-scripts-unbound-title = The connection this tab was opened under was deleted.
# Surface 5 (Script tab).
hv-scripts-unbound-body = The text is kept. It runs nowhere until you open it under a connection.
# Surface 5 (Script tab).
# Variables:
#   $database (String) - a database name, verbatim (data), e.g. "orders".
hv-scripts-use-log = This tab's session now uses { $database }
# Surface 6 (Changes on disk).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
hv-scripts-disk-changed = { $file } changed on disk.
# Surface 6 (Changes on disk).
hv-scripts-disk-changed-body = Your edits in this tab aren't saved. Reload discards them; Keep mine keeps them, and saving then asks before it overwrites.
# Surface 6 (Changes on disk).
hv-scripts-disk-reload = Reload
# Surface 6 (Changes on disk).
hv-scripts-disk-keep = Keep mine
# Surface 6 (Changes on disk).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
hv-scripts-disk-reload-title = Discard your edits and reload { $file }?
# Surface 6 (Changes on disk).
hv-scripts-disk-reload-body = The file on disk replaces the text in this tab. Your edits are discarded.
# Surface 6 (Changes on disk).
hv-scripts-disk-reload-yes = Discard and reload
# Surface 6 (Changes on disk).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
hv-scripts-disk-reloaded = Reloaded { $file }: it changed on disk
# Surface 6 (Changes on disk).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
#   $new (String) - the new file name, verbatim (data), e.g. "haftalık_rapor.sql".
hv-scripts-disk-renamed = { $file } was renamed to { $new }; this tab follows it
# Surface 6 (Changes on disk).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
hv-scripts-disk-deleted = { $file } was deleted on disk.
# Surface 6 (Changes on disk).
hv-scripts-disk-deleted-body = The text stays in this tab. Save writes the file again.
# Surface 6 (Changes on disk).
hv-scripts-disk-overwrite-title = The file changed on disk since you opened it. Overwrite?
# Surface 6 (Changes on disk).
hv-scripts-disk-overwrite-body = Saving replaces the version on disk, including the changes made outside Hacivat.
# Surface 6 (Changes on disk).
hv-scripts-disk-overwrite = Overwrite
# Surface 6 (Changes on disk).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
#   $conn (String) - a connection name, verbatim (data), e.g. "orders-prod".
hv-scripts-unlisted = { $file } is no longer listed under { $conn }.
# Surface 6 (Changes on disk).
# Variables:
#   $reason (String) - selector: the reason; one of binding, env (anything else: the default).
#   $folder (String) - a folder name, verbatim (data), e.g. "@prod".
#   $conn (String) - a connection name, verbatim (data), e.g. "orders-prod".
hv-scripts-unlisted-body =
    { $reason ->
        [binding] Its library no longer binds to this database. This tab still runs only on { $conn }.
       *[env] It moved into { $folder }, a folder for another environment. This tab still runs only on { $conn }.
    }
# Surface 7 (Go to script).
hv-cmd-scripts-goto = Go to script
# Surface 7 (Go to script).
hv-cmd-file-save = Save script
# Surface 7 (Go to script).
hv-cmd-file-save-all = Save all scripts
# Surface 7 (Go to script).
hv-cmd-scripts-attach = Attach script library…
# Surface 7 (Go to script).
# Variables:
#   $os (String) - selector: the host (macos, windows, linux); one of linux, macos, windows (anything else: the default).
hv-cmd-scripts-reveal =
    { $os ->
        [windows] Reveal script in File Explorer
        [linux] Open the script's folder
       *[macos] Reveal script in Finder
    }
# Surface 2 (Context menus). The palette title of F2 on a script file or folder row (controller ruling c).
hv-cmd-scripts-rename = Rename script or folder
# Surface 7 (Go to script).
hv-scripts-goto-ph = Go to a script by name
# Surface 7 (Go to script).
hv-scripts-goto-none = No script matches
# Surface 7 (Go to script).
hv-scripts-goto-empty = No script libraries yet
# Surface 7 (Go to script).
hv-scripts-goto-empty-body = Attach a folder from a database's menu or Settings › Script libraries.
# Surface 7 (Go to script).
hv-scripts-goto-hint-open = Open
# Surface 7 (Go to script).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
hv-scripts-choose-title = Open { $file } under
# Surface 7 (Go to script).
hv-scripts-choose-current = current
# Surface 7 (Go to script).
hv-scripts-choose-prod-note = Production is never chosen for you.
# Surface 7 (Go to script).
hv-scripts-choose-none = No connection shows this file.
# Surface 7 (Go to script).
hv-scripts-choose-none-row = Open without a connection
# Surface 7 (Go to script).
hv-scripts-choose-none-body = It opens unbound and runs nothing until you open it under a connection.
# Surface 7 (Go to script).
hv-scripts-choose-hint-choose = Choose
# Surface 7 (Go to script).
hv-scripts-choose-hint-back = Back
# Surface 8 (Secret warning).
# Variables:
#   $kind (String) - selector: the finding's kind; one of key, password, token, url (anything else: the default).
hv-scripts-secret-gutter =
    { $kind ->
        [token] Looks like an access token
        [key] Looks like a private key
        [url] Looks like a connection URL with a password
       *[password] Looks like a password
    }
# Surface 8 (Secret warning).
# m4b-t21-shots (named exception, T17 review m5): the marker is a placeable, so no locale mangles it.
# Variables:
#   $marker (String) - the literal hacivat:allow-secret (never translated).
hv-scripts-secret-gutter-hint = Saving asks first. A comment on this line such as -- { $marker } (with a space after the dashes) keeps it quiet.
# Surface 8 (Secret warning).
# Variables:
#   $n (Number) - a count, e.g. 3.
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
hv-scripts-secret-title =
    { $n ->
        [one] { $n } line in { $file } looks like a secret
       *[other] { $n } lines in { $file } look like secrets
    }
# Surface 8 (Secret warning).
hv-scripts-secret-body = Hacivat found text that looks like a password or a token. The values aren't shown here.
# Surface 8 (Secret warning).
# Variables:
#   $line (Number) - a line number, shown without grouping, e.g. 48213.
hv-scripts-secret-line = Line { NUMBER($line, useGrouping: "false") }
# Surface 8 (Secret warning).
hv-scripts-secret-git = If you commit this file, the value is shared with everyone who can read the repository.
# Surface 8 (Secret warning).
# Variables:
#   $marker (String) - the literal hacivat:allow-secret (never translated).
hv-scripts-secret-allow = To keep a line on purpose, add the comment -- { $marker } (with a space after the dashes) or /* { $marker } */ on it.
# Surface 8 (Secret warning).
hv-scripts-secret-remember = Save anyway is remembered for this tab until you quit.
# Surface 8 (Secret warning).
hv-scripts-secret-save = Save anyway
# Surface 9 (Large files).
hv-scripts-run-as-script = Run as script
# Surface 9 (Large files).
hv-scripts-run-as-script-tip = Sends the whole file from disk, one statement at a time, each through Run's checks. It stops at the first error.
# Surface 9 (Large files).
hv-scripts-run-running = Running as script
# Surface 9 (Large files).
# Variables:
#   $n (Number) - a count, e.g. 3.
hv-scripts-run-sent =
    { $n ->
        [one] { $n } statement sent
       *[other] { $n } statements sent
    }
# Surface 9 (Large files).
# Variables:
#   $line (Number) - a line number, shown without grouping, e.g. 48213.
hv-scripts-run-line = line { NUMBER($line, useGrouping: "false") }
# Surface 9 (Large files).
# Variables:
#   $read (String) - bytes read so far (an hv-scripts-size-mb message), e.g. "38 MB".
#   $size (String) - a size (an hv-scripts-size-mb message), e.g. "148 MB".
hv-scripts-run-read = { $read } of { $size }
# Surface 9 (Large files).
hv-scripts-run-busy = Running as script: wait for it, or stop it
# Surface 9 (Large files).
# Variables:
#   $n (Number) - a count, e.g. 3.
hv-scripts-run-stopped = Stopped at statement { $n }: the server returned an error. Nothing after it was sent.
# Surface 9 (Large files).
# Variables:
#   $n (Number) - a count, e.g. 3.
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
hv-scripts-run-done =
    { $n ->
        [one] Ran { $n } statement from { $file }
       *[other] Ran { $n } statements from { $file }
    }
# Surface 9 (Large files).
# Variables:
#   $n (Number) - a count, e.g. 3.
hv-scripts-run-cancelled =
    { $n ->
        [one] Stopped after { $n } statement. Nothing after it was sent.
       *[other] Stopped after { $n } statements. Nothing after them was sent.
    }
# Surface 9 (Large files).
# Variables:
#   $n (Number) - a count, e.g. 3.
hv-scripts-size-mb = { $n } MB
# Surface 10 (Encoding).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
hv-scripts-enc-title = { $file } isn't UTF-8 text.
# Surface 10 (Encoding).
hv-scripts-enc-body = Choose the encoding it was saved in, or open it read-only to look first.
# Surface 10 (Encoding).
hv-scripts-enc-label = Encoding
# Surface 10 (Encoding).
hv-scripts-enc-open = Open
# Surface 10 (Encoding).
hv-scripts-enc-ro = Open read-only
# Surface 10 (Encoding).
hv-scripts-enc-ro-chip = read-only
# Surface 10 (Encoding).
hv-scripts-enc-ro-note = Shown as UTF-8; characters it can't read show as �. Reopen with an encoding to edit.
# Surface 10 (Encoding).
# Variables:
#   $enc (String) - selector: the encoding id; one of iso-8859-9, other, utf-16le, windows-1252, windows-1254 (anything else: the default).
#   $encoding (String) - an encoding's name (data), e.g. "Windows-1254".
hv-scripts-enc-name =
    { $enc ->
        [windows-1254] Windows-1254 (Turkish)
        [iso-8859-9] ISO-8859-9 (Turkish)
        [windows-1252] Windows-1252 (Western)
        [utf-16le] UTF-16 LE
       *[other] { $encoding }
    }
# Surface 10 (Encoding).
# Variables:
#   $encoding (String) - an encoding's name (data), e.g. "Windows-1254".
hv-scripts-enc-bom = { $encoding } with BOM
# Surface 10 (Encoding).
hv-scripts-enc-seg-tip = Encoding and line endings. Saving keeps both.
# Surface 10 (Encoding).
# Variables:
#   $encoding (String) - an encoding's name (data), e.g. "Windows-1254".
hv-scripts-enc-menu-head = Encoding: { $encoding }
# Surface 10 (Encoding).
hv-scripts-enc-reopen = Reopen with encoding…
# Surface 10 (Encoding).
hv-scripts-enc-save-with = Save with encoding…
# Surface 10 (Encoding).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
hv-scripts-enc-unencodable-title = Couldn't save { $file }
# Surface 10 (Encoding).
# Variables:
#   $char (String) - the character that cannot be encoded (data), e.g. "漢".
#   $line (Number) - a line number, shown without grouping, e.g. 48213.
#   $encoding (String) - an encoding's name (data), e.g. "Windows-1254".
hv-scripts-enc-unencodable-body = { $char } on line { NUMBER($line, useGrouping: "false") } can't be written in { $encoding }. Remove it, or use Save with encoding….
# Surface 11 (Diagnostics).
# Variables:
#   $n (Number) - a count, e.g. 3.
#   $library (String) - a library's folder name, verbatim (data), e.g. "orders-scripts".
hv-scripts-diag-title =
    { $n ->
        [one] { $n } problem in { $library }
       *[other] { $n } problems in { $library }
    }
# Surface 11 (Diagnostics).
# Variables:
#   $folder (String) - a folder name, verbatim (data), e.g. "@prod".
#   $suggestion (String) - the tag it probably meant, e.g. "@production".
hv-scripts-diag-unknown-env = { $folder } is not an environment tag — files under it are hidden (did you mean { $suggestion }?)
# Surface 11 (Diagnostics).
# Variables:
#   $folder (String) - a folder name, verbatim (data), e.g. "@prod".
hv-scripts-diag-unknown-env-nosug = { $folder } is not an environment tag — files under it are hidden.
# Surface 11 (Diagnostics).
hv-scripts-diag-env-help = Environment folders are @local, @dev, @test, @staging and @production.
# Surface 11 (Diagnostics).
# Variables:
#   $path (String) - a folder or file path, verbatim (data), e.g. "~/work/orders-scripts".
hv-scripts-diag-contradict = { $path } names two environments, so the files under it are hidden everywhere.
# Surface 11 (Diagnostics).
# Variables:
#   $limit (String) - selector: which limit; one of depth, files (anything else: the default).
#   $n (Number) - a count, e.g. 3.
hv-scripts-diag-limit =
    { $limit ->
        [depth] Folders deeper than { $n } levels aren't listed.
       *[files] Listing stopped at { $n } files; the rest of the folder isn't shown.
    }
# Surface 11 (Diagnostics).
# Variables:
#   $reason (String) - selector: the reason; one of network, watches (anything else: the default).
hv-scripts-diag-polling =
    { $reason ->
        [watches] The system's file-watch limit is low, so changes are checked every 2 seconds.
       *[network] This folder is on a network drive, so changes are checked every 2 seconds.
    }
# Surface 11 (Diagnostics).
# Variables:
#   $reason (String) - selector: the reason; one of invalid, large, newer (anything else: the default).
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
#   $folder (String) - a folder name, verbatim (data), e.g. "@prod".
hv-scripts-diag-toml =
    { $reason ->
        [newer] { $file } was written by a newer Hacivat, so the library binds by its folder name, { $folder }.
        [large] { $file } is larger than 64 KB, so it isn't read; the library binds by its folder name, { $folder }.
       *[invalid] { $file } isn't valid TOML, so the library binds by its folder name, { $folder }.
    }
# Surface 11 (Diagnostics).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
hv-scripts-diag-toml-keys = { $file } has keys Hacivat doesn't use. They are ignored.
# Surface 11 (Diagnostics).
hv-scripts-diag-parser = Parser message
# Surface 11 (Diagnostics).
hv-scripts-diag-untrusted = Git status is off: the repository belongs to another user.
# Surface 11 (Diagnostics).
hv-scripts-diag-untrusted-help = Git itself refuses such a repository until it is marked safe (safe.directory). Hacivat shows no badges for it.
# Surface 11 (Diagnostics).
# Variables:
#   $path (String) - a folder or file path, verbatim (data), e.g. "~/work/orders-scripts".
hv-scripts-diag-missing = The folder isn't there: { $path }. It may be on a drive that isn't connected.
# Surface 12 (Recovery).
# Variables:
#   $n (Number) - a count, e.g. 3.
hv-scripts-recover-title =
    { $n ->
        [one] Recovered unsaved changes to { $n } script
       *[other] Recovered unsaved changes to { $n } scripts
    }
# Surface 12 (Recovery).
hv-scripts-recover-body = Hacivat closed before they were saved. Their tabs are open again with your changes; save to keep them.
# Surface 12 (Recovery).
hv-scripts-recover-show = Show
# Surface 12 (Recovery).
hv-scripts-recover-dismiss = Dismiss
# Surface 12 (Recovery).
# Variables:
#   $date (DateTime) - when the recovery copy was written, e.g. 2026-11-21 14:05.
# m4b-t21-shots (named exception): DATETIME takes dateStyle and timeStyle only; the other options were ignored and the time was lost.
hv-scripts-recover-tab = Recovered unsaved changes from { DATETIME($date, dateStyle: "medium", timeStyle: "short") }.
# Surface 12 (Recovery).
hv-scripts-recover-tab-body = Save keeps them. Discard goes back to the file on disk.
# Surface 12 (Recovery).
hv-scripts-recover-discard = Discard…
# Surface 12 (Recovery).
# Variables:
#   $what (String) - selector: what is missing; one of file, library (anything else: the default).
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
#   $library (String) - a library's folder name, verbatim (data), e.g. "orders-scripts".
hv-scripts-recover-missing =
    { $what ->
        [library] The library { $library } isn't attached any more.
       *[file] { $file } isn't in the library any more.
    }
# Surface 12 (Recovery).
hv-scripts-recover-missing-body = Your recovered text is in this tab. Save writes it back to the same place.
# Surface 5 (Script tab).
# Variables:
#   $bound (String) - the database the tab is bound to (data), e.g. "kampanya".
#   $current (String) - the database the session is on (data), e.g. "api".
hv-scripts-run-refused = Run refused: this tab runs on { $bound }, but its session is on { $current }.
# Surface 5 (Script tab).
# Variables:
#   $reason (String) - selector: the reason; one of edited, mysql (anything else: the default).
#   $bound (String) - the database the tab is bound to (data), e.g. "kampanya".
#   $conn (String) - a connection name, verbatim (data), e.g. "orders-prod".
#   $current (String) - the database the session is on (data), e.g. "api".
hv-scripts-run-refused-body =
    { $reason ->
        [edited] { $conn } now opens { $current }: the connection was edited. Nothing was sent. Open the script under the connection that shows { $bound }.
       *[mysql] Hacivat switched the session back once and it still isn't on { $bound }. Nothing was sent. Try again, or open the script under another connection.
    }
# Surface 5 (Script tab).
# Variables:
#   $bound (String) - the database the tab is bound to (data), e.g. "kampanya".
hv-scripts-run-refused-retry = Switch to { $bound } and run
# Surface 5 (Script tab).
hv-scripts-run-refused-unbound = Run refused: this tab has no connection.
# Surface 6b (Unsaved changes).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
hv-scripts-unsaved-title = Save changes to { $file }?
# Surface 6b (Unsaved changes).
hv-scripts-unsaved-body = Your changes are lost if you don't save them.
# Surface 6b (Unsaved changes).
# Variables:
#   $n (Number) - a count, e.g. 3.
#   $total (Number) - the total, e.g. 2.
hv-scripts-unsaved-count = { $n } of { $total } unsaved scripts
# Surface 6b (Unsaved changes).
hv-scripts-unsaved-save = Save
# Surface 6b (Unsaved changes).
hv-scripts-unsaved-discard = Don't save
# Surface 1 (Scripts node).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
hv-scripts-downloading-tab = Downloading { $file }…
# Surface 1 (Scripts node).
hv-scripts-downloading-body = It is kept in the cloud; the text shows when the download ends. Nothing runs until then.
# Surface 2 (Context menus).
hv-scripts-menu-open-under-none = No other connection shows this file
# Surface 4 (Settings › Script libraries).
hv-scripts-remove-body-none = The folder and its files stay as they are, including .hacivat/workspace.toml. Its Scripts nodes go away.
# Surface 1 (Scripts node).
# Variables:
#   $n (Number) - a count, e.g. 3.
hv-scripts-node-row-tip =
    { $n ->
        [one] { $n } script listed under this connection.
       *[other] { $n } scripts listed under this connection.
    }
# Surface 7 (Go to script).
hv-scripts-choose-hint-close = Close
# Surface 1 (Scripts node).
hv-scripts-dataless-row-tip = Kept in the cloud, not downloaded. Opening it downloads it first.
# Surface 1 (Scripts node).
hv-scripts-downloading-row-tip = Downloading… The tab shows the text when the download ends.

## m4b-t13-explorer
# Surface 1 (Scripts node). What assistive technology reads for a file's git badge.
hv-scripts-a11y-modified = modified
# Surface 1 (Scripts node).
hv-scripts-a11y-new = new
# Surface 1 (Scripts node).
hv-scripts-a11y-conflicted = conflicted
# Surface 1 (Scripts node).
hv-scripts-a11y-unknown = git status unknown
# Surface 1 (Scripts node). A folder that holds changed files.
hv-scripts-a11y-contains = contains changes
# Surface 1 (Scripts node).
hv-scripts-a11y-outside = points outside the library
# Surface 1 (Scripts node).
hv-scripts-a11y-dataless = not downloaded
# Surface 1 (Scripts node). The "?" badge's text alternative for the reasons hv-scripts-git-unknown-tip does not cover.
# Variables:
#   $reason (String) - selector: the reason; one of case, notafile, submodule, unsafepath (anything else: the default).
hv-scripts-git-unknown-other-tip =
    { $reason ->
        [submodule] Git status unknown: it is a submodule, and Hacivat never opens one.
        [case] Git status unknown: the repository spells this name with other capitals.
        [unsafepath] Git status unknown: the repository names this file in a way Hacivat does not follow.
       *[notafile] Git status unknown: it isn't a regular file.
    }
# Surface 1 (Scripts node). The Scripts node's child row when the library folder can't be listed.
# Variables:
#   $reason (String) - selector: why; one of moved, responding, unreadable (anything else: the default).
hv-scripts-unavailable-row =
    { $reason ->
        [responding] The folder isn't answering
        [moved] The folder was moved or replaced
       *[unreadable] The folder can't be read
    }
# Surface 11 (Diagnostics). The problems the other diagnostics messages do not cover.
# Variables:
#   $what (String) - selector: the problem; one of badname, hidden, linkscope, manifest, moved, noname, rootenv, suppressed, unreadable (anything else: the default).
#   $n (Number) - a count, e.g. 3.
#   $folder (String) - the library's folder name (data), e.g. "orders-scripts".
hv-scripts-diag-other =
    { $what ->
        [badname] Some names break the naming rules, so those files aren't listed.
        [unreadable] A folder can't be read, so its files aren't listed.
        [linkscope] A link points into another environment's folder, so it isn't listed anywhere.
        [suppressed] { $n } more problems aren't listed.
        [hidden] { $n } more problems concern files this connection doesn't show.
        [noname] Neither .hacivat/workspace.toml nor the folder name { $folder } is a database name, so the library shows only where it is mapped.
        [rootenv] The library folder is inside an environment folder, so nothing in it is listed.
        [moved] The library folder was moved or replaced, so nothing in it is listed.
       *[manifest] .hacivat/workspace.toml can't be used, so the library binds by its folder name, { $folder }.
    }
# Surface 2 (Context menus). The reasons hv-scripts-name-err does not cover, under the name field.
# Variables:
#   $reason (String) - selector: the reason; one of chars, hidden, long (anything else: the default).
hv-scripts-name-err-more =
    { $reason ->
        [long] A name can be at most 255 bytes.
        [hidden] A name can't start with a dot, and node_modules isn't listed.
       *[chars] A name can't contain < > : " | ? * or control characters.
    }
# Surface 2 (Context menus). The toast's title when a file operation was refused.
# Variables:
#   $op (String) - selector: the operation; one of create, rename, trash (anything else: the default).
#   $os (String) - selector: the host (macos, windows, linux); one of linux, macos, windows (anything else: the default).
#   $name (String) - the file or folder name (data), e.g. "orders_report.sql".
hv-scripts-op-failed-title =
    { $op ->
        [create] Couldn't create { $name }
        [rename] Couldn't rename { $name }
       *[trash]
            { $os ->
                [windows] Couldn't move { $name } to the Recycle Bin
               *[other] Couldn't move { $name } to the Trash
            }
    }
# Surface 2 (Context menus). The toast's body: why, in plain words; nothing was changed.
# Variables:
#   $reason (String) - selector: the reason; one of changed, exists, gone, hidden, io, large, listed, name, nohardlinks, path, scope, trash, unavailable (anything else: the default).
hv-scripts-op-failed =
    { $reason ->
        [exists] Something in this folder already has that name. Nothing was changed.
        [scope] This would change which connections see these files. Nothing was changed; rename it in your file manager if that is what you want.
        [large] This folder is too large to check; remove it in your file manager.
        [nohardlinks] This drive can't rename without risking a file of the same name. Nothing was changed; rename it in your file manager.
        [changed] It went to the trash, but a folder on the way changed meanwhile. Check the trash.
        [gone] It isn't in the library any more. Nothing was changed.
        [listed] This connection doesn't show that folder, so Hacivat won't change it from here.
        [hidden] The folder holds files this connection doesn't show, so Hacivat won't move them from here.
        [name] That name can't be used here. Nothing was changed.
        [path] A folder on the way changed or became a link, so Hacivat stopped. Nothing was changed.
        [unavailable] The library folder isn't available right now. Nothing was changed.
        [trash] The system's trash refused it. The file is where it was.
       *[io] The system refused it. Nothing was changed.
    }
# Surface 2 (Context menus). Palette title.
hv-cmd-scripts-open = Open script
# Surface 2 (Context menus). Palette title.
hv-cmd-scripts-new-script = New script
# Surface 2 (Context menus). Palette title.
hv-cmd-scripts-new-folder = New script folder
# Surface 2 (Context menus). Palette title; no default chord (controller ruling c).
# Variables:
#   $os (String) - selector: the host (macos, windows, linux); one of linux, macos, windows (anything else: the default).
hv-cmd-scripts-trash =
    { $os ->
        [windows] Move script to the Recycle Bin
       *[other] Move script to the Trash
    }
# Surface 2 (Context menus). Palette title.
hv-cmd-scripts-copy-path = Copy script path
# Surface 2 (Context menus). Palette title.
hv-cmd-scripts-copy-relative-path = Copy script relative path
# Surface 2 (Context menus). Palette title.
hv-cmd-scripts-refresh = Refresh script library
# Surface 2 (Context menus). Under the name field when the name holds invisible or bidi characters, shown escaped.
# Variables:
#   $name (String) - the name with invisible characters written as <U+XXXX> (data), e.g. "a<U+202E>lqs.sql".
hv-scripts-name-hidden-chars = This name holds hidden characters: { $name }
# Surface 2 (Context menus). Moved to Trash, but a folder on the way changed meanwhile.
# Variables:
#   $os (String) - selector: the host (macos, windows, linux); one of linux, macos, windows (anything else: the default).
#   $name (String) - the file or folder name (data), e.g. "orders_report.sql".
hv-scripts-trashed-changed =
    { $os ->
        [windows] Moved { $name } to the Recycle Bin, but a folder on the way changed meanwhile. Check the Recycle Bin.
       *[other] Moved { $name } to the Trash, but a folder on the way changed meanwhile. Check the Trash.
    }
## m4b-t14-tabs

# Surface 5 (Script tab): the tab while its file is read.
# Variables:
#   $file (String) - the file name (data), e.g. "orders_report.sql".
hv-scripts-opening = Opening { $file }…
# Surface 5 (Script tab): the file could not be read; the tab stays and runs nothing.
# Variables:
#   $file (String) - the file name (data), e.g. "orders_report.sql".
hv-scripts-open-failed = { $file } couldn't be opened.
# Surface 5 (Script tab).
# Variables:
#   $reason (String) - selector: why; one of missing, library, scope, binary, refused, lossy, encoding (anything else: the default).
#   $conn (String) - the connection's name (data), e.g. "orders-staging".
hv-scripts-open-failed-body =
    { $reason ->
        [missing] It isn't in the library any more.
        [library] Its library isn't attached or can't be reached now.
        [scope] It isn't listed under { $conn }.
        [binary] It looks like a binary file, not text.
        [refused] Its path isn't one a script may have, or it leads out of the library.
        [lossy] Saving it back would change bytes that weren't edited, so it isn't opened.
        [encoding] It doesn't decode in the encoding you chose.
       *[other] The file couldn't be read.
    }
# Surface 5 (Script tab): a PostgreSQL or SQLite script is opened only on the database its session is on (R-M4b-25).
# Variables:
#   $file (String) - the file name (data), e.g. "orders_report.sql".
#   $conn (String) - the connection's name (data), e.g. "orders-prod".
#   $database (String) - the database the script was opened under (data), e.g. "orders_archive".
#   $current (String) - the database the connection runs on (data), e.g. "orders".
hv-scripts-open-other-db = { $file } wasn't opened: { $conn } runs on { $current }, not { $database }.
# Surface 5 (Script tab).
# Variables:
#   $file (String) - the file name (data), e.g. "orders_report.sql".
#   $conn (String) - the connection's name (data), e.g. "orders-prod".
#   $database (String) - the database the script was opened under (data), e.g. "orders".
hv-scripts-open-db-unknown = { $file } wasn't opened: { -brand-hacivat } can't tell which database { $conn } runs on, so it can't be sure it is { $database }.
# Surface 5 (Script tab): a MySQL profile set to one session cannot give a script tab its own session on another database.
# Variables:
#   $file (String) - the file name (data), e.g. "kampanya_özet.sql".
#   $conn (String) - the connection's name (data), e.g. "api-dev".
#   $database (String) - the database (data), e.g. "kampanya".
hv-scripts-open-single-session = { $file } wasn't opened: { $conn } uses one session for every console, so a script can't run on { $database } beside it.
# Surface 5 (Script tab): the connection the tab was opened under no longer exists.
# Variables:
#   $file (String) - the file name (data), e.g. "orders_report.sql".
hv-scripts-open-no-profile = { $file } wasn't opened: its connection no longer exists.
# Surface 5 (Script tab): the session's database could not be read before Run (R-M4b-27).
# Variables:
#   $bound (String) - the database the tab is bound to (data), e.g. "orders".
hv-scripts-run-refused-unknown = Run refused: this tab runs on { $bound }, but { -brand-hacivat } can't tell which database its session is on.
# Surface 5 (Script tab).
hv-scripts-run-refused-unknown-body = Nothing was sent. Reconnect, or open the script under another connection.
# Surface 5 (Script tab): the MySQL switch made when the tab opened failed.
# Variables:
#   $database (String) - the database (data), e.g. "kampanya".
hv-scripts-switch-failed = This tab's session couldn't switch to { $database }.
# Surface 5 (Script tab).
hv-scripts-switch-failed-body = Run stays off, so nothing runs on the wrong database. Close the tab and open the script again to retry.
# Surface 9 (Large files): the status bar's segment of a T2 preview tab.
hv-scripts-preview-segment = Read-only preview
# Surface 9 (Large files): why Save does nothing on a preview.
hv-scripts-preview-no-save = A read-only preview isn't saved: it holds only the start of the file.
# Surface 9 (Large files): why Run does nothing on a preview.
hv-scripts-preview-no-run = A read-only preview doesn't run. Run as script sends the whole file.
# Surface 6 (Changes on disk): a save failed; the tab keeps its edits.
# Variables:
#   $file (String) - the file name (data), e.g. "orders_report.sql".
hv-scripts-save-failed = Couldn't save { $file }
# Surface 6 (Changes on disk).
# Variables:
#   $reason (String) - selector: why; one of path, scope, damaged, missing (anything else: the default).
#   $conn (String) - the connection's name (data), e.g. "orders-staging".
hv-scripts-save-failed-body =
    { $reason ->
        [path] A folder on its path changed, or it became a link, so nothing was written. Your edits are still in this tab.
        [scope] It isn't listed under { $conn } any more, so nothing was written. Your edits are still in this tab.
        [damaged] The file couldn't be written and couldn't be put back. Your text is kept in this tab and in a recovery copy.
        [missing] Its library isn't attached or can't be reached now, so nothing was written. Your edits are still in this tab.
       *[other] Nothing was written. Your edits are still in this tab.
    }
# Surface 6 (Changes on disk): the save went through, but a folder check after it failed.
# Variables:
#   $file (String) - the file name (data), e.g. "orders_report.sql".
hv-scripts-saved-path-changed = Saved { $file }, but a folder on its path changed while it was saved. Check that it is where you expect.
# Surface 6 (Changes on disk): a rename on disk the tab follows (carries-w1 T14).
# Variables:
#   $path (String) - the new path in the library (data), e.g. "reports/haftalık_rapor.sql".
hv-scripts-moved = Moved to { $path }
# Surface 6 (Changes on disk).
# Variables:
#   $conn (String) - the connection's name (data), e.g. "orders-staging".
hv-scripts-moved-body = The file was moved on disk and this tab follows it. It still runs only on { $conn }.
# Surface 6 (Changes on disk): the Deleted on disk notice's second button.
hv-scripts-disk-close = Close tab
# Surface 12 (Recovery): a recovered copy whose writer may still be running (R-T5-5).
hv-scripts-recover-unknown = This copy may belong to another { -brand-hacivat } window that is still open. It is kept until you save or discard it.
# Surface 12 (Recovery): the launch notice's list line.
# Variables:
#   $file (String) - the file name (data), e.g. "orders_report.sql".
#   $conn (String) - the connection's name (data), e.g. "orders-prod".
hv-scripts-recover-item = { $file } · { $conn }
# Surface 12 (Recovery).
# Variables:
#   $file (String) - the file name (data), e.g. "orders_report.sql".
hv-scripts-recover-item-unbound = { $file } · no connection
# Surface 10 (Encoding): an encoding the user can open the file in (one button each).
# Variables:
#   $encoding (String) - the encoding's name (data), e.g. "windows-1254".
hv-scripts-enc-open-as = Open as { $encoding }
# Fix round 1 (T14 review).
# Surface 5 (Script tab): a MySQL/MariaDB script tab never shares a session (R-T14-3); the profile cannot give it one of its own.
hv-scripts-one-session = This connection allows one session; close the console or raise the limit.
# Surface 6 (Changes on disk): why a "No longer listed" tab is off (R-T14-4); the file moved where this connection does not show it.
hv-scripts-unlisted-gone = It moved to a place this connection doesn't show.
# Surface 6 (Changes on disk).
# Variables:
#   $folder (String) - the environment folder (data), e.g. "@production".
hv-scripts-unlisted-env = It is in { $folder }, a folder for another environment.
# Surface 6 (Changes on disk): Run and Save are off on a tab whose file is no longer listed (R-T14-4).
hv-scripts-unlisted-off = Run and Save are off in this tab, so it never runs or writes the file where it no longer belongs.
# Surface 5 (Script tab): why "Open under another connection…" is disabled (a sentence, N2).
hv-scripts-open-under-none-reason = No other connection shows this file.
# Surface 5 (Script tab): why "Open under another connection…" is disabled until the chooser can pick (production is never chosen for you).
hv-scripts-open-under-chooser = Open it from the Scripts node of the connection you want.
# Surface 6b (Unsaved changes): the prompt's subtitle (I5).
# Variables:
#   $env (String) - the environment word, e.g. "PRODUCTION".
#   $conn (String) - the connection's name (data), e.g. "orders-prod".
#   $database (String) - the database (data), e.g. "orders".
hv-scripts-unsaved-subtitle = { $env } · { $conn } · { $database }
# Surface 6b (Unsaved changes): the subtitle of a tab with no connection.
hv-scripts-unsaved-subtitle-unbound = No connection
# m4b-t14-tabs fix round 3 (R-T14-5): the first Run after a launch restore follows a SQLite
# file that is now on another volume (same path and inode).
# Variables:
#   $path (String) - the database file's path (data), e.g. "/Volumes/data/orders.db".
hv-scripts-sqlite-remounted = The database file is on a different volume now; running on { $path }.

## m4b-t15-goto
# Surface 7 (Go to script). Palette title of the tree row's "Open under another connection…" (the chooser without the row's connection); no default chord.
hv-cmd-scripts-open-under = Open script under another connection…

## m4b-t16-attach

# The command behind the database menu's "Map to a script library…" (menu and palette).
hv-cmd-scripts-map = Map to a script library…
# The command behind "Script library settings…" (Settings › Script libraries).
hv-cmd-scripts-library-settings = Script library settings…
# Surface 3 (Attach dialog): why the folder can't be attached; nothing was written.
# Variables:
#   $reason (String) - selector: why; one of envfolder, link, home, app, root, nested, notfolder, readonly, changed (anything else: the default).
#   $folder (String) - an environment folder's name, verbatim (data), e.g. "@production".
hv-scripts-attach-refused =
    { $reason ->
        [envfolder] This folder is inside the environment folder { $folder }. Attach the folder that holds { $folder } instead.
        [link] This is a link. Choose the folder it points to instead.
        [home] Your home folder can't be a library. Choose a folder of scripts inside it.
        [app] This is one of Hacivat's own folders, or holds one. Choose another folder.
        [root] A whole drive can't be a library. Choose a folder of scripts.
        [nested] This folder is inside an attached library, or holds one.
        [notfolder] This isn't a folder.
        [readonly] Hacivat can't change its list of libraries now. Settings › Script libraries says why.
        [changed] The folder changed while Hacivat wrote the file. Nothing was attached; try again.
       *[other] Hacivat couldn't attach this folder. Nothing was written.
    }
# Surface 3 (Attach dialog): an alias Hacivat can't write.
# Variables:
#   $reason (String) - selector: why; one of control, long, many (anything else: the default).
hv-scripts-attach-aliases-err =
    { $reason ->
        [control] An alias can't contain control characters.
        [long] An alias can be at most 128 bytes.
       *[many] Hacivat keeps up to 32 aliases.
    }
# Surface 3 (Attach dialog): while the chosen folder is read.
hv-scripts-attach-reading = Reading the folder…
# Surface 3 (Attach dialog): the subtitle, the connection and database it was opened from.
# Variables:
#   $conn (String) - a connection name, verbatim (data), e.g. "orders-prod".
#   $database (String) - a database name, verbatim (data), e.g. "orders".
hv-scripts-attach-from = { $conn } › { $database }
# Surface 3 (Mapping): a library in the Library select.
# Variables:
#   $library (String) - a library's folder name, verbatim (data), e.g. "orders-scripts".
#   $database (String) - a database name, verbatim (data), e.g. "orders".
hv-scripts-map-option = { $library } binds to { $database }
# Surface 3 (Mapping): why the share box can't be ticked.
# Variables:
#   $reason (String) - selector: why; one of none, unreadable (anything else: the default).
hv-scripts-map-alias-off =
    { $reason ->
        [none] This folder has no .hacivat/workspace.toml to add it to.
        [unreadable] Hacivat can't read this folder's .hacivat/workspace.toml, so it can't add to it.
       *[name] This name can't be an alias.
    }
# Surface 3 (Mapping): why nothing was saved.
# Variables:
#   $reason (String) - selector: why; one of changed, unavailable (anything else: the default).
hv-scripts-map-failed =
    { $reason ->
        [changed] .hacivat/workspace.toml changed after this dialog read it. Nothing was saved; look at it again.
        [unavailable] The library's folder isn't available now. Nothing was saved.
       *[other] Hacivat couldn't save the mapping. Nothing was changed.
    }
# Surface 4 (Settings › Script libraries): how a library's changes are found (R-T9-1).
hv-scripts-set-watch = Watch
# Surface 4 (Settings › Script libraries): the watcher's own choice.
hv-scripts-set-watch-auto = Automatic
# Surface 4 (Settings › Script libraries): checking the folder every 2 seconds.
hv-scripts-set-watch-poll = Every 2 seconds
# Surface 4 (Settings › Script libraries): why a library is polled, when the user chose it.
hv-scripts-set-watch-chosen = You chose to check this folder every 2 seconds.
# Surface 4 (Settings › Script libraries): states beyond the mock's four.
# Variables:
#   $state (String) - selector: the state; one of moved, envfolder, unreadable, notresponding, unwatched (anything else: the default).
hv-scripts-set-state =
    { $state ->
        [moved] This isn't the folder that was attached: it was moved or replaced.
        [envfolder] This folder is inside an environment folder, so nothing in it is shown.
        [unreadable] Hacivat can't read this folder.
        [notresponding] The folder isn't answering.
        [unwatched] Nothing reports changes in this folder; Refresh shows them.
       *[scanning] Reading the folder…
    }
# Surface 4 (Settings › Script libraries): the confirmed fix for a moved or replaced folder.
hv-scripts-set-adopt = Use this folder…
# Surface 4 (Settings › Script libraries).
# Variables:
#   $library (String) - a library's folder name, verbatim (data), e.g. "orders-scripts".
hv-scripts-adopt-title = Use this folder as { $library }?
# Surface 4 (Settings › Script libraries).
# Variables:
#   $path (String) - a folder path, verbatim (data), e.g. "~/work/orders-scripts".
hv-scripts-adopt-body = Hacivat stopped showing this library because the folder at { $path } is not the one that was attached. If you moved or restored it yourself, use it: the library keeps its local mappings. If you don't know why it changed, look at the folder first.
# Surface 4 (Settings › Script libraries).
hv-scripts-adopt-btn = Use this folder
# Surface 4 (Settings › Script libraries): the Remove and Use this folder questions' subtitle.
# Variables:
#   $path (String) - a folder path, verbatim (data), e.g. "~/work/orders-scripts".
hv-scripts-remove-path = { $path }
# Surface 11 (Diagnostics): nothing watches the library (R-T9-1: the poll suggestion).
# Variables:
#   $folder (String) - a library's folder name, verbatim (data), e.g. "orders-scripts".
hv-scripts-diag-unwatched = Nothing reports changes in { $folder }. If edits made outside Hacivat don't show up, choose Watch: Every 2 seconds in Settings › Script libraries.
# m4b-t16-attach fix round 1 (I1, R-T10-7): the manifest can't be read and the library keeps the binding its last good read gave.
# Variables:
#   $folder (String) - a library's folder name, verbatim (data), e.g. "billing".
#   $database (String) - a database name, verbatim (data), e.g. "orders".
hv-scripts-diag-toml-kept = .hacivat/workspace.toml in { $folder } can't be read, so the library keeps its last binding, { $database }, until the file is fixed.
# m4b-t16-attach fix round 1 (I4): why a found .hacivat/workspace.toml can't be read.
# Variables:
#   $reason (String) - selector: why; one of link, notfile, changed, unreadable, large, notutf8, deep, invalid, noformat, badformat, newer, database (anything else: the default).
hv-scripts-manifest-reason =
    { $reason ->
        [link] it is a link, which Hacivat never follows
        [notfile] it is not a file
        [changed] it changed while it was read
        [unreadable] it can't be opened
        [large] it is over 64 KB
        [notutf8] it is not UTF-8 text
        [deep] it is nested too deeply
        [invalid] it is not valid TOML
        [noformat] it has no format line
        [badformat] its format is not a number
        [newer] a newer Hacivat wrote it
        [database] its database name is not valid
       *[other] it can't be understood
    }
# m4b-t16-attach fix round 1 (I4): the attach dialog over a found file it can't read.
# Variables:
#   $reason (String) - why, an hv-scripts-manifest-reason text, e.g. "it is not valid TOML".
#   $folder (String) - a library's folder name, verbatim (data), e.g. "orders-scripts".
hv-scripts-attach-found-broken = Found .hacivat/workspace.toml, but it can't be read ({ $reason }); the library will bind by its folder name, { $folder }, until it is fixed.
# m4b-t16-attach fix round 1 (m1): a long found file is shown up to a cap.
# Variables:
#   $n (Number) - a count, e.g. 3.
hv-scripts-attach-more-lines =
    { $n ->
        [one] … and { $n } more line
       *[other] … and { $n } more lines
    }
# m4b-t16-attach fix round 1 (I6): an action in Settings › Script libraries did nothing.
# Variables:
#   $reason (String) - selector: why; one of readonly, state, refused (anything else: the default).
hv-scripts-set-failed =
    { $reason ->
        [readonly] Hacivat can't change its list of libraries now, so nothing was changed.
        [state] The library changed meanwhile, so nothing was done.
        [refused] Hacivat can't use this folder as the library; nothing was changed.
       *[other] Hacivat couldn't save this change; nothing was changed.
    }
# m4b-t16-attach fix round 1 (m7): Remove… while script tabs from the library have unsaved changes.
# Variables:
#   $n (Number) - a count, e.g. 3.
hv-scripts-remove-dirty =
    { $n ->
        [one] { $n } open script tab from it has unsaved changes. Removing the library discards its recovery copy, so save it first.
       *[other] { $n } open script tabs from it have unsaved changes. Removing the library discards their recovery copies, so save them first.
    }

## m4b-t17-secrets-ui

# Surface 8 (Secret warning): the gutter mark and the dialog of a script whose secret scan couldn't run (fail closed).
hv-scripts-secret-unscanned = Couldn't be checked for secrets
# Surface 8 (Secret warning): the unscanned mark's tooltip line.
hv-scripts-secret-unscanned-hint = Saving asks first.
# Surface 8 (Secret warning): the dialog's title when the scan couldn't run.
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
hv-scripts-secret-unscanned-title = { $file } couldn't be checked for secrets
# Surface 8 (Secret warning): the dialog's text when the scan couldn't run.
hv-scripts-secret-unscanned-body = { -brand-hacivat } couldn't check this text for passwords or tokens, so it asks before saving it into the library.
# Surface 12 (Recovery): the tab's recovered text looked like it held a password or a token when it was copied (never which, never the value; R-T5-4).
hv-scripts-recover-secret = Some lines of this text look like a password or a token. Saving asks first.

## m4b-t18-run-as-script
# Surface 9 (Large files): the strip while pass 1 reads the whole file and runs nothing (R-M4b-47).
hv-scripts-run-checking = Checking the whole file before it runs
# Surface 9 (Large files): the user answered No (or Cancel) to a statement's confirmation (R-T12-2).
# Variables:
#   $n (Number) - the statement's position in the file, e.g. 1205.
hv-scripts-run-declined = Stopped at statement { $n }: you didn't confirm it. The statements before it ran and stay applied; nothing after it was sent.
# Surface 9 (Large files): the strip's button once the run ended.
hv-scripts-run-close = Close
# Surface 9 (Large files): the confirmation of a statement in Run as script (T12 m4): the run has not read the later statements yet, so Execute all and Skip all name no count.
# Variables:
#   $env (String) - the connection's environment word (an hv-conn-env-* message), e.g. "staging".
hv-scripts-confirm-open-ended = Execute all also runs, without asking, every later statement in the file that would ask on this connection ({ $env }); a statement that needs its name typed still asks. Skip all skips every later statement that would ask.
# Surface 9 (Large files): as hv-scripts-confirm-open-ended where Execute all is not offered (production).
# Variables:
#   $env (String) - the connection's environment word (an hv-conn-env-* message), e.g. "production".
hv-scripts-confirm-open-ended-skip = Skip all skips every later statement in the file that would ask on this connection ({ $env }).
# Surface 9 (Large files): the name a pasted preview's run goes by (the strip, the history row).
hv-scripts-pasted-name = Pasted text
# Surface 9 (Large files): "Paste as preview" in a console with no connection (the preview opens on the console's connection).
hv-scripts-paste-preview-unbound = Paste as preview opens on this console's connection, and it has none.
# m4b-t18-run-as-script fix round 1 (I1): Run (or Run as script) refused while a script runs on the same session; two runs never interleave.
hv-scripts-run-session-busy = A script is running on this connection's session. Wait for it, or stop it, then run again.
# m4b-t18-run-as-script fix round 1 (m4, R-T18-2, PO informed): Stop cancelled the statement that was running.
# Variables:
#   $n (Number) - the statement's position in the file, e.g. 1205.
hv-scripts-run-cancelled-during = Stopped during statement { $n }; statements after it were not sent.
