# M3b "Safe": design notes

**Binding for M3b (surfaces 1–13). Task 2b appends the M3c surfaces (14–24) to this file as a `## M3c surfaces` part and to `index.html` as new sections.**

Clickable mock: `design/m3b-safe/index.html` (one file, one app, a surface switcher). It is the M3a mock's world, unchanged: the same CSS (copied, with the M3b additions after it), the same Lucide 0.460 sprite (ISC) plus seven icons, the same mock bar, Notes drawer, toggles and deep links. It runs with no network except Google Fonts. The M3a strings are reused verbatim (`i18n-m3a`); M3b never redefines an M3a or earlier Fluent id (the mock checks this at boot and the verification pass asserts it).

The world is the one after Task 26: production consoles are read-write behind the gate. Surface 13 keeps the M3a read-only notice as a comparison scenario only.

**How to drive it**
- The dashed **mock bar** is not product UI. Row 1 holds the thirteen surfaces. Row 2 holds the scenarios of the current surface (`[` / `]`), the window size (Fit, 1280 × 800, 960 × 600; `W`), EN/TR (`L`), dark/light (`T`) and Notes (`N`).
- Keys `1`–`9`, `0`, `-`, `=` jump to surfaces 1–12. Surface 13 is reached with the bar or a deep link.
- Deep links: `?s=1&sc=type&theme=light&lang=tr&size=960&bar=0`. `bar=0` hides the mock bar, so the window is exactly the viewport. `shot=1` freezes every clock (the run clock, the D-014 countdown, the escalation clock, the badge age) for screenshots.
- Every scenario named "960 · TR" pins Turkish at 960 × 600; an explicit `lang` or `size` in the link wins.
- The Notes drawer repeats these notes per surface and lists the surface's strings (Fluent id, EN, TR with `Review: PO`).
- Numbers and dates follow the language toggle as a stand-in for the format locale (the M2a stand-in); the product follows the OS region (L3, L15(d)).

**Flows that run**
- **Gate confirmation.** In any confirm scenario: Cancel (or Enter, or Escape) sends nothing and logs it; Execute runs; in "Statement 2 of 3" Execute moves to statement 3; Skip / Skip all log the skip. Type-to-confirm: type `müşteriler` (exact bytes) to enable Execute; `Müşteriler` keeps it disabled and shows the hint; Enter in the field never executes.
- **Confirm in transaction.** The countdown repaints once a second (open without `shot=1`); Commit / Roll back (own form) and Keep / Undo (savepoint form) update the gutter, the badge and Output; Escape answers Roll back / Undo; at 0:00 the statement is undone and Output says so. In the Production surface, Run on the `UPDATE` line opens this prompt.
- **Transaction chip and badge.** The chip opens the mode menu (→ opens Isolation); the status segment opens the same menu above the bar; the badge's Commit / Roll back end the transaction.
- **Idle toast.** Commit / Roll back end the transaction; Keep open only dismisses the toast.
- **Cancel escalation.** In "Slow cancel" Show options… (or ⌘., a second Stop) opens the dialog; Keep waiting leaves the hourglass in the gutter (once a second); Terminate on server and Disconnect end it with their log lines.
- **Error bar.** Ignore / Ignore all continue, Stop skips the rest, Roll back and continue (failed PG transaction) rolls back and continues. F6 moves the focus into the bar; Escape there answers Stop.
- **Parameters.** NULL toggles; "Use 1.234" clears the flag and enables Run; typing a Turkish-grouped number (`1.500`) flags it again.
- **Chooser.** ↓ highlights Run as one, ↵ runs it into an untyped tab; Customize… opens the policy popover.
- **Results.** Right-click a result tab for Pin / Unpin, Close tab, Close other tabs; Σ Count rows counts, Cancel stops it; Fetch all asks first.
- **History.** Type to search (`musteri` finds `müşteri` and `MÜŞTERİLER`), ↑/↓ move, ↵ inserts at the caret, ⌘↵ inserts and runs through the gate; the scope and outcome filters work.
- **Single session.** The checkbox swaps the Connections line.

**Product truth used:** the M3b plan (`docs/superpowers/plans/2026-11-16-m3b-safe-and-complete.md`: "New UI surfaces" 1–13, Tasks 2a, 21–27, R-M3-55, -60, -61, -66, -68, -76, -78, -79, -84, P-M3b-1, -5, -19, -30), its Global Constraints, the spec rows "Production UX" and "Sessions and transactions", D-013, D-014, D-035, D-036, D-061, D-062, alg/20 HV-EXEC-08/-09/-10/-12, alg/13 SQL-SAFE-07, DESIGN.md, `design/m3a-query/NOTES.md` (binding for every M3a surface touched here), `design/m2-connect/NOTES.md` (the connection dialog and its "No extra confirmations" line off production, untouched) and `design/m2b-secure/NOTES.md` (dialog anatomy, safe default, the vault-reset type-to-confirm precedent).

---

## Cross-cutting decisions

1. **No new hue, no new filled block.** Red: production marks and destructive actions only (the gate's Execute, Disconnect). Amber: pending (the badge stamp, the statement waiting for an answer, the escalation hourglass, the counting marker). Green: a check that something succeeded. Blue: selection, focus, the one primary button. Every failure, refusal or ambiguity is an ink glyph. The capsule's cp-fill block stays the window's one filled badge (checked in every render).
2. **The safe answer is the default.** Every M3b confirmation is an alertdialog whose primary button is the safe answer with the initial focus: Cancel (gate), Roll back or Undo (confirm in transaction), Keep waiting (escalation). Enter acts on the focused button only, on key-down, never repeated, never with a modifier. Escape answers the same safe value. Closing the window or quitting answers it too.
3. **Fail-closed.** When something is ambiguous nothing is sent: a flagged parameter value or a template variable keeps Run disabled; a type-to-confirm field compares exact bytes; Run as one is never preselected; a gate dialog closed by any means answers Cancel.
4. **No silent rollback of the user's transaction (D-014).** Only Hacivat's own transactions end without an answer: the confirm-in-transaction deadline undoes the confirmed statement only (`ROLLBACK TO SAVEPOINT` inside a user transaction), and the dialog says so before it happens. The idle toast's Keep open never ends anything.
5. **Escape never cancels a running statement** (R-M3-38). In every M3b dialog and in the error bar Escape answers the safe value; it never sends Terminate or Disconnect, and Stop in the error bar ends only the paused run.
6. **Production is unmistakable.** After the unlock the hairline, the tab swatch, the capsule's filled block and the status bar's red production segment are unchanged (only its lock goes, RC-6: a lock means read-only); the console additionally shows its Manual chip. Every dialog's subtitle carries the environment mark and the connection name in mono.
7. **Trust signals never clip.** Environment (capsule block, status segment, tab swatch), transaction (badge stamp and "Transaction · n statements", status segment, mode chip), read-only (lock chip, capsule access) and cursor ("Cursor open") are never hidden or truncated; the Overflow Rule collapses other items first. The verification pass checks each one is drawn, whole and inside its clipping ancestor in every render.
8. **The Raised Ink Rule holds.** Toolbars, the error bar, menus, popovers, toasts and dialogs map text-3 to text-2; field wells (SQL inset, value fields, the history list, the server-message box) restore text-3 on their own fill. Disabled items keep text-3.
9. **Countdowns and clocks repaint at most once a second** (the D-014 countdown, the escalation clock, the run clock); the badge age once a minute. Nothing spins or pulses; "Counting…" uses the static amber run marker.
10. **Server text is never inside a translated sentence.** Terminate errors, notices, warnings and history failures show the server's text verbatim in its own mono field, with the severity and code as chips.
11. **GPUI-reproducible.** Solid fills, 1 px borders, radii 0/3/4/6, the popover shadow only; no transforms, filters, blur or masks.
12. **Fits 960 × 600 in Turkish.** Every scenario was rendered in EN/TR × dark/light at 1280 × 800 and 960 × 600 (Verification).

---

## Surfaces

### 1 · Gate confirmation dialog (Task 21)
**Intent.** A statement the gate marks `Confirm` or `Warn` asks once, before anything is sent, naming the connection, showing the SQL and saying in words why it asks.

**Anatomy** (480 px alertdialog over the scrim below the title bar; the M2b dialog anatomy).
- **Header:** title 13/600 `text` "Run this statement on orders-prod?" (Warn: "This statement locks rows on orders-prod"). Subtitle 12.5 `text-2`: the 7 px environment mark, the connection in Fragment Mono 11.5 `text`, the engine and major version, and for a multi-unit run "statement 2 of 3".
- **Body** (padding 12/16, scrolls when the window is short):
  - caption "Statement" (12 `text-2`) over the **SQL inset**: panel (dark) / ground (light), 1 px `seam-strong`, radius 4, padding 7/10, Fragment Mono 11.5/17 with the syntax tokens, at most 6 lines then it scrolls (4 px `seam-strong` thumb); the masked final text Run would send (P-M3b-2).
  - **Reasons**: a list, 12.5 `text`, 3 px square `text-2` bullets; identifiers in Fragment Mono 11.5 and bidi-isolated (`<bdi>`, FSI/PDI in GPUI). One reason per `DestructiveKind`/reason code.
  - **Effect** row: "Effect" (`text-2`) and the GateClass in words (`text`).
  - MySQL/MariaDB DDL: an inline notice (info glyph `text-2`) "MySQL commits DDL at once, so this can't be rolled back."
  - **Type-to-confirm** (DmlUnsafe on production; `DROP SCHEMA`/`DATABASE`): "Type `müşteriler` to run it" (12.5 `text`; the name is a selectable mono run followed by a 20 px ghost **Copy** icon button, so a name the keyboard can't reproduce, e.g. an NFD spelling in pasted SQL, can still be pasted) over a full-width mono field (26 px). After a mismatch the field border turns `text-2` and the ink circle-alert line says "Type it exactly as shown, letter case included."
  - Multi-unit only, 12 px `text-2`: the **Execute all scope line** (RC-1): "Execute all also runs the remaining 1 statement that would ask, without asking. A statement that needs its name typed still asks, and a production change still waits for your answer after it runs."
  - When "don't ask again" is offered, one 12 px `text-2` line names its scope: "Not asked again for statements of the same shape in orders-prod until it closes."
  - Last line, 12.5 `text-2`: "Nothing is sent until you choose." Off production (calm variant): "Off production, Hacivat asks only before statements like this one. Settings › Execution can turn it off."
- **Footer** (padding 12/16/16): two groups 28 px apart (`column-gap`).
  - Left, the run group: **Execute** (Danger), **Execute all** (multi-unit only), **Execute and don't ask again** (Danger; on production its label names the scope: "Execute; don't ask again in this console").
  - Right, the stop group: **Skip**, **Skip all** (multi-unit only; Default), **Cancel** (Primary, initial focus, minimum 72 px).
  - **Wrapping rule:** when both groups don't fit one line, the right group wraps under the left one, still right-aligned (the M2a footer rule). Buttons never truncate. At 960 × 600 in Turkish the multi-unit footer takes two lines; the single-unit footers fit one.
  - **Isolation (RC-3):** the three Danger buttons of a multi-unit footer stay Danger (each sends a destructive statement; outline-only at rest). Inside a dialog footer the isolation DESIGN.md:558 asks for is the 28 px column gap between the run and stop groups, with no seam (the M2b vault-reset and damaged-vault footers are the precedent).
- **Execute all (RC-1, binding):**
  - It is disabled until the current unit's name is typed, exactly like Execute (same exact-bytes rule).
  - It answers only the remaining units that would show this dialog **without** `type_to_confirm`; a later unit that needs its name typed still asks, and a `RunInTxThenConfirm` unit still gets its Commit / Roll back or Keep / Undo prompt after it runs (Execute all never pre-answers it).
  - It is Danger, unless this unit and every unit it would answer are Warn (then Default).
  - Task 21 pins it with `execute_all_disabled_until_exact_bytes` and `execute_all_never_answers_a_type_to_confirm_unit`.
  - **The count in the scope line (R1-m1):** Task 21's `{ $n }` in `hv-confirm-execute-all-scope` ("the remaining { $n } statements that would ask") counts only the later units that would show this dialog **without** `type_to_confirm`: units that run without asking, units that will still ask for a typed name and `RunInTxThenConfirm` units are not counted. (The mock's fixtures show `n − i`, which equals that count there.)
- **Warn** (`Lock`, `FOR UPDATE`; RC-2): the same anatomy; its Execute and "don't ask again" are **Default** buttons (a row lock changes no data) and are never focused; Cancel still has the focus. Task 21's test is `cancel_has_initial_focus_and_execute_is_danger_except_lock`: Default and unfocused for `Lock`, Danger for every other Confirm class, the calm variant included.

**States.** Production · one statement (Unknown: an `UPDATE` calling a function Hacivat can't classify) · Statement 2 of 3 (`DROP TABLE`) · Statement 2 of 3 · type the name (Execute and Execute all disabled) · Type the name (no WHERE) · Name typed · Wrong letter case · DROP SCHEMA (CASCADE, type the name) · MySQL DDL (can't be rolled back) · MySQL implicit commit (DDL in an open transaction) · Routine (`CALL`) · Off production (calm) · Warn (`FOR UPDATE`) · After "don't ask again" (Output) · 960 × 600 in Turkish.

**Keyboard and roles.** `role=alertdialog`, `aria-modal`, `aria-describedby` = the reasons list; Execute all is described by its scope line. Initial focus Cancel. Enter (key-down, no repeat, no modifier) acts on the focused button; Enter in the type field never executes. Escape, the close icon, ⌘W and closing the window answer Cancel. Tab is trapped in the card and follows the visual order: the type field (when present) → its Copy button → the run group → the stop group. **No access keys** on any button (labels may sit next to SQL).

### 2 · Confirm-in-transaction dialog (Task 21; production only, P-M3b-30)
**Intent.** A production DML has run inside a transaction and waits for an answer; locks are held meanwhile (D-014). The safe answer is the default and a countdown says when Hacivat undoes it.

**Which form (RC-5, binding).** The form follows `CommitQuestion.savepoint`, which the session sets from live state (P-M3b-1): **Manual and Smart sessions always get the savepoint form** (the session has opened the transaction first, so there is always one to keep the change in); **only Auto-commit with no transaction open gets Hacivat's own `BEGIN`** and the Commit / Roll back form. Production consoles start in Manual (D-036), so the commonest production write is the savepoint form on a console with no badge yet.

**Anatomy** (480 px alertdialog).
- **Savepoint form** (Manual / Smart, or Auto inside an open transaction): title "1 row changed inside your transaction"; the SQL inset; two lines that name the semantics: "Keep: the change joins your open transaction. Nothing is committed yet." / "Undo: removes this statement's change; your transaction stays open."; then "With no answer, Hacivat undoes the statement after 5 minutes and leaves your transaction open." Footer: countdown "Undoes this statement in 4:51"; **Keep** (Default) and **Undo** (Primary, initial focus; Turkish "İfadeyi geri al"). On a Manual console's first change there is no badge while the prompt is open; Keep makes the badge appear ("Transaction · 1 statement · <1 min").
- **Own-transaction form** (Auto-commit, idle; Hacivat's `BEGIN`, `TxOwner::Confirm`): title "214 rows changed on orders-prod" (the count from the driver, format locale); subtitle as surface 1; the SQL inset; body "Hacivat ran this statement in a transaction of its own. Other sessions don't see the change until you commit." and, in `text-2`, "Its rows stay locked while this waits. With no answer, Hacivat rolls back after 5 minutes." Footer: the **countdown** on the left; **Commit** (Default, check icon) and **Roll back** (Primary, undo icon, initial focus) on the right.
- **Countdown:** the 12 px timer glyph and "Rolls back in 4:32": the words in caption 12 `text-2`, the time in Fragment Mono 11.5 `text-2`, `m:ss` in ASCII digits in every locale; repainted once a second on the executor clock; no colour or weight change near the end. `role=timer`, `aria-live=off`; its accessible value is derived from the same tick (never a stale opening value).
- **Connection lost** (I2): title "The connection to orders-prod was lost", the unplug glyph, **Close** (Primary, focused), no countdown. Own form: "The server rolled back these changes. Nothing was committed." Savepoint form, which loses the user's whole transaction: "The server rolled back your open transaction: this statement and the 3 before it. Nothing was committed." Output says the same with the count (`exec-confirm-dropped-sp`).
- **Around it:** the statement's gutter keeps the amber stamp with "waiting for Commit or Roll back" (savepoint: "waiting for Keep or Undo"); Run is disabled while the prompt is open; the badge (surface 4) shows only a user transaction.
- **Afterwards:** Commit → "1 row committed" (git-commit glyph); Keep → the badge counts the statement; Roll back / Undo / deadline → the gutter's undo glyph (`text-2`) with "rolled back", and Output "Statement rolled back; nothing was changed", "Statement undone; your transaction is still open", or "No answer within 5 min: the statement was rolled back."

**States.** Manual · first change (savepoint) · Auto · own transaction · 214 rows (Auto) · Savepoint inside a user transaction · Connection lost (own) · Connection lost · savepoint · Deadline reached (Output) · After Keep (the badge counts it) · 960 × 600 in Turkish (Manual).

**Keyboard and roles.** `alertdialog`; Enter acts on the focused button (Undo / Roll back at first); Escape answers Undo / Roll back; quitting answers the same. Neither answer is Danger: the destructive question, if any, was asked before the run (surface 1).

### 3 · Transaction chip and menu (Task 22)
**Intent.** The transaction mode is visible and changeable where you run.

**Anatomy.**
- **Chip** (editor toolbar, right after the connection label): the qchip anatomy (22 px, 1 px `seam-strong`, radius 4, 12.5): the commit glyph (12 px `text-2`), the mode ("Auto-commit", "Manual", "Smart"; Turkish "Otomatik commit", "Manuel", "Akıllı") and a chevron-down (`text-2`). `aria-haspopup=menu`, `aria-expanded`. Production consoles start in Manual (D-036).
- **Menu** (PopupMenu, raised, radius 6, popover shadow, rows 24 px, sized to its content): when a transaction is open, "Commit transaction" (the git-commit glyph, as the chip: a check would read as the current-mode mark, M1) and "Roll back transaction" (undo) first, icons `text-2`, a separator; then the header "Transaction mode" (12 `text-2`); three `menuitemradio` rows with the blue check on the current mode and a one-line hint in 12 `text-2` ("each statement commits as it runs", "changes wait for Commit", "Manual from the first change"); a separator; **Isolation ›** with its current value as the hint ("Server default · READ COMMITTED"); on production a note "Production consoles start in Manual."
- **Isolation submenu** beside the menu (flips left at the window edge): "Server default (READ COMMITTED)" with the blue check, then `READ UNCOMMITTED`, `READ COMMITTED`, `REPEATABLE READ`, `SERIALIZABLE` in Fragment Mono 12, untranslated (they are SQL).
- **While a transaction is open** the chip stays enabled and the menu opens; the three modes and Isolation are disabled (`text-3`, `aria-disabled`) with the reason "Commit or roll back first" as their hint.
- **Read-only session:** the chip reads Auto-commit, disabled (`aria-disabled`, no chevron, `text-3`), still focusable; its tooltip is the reason: "A read-only session runs each statement on its own, so its mode is always Auto-commit."

**States.** Auto-commit chip · Mode menu · Isolation submenu · Transaction open (disabled with reasons) · Read-only (disabled chip with its reason) · Production (Manual, the note) · Smart.

**Keyboard.** The chip is a Tab stop; Space / Enter / ↓ open the menu; ↑/↓ move (disabled rows are skipped); → opens Isolation, ← closes it; Enter picks; Escape closes and returns the focus to the chip.

### 4 · Transaction badge (Task 22)
**Intent.** An open user transaction is a trust signal: a badge where you run, with its size, age and both ways out.

**Anatomy.**
- In the editor toolbar, right-aligned after a 1 px seam: the 7 px amber stamp (pending), "Transaction · 3 statements · 2 min" (12.5 `text`; the count and age in Fragment Mono 11.5; the age repainted once a minute, "<1 min" first), then **Commit** (check) and **Roll back** (undo), Default buttons at 22 px with ink icons. `role=status`; the tooltip "Not committed yet: other sessions don't see these changes, and the rows they touch stay locked."
- **Failed** (PG aborted; R-M3-16): the ink alert glyph, "Transaction failed", **Roll back** only. The word is M3a's ("Transaction failed · roll back to end it" in the status bar and the close prompt), not the plan's "aborted" (ruling 8).
- Shown only for `TxOwner::User`, never for Hacivat's own transactions (cursor, guard, confirm). A shared session shows it in every console of the connection (surface 12).
- **Status bar:** M3a's segment ("Transaction open · 3 statements" with the amber stamp; "Transaction failed · roll back to end it" with the ink glyph) is now a button: it opens the surface 3 menu anchored to the segment's top edge (4 px above it, left edges aligned; M2); its tooltip says so.
- **Overflow Rule** (toolbar, first first): the database crumb; the badge's age ("· 2 min"); then the shared-session chip and the badge's two buttons become icon-only (the word stays as the tooltip and the accessible name). The stamp, "Transaction · n statements", the mode chip, Run/Stop and the connection name never hide or truncate.

**States.** Open · Failed (roll back only) · Status segment menu · Production console · 960 × 600 in Turkish (overflow).

**Keyboard.** The badge buttons are Tab stops in the toolbar; the status segment is a button (Enter / Space); the palette keeps "Commit transaction" / "Roll back transaction".

### 5 · Idle-transaction toast (Task 22)
**Intent.** A transaction left idle is named and never ended silently (D-014 A).

**Anatomy.** The M2a sticky toast, wide (380 px), no TTL, `role=alert`: the clock glyph (`text-2`), title 600 "Transaction open for 10 min on orders-prod", line `text-2` "It may hold locks that block other sessions.", then **Commit**, **Roll back**, **Keep open** (all Default). One per session: a newer reminder for the same session replaces the older one. Keep open only dismisses the toast; the badge stays and nothing is sent.

**States.** Idle 10 min · Newer replaces older · 960 × 600 in Turkish.

**Keyboard.** The toast never takes the focus unless asked (F6); Escape dismisses it (= Keep open).

### 6 · Cancel escalation (Task 23)
**Intent.** A cancel that doesn't land escalates in steps the user controls. Nothing escalates by itself, Escape never escalates, and every consequence is stated before the action that causes it (I3).

**Anatomy.**
- **Slow-cancel toast** at 10 s (HV-EXEC-09 step 4): the wide sticky toast with the hourglass glyph, "Cancelling is taking long" / "The server hasn't stopped the statement on orders-dev after 11 s.", and one Default button **Show options…**, which opens the dialog below with the focus on Keep waiting. The toast never terminates or disconnects in one click: the consequence text has one source, the dialog.
- **Second Stop press** (⌘. while Cancelling) or Show options… → **420 alertdialog**: title "The statement isn't answering the cancel"; body (`text`) "It has been cancelling for 14 s. Hacivat keeps waiting unless you choose otherwise."; then one consequence line per action, in `text-2`:
  - Terminate: "Terminate on server ends this session on the server with `pg_terminate_backend(18342)`." plus, when this console holds a user transaction, "This console's open transaction (3 statements) rolls back."
  - Disconnect, worded from R-M3-73: from the profile's **default console** "Disconnect closes every connection of orders-dev here; the statement may continue on the server." plus this console's open transaction and one sentence per other console of the profile that holds one: "orders-dev (2) also has an open transaction (3 statements); the server rolls it back."; from **any other console** "Disconnect closes only this console's session (orders-dev (2)); the statement may continue on the server." plus its own transaction only.
- **Footer** (M8): **Disconnect** (Danger, unplug glyph) alone on the far left; **Terminate on server** (Default, octagon-x) and **Keep waiting** (Primary, initial focus) on the right, the M2b damaged-vault arrangement.
- **No terminate** (SQLite, or a role that may not end other sessions; M7): the Terminate button and its line are omitted; Disconnect and Keep waiting remain. On SQLite (a local file, nothing runs "on the server") the Disconnect line reads "Disconnect closes this console's connection to the file app.db; SQLite stops the statement when its connection closes." A SQLite console's status bar has no TLS segment and its explorer row no latency.
- **Failed terminate:** the dialog stays; the ink alert line "Couldn't terminate the session. Hacivat is still waiting for the cancel." and the server-message box (caption "Server message · SQLSTATE 42501", the text verbatim in Fragment Mono 11.5/17). The focus returns to Keep waiting.
- **Gutter badge:** after Keep waiting the statement's gutter glyph is the 12 px hourglass in amber with "cancel not answered · 14 s" (amber, once a second). The toolbar keeps "Cancelling… 14 s" and Stop stays enabled (ruling 29).
- **Afterwards:** Terminate → "Cancelled after 23 s 108 ms" and "Session terminated on the server; its open transaction (3 statements) was rolled back." (octagon-x); Disconnect → "Disconnected orders-dev. The statement may still be running on the server." (unplug) and, from the default console, the capsule says No connection.
- **Vocabulary (M11):** the button is **Disconnect** / "Bağlantıyı kes", the app's existing verb (`hv-conn-menu-disconnect`); the engine call stays `Escalation::Deactivate` (HV-EXEC-09). The word "Deactivate" appears nowhere in the UI.

**States.** Slow cancel (10 s) · Second press · Terminate failed · Kept waiting (gutter) · No terminate (SQLite) · Terminated · Disconnected · 960 × 600 in Turkish.

**Keyboard.** Initial focus Keep waiting; Enter acts on the focused button; Escape answers Keep waiting.

### 7 · Error-policy bar (Task 23)
**Intent.** When one statement of a multi-statement run fails (policy Ask, R-M3-61), the run pauses and asks inline, between the editor and the results; never a modal.

**Anatomy.** A 32 px bar at the foot of the editor tile on raised, 1 px seam above, `role=alert`: the ink triangle-alert glyph; "Statement 2 of 5 failed" (12.5/600 `text`); the SQLSTATE chip (mono 11, 1 px `seam-strong`, radius 3); "3 more wait" (`text-2`); then **Retry**, **Ignore**, **Ignore all**, **Stop** (Default, 22 px). Inside a failed PostgreSQL transaction (P-M3b-19): "Statement 3 of 6 failed, and the transaction failed with it"; **Roll back and continue**, **Ignore all**, **Stop**; no Retry. The Roll back and continue tooltip (I4) counts the whole open transaction, which may have started before this run, and says how the rest runs from the live mode: "Rolls back your open transaction (2 statements), then continues with statement 4 in a new transaction." (Manual / Smart) or "…, then continues with statement 4, committing each statement as it runs." (Auto with a user `BEGIN`; P-M3b-19 keeps the mode). Turkish puts no suffix on a numeral ("4. ifadeden devam eder"). Run is disabled while the bar waits; the gutter shows the failed glyph on the unit and queued squares on the rest; Output carries the error with the server's text.

**Narrow** (the strip's rule): first "n more wait" hides, then the sentence wraps to a second line and the bar grows (padding 5); buttons never truncate.

**States.** Statement 2 of 5 failed · PG transaction failed · Keyboard (F6) · After Ignore all (summary) · 960 × 600 in Turkish (wraps).

**Keyboard.** The bar doesn't take the focus (it is announced). **F6 cycle (M6):** the first F6 moves the focus to the bar's first button, the next F6 to the toasts (the slow-cancel or idle toast may be up at the same time), the next back to the editor; without a bar F6 goes to the toasts as in M3a. Tab order Retry → Ignore → Ignore all → Stop (or Roll back and continue → Ignore all → Stop). Escape inside the bar answers Stop. Closing the console answers Stop.

### 8 · Parameter prompt (Task 24)
**Intent.** Bind values are asked for once per name, before classification, and treated as data (R-M3-79).

**Anatomy** (480 px dialog, not alert: it asks for data, not consent).
- Header "Values for this statement", subtitle with the connection and "2 parameters".
- The SQL inset (at most 3 lines here).
- A header row (12 `text-2`): Name · Value · Type. One row per name: the name in Fragment Mono 11.5 (`:musteri_id`, 104 px, ellipsized with the whole in the tooltip: it is data), a mono value field (flexible), the type hint in Fragment Mono 11 `text-2` (80 px; "unknown" when no hint), a **NULL** toggle (22 px, small caps; pressed = `blue-tint-strong` with a blue border: a selection, not a hue). With NULL on, the field shows `NULL` in small caps `text-3` and is disabled.
- **Remembered** values (from an earlier run in this app session) carry a 12 px history glyph and "remembered" (`text-2`) under the field; the tooltip says it is never saved to disk.
- **Ambiguity flag** (a Turkish-grouped number such as `1.234`): the field border turns `text-2`; under it the ink circle-alert glyph and "Read as the decimal 1.234: a dot is the decimal point here. If you meant 1234, write it without the dot." and a **Use 1.234** button. **Run stays disabled** until the value is edited or confirmed (fail-closed).
- **Template variable** (`${tablo}`): an inline notice with the ink glyph "${tablo} is a template variable, not a parameter. Hacivat doesn't fill it in, so this statement can't run as written." Run stays disabled.
- **PG `?`:** an info notice "On PostgreSQL ? isn't a parameter here, because jsonb operators use it. Settings › Parameters can turn it on."
- A 12 px `text-2` line: "Values are kept for this app session only and are masked in history."
- Footer right: **Cancel** (Default) and **Run** (Primary, play icon; disabled with the tooltip "Resolve the flagged value first").

**States.** Two names · Remembered · Turkish 1.234 · NULL · Template variable · PG ? off · 960 × 600 in Turkish.

**Keyboard.** Initial focus on the first empty value (else the first value). Tab moves value → NULL → next row. Enter in a value field runs when Run is enabled; Escape and Cancel send nothing.

### 9 · Chooser additions (Task 24)
**Intent.** The explicit way out of an uncertain boundary, and the per-console policy (CHOOSE-06).

**Anatomy.**
- **Boundary uncertain** (M3a's popup, unchanged text): after a separator, a **Run as one** row (the kind column) with "2 statements", then a wrapping 12 px `text-2` line "Run as one sends the whole region in one request; its results come back untyped." The row is never preselected: Enter does nothing until ↓ highlights it. The footer gains ↑↓ Navigate and ↵ Run. **On a read-only session** (the gate refuses Run as one, R-M3-60; M7) the row stays visible but disabled (`text-3`, skipped by the arrows) and its line gives the reason: "Not on a read-only session: its guard checks one statement at a time."
- **Customize…** is the last row of the normal chooser, after a separator. It opens the **policy popover** (the explanation popover anatomy, 300 px, settings glyph, title "How Run picks statements in this console"): "Caret inside a statement" (Ask / Smallest statement / Largest statement / Whole script) and "With a selection" (As separate statements / As one), each a compact choice list (rows 26 px, radio, `blue-tint` when chosen); a note "Kept for this console until it closes."
- **Selection: As one** — the preview band covers the selection as one unit with one gutter bar, and the end of its first line says "as one · 2 statements" with the layers glyph (instead of M3a's "1 of 3", "2 of 3").

**States.** Uncertain → Run as one · Run as one highlighted · Run as one · read-only (disabled with its reason) · Customize… · Policy popover · Selection as one · 960 × 600 in Turkish.

**Keyboard.** As M3a's chooser (no access keys). The popover: Tab between the two groups, arrows within a group, Escape closes and returns the focus to the editor.

### 10 · Results additions (Task 25)
**Intent.** Several result sets per run, each in its own tab, and what a result costs said in place.

**Anatomy.**
- **Tab titles:** a result tab is titled by its statement's leading comment ("Şehre göre ciro"; data: ellipsized at 190 px with the whole in the tooltip), else M3a's "Result n". A repeated title gets " (2)". A **pinned** tab shows the 12 px pin glyph (`text-2`; tooltip "Pinned: the next run opens a new tab") and is never reused.
- **Tab menu** (right-click or the context-menu key on a tab; PopupMenu with an icon column): Pin tab / Unpin tab, separator, Close tab, Close other tabs.
- **Run as one:** "untyped (run as one)" in `text-2` after the title (the "(staging)" anatomy), with a tooltip; every column's type subtitle is `text`.
- **Result-set limit** (R-M3-66): a wrapping info row under the toolbar (layers glyph): "The first 100 result sets are shown; 37 more were read and not kept."
- **Σ Count rows** (R-M3-76) in the result toolbar after Copy (ghost, sigma glyph; tooltip "Runs SELECT COUNT(*) over this query on a separate session."). Beside it: the planner's estimate "≈ 1,048,576 rows (estimate)" in mono `text-2`; while counting the amber run marker, "Counting…" and a ghost **Cancel** (the static marker replaces Task 25's "cancellable spinner": M3a ruling 5, no ambient motion; ruling 21); then "1,048,213 rows (exact)" in mono `text`. The button is disabled while counting.
- **Closed to free memory** (R-M3-68): the strip's closed anatomy (info glyph `text-2`): "Closed to free memory; the first 4,096 rows are kept." and Re-run.
- **Fetch-all question:** Fetch all past the per-result cap opens a 480 dialog "Fetching every row would pass this result's memory limit" / "The query has more rows than fit in 256 MB. Hacivat can fetch up to the limit and keep the cursor open for the rest."; **Cancel** (Default) and **Fetch up to the memory limit** (Primary, focused: it is the bounded, safe choice). Not an alertdialog.
- **Notices and warnings** in Output, one row each: a glyph (info `text-2` for a notice; triangle-alert ink and a 600 label for a warning), the label "Notice" / "Warning", the server's severity as a chip only when it differs from the label's (M4: `WARNING` under Warning and `NOTICE` under Notice are dropped; PG `INFO`, `DEBUG` or `WARNING` raised as a notice keep theirs), the code as a chip (PG SQLSTATE; MySQL vendor code beside SQLSTATE, "1292 · 22007"), then the server's text verbatim in Fragment Mono 11.5 `text-2` (ellipsized, whole in the tooltip). Flood: "97 more notices not shown" (`text-2`, ellipsis glyph).

**States.** Titled tabs, pin · Tab menu · Untyped (run as one) · Result-set limit · Count: estimate · counting · exact · Closed to free memory · Fetch-all question · PG notices · MySQL warnings · 960 × 600 in Turkish.

### 11 · History dialog (Task 27)
**Intent.** Every statement you ran is findable with its outcome, and comes back into the editor in one key (HV-EXEC-12, D-061).

**Anatomy** (640 px dialog).
- Header "Query history", subtitle "5 entries".
- **Bar:** the search field (search glyph, placeholder "Search history"; tooltip "Search ignores letter case and accents: musteri finds müşteri."; debounced, off the UI thread), a Segmented "This connection / All connections", an outcome Select ("Any outcome", Succeeded, Failed, Cancelled).
- **List** (a field well: panel / ground, 1 px `seam-strong`, radius 4, at most 250 px, scrolls): group headers "Today", "Yesterday" (12 `text-2`); 24 px rows on a grid: time (Fragment Mono 11 `text-2`, locale medium time, ISO in the tooltip), the env mark, the connection (mono 11.5, 96 px), the outcome glyph (green check, ink triangle-alert, ban `text-2`), the statement's first line (mono 11.5 `text-2`, ellipsized: data), duration (mono 11, 64 px, right), rows (mono 11, 76 px, right), the pin glyph when pinned. The selected row is `blue-tint-strong` with its SQL in `text`.
- **Preview** (under a seam): a meta line (env mark, connection, ISO time, outcome glyph and word, duration, rows, "Ran 4 times", the "redacted" chip when a secret was removed) with Pin and Open in new console as ghost icon buttons; the masked SQL in the SQL inset (wrapping); the redacted line (eye-off glyph `text-2`): "A password or secret was removed before this was saved."; for a failure the server-message box.
- **Target line (M12):** when the selected entry's connection is not the focused console's, a 12.5 `text-2` line under the preview names where Insert and Insert and run go: "→ Goes into ◉ orders-dev, the focused console; Run checks it there." (env mark, connection mono). Insert and run still goes through that console's gate.
- **Footer:** key hints on the left (↵ Insert · ⌘↵ Insert and run · esc Close, 11.5 `text-2`); **Copy SQL**, **Insert and run** (Default) and **Insert** (Primary) on the right; the same wrapping rule as surface 1.
- **Empty:** "No history yet. Statements you run are kept here for 90 days." **No match:** "Nothing matches “faturalar”." (centred, `text-3` on the well).

**States.** List + preview · Turkish search · All connections, failed · Redacted entry · Other connection (target line) · Empty · No match · 960 × 600 in Turkish.

**Keyboard.** The search field has the focus; ↑/↓ move the selection while the focus stays in the field; ↵ inserts at the caret (one undo group; deferred while a composition is active); ⌘↵ / Ctrl+↵ inserts and runs through the same gate; Escape closes.

### 12 · Single session (Task 22)
**Intent.** A connection can make its consoles share one session; the console says so where you run.

**Anatomy.**
- **Connection dialog › Advanced** (the M2a 640 form dialog, unchanged otherwise): a new row "Sessions" with the checkbox "Use one session for every console" and the sub-label "Consoles of this connection share one session: its transaction, variables and temporary tables." When checked, the Connections row reads "One session, shared by every console" instead of "Up to 4 open at once; more consoles wait their turn" (D-035).
- **Console toolbar:** a "Shared session" chip (users glyph `text-2`, qchip anatomy, not a button) after the mode chip; its tooltip: "Every console of orders-dev uses this session: a transaction opened here is open in all of them." Narrow: icon-only (the words stay in the tooltip and the accessible name).
- A transaction opened in one shared console shows its badge, mode and status segment in all of them.

**States.** Advanced · checkbox · Shared session chip · Shared, with a transaction · 960 × 600 in Turkish.

### 13 · Production console after the unlock (Task 26)
**Intent.** A production console is writable behind the gate; it stays visually unmistakable.

**Anatomy.** The M3a notice "Production is read-only in this version" (and its popover strings) is gone. The 1 px red hairline, the tab swatch and the capsule's PRODUCTION / PROD block are unchanged; the capsule's access segment and the status bar say read-write. **Status segment (RC-6):** it keeps the `red-tint` fill, the 7 px swatch and the red 600 text, and shows the red lock **only when access is read-only** (DESIGN.md:492: the lock means read-only production). This amends DESIGN.md:620 ("production: red-tint fill, red 600 text with swatch and lock") for M3b; Task 26 lists the M3a shots whose production segment loses the lock (R-M3-81). The mode chip shows Manual (D-036). A write goes through surfaces 1 and 2. A profile retagged to production gets confirmations from the next Run (R-M3-78), with no visual change beyond the marks.

**States.** After the unlock · M3a (for comparison, annotated) · Status and capsule · 960 × 600 in Turkish.

---

## New primitives

All use existing tokens only and are reproducible in GPUI.

| Primitive | Anatomy and exact tokens |
|---|---|
| **SQL inset** | panel (dark) / ground (light), 1 px `seam-strong`, radius 4, padding 7/10, Fragment Mono 11.5/17 with the syntax tokens, max 6 lines then scrolls (4 px `seam-strong` thumb, radius 3, no track); text-3 restored (field well). |
| **Reasons list** | 12.5 `text`, 3 px square `text-2` bullets 7 px before the text, 3 px between items; identifiers mono 11.5 `text`, bidi-isolated. |
| **Effect row** | "Effect" 12.5 `text-2`, 8 px gap, the class in 12.5 `text`. |
| **Type-to-confirm field** | label 12.5 `text` with the name mono; a full-width 26 px mono field; mismatch: border `text-2` + ink circle-alert line 12 `text-2`. |
| **Grouped footer** | two flex groups, 8 px inside, 28 px between, `flex-wrap`; the right group (`margin-left:auto`) wraps under the left, right-aligned. |
| **Countdown** | 12 px timer glyph `text-2`; words caption 12 `text-2`; the `m:ss` value Fragment Mono 11.5 `text-2`; repaint once a second; no hue. |
| **Mode chip** | qchip (22 px, `seam-strong`, radius 4, 12.5 `text`), commit glyph and chevron-down `text-2`; disabled: `text-3`, no chevron. |
| **Transaction badge** | 7 px amber stamp, label 12.5 `text` with mono 11.5 numbers, 22 px Default buttons with ink icons; failed: ink triangle-alert. |
| **Error bar** | 32 px raised, seam above, ink glyph, 12.5/600 title, SQLSTATE chip, `text-2` waiting count, 22 px Default buttons; wraps to two lines (padding 5, line 16). |
| **Parameter row** | grid 104 / 1fr / 80 / 48, 8 px gaps; name mono 11.5, value mono field, type mono 11 `text-2`, NULL toggle 22 px small caps (pressed `blue-tint-strong` + blue border). |
| **Flag line** | grid 12 px glyph + text; ink circle-alert; 12 `text`; a 20 px Default button under the text. |
| **History row** | grid 58 / 7 / 96 / 12 / 1fr / 64 / 76 / 12, 24 px, 8 px gaps; selected `blue-tint-strong`. |
| **Server-message box** | the gutter tooltip's box inside a dialog: panel / ground, `seam-strong`, radius 4, padding 6/8, caption 12 `text-2`, text mono 11.5/17. |
| **Log notice row** | glyph, label, severity chip and code chip (mono 11, `seam-strong`, radius 3), server text mono 11.5 `text-2`. |
| **Gutter marks (M3b)** | awaiting an answer: the 7 px amber stamp; cancel not answered: hourglass 12 px amber; rolled back / undone: undo-2 12 px `text-2`. |

**Icons (RC-4).** The mock adds seven Lucide 0.460 icons to the sprite (ISC): `history`, `timer`, `hourglass`, `users`, `eye-off` (already in the M2b sprite), `octagon-x`, `clock`. All seven are missing from the app's set (`crates/ui/hacivat-ui-core/assets/icons/lucide/`) and come from **one Task 1b-owned addendum** (SVGs, `assets.rs` rows, `IconName` variants, `new_icons_are_embedded`), routed by the controller; no task in 21–27 edits the icon table. Existing glyphs substitute elsewhere: the outcome filter uses `filter` (not `list-filter`), Unpin uses `pin` (the label carries the meaning, not `pin-off`), Disconnect uses `unplug` (not `power`). `pin` itself comes with Task 1b (P-M3b-37).

---

## Open questions and rulings

Each was answered by the ruling noted, so the build is not blocked; the PO can overrule any.

1. **"Production single DML" in surface 1.** A plain production `Dml` has no pre-run dialog: R-M3-55 makes it `RunInTxThenConfirm` (surface 2). *Ruling:* surface 1's single-statement scenario is a one-statement DML that does reach `Confirm` without a typed name on production: an `UPDATE` whose `SET` calls a function Hacivat can't classify (GateClass `Unknown`). *Cost if wrong:* a fixture in Task 21's shots.
2. **Which buttons are Danger in the gate dialog.** *Ruling:* every button that sends the statement (Execute, Execute all, Execute and don't ask again) is Danger, because the dialog only appears for destructive or unknown classes; **Warn (`FOR UPDATE`) uses a Default Execute**, because locking rows changes no data (Hue Is Meaning: red for destructive actions only). Accepted as RC-2: Warn's Execute and "don't ask again" are Default and never focused; Task 21's test is `cancel_has_initial_focus_and_execute_is_danger_except_lock`. *Cost if wrong:* one variant flag.
3. **The six-button row in 480 px.** *Ruling:* two groups (run left, stop right) 28 px apart; when they don't fit one line the right group wraps under the left, still right-aligned (the M2a footer rule). Multi-unit footers always take two lines at 480 px; single-unit ones fit one line in both languages. *Cost if wrong:* layout constants in Task 21.
4. **"Don't ask again": label and scope.** *Ruling:* off production "Execute and don't ask again"; on production the label names the scope, "Execute; don't ask again in this console"; in both, a one-line body note states the scope (this statement shape, this console, until it closes). Not offered with type-to-confirm (R-M3-55) nor for a MySQL implicit commit (its consequence depends on the open transaction, not the statement's shape). *Cost if wrong:* copy.
5. **Type-to-confirm comparison.** *Ruling:* exact bytes; no case folding, no Turkish İ/i folding, no trimming. The hint appears after a mismatch only. Enter in the field never executes (M2b vault reset precedent). *Cost if wrong:* none (it fails closed).
6. **Commit prompt: which answer is primary.** *Ruling:* the safe answer (Roll back; Undo) is Primary with the focus; Commit and Keep are Default outline buttons, never Danger (the destructive question was asked before the run). *Cost if wrong:* button variants.
7. **The savepoint form's Undo in Turkish.** "Geri al" is already Roll back. *Ruling:* "İfadeyi geri al" (undo the statement) so the two never read alike. *Cost if wrong:* copy.
8. **"Aborted" or "failed".** The plan says "Transaction aborted — roll back"; M3a shipped "Transaction failed · roll back to end it". *Ruling:* "failed" everywhere (one word for one state): the badge reads "Transaction failed", the error bar "…and the transaction failed with it". *Cost if wrong:* copy.
9. **Where the chip and the badge sit.** *Ruling:* the chip right after the connection label (it describes the session, like the label); the badge right-aligned after a seam (it is state with actions); the read-only lock chip after it. *Cost if wrong:* toolbar order in Task 22.
10. **A chip that is "disabled with a reason" while a transaction is open.** *Ruling:* the chip stays enabled so the reason is visible; its modes and Isolation are disabled in the menu with "Commit or roll back first" as their hint, and Commit / Roll back lead the menu. Only a read-only session disables the chip itself (focusable, reason in the tooltip). **Task 22 note:** this changes Task 22's Produces text ("the chip … disabled with the reason … while a tx is open"): the chip stays enabled, and `chip_disabled_with_reason_while_tx_open` asserts that the menu's mode and Isolation items are disabled with that reason. *Cost if wrong:* one test's target.
11. **Isolation names.** *Ruling:* SQL keywords in Fragment Mono, untranslated (Mono Means Copyable); only "Server default" is translated. *Cost if wrong:* five strings.
12. **The badge's age cadence.** *Ruling:* once a minute ("<1 min", "2 min"); never seconds (no ambient motion). *Cost if wrong:* one timer.
13. **Idle toast "Keep".** *Ruling:* "Keep open" ("Açık bırak"), so the label says nothing is ended. *Cost if wrong:* copy.
14. **Escalation dialog variants.** *Ruling:* Keep waiting Primary + focus; Terminate on server Default beside it; Disconnect Danger alone on the far left; each with its consequence line before it. The slow-cancel toast carries only Show options… (superseded by ruling 35). *Cost if wrong:* variants.
15. **The escalation gutter badge.** *Ruling:* the 12 px hourglass in amber (pending, not failed) with "cancel not answered · 14 s" once a second; not a new glyph shape for "escalated". *Cost if wrong:* one glyph in Task 34's painter (Task 23 sets the state).
16. **Where the error bar lives and who has the focus.** *Ruling:* at the foot of the editor tile (above the seam to the results), so it reads as part of the run; it never takes the focus by itself (typing must not answer it); F6 enters it. *Cost if wrong:* placement in Task 23.
17. **An ambiguous parameter value.** *Ruling:* fail-closed: Run stays disabled until the value is edited or "Use 1.234" confirms the machine reading. *Cost if wrong:* one click for Turkish users who meant the decimal.
18. **The NULL toggle.** *Ruling:* a small-caps "NULL" toggle per row (pressed = selection tint); the field shows `NULL` disabled. A checkbox would need a label longer than the column. *Cost if wrong:* a widget swap.
19. **Run as one in the uncertain chooser.** *Ruling:* a row after the explanation, never preselected, with its consequence on its own wrapping line (not ellipsized); R-M3-24 still holds: nothing runs by Enter alone. *Cost if wrong:* menu layout.
20. **"untyped (run as one)" placement.** *Ruling:* a tab suffix in `text-2` (the "(staging)" anatomy), so it follows the tab wherever the grid scrolls. *Cost if wrong:* one label.
21. **Count rows estimate.** *Ruling:* show the planner's row estimate with "≈" in `text-2` when Task 16's count service supplies one, and only the button otherwise; counting uses the static run marker, not a spinner (M3a ruling 5). **Task 25 note:** this is a named deviation from Task 25's "cancellable spinner": the cancellable state is the static amber marker with Cancel. *Cost if wrong:* one optional label.
22. **The fetch-all question's default.** *Ruling:* "Fetch up to the memory limit" is Primary and focused: it is bounded and destroys nothing; the dialog is not an alertdialog. *Cost if wrong:* the focus target.
23. **Notice rows.** *Ruling:* one row per notice, label + severity chip + code chip + verbatim text; warnings use the ink glyph and a 600 label, never amber or red. *Cost if wrong:* row anatomy in Task 25.
24. **History layout at 640 px.** *Ruling:* the list above the preview (not side by side), so both keep full width at 960 × 600; time / connection / outcome / SQL / duration / rows / pin on one 24 px row. *Cost if wrong:* layout in Task 27.
25. **Where the single-session setting sits.** *Ruling:* its own "Sessions" row in Advanced, beside the Connections fact it changes; not under Read-only (it is not a trust decision). *Cost if wrong:* one row.
26. **Shared-session chip at narrow widths.** *Ruling:* icon-only before any trust signal collapses; it is information, not a trust signal. *Cost if wrong:* priority number.
27. **Production after the unlock: anything new?** *Ruling:* nothing but the Manual chip; production's existing marks already make it unmistakable, and adding a "writable" notice would be a second production surface to keep in sync. *Cost if wrong:* a toolbar item.
28. **Fluent ids.** *Ruling:* the ids in the copy table are binding for Tasks 21–27, except `safety-reason-*`, whose ids Task 4 defines (`Reason::fluent_id`); the copy here is the binding wording for them. Core log lines (`exec-*`) belong to the task that emits them (Task 16, 19, 23 or 24). *Cost if wrong:* renames.
29. **Stop while Cancelling.** M3a disabled Stop until the server answered. *Ruling:* in M3b Stop stays enabled while Cancelling (its tooltip names the dialog), because its second press is the way to the escalation dialog (HV-EXEC-09 step 5); the toolbar still shows "Cancelling… 14 s". **Task 23 note:** this changes the approved M3a shots of the cancelling toolbar (Stop no longer disabled); under R-M3-81 Task 23 lists each changed PNG and the controller records the PO item. *Cost if wrong:* the M3a `stopb` disabled state, one line in Task 23's named `cancel.rs` edit.
30. **A short result tab strip.** *Ruling:* first "untyped (run as one)" takes its short form "untyped" (the whole stays in the tooltip), then titles ellipsize at 120 px instead of 190 px (they are data). *Cost if wrong:* one fit step in Task 25.
31. **What the gutter shows while an answer is pending.** *Ruling:* the 7 px amber stamp (pending) with "waiting for Commit or Roll back" / "waiting for Keep or Undo"; after the answer, the green check with the row count (Commit, Keep) or the undo glyph with "rolled back" (Roll back, Undo, deadline). Run stays disabled until the answer. *Cost if wrong:* one `ExecMarkState` arm.

32. **Execute all (RC-1).** *Ruling:* disabled until the current unit's name is typed; answers only later units that would ask without a typed name; never pre-answers a confirm-in-transaction prompt; Danger unless every unit it answers is Warn; one scope line in the dialog. T21 tests `execute_all_disabled_until_exact_bytes`, `execute_all_never_answers_a_type_to_confirm_unit`. *Cost if wrong:* one extra dialog on scripts that mix a no-WHERE DML with other confirms.
33. **Three Danger buttons (RC-3).** *Ruling:* kept; the isolation inside a dialog footer is the 28 px gap with no seam (M2b precedent). *Cost if wrong:* button variants.
34. **Which confirm-in-transaction form (RC-5).** *Ruling:* `CommitQuestion.savepoint` decides: Manual and Smart always savepoint (Keep / Undo); own `BEGIN` (Commit / Roll back) only in Auto-commit with no transaction open. Every scenario drawn under a Manual chip shows the savepoint form; "Manual · first change" is the commonest production write. *Cost if wrong:* none; it is P-M3b-1.
35. **Escalation consequences (I3).** *Ruling:* every action's consequence is shown before it, naming each user transaction it rolls back (this console's; with Disconnect from the default console, every other console of the profile that holds one); the slow-cancel toast only opens the dialog. Disconnect's scope follows R-M3-73 (default console: the whole profile; any other: only its session); **Task 23 confirms** that `Escalation::Deactivate` behaves so, and the copy follows the code if it differs. *Cost if wrong:* two copy variants.
36. **Disconnect, not Deactivate (M11).** *Ruling:* the app's existing verb, EN "Disconnect", TR "Bağlantıyı kes"; the engine name stays. *Cost if wrong:* two strings.
37. **Connection lost in the savepoint form (I2).** *Ruling:* its own sentence counting the statements the server rolled back with the user's transaction. *Cost if wrong:* one string.
38. **Roll back and continue (I4).** *Ruling:* the tooltip counts the whole open transaction and states the continuation mode from live state; Turkish avoids suffixes on numerals. *Cost if wrong:* two strings.
39. **The production status segment's lock (RC-6).** *Ruling:* only on read-only production; amends DESIGN.md:620 for M3b; Task 26 lists the changed shots. *Cost if wrong:* one glyph, a re-baseline round.
40. **F6 with a bar and a toast (M6).** *Ruling:* one cycle: error bar → toasts → editor. *Cost if wrong:* the cycle order.
41. **Missing capabilities (M7).** *Ruling:* no Terminate → omit the button and its line; Run as one on a read-only session → the row stays, disabled, with its reason. *Cost if wrong:* two variants.
42. **History target (M12).** *Ruling:* when the entry's connection differs from the focused console's, a line names where Insert and Insert and run go; the gate of that console applies. *Cost if wrong:* one line.
43. **Type-to-confirm with an untypeable name (M14).** *Ruling:* keep exact bytes (fail-closed); the name is a selectable mono run with a Copy button so it can be pasted. *Cost if wrong:* none.
44. **The unsafe-WHERE guard off production vs M2a's line (I6).** *Ruling (default in force, PO question):* the guard stays on off production (Q1), which makes M2a's binding `prod-other` line "No extra confirmations. Statements commit as they run." untrue. Proposed amendment (a named exception in Task 24): EN "No extra confirmations, except before a statement that changes every row. Statements commit as they run." / TR "Ek onay yok; yalnızca tüm satırları değiştiren bir ifadeden önce sorulur. İfadeler çalıştıkça commit edilir." The alternative is Q1 off by default. *Cost if wrong:* one line in the connection dialog or one setting default.

### Open items for the PO

1. **Turkish copy:** every TR string in the table below is marked `Review: PO`; in particular "Manuel" (vs "Elle"), "Bağlantıyı kes" for Disconnect, "İfadeyi geri al" for the savepoint Undo, "Açık bırak" for Keep open, "türsüz (tek parça)", and the review's TR suggestions (applied).
2. **Q1 and M2a's "No extra confirmations" (ruling 44):** the guard is on off production by default; the PO either accepts the amended `prod-other` line (recommended) or turns Q1 off.
3. **Warn's Execute is Default and unfocused** (RC-2, ruling 2): accepted by the controller; listed for the PO.
4. **Three Danger buttons** in a multi-unit footer (RC-3, ruling 33): accepted; optional PO question whether production should offer "don't ask again" for `DdlDestructive` / `Admin` / `Unknown` at all (R-M3-55 allows it).
5. **"Transaction failed" instead of the plan's "aborted"** (ruling 8).
6. **Icons** (RC-4): seven new icons from one Task 1b addendum; three substitutions.
7. **Count rows estimate** (ruling 21): shown only if Task 16 supplies a planner estimate.
8. **Chip enabled while a transaction is open** (ruling 10): changes Task 22's Produces text and the meaning of `chip_disabled_with_reason_while_tx_open`.

---

## Verification

See the Task 2a report for the run log. In short:
- Headless Chrome over the DevTools protocol (the harness lives in the session scratchpad and is not committed) rendered every scenario of the thirteen surfaces (93 scenarios) × EN/TR × dark/light × 1280 × 800 and 960 × 600 with `bar=0&shot=1`.
- Each render was checked for: console errors and exceptions; a toolbar, strip, status bar, tab strip, capsule, dialog (head, body, footer), menu, popover, tooltip, toast, notice, info row, error bar, parameter row or history bar that overflows sideways; chrome text that ellipsizes or runs past a clipping ancestor (data exempt: SQL, cell text, server text, titles from comments, names, the connection crumb); a floating layer outside the window; the number of cp-fill blocks larger than a mark (exactly one when the focused console is production, else none); text below 11 px (small caps excepted); `text-3` on raised (dark, disabled exempt); Danger on anything but a destructive execute or Disconnect; Primary on a risky answer; an alertdialog that does not open on its safe answer; Execute all enabled while Execute waits for the typed name; `NaN`, `undefined` or an unfilled `{placeholder}` in any drawn text or tooltip; TLS or "on the server" wording on a SQLite console; a toast that terminates or disconnects in one click; a lock in a read-write production status segment; the own-BEGIN form under a Manual or Smart chip; the status-segment menu detached from its segment; a modal whose focus is outside it; every trust signal drawn, whole and inside its clipping ancestor; the production hairline; no production read-only notice after the unlock; M3b redefining an earlier Fluent id.
- Result on the committed file (fix round 1): **744 renders (93 scenarios × 8), 0 findings.** (The first commit: 696 renders, 87 scenarios, 0 findings; the design review then found what the checks did not cover, and the checks above were added for each.) The earlier passes found and fixed (in the design, never by relaxing a check): the transaction menu overflowing a fixed width; the parameter type column ellipsizing `timestamptz`; the Run-as-one hint ellipsized in the chooser (now its own wrapping line; the checker's exemption for it was removed); the untyped tab suffix clipping the result tab strip at 960 in Turkish (now a short form, then shorter titles).
- The PNGs were looked at (dark and light, EN at 1280 and TR at 960) for every surface. That review caught what the checker could not: footer groups indented by the explorer's `.grp` class (renamed `.fgrp`); notice rows in Output drawn as inline notices (class collision, renamed); the ambiguity flag wrapping its glyph onto its own line (now a 12 px + text grid) and its copy repeating the value ("for 1.234 write 1234" → "If you meant 1234, write it without the dot"); Stop disabled while Cancelling, which made the second press impossible (ruling 29); the MySQL DDL notice missing from the implicit-commit scenario; a history list whose duration and row columns did not align.
- Cadence, measured without `shot=1`: the D-014 countdown went 4:32 → 4:28 in 3.3 s with DOM mutations exactly 1,000 ± 1 ms apart (4 batches); the escalation clock advanced 15 s → 18 s in 3 batches; the badge age did not change and caused no mutation in 3.3 s (once a minute).
- Behaviour probes: in "Statement 2 of 3 · type the name" Execute and Execute all are both disabled and both enabled once `müşteriler` is typed; the slow-cancel toast offers only Show options…, which opens the dialog focused on Keep waiting; Run on the production surface (Manual) opens the savepoint form, and Keep makes the badge appear with 1 statement. The countdown's tooltip follows the tick (4:56 → 4:52, title 4:52).
- Keyboard spot checks: an alertdialog opens with the focus on its safe answer (checked in every render); Enter on the focused Cancel sends nothing and logs it; Enter in the type-to-confirm field does nothing; Escape answers the safe value.
- The copy table below is generated from the mock's own `i18n` block (213 ids), so the two cannot drift; `Review: PO` appears once per row (213) and in three prose lines (216 in all).

---

## Copy strings (EN / TR)

The id is the Fluent id. Placeholders are Fluent variables (`{ $rows }`); counts, durations and sizes arrive pre-formatted by the format locale (`exec-dur-*`, `exec-size-*`); a clock reading (`{ $time }`, `4:32`) uses ASCII digits in every locale. Every EN message with a count has a plural selector; where the singular differs, its `one` form follows in the EN cell. Turkish needs no plural after a numeral and never puts a case suffix on one (`{ $next }. ifadeden`). Identifiers inside a message (`{ $table }`, `{ $fn }`, `{ $name }`, `{ $console }`) are FSI/PDI-isolated by the caller.

| Prefix | File | Owner (own `## m3b-tNN-*` block) |
|---|---|---|
| `hv-confirm-*`, `hv-commit-*`, `hv-editor-mark-awaiting*`, `hv-editor-mark-rolled-back` | `locales/{en,tr}/hacivat-ui.ftl` | Task 21 |
| `safety-reason-*` | the file Task 4 reads (`Reason::fluent_id`) | Task 4 (the ids are Task 4's; the wording here is binding) |
| `exec-confirm-deadline*`, `-committed`, `-rolled-back`, `-kept`, `-undone`, `-dropped*` | `locales/{en,tr}/hacivat-core.ftl` | Task 19 |
| `exec-confirm-suppressed`, `-skipped`, `-cancelled` | `locales/{en,tr}/hacivat-core.ftl` | Task 24 |
| `hv-tx-chip-*`, `hv-tx-menu-*`, `hv-tx-mode-*`, `hv-tx-iso-*`, `hv-tx-badge*`, `hv-tx-segment-tip`, `hv-tx-idle-*`, `hv-conn-*`, `hv-console-shared*` | `locales/{en,tr}/hacivat-ui.ftl` | Task 22 |
| `exec-dur-lt-min` | `locales/{en,tr}/hacivat-core.ftl` | Task 22 |
| `hv-cancel-*`, `hv-errbar-*`, `hv-editor-mark-escalated` | `locales/{en,tr}/hacivat-ui.ftl` | Task 23 |
| `exec-terminated`, `exec-terminate-failed`, `exec-disconnected`, `exec-run-*` | `locales/{en,tr}/hacivat-core.ftl` | Task 23 |
| `hv-params-*`, `hv-chooser-*`, `hv-policy-*`, `hv-editor-preview-as-one` | `locales/{en,tr}/hacivat-ui.ftl` | Task 24 |
| `hv-results-*`, `hv-strip-evicted`, `hv-fetchall-*` | `locales/{en,tr}/hacivat-ui.ftl` | Task 25 |
| `exec-notice`, `exec-warning`, `exec-notices-dropped`, `exec-result-set-limit` | `locales/{en,tr}/hacivat-core.ftl` | Task 16 |
| `hv-history-*` | `locales/{en,tr}/hacivat-ui.ftl` | Task 27 |

Reused, not repeated here: every M3a id (`design/m3a-query/NOTES.md`), notably `hv-tx-open`, `hv-tx-failed`, `hv-tx-cmd-commit`, `hv-tx-cmd-rollback`, `hv-results-result-n`, `hv-results-copy`, `hv-strip-*`, `exec-dur-*`, `exec-error`, `exec-summary`, `exec-tx-committed`, `exec-tx-rolled-back`, `exec-cancelled`; the M2a connection-dialog strings (`dlg-*`, `sec-*`, `adv-*`, and `prod-other`, whose amendment is ruling 44); `err-server`, `hui-tab-close`, `hui-palette-hint-*`. Literal data is not translated: SQL, connection names, isolation levels (`READ COMMITTED`), server severities and codes, server text, `NULL`.

None of the Turkish below has been signed off yet; the Status column says so for each row.

| id | EN | TR | Status |
|---|---|---|---|
| `hv-confirm-title` | Run this statement on { $name }? | Bu ifade { $name } üzerinde çalıştırılsın mı? | Review: PO |
| `hv-confirm-title-warn` | This statement locks rows on { $name } | Bu ifade { $name } üzerinde satırları kilitler | Review: PO |
| `hv-confirm-sub-unit` | statement { $i } of { $n } | ifade { $i }/{ $n } | Review: PO |
| `hv-confirm-sql` | Statement | İfade | Review: PO |
| `hv-confirm-effect` | Effect | Etkisi | Review: PO |
| `hv-confirm-prod-line` | Nothing is sent until you choose. | Siz seçene kadar hiçbir şey gönderilmez. | Review: PO |
| `hv-confirm-calm-line` | Off production, Hacivat asks only before statements like this one. Settings › Execution can turn it off. | Üretim dışında Hacivat yalnızca bunun gibi ifadelerden önce sorar. Ayarlar › Çalıştırma bölümünden kapatılabilir. | Review: PO |
| `hv-confirm-class-lock` | Reads rows and locks them | Satırları okur ve kilitler | Review: PO |
| `hv-confirm-class-dml` | Changes data | Veriyi değiştirir | Review: PO |
| `hv-confirm-class-dml-unsafe` | Changes data with no usable filter | Veriyi kullanılabilir bir filtre olmadan değiştirir | Review: PO |
| `hv-confirm-class-ddl-destructive` | Removes schema objects or their data | Şema nesnelerini ya da verilerini kaldırır | Review: PO |
| `hv-confirm-class-ddl-other` | Changes the schema | Şemayı değiştirir | Review: PO |
| `hv-confirm-class-routine` | Runs a procedure | Bir yordam çalıştırır | Review: PO |
| `hv-confirm-class-unknown` | Unknown: Hacivat can't tell what it does | Bilinmiyor: Hacivat ne yaptığını anlayamıyor | Review: PO |
| `hv-confirm-mysql-ddl` | MySQL commits DDL at once, so this can't be rolled back. | MySQL DDL'i hemen commit eder; bu yüzden bu işlem geri alınamaz. | Review: PO |
| `hv-confirm-type-label` | Type { $name } to run it | Çalıştırmak için { $name } yazın | Review: PO |
| `hv-confirm-type-hint` | Type it exactly as shown, letter case included. | Büyük-küçük harf dahil, tam gösterildiği gibi yazın. | Review: PO |
| `hv-confirm-execute` | Execute | Çalıştır | Review: PO |
| `hv-confirm-execute-all` | Execute all | Tümünü çalıştır | Review: PO |
| `hv-confirm-execute-suppress` | Execute and don't ask again | Çalıştır, bir daha sorma | Review: PO |
| `hv-confirm-execute-suppress-prod` | Execute; don't ask again in this console | Çalıştır; bu konsolda bir daha sorma | Review: PO |
| `hv-confirm-suppress-scope` | Not asked again for statements of the same shape in { $console } until it closes. | Aynı biçimdeki ifadeler { $console } kapanana kadar yeniden sorulmaz. | Review: PO |
| `hv-confirm-skip` | Skip | Atla | Review: PO |
| `hv-confirm-skip-all` | Skip all | Tümünü atla | Review: PO |
| `hv-confirm-cancel` | Cancel | Vazgeç | Review: PO |
| `safety-reason-no-where` | No WHERE clause: every row of { $table } changes. | WHERE koşulu yok: { $table } tablosunun her satırı değişir. | Review: PO |
| `safety-reason-always-true` | The WHERE clause is always true: every row of { $table } changes. | WHERE koşulu her zaman doğru: { $table } tablosunun her satırı değişir. | Review: PO |
| `safety-reason-truncate` | Removes every row of { $table }. | { $table } tablosunun tüm satırlarını siler. | Review: PO |
| `safety-reason-drop-table` | Drops the table { $object } and its data. | { $object } tablosu verisiyle birlikte kaldırılır. | Review: PO |
| `safety-reason-drop-schema` | Drops the schema { $object } and everything in it. | { $object } şeması içindeki her şeyle birlikte kaldırılır. | Review: PO |
| `safety-reason-drop-cascade` | CASCADE also drops everything that depends on it. | CASCADE, ona bağlı her şeyi de kaldırır. | Review: PO |
| `safety-reason-drop-column` | Drops the column { $column } and the data in it. | { $column } sütunu, içindeki veriyle birlikte kaldırılır. | Review: PO |
| `safety-reason-unknown-function` | Calls { $fn }(), whose effects Hacivat can't determine. | Etkilerini Hacivat'ın belirleyemediği { $fn }() fonksiyonunu çağırır. | Review: PO |
| `safety-reason-routine` | Calls the procedure { $fn }; Hacivat can't see what it changes. | { $fn } yordamını çağırır; Hacivat neleri değiştirdiğini göremez. | Review: PO |
| `safety-reason-lock-row` | FOR UPDATE locks the rows it reads until the transaction ends. | FOR UPDATE, okuduğu satırları işlem bitene kadar kilitler. | Review: PO |
| `safety-reason-implicit-commit` | MySQL first commits your open transaction ({ $n } statements), then runs the DDL. · *one:* MySQL first commits your open transaction ({ $n } statement), then runs the DDL. | MySQL önce açık işleminizi ({ $n } ifade) commit eder, sonra DDL'i çalıştırır. | Review: PO |
| `hv-commit-title` | { $rows } rows changed on { $name } · *one:* { $rows } row changed on { $name } | { $name } üzerinde { $rows } satır değişti | Review: PO |
| `hv-commit-body-own` | Hacivat ran this statement in a transaction of its own. Other sessions don't see the change until you commit. | Hacivat bu ifadeyi kendi açtığı bir işlemde çalıştırdı. Commit edene kadar diğer oturumlar değişikliği görmez. | Review: PO |
| `hv-commit-locks` | Its rows stay locked while this waits. With no answer, Hacivat rolls back after 5 minutes. | Yanıt beklenirken satırlar kilitli kalır. Yanıt gelmezse Hacivat 5 dakika sonra geri alır. | Review: PO |
| `hv-commit-commit` | Commit | Commit et | Review: PO |
| `hv-commit-rollback` | Roll back | Geri al | Review: PO |
| `hv-commit-countdown` | Rolls back in { $time } | { $time } sonra geri alınır | Review: PO |
| `hv-commit-sp-title` | { $rows } rows changed inside your transaction · *one:* { $rows } row changed inside your transaction | İşleminizin içinde { $rows } satır değişti | Review: PO |
| `hv-commit-sp-keep-line` | Keep: the change joins your open transaction. Nothing is committed yet. | Koru: değişiklik açık işleminize katılır. Henüz hiçbir şey commit edilmez. | Review: PO |
| `hv-commit-sp-undo-line` | Undo: removes this statement's change; your transaction stays open. | İfadeyi geri al: yalnızca bu ifadenin değişikliği kaldırılır; işleminiz açık kalır. | Review: PO |
| `hv-commit-sp-locks` | With no answer, Hacivat undoes the statement after 5 minutes and leaves your transaction open. | Yanıt gelmezse Hacivat 5 dakika sonra ifadeyi geri alır ve işleminizi açık bırakır. | Review: PO |
| `hv-commit-keep` | Keep | Koru | Review: PO |
| `hv-commit-undo` | Undo | İfadeyi geri al | Review: PO |
| `hv-commit-sp-countdown` | Undoes this statement in { $time } | İfade { $time } sonra geri alınır | Review: PO |
| `hv-commit-dropped-title` | The connection to { $name } was lost | { $name } bağlantısı koptu | Review: PO |
| `hv-commit-dropped-body` | The server rolled back these changes. Nothing was committed. | Sunucu bu değişiklikleri geri aldı. Hiçbir şey commit edilmedi. | Review: PO |
| `hv-commit-close` | Close | Kapat | Review: PO |
| `hv-editor-mark-awaiting` | waiting for Commit or Roll back | Commit ya da geri alma bekleniyor | Review: PO |
| `hv-editor-mark-awaiting-sp` | waiting for Keep or Undo | Yanıt bekleniyor: Koru ya da İfadeyi geri al | Review: PO |
| `hv-editor-mark-escalated` | cancel not answered · { $duration } | iptal yanıtlanmadı · { $duration } | Review: PO |
| `hv-editor-mark-rolled-back` | rolled back | geri alındı | Review: PO |
| `exec-confirm-deadline` | No answer within { $duration }: the statement was rolled back. | { $duration } içinde yanıt gelmedi: ifade geri alındı. | Review: PO |
| `exec-confirm-deadline-sp` | No answer within { $duration }: the statement was undone; your transaction is still open. | { $duration } içinde yanıt gelmedi: ifade geri alındı; işleminiz hâlâ açık. | Review: PO |
| `exec-confirm-committed` | { $rows } rows committed · *one:* { $rows } row committed | { $rows } satır commit edildi | Review: PO |
| `exec-confirm-rolled-back` | Statement rolled back; nothing was changed | İfade geri alındı; hiçbir şey değişmedi | Review: PO |
| `exec-confirm-kept` | Statement kept in your transaction; nothing is committed yet | İfade işleminizde tutuldu; henüz hiçbir şey commit edilmedi | Review: PO |
| `exec-confirm-undone` | Statement undone; your transaction is still open | İfade geri alındı; işleminiz hâlâ açık | Review: PO |
| `exec-confirm-dropped` | Connection lost: the server rolled back the uncommitted changes. | Bağlantı koptu: sunucu commit edilmemiş değişiklikleri geri aldı. | Review: PO |
| `exec-confirm-suppressed` | Ran without asking: you chose not to be asked again for statements like this in { $console }. | Sorulmadan çalıştı: { $console } içinde bunun gibi ifadeler için bir daha sorulmamasını seçtiniz. | Review: PO |
| `exec-confirm-skipped` | Statement { $n } skipped: you chose Skip. | { $n }. ifade atlandı: Atla'yı seçtiniz. | Review: PO |
| `exec-confirm-cancelled` | Nothing was sent: you cancelled at statement { $n }. | Hiçbir şey gönderilmedi: { $n }. ifadede vazgeçtiniz. | Review: PO |
| `hv-tx-chip-auto` | Auto-commit | Otomatik commit | Review: PO |
| `hv-tx-chip-manual` | Manual | Manuel | Review: PO |
| `hv-tx-chip-smart` | Smart | Akıllı | Review: PO |
| `hv-tx-chip-tip` | Transaction mode: { $mode } | İşlem kipi: { $mode } | Review: PO |
| `hv-tx-menu-title` | Transaction mode | İşlem kipi | Review: PO |
| `hv-tx-mode-auto-hint` | each statement commits as it runs | her ifade çalışınca commit edilir | Review: PO |
| `hv-tx-mode-manual-hint` | changes wait for Commit | değişiklikler Commit'i bekler | Review: PO |
| `hv-tx-mode-smart-hint` | Manual from the first change | ilk değişiklikten itibaren Manuel | Review: PO |
| `hv-tx-menu-isolation` | Isolation | Yalıtım düzeyi | Review: PO |
| `hv-tx-iso-default` | Server default | Sunucu varsayılanı | Review: PO |
| `hv-tx-menu-reason-open` | Commit or roll back first | Önce commit edin ya da geri alın | Review: PO |
| `hv-tx-menu-prod-note` | Production consoles start in Manual. | Üretim konsolları Manuel kipte başlar. | Review: PO |
| `hv-tx-chip-ro-reason` | A read-only session runs each statement on its own, so its mode is always Auto-commit. | Salt okunur bir oturum her ifadeyi tek başına çalıştırır; bu yüzden kipi her zaman Otomatik commit'tir. | Review: PO |
| `hv-tx-badge` | Transaction · { $n } statements · { $age } · *one:* Transaction · { $n } statement · { $age } | İşlem · { $n } ifade · { $age } | Review: PO |
| `hv-tx-badge-short` | Transaction · { $n } statements · *one:* Transaction · { $n } statement | İşlem · { $n } ifade | Review: PO |
| `hv-tx-badge-failed` | Transaction failed | İşlem başarısız | Review: PO |
| `hv-tx-badge-tip` | Not committed yet: other sessions don't see these changes, and the rows they touch stay locked. | Henüz commit edilmedi: diğer oturumlar bu değişiklikleri görmez ve dokunulan satırlar kilitli kalır. | Review: PO |
| `hv-tx-badge-failed-tip` | An error or a cancel stopped the transaction. The server now accepts only a rollback. | Bir hata ya da iptal işlemi durdurdu. Sunucu artık yalnızca geri almayı kabul eder. | Review: PO |
| `hv-tx-badge-commit` | Commit | Commit et | Review: PO |
| `hv-tx-badge-rollback` | Roll back | Geri al | Review: PO |
| `hv-tx-segment-tip` | Click for Commit, Roll back and the transaction mode. | Commit, geri alma ve işlem kipi için tıklayın. | Review: PO |
| `exec-dur-lt-min` | <1 min | <1 dk | Review: PO |
| `hv-tx-idle-title` | Transaction open for { $age } on { $name } | { $name } üzerinde { $age } süredir açık bir işlem var | Review: PO |
| `hv-tx-idle-body` | It may hold locks that block other sessions. | Diğer oturumları engelleyen kilitler tutuyor olabilir. | Review: PO |
| `hv-tx-idle-commit` | Commit | Commit et | Review: PO |
| `hv-tx-idle-rollback` | Roll back | Geri al | Review: PO |
| `hv-tx-idle-keep` | Keep open | Açık bırak | Review: PO |
| `hv-cancel-slow-title` | Cancelling is taking long | İptal uzun sürüyor | Review: PO |
| `hv-cancel-slow-body` | The server hasn't stopped the statement on { $name } after { $duration }. | { $duration } geçti; sunucu { $name } üzerindeki ifadeyi hâlâ durdurmadı. | Review: PO |
| `hv-cancel-terminate` | Terminate on server | Sunucuda sonlandır | Review: PO |
| `hv-cancel-dlg-title` | The statement isn't answering the cancel | İfade iptale yanıt vermiyor | Review: PO |
| `hv-cancel-dlg-body` | It has been cancelling for { $duration }. Hacivat keeps waiting unless you choose otherwise. | { $duration } süredir iptal ediliyor. Siz başka bir şey seçmedikçe Hacivat beklemeyi sürdürür. | Review: PO |
| `hv-cancel-keep-waiting` | Keep waiting | Beklemeyi sürdür | Review: PO |
| `hv-cancel-terminate-line` | Terminate on server ends this session on the server with { $fn }. | “Sunucuda sonlandır” bu oturumu sunucuda { $fn } ile kapatır. | Review: PO |
| `hv-cancel-terminate-failed` | Couldn't terminate the session. Hacivat is still waiting for the cancel. | Oturum sonlandırılamadı. Hacivat iptali beklemeyi sürdürüyor. | Review: PO |
| `exec-terminated` | Session terminated on the server; its open transaction ({ $n } statements) was rolled back. | Oturum sunucuda sonlandırıldı; açık işlemi ({ $n } ifade) geri alındı. | Review: PO |
| `exec-terminate-failed` | Couldn't terminate the session on the server. | Oturum sunucuda sonlandırılamadı. | Review: PO |
| `hv-errbar-failed` | Statement { $i } of { $n } failed | { $i }. ifade başarısız oldu ({ $n } ifadeden) | Review: PO |
| `hv-errbar-aborted` | Statement { $i } of { $n } failed, and the transaction failed with it | { $i }. ifade başarısız oldu; işlem de başarısız oldu | Review: PO |
| `hv-errbar-waiting` | { $n } more wait | { $n } ifade daha bekliyor | Review: PO |
| `hv-errbar-retry` | Retry | Yeniden dene | Review: PO |
| `hv-errbar-ignore` | Ignore | Yok say | Review: PO |
| `hv-errbar-ignore-all` | Ignore all | Tümünü yok say | Review: PO |
| `hv-errbar-stop` | Stop | Durdur | Review: PO |
| `hv-errbar-rollback-continue` | Roll back and continue | Geri al ve devam et | Review: PO |
| `hv-errbar-rollback-continue-tip` | Rolls back your open transaction ({ $n } statements), then continues with statement { $next } in a new transaction. | Açık işleminizi ({ $n } ifade) geri alır, sonra { $next }. ifadeden yeni bir işlemde devam eder. | Review: PO |
| `exec-run-stopped` | Stopped at statement { $i }: you chose Stop. | { $i }. ifadede durduruldu: Durdur'u seçtiniz. | Review: PO |
| `exec-run-rolled-back-continue` | Transaction rolled back; continuing with statement { $i }. | İşlem geri alındı; { $i }. ifadeden devam ediliyor. | Review: PO |
| `hv-params-title` | Values for this statement | Bu ifadenin değerleri | Review: PO |
| `hv-params-sub` | { $n } parameters · *one:* { $n } parameter | { $n } parametre | Review: PO |
| `hv-params-col-name` | Name | Ad | Review: PO |
| `hv-params-col-value` | Value | Değer | Review: PO |
| `hv-params-col-type` | Type | Tür | Review: PO |
| `hv-params-null-tip` | Send NULL | NULL gönder | Review: PO |
| `hv-params-remembered` | remembered | hatırlandı | Review: PO |
| `hv-params-remembered-tip` | From an earlier run in this app session; never saved to disk. | Bu uygulama oturumundaki önceki bir çalıştırmadan; diske hiç kaydedilmez. | Review: PO |
| `hv-params-ambiguous` | Read as the decimal { $value }: a dot is the decimal point here. If you meant { $plain }, write it without the dot. | Ondalık { $value } olarak okunur: burada nokta ondalık ayırıcıdır. { $plain } demek istediyseniz noktasız yazın. | Review: PO |
| `hv-params-ambiguous-use` | Use { $value } | { $value } kullan | Review: PO |
| `hv-params-template` | { $var } is a template variable, not a parameter. Hacivat doesn't fill it in, so this statement can't run as written. | { $var } bir şablon değişkeni, parametre değil. Hacivat onu doldurmaz; bu yüzden ifade bu hâliyle çalıştırılamaz. | Review: PO |
| `hv-params-pg-q` | On PostgreSQL ? isn't a parameter here, because jsonb operators use it. Settings › Parameters can turn it on. | PostgreSQL'de ? burada parametre değildir, çünkü jsonb işleçleri onu kullanır. Ayarlar › Parametreler bölümünden açılabilir. | Review: PO |
| `hv-params-note` | Values are kept for this app session only and are masked in history. | Değerler yalnızca bu uygulama oturumu boyunca tutulur ve geçmişte maskelenir. | Review: PO |
| `hv-params-run` | Run | Çalıştır | Review: PO |
| `hv-params-cancel` | Cancel | Vazgeç | Review: PO |
| `hv-params-resolve-first` | Resolve the flagged value first | Önce işaretli değeri düzeltin | Review: PO |
| `hv-params-type-unknown` | unknown | bilinmiyor | Review: PO |
| `hv-chooser-run-as-one` | Run as one | Tek parça çalıştır | Review: PO |
| `hv-chooser-run-as-one-hint` | Run as one sends the whole region in one request; its results come back untyped. | Tek parça çalıştırma bölgenin tamamını tek istekte gönderir; sonuçları türsüz gelir. | Review: PO |
| `hv-chooser-customize` | Customize… | Özelleştir… | Review: PO |
| `hv-policy-title` | How Run picks statements in this console | Bu konsolda Çalıştır hangi ifadeleri seçer | Review: PO |
| `hv-policy-inside` | Caret inside a statement | İmleç bir ifadenin içindeyken | Review: PO |
| `hv-policy-inside-ask` | Ask | Sor | Review: PO |
| `hv-policy-inside-smallest` | Smallest statement | En küçük ifade | Review: PO |
| `hv-policy-inside-largest` | Largest statement | En geniş ifade | Review: PO |
| `hv-policy-inside-whole` | Whole script | Tüm betik | Review: PO |
| `hv-policy-selection` | With a selection | Seçim varken | Review: PO |
| `hv-policy-sel-separate` | As separate statements | Ayrı ifadeler olarak | Review: PO |
| `hv-policy-sel-as-one` | As one | Tek parça olarak | Review: PO |
| `hv-policy-note` | Kept for this console until it closes. | Bu konsol kapanana kadar geçerli. | Review: PO |
| `hv-editor-preview-as-one` | as one · { $n } statements | tek parça · { $n } ifade | Review: PO |
| `hv-results-tab-dup` | { $title } ({ $n }) | { $title } ({ $n }) | Review: PO |
| `hv-results-tab-pinned` | Pinned: the next run opens a new tab | Sabitlendi: bir sonraki çalıştırma yeni sekme açar | Review: PO |
| `hv-results-tab-pin` | Pin tab | Sekmeyi sabitle | Review: PO |
| `hv-results-tab-unpin` | Unpin tab | Sabitlemeyi kaldır | Review: PO |
| `hv-results-tab-close-others` | Close other tabs | Diğer sekmeleri kapat | Review: PO |
| `hv-results-untyped` | untyped (run as one) | türsüz (tek parça) | Review: PO |
| `hv-results-untyped-short` | untyped | türsüz | Review: PO |
| `hv-results-untyped-tip` | Run as one sends the statements together, so the server returns every column as text. Numbers sort as text. | Tek parça çalıştırma ifadeleri birlikte gönderir; bu yüzden sunucu her sütunu metin olarak döndürür. Sayılar metin gibi sıralanır. | Review: PO |
| `hv-results-set-limit` | The first { $kept } result sets are shown; { $skipped } more were read and not kept. | İlk { $kept } sonuç kümesi gösteriliyor; { $skipped } küme daha okundu ve tutulmadı. | Review: PO |
| `hv-results-count` | Count rows | Satırları say | Review: PO |
| `hv-results-count-estimate` | ≈ { $rows } rows (estimate) | ≈ { $rows } satır (tahmini) | Review: PO |
| `hv-results-count-exact` | { $rows } rows (exact) · *one:* { $rows } row (exact) | { $rows } satır (kesin) | Review: PO |
| `hv-results-counting` | Counting… | Sayılıyor… | Review: PO |
| `hv-results-count-cancel` | Cancel | Vazgeç | Review: PO |
| `hv-results-count-tip` | Runs SELECT COUNT(*) over this query on a separate session. | Bu sorgu üzerinde ayrı bir oturumda SELECT COUNT(*) çalıştırır. | Review: PO |
| `hv-strip-evicted` | Closed to free memory; the first { $rows } rows are kept. | Bellek açmak için kapatıldı; ilk { $rows } satır tutuldu. | Review: PO |
| `hv-fetchall-title` | Fetching every row would pass this result's memory limit | Tüm satırları getirmek bu sonucun bellek sınırını aşar | Review: PO |
| `hv-fetchall-body` | The query has more rows than fit in { $size }. Hacivat can fetch up to the limit and keep the cursor open for the rest. | Sorgunun satırları { $size } sınırına sığmıyor. Hacivat sınıra kadar getirip kalanı için imleci açık tutabilir. | Review: PO |
| `hv-fetchall-ok` | Fetch up to the memory limit | Bellek sınırına kadar getir | Review: PO |
| `hv-fetchall-cancel` | Cancel | Vazgeç | Review: PO |
| `exec-notice` | Notice | Bildirim | Review: PO |
| `exec-warning` | Warning | Uyarı | Review: PO |
| `exec-notices-dropped` | { $n } more notices not shown · *one:* { $n } more notice not shown | { $n } bildirim daha gösterilmedi | Review: PO |
| `exec-result-set-limit` | Kept the first { $kept } result sets; { $skipped } more were read and not shown. | İlk { $kept } sonuç kümesi tutuldu; { $skipped } küme daha okundu ve gösterilmedi. | Review: PO |
| `hv-history-title` | Query history | Sorgu geçmişi | Review: PO |
| `hv-history-search` | Search history | Geçmişte ara | Review: PO |
| `hv-history-search-tip` | Search ignores letter case and accents: musteri finds müşteri. | Arama büyük-küçük harfi ve aksanları yok sayar: musteri, müşteri'yi bulur. | Review: PO |
| `hv-history-filter-this` | This connection | Bu bağlantı | Review: PO |
| `hv-history-filter-all` | All connections | Tüm bağlantılar | Review: PO |
| `hv-history-outcome-any` | Any outcome | Tüm sonuçlar | Review: PO |
| `hv-history-outcome-ok` | Succeeded | Başarılı | Review: PO |
| `hv-history-outcome-failed` | Failed | Başarısız | Review: PO |
| `hv-history-outcome-cancelled` | Cancelled | İptal edildi | Review: PO |
| `hv-history-today` | Today | Bugün | Review: PO |
| `hv-history-yesterday` | Yesterday | Dün | Review: PO |
| `hv-history-count` | { $n } entries · *one:* { $n } entry | { $n } kayıt | Review: PO |
| `hv-history-rows` | { $rows } rows · *one:* { $rows } row | { $rows } satır | Review: PO |
| `hv-history-redacted` | redacted | gizlendi | Review: PO |
| `hv-history-redacted-line` | A password or secret was removed before this was saved. | Kaydedilmeden önce bir parola ya da gizli değer çıkarıldı. | Review: PO |
| `hv-history-pin` | Pin | Sabitle | Review: PO |
| `hv-history-pinned` | Pinned: never removed by retention | Sabitlendi: saklama süresi dolsa da silinmez | Review: PO |
| `hv-history-insert` | Insert | Ekle | Review: PO |
| `hv-history-insert-run` | Insert and run | Ekle ve çalıştır | Review: PO |
| `hv-history-copy` | Copy SQL | SQL'i kopyala | Review: PO |
| `hv-history-open-new` | Open in new console | Yeni konsolda aç | Review: PO |
| `hv-history-empty` | No history yet. Statements you run are kept here for 90 days. | Henüz geçmiş yok. Çalıştırdığınız ifadeler burada 90 gün tutulur. | Review: PO |
| `hv-history-nomatch` | Nothing matches “{ $q }”. | “{ $q }” ile eşleşen bir şey yok. | Review: PO |
| `hv-history-ran` | Ran { $n } times · *one:* Ran once | { $n } kez çalıştı | Review: PO |
| `hv-conn-sessions` | Sessions | Oturumlar | Review: PO |
| `hv-conn-single-session` | Use one session for every console | Tüm konsollar için tek oturum kullan | Review: PO |
| `hv-conn-single-session-sub` | Consoles of this connection share one session: its transaction, variables and temporary tables. | Bu bağlantının konsolları tek oturumu paylaşır: işlemini, değişkenlerini ve geçici tablolarını. | Review: PO |
| `hv-conn-budget-single` | One session, shared by every console | Tüm konsolların paylaştığı tek oturum | Review: PO |
| `hv-console-shared` | Shared session | Paylaşılan oturum | Review: PO |
| `hv-console-shared-tip` | Every console of { $name } uses this session: a transaction opened here is open in all of them. | { $name } bağlantısının tüm konsolları bu oturumu kullanır: burada açılan bir işlem hepsinde açıktır. | Review: PO |
| `hv-confirm-execute-all-scope` | Execute all also runs the remaining { $n } statements that would ask, without asking. A statement that needs its name typed still asks, and a production change still waits for your answer after it runs. · *one:* Execute all also runs the remaining { $n } statement that would ask, without asking. A statement that needs its name typed still asks, and a production change still waits for your answer after it runs. | Tümünü çalıştır, sorulacak kalan { $n } ifadeyi de sormadan çalıştırır. Adının yazılması gereken bir ifade yine sorar; üretimdeki bir değişiklik de çalıştıktan sonra yine yanıtınızı bekler. | Review: PO |
| `hv-commit-dropped-sp-body` | The server rolled back your open transaction: this statement and the { $n } before it. Nothing was committed. | Sunucu açık işleminizi geri aldı: bu ifade ve ondan önceki { $n } ifade. Hiçbir şey commit edilmedi. | Review: PO |
| `exec-confirm-dropped-sp` | Connection lost: the server rolled back your open transaction ({ $n } statements). | Bağlantı koptu: sunucu açık işleminizi ({ $n } ifade) geri aldı. | Review: PO |
| `hv-cancel-disconnect` | Disconnect | Bağlantıyı kes | Review: PO |
| `hv-cancel-options` | Show options… | Seçenekleri göster… | Review: PO |
| `hv-cancel-own-tx` | This console's open transaction ({ $n } statements) rolls back. | Bu konsolun açık işlemi ({ $n } ifade) geri alınır. | Review: PO |
| `hv-cancel-disconnect-line-all` | Disconnect closes every connection of { $name } here; the statement may continue on the server. | “Bağlantıyı kes”, { $name } için buradaki tüm bağlantıları kapatır; ifade sunucuda çalışmayı sürdürebilir. | Review: PO |
| `hv-cancel-disconnect-line-one` | Disconnect closes only this console's session ({ $console }); the statement may continue on the server. | “Bağlantıyı kes” yalnızca bu konsolun oturumunu ({ $console }) kapatır; ifade sunucuda çalışmayı sürdürebilir. | Review: PO |
| `hv-cancel-disconnect-line-local` | Disconnect closes this console's connection to the file { $name }; SQLite stops the statement when its connection closes. | “Bağlantıyı kes”, bu konsolun { $name } dosyasına bağlantısını kapatır; SQLite, bağlantı kapanınca ifadeyi durdurur. | Review: PO |
| `hv-cancel-other-tx` | { $console } also has an open transaction ({ $n } statements); the server rolls it back. | { $console } konsolunda da açık bir işlem var ({ $n } ifade); sunucu onu geri alır. | Review: PO |
| `exec-disconnected` | Disconnected { $name }. The statement may still be running on the server. | { $name } bağlantısı kesildi. İfade sunucuda hâlâ çalışıyor olabilir. | Review: PO |
| `hv-errbar-rollback-continue-tip-auto` | Rolls back your open transaction ({ $n } statements), then continues with statement { $next }, committing each statement as it runs. | Açık işleminizi ({ $n } ifade) geri alır, sonra { $next }. ifadeden devam eder; her ifade çalışınca commit edilir. | Review: PO |
| `hv-chooser-run-as-one-ro` | Not on a read-only session: its guard checks one statement at a time. | Salt okunur oturumda kullanılamaz: koruma her seferinde tek bir ifadeyi denetler. | Review: PO |
| `hv-history-target` | Goes into { $name }, the focused console; Run checks it there. | Odaktaki konsola, { $name } bağlantısına eklenir; Çalıştır denetimi orada yapılır. | Review: PO |
