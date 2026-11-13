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
- Read-only: the access segment in the capsule and the sheet's Access value both open the popover. Copy SQL confirms. A click outside or Esc closes it.
- Idle: clicking the capsule reconnects ("It reconnects when you use it"). Error dialogs: Try again runs and fails again; Review certificate… on `tls.pin_changed` opens surface 5.

**Product truth used:** the M2b plan's rulings B5, B8, B11, B13, B18, B19, B21, B24, B28 and B30, the brief for Task 2, DESIGN.md, and the shipped strings in `locales/*/hacivat-ui.ftl` where an M2b label already exists (`hv-conn-access-ro-guarded`, `hv-conn-enc-unverified`, `hv-conn-pw-*`). The approved M2a surfaces 5 (TLS) and 6 (SSH settings, new host key) are not re-mocked (B28); this mock reuses M2a's new-host-key prompt unchanged.

---

## Cross-cutting decisions

1. **No new hue, no new filled block.** Every new surface uses ink, text-2 and the existing four hues for their existing meanings only. Green appears only as a check that a rule or a hop succeeded. Red appears only for production (lock, swatch, capsule) and for the vault reset Danger button. A failed hop, a mismatch or a rejected answer is an ink ✕ or an ink alert glyph. The capsule's cp-fill block stays the only filled badge (checked in every scenario: exactly one element carries the cp-fill colour).
2. **The safe answer is the default.** In the three hard stops (host key changed, pin changed, vault damaged) the primary button is Cancel or Close, with the initial focus, so Enter and Esc both stop. The trust-widening action is a plain outline button on the far left of the footer, away from the default. The vault reset is a Danger button that stays disabled until the typed word matches and is never triggered by Enter.
3. **Server text is never inside a translated sentence.** Keyboard-interactive name, instruction and prompt labels sit verbatim in their own mono field, labelled "Sent by the server, shown as received". Server messages in errors stay in Technical details. Sentences only interpolate values Hacivat itself chose or dialled (host, user, key path, file path, connection name).
4. **Every SSH prompt names its hop.** The subtitle reads "bastion.acme.io, hop 2 of 2", and the SSH secret prompt boxes the asking hop in the route. Errors draw the route with ✓ / ✕ / pending per hop and state "Failed at hop 2 (bastion.acme.io)".
5. **480 px, Turkish first.** Every dialog is the M2a 480 "wide" size (the vault reset is a 420 confirmation). Inside them the label column narrows to 120 px. A headless pass rendered all 31 scenarios in EN/TR × dark/light (124 renders) at 1440 × 900: no console errors, and no text clipped or overflowing inside any dialog, popover, toast, menu or the capsule. The only ellipsized values are the sheet's Host and Path values in the 150 px value column, which ellipsize by DESIGN.md's long-locale rule (as in M2a).
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
- The command is `ssh-keygen -R bastion.acme.io` for port 22 and `ssh-keygen -R "[bastion.acme.io]:2222"` for any other port (see open question 1).

**Keyboard.** Initial focus on Cancel; Enter or Esc cancels. Tab cycles Copy (stored) → Copy (offered) → [Copy command] → the trust action or Try again → Cancel, trapped in the dialog.

### 2 · Keyboard-interactive (RFC 4256)
**Intent.** Show exactly what the server sent, never mixed into our own words, and ask for the answers in the order it asked.

**Anatomy.**
- 480 px dialog. Title "Sign in to bastion.acme.io"; subtitle: env mark, connection, SSH user, hop.
- Lead line (ours): "bastion.acme.io asks:".
- A mono inset captioned "Sent by the server, shown as received": the server's name (text), then its instruction (text-2), line breaks kept. When both are empty: "The server sent no name or instructions." in the UI face, so it can't be mistaken for server text.
- One to three prompts, stacked: the prompt label verbatim in mono above a full-width field. `echo = false` gives a masked field with the eye toggle; `echo = true` gives a plain field.
- Meta line: "Attempt 1 of 3" and "Answers go to this server only and aren't kept." After a rejected round, an ink alert glyph and "The server didn't accept the answers and asked again." come first.
- Footer: Cancel (ghost), Continue (primary, ↵).
- Switch to TR: the server's English stays English.

**Keyboard.** Initial focus on the first prompt. Enter in any field continues. Tab walks the fields, then Cancel and Continue. Esc cancels the connect (the capsule shows Failed, `ssh.auth`).

### 3 · SSH password and key passphrase, per hop
**Intent.** The M2a password prompt, per hop: it always says which machine on the route is asking and which key is being unlocked.

**Anatomy.**
- 480 px dialog. Titles "SSH password for jump-eu.acme.io" and "Passphrase for ~/.ssh/id_ed25519". Subtitle: env mark, connection, "jump-eu.acme.io, hop 1 of 2".
- Body line mirrors `hv-conn-pw-body`: "Enter the SSH password for deploy@jump-eu.acme.io. It's kept only if the connection works."
- Rows (120 px labels): Route (this Mac → **jump-eu.acme.io** boxed → bastion.acme.io → db-prod-eu.acme.io:5432; the others text-2), SSH user (static mono), Password (masked field with eye), Remember (240 px Select, default Forever, B11 `ssh_retention`).
- Passphrase variant: Key file and Public key (ED25519 `SHA256:` fingerprint) as static mono rows instead of SSH user.
- Wrong password: "jump-eu.acme.io didn't accept the password." and "Attempt 2 of 3" under the fields.
- Footer: Cancel (ghost), Continue (primary, ↵).

**Keyboard.** Initial focus on the secret field. Enter continues. The Select opens with Space, ↓ or ↵; ↑/↓ move, ↵ picks, Esc closes it. Esc on the dialog cancels the connect.

### 4 · Vault (Linux without a system keyring, B13)
**Intent.** Forever means an encrypted file behind a master password. Say why, say the cost (no recovery), and never block the connect on it.

**Anatomy.** The window chrome switches to Linux (no traffic lights, controls on the right) because the vault exists only there.
- **Create** (480): title "Create a vault for saved passwords"; first line, in text, "This computer has no system keyring."; then what Hacivat does; the file (`~/.local/share/hacivat/vault.bin`); Master password and Repeat (masked, eye); two live rules ("At least 8 characters" with a hollow marker, then a green check once met; "Both entries match" / an ink ✕ "The two entries don't match" once both are filled); an info notice "Hacivat can't recover it. If you forget it, you reset the vault and enter your saved passwords again."; and "Not now keeps this password until Hacivat restarts." Footer: Not now (ghost), Create vault (primary, disabled until both rules hold).
- **Unlock** (480): "Hacivat needs the master password to read the saved password for orders-dev. The vault stays unlocked until you quit." One masked field. After a miss: "That isn't the master password." and "Attempt 2 of 3". Footer: Reset vault… (ghost, far left) … Cancel (ghost), Unlock (primary, disabled while empty).
- **Reset** (420 alertdialog, stacked over Unlock): "This deletes the vault and the 6 passwords saved in it. Your connections stay, and Hacivat asks for each password the next time you connect. This can't be undone." Then "Type `reset` to confirm" and a mono field. Footer: Cancel (ghost), Reset vault (Danger, trash icon, disabled until the field reads exactly `reset`).
- **Damaged file** (480 alertdialog): the integrity check failed, the file was neither used nor changed, restore or reset, and "Until then, passwords you enter are kept until Hacivat restarts." Footer: Reset vault… (outline, far left) … Close (primary).
- **Remember Select:** the fourth option reads "Forever (encrypted file)" with the hint "in Hacivat's vault, behind a master password" (it replaces M2a's "Forever (Keychain)" and the disabled `hv-conn-ret-forever-off-h`). The sheet's Password row reads "vault · forever".

**Keyboard.** Create: focus on Master password; Enter creates only when enabled. Unlock: focus on the field; Enter unlocks. Reset: focus on the typed-confirmation field; Enter does nothing; Tab reaches Cancel, then Reset vault once enabled; Esc returns to Unlock. Damaged: focus on Close.

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
**Intent.** Read-only says how it is enforced, by the server's role or only guarded by Hacivat, and the difference is one click away, with the fix.

**Anatomy.**
- **Capsule access segment:** lock + "read-only (enforced by server role)" when the role probe found a least-privilege role (PostgreSQL, MySQL); lock + "read-only (guarded)" otherwise (MariaDB and SQLite always). The lock is red only on production; elsewhere it is text-2, and the Explorer's lock flag follows the same rule (M2a drew every read-only lock red).
- **Popover** (300 px, the M2a explanation popover anatomy) under the segment:
  - Enforced: "Read-only, enforced by the server" and "The role app_ro can only read here. The server itself refuses writes, whatever runs in Hacivat."
  - Guarded (PostgreSQL, MySQL): "Read-only is guarded, not enforced"; why ("The role qa isn't limited to reading here, so Hacivat stops writes itself…"); the limit ("Only a read-only role on the server is a hard guarantee."); the fix line; a SQL inset with `<role>`, `<password>` and `<database>` placeholders (italic text-2; keywords and strings in the syntax colours); Copy SQL.
  - MariaDB: always guarded, says why, and still offers the MySQL-syntax role snippet ("The label stays guarded").
  - SQLite: no server roles; no snippet.
- **Sheet Access row** (new row under Environment): lock + "read-only · enforced" or "read-only · guarded" (short forms, so Turkish fits the 150 px column); the whole value is the popover trigger, underlined on hover. Read-write rows show "read-write" in text-2.
- **Status bar:** the environment segment carries the capsule's label ("production · read-only (enforced by server role)").

PostgreSQL snippet:
```sql
CREATE ROLE <role> LOGIN
  PASSWORD '<password>';
GRANT pg_read_all_data TO <role>;
REVOKE TEMP ON DATABASE <database>
  FROM PUBLIC;
```
MySQL and MariaDB snippet:
```sql
CREATE USER '<role>'@'%'
  IDENTIFIED BY '<password>';
GRANT SELECT, SHOW VIEW
  ON <database>.* TO '<role>'@'%';
```

**Keyboard.** The capsule segment and the sheet value open the popover on click. From the keyboard it opens through a palette command, as M2a's "unencrypted" popover does ("Opened from the capsule segment or its command", DESIGN.md). Esc closes it and returns the focus to the trigger. Copy SQL is a Button inside the popover and takes Tab focus while it is open.

### 7 · Idle and the new error copy
**Intent.** An idle disconnect is quiet and reversible. New error classes use the M2a error-dialog anatomy unchanged.

**Anatomy.**
- **Idle toast** (the existing 320 px Toast, unplug icon, standard TTL): "Disconnected orders-dev after 4 h idle. It reconnects when you use it." The capsule keeps the environment and name and adds a segment with the unplug glyph and "Disconnected (idle)" in text-2; the Explorer row's meta shows the unplug glyph; the status bar says the same. Clicking the capsule reconnects.
- **Errors** (480, M2a anatomy: message, explanation, step trail, fixes, Technical details, Close):

| Class | Message | Body (short) | Fixes (existing FixIds) |
|---|---|---|---|
| `auth.insecure_method` (TLS, not verified) | The server asked for the password in a form that isn't safe on this connection. | Encrypted but unverified, so the password wasn't sent; pin the certificate. | **Review certificate…**, Edit connection |
| `auth.insecure_method` (no TLS) | same | Not encrypted, so the password wasn't sent; turn on TLS, or set Encryption to Disable to accept the risk. | **Edit connection** |
| `ssh.auth` | bastion.acme.io rejected the SSH sign-in for deploy. | Which keys were tried; check authorized_keys or choose another method. "Failed at hop 2 (bastion.acme.io)" and the hop route. | **Edit connection**, Try again |
| `ssh.channel_open` | bastion.acme.io doesn't allow tunnels (AllowTcpForwarding) | Signing in worked, forwarding to db-prod-eu.acme.io:5432 was refused; AllowTcpForwarding and PermitOpen. Hop 2. | **Try again**, Edit connection |
| `ssh.agent_unavailable` | Hacivat can't reach ssh-agent. | SSH_AUTH_SOCK isn't set; start the agent and ssh-add, or choose a key file in the SSH settings. Hop 1. | **Try again**, Edit connection |
| `tls.pin_changed` | db-qa.acme.io sent a different certificate from the pinned one. | Stopped before signing in; compare before replacing. | **Review certificate…** (opens surface 5), Edit connection |
| `ssh.host_key_changed` | The host key of bastion.acme.io changed. | Stopped before signing in; review the new key. Hop 2. | **Review host key…** (opens surface 1), Edit connection |
| `ssh.host_key_revoked` | bastion.acme.io sent a key that ~/.ssh/known_hosts marks as revoked. | Never connects with a revoked key; ask for the current one. Hop 2. | Edit connection |

- Under SSH the step trail is SSH → Connect → TLS → Sign in → Check server (Resolve drops out, B11). Technical details show the stage with the hop host ("SSH · bastion.acme.io"), the class, the code chip, the server's own text only when it sent some (the `ssh.auth` method list, the channel-open description), and the elapsed time.

**Keyboard.** As M2a: initial focus on the first fix; Tab through fixes, Technical details, Copy details and Close; Esc closes.

---

## NEW primitives

All use harman tokens only and are reproducible in GPUI (solid fills, 1 px borders, radii 3/4/6, the popover shadow only).

| Primitive | Anatomy and exact tokens |
|---|---|
| **Key comparison** | Two fingerprint insets stacked 8 px apart (panel / ground, 1 px seam-strong, radius 4, padding 8/10). Header: label 12.5 text-2, algorithm Fragment Mono 11.5 text, a ghost icon Copy on the right. Value Fragment Mono 11.5/17 text. An optional "Stored in" line under a 1 px seam (12 px text-2 + mono 11.5 text). Labels state facts (Stored key / Offered now, Pinned / Now); no hue distinguishes them. |
| **Command inset** | The same inset with the command in Fragment Mono 11.5/17 text (wraps anywhere) and a ghost Copy command button on the right. |
| **Server-text inset** | The same inset, caption 12 text-2 "Sent by the server, shown as received", name in mono 11.5 text, instruction in mono 11.5 text-2, `white-space: pre-wrap`. Prompts below it: mono 11.5 label (text) above a full-width TextInput, 10 px apart. |
| **Hop route** | The M2a route row (12 px text-2, arrows 12 px). The asking hop is boxed: 1 px seam-strong, radius 3, padding 0/4, text; other stops text-2. In errors each hop carries the step-trail marks: check 12 px green, ✕ 12 px ink with the host at weight 600, 7 px hollow seam-strong square for pending. |
| **Validation lines** | 12 px rows, 3 px apart, under the field column: 7 px hollow seam-strong square (unmet), check 12 px green (met), ✕ 12 px ink with text ink (mismatch). |
| **SQL inset in the popover** | Fingerprint-inset anatomy inside the 300 px popover, `pre` in Fragment Mono 11.5/17; keywords syntax-keyword, strings syntax-string, placeholders text-2 italic. Copy SQL is a 22 px Button right-aligned under it. Lines are pre-broken to ≤ 36 characters so nothing scrolls. |
| **Capsule: access and idle segments** | Access: lock 12 px (red on production, text-2 otherwise) + label text; clickable. Idle: unplug 12 px + label, both text-2. |
| **Sheet Access row** | The kv row; the value is a borderless button (lock 12 px + label 12.5 text, ellipsis), underline on hover. |
| **Linux window chrome** | No traffic lights; three 28 × 24 icon slots (minus, square, x at 12 px text-2) after the palette. Mock-only stand-in for the platform frame. |

## Open questions for the PO

Each was answered by the ruling noted, so the build is not blocked; the PO can overrule any.

1. **The `ssh-keygen -R` command.** The brief shows `ssh-keygen -R "[bastion.acme.io]:22"`. OpenSSH writes port-22 entries as the bare host, so that command would not remove the line. *Ruling:* the command is derived from the port: `ssh-keygen -R bastion.acme.io` for 22 and `ssh-keygen -R "[host]:port"` otherwise (B11 known-hosts rules; Task 4's `non_default_port_uses_bracket_form`). The mock shows both.
2. **Vault reset: "the primary button is disabled until it matches".** That conflicts with "no destructive action is ever the default or primary". *Ruling:* the confirm button sits in the primary slot but is the Danger variant: disabled until the field reads `reset`, outline-only until hover or focus, and Enter in the field never triggers it (B13: typing the word deletes the vault; DESIGN.md Danger button).
3. **Is the typed word translated?** *Ruling:* no. It is the literal `reset` in both languages, shown in mono, so Turkish casing (İ/ı) and keyboard layouts can't make the match fail (B13).
4. **Colour of "Remove the old key and trust the new one" and "Replace pin".** *Ruling:* plain outline buttons, not Danger red. Red means destroying user data (Hue Is Meaning); these widen trust, which is guarded by placement (far left), by Cancel being primary with the focus, and by the dialog being an alertdialog (B5, B11).
5. **What the capsule shows after an idle disconnect.** No mock existed. *Ruling:* the capsule keeps the environment and name and adds "Disconnected (idle)" with the unplug glyph in text-2; using the connection, or clicking the capsule, reconnects lazily (B18). It is not "No connection", because the profile is still the active one.
6. **The PostgreSQL remediation snippet revokes TEMP from PUBLIC**, which affects every role on that database. *Ruling:* keep it, because it mirrors the probe's reference role `hv_ro` (`pg_read_all_data`, `REVOKE TEMP`; B17, B21 L5). The snippet must match the role probe's criteria exactly; if the probe does not require "no TEMP", Task 22 drops the last statement.
7. **MariaDB's popover offers a role snippet although the label stays guarded.** *Ruling:* offer it, and say the label stays guarded. A read-only role still makes the server refuse writes, and flipping the MariaDB label is a gate decision (B16, B21, C19).
8. **The sheet uses a shorter label than the capsule.** *Ruling:* "read-only · enforced / guarded" in the sheet (150 px value column; a trust label must not ellipsize in Turkish), the full label in the capsule, the status bar and the popover (B21; DESIGN.md long-locale rule).
9. **`ssh.agent_unavailable` has no "Use a key file" fix.** *Ruling:* the FixId set is unchanged in M2b (B24), so the body says where to choose a key file and the fixes are Try again and Edit connection.
10. **Two bodies for `auth.insecure_method`.** *Ruling:* unverified TLS points to pinning (Review certificate…, which leads to the `Pinned` outcome the guard allows); no TLS advises turning TLS on, or Disable to accept the risk, exactly the cost B8 records.
11. **Remember for keyboard-interactive answers.** *Ruling:* none. Answers are usually one-time codes; only passwords and key passphrases have secret slots (B11, `SecretSlot::SshPassphrase { hop }`).
12. **Copy for classes outside the brief's list.** *Ruling:* `ssh.host_key_changed` and `ssh.host_key_revoked` get error copy here too, because Cancel in surface 1 lands in the first and B24 adds the second.
13. **Guarded label in Turkish.** *Ruling:* reuse the shipped `hv-conn-access-ro-guarded` translation unchanged, so the M2a label and the M2b popover title agree (B21).
14. **Headline punctuation for `auth.insecure_method`.** The brief quotes it without a full stop. *Ruling:* add one, like every other `error-conn-*-title` that is a sentence. "…doesn't allow tunnels (AllowTcpForwarding)" stays without one, as quoted, because it ends in a parenthesis (B24).
15. **Vault "Not now".** *Ruling:* not creating a vault keeps the secret until restart, and the dialog says so before the choice (B13, Q38).

---

## Copy strings (EN / TR)

The ids map to Fluent ids by prefix:

| Mock prefix | Fluent id |
|---|---|
| `hk-changed-*` | `hv-conn-hk-changed-*` |
| `kbd-*` | `hv-conn-kbd-*` |
| `sshsec-*` | `hv-conn-ssh-secret-*` |
| `vault-*` | `hv-conn-vault-*` |
| `pin-changed-*` | `hv-conn-tls-pin-changed-*` |
| `ro-*` | `hv-conn-ro-*` |
| `idle-*` | `hv-conn-idle-*` |
| `err-<class>-title`, `-body`, `-body-tls`, `-body-plain` | `error-conn-<class>-title` / `-body` / … (class with `.` and `_` as `-`, e.g. `error-conn-ssh-channel-open-title`) |
| `err-ssh-hop` | `hv-conn-err-ssh-hop` (a UI line, not class copy) |
| `fix-review-host-key` | `hv-conn-fix-review-host-key` (FixId `ReviewHostKey`) |

`{name}`-style placeholders become Fluent variables (`{ $name }`); `{n}`, `{total}` take `NUMBER()`. Literal data (host names, fingerprints, commands, SQL, the confirmation word `reset`, `~/.ssh/known_hosts`, `vault.bin`) is not translated and is not in the table. Reused M2a strings (`dlg-cancel`, `dlg-remember`, `ret-*`, `err-title`, `err-tech`, `fix-edit`, `fix-retry`, `fix-review-cert`, `fix-close`, `hk-title`, `hk-body`, `hk-store`, `hk-trust`, `tls-*`, `step-*`, `cap-*` …) are in the M2a table and are not repeated. None of the Turkish below has been signed off yet; the Status column says so for each row.

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
| `vault-reset-btn` | Reset vault | Kasayı sıfırla | PO review |
| `vault-reset-done` | The vault was reset. Its saved passwords are gone. | Kasa sıfırlandı. İçindeki kayıtlı parolalar silindi. | PO review |
| `vault-damaged-title` | The vault can't be read | Kasa okunamıyor | PO review |
| `vault-damaged-body` | The vault file failed its integrity check, so Hacivat didn't use it and didn't change it. Restore it from a backup, or reset the vault. | Kasa dosyası bütünlük denetiminden geçemedi; Hacivat dosyayı kullanmadı ve değiştirmedi. Bir yedekten geri yükleyin ya da kasayı sıfırlayın. | PO review |
| `vault-damaged-meanwhile` | Until then, passwords you enter are kept until Hacivat restarts. | O zamana kadar girdiğiniz parolalar Hacivat yeniden başlayana kadar tutulur. | PO review |
| `vault-ret-forever` | Forever (encrypted file) | Kalıcı (şifreli dosya) | PO review |
| `vault-ret-forever-h` | in Hacivat's vault, behind a master password | Hacivat'ın kasasında, ana parolayla | PO review |
| `vault-secret` | vault · forever | kasa · kalıcı | PO review |
| `pin-changed-title` | The certificate of {host}:{port} changed | {host}:{port} sertifikası değişti | PO review |
| `pin-changed-body` | This connection is pinned to one certificate, and the server now presents a different one. Hacivat stopped before signing in. | Bu bağlantı tek bir sertifikaya sabitlendi; sunucu şimdi farklı bir sertifika sunuyor. Hacivat oturum açmadan önce durdu. | PO review |
| `pin-changed-pinned` | Pinned | Sabitlenen | PO review |
| `pin-changed-now` | Now | Şimdiki | PO review |
| `pin-changed-ask` | A renewed certificate looks like this too. Ask whoever runs the server before you replace the pin. | Yenilenmiş bir sertifika da böyle görünür. Sabitlemeyi değiştirmeden önce sunucuyu yöneten kişiye sorun. | PO review |
| `pin-changed-note` | Replacing the pin trusts exactly the new certificate for {host}:{port} and removes the old pin. A pin doesn't extend the expiry date. | Sabitlemeyi değiştirmek, {host}:{port} için yalnızca yeni sertifikaya güvenir ve eski sabitlemeyi kaldırır. Sabitleme geçerlilik süresini uzatmaz. | PO review |
| `pin-changed-replace` | Replace pin | Sabitlemeyi değiştir | PO review |
| `pin-changed-done` | Pinned the new certificate of {host} | {host} sunucusunun yeni sertifikası sabitlendi | PO review |
| `ro-enforced` | read-only (enforced by server role) | salt okunur (sunucu rolüyle zorunlu) | PO review |
| `ro-guarded` | read-only (guarded) | salt okunur (korumalı) | PO review |
| `ro-kv-access` | Access | Erişim | PO review |
| `ro-sheet-enforced` | read-only · enforced | salt okunur · zorunlu | PO review |
| `ro-sheet-guarded` | read-only · guarded | salt okunur · korumalı | PO review |
| `ro-why` | Why | Neden | PO review |
| `ro-pop-enf-title` | Read-only, enforced by the server | Salt okunur, sunucu tarafından zorunlu | PO review |
| `ro-pop-enf-body` | The role {user} can only read here. The server itself refuses writes, whatever runs in Hacivat. | {user} rolü burada yalnızca okuyabilir. Hacivat'ta ne çalışırsa çalışsın, yazmaları sunucunun kendisi reddeder. | PO review |
| `ro-pop-g-title` | Read-only is guarded, not enforced | Salt okunurluk korumalı, zorunlu değil | PO review |
| `ro-pop-g-body` | The role {user} isn't limited to reading here, so Hacivat stops writes itself: it checks every statement and runs it read-only. | {user} rolü burada yalnızca okumayla sınırlı değil; bu yüzden yazmaları Hacivat kendisi durdurur: her ifadeyi denetler ve salt okunur çalıştırır. | PO review |
| `ro-pop-g-limit` | That catches mistakes. Only a read-only role on the server is a hard guarantee. | Bu, hataları yakalar. Kesin güvence ancak sunucudaki salt okunur bir rolle sağlanır. | PO review |
| `ro-pop-g-fix` | To enforce it, ask your DBA for a role like this, then make it this connection's user: | Zorunlu hale getirmek için veritabanı yöneticinizden böyle bir rol isteyin ve onu bu bağlantının kullanıcısı yapın: | PO review |
| `ro-pop-maria-body` | Hacivat can't yet confirm that a MariaDB role is read-only, so it always guards MariaDB connections itself. | Hacivat bir MariaDB rolünün salt okunur olduğunu henüz doğrulayamıyor; bu yüzden MariaDB bağlantılarını her zaman kendisi korur. | PO review |
| `ro-pop-maria-fix` | A read-only role still makes the server refuse writes. The label stays guarded: | Salt okunur bir rol yine de sunucunun yazmaları reddetmesini sağlar. Etiket korumalı kalır: | PO review |
| `ro-pop-sqlite-body` | SQLite has no server roles. Hacivat opens the file read-only and lets through only statements that read. | SQLite'ta sunucu rolü yoktur. Hacivat dosyayı salt okunur açar ve yalnızca okuyan ifadelere izin verir. | PO review |
| `ro-copy-sql` | Copy SQL | SQL'i kopyala | PO review |
| `ro-copied` | Copied the SQL. Replace the values in angle brackets before you run it. | SQL kopyalandı. Çalıştırmadan önce köşeli parantez içindeki değerleri değiştirin. | PO review |
| `idle-toast` | Disconnected {name} after {duration} idle. It reconnects when you use it. | {name} bağlantısı {duration} boşta kaldıktan sonra kesildi. Kullandığınızda yeniden bağlanır. | PO review |
| `idle-hours` | {n} h | {n} sa | PO review |
| `idle-cap` | Disconnected (idle) | Bağlantı kesik (boşta) | PO review |
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

107 strings. Not included: mock-only labels (mock bar, Notes drawer) and literal data.
