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
# The fix for an untrusted or mismatched certificate (R3): TLS without verification.
hv-conn-fix-require = Encrypt without verifying
# Shown with that fix: what it gives up, plainly.
hv-conn-fix-require-note = The connection will be encrypted, but { -brand-hacivat } won't check that the server is who it claims to be.
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
hv-conn-issue-tls-key-encrypted = The private key is encrypted. Use an unencrypted key file.
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
# Review: PO
hv-conn-issue-tls-key-legacy-encrypted = This private key uses an old encryption format. Convert it to PKCS #8.

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
