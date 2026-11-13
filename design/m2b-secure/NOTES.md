# M2b Secure network and sessions: design notes

Clickable mock: `design/m2b-secure/index.html` (one file, one app, surface switcher). It is the M2a mock's world, unchanged: the same CSS (copied, with two rules marked "M2b"), the same Lucide 0.460 sprite (ISC), the same mock bar, Notes drawer, toggles and deep links. It runs with no network except Google Fonts.

**How to drive it**
- The dashed **mock bar** is not product UI. It holds the surfaces `1`–`7`, the scenarios (`[` / `]`), EN/TR (`L`), dark/light (`T`) and Notes (`N`).
- The Notes drawer repeats these notes per surface and lists every string on the surface. New M2b strings come first; reused M2a strings are marked `M2a`.
- Deep links: `?s=4&sc=reset&theme=light&lang=tr&notes=1`.

**Flows that run**
- Host key changed: Cancel (or Enter, or Esc) fails the connect; the capsule's body then opens the `ssh.host_key_changed` error, whose "Review host key…" reopens the dialog. "Remove the old key and trust the new one" connects. In the `~/.ssh` variants, Copy command copies the exact command and Try again leads to M2a's new-host-key prompt (what happens after the user runs it).
- Keyboard-interactive and SSH secrets: Enter continues. The hop 1 password leads to the hop 2 key passphrase, then the capsule connects. The eye toggles masked fields. The Remember Select opens with Space, ↓ or ↵.
- Vault: the create rules update as you type; Create enables only when both hold. Unlock → Reset vault… stacks the reset dialog. Typing `reset` enables the Danger button; Enter never deletes.
- Pin changed: Replace pin connects and confirms which host was pinned. Cancel fails the connect with `tls.pin_changed`.
- Read-only: the access segment in the capsule and the sheet's Access value both open the popover, by click or from the keyboard (both are Tab stops; Enter or Space). Copy SQL copies exactly Task 5's `remediation()` string. A click outside or Esc closes it, and Esc returns the focus to the trigger.
- Idle: clicking the capsule body reconnects ("It reconnects when you use it"). Error dialogs: Try again runs and fails again; Review certificate… on `tls.pin_changed` opens surface 5.

**Product truth used:** the M2b plan's rulings B5, B8, B11, B13, B18, B19, B21, B24, B28 and B30, the brief for Task 2, DESIGN.md, and the shipped strings in `locales/*/hacivat-ui.ftl` where an M2b label already exists (`hv-conn-access-ro-guarded`, `hv-conn-enc-unverified`, `hv-conn-pw-*`). The approved M2a surfaces 5 (TLS) and 6 (SSH settings, new host key) are not re-mocked (B28); this mock reuses M2a's new-host-key prompt unchanged.

---

## Cross-cutting decisions

1. **No new hue, no new filled block.** Every new surface uses ink, text-2 and the existing four hues for their existing meanings only. Green appears only as a check that a rule or a hop succeeded. Red appears only for production (lock, swatch, capsule) and for the vault reset Danger button. A failed hop, a mismatch or a rejected answer is an ink ✕ or an ink alert glyph. The capsule's cp-fill block stays the only filled badge (checked in every scenario: exactly one element carries the cp-fill colour).
2. **The safe answer is the default.** In the three hard stops (host key changed, pin changed, vault damaged) the primary button is Cancel or Close, with the initial focus, so Enter and Esc both stop. The trust-widening action is a plain outline button on the far left of the footer, away from the default. The vault reset is a Danger button that stays disabled until the typed word matches and is never triggered by Enter.
3. **Server text is never inside a translated sentence.** Keyboard-interactive name, instruction and prompt labels sit verbatim in their own mono field, labelled "Sent by the server, shown as received". Server messages in errors stay in Technical details. Sentences only interpolate values Hacivat itself chose or dialled (host, user, key path, file path, connection name).
4. **Every SSH prompt names its hop.** The subtitle reads "bastion.acme.io, hop 2 of 2", and the SSH secret prompt boxes the asking hop in the route. Errors draw the route with ✓ / ✕ / pending per hop and state "Failed at hop 2 (bastion.acme.io)".
5. **480 px, Turkish first.** Every dialog is the M2a 480 "wide" size (the vault reset is a 420 confirmation). Inside them the label column narrows to 120 px. A headless pass rendered all 36 scenarios in EN/TR × dark/light (144 renders) at 1440 × 900: no console errors, and no text clipped or overflowing inside any dialog, popover, toast, menu or the capsule. The only ellipsized values are the sheet's Host and Path values in the 150 px value column, which ellipsize by DESIGN.md's long-locale rule (as in M2a).
6. **GPUI-reproducible.** Solid fills, 1 px borders, radii 3/4/6, the popover shadow only, no transforms, filters, blur or masks. Fingerprints, commands and SQL use Fragment Mono 11.5/17 in the existing fingerprint-inset anatomy.

## Surfaces

### 1 · Host key changed (`ssh.host_key_changed`)
**Intent.** A hard stop that states the fact, shows both keys and says where the old one is stored. Trusting the new key takes a deliberate second button; the default stops.

**Anatomy.**
- 480 px alertdialog over the capsule's "Connecting · SSH" state. No warning icon, no hue.
- Title: "The host key of bastion.acme.io changed". Subtitle: env mark, connection name, "bastion.acme.io, hop 2 of 2".
- Body: what happened, the two benign causes and the hostile one, and "Hacivat stopped before signing in."
- Two fingerprint insets: "Stored key" and "Offered now", each with the algorithm (ED25519), the `SHA256:` fingerprint and a Copy button. The stored inset ends with a seam and "Stored in: Hacivat's known hosts" or "Stored in: ~/.ssh/known_hosts line 14".
- "Ask whoever runs bastion.acme.io whether its key changed…".
- **Hacivat store:** footer = "Remove the old key and trust the new one" (outline, far left, key icon) … Cancel (primary).
- **`~/.ssh` source:** no remove button. "Hacivat doesn't change ~/.ssh/known_hosts. If the new key is genuine, remove the old line in a terminal, then try again:" and a command inset with Copy command. Footer = Try again (outline) and Cancel (primary).
- The command is `ssh-keygen -R bastion.acme.io` for port 22 and `ssh-keygen -R "[bastion.acme.io]:2222"` for any other port. When "Stored in" names any file other than `~/.ssh/known_hosts` (B11 also reads `known_hosts2`), the command adds `-f <file>`: `ssh-keygen -f ~/.ssh/known_hosts2 -R bastion.acme.io` (scenario "known_hosts2"; see open question 1).

**Keyboard.** Initial focus on Cancel; Enter or Esc cancels. Tab cycles Copy (stored) → Copy (offered) → [Copy command] → the trust action or Try again → Cancel, trapped in the dialog.

### 2 · Keyboard-interactive (RFC 4256)
**Intent.** Show exactly what the server sent, never mixed into our own words, and ask for the answers in the order it asked.

**Anatomy.**
- 480 px dialog. Title "Sign in to bastion.acme.io"; subtitle: env mark, connection, SSH user, hop.
- Lead line (ours): "bastion.acme.io asks:".
- A mono inset captioned "Sent by the server, shown as received": the server's name (text), then its instruction (text-2), line breaks kept. When both are empty: "The server sent no name or instructions." in the UI face (no caption), so it can't be mistaken for server text.
- **Verbatim but visible** (OpenSSH `strnvis` parity): in the name, the instruction and every prompt label, a literal backslash is doubled first (`\` → `\\`, as `strnvis` does), so server text can't imitate an escape. Then C0/C1 control characters (ANSI escapes included) render as `\ooo` octal (ESC → `\033`) and bidi controls (U+061C, U+200E/F, U+202A–202E, U+2066–2069) as `\u{202E}`; only `\n` is kept. The caption stays fixed at the top of the inset; only the server's text scrolls, capped at 119 px (7 mono lines, so the whole inset stays ≤ 160 px), with M2a's 4 px seam-strong scroll thumb (radius 3, no track). A hostile or chatty server can neither draw UI-looking text nor push the prompts out of the dialog (scenario "Hostile text").
- One to three prompts, stacked: the prompt label verbatim in mono above a full-width field. `echo = false` gives a masked field with the eye toggle; `echo = true` gives a plain field.
- Meta line: "Attempt 1 of 3" and "Answers go to this server only and aren't kept." After a rejected round, an ink alert glyph and "The server didn't accept the answers and asked again." come first.
- Footer: Cancel (ghost), Continue (primary, ↵).
- Switch to TR: the server's English stays English.

**Keyboard.** Initial focus on the first prompt.
- Enter in a prompt that isn't the last moves the focus to the next prompt; it never submits.
- Enter in the last prompt runs Continue, but only when every masked prompt has an answer; otherwise it moves the focus to the first empty masked prompt. A half-filled round never spends one of the 3 attempts, a server MaxAuthTries slot or a second Duo push.
- Continue (button) always submits what is there.
- Tab walks the fields, then Cancel and Continue. Esc cancels the connect (the capsule shows Failed, `ssh.auth`).

### 3 · SSH password and key passphrase, per hop
**Intent.** The M2a password prompt, per hop: it always says which machine on the route is asking and which key is being unlocked.

**Anatomy.**
- 480 px dialog. Titles "SSH password for jump-eu.acme.io" and "Passphrase for ~/.ssh/id_ed25519". Subtitle: env mark, connection, "jump-eu.acme.io, hop 1 of 2".
- Body line mirrors `hv-conn-pw-body`: "Enter the SSH password for deploy@jump-eu.acme.io. It's kept only if the connection works."
- Rows (120 px labels): Route (this Mac → **jump-eu.acme.io** boxed → bastion.acme.io → db-prod-eu.acme.io:5432; the others text-2), SSH user (static mono), Password (masked field with eye), Remember (240 px Select, default Forever, B11 `ssh_retention`).
- Passphrase variant: Key file and Public key (ED25519 `SHA256:` fingerprint) as static mono rows instead of SSH user.
- Wrong password: "jump-eu.acme.io didn't accept the password." and "Attempt 2 of 3" directly under the Password / Passphrase field (above Remember), as in vault Unlock.
- Footer: Cancel (ghost), Continue (primary, ↵).

**Keyboard.** Initial focus on the secret field. Enter continues. The Select opens with Space, ↓ or ↵; ↑/↓ move, ↵ picks, Esc closes it. Esc on the dialog cancels the connect.

### 4 · Vault (Linux without a system keyring, B13)
**Intent.** Forever means an encrypted file behind a master password. Say why, say the cost (no recovery), and never block the connect on it.

**Anatomy.** The window chrome switches to Linux (no traffic lights, controls on the right) because the vault exists only there.
- **Create** (480): title "Create a vault for saved passwords"; first line, in text, "This computer has no system keyring."; then what Hacivat does; the file (`~/.local/share/hacivat/vault.bin`); Master password and Repeat (masked, eye); two live rules ("At least 8 characters" with a hollow marker, then a green check once met; "Both entries match" / an ink ✕ "The two entries don't match" once both are filled); an info notice "Hacivat can't recover it. If you forget it, you reset the vault and enter your saved passwords again."; and "Not now keeps this password until Hacivat restarts." Footer: Not now (ghost), Create vault (primary, disabled until both rules hold).
- **Unlock** (480): "Hacivat needs the master password to read the saved password for orders-dev. The vault stays unlocked until you quit." One masked field. After a miss: "That isn't the master password." and "Attempt 2 of 3". Footer: Reset vault… (ghost, far left) … Cancel (ghost), Unlock (primary, disabled while empty).
- **Reset** (420 alertdialog, stacked over Unlock): "This deletes the vault and the 6 passwords saved in it. Your connections stay, and Hacivat asks for each password the next time you connect. This can't be undone." Then "Type `reset` to confirm" and a mono field. The word is the string `vault-reset-word` (`hv-conn-vault-reset-word`), `reset` in both languages on purpose. Footer: Cancel (ghost), Reset vault (Danger, trash icon, disabled until the field equals that word exactly).
- **Damaged file** (480 alertdialog): the integrity check failed, the file was neither used nor changed, restore or reset, and "Until then, passwords you enter are kept until Hacivat restarts." Footer: Reset vault… (outline, far left) … Close (primary).
- **Remember Select:** the fourth option reads "Forever (encrypted file)" (`ret-forever-vault` → `hv-conn-ret-forever-vault`, the id Task 21's `forever_label` returns) with the hint "in Hacivat's vault, behind a master password" (`ret-forever-vault-h`). It replaces M2a's "Forever (Keychain)" and the disabled `hv-conn-ret-forever-off-h`. The sheet's Password row reads "vault · forever".

**Keyboard.** Create: focus on Master password; Enter there moves to Repeat; Enter in Repeat creates only when enabled. Unlock: focus on the field; Enter unlocks. Reset: focus on the typed-confirmation field; Enter does nothing; Tab reaches Cancel, then Reset vault once enabled; Esc returns to Unlock. Damaged: focus on Close.

### 5 · Certificate changed on a pinned endpoint (`tls.pin_changed`, B5)
**Intent.** M2a surface 5's anatomy for a pinned certificate that changed: both fingerprints, the new certificate's facts, and an explicit Replace pin that is never the default.

**Anatomy.**
- 480 px alertdialog over "Connecting · TLS". Title "The certificate of db-qa.acme.io:5432 changed". Subtitle: env mark and connection.
- Body: "This connection is pinned to one certificate, and the server now presents a different one. Hacivat stopped before signing in."
- Two fingerprint insets: **Pinned** (the M2a pin, SHA-256, 2 × 16 bytes, Copy) and **Now** (SHA-256, 2 × 16 bytes, Copy), the second also holding Issued to, Issued by, Valid until (formatted by locale) and the chain (server → root · not trusted by this Mac).
- "A renewed certificate looks like this too. Ask whoever runs the server before you replace the pin." and the rule: replacing trusts exactly the new certificate, removes the old pin, and doesn't extend the expiry date.
- Footer: Replace pin (outline, fingerprint icon, far left) … Cancel (primary).

**Keyboard.** Initial focus on Cancel; Enter or Esc cancels (`tls.pin_changed` in the capsule). Tab: Copy, Copy, Replace pin, Cancel.

### 6 · Read-only labels (B21)
**Intent.** Read-only says how it is enforced, by the server's role or only guarded by Hacivat. The difference, what the role probe found and the fix are one click or one keystroke away.

**Anatomy.**
- **Capsule access segment:**
  - Lock + "read-only (enforced by server role)" (`access-ro-enforced` → `hv-conn-access-ro-enforced`) when the role probe found a least-privilege role (PostgreSQL, MySQL).
  - Lock + "read-only (guarded)" otherwise, which is the shipped `hv-conn-access-ro-guarded`, reused and not duplicated. MariaDB and SQLite always show this.
  - The lock is red only on production; elsewhere it is text-2. The Explorer's lock flag follows the same rule (M2a drew every read-only lock red).
- **Popover** (300 px, the M2a explanation popover anatomy) under the segment. **Height cap:** it never grows past the space between the title bar and the status bar (6 px clear of each); beyond that the body scrolls with the 4 px seam-strong thumb while the title stays on top and Copy SQL stays at the bottom (the Overflow Rule; at the 960 × 600 minimum window "All findings" scrolls).
  - **Enforced:** "Read-only, enforced by the server" and "The role app_ro can only read here. The server itself refuses writes, whatever runs in Hacivat."
    - If the probe still reported `TempPrivilege`, the findings list follows with that one (label-neutral) line, then the enforced-only reassurance `ro-pop-enf-temp`: "TEMP alone doesn't weaken enforcement, so the label stays enforced." (Task 5, Q21; scenario "Enforced · TEMP"). That sentence appears nowhere else.
  - **Guarded (PostgreSQL, MySQL):**
    - the title "Read-only is guarded, not enforced";
    - why ("The role qa isn't limited to reading here, so Hacivat stops writes itself…");
    - **the findings list**: "Found on this role:", then one line per `RoleWarning` from the probe. The realistic PostgreSQL case (scenario "Guarded · PG") lists Superuser, WritePrivileges, TempPrivilege and NonTransactionalWrite, since PUBLIC holds TEMP and (Task 23 finding F1) `EXECUTE` on `pg_logical_emit_message` by default;
    - the limit ("Only a read-only role on the server is a hard guarantee.") and the fix line;
    - the SQL inset showing exactly what Task 5's `remediation(engine)` returns, then Copy SQL, which copies that same string.
  - **MariaDB:**
    - it is always guarded (C19), and the body says why;
    - the findings list follows when the probe reported any;
    - `remediation(MariaDb)` is `None` (Task 5, tested), so there is no SQL inset. Instead: "No role change makes this label enforced, so there is no SQL to copy."
  - **SQLite:** no server roles; no findings, no snippet.
- **Findings list** (`ro-warn-*` → Task 22's `hv-conn-ro-warn-*`), one row per `RoleWarning` (Task 5 `interpret_probe`):

| RoleWarning | Mock id | EN |
|---|---|---|
| `Superuser` | `ro-warn-superuser` | It is a superuser. |
| `BypassRls` | `ro-warn-bypass-rls` | It bypasses row-level security. |
| `Replication` | `ro-warn-replication` | It can start replication. |
| `CreateRole` | `ro-warn-create-role` | It can create and change roles. |
| `ServerFileRoles` | `ro-warn-server-file-roles` | It can read or write files on the database server. |
| `ExecuteServerProgram` | `ro-warn-execute-server-program` | It can run programs on the database server. |
| `SignalBackend` | `ro-warn-signal-backend` | It can cancel or end other sessions. |
| `WritePrivileges` | `ro-warn-write-privileges` | It can change data in some tables. |
| `TempPrivilege` | `ro-warn-temp-privilege` | It can create temporary tables, which read-only transactions block. (Label-neutral: it appears in guarded and enforced popovers alike.) |
| `EgressExtension(name)` | `ro-warn-egress-extension` | The `{name}` extension can reach other servers. |
| `RoleGrantNotExpanded` | `ro-warn-role-grant-not-expanded` | Other roles are granted to it, and Hacivat can't see what they allow. |
| `Unparsed` | `ro-warn-unparsed` | Hacivat couldn't read its privileges. |
| `NonTransactionalWrite` | `ro-warn-non-transactional-write` | It can run functions whose effects a rollback can't undo. (Task 23 finding F1: PostgreSQL row 6, e.g. `pg_logical_emit_message`.) |

  `{name}` is rendered as a mono literal. It is never server prose: the probe only returns names from its fixed list (`dblink`, `postgres_fdw`, `pg_net`, `http`). Scenario "All findings" shows all thirteen in Turkish inside the 300 px popover.
- **Sheet Access row** (new row under Environment): lock + "read-only · enforced" or "read-only · guarded". These are short forms, so Turkish fits the 150 px column. The whole value is a button that opens the popover, underlined on hover. Read-write rows show "read-write" in text-2.
- **Status bar:** the environment segment carries the capsule's label ("production · read-only (enforced by server role)").

The SQL, exactly as `remediation()` returns it. PostgreSQL:
```sql
CREATE ROLE <role> LOGIN PASSWORD '<password>';
GRANT pg_read_all_data TO <role>;
-- optional: REVOKE TEMP ON DATABASE <database> FROM PUBLIC;
-- optional: REVOKE CREATE ON SCHEMA public FROM PUBLIC;
-- optional: REVOKE EXECUTE ON FUNCTION pg_logical_emit_message(boolean, text, text, boolean), pg_logical_emit_message(boolean, text, bytea, boolean) FROM PUBLIC;
```
(Task 23 finding F1: this is the form for PostgreSQL 17 and later, which `remediation()` returns; `remediation_for` gives 13–16 the three-argument signatures, `pg_logical_emit_message(boolean, text, text), pg_logical_emit_message(boolean, text, bytea)`, and 13 its schema-grant form.)

MySQL:
```sql
CREATE USER '<role>'@'%' IDENTIFIED BY '<password>';
GRANT SELECT, SHOW VIEW ON <database>.* TO '<role>'@'%';
```
In the inset each line is its own block (Fragment Mono 11.5/17), and blocks sit 4 px apart. A line longer than the inset wraps inside it (`pre-wrap`, breaking anywhere as a last resort) with a 2ch hanging indent (`padding-left: 2ch; text-indent: -2ch`; in GPUI, the wrapped line's indent), so a continuation such as `'<password>';` reads as part of its statement; nothing is clipped and nothing scrolls sideways. Copy SQL still copies Task 5's string unchanged. Keywords use the syntax keyword colour and strings the syntax string colour. Placeholders are italic text-2. The `-- optional:` line is a comment, in the syntax comment colour and italic.

**Keyboard.**
- The capsule access segment and the sheet's Access value are both Tab stops (in the capsule, the segment after the capsule itself). Enter or Space opens the popover.
- The palette command **"Explain read-only access"** (`ro-cmd` → `hv-cmd-connection-explain-access`) opens it for the active connection. The capsule segment shows the same label as its tooltip.
- On open, the focus moves to the popover, which is labelled by its title, so the title is announced. Tab reaches Copy SQL.
- Esc, or a click outside, closes the popover; Esc returns the focus to whichever trigger opened it.

### 7 · Idle and the new error copy
**Intent.** An idle disconnect is quiet and reversible. New error classes use the M2a error-dialog anatomy unchanged.

**Anatomy.**
- **Idle toast** (the existing 320 px Toast, unplug icon).
  - Copy: `idle-disconnected` → `hv-conn-idle-disconnected` (`$name`, `$hours`): "Disconnected orders-dev after 4 h idle. It reconnects when you use it."
  - `idle_disconnect_s` is free per profile (B18), so a threshold that isn't a whole number of hours uses `idle-disconnected-minutes` → `hv-conn-idle-disconnected-minutes` (`$name`, `$minutes`): "…after 90 min idle…" (scenario "Idle 90 min"). Each message carries its own unit (INV-1).
  - In the product the toast has the standard TTL; the mock pins it so it can be reviewed.
- **Idle capsule.** The capsule keeps the environment and name and adds a segment with the unplug glyph and "Disconnected (idle)" in text-2. The Explorer row's meta shows the unplug glyph, and the status bar says the same. Clicking the capsule body reconnects; the chevron still opens the connection switcher, as in M2a.
- **Errors** (480, M2a anatomy: message, explanation, step trail, fixes, Technical details, Close):

| Class | Message | Body (short) | Fixes (existing FixIds) |
|---|---|---|---|
| `auth.insecure_method` (TLS, not verified) | The server asked for the password in a form that isn't safe on this connection. | Encrypted but unverified, so the password wasn't sent; pin the certificate. | **Review certificate…**, Edit connection |
| `auth.insecure_method` (no TLS) | same | Not encrypted, so the password wasn't sent; turn on TLS, or set Encryption to Disable to accept the risk. The trail has no TLS step (Resolve → Connect → Sign in ✕ → Check server). | **Edit connection** |
| `ssh.auth` | bastion.acme.io rejected the SSH sign-in for deploy. | Which keys were tried; check authorized_keys or choose another method. "Failed at hop 2 (bastion.acme.io)" and the hop route. | **Edit connection**, Try again |
| `ssh.channel_open` | bastion.acme.io doesn't allow tunnels (AllowTcpForwarding) | Signing in worked, forwarding to db-prod-eu.acme.io:5432 was refused; AllowTcpForwarding and PermitOpen. Hop 2. | **Try again**, Edit connection |
| `ssh.agent_unavailable` | Hacivat can't reach ssh-agent. | SSH_AUTH_SOCK isn't set; start the agent and ssh-add, or choose a key file in the SSH settings. Hop 1. | **Try again**, Edit connection |
| `tls.pin_changed` | db-qa.acme.io sent a different certificate from the pinned one. | Stopped before signing in; compare before replacing. | **Review certificate…** (opens surface 5), Edit connection |
| `ssh.host_key_changed` | The host key of bastion.acme.io changed. | Stopped before signing in; review the new key. Hop 2. | **Review host key…** (opens surface 1), Edit connection |
| `ssh.host_key_revoked` | bastion.acme.io sent a key that ~/.ssh/known_hosts marks as revoked. | Never connects with a revoked key; ask for the current one. Hop 2. | Edit connection |

- Under SSH the step trail is SSH → Connect → TLS → Sign in → Check server (Resolve drops out, B11).
- Technical details show:
  - the stage with the hop host ("SSH · bastion.acme.io");
  - the class and the code chip;
  - the server's own text, only when it sent some (the `ssh.auth` method list, the channel-open description);
  - the elapsed time.
- In the hop route each arrow travels with the stop after it, so a wrapped route never ends a line on an arrow.

**Keyboard.** As M2a: initial focus on the first fix; Tab through fixes, Technical details, Copy details and Close; Esc closes.

---

## NEW primitives

All use harman tokens only and are reproducible in GPUI (solid fills, 1 px borders, radii 3/4/6, the popover shadow only).

| Primitive | Anatomy and exact tokens |
|---|---|
| **Key comparison** | Two fingerprint insets stacked 8 px apart (panel / ground, 1 px seam-strong, radius 4, padding 8/10). Header: label 12.5 text-2, algorithm Fragment Mono 11.5 text, a ghost icon Copy on the right. Value Fragment Mono 11.5/17 text. An optional "Stored in" line under a 1 px seam (12 px text-2 + mono 11.5 text). Labels state facts (Stored key / Offered now, Pinned / Now); no hue distinguishes them. |
| **Command inset** | The same inset with the command in Fragment Mono 11.5/17 text (wraps anywhere) and a ghost Copy command button on the right. |
| **Server-text inset** | The same inset, at most 160 px tall. Caption 12 text-2 "Sent by the server, shown as received", fixed at the top. Below it, the server's text scrolls on its own (max 119 px, 4 px seam-strong thumb, radius 3): name in mono 11.5 text; instruction in mono 11.5 text-2; `white-space: pre-wrap`; backslashes doubled, controls and bidi marks shown escaped. Prompts below it: a mono 11.5 label (text) above a full-width TextInput, 10 px apart. |
| **Hop route** | The M2a route row (12 px text-2, arrows 12 px). Each arrow is bound to the stop after it (no line ends on an arrow). The asking hop is boxed: 1 px seam-strong, radius 3, padding 0/4, text; other stops text-2. In errors each hop carries the step-trail marks: check 12 px green, ✕ 12 px ink with the host at weight 600, 7 px hollow seam-strong square for pending. |
| **Validation lines** | 12 px rows, 3 px apart, under the field column: 7 px hollow seam-strong square (unmet), check 12 px green (met), ✕ 12 px ink with text ink (mismatch). |
| **Findings list in the popover** | The sticky-toast list anatomy: a 12.5 text-2 lead ("Found on this role:"), then 12 px text rows, 2 px apart, each with a 3 px square text-2 bullet; literals in mono 11.5. |
| **SQL inset in the popover** | Fingerprint-inset anatomy inside the 300 px popover. One Fragment Mono 11.5/17 block per line, 4 px apart, wrapping inside the inset with a 2ch hanging indent. Keywords syntax-keyword, strings syntax-string, placeholders text-2 italic, comments syntax-comment italic. Copy SQL is a 22 px Button right-aligned under it. |
| **Popover height cap** | The explanation popover is a column: title (fixed), body (scrolls, 4 px seam-strong thumb), Copy SQL row (fixed). Max height = window height − 40 px title bar − 24 px status bar − 2 × 6 px; it opens below its trigger, or above when that fits better, and never crosses either bar. |
| **Capsule: access and idle segments** | Access: lock 12 px (red on production, text-2 otherwise) + label text; a Tab stop (focus ring: 1 px blue inset), and Enter or Space opens the popover. Idle: unplug 12 px + label, both text-2. |
| **Sheet Access row** | The kv row; the value is a borderless button (lock 12 px + label 12.5 text, ellipsis), underline on hover, a Tab stop. |
| **Linux window chrome** | No traffic lights; three 28 × 24 icon slots (minus, square, x at 12 px text-2) after the palette. Mock-only stand-in for the platform frame. |

## Open questions for the PO

Each was answered by the ruling noted, so the build is not blocked; the PO can overrule any.

1. **The `ssh-keygen -R` command.** The brief shows `ssh-keygen -R "[bastion.acme.io]:22"`. OpenSSH writes port-22 entries as the bare host, so that command would not remove the line.
   - *Ruling:* the command is derived from the port: `ssh-keygen -R bastion.acme.io` for 22 and `ssh-keygen -R "[host]:port"` otherwise (B11 known-hosts rules; Task 4's `non_default_port_uses_bracket_form`).
   - `ssh-keygen -R` edits only `~/.ssh/known_hosts` unless it is given `-f`. So when the stored key is in any other file (B11 also reads `~/.ssh/known_hosts2`), the command is `ssh-keygen -f <file> -R <name>`.
   - The mock shows all three. Tasks 4 and 21 build the command the same way.
2. **Vault reset: "the primary button is disabled until it matches".** That conflicts with "no destructive action is ever the default or primary". *Ruling:* the confirm button sits in the primary slot but is the Danger variant: disabled until the field matches, outline-only until hover or focus, and Enter in the field never triggers it (B13: typing the word deletes the vault; DESIGN.md Danger button).
3. **Is the typed word translated?** *Ruling:* no. It is the string `vault-reset-word` (`hv-conn-vault-reset-word`, the id Task 21 tests), whose EN and TR values are both `reset`, shown in mono. Turkish casing (İ/ı) and keyboard layouts can't make the match fail (B13).
4. **Colour of "Remove the old key and trust the new one" and "Replace pin".** *Ruling:* plain outline buttons, not Danger red. Red means destroying user data (Hue Is Meaning); these widen trust, which is guarded by placement (far left), by Cancel being primary with the focus, and by the dialog being an alertdialog (B5, B11).
5. **What the capsule shows after an idle disconnect.** No mock existed. *Ruling:* the capsule keeps the environment and name and adds "Disconnected (idle)" with the unplug glyph in text-2. Using the connection, or clicking the capsule body, reconnects lazily (B18). The chevron keeps opening the connection switcher. It is not "No connection", because the profile is still the active one.
6. **TEMP in the PostgreSQL fix-it SQL.** *Ruling (Task 5, Q21):* the popover shows Task 5's `remediation()` string unchanged. TEMP appears only as the comment `-- optional: REVOKE TEMP ON DATABASE <database> FROM PUBLIC;`, because revoking from PUBLIC affects every role, and TEMP alone doesn't clear least privilege. TEMP is reported as a finding (`TempPrivilege`) instead. The UI never edits, re-wraps or pre-breaks the string; the inset wraps it.
7. **MariaDB and the fix-it SQL.** *Ruling (Task 5):* `remediation(MariaDb)` is `None` and is tested, so the popover shows no snippet. It shows the findings and "No role change makes this label enforced, so there is no SQL to copy." The label stays guarded (C19, B16, B21). Offering a MySQL-syntax snippet would need a controller ruling that amends Task 5 first.
8. **The sheet uses a shorter label than the capsule.** *Ruling:* "read-only · enforced / guarded" in the sheet (150 px value column; a trust label must not ellipsize in Turkish), the full label in the capsule, the status bar and the popover (B21; DESIGN.md long-locale rule).
9. **`ssh.agent_unavailable` has no "Use a key file" fix.** *Ruling:* the FixId set is unchanged in M2b (B24), so the body says where to choose a key file and the fixes are Try again and Edit connection.
10. **Two bodies for `auth.insecure_method`.** *Ruling:*
    - Unverified TLS points to pinning: Review certificate…, which leads to the `Pinned` outcome the guard allows.
    - No TLS advises turning TLS on, or Disable to accept the risk, exactly the cost B8 records. Its trail has no TLS step.
    - The no-TLS variant narrows B8's `[ReviewCertificate, EditConnection]` to Edit connection. Task 14 must therefore omit ReviewCertificate whenever `ConnectError.certificate` is `None`.
11. **Remember for keyboard-interactive answers.** *Ruling:* none. Answers are usually one-time codes; only passwords and key passphrases have secret slots (B11, `SecretSlot::SshPassphrase { hop }`).
12. **Copy for classes outside the brief's list.** *Ruling:* `ssh.host_key_changed` and `ssh.host_key_revoked` get error copy here too, because Cancel in surface 1 lands in the first and B24 adds the second.
13. **Guarded label.** *Ruling:* reuse the shipped `hv-conn-access-ro-guarded` (EN and TR) unchanged, rather than adding a mock duplicate. The enforced label is the new `hv-conn-access-ro-enforced`, the id Task 22 tests (B21).
14. **Headline punctuation for `auth.insecure_method`.** The brief quotes it without a full stop. *Ruling:* add one, like every other `error-conn-*-title` that is a sentence. "…doesn't allow tunnels (AllowTcpForwarding)" stays without one, as quoted, because it ends in a parenthesis (B24).
15. **Vault "Not now".** *Ruling:* not creating a vault keeps the secret until restart, and the dialog says so before the choice (B13, Q38).
16. **The Turkish word for "enforced".** The mock uses *zorunlu* ("salt okunur (sunucu rolüyle zorunlu)", "salt okunur · zorunlu"). It can read as "mandatory" rather than "enforced". Alternatives for the PO: *sunucu rolüyle uygulanıyor* or *sunucu rolüyle sağlanıyor*.
    - *Ruling:* keep *zorunlu* until the PO picks. It is the shortest, it fits the sheet's 150 px column, and it pairs with the shipped *korumalı* (B21).
    - The label is a trust signal, so this is a PO question rather than a copy-edit.
17. **"Forgot it?" link vs "Reset vault…".** Plan Task 21 describes the unlock dialog's reset entry as the link "Forgot it?". *Ruling:* the mock's ghost button "Reset vault…" (`vault-reset-link`) supersedes it, and Task 21 builds only that one entry. It names the consequence rather than the situation (B13).

---

## Copy strings (EN / TR)

The ids map to Fluent ids by prefix, except for the explicit rows, which carry the exact ids that Tasks 21 and 22 test:

| Mock id or prefix | Fluent id |
|---|---|
| `hk-changed-*` | `hv-conn-hk-changed-*` |
| `kbd-*` | `hv-conn-kbd-*` |
| `sshsec-*` | `hv-conn-ssh-secret-*` |
| `vault-*` (including `vault-reset-word`) | `hv-conn-vault-*` (`hv-conn-vault-reset-word`, Task 21) |
| `ret-forever-vault`, `ret-forever-vault-h` | `hv-conn-ret-forever-vault`, `hv-conn-ret-forever-vault-h` (Task 21 `forever_label`) |
| `pin-changed-*` | `hv-conn-tls-pin-changed-*` |
| `access-ro-enforced` | `hv-conn-access-ro-enforced` (Task 22). Guarded reuses the shipped `hv-conn-access-ro-guarded`, so it is not in this table. |
| `ro-warn-*` | `hv-conn-ro-warn-*` (Task 22) |
| `ro-cmd` | `hv-cmd-connection-explain-access` (palette command) |
| `ro-*` (the rest) | `hv-conn-ro-*` |
| `idle-disconnected`, `idle-disconnected-minutes` | `hv-conn-idle-disconnected` (`$name`, `$hours`, Task 22), `hv-conn-idle-disconnected-minutes` (`$name`, `$minutes`) |
| `idle-cap` | `hv-conn-idle-cap` |
| `err-<class>-title`, `-body`, `-body-tls`, `-body-plain` | `error-conn-<class>-title` / `-body` / … (class with `.` and `_` as `-`, e.g. `error-conn-ssh-channel-open-title`) |
| `err-ssh-hop` | `hv-conn-err-ssh-hop` (a UI line, not class copy) |
| `fix-review-host-key` | `hv-conn-fix-review-host-key` (FixId `ReviewHostKey`) |

`{name}`-style placeholders become Fluent variables (`{ $name }`). `{n}`, `{total}`, `{hours}` and `{minutes}` take `NUMBER()`.

Literal data is not translated and is not in the table: host names, fingerprints, commands, the remediation SQL (Task 5 owns it), `~/.ssh/known_hosts` and `vault.bin`. `vault-reset-word` is deliberately the same in both languages but has a row, because Task 21 reads it through Fluent.

Reused M2a strings are in the M2a table and are not repeated: `dlg-cancel`, `dlg-remember`, `ret-*`, `err-title`, `err-tech`, `fix-edit`, `fix-retry`, `fix-review-cert`, `fix-close`, `hk-title`, `hk-body`, `hk-store`, `hk-trust`, `tls-*`, `step-*`, `cap-*` and others. So are the shipped `hv-conn-access-ro-guarded` and `hv-conn-enc-unverified`.

None of the Turkish below has been signed off yet; the Status column says so for each row.

| id | EN | TR | Status |
|---|---|---|---|
| `hk-changed-title` | The host key of {host} changed | {host} sunucusunun anahtarı değişti | PO review |
| `hk-changed-body` | The server answered with a different key from the one stored for it. That happens when a server is rebuilt or its keys are rotated, and also when someone is intercepting the connection. Hacivat stopped before signing in. | Sunucu, kayıtlı anahtarından farklı bir anahtarla yanıt verdi. Bu, sunucu yeniden kurulduğunda ya da anahtarları yenilendiğinde olur; biri bağlantının arasına girdiğinde de olur. Hacivat oturum açmadan önce durdu. | PO review |
| `hk-changed-old` | Stored key | Kayıtlı anahtar | PO review |
| `hk-changed-new` | Offered now | Şimdi gönderilen | PO review |
| `hk-changed-stored-in` | Stored in | Kayıtlı olduğu yer | PO review |
| `hk-changed-src-hacivat` | Hacivat's known hosts | Hacivat'ın bilinen sunucuları | PO review |
| `hk-changed-src-openssh` | {file} line {n} | {file}, {n}. satır | PO review |
| `hk-changed-ask` | Ask whoever runs {host} whether its key changed, and compare the new fingerprint before you trust it. | {host} sunucusunu yöneten kişiye anahtarın değişip değişmediğini sorun ve güvenmeden önce yeni parmak izini karşılaştırın. | PO review |
| `hk-changed-replace` | Remove the old key and trust the new one | Eski anahtarı sil, yenisine güven | PO review |
| `hk-changed-openssh-note` | Hacivat doesn't change {file}. If the new key is genuine, remove the old line in a terminal, then try again: | Hacivat {file} dosyasını değiştirmez. Yeni anahtar gerçekse eski satırı bir terminalde silin, sonra yeniden deneyin: | PO review |
| `hk-changed-copy` | Copy command | Komutu kopyala | PO review |
| `hk-changed-copied` | Copied the command | Komut kopyalandı | PO review |
| `hk-changed-trusted` | Trusted the new key of {host} | {host} sunucusunun yeni anahtarına güvenildi | PO review |
| `kbd-title` | Sign in to {host} | {host} oturumu | PO review |
| `kbd-asks` | {host} asks: | {host} soruyor: | PO review |
| `kbd-from-server` | Sent by the server, shown as received | Sunucunun gönderdiği metin, olduğu gibi | PO review |
| `kbd-no-text` | The server sent no name or instructions. | Sunucu bir ad ya da açıklama göndermedi. | PO review |
| `kbd-show` | Show | Göster | PO review |
| `kbd-hide` | Hide | Gizle | PO review |
| `kbd-attempt` | Attempt {n} of 3 | 3 denemeden {n}. | PO review |
| `kbd-not-kept` | Answers go to this server only and aren't kept. | Yanıtlar yalnızca bu sunucuya gider ve saklanmaz. | PO review |
| `kbd-rejected` | The server didn't accept the answers and asked again. | Sunucu yanıtları kabul etmedi ve yeniden sordu. | PO review |
| `kbd-continue` | Continue | Devam et | PO review |
| `sshsec-hop` | {host}, hop {n} of {total} | {host}, {total} atlamadan {n}. | PO review |
| `sshsec-pw-title` | SSH password for {host} | {host} için SSH parolası | PO review |
| `sshsec-pw-body` | Enter the SSH password for {user}. It's kept only if the connection works. | {user} için SSH parolasını girin. Yalnızca bağlantı kurulursa saklanır. | PO review |
| `sshsec-pass-title` | Passphrase for {key} | {key} için parola ifadesi | PO review |
| `sshsec-pass-body` | This key file is encrypted. Enter its passphrase. It's kept only if the connection works. | Bu anahtar dosyası şifreli. Parola ifadesini girin. Yalnızca bağlantı kurulursa saklanır. | PO review |
| `sshsec-pubkey` | Public key | Açık anahtar | PO review |
| `sshsec-wrong` | {host} didn't accept the password. | {host} parolayı kabul etmedi. | PO review |
| `sshsec-attempt` | Attempt {n} of 3 | 3 denemeden {n}. | PO review |
| `sshsec-continue` | Continue | Devam et | PO review |
| `vault-create-title` | Create a vault for saved passwords | Kayıtlı parolalar için bir kasa oluşturun | PO review |
| `vault-create-why` | This computer has no system keyring. | Bu bilgisayarda sistem anahtarlığı yok. | PO review |
| `vault-create-what` | Hacivat keeps the passwords you save Forever in an encrypted file, locked with a master password. | Hacivat, kalıcı olarak kaydettiğiniz parolaları ana parolayla kilitlenen şifreli bir dosyada tutar. | PO review |
| `vault-master` | Master password | Ana parola | PO review |
| `vault-repeat` | Repeat | Tekrar | PO review |
| `vault-min` | At least 8 characters | En az 8 karakter | PO review |
| `vault-match` | Both entries match | İki giriş eşleşiyor | PO review |
| `vault-mismatch` | The two entries don't match | İki giriş eşleşmiyor | PO review |
| `vault-norecover` | Hacivat can't recover it. If you forget it, you reset the vault and enter your saved passwords again. | Hacivat bu parolayı kurtaramaz. Unutursanız kasayı sıfırlar ve kayıtlı parolalarınızı yeniden girersiniz. | PO review |
| `vault-notnow-note` | Not now keeps this password until Hacivat restarts. | Şimdi değil derseniz bu parola Hacivat yeniden başlayana kadar tutulur. | PO review |
| `vault-notnow` | Not now | Şimdi değil | PO review |
| `vault-create-btn` | Create vault | Kasayı oluştur | PO review |
| `vault-created` | Vault created. Passwords you save Forever go there. | Kasa oluşturuldu. Kalıcı kaydettiğiniz parolalar oraya gider. | PO review |
| `vault-unlock-title` | Unlock the vault | Kasanın kilidini açın | PO review |
| `vault-unlock-body` | Hacivat needs the master password to read the saved password for {name}. The vault stays unlocked until you quit. | Hacivat'ın {name} için kayıtlı parolayı okuyabilmesi için ana parola gerekiyor. Kasa, uygulamadan çıkana kadar açık kalır. | PO review |
| `vault-unlock-wrong` | That isn't the master password. | Bu, ana parola değil. | PO review |
| `vault-attempt` | Attempt {n} of 3 | 3 denemeden {n}. | PO review |
| `vault-unlock-btn` | Unlock | Kilidi aç | PO review |
| `vault-reset-link` | Reset vault… | Kasayı sıfırla… | PO review |
| `vault-reset-title` | Reset the vault? | Kasa sıfırlansın mı? | PO review |
| `vault-reset-body` | This deletes the vault and the {n} passwords saved in it. Your connections stay, and Hacivat asks for each password the next time you connect. This can't be undone. | Bu işlem kasayı ve içindeki {n} kayıtlı parolayı siler. Bağlantılarınız kalır; Hacivat her parolayı bir sonraki bağlanışta sorar. Bu işlem geri alınamaz. | PO review |
| `vault-reset-type` | Type {word} to confirm | Onaylamak için {word} yazın | PO review |
| `vault-reset-word` | reset | reset | PO review |
| `vault-reset-btn` | Reset vault | Kasayı sıfırla | PO review |
| `vault-reset-done` | The vault was reset. Its saved passwords are gone. | Kasa sıfırlandı. İçindeki kayıtlı parolalar silindi. | PO review |
| `vault-damaged-title` | The vault can't be read | Kasa okunamıyor | PO review |
| `vault-damaged-body` | The vault file failed its integrity check, so Hacivat didn't use it and didn't change it. Restore it from a backup, or reset the vault. | Kasa dosyası bütünlük denetiminden geçemedi; Hacivat dosyayı kullanmadı ve değiştirmedi. Bir yedekten geri yükleyin ya da kasayı sıfırlayın. | PO review |
| `vault-damaged-meanwhile` | Until then, passwords you enter are kept until Hacivat restarts. | O zamana kadar girdiğiniz parolalar Hacivat yeniden başlayana kadar tutulur. | PO review |
| `ret-forever-vault` | Forever (encrypted file) | Kalıcı (şifreli dosya) | PO review |
| `ret-forever-vault-h` | in Hacivat's vault, behind a master password | Hacivat'ın kasasında, ana parolayla | PO review |
| `vault-secret` | vault · forever | kasa · kalıcı | PO review |
| `pin-changed-title` | The certificate of {host}:{port} changed | {host}:{port} sertifikası değişti | PO review |
| `pin-changed-body` | This connection is pinned to one certificate, and the server now presents a different one. Hacivat stopped before signing in. | Bu bağlantı tek bir sertifikaya sabitlendi; sunucu şimdi farklı bir sertifika sunuyor. Hacivat oturum açmadan önce durdu. | PO review |
| `pin-changed-pinned` | Pinned | Sabitlenen | PO review |
| `pin-changed-now` | Now | Şimdiki | PO review |
| `pin-changed-ask` | A renewed certificate looks like this too. Ask whoever runs the server before you replace the pin. | Yenilenmiş bir sertifika da böyle görünür. Sabitlemeyi değiştirmeden önce sunucuyu yöneten kişiye sorun. | PO review |
| `pin-changed-note` | Replacing the pin trusts exactly the new certificate for {host}:{port} and removes the old pin. A pin doesn't extend the expiry date. | Sabitlemeyi değiştirmek, {host}:{port} için yalnızca yeni sertifikaya güvenir ve eski sabitlemeyi kaldırır. Sabitleme geçerlilik süresini uzatmaz. | PO review |
| `pin-changed-replace` | Replace pin | Sabitlemeyi değiştir | PO review |
| `pin-changed-done` | Pinned the new certificate of {host} | {host} sunucusunun yeni sertifikası sabitlendi | PO review |
| `access-ro-enforced` | read-only (enforced by server role) | salt okunur (sunucu rolüyle zorunlu) | PO review |
| `ro-kv-access` | Access | Erişim | PO review |
| `ro-sheet-enforced` | read-only · enforced | salt okunur · zorunlu | PO review |
| `ro-sheet-guarded` | read-only · guarded | salt okunur · korumalı | PO review |
| `ro-why` | Why | Neden | PO review |
| `ro-pop-enf-title` | Read-only, enforced by the server | Salt okunur, sunucu tarafından zorunlu | PO review |
| `ro-pop-enf-body` | The role {user} can only read here. The server itself refuses writes, whatever runs in Hacivat. | {user} rolü burada yalnızca okuyabilir. Hacivat'ta ne çalışırsa çalışsın, yazmaları sunucunun kendisi reddeder. | PO review |
| `ro-pop-enf-temp` | TEMP alone doesn't weaken enforcement, so the label stays enforced. | Tek başına TEMP yetkisi zorunluluğu zayıflatmaz; etiket zorunlu kalır. | PO review |
| `ro-pop-g-title` | Read-only is guarded, not enforced | Salt okunurluk korumalı, zorunlu değil | PO review |
| `ro-pop-g-body` | The role {user} isn't limited to reading here, so Hacivat stops writes itself: it checks every statement and runs it read-only. | {user} rolü burada yalnızca okumayla sınırlı değil; bu yüzden yazmaları Hacivat kendisi durdurur: her ifadeyi denetler ve salt okunur çalıştırır. | PO review |
| `ro-pop-g-limit` | That catches mistakes. Only a read-only role on the server is a hard guarantee. | Bu, hataları yakalar. Kesin güvence ancak sunucudaki salt okunur bir rolle sağlanır. | PO review |
| `ro-pop-g-fix` | To enforce it, ask your DBA for a role like this, then make it this connection's user: | Zorunlu hale getirmek için veritabanı yöneticinizden böyle bir rol isteyin ve onu bu bağlantının kullanıcısı yapın: | PO review |
| `ro-pop-maria-body` | Hacivat can't yet confirm that a MariaDB role is read-only, so it always guards MariaDB connections itself. | Hacivat bir MariaDB rolünün salt okunur olduğunu henüz doğrulayamıyor; bu yüzden MariaDB bağlantılarını her zaman kendisi korur. | PO review |
| `ro-pop-no-fix` | No role change makes this label enforced, so there is no SQL to copy. | Hiçbir rol değişikliği bu etiketi zorunlu yapmaz; bu yüzden kopyalanacak SQL yok. | PO review |
| `ro-pop-sqlite-body` | SQLite has no server roles. Hacivat opens the file read-only and lets through only statements that read. | SQLite'ta sunucu rolü yoktur. Hacivat dosyayı salt okunur açar ve yalnızca okuyan ifadelere izin verir. | PO review |
| `ro-copy-sql` | Copy SQL | SQL'i kopyala | PO review |
| `ro-copied` | Copied the SQL. Replace the values in angle brackets before you run it. | SQL kopyalandı. Çalıştırmadan önce köşeli parantez içindeki değerleri değiştirin. | PO review |
| `ro-warn-head` | Found on this role: | Bu rolde bulunanlar: | PO review |
| `ro-warn-superuser` | It is a superuser. | Süper kullanıcı. | PO review |
| `ro-warn-bypass-rls` | It bypasses row-level security. | Satır düzeyi güvenliği atlıyor. | PO review |
| `ro-warn-replication` | It can start replication. | Çoğaltma başlatabilir. | PO review |
| `ro-warn-create-role` | It can create and change roles. | Rol oluşturup değiştirebilir. | PO review |
| `ro-warn-server-file-roles` | It can read or write files on the database server. | Veritabanı sunucusundaki dosyaları okuyabilir ya da yazabilir. | PO review |
| `ro-warn-execute-server-program` | It can run programs on the database server. | Veritabanı sunucusunda program çalıştırabilir. | PO review |
| `ro-warn-signal-backend` | It can cancel or end other sessions. | Başka oturumları iptal edebilir ya da sonlandırabilir. | PO review |
| `ro-warn-write-privileges` | It can change data in some tables. | Bazı tablolardaki verileri değiştirebilir. | PO review |
| `ro-warn-temp-privilege` | It can create temporary tables, which read-only transactions block. | Geçici tablo oluşturabilir; salt okunur işlemler bunları engeller. | PO review |
| `ro-warn-egress-extension` | The {name} extension can reach other servers. | {name} eklentisi başka sunuculara erişebilir. | PO review |
| `ro-warn-role-grant-not-expanded` | Other roles are granted to it, and Hacivat can't see what they allow. | Ona başka roller verilmiş; Hacivat bunların neye izin verdiğini göremiyor. | PO review |
| `ro-warn-unparsed` | Hacivat couldn't read its privileges. | Hacivat yetkilerini okuyamadı. | PO review |
| `ro-warn-non-transactional-write` | It can run functions whose effects a rollback can't undo. | Etkileri ROLLBACK ile geri alınamayan fonksiyonlar çalıştırabilir. | PO review |
| `ro-cmd` | Explain read-only access | Salt okunur erişimi açıkla | PO review |
| `idle-cap` | Disconnected (idle) | Bağlantı kesik (boşta) | PO review |
| `idle-disconnected` | Disconnected {name} after {hours} h idle. It reconnects when you use it. | {name} bağlantısı {hours} sa boşta kaldıktan sonra kesildi. Kullandığınızda yeniden bağlanır. | PO review |
| `idle-disconnected-minutes` | Disconnected {name} after {minutes} min idle. It reconnects when you use it. | {name} bağlantısı {minutes} dk boşta kaldıktan sonra kesildi. Kullandığınızda yeniden bağlanır. | PO review |
| `err-ssh-auth-title` | {host} rejected the SSH sign-in for {user}. | {host}, {user} için SSH oturumunu reddetti. | PO review |
| `err-ssh-auth-body` | Hacivat tried the keys in ssh-agent and {key}. Check that your public key is in ~/.ssh/authorized_keys on that host, or choose another sign-in method. | Hacivat ssh-agent'taki anahtarları ve {key} dosyasını denedi. Açık anahtarınızın o sunucudaki ~/.ssh/authorized_keys dosyasında olduğunu denetleyin ya da başka bir oturum açma yöntemi seçin. | PO review |
| `err-ssh-hop` | Failed at hop {n} ({host}) | {n}. atlamada başarısız ({host}) | PO review |
| `err-ssh-channel-open-title` | {host} doesn't allow tunnels (AllowTcpForwarding) | {host} tünellere izin vermiyor (AllowTcpForwarding) | PO review |
| `err-ssh-channel-open-body` | Signing in to {host} worked, but it refused to forward the connection to {target}. Ask its administrator to allow TCP forwarding for {user}, and to allow {target} if PermitOpen is set. | {host} oturumu açıldı, ancak sunucu bağlantıyı {target} adresine iletmeyi reddetti. Yöneticisinden {user} için TCP iletimine, PermitOpen ayarlıysa {target} adresine de izin vermesini isteyin. | PO review |
| `err-ssh-agent-unavailable-title` | Hacivat can't reach ssh-agent. | Hacivat ssh-agent'a ulaşamıyor. | PO review |
| `err-ssh-agent-unavailable-body` | This connection signs in to {host} with keys from ssh-agent, but SSH_AUTH_SOCK isn't set for Hacivat. Start the agent and add your key with ssh-add, or choose a key file in the connection's SSH settings. | Bu bağlantı {host} oturumunu ssh-agent'taki anahtarlarla açar, ancak Hacivat için SSH_AUTH_SOCK ayarlı değil. Ajanı başlatıp anahtarınızı ssh-add ile ekleyin ya da bağlantının SSH ayarlarında bir anahtar dosyası seçin. | PO review |
| `err-auth-insecure-method-title` | The server asked for the password in a form that isn't safe on this connection. | Sunucu parolayı bu bağlantıda güvenli olmayan bir biçimde istedi. | PO review |
| `err-auth-insecure-method-body-tls` | The connection is encrypted, but the certificate isn't verified, so Hacivat can't be sure the password reaches {host}. It didn't send it. Review the certificate and pin it, then connect again. | Bağlantı şifreli, ancak sertifika doğrulanmadığı için Hacivat parolanın {host} sunucusuna ulaşacağından emin olamıyor ve parolayı göndermedi. Sertifikayı inceleyip sabitleyin, sonra yeniden bağlanın. | PO review |
| `err-auth-insecure-method-body-plain` | The connection isn't encrypted, so the password would cross the network in a form others could read or reuse. Hacivat didn't send it. Turn on TLS on the server, or, if you accept the risk on this network, set Encryption to Disable. | Bağlantı şifreli değil; parola ağda başkalarının okuyabileceği ya da yeniden kullanabileceği bir biçimde taşınırdı. Hacivat parolayı göndermedi. Sunucuda TLS'i açın ya da bu ağda riski kabul ediyorsanız Şifreleme'yi Kapalı yapın. | PO review |
| `err-tls-pin-changed-title` | {host} sent a different certificate from the pinned one. | {host}, sabitlenenden farklı bir sertifika gönderdi. | PO review |
| `err-tls-pin-changed-body` | Hacivat stopped before signing in. Compare the new fingerprint with whoever runs the server before you replace the pin. | Hacivat oturum açmadan önce durdu. Sabitlemeyi değiştirmeden önce yeni parmak izini sunucuyu yöneten kişiyle karşılaştırın. | PO review |
| `err-ssh-host-key-changed-title` | The host key of {host} changed. | {host} sunucusunun anahtarı değişti. | PO review |
| `err-ssh-host-key-changed-body` | Hacivat stopped before signing in. Review the new key before you trust it. | Hacivat oturum açmadan önce durdu. Güvenmeden önce yeni anahtarı inceleyin. | PO review |
| `err-ssh-host-key-revoked-title` | {host} sent a key that {file} marks as revoked. | {host}, {file} dosyasında iptal edilmiş olarak işaretli bir anahtar gönderdi. | PO review |
| `err-ssh-host-key-revoked-body` | Hacivat never connects with a revoked key. Ask the server's administrator for its current key. | Hacivat iptal edilmiş bir anahtarla asla bağlanmaz. Sunucunun güncel anahtarını yöneticisinden isteyin. | PO review |
| `fix-review-host-key` | Review host key… | Sunucu anahtarını incele… | PO review |

122 strings. Not included: mock-only labels (mock bar, Notes drawer) and literal data.
