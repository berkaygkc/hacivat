# M2a Connect: design notes

Clickable mock: `design/m2-connect/index.html` (one file, one app, surface switcher). It runs with no network except Google Fonts, the same as harman. The Lucide icons (0.460, ISC) are inlined as an SVG sprite.

**How to drive it**
- The dashed **mock bar** at the top is not product UI. It holds the surfaces `1`–`8`, the scenarios (`[` / `]`), EN/TR (`L`), dark/light (`T`) and Notes (`N`).
- The Notes drawer repeats these notes for each surface. It also lists every string on the current surface, with a **PO review** marker on each Turkish string.
- Deep links: `?s=3&sc=db&theme=light&lang=tr&notes=1`.
- Every ⌘ shortcut also works with Alt or Ctrl, because the browser keeps ⌘T and ⌘N for itself.

**Flows that run**
- Paste a URL into any field and the form fills.
- Test (⌘T) runs the pipeline, then shows a result or an inline error with fixes.
- Save (⌘S) or Save and connect (⌘↵) on a remote host opens the production-tag prompt. The connection is then saved, the capsule goes from connecting to connected, and the Explorer updates.
- Errors have working fixes:
  - re-enter the password, then connect
  - pick an existing database, then connect
  - review the certificate, then pin it, then test again
  - connect through SSH, which opens the SSH section
- Import from `~/.ssh/config`.
- Explorer context menu, from the keyboard (Shift-F10, ↑/↓, ↵) or from a right-click.
- Copy as URL, delete with confirmation, duplicate.
- Esc cancels a connect that is in progress.

Product truth used: spec L15 (M2a scope), §4 (Connections and profiles; TLS and SSH; Secrets; Safety), D-004, D-005, D-016, D-035, D-037, D-085, and the research docs `algorithms/01` (HV-DRV-04, HV-CONN-01/03/05/07, HV-SEC-01..05), `datagrip/02` and `clients/dbeaver-01`. The visual world is harman, unchanged. Shell metrics come from `hacivat-ui`: the Dialog card is 420 px with 16 px padding and a 12 px gap, menu rows are 24 px with a 200 px minimum width, the TextInput is 26 px, and the scrim is ground at 0.6.

---

## Cross-cutting decisions

1. **The capsule is never dimmed (decision M2a-1).** The dialog scrim starts below the 40 px title bar, so environment and connection state stay readable behind every dialog (Principle 1, "trust is visible"). This changes `Layers::elements` in `shell/layers.rs`, where the scrim currently covers the whole window. See PO question 1.
2. **Colour tied to the tag means production red plus shape-coded neutral marks.** Users do not pick free colours. Circles are for personal or pre-release environments: local ○ (existing), dev ◉ (NEW) and test ● (NEW). Squares are for shared servers: staging □ (existing) and production ■ in red (existing). Every place that shows an environment (Segmented, Explorer group and row, capsule, status bar, sheet, prompt) uses the same mark. See PO question 2.
3. **No new hue for errors.** Hue Is Meaning stays intact:
   - A failure uses an ink-coloured alert glyph, the ✕ in the step trail, and plain words.
   - Amber appears only as the splash's 7 px run marker, which means "in progress" (a pending state).
   - Green means a step or connection succeeded, or the path is direct.
   - Red appears only for production and for destructive actions.
4. **Raised Ink Rule inside dialogs.** `.dlg` remaps text-3 to text-2. Field wells (TextInput, Select, Segmented, inset boxes) restore text-3 on their own panel or ground fill.
5. **Segmented inside a dialog sits in a field well.** The well is panel in dark and ground in light. The pressed segment is raised in dark and panel in light, which is harman's own rule applied to a raised surface. The pressed label is weight 600.
6. **Format locale.** In the mock, numbers and dates follow the language toggle as a stand-in. In the product they follow the OS region (L15 d), for example `0,06 sn` and `14 Mart 2027`.
7. **No motion beyond progress.** Dialogs and menus appear instantly. The only timed things are:
   - The fields filled from a pasted URL keep the blue-tint for 1.6 s, then fade over 240 ms (the state token).
   - The running clocks in the capsule and the test strip.

## Surfaces

### 1 · New / Edit connection dialog
**Intent.** One calm column, edited top to bottom. Everything the user must decide is visible, and everything they rarely touch is folded away. The URL row mirrors the form live, so the form and the URL are the same object seen two ways.

- **Size.** The dialog is 640 px wide (NEW size), with a header, a scrolling body, the test strip and the footer, separated by seams.
- **Order.** URL, Engine (Segmented: PostgreSQL / MySQL / MariaDB / SQLite), Name, Environment (Segmented with marks), Read-only, then Server (Host + Port, Database, User), then Authentication, then the TLS, SSH and Advanced disclosures.
  - SQLite swaps Server for File + Choose… and two checkboxes: open the file read-only, and create it if missing.
  - MySQL and MariaDB hide pgpass.
- **Initial focus is Host.** The fastest typed path starts there. Paste works in every field, so a separate import step isn't needed.
- **Environment.**
  - For a remote host it starts unset, with the hint "Asked when you save" (D-004 B), and the prompt on save decides.
  - Loopback addresses, sockets and SQLite files become local automatically.
- **Read-only.** This checkbox sits next to Environment, not under Advanced, because it is a trust decision.
- **Authentication** is a Segmented: Password / pgpass / Client certificate / None.
  - Password shows the password field next to **Remember**, a Select with four options: Never (ask on every connect), Until restart (kept in memory), For this session (forgotten on disconnect), Forever (Keychain).
  - The default is Forever. See PO question 4.
  - pgpass shows the matched line, with the password masked, and says that Hacivat stores no password.
- **Disclosures** show a live summary when folded. For example, "Auto: verify when the server offers TLS", "via bastion.acme.io · 1 jump host", or "Timeout 15 s · keep-alive off".
- **Advanced** holds the connect timeout, keep-alive, application name, the connection budget stated as fact ("Up to 4 open at once", D-035), and extra properties.
- **Test connection** (⌘T) runs the HV-CONN-07 pipeline and shows the result in a strip above the footer, so it is visible at any scroll position.
  - While it runs, the strip shows the step trail with the run marker and a clock. Esc stops the test.
  - On success it shows "Connected in 54 ms", the line "Nothing was saved…", the trail with a duration for each step, and diagnostics: server and version, latency (median of 3), sign-in user and method, warnings, and encryption with version, state and cipher. A Copy report button sits on the right.
  - On failure it shows the same error block as surface 3.
- **Footer.** Test sits on the left. Cancel, Save (⌘S) and the single primary, Save and connect (⌘↵), sit on the right.
- **Keyboard path.**
  - Tab moves through URL → Engine → Name → Environment → Read-only → Host → Port → Database → User → Auth → Password → Remember → TLS → SSH → Advanced → Test → Cancel → Save → Save and connect, then wraps around (the focus is trapped in the dialog).
  - Arrow keys move inside a Segmented (roving tabindex).
  - Space or ↓ opens a Select. Space toggles a checkbox or a disclosure.
  - Esc closes the dialog, or stops a running test first.
  - On Windows and Linux, the existing Dialog access keys apply.

### 2 · URL paste, both directions
**Intent.** Paste is the fastest way in. The secret never lives in a URL: it is split off on paste and never written on copy.

- **Accepted input.** Paste into any field: `postgres://`, `postgresql://`, `mysql://` and `mariadb://` URLs, `jdbc:` URLs, a libpq `key=value` string, or a SQLite path or `file:` URI.
- **What changes.** The fields fill in, the filled fields keep the blue-tint for 1.6 s, and the Name is suggested from the database and host.
- **The notice says exactly what happened:**
  - The password was moved out of the URL and goes to the Keychain on save. The text is honest that it is stored only on save.
  - `sslmode=verify-full` became Encryption: Verify full.
  - `application_name` became the application name.
  - An unknown parameter was kept as an extra property.
  - The notice has an Undo button.
- **A URL for another engine** is never applied silently when the form already has input (HV-DRV-04). Instead the notice asks: "This is a MySQL URL, and the form is set to PostgreSQL", with the buttons Switch to MySQL and fill / Keep PostgreSQL. An untouched form switches engine and says so.
- **Text that isn't a URL** changes nothing, and the notice lists the accepted formats.
- **Copy as URL** is available as the button in the URL row, as ⌘⇧C, and in the Explorer menu. A toast shows the exact URL with "The password isn't included". When the profile uses SSH, the toast adds that the tunnel can't be written into a URL.

### 3 · Connect errors with quick fixes
**Intent.** An error is a sentence about the user's situation, a trail that shows where the connection broke, and one-click fixes. The raw server text is one disclosure away, never the headline.

- **Where errors appear.** Connecting from the Explorer or the capsule opens a 480 px dialog (NEW size), and the capsule shows Failed with a Retry segment. In the connection dialog, the same block appears inline in the test strip.
- **Anatomy.**
  - A human message, built from what Hacivat sent (the user and the host) and never parsed from server text.
  - An explanation.
  - The step trail, with a ✕ at the stage that failed.
  - The fixes, as buttons.
  - "Technical details": stage, class id, code chip, the server message verbatim, the elapsed time, and Copy details (English).
- **The five errors.**

| Class | Message | Fixes |
|---|---|---|
| `auth.invalid` (28P01) | The server rejected the password for deploy. | **Enter password again** opens an inline field, then Connect. The dialog counts attempts and stops at 3 to avoid a lockout. Also Edit connection. |
| `network.dns` | dev-db.acme.internal couldn't be found. | Try again, Edit connection, Connect through SSH… The body mentions the VPN. |
| `tls.untrusted` | db-qa.acme.io sent a certificate this Mac doesn't trust. | **Review certificate…** opens surface 5 (this is what remains after the user cancelled the TLS prompt). |
| `config.unknown_database` (3D000) | The database orders_v2 doesn't exist on db-stg.acme.io. | One button per database the user can open (**Use orders**, Use orders_archive, Use postgres). Each sets the database and connects. |
| `server.unsupported_version` (NEW class id) | crm-db.acme.io runs PostgreSQL 9.6.24, which Hacivat doesn't support. | Supported versions ↗, Edit connection, and a ghost button **Connect anyway, unsupported**. See PO question 3. |

- **Classification** comes from the code first, never from localized server text: a Turkish server's `HATA` still maps to the right title.

### 4 · Production-tag prompt (D-004)
**Intent.** A quiet question, not an alarm. The title is neutral, there is no warning icon, and the consequences are written in plain words using harman's own marks.

- **When it appears.** On the first save of a profile whose host isn't on this Mac. It stacks over the connection dialog.
- **Title.** "How careful should Hacivat be with orders-prod-us?" The body says the host isn't on this Mac and that the choice can be changed later.
- **The choice list (NEW)** has four rows (Production, Staging, Test, Dev), each with its mark and a one-line description. Local isn't offered, because the host was already judged not to be local.
- **With production selected,** the prompt lists what changes:
  - The capsule turns red. The prompt shows this as a red word in small caps with the swatch, never as a second filled badge (One Filled Badge Rule).
  - Statements that change data or schema ask for confirmation.
  - Commit is manual.
- **With any other tag selected,** it shows one calm line: "No extra confirmations. Statements commit as they run."
- **Suggestion.** The prompt preselects Production only when the host or name contains prod, prd or live, and then shows "Suggested because the host name contains “prod”". Otherwise nothing is selected and the primary button reads "Pick an environment", disabled. See PO question 5.
- **The primary button names the result:** "Save as production" / "Üretim olarak kaydet". Enter confirms. Esc returns to the form unchanged.

### 5 · TLS certificate decision (D-005 Auto)
- **Untrusted certificate.** A 480 px dialog that appears while the capsule shows "Connecting · TLS".
  - It shows who the certificate was issued to and by, the expiry date (formatted by locale), the SHA-256 fingerprint (two lines of 16 bytes, with a copy button) and the chain: server → intermediate → root, "not trusted by this Mac".
  - It states the rules: pinning trusts exactly this certificate for host:port, a change stops the connection and asks again, and a pin doesn't extend the expiry date.
  - Actions: Cancel (left), Import CA… and **Pin this certificate** (primary). Pin is primary because it is the precise, reversible answer.
- **Unencrypted badge.** Plaintext is allowed only for loopback and private addresses. The badge is permanent and appears in four places: a capsule segment (lock-open + "unencrypted"), a status-bar segment, the Encryption row of the connection sheet, and a flag on the Explorer row. It has no hue: it is a fact, not an alarm. Clicking the capsule segment opens a popover that explains why plaintext was allowed and how to remove the badge.
- **Public host without TLS.** A 420 px dialog says the password and results would cross the network in plain text, with the checkbox "Don't ask again for this connection" and the buttons Cancel, Connect without encryption and **Use an SSH tunnel…** (primary).

### 6 · SSH tunnel settings (HV-SEC-01)
- **Layout.** Inside the SSH disclosure: the checkbox "Connect through an SSH tunnel" and **Import from ~/.ssh/config…**, then SSH host + port, SSH user, auth (Segmented: Agent / Key file / Password / Keyboard-interactive) with fields that depend on the auth method, and the jump hosts.
- **Jump hosts** form an ordered list. Each row has a number, a host, a user and a remove button, and the list ends with Add jump host.
- **Route row.** It draws the real path: this Mac → jump-eu.acme.io → bastion.acme.io → db-prod-eu.acme.io:5432.
- **Host keys** is a Select: Ask for new hosts (default) / Accept new, stop on change / Known hosts only. There is never an "accept everything" option.
- **Import menu.** It lists the `Host` aliases with what each one resolves to. A ProxyCommand entry is shown but disabled: "uses ProxyCommand, not supported" (D-037). Picking an entry fills the fields, highlights them and shows a notice.
- **New host key prompt.** A 420 px dialog with the ED25519 SHA256 fingerprint and the note "Hacivat saves it in its own known-hosts list. ~/.ssh/known_hosts isn't changed." Buttons: Cancel and **Trust and connect**.

### 7 · Live capsule states
These states are the harman capsule, extended. For production, the filled PRODUCTION block always stays first.
- **No connection:** a database glyph + "No connection" + chevron.
- **Connecting:** env, name and engine, then a segment with the 7 px amber run marker, "Connecting", the current step in mono and a running clock. Esc cancels, and a toast confirms the cancel.
- **Connected (not production):** env mark + tag, name and engine, read-write (text-2) or read-only (red lock), the path (green "Direct" + latency, or the route glyph with "SSH via bastion" + latency), and an "unencrypted" segment when it applies.
- **Connected (production):** the filled block, then the same segments as above.
- **Failed:** an alert glyph + "Couldn't connect", then a Retry segment. Clicking the capsule opens the error dialog.
- **Reconnecting:** the run marker, "Reconnecting · attempt 3" and "in 4 s" (HV-CONN-05 backoff).
- **Session lost (D-085):** the capsule shows connected again, and a sticky toast (NEW variant) titled "Reconnected to orders-prod" lists what was rolled back and what is gone, with Show details and Dismiss.
- **The status bar mirrors the capsule:** environment segment, state or path, encryption, and engine version.
- **▶ Play** runs a full connect through SSH.

### 8 · Explorer list of saved connections
- **No tree yet (M4).** The list is flat, grouped by environment in severity order (production, staging, test, dev, local), using harman's small-caps group header with a rule and a count.
- **Rows** show an engine glyph, the name, the environment mark, flags (read-only lock in red, SSH route, unencrypted) and, on the right, the engine and major version. A live row shows a green ✓ + latency, the run marker while connecting, or ⚠ after a failure.
- **Selection** uses harman's blue-tint with a 1 px blue edge. The connection sheet below the list follows the selection.
- **Keyboard.** ↑/↓ select, ↵ connects (or disconnects), ⌘E edits, ⌘D duplicates, ⌘⇧C copies as URL, ⌫ deletes, Shift-F10 opens the menu. Right-click and double-click work too.
- **Context menu.** Connect (or Disconnect), Edit…, Duplicate, Copy as URL, separator, Delete… in red.
- **Delete** asks for confirmation in an alertdialog. It says the Keychain entry is removed too, that scripts and history stay, and that the delete can't be undone. The danger button is outline-only until hover or focus, and initial focus is on Cancel.
- **Empty state.** "No connections yet", one sentence, then the primary New connection ⌘N, Paste URL ⌘⇧V, and a ghost **Import pg_service.conf…** (HV-SEC-04).
- **The centre pane** has no query console in M2 (L15 c). It shows the keyboard entry points instead.

---

## NEW primitives

All of these use harman tokens only, and every one is reproducible in GPUI: solid fills, 1 px borders, radii 0/3/4/6, the popover shadow only, no transforms, no blur, no masks.

| Primitive | Anatomy and exact tokens |
|---|---|
| **Dialog sizes** | Existing Dialog anatomy (raised, 1 px seam-strong, radius 6 `md`, popover shadow, padding 16 `3xl`). Widths: 420 (existing), **480 "wide"** for technical content (errors, certificates), **640 "form"** for the connection dialog. The form variant adds a header (padding 12/16, seam below), a scrolling body (padding 12/16/14) and a footer (padding 12/16, seam above). |
| **Scrim below the title bar** | `ground` at alpha 0.6 (unchanged), positioned from y = 40 px (`titlebar-height`) to the bottom of the window. |
| **Checkbox** | 14 × 14, radius 3 `xs`, 1 px `seam-strong`, fill `panel` (dark) / `ground` (light). Checked: fill and border `blue`, Lucide check at 12 px, stroke 2.2, `on-accent`. Label 13 px `text`, optional sub-label `text-2`. Focus: 1 px blue ring at 1 px offset. |
| **Select** | A field-shaped button: 26 px `field-height`, radius 4 `sm`, 1 px `seam-strong`, padding 0 6 0 9, fill `panel` / `ground`, value 13 px `text`, chevrons-up-down at 12 px `text-2`. Opens the existing PopupMenu (raised, radius 6, rows 24 px, min 200 px, active row `hover`) with a `blue` check column and an optional `text-2` hint. Keys: Space / ↓ / ↵ open it, ↑/↓ move, ↵ picks, Esc closes. |
| **Disclosure** | A 32 px row with a 1 px `seam` above. A chevron (right or down, swapped rather than rotated) at 12 px `text-2`, the title at 13 px weight 600 `text`, and the summary right-aligned at 12.5 px `text-2` with an ellipsis. Content is indented 22 px. |
| **Inline notice** | Padding 8/10, radius 4, 1 px `seam-strong`, fill `panel` / `ground`, 12.5 px `text`, secondary lines `text-2`, parameter names in Fragment Mono 11.5. The icon is 14 px `text-2` (info), or a `green` circle-check when it reports something done. Actions are existing Buttons. |
| **Step trail** | An inline list, 20 px tall, 14 px column gap, wraps without separators. Done: check 12 px `green` + label `text` + mono 11 px duration `text-2`. Running: the 7 px square `amber` run marker. Failed: ✕ 12 px `text` + label weight 600. Pending: a 7 px hollow square in `seam-strong` + label `text-2`. |
| **Choice list (radio rows)** | A group with a 1 px `seam-strong` border, radius 4, and rows divided by `seam`. Each row is at least 34 px, padding 7/10, with a 14 px radio (1 px `seam-strong`; selected: `blue` border and a 6 px `blue` dot), the 7 px environment mark, a label at 13 px weight 600 and a description at 12.5 px `text-2`. Selected row: `blue-tint`. Hover: `hover`. Arrow keys move and select. |
| **Environment marks: dev, test** | 7 px, the same family as the existing marks. dev: a hollow circle with a 1 px `text-2` border and a 3 px `text-2` centre dot (◉). test: a filled `text-2` circle (●). Circles are an established exception for environment marks only (local ○). |
| **Toast, sticky + actions** | The existing Toast (320 px; 380 when the content is wide). Sticky toasts have no TTL and use `Role::Alert`. They have a title (600), body lines in `text-2`, a list with 3 px square bullets in `text-2`, and a row of Buttons. |
| **Capsule states** | Extra segments in the existing capsule anatomy (26 px, seams `seam-strong`). Run segment: the 7 px `amber` run marker, the label in `text`, mono 11.5 `text-2` for the step and the clock. Fail segment: triangle-alert at 12 px `text`. Retry segment: rotate-cw + `text-2`, which turns `text` on hover. Unencrypted segment: lock-open at 12 px `text-2` + label `text`. Non-production tag segment: the mark + tag in `text-2`. |
| **Popover (explanation)** | The popover surface (raised, seam-strong, radius 6, popover shadow), 300 px wide, padding 10/12, a title at 13 px weight 600 with an icon, and body at 12.5 px `text-2`. |
| **Fingerprint inset** | The DML-preview anatomy reused: `panel` / `ground`, 1 px `seam-strong`, radius 4, padding 8/10, Fragment Mono 11.5/17 `text`. Not new as a primitive; listed for completeness. |
| **Chain list** | 22 px rows indented 8 and 24 px, with an L-shaped 1 px `seam-strong` tick (two borders), the subject in mono 11.5 `text` and the role in `text-2`. |

## Open questions for the PO
1. **Should a dialog dim the title bar?** I kept the capsule (environment, connection, state) undimmed by starting the scrim below the title bar. The current `Layers` host dims the whole window, so a yes means a one-line change there.
2. **Does "colour tied to the tag" mean only production red, with shape marks for the others?** The alternative is to give staging, test and dev their own hues (or let users pick colours, as DBeaver and DataGrip do), which breaks Hue Is Meaning. I chose shapes: ○ local, ◉ dev, ● test, □ staging, ■ production.
3. **For a server older than the supported versions (for example PostgreSQL 9.6), should Hacivat stop, or offer "Connect anyway, unsupported"?** The mock shows the ghost button. Dropping it makes the support boundary hard.
4. **Should new passwords default to "Forever (Keychain)"?** The other options are "For this session" and asking the first time. Forever is the fastest choice and matches DataGrip. The paste notice already says the Keychain is used only on save.
5. **Should the production prompt preselect Production when the host or name contains prod, prd or live?** The other choice is to never preselect, which forces an explicit pick every time. With no match, the mock preselects nothing and the Save button stays disabled.

---

## Copy strings (EN / TR)

The ids follow the shape of Fluent message ids. `{name}`-style placeholders become Fluent variables (`{ $name }`). Every Turkish string is **PO review**: none has been signed off yet.

| id | EN | TR | Status |
|---|---|---|---|
| `app-palette` | Search or run a command | Ara veya komut çalıştır | PO review |
| `explorer` | Explorer | Gezgin | PO review |
| `filter-conn` | Filter connections | Bağlantılarda ara | PO review |
| `new-conn-tip` | New connection | Yeni bağlantı | PO review |
| `env-local` | local | yerel | PO review |
| `env-dev` | dev | geliştirme | PO review |
| `env-test` | test | test | PO review |
| `env-staging` | staging | hazırlık | PO review |
| `env-production` | production | üretim | PO review |
| `env-prod-cap` | production | prod | PO review |
| `acc-ro` | read-only | salt okunur | PO review |
| `acc-rw` | read-write | okuma-yazma | PO review |
| `path-direct` | Direct | Doğrudan | PO review |
| `path-ssh` | SSH via {host} | {host} üzerinden SSH | PO review |
| `enc-verified` | TLS 1.3 · verified | TLS 1.3 · doğrulandı | PO review |
| `enc-pinned` | TLS 1.3 · pinned | TLS 1.3 · sabitlendi | PO review |
| `enc-unencrypted` | unencrypted | şifrelenmemiş | PO review |
| `enc-none-file` | not applicable (file) | geçerli değil (dosya) | PO review |
| `kv-env` | Environment | Ortam | PO review |
| `kv-engine` | Engine | Motor | PO review |
| `kv-host` | Host | Adres | PO review |
| `kv-file` | File | Dosya | PO review |
| `kv-user` | User | Kullanıcı | PO review |
| `kv-path` | Path | Yol | PO review |
| `kv-enc` | Encryption | Şifreleme | PO review |
| `kv-secret` | Password | Parola | PO review |
| `secret-forever` | Keychain · forever | Anahtar Zinciri · kalıcı | PO review |
| `secret-never` | asks every time | her seferinde sorar | PO review |
| `secret-session` | for this session | bu oturum boyunca | PO review |
| `secret-restart` | until restart | yeniden başlatana kadar | PO review |
| `secret-pgpass` | from ~/.pgpass | ~/.pgpass dosyasından | PO review |
| `secret-none` | none | yok | PO review |
| `sheet-meta` | connection | bağlantı | PO review |
| `sheet-none` | Select a connection to see its details. | Ayrıntılarını görmek için bir bağlantı seçin. | PO review |
| `home-title` | No query open | Açık sorgu yok | PO review |
| `home-new` | New connection | Yeni bağlantı | PO review |
| `home-paste` | New connection from a copied URL | Kopyalanan URL'den yeni bağlantı | PO review |
| `home-connect` | Connect to the selected connection | Seçili bağlantıya bağlan | PO review |
| `home-search` | Search or run a command | Ara veya komut çalıştır | PO review |
| `home-hint` | Pick a connection in the Explorer, or add one. | Gezgin'den bir bağlantı seçin ya da yeni bir tane ekleyin. | PO review |
| `sb-noconn` | No connection | Bağlantı yok | PO review |
| `sb-env` | {env} · {access} | {env} · {access} | PO review |
| `dlg-title-new` | New connection | Yeni bağlantı | PO review |
| `dlg-title-edit` | Edit {name} | {name} bağlantısını düzenle | PO review |
| `dlg-sub-new` | Not saved yet | Henüz kaydedilmedi | PO review |
| `dlg-sub-edit` | Saved · changes apply on the next connect | Kayıtlı · değişiklikler bir sonraki bağlanışta geçerli olur | PO review |
| `dlg-url` | URL | URL | PO review |
| `dlg-url-ph` | Paste a connection URL to fill the form | Formu doldurmak için bir bağlantı URL'si yapıştırın | PO review |
| `dlg-copy-url` | Copy as URL | URL olarak kopyala | PO review |
| `dlg-engine` | Engine | Motor | PO review |
| `dlg-name` | Name | Ad | PO review |
| `dlg-env` | Environment | Ortam | PO review |
| `dlg-env-ask` | Asked when you save, because this host isn't on this Mac. | Bu adres bu Mac'te olmadığı için kaydederken sorulur. | PO review |
| `dlg-env-local` | Set to local: the server runs on this Mac. | Yerel olarak ayarlandı: sunucu bu Mac'te çalışıyor. | PO review |
| `dlg-ro` | Read-only | Salt okunur | PO review |
| `dlg-ro-hint` | Block statements that change data or schema | Veriyi ya da şemayı değiştiren ifadeleri engelle | PO review |
| `sec-server` | Server | Sunucu | PO review |
| `dlg-host` | Host | Adres | PO review |
| `dlg-port` | Port | Port | PO review |
| `dlg-db` | Database | Veritabanı | PO review |
| `dlg-user` | User | Kullanıcı | PO review |
| `dlg-file` | File | Dosya | PO review |
| `dlg-choose` | Choose… | Seç… | PO review |
| `dlg-sqlite-ro` | Open the file read-only | Dosyayı salt okunur aç | PO review |
| `dlg-sqlite-create` | Create the file if it doesn't exist | Dosya yoksa oluştur | PO review |
| `sec-auth` | Authentication | Kimlik doğrulama | PO review |
| `auth-password` | Password | Parola | PO review |
| `auth-pgpass` | pgpass | pgpass | PO review |
| `auth-cert` | Client certificate | İstemci sertifikası | PO review |
| `auth-none` | None | Yok | PO review |
| `dlg-password` | Password | Parola | PO review |
| `dlg-remember` | Remember | Hatırla | PO review |
| `dlg-pw-saved` | saved in the Keychain | Anahtar Zinciri'nde kayıtlı | PO review |
| `dlg-pw-from-url` | from the pasted URL | yapıştırılan URL'den | PO review |
| `ret-never` | Never | Hiçbir zaman | PO review |
| `ret-never-h` | ask on every connect | her bağlanışta sor | PO review |
| `ret-restart` | Until restart | Yeniden başlatana kadar | PO review |
| `ret-restart-h` | kept in memory | bellekte tutulur | PO review |
| `ret-session` | For this session | Bu oturum boyunca | PO review |
| `ret-session-h` | forgotten on disconnect | bağlantı kesilince unutulur | PO review |
| `ret-forever` | Forever (Keychain) | Kalıcı (Anahtar Zinciri) | PO review |
| `ret-forever-h` | stored in the macOS Keychain | macOS Anahtar Zinciri'nde saklanır | PO review |
| `pgpass-match` | Matches line {n} of ~/.pgpass | ~/.pgpass dosyasının {n}. satırıyla eşleşiyor | PO review |
| `pgpass-note` | Read on every connect. Hacivat stores no password for this connection. | Her bağlanışta okunur. Hacivat bu bağlantı için parola saklamaz. | PO review |
| `dlg-cert` | Certificate | Sertifika | PO review |
| `dlg-key` | Private key | Özel anahtar | PO review |
| `dlg-key-pass` | Key passphrase | Anahtar parolası | PO review |
| `auth-none-note` | Connects without a password. Use only for servers that trust this machine. | Parolasız bağlanır. Yalnızca bu makineye güvenen sunucularda kullanın. | PO review |
| `sec-tls` | Encryption (TLS) | Şifreleme (TLS) | PO review |
| `tls-sum-auto` | Auto: verify when the server offers TLS | Otomatik: sunucu TLS sunarsa doğrula | PO review |
| `tls-mode` | Mode | Mod | PO review |
| `tls-auto` | Auto | Otomatik | PO review |
| `tls-auto-h` | verify if offered; plaintext only on private networks | sunulursa doğrula; şifresiz yalnızca özel ağlarda | PO review |
| `tls-require` | Require | Zorunlu | PO review |
| `tls-require-h` | encrypt, don't verify | şifrele, doğrulama | PO review |
| `tls-verify-ca` | Verify CA | CA'yı doğrula | PO review |
| `tls-verify-ca-h` | check the chain | zinciri denetle | PO review |
| `tls-verify-full` | Verify full | Tam doğrula | PO review |
| `tls-verify-full-h` | check the chain and host name | zinciri ve sunucu adını denetle | PO review |
| `tls-disable` | Disable | Kapalı | PO review |
| `tls-disable-h` | never encrypt | hiç şifreleme | PO review |
| `tls-ca` | CA certificate | CA sertifikası | PO review |
| `tls-ca-ph` | System trust store | Sistem güven deposu | PO review |
| `tls-pin` | Pinned certificate | Sabitlenmiş sertifika | PO review |
| `tls-pin-none` | None | Yok | PO review |
| `tls-remove-pin` | Remove pin | Sabitlemeyi kaldır | PO review |
| `sec-ssh` | SSH tunnel | SSH tüneli | PO review |
| `ssh-sum-off` | Off | Kapalı | PO review |
| `ssh-sum-on` | via {host} | {host} üzerinden | PO review |
| `ssh-sum-hops` | via {host} · {n} jump host | {host} üzerinden · {n} ara sunucu | PO review |
| `ssh-use` | Connect through an SSH tunnel | SSH tüneli üzerinden bağlan | PO review |
| `ssh-import` | Import from ~/.ssh/config… | ~/.ssh/config'den içe aktar… | PO review |
| `ssh-host` | SSH host | SSH sunucusu | PO review |
| `ssh-user` | SSH user | SSH kullanıcısı | PO review |
| `ssh-auth-agent` | Agent | Ajan | PO review |
| `ssh-auth-key` | Key file | Anahtar dosyası | PO review |
| `ssh-auth-password` | Password | Parola | PO review |
| `ssh-auth-kbd` | Keyboard-interactive | Klavye etkileşimli | PO review |
| `ssh-agent-note` | Uses the keys loaded in ssh-agent (2 keys). | ssh-agent'a yüklü anahtarları kullanır (2 anahtar). | PO review |
| `ssh-key` | Key file | Anahtar dosyası | PO review |
| `ssh-passphrase` | Passphrase | Parola ifadesi | PO review |
| `ssh-kbd-note` | The server's questions, such as a one-time code, appear when you connect. | Sunucunun soruları, örneğin tek kullanımlık kod, bağlanırken gösterilir. | PO review |
| `ssh-jumps` | Jump hosts | Ara sunucular | PO review |
| `ssh-add-jump` | Add jump host | Ara sunucu ekle | PO review |
| `ssh-route` | Route | Rota | PO review |
| `ssh-this-mac` | this Mac | bu Mac | PO review |
| `ssh-hostkeys` | Host keys | Sunucu anahtarları | PO review |
| `hk-ask` | Ask for new hosts | Yeni sunucularda sor | PO review |
| `hk-accept` | Accept new, stop on change | Yeniyi kabul et, değişirse dur | PO review |
| `hk-strict` | Known hosts only | Yalnızca bilinen sunucular | PO review |
| `ssh-import-title` | Hosts in ~/.ssh/config | ~/.ssh/config içindeki sunucular | PO review |
| `ssh-proxycmd` | uses ProxyCommand, not supported | ProxyCommand kullanıyor, desteklenmiyor | PO review |
| `ssh-imported` | Imported {alias} from ~/.ssh/config: host, user, key and jump host. | {alias}, ~/.ssh/config'den içe aktarıldı: adres, kullanıcı, anahtar ve ara sunucu. | PO review |
| `sec-adv` | Advanced | Gelişmiş | PO review |
| `adv-sum` | Timeout {s} s · keep-alive off | Zaman aşımı {s} sn · canlı tutma kapalı | PO review |
| `adv-timeout` | Connect timeout | Bağlanma zaman aşımı | PO review |
| `adv-keepalive` | Keep-alive every | Canlı tutma aralığı | PO review |
| `adv-off` | Off | Kapalı | PO review |
| `adv-app` | Application name | Uygulama adı | PO review |
| `adv-extra` | Extra properties | Ek özellikler | PO review |
| `adv-extra-add` | Add property | Özellik ekle | PO review |
| `adv-budget` | Connections | Bağlantı sayısı | PO review |
| `adv-budget-v` | Up to 4 open at once; more consoles wait their turn | Aynı anda en çok 4 açık; fazlası sıra bekler | PO review |
| `unit-s` | s | sn | PO review |
| `dlg-test` | Test connection | Bağlantıyı test et | PO review |
| `dlg-cancel` | Cancel | İptal | PO review |
| `dlg-save` | Save | Kaydet | PO review |
| `dlg-save-connect` | Save and connect | Kaydet ve bağlan | PO review |
| `test-running` | Testing | Test ediliyor | PO review |
| `test-stop` | Stop | Durdur | PO review |
| `test-ok` | Connected in {ms} | {ms} içinde bağlandı | PO review |
| `test-ok-sub` | Nothing was saved. The test opened its own connection and closed it. | Hiçbir şey kaydedilmedi. Test kendi bağlantısını açtı ve kapattı. | PO review |
| `test-copy` | Copy report | Raporu kopyala | PO review |
| `diag-server` | Server | Sunucu | PO review |
| `diag-latency` | Latency | Gecikme | PO review |
| `diag-latency-v` | {ms} · median of 3 pings | {ms} · 3 ölçümün ortancası | PO review |
| `diag-tls` | Encryption | Şifreleme | PO review |
| `diag-auth` | Signed in as | Oturum | PO review |
| `diag-warn` | Warnings | Uyarılar | PO review |
| `diag-none` | None | Yok | PO review |
| `step-resolve` | Resolve | Çözümle | PO review |
| `step-tcp` | Connect | Bağlan | PO review |
| `step-tls` | TLS | TLS | PO review |
| `step-auth` | Sign in | Oturum aç | PO review |
| `step-check` | Check server | Sunucuyu denetle | PO review |
| `step-ssh` | SSH | SSH | PO review |
| `step-pgpass` | pgpass | pgpass | PO review |
| `step-open` | Open file | Dosyayı aç | PO review |
| `url-filled` | Filled from the pasted URL. | Yapıştırılan URL'den dolduruldu. | PO review |
| `url-secret` | The password was taken out of the URL. It goes to the Keychain when you save. | Parola URL'den ayrıldı. Kaydettiğinizde Anahtar Zinciri'ne gider. | PO review |
| `url-secret-other` | The password was taken out of the URL. Hacivat keeps it {ret}. | Parola URL'den ayrıldı. Hacivat onu şöyle tutar: {ret}. | PO review |
| `url-mapped` | became | şuna dönüştü: | PO review |
| `url-extra` | kept as an extra property | ek özellik olarak saklandı | PO review |
| `url-engine-set` | Engine set to {engine} from the URL. | Motor URL'ye göre {engine} olarak ayarlandı. | PO review |
| `url-undo` | Undo | Geri al | PO review |
| `url-switch` | This is a {engine} URL, and the form is set to {current}. | Bu bir {engine} URL'si, form ise {current} için ayarlı. | PO review |
| `url-switch-l2` | Nothing was changed yet. | Henüz hiçbir şey değişmedi. | PO review |
| `url-switch-btn` | Switch to {engine} and fill | {engine} motoruna geç ve doldur | PO review |
| `url-keep` | Keep {current} | {current} kalsın | PO review |
| `url-unrec` | Hacivat can't read this as a connection URL. Nothing was changed. | Hacivat bunu bir bağlantı URL'si olarak okuyamıyor. Hiçbir şey değişmedi. | PO review |
| `url-formats` | It reads postgres://, mysql://, mariadb://, jdbc:… URLs, key=value strings and SQLite file paths. | postgres://, mysql://, mariadb://, jdbc:… URL'lerini, anahtar=değer dizelerini ve SQLite dosya yollarını okur. | PO review |
| `toast-copied` | Copied as URL | URL olarak kopyalandı | PO review |
| `toast-nopw` | The password isn't included. | Parola dahil değil. | PO review |
| `toast-nossh` | The SSH tunnel can't be written into a URL and was left out. | SSH tüneli URL'ye yazılamadığı için dışarıda kaldı. | PO review |
| `err-title` | Couldn't connect to {name} | {name} bağlantısı kurulamadı | PO review |
| `err-auth-h` | The server rejected the password for {user}. | Sunucu {user} kullanıcısının parolasını reddetti. | PO review |
| `err-auth-b` | The password may have changed or expired. Hacivat stops after 3 attempts so the account isn't locked. | Parola değişmiş ya da süresi dolmuş olabilir. Hesap kilitlenmesin diye Hacivat 3 denemeden sonra durur. | PO review |
| `err-dns-h` | {host} couldn't be found. | {host} bulunamadı. | PO review |
| `err-dns-b` | This Mac couldn't look up the host name. Check the spelling, or connect to your VPN if the host is on a private network. | Bu Mac sunucu adını çözümleyemedi. Yazımı denetleyin; sunucu özel bir ağdaysa VPN'e bağlanın. | PO review |
| `err-tls-h` | {host} sent a certificate this Mac doesn't trust. | {host}, bu Mac'in güvenmediği bir sertifika gönderdi. | PO review |
| `err-tls-b` | It may be self-signed or issued by your company's own CA. Review it before you trust it. | Kendinden imzalı ya da şirketinizin kendi CA'sınca verilmiş olabilir. Güvenmeden önce inceleyin. | PO review |
| `err-db-h` | The database {db} doesn't exist on {host}. | {host} üzerinde {db} adlı bir veritabanı yok. | PO review |
| `err-db-b` | Signing in worked. These databases are open to {user}: | Oturum açıldı. {user} kullanıcısının erişebildiği veritabanları: | PO review |
| `err-old-h` | {host} runs {engine} {version}, which Hacivat doesn't support. | {host}, Hacivat'ın desteklemediği {engine} {version} sürümünü çalıştırıyor. | PO review |
| `err-old-b` | Hacivat supports PostgreSQL 14 and later. Its schema reading and safety checks rely on newer system catalogs. | Hacivat PostgreSQL 14 ve sonrasını destekler. Şema okuma ve güvenlik denetimleri daha yeni sistem kataloglarına dayanır. | PO review |
| `fix-reenter` | Enter password again | Parolayı yeniden gir | PO review |
| `fix-edit` | Edit connection | Bağlantıyı düzenle | PO review |
| `fix-retry` | Try again | Yeniden dene | PO review |
| `fix-ssh` | Connect through SSH… | SSH üzerinden bağlan… | PO review |
| `fix-review-cert` | Review certificate… | Sertifikayı incele… | PO review |
| `fix-use-db` | Use {db} | {db} kullan | PO review |
| `fix-versions` | Supported versions | Desteklenen sürümler | PO review |
| `fix-anyway` | Connect anyway, unsupported | Yine de bağlan (desteksiz) | PO review |
| `fix-connect` | Connect | Bağlan | PO review |
| `fix-close` | Close | Kapat | PO review |
| `err-attempt` | Attempt {n} of 3 | 3 denemeden {n}. | PO review |
| `err-new-pw` | Password for {user} | {user} için parola | PO review |
| `err-tech` | Technical details | Teknik ayrıntılar | PO review |
| `err-stage` | Failed at | Hata adımı | PO review |
| `err-class` | Class | Sınıf | PO review |
| `err-code` | Code | Kod | PO review |
| `err-server` | Server message | Sunucu mesajı | PO review |
| `err-elapsed` | Elapsed | Geçen süre | PO review |
| `err-copy` | Copy details (English) | Ayrıntıları kopyala (İngilizce) | PO review |
| `prod-title` | How careful should Hacivat be with {name}? | Hacivat {name} için ne kadar dikkatli olsun? | PO review |
| `prod-body` | {host} isn't on this Mac. Pick the kind of database it is. You can change this later in the connection settings. | {host} bu Mac'te değil. Nasıl bir veritabanı olduğunu seçin. Bunu daha sonra bağlantı ayarlarından değiştirebilirsiniz. | PO review |
| `prod-suggest` | Suggested because the host name contains “prod”. | Adreste “prod” geçtiği için önerildi. | PO review |
| `prod-opt-production` | Production | Üretim | PO review |
| `prod-opt-production-d` | Live data that people or services rely on | İnsanların ya da servislerin dayandığı canlı veri | PO review |
| `prod-opt-staging` | Staging | Hazırlık | PO review |
| `prod-opt-staging-d` | A copy that mirrors production | Üretimi yansıtan bir kopya | PO review |
| `prod-opt-test` | Test | Test | PO review |
| `prod-opt-test-d` | Shared test or CI data | Ortak test ya da CI verisi | PO review |
| `prod-opt-dev` | Dev | Geliştirme | PO review |
| `prod-opt-dev-d` | A development database you can break | Bozulmasında sakınca olmayan geliştirme veritabanı | PO review |
| `prod-changes` | With production, Hacivat: | Üretim seçilirse Hacivat: | PO review |
| `prod-c1` | shows the {word} capsule in the title bar while you work on it, | bu bağlantıda çalışırken başlık çubuğunda {word} kapsülünü gösterir, | PO review |
| `prod-c1-word` | production | prod | PO review |
| `prod-c2` | asks you to confirm statements that change data or schema, | veriyi ya da şemayı değiştiren ifadeler için onay ister, | PO review |
| `prod-c3` | uses manual commit: nothing is written until you commit. | elle commit kullanır: siz commit edene kadar hiçbir şey yazılmaz. | PO review |
| `prod-other` | No extra confirmations. Statements commit as they run. | Ek onay yok. İfadeler çalıştıkça commit edilir. | PO review |
| `prod-save` | Save as {env} | {Env} olarak kaydet | PO review |
| `prod-pick` | Pick an environment | Bir ortam seçin | PO review |
| `tls-title` | Certificate not trusted | Sertifikaya güvenilmiyor | PO review |
| `tls-body` | {host}:{port} offered a certificate that no CA trusted by this Mac has signed. Compare the fingerprint with whoever runs the server before you continue. | {host}:{port}, bu Mac'in güvendiği hiçbir CA'nın imzalamadığı bir sertifika sundu. Devam etmeden önce parmak izini sunucuyu yöneten kişiyle karşılaştırın. | PO review |
| `tls-subject` | Issued to | Verildiği ad | PO review |
| `tls-issuer` | Issued by | Veren | PO review |
| `tls-valid` | Valid until | Geçerlilik sonu | PO review |
| `tls-fp` | SHA-256 fingerprint | SHA-256 parmak izi | PO review |
| `tls-chain` | Chain | Zincir | PO review |
| `tls-leaf` | server | sunucu | PO review |
| `tls-inter` | intermediate | ara sertifika | PO review |
| `tls-root` | root · not trusted by this Mac | kök · bu Mac güvenmiyor | PO review |
| `tls-pin-note` | Pinning trusts exactly this certificate for {host}:{port}. If it ever changes, Hacivat stops and asks again. A pin doesn't extend the expiry date. | Sabitleme, {host}:{port} için yalnızca bu sertifikaya güvenir. Sertifika bir gün değişirse Hacivat durur ve yeniden sorar. Sabitleme geçerlilik süresini uzatmaz. | PO review |
| `tls-pin-btn` | Pin this certificate | Bu sertifikayı sabitle | PO review |
| `tls-import` | Import CA… | CA içe aktar… | PO review |
| `tls-copy-fp` | Copy fingerprint | Parmak izini kopyala | PO review |
| `unenc-title` | Connected without encryption | Şifrelemesiz bağlanıldı | PO review |
| `unenc-body` | {host} doesn't offer TLS. Hacivat allowed plaintext because the address is on a private network. This badge stays for as long as the connection is unencrypted. | {host} TLS sunmuyor. Adres özel bir ağda olduğu için Hacivat şifresiz bağlantıya izin verdi. Bağlantı şifresiz kaldıkça bu rozet görünür. | PO review |
| `unenc-fix` | To remove it, turn on TLS on the server or connect through SSH. | Kaldırmak için sunucuda TLS'i açın ya da SSH üzerinden bağlanın. | PO review |
| `plain-title` | {host} doesn't offer encryption | {host} şifreleme sunmuyor | PO review |
| `plain-body` | {ip} accepts only unencrypted connections and isn't on a private network. Your password and query results would cross the network in plain text. | {ip} yalnızca şifresiz bağlantı kabul ediyor ve özel bir ağda değil. Parolanız ve sorgu sonuçlarınız ağda düz metin olarak taşınır. | PO review |
| `plain-ssh` | Use an SSH tunnel… | SSH tüneli kullan… | PO review |
| `plain-connect` | Connect without encryption | Şifrelemeden bağlan | PO review |
| `plain-remember` | Don't ask again for this connection | Bu bağlantı için bir daha sorma | PO review |
| `hk-title` | First connection to {host} | {host} ile ilk bağlantı | PO review |
| `hk-body` | This Mac hasn't connected to this SSH server before. Compare the fingerprint with the one your administrator gave you. | Bu Mac bu SSH sunucusuna daha önce bağlanmadı. Parmak izini yöneticinizin verdiğiyle karşılaştırın. | PO review |
| `hk-fp` | Host key | Sunucu anahtarı | PO review |
| `hk-store` | Hacivat saves it in its own known-hosts list. ~/.ssh/known_hosts isn't changed. | Hacivat bunu kendi bilinen sunucular listesine kaydeder. ~/.ssh/known_hosts değişmez. | PO review |
| `hk-trust` | Trust and connect | Güven ve bağlan | PO review |
| `cap-none` | No connection | Bağlantı yok | PO review |
| `cap-connecting` | Connecting | Bağlanıyor | PO review |
| `cap-failed` | Couldn't connect | Bağlanılamadı | PO review |
| `cap-retry` | Retry | Yeniden dene | PO review |
| `cap-reconnecting` | Reconnecting · attempt {n} | Yeniden bağlanıyor · {n}. deneme | PO review |
| `cap-next` | in {s} s | {s} sn sonra | PO review |
| `lost-title` | Reconnected to {name} | {name} ile yeniden bağlanıldı | PO review |
| `lost-body` | The server session ended while the network was down. Hacivat opened a new one. | Ağ kesikken sunucu oturumu sona erdi. Hacivat yeni bir oturum açtı. | PO review |
| `lost-tx` | Rolled back: your open transaction, with 2 uncommitted changes | Geri alındı: commit edilmemiş 2 değişiklik içeren açık işleminiz | PO review |
| `lost-other` | Also gone: temp tables and session variables | Ayrıca kayboldu: geçici tablolar ve oturum değişkenleri | PO review |
| `lost-details` | Show details | Ayrıntıları göster | PO review |
| `lost-dismiss` | Dismiss | Kapat | PO review |
| `toast-cancelled` | Connecting to {name} was cancelled. | {name} bağlantısı iptal edildi. | PO review |
| `toast-connected` | Connected to {name} | {name} ile bağlanıldı | PO review |
| `exp-empty-title` | No connections yet | Henüz bağlantı yok | PO review |
| `exp-empty-body` | Add a PostgreSQL, MySQL, MariaDB or SQLite database. If you paste a connection URL, Hacivat fills the form for you. | Bir PostgreSQL, MySQL, MariaDB ya da SQLite veritabanı ekleyin. Bir bağlantı URL'si yapıştırırsanız Hacivat formu sizin için doldurur. | PO review |
| `exp-new` | New connection | Yeni bağlantı | PO review |
| `exp-paste` | Paste URL | URL yapıştır | PO review |
| `exp-import` | Import pg_service.conf… | pg_service.conf içe aktar… | PO review |
| `menu-connect` | Connect | Bağlan | PO review |
| `menu-disconnect` | Disconnect | Bağlantıyı kes | PO review |
| `menu-edit` | Edit… | Düzenle… | PO review |
| `menu-dup` | Duplicate | Çoğalt | PO review |
| `menu-copy-url` | Copy as URL | URL olarak kopyala | PO review |
| `menu-delete` | Delete… | Sil… | PO review |
| `del-title` | Delete {name}? | {name} silinsin mi? | PO review |
| `del-body` | Its saved password is removed from the Keychain too. Scripts and query history stay. This can't be undone. | Kayıtlı parolası Anahtar Zinciri'nden de silinir. Betikler ve sorgu geçmişi kalır. Bu işlem geri alınamaz. | PO review |
| `del-btn` | Delete | Sil | PO review |
| `toast-saved` | Saved {name} | {name} kaydedildi | PO review |
| `toast-dup` | Duplicated as {name} | {name} olarak çoğaltıldı | PO review |
| `toast-deleted` | Deleted {name} | {name} silindi | PO review |
| `toast-versions` | Opens hacivat.dev/docs/engines in your browser. | Tarayıcınızda hacivat.dev/docs/engines sayfasını açar. | PO review |

292 strings. Not included: mock-only labels (mock bar, Notes drawer) and literal data such as host names, codes and fingerprints.


## PO answers (2026-11-12)

| # | Question | Answer |
|---|---|---|
| Q1 | Should the dialog scrim dim the title bar? | No. The capsule stays readable and the scrim starts below the title bar (designer's default, kept). |
| Q2 | Tag colour | Only production gets a hue (red). The other tags use neutral shape marks. |
| Q3 | Server older than the supported floor | Warn, and offer the secondary action "Connect anyway (unsupported)". The capsule carries a permanent "unsupported" mark. |
| Q4 | Default password retention | Forever, stored in the Keychain. |
| Q5 | Production prompt preselection | Preselect Production when the host or connection name contains `prod`, `prd` or `live`. The prompt is still always asked. |

Overall visual sign-off: pending the PO's review of the mock.
