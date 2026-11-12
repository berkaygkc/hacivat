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
hv-conn-dlg-key-pass = Key passphrase
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
