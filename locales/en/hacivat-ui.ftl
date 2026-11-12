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
