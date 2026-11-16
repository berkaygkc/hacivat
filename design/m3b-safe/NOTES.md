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

---

## M3c surfaces

**Binding for M3c (surfaces 14–24, plus the Task 42 settings rows as surface 25). Task 2b, appended to the M3b part above, which stays binding as it is.** Everything in the M3b part's cross-cutting decisions holds here too; this part adds what M3c needs.

**How to drive it.** The mock bar's first row now holds 25 surfaces; 14–25 are M3c (the tag reads "MOCK · M3b + M3c"). Deep links work as before: `?s=16&sc=invalid&theme=light&lang=tr&size=960&bar=0&shot=1`. The number keys still reach surfaces 1–12 only; use the bar or a link for 13–25. The M3b surfaces keep their approved state, with two system-wide exceptions that fix round 1 made: the disabled ink token and the keyboard-active menu row (rulings M3c-36 and M3c-38). The M3c toolbar, grid and editor additions (view switch, record toggle, find bars, folds, carets) appear in the M3c surfaces only (ruling M3c-1). Mock-only marks are dashed: the "M3c" tags in the Settings navigation, the pane width labels, the mock notes and the OS candidate window.

**Flows that run.** Find: the toggles press, ↑/↓ (or Enter / ⇧Enter in the field) step the count, Escape closes the bar. Grid find: the same, plus Filter rows; Escape closes it. Folding: a chevron or a "⋯ n lines" inlay folds and unfolds. Gutter: ▶ runs the statement through the same path as Run. Mixed line endings: the status button opens the menu; Normalize removes the button. Views: the Segmented switches Table / Transposed / Text (Text above 100,000 rows asks first); the record toggle opens and closes the inspector, ▲▼ move it, Show more opens the value viewer. Value viewer: Raw / Pretty switch, Copy raw value, Re-run, Escape. Go to row: ⌘L opens it in any M3c grid; Escape or Enter closes it; Fetch and go shows the fetch running with Stop fetching. Text above 100,000 rows: Show as text shows the build running with Cancel. Grid find: Select all occurrences selects every match up to 10,000; Show all rows turns the filter off. Menus: arrows, Enter and Escape as in M3b; a menu item with an action performs it (Undo the selection change, Normalize, Open value viewer, Go to row…).

**Product truth used:** the plan's "New UI surfaces" 14–24, Tasks 2b, 31–39, 41, 42, the Task 1b chord table, R-M3-10, -64, -69, -70, -75, -82, P-M3b-17/-20/-21; DESIGN.md; this file's M3b part (RC-1…RC-6 and rulings 1–44); the shipped Settings page (`settings_ui/mod.rs`: 176 px navigation, 200 px title column, 320 px search, Segmented rows) and status bar (`connections/status.rs`); the Task 1b command titles and the M3a/M3b console strings, reused verbatim (`i18n-app`).

### M3c cross-cutting decisions

1. **One field anatomy for find.** The editor find bar and the grid find bar share the find field (search glyph, mono query, toggles inside the well), the count, previous / next and close. The grid adds Filter rows. Both offer Select all occurrences (⌃⌘G; one name, the shipped command title, ruling M3c-44), capped at 10,000.
2. **Find never invents a hue.** Other matches are a 1 px `text-3` outline; the current match is the selection (`blue-tint-strong`, the caret or the blue box); scrollbar ticks are `text-2`, the current one blue. A pending count uses the static amber marker; an invalid pattern is ink.
3. **Nothing covers a trust signal.** The find bar is a row under the editor toolbar (not a box over the code). The go-to popover and the value viewer never cross the title bar, **the console toolbar** (its crumb, mode chip, badge and read-only chips), the cursor strip or the status bar: they sit below the toolbar and above the strip, flipping above their cell when there is room; when there isn't, the value well shrinks first (down to 120 px) and scrolls, and the popover never moves up over the toolbar (fix round 2). Menus stop above the status bar. The checker asserts it in every render, the toolbar included.
4. **Data ellipsizes, chrome wraps or collapses.** Cell values, names in crumbs and tabs, the value viewer's title ellipsize (data). Chrome follows the Overflow Rule: the result toolbar drops rows · timing first (the strip and the status bar repeat it), then the view switch keeps its icons; narrow console toolbars wrap into two rows and chips wrap inside themselves; nothing chrome ends in "…".
5. **Local is said to be local.** A sort or a filter over fetched rows says so in a note under the toolbar; the database's order is never implied.
6. **Copy is raw and follows what is shown.** Pretty-printing, raw display and sorting change what you see, never the bytes Copy writes (Copy raw value; copy through the ViewMap).
7. **Keyboard-complete.** Every new surface has its chord or its menu path (Task 1b table); popovers take the focus (the go-to field, the value well, the fetch confirmation's primary, Stop fetching while it runs) and Escape returns it; menus opened from the keyboard start on their first item. An open find bar joins T2a ruling 40's F6 cycle first: find bar → error bar → toasts → editor.
8. **Disabled looks disabled** (fix round 1, ruling M3c-38): one disabled ink token, `text-disabled` (dark #636A76, 2.68:1 on `raised`, 1.77:1 from `text-2`; light #A0A0A8, 2.60:1 on white, 2.84:1 from `text-2`), for disabled buttons, Segmented segments, chips, find toggles and menu rows. The Raised Ink Rule never remaps it: inside the find bar and the result toolbar a disabled step button or segment is `text-disabled`, not `text-2`. A disabled item's reason stays `text-2` (readable). Owner: Task 41 (theme tokens), DESIGN.md row in Task 43.
9. **The keyboard-active menu row** (every PopupMenu, ruling M3c-36): `blue-tint-strong`, a 1 px blue left edge (the settings-navigation cue) and `text` ink for its label, hint and key. Owner: `hacivat-ui` PopupMenu, Task 41.

### 14 · Editor multi-cursor (Task 31)
**Intent.** Several carets edit as one; nothing about it is surprising for the IME or the clipboard.

**Anatomy.**
- Every caret is the existing 2 px blue caret; carets blink in phase (one timer); there is no visual "primary". Each selection uses `blue-tint-strong` (the M3a selection); unfocused, they dim to `seam-strong` like M3a's.
- **Status bar:** "3 carets" (editor group, before "27 selected"); it gives way together with the selected count. One caret shows nothing new.
- **Column selection** (⌥-drag; Ctrl+Alt-drag elsewhere): a box in display columns; a line shorter than the box gets an empty range with its caret at the line end (no virtual space).
- **IME, mirrored:** with every caret in plain text the marked text appears at every caret with the M3a underline (1 px, the selected clause 2 px); the OS candidate window anchors to the main caret only.
- **IME, main caret only (R-M3-64):** when any caret sits inside a string or a comment, the other carets collapse first and the composition goes to the main caret; the status bar says once: "Input method: typing at the main caret only, because another caret was inside a string or comment" (polite announcement).

- **Run follows the main caret** (controller ruling 2): with carets in several statements, the preview band and the gutter bar show only the main caret's statement (the last caret added), and that is what Run runs; carets in other statements add nothing to the run.

**States.** Next occurrence × 3 · Column selection · IME mirrored · IME main caret only · Run follows the main caret · 960 × 600 in Turkish.

**Keyboard.** ⌃G select the next occurrence (case- and Turkish-exact), ⌃⌘G select all occurrences, ⌘⌥↑/↓ add a caret above/below, ⌘U undo the selection change, Escape collapses to the main caret (M3a `editor.escape`).

### 15 · Folding (Task 32)
**Intent.** Long scripts fold by statement, block, block comment and `-- region`, without hiding what ran.

**Anatomy.**
- **Fold column:** the gutter's last 14 px, right of the line numbers (the gutter keeps M3a's 58 px + digits width). Chevron 12 px `text-2` (`text` on hover); chevron-down on an open range, chevron-right on a folded one. Open ranges show their chevrons only while the pointer is over the gutter or a gutter button has the keyboard focus; a folded header always shows its chevron.
- **Folded range:** the header line stays; after its text an inlay "⋯ 5 lines" (Source Sans 3 500 11/14, `text-2`, `seam-soft` fill (light: `seam`), radius 3, 8 px before it; `text` on hover; clicking unfolds); a 1 px `seam-soft` line under the header row. Line numbers skip the hidden lines.
- **Fold ranges:** statements, blocks, block comments and `-- region` markers, each at least three lines (a 2-line comment does not fold).
- **Marks:** a mark inside a fold paints on the header row; the one that most needs the user wins: awaiting an answer (the amber stamp) > cancel not answered (the amber hourglass) > failed > running > cancelling > queued > cancelled > rolled back / undone > skipped > done > stale. Its end-of-line status says what and where ("failed inside the fold · line 9" with the SQLSTATE chip; the line is the statement's first line; "cancel not answered · 14 s"). Range bars collapse to the header row.
- **Run at a caret inside a folded statement** runs the whole statement; the preview band and the gutter bar cover the header row; Describe execution range names the real lines ("Run executes lines 9–14: …").
- **Focus:** a focused chevron draws the 1 px blue focus ring.

**States.** Chevrons on hover · Folded statements · Folds + execution marks · Failure inside a fold · Run at a caret in a fold · Cancel unanswered in a fold · Keyboard: focused chevron · 960 × 600 in Turkish.

**Keyboard / a11y.** No default chord (Task 1b: Fold, Unfold, Fold all, Unfold all are palette commands). Chevrons are buttons (`aria-expanded`, name "Fold lines 2–7" / "Unfold lines 2–7"); folded lines leave the AccessKit text runs.

### 16 · Editor find panel (Task 33)
**Intent.** Find in a script without covering it or its toolbar, Turkish-aware.

**Anatomy** (the find bar, 32 px, `raised`, seam below, padding 0 10 0 12, gap 6, under the editor toolbar; Raised Ink Rule).
- **Find field** (the shared primitive): 24 px, field well (`panel` dark / `ground` light), 1 px `seam-strong`, radius 4, flex 1 1 300 px (150–380 px); search glyph 12 `text-3`; the query in Fragment Mono 12 (placeholder "Find" in Source Sans 12.5 `text-3`); then four 20 × 18 toggles inside the well: Match case (case-sensitive), Whole words (whole-word), Regular expression (regex), Turkish-tolerant (languages). A toggle is `text-2`; pressed: `blue-tint-strong` fill + 1 px blue border + `text`. Names and the Turkish-tolerant explanation are tooltips and accessible names.
- **Count:** Fragment Mono 11.5 `text-2`, at least 58 px wide so the bar does not jump: "3 of 41"; "No matches" (Source Sans 12 `text-2`); counting: the static amber marker and "3 of 1,240+"; invalid: ink circle-alert + "Invalid pattern" (`text`), the parser's message in the tooltip, and the field border `text-2`.
- Previous / next (ghost icon buttons, arrow-up / arrow-down), **Select all occurrences** (ghost, words, ⌃⌘G; the shipped command title; capped at 10,000: the status bar says "Selected the first 10,000 of 41,208 matches"), the grow, close (ghost x). With no matches or an invalid pattern, previous / next / Select all occurrences are disabled in `text-disabled`.
- **Matches in the text:** others a 1 px `text-3` outline (no fill); the current one is the selection with the caret. **Scrollbar:** a 9 px track at the code area's right edge with a 2 px tick per match (`text-2`), the current one blue.

**States.** 3 of 41 · Turkish-tolerant (on by default in Turkish) · Regex · Invalid regex · No matches · Counting · Select all · capped · 960 × 600 in Turkish.

**Keyboard.** ⌘F opens (the field takes the focus, seeded from the selection), Enter / ⇧Enter or ⌘G / ⇧⌘G step, ⌃⌘G selects all, Tab order field → toggles → previous → next → Select all → close; Escape closes and returns the caret to where find started. While the editor has the focus ⌘F shadows the explorer's filter focus (P-M3b-21).

### 17 · Gutter extras and line commands (Task 34)
**Intent.** Run a statement from its gutter and see its last runs; edit lines with one key; never be surprised by line endings.

**Anatomy.**
- **▶ (gutter run):** a 16 × 18 button in the mark column of a statement's first row, shown while the pointer is on that row or a gutter button has the focus; the 10 px play glyph `text-2`; hover: `hover` fill, `text`; focus: the 1 px blue ring. It takes a done or failed mark's place until the pointer leaves (the end-of-line status still shows the outcome). It runs through the same resolve, preview and gate path as ⌘↵. **▶ appears only while Run would be enabled** (not while a statement runs, waits for Commit / Roll back or Keep / Undo, is cancelling or waits for a connection), and never over a mark that needs the user or is still going (awaiting, escalated, running, cancelling, queued): those marks always keep their place.
- **Last-runs tooltip** (the gutter tooltip anatomy, 300 px): "Run this statement" with the ⌘↵ key, caption "Last runs", up to three rows (outcome glyph: green check / ink alert / ban `text-2`; the time in Fragment Mono 11 `text-2`; the M3a mark sentence; a failure's SQLSTATE chip), then "Runs through the same checks as Run." A statement never run says "Not run yet."
- **Mixed line endings:** a status-bar button "Mixed line endings" (`text`, the editor's right group, after Reduced mode). Its menu opens above it: a header "Line endings: LF 214, CRLF 12", then "Normalize to LF" with the hint "most lines" (the dominant ending, first) and "Normalize to CRLF". One undo group; the button leaves once the endings are uniform.
- **Line commands** in the editor menu (and the palette), with their keys: Comment or uncomment ⌘/, Duplicate line ⌘D, Delete line ⇧⌘K, Move line up ⌥⇧↑, Move line down ⌥⇧↓; then Select all occurrences ⌃⌘G and Undo the selection change ⌘U. A moved line carries a fold with it.
- **History recall:** ⌃⌥↑ / ⌃⌥↓ at the last line of an empty or recalled console puts the statement in the editor; the status bar says "From history: statement 2 of 48 · ⌃⌥↑ older · ⌃⌥↓ newer".

**States.** ▶ on hover · Last three runs · Keyboard: focused ▶ · No ▶ while Run is off (cancel not answered) · Mixed line endings · Normalize… · Line commands (menu) · Undo selection (⌘U) · History recall · 960 × 600 in Turkish.

### 18 · Grid model states (Tasks 35, 36)
**Intent.** Sort, freeze and select locally, and say so.

**Anatomy.**
- **Sort indicator:** in the header after the name, a 12 px blue arrow (up ascending, down descending); with two or more levels the level number follows in Fragment Mono 11 blue. Tooltip: "Sort level 1, ascending. Text compares by the column's collation, tr-x-icu." or, without a mapped collation, "…has no equivalent here, so text compares by code point."
- **Sort note** (the info row under the result toolbar; `raised`, padding 5/12, 12 px `text-2`, glyph arrow-up-down): partial result "Sorted locally: the database might order differently. Only the 10,000 fetched rows are sorted." + Clear sort (Default 20 px); sorting (after 200 ms) the static amber marker + "Sorting 1,048,576 rows…" + Cancel (the grid keeps the old order meanwhile); tail on a complete result "1,500 new rows at the end (not sorted)" + Re-sort; **partial result with a tail** (the common case after Fetch more) both facts in one row: "Sorted locally: the database might order differently. Only the 10,000 fetched rows are sorted; 1,500 new rows at the end are not." + Re-sort and Clear sort.
- **Unsorted tail boundary:** a 1 px dashed `text-3` line above the first tail row.
- **Frozen columns:** their own layer, a 1 px `seam-strong` right edge on the last frozen column (header and rows); the rest scrolls under it; no glyph; header tooltip "Frozen: stays in place when you scroll sideways".
- **Multi-rect selection:** one blue box per range (the M3a box and 5 px handle), the active cell `blue-tint-strong`; the strip says "Selection 2 ranges · 10 cells".
- **Column resize:** the header edge shows a 1 px `text-2` handle on hover; while dragging a dashed `text-2` guide runs down the grid and a tooltip shows the width ("136 px · Drag to resize · double-click to fit"); double-click fits.

**States.** Sort · one level · Sort · levels 1 and 2 · Sorted locally (partial) · Code-point collation (tooltip) · Sorting a million rows · Partial + unsorted tail · Complete + unsorted tail · Frozen columns, scrolled · Multi-rect selection · Column resize handle · 960 × 600 in Turkish.

### 19 · Grid find bar (Task 37)
**Intent.** Find over what the grid shows.

**Anatomy.** The find bar replaces the result toolbar's left group (Copy, Count rows) inside the 34 px toolbar: the find field (min 180 px, max 400 px) with five toggles (Match case, Whole words, Regular expression, Filter rows (filter glyph), Turkish-tolerant), the count, previous / next, close. The right group stays (rows · timing, the view switch, Record). Matches: a 1 px `text-3` outline around the matching text in the cell; the current one `blue-tint-strong` with a blue outline, in the focused cell with the blue box. The placeholder "Find in the shown values" carries the scope in its tooltip ("…NULL never matches."). Truncated cells add "· may be incomplete" (dotted underline, tooltip "1 value was cut at 1 MB; text past the cut isn't searched."). Scanning: "3 of 214+" with the static amber marker. **Select all occurrences** (ghost, ⌃⌘G) after the step buttons selects every match as a range, at most 10,000: the strip says "Selection 41 ranges · 41 cells"; past the cap "10,000 ranges · 10,000 cells" and the status bar "Selected the first 10,000 of 41,208 matches". **Filter rows:** only matching rows, numbered as view rows 1…n, with a note row under the toolbar (cross-cutting 5): "Showing the 41 rows that match, of the 10,000 fetched. Rows not fetched yet aren't filtered." + Show all rows. The cursor strip stays as it is (Cursor open, Close cursor, Fetch more, Fetch all).

**States.** k of N · Filter rows · May be incomplete · Scanning · Invalid regex · Select all occurrences · Select all · over 10,000 · 960 × 600 in Turkish.

**Keyboard.** ⌘F in the grid (shadows the explorer's filter focus while the grid has the focus), Enter / ⇧Enter step, Escape closes and returns the focus to the cell.

### 20 · Go to row (Task 37)
**Intent.** Jump to a row and column by number, in the format locale.

**Anatomy** (the explanation-popover anatomy, 300 px, under the result toolbar's left edge; clamped above the status bar): title "Go to row" with the list-ordered glyph and the ⌘L key; a 26 px mono field (placeholder "row or row:column"); one line under it (12 px, a 12 px glyph column): the hint (info `text-2`), the live target (the ↵ glyph, "Row 1,000, column city"; `1.000` is row 1000 in Turkish), or an ink error with the field border `text-2`: "There's no row 40: the result has 12 rows.", "No column is named ilce.", "Column meta is hidden." + Show it and go (Default). Key hints "↵ Go · esc Close" (Close reuses the palette's key hint). The × is not a Tab stop (T2a dialogs); Tab cycles inside the popover; a click outside closes it. Past the fetched rows on an open cursor it fetches; above 1,000,000 rows it asks in place: "Row 1,500,000 isn't fetched yet. Fetching 1,490,000 more rows takes a while, and the result's memory limit still applies." with Cancel (Default) and **Fetch and go** (Primary, last, focused: bounded by the memory limit, cancellable). **While it fetches** the popover shows the static amber marker and "Fetching to row 1,500,000: 214,000 of 1,490,000 rows", the line "Escape hides this; fetching goes on in the strip." and **Stop fetching** (focused). Escape hides the popover and the strip keeps "Fetching… 224,000 rows"; Stop fetching ends the fetch and keeps the cursor open with what was fetched; when the row arrives the grid goes to it. Below 1,000,000 rows the same running state appears without the question.

**States.** Popover · Turkish 1.000 · Out of range · Unknown column · Hidden column · Fetch to row (> 1M) · Fetching to the row · 960 × 600 in Turkish.

### 21 · Transposed, record and text views (Task 38)
**Intent.** Read a result the way it reads best, one record at a time when that's easier.

**Anatomy.**
- **View switch:** a 22 px Segmented in the result toolbar's right group: Table / Transposed / Text (table-2, table-properties, align-left glyphs + words). Transposed is disabled above 100,000 rows: `text-disabled`, no hover, focusable, reason in its tooltip ("Transposed shows up to 100,000 rows; this result has 1,048,576."). Overflow: the words go first (icons stay, names in tooltips and accessible names).
- **Record toggle:** a ghost icon button (panel-right) after the switch; pressed: `blue-tint-strong` + blue border. A one-row result shows it with its word ("Record").
- **Record inspector:** 252 px on the `gutter` tone with a seam on its left; a 30 px header (`head`): small-caps "RECORD", "Row 4 of 184" (numbers in mono), ▲ ▼ (Alt+↑ / Alt+↓). One field per column: the name (Fragment Mono 11 `text`), a duplicate suffix "(2)" in `text-2` with a tooltip, the type right-aligned (mono 11 `text-3`), the value below (13 px; mono 11.5 for mono types; numbers right-aligned) clamped to three lines with "Show more" (opens the value viewer). Fields are divided by `seam-soft`; virtualized above 200 columns.
- **Transposed:** the first column (150 px, `head` tone) holds each column's name (600) and type (mono 11 `text-3`); one column per row titled by its number (mono, right-aligned); 34 px rows.
- **Text:** a Pretty ASCII table in Fragment Mono 11.5/17 on the panel, rules in `text-2`, widths capped at 255 characters; whole lines select (`blue-tint-strong`) and copy; a 4 px horizontal thumb says a wide table scrolls sideways. Above 100,000 rows it asks in place with an inline notice: "Show 1,048,576 rows as text?" / "Building the text view for this many rows takes a while and uses about 180 MB of memory." with Back to the table (Default) and **Show as text** (Primary, last: the M2a footer order). The build then runs in place: the static amber marker, "Building the text view: 412,000 of 1,048,576 rows" and Cancel (focused; Cancel or Escape returns to the table).

**States.** Transposed disabled · Transposed · Record inspector · Duplicate names · Show more · One-row result · Text view · Text view > 100k asks · Building the text view · 960 × 600 in Turkish.

### 22 · Value viewer (Task 38, R-M3-82)
**Intent.** Read one value whole; copy exactly what the server sent.

**Anatomy** (480 px popover on the dialog anatomy: `raised`, 1 px `seam-strong`, radius 6, popover shadow; no scrim, not modal; anchored under its cell, flipped above or clamped so it never crosses the title bar or the status bar).
- **Header:** the column name (Fragment Mono 12.5/600, data, ellipsizes), "{type} · row 4" (12.5 `text-2`, type and row in mono), the × (name "Close", `hui-dialog-close`; not a Tab stop, like T2a's dialogs).
- **Control row:** for JSON and XML a 22 px Segmented Raw / Pretty (Pretty's tooltip: "Display only: Copy raw value copies the server's text unchanged."); invalid JSON keeps Raw pressed and Pretty disabled, with the line "Not valid JSON, so Pretty is off and the value shows as the server sent it." beside the switch; XML Pretty indents elements and colours tags (keyword ink), attribute names (identifier ink) and values (string ink); for binary a single "Hex" segment.
- **Value well:** `panel` / `ground`, 1 px `seam-strong`, radius 4, padding 7/10, Fragment Mono 11.5/17, wrapped, scrolls past 240 px (4 px thumb). JSON uses the syntax tokens (keys identifier ink, strings green, numbers number ink, literals keyword ink). Hex: offset (`text-3`), then **8 bytes per row below a 560 px popover** (16 from 560 px) in 2-byte groups, then the ASCII column (`text-2`), so the ASCII column is never clipped. A value taller than the well shows its 4 px `seam-strong` thumb (drawn: the well scrolls).
- **Truncated value:** an inline notice "Showing the first 1 MB of 5.2 MB." with **Re-run with a 64 MB value limit** (Default, rotate-cw; tooltip: it re-runs the statement with a larger cell cap through the same checks as Run; on production the gate asks).
- **Footer:** the size ("1,048,576 characters", "48,213 bytes", mono 11.5 `text-2`) and **Copy raw value** (Default).

**States.** Wrapped text · JSON Pretty · JSON Raw · Binary (hex) · XML Pretty · Invalid JSON · Truncated · Re-run · 960 × 600 in Turkish.

**Keyboard.** ⇧↵ opens from the focused cell; the value well takes the focus (arrows scroll); Tab cycles Raw / Pretty → well → Re-run → Copy raw value inside the popover; Escape or a click outside closes it and returns the focus to the cell.

### 23 · Grid context menu (Task 39)
**Intent.** Every grid command within reach, with disabled items saying why.

**Anatomy** (the PopupMenu with a 12 px icon column; M3c menus share one row anatomy). Disabled items keep their place in `text-disabled`, with the reason as the right-hand hint in `text-2`; a checkbox item (Raw display) shows the blue check in the icon column and its scope hint "this column".
- **Cell:** Copy ⌘C · Copy with headers | Open value viewer ⇧↵ · Raw display (this column) | Sort ascending · Sort descending · Clear sort | Go to row… ⌘L.
- **Header:** Sort ascending (✓ when current) · Sort descending · Clear sort · Add as sort level 2 (when another column is sorted) | Freeze up to here / Unfreeze columns · Hide column · Show all columns · Fit width | Raw display · Copy column name.
- **Row number:** Copy rows ⌘C · Copy with headers | Show as record · Go to row… ⌘L.
- Reasons: "Ranges don't line up" (copy of a non-stackable union, M3a refusal), "Select one cell", "Not sorted", "None hidden", "Last shown column".

**States.** Cell · Header · Header · frozen (Unfreeze columns) · Row number · Disabled with reasons · Keyboard (⇧F10) · 960 × 600 in Turkish.

**Keyboard.** ⇧F10 or the context-menu key opens it at the focus cell with the first item active (selection tint); a double-click on a header edge fits the column.

### 24 · Narrow panes (Task 41, R-M3-75)
**Intent.** A console split down to 120 px keeps every trust signal whole in every locale.

**Anatomy** (below the per-locale minimum: en 126, tr 141, pseudo 219; the mock draws panes of 120, 141, 180 and 219 px).
- **A pane is a console tile:** its tabs, its toolbar, its editor and, at its foot, its results (drawn collapsed to the cursor strip). The panes span the centre column; the status bar follows the focused pane (its environment, transaction, cursor, rows, editor segment), like the capsule.
- The console toolbar wraps into two rows (`Toolbar::wrap_row`; padding 4/8, 4 px between rows).
- **The mode chip keeps its word at every width:** "Auto" / "Otomatik" (`hv-tx-chip-auto-short`), "Manual" / "Manuel", "Smart" / "Akıllı", with its chevron; only the commit glyph goes. Production panes keep "Manual", the production marker after the unlock (T2a ruling 27). A read-only pane keeps T2a's disabled "Auto-commit" chip (`text-disabled`, focusable, reason in the tooltip).
- **Read-only chip:** the lock (red on production) and the whole label "read-only (guarded)" / "salt okunur (korumalı)", wrapping inside the chip.
- **Waiting item:** two lines, "Waiting for a connection" / "4 of 4 in use" (`text-2`), then Cancel.
- **Badge:** the stamp, "Transaction · 3 statements" (wrapping inside the chip) and Commit and Roll back as 20 px icon buttons (check, undo-2; words in tooltips and names).
- **Chips wrap inside themselves** rather than clip (the pending read-only chip "Read-only after this transaction", the badge); no chrome ends in "…".
- **Strip** (the pane's own results): "Cursor open" (the short form), Close cursor, Fetch more as an icon; it wraps.
- **Status bar:** the editor's segment sits in the right group before the version (the M3a order); Reduced mode never gives way; Ln/Col gives way after the path; the collapse order is version, encryption, selected count, carets, mixed line endings, path, Ln/Col, rows · timing. "Nothing to run here" is a status sentence (Task 41 carry), and an empty console draws the editor's placeholder, wrapping in a narrow pane.
- The production hairline and tab swatch stay on every production pane.

**Named exceptions to T2a cross-cutting 7 and surface 4** (below the per-locale minimum only):
- **N1 · Run and Stop icon-only.** Their words and keys stay in the tooltip and the accessible name ("Run · ⌘↵", "Stop · ⌘.").
- **N2 · The connection name may ellipsize** (it is data, whole in the tooltip and the tab's tooltip); the environment mark before it never does, nor the tab swatch or the hairline.
- **N3 · The badge drops its age;** Commit and Roll back stay in the badge as icon buttons (and in the mode chip's menu, as in T2a surface 4).
- **N4 · The mode chip drops its commit glyph** and keeps its word; "Auto-commit" becomes "Auto" except on a read-only pane, whose disabled chip keeps "Auto-commit".
Nothing else relaxes: the read-only label stays whole ("read-only (guarded)"), the waiting sentence keeps all its words on two lines, and the status bar keeps every trust segment.

**States.** Idle · read-only production · Running / cancelling · Waiting (two lines) · Pending read-only + badge · Strip · Cursor open · Status · Reduced never gives way · Long name ellipsized, mark kept · Empty console · Nothing to run · Pseudo en-XA · 219 px · 960 × 600 in Turkish.

### 25 · Settings rows (Task 42)
**Intent.** The M3b settings get rows on the shipped Settings page.

**Anatomy** (the shipped page: a 176 px navigation on the `side` tone, the selected item `blue-tint` with a 1 px blue left edge; a search bar (field max 320 px, padding 8/12, seam below); a 30 px small-caps section title (`text-2`); rows padding 6/12, a 200 px title column in 13 px `text-2`, the control (26 px Segmented), then notes in 12 px `text-2`).
- Four sections join General / Appearance / Keyboard: **Editor** (Tab size 2 / 4 / 8 in mono; Indent with spaces; Copy the line when nothing is selected; Blinking caret), **Execution** (Confirm UPDATE or DELETE without WHERE off production, note "Production always asks, with the table's name typed."; Ask what to do when a statement fails, note "Off: a run of several statements stops at the first failure."), **Results** (Largest value kept per cell: an 88 px mono number field, right-aligned, and a 76 px KB / MB Select, note "From 64 KB to 64 MB. …"; Show grid values raw, note), **History** (Record query history, note: "Up to 10,000 statements per connection are kept for 90 days; recognised passwords and secrets are masked." (R-M3-62)).
- **Clamp:** a value above 64 MB (or below 64 KB) is set to the bound and says so under the control with the info glyph: "128 MB is above the largest allowed, so it was set to 64 MB."
- **Search** finds rows across sections in English and Turkish (Turkish-tolerant); while searching, the section title hides (shipped behaviour) and each row names its section after the control.

**States.** Editor · Execution · Results · Value limit clamped · Value limit in KB · Search (TR) · 960 × 600 in Turkish.

### New primitives (M3c)

All use existing tokens only and are reproducible in GPUI.

| Primitive | Anatomy and exact tokens |
|---|---|
| **Find field** | 24 px; field well `panel` / `ground`; 1 px `seam-strong`; radius 4; padding 0 2 0 7; search glyph 12 `text-3`; query Fragment Mono 12; invalid: border `text-2`. |
| **Disabled ink** | `text-disabled`: dark #636A76, light #A0A0A8; disabled buttons (and their keys and glyphs), segments, chips, toggles, menu rows; never remapped by the Raised Ink Rule; a reason hint stays `text-2`. |
| **Find toggle** | 20 × 18 inside the field; radius 3; 1 px transparent border; glyph 13 px `text-2`; hover `hover` + `text`; pressed `blue-tint-strong` + 1 px blue + `text`; disabled `text-disabled`; focus ring 1 px blue. |
| **Find count** | min 58 px; Fragment Mono 11.5 `text-2`; counting: 7 px amber marker; invalid: ink glyph + `text`; incomplete: `text-2` dotted underline. |
| **Find bar** | 32 px, `raised`, seam below, padding 0 10 0 12, gap 6; buttons 22 px. |
| **Match outline** | 1 px `text-3` outline, no fill; the current match is the selection. |
| **Scrollbar ticks** | 9 px track, `seam-soft` left edge, thumb `seam-strong` radius 3; ticks 2 px `text-2`, current blue. |
| **Fold chevron** | 12 × 18 in the gutter's last 14 px; chevron-down / -right 12 px `text-2`; focus ring 1 px blue. |
| **Fold inlay** | 14 px; Source Sans 3 500 11/14 `text-2`; `seam-soft` (light `seam`); radius 3; padding 0 5; 8 px before; "⋯ n lines". |
| **Gutter ▶** | 16 × 18 in the mark column; play 10 px `text-2`; hover `hover` + `text`; focus ring 1 px blue. |
| **Sort indicator** | arrow-up / arrow-down 12 px blue; level digit Fragment Mono 11 blue (two or more levels only). |
| **Grid note row** | the M3a info row (`raised`, padding 5/12, 12 px `text-2`); glyph 12; a 20 px Default button on the right; pending: the 7 px amber marker. |
| **Tail boundary** | 1 px dashed `text-3` across the row above the tail. |
| **Frozen edge** | 1 px `seam-strong` right border on the last frozen column. |
| **Resize handle / guide** | 1 px `text-2` on the header edge; dashed 1 px `text-2` guide; width tooltip (popover anatomy, 12 px, mono value). |
| **View switch** | 22 px Segmented; segments 12 px with a 12 px glyph; disabled `text-disabled`, focusable; compact: glyphs only. |
| **Record inspector** | 252 px `gutter`; header 30 px `head`; fields padding 5/12/6, `seam-soft` dividers; key mono 11, type mono 11 `text-3`, value 13 (mono 11.5) clamped to 3 lines. |
| **Value viewer** | 480 px popover on the dialog anatomy (header 12/12/0/16, body 8/16, footer 12/16/14); value well like the SQL inset, max 240 px, shrinking to 120 px before the popover would reach the console toolbar. |
| **Go-to popover** | 300 px explanation-popover anatomy; 26 px mono field; one 12 px line with a 12 px glyph column; key hints 11.5 `text-2`. |
| **Menu row** (every PopupMenu) | PopupMenu row + 12 px icon / check column; disabled `text-disabled` with its reason as a `text-2` hint; keyboard-active row `blue-tint-strong`, a 1 px blue left edge, `text` ink for label, hint and key. |
| **Narrow toolbar** | `wrap_row`: padding 4/8, row gap 4, column gap 6; icon-only Run / Stop (N1); the mode chip keeps its word (N4); chips wrap (line 15 px); badge buttons 20 px icons (N3). |
| **Progress line** | the static 7 px amber marker, a sentence with mono counts, one Default Stop / Cancel button; in the go-to popover, the text view's notice and the sort note row. |
| **Settings row** | padding 6/12; title column 200 px 13 `text-2`; 26 px Segmented / field; notes 12 `text-2`. |

**Icons (RC-4 pattern).** The mock adds four Lucide 0.460 icons (ISC) that the app does not ship: `arrow-up` (ascending sort, previous match, previous record), `case-sensitive`, `whole-word`, `regex` (find toggles). Controller ruling 8: **Task 31 (W8) adds all four** as a named exception, like RC-4 (SVGs, `assets.rs` rows, `IconName` variants, `new_icons_are_embedded`), so Task 33 (find toggles) and Task 36 (the sort arrow, which runs in W9 beside Task 33) both find them in place. Everything else reuses shipped glyphs: `languages` (Turkish-tolerant), `filter` (Filter rows), `snowflake` (freeze), `arrow-up-down` (sort notes, add a sort level), `table-2` / `table-properties` / `align-left` (views), `panel-right` (record), `eye` / `eye-off`, `arrow-left-right` (fit width), `list-ordered` (go to row), `play` (gutter run), `chevron-down` / `chevron-right` (folds).

### M3c rulings

Each answers an open layout question so the build is not blocked; the controller or the PO can change any.

1. **M3c-1 · The M3b surfaces stay as approved,** except for the two system-wide tokens of fix round 1 (disabled ink, M3c-38; the keyboard-active menu row, M3c-36), which the M3b surfaces adopt at build; their M3b shots change only there (listed in Verification). The M3c additions (view switch, record toggle, find bars, folds, carets) appear in surfaces 14–25 only. *Cost if wrong:* none (mock structure).
2. **M3c-2 · Carets look alike.** Every caret is the M3a caret and they blink in phase; nothing marks the main caret (only the IME and the OS need it). *Cost:* one paint flag in Task 31.
3. **M3c-3 · "3 carets" in the status bar,** in the editor group before the selected count, giving way with it. *Cost:* one segment.
4. **M3c-4 · The IME fallback is said once,** as a transient status sentence (polite), never a toast. *Cost:* one string.
5. **M3c-5 · Fold column right of the line numbers,** in the gutter's existing 14 px right padding, so the gutter width does not change; open chevrons only on gutter hover or focus, folded ones always. *Cost:* gutter layout in Task 32.
6. **M3c-6 · Fold inlay "⋯ n lines"** on the DESIGN.md inlay token; n is the hidden line count. *Cost:* one string.
7. **M3c-7 · Marks in a fold:** the mark that most needs the user wins on the header, M3b marks included: awaiting answer > escalated > failed > running > cancelling > queued > cancelled > rolled back > skipped > done > stale; a failure names the failing statement's first line. *Cost:* one order in Task 32's `marks.rs`.
8. **M3c-8 · No default fold chord** (Task 1b table lists none): palette, gutter buttons and AccessKit. *Cost:* a later keymap row.
9. **M3c-9 · The editor find bar is a 32 px row under the toolbar,** not a box floating over the code: the toolbar's trust signals and the first lines stay visible, and it is the same anatomy the grid uses. *Cost:* layout in Task 33.
10. **M3c-10 · Match colours:** outline `text-3` for other matches, the selection for the current one, `text-2` / blue scrollbar ticks; no amber (pending) or red. *Cost:* paint constants.
11. **M3c-11 · An invalid regex is ink, not red** (Task 33's text says "turns the field red"; Hue Is Meaning and the M3b type-to-confirm precedent): border `text-2`, ink glyph, "Invalid pattern". *Cost:* one colour.
12. **M3c-12 · Counting shows "3 of 1,240+"** with the static amber marker (no spinner; M3a ruling 5). *Cost:* one string.
13. **M3c-13 · Select all occurrences past 10,000** (editor and grid) selects the first 10,000 and says so in the status bar. *Cost:* one string.
14. **M3c-14 · Find toggles are icon buttons** inside the field (case-sensitive, whole-word, regex, languages; filter in the grid); the words live in tooltips and accessible names. *Cost:* four icons (Task 31, controller ruling 8).
15. **M3c-15 · ▶ takes a done or failed mark's place while hovered or focused, only while Run would be enabled, and never over an awaiting, escalated, running, cancelling or queued mark;** its tooltip shows the last three runs; a failure's details stay reachable from the end-of-line chip and Output. *Cost:* gutter hit-testing in Task 34.
16. **M3c-16 · Mixed line endings is a status-bar button** in the editor group with a menu (counts, dominant ending first with "most lines"); never automatic. *Cost:* one segment, one menu.
17. **M3c-17 · Line commands live in the editor menu** (two new groups) and the palette, each with its chord. *Cost:* menu rows.
18. **M3c-18 · History recall says where you are** in the status bar ("From history: statement 2 of 48 · ⌃⌥↑ older · ⌃⌥↓ newer"). *Cost:* one string.
19. **M3c-19 · Sort level numbers only with two or more levels;** the collation lives in the header tooltip. *Cost:* one condition.
20. **M3c-20 · Local sort states are a note row** under the toolbar: partial (Clear sort), sorting (Cancel), unsorted tail on a complete result (Re-sort), and partial with a tail in one row (Re-sort, Clear sort); the tail is marked in the grid by a dashed `text-3` line. *Cost:* one row component.
21. **M3c-21 · Frozen columns get an edge, no glyph.** *Cost:* none.
22. **M3c-22 · Multi-rect strip reads "2 ranges · 10 cells".** *Cost:* one string.
23. **M3c-23 · Column resize** draws a `text-2` handle, a dashed guide and the width in px. Surface 18's "column grow/shrink handle" is read as this width handle; growing and shrinking the selection (⌥↑ / ⌥↓, cell → row segment → row → all) has no surface beyond the M3a selection box. *Cost:* paint in Task 35.
24. **M3c-24 · The grid find bar keeps the right group;** the toolbar's Overflow order is rows · timing first, then the view switch's words. *Cost:* priorities in Task 37/38.
25. **M3c-25 · Grid matches outline the matching text,** not the whole cell (what you see is what matched). *Cost:* text-run decoration in the grid.
26. **M3c-26 · Filter rows numbers view rows 1…n** and says so in a note row under the toolbar (with Show all rows); the cursor strip is never replaced. *Cost:* one note row.
27. **M3c-27 · Go to row is a popover,** not a dialog; errors inline; above 1M rows the fetch confirmation is inline with **Fetch and go** Primary and focused (bounded by the memory limit and cancellable; the M3b fetch-all precedent, ruling 22). *Cost:* focus target.
28. **M3c-28 · The view switch is a Segmented in the right group;** disabled Transposed keeps `text-3` and its reason in the tooltip (it is focusable, like the read-only chip). *Cost:* none.
29. **M3c-29 · A one-row result shows the Record toggle with its word,** no hue (Task 38's "highlights the toggle"). *Cost:* one label.
30. **M3c-30 · Text view above 100k asks in place** (an inline notice, not a dialog), Show as text Primary. *Cost:* one notice.
31. **M3c-31 · The value viewer is non-modal,** anchored to its cell, never over the title or status bar; Re-run and Copy raw value are Default (nothing destructive, no single primary action). *Cost:* none.
32. **M3c-32 · Sizes read "64 MB",** formatted by the existing `exec-size-*` messages; the plan's "64 MiB" names the binary unit the app already displays as MB. *Cost:* one string.
33. **M3c-33 · Raw display is per column in the menu** (a checkbox item with "this column") and global in Settings. *Cost:* none.
34. **M3c-34 · Narrow forms** (named exceptions N1–N4 in surface 24): two-row toolbar; icon-only Run / Stop with their words in tooltips and names; the crumb's name ellipsizes (data) but never its mark; the mode chip keeps its word; the read-only chip keeps its whole label; the badge keeps Commit and Roll back as icons; chips wrap inside themselves; Fetch more becomes an icon in the strip. *Cost:* `ToolbarItem::narrow` forms in Task 41.
35. **M3c-35 · Status bar order:** the editor segment stays in the right group before the version, as the M3a NOTES (binding) draw it; Task 41's "right of the version" is read as "in the right-aligned group". *Cost:* the order of two cells.
36. **M3c-36 · The keyboard-active row of every PopupMenu** (controller ruling 5, fix round 1): `blue-tint-strong`, a 1 px blue left edge and `text` ink for label, hint and key; the `hover` row was nearly invisible on `raised` in dark (1.50:1 for the tint alone). Owner: `hacivat-ui` PopupMenu, Task 41; the M3a/M3b menu shots change. *Cost:* one row style.
37. **M3c-37 · Settings get four sections** (Editor, Execution, Results, History) rather than one "M3b" section; the value limit is a number field plus a KB / MB Select with a clamp note. *Cost:* navigation rows in Task 42.
38. **M3c-38 · Disabled ink token** (controller ruling 4, fix round 1): `text-disabled` dark #636A76 / light #A0A0A8 for every disabled control and menu row, never remapped inside raised bars. Owner: Task 41 (theme token), DESIGN.md row in Task 43. *Cost:* one token.
39. **M3c-39 · Grid Select all occurrences** is drawn (Task 37): ⌃⌘G, at most 10,000 ranges, the strip counts ranges and cells, past the cap the status bar says so. *Cost:* none (plan text).
40. **M3c-40 · Long local operations show progress** in place with Stop / Cancel: the go-to fetch, the text-view build (the sort already did). Escape hides the go-to popover and the fetch goes on in the strip. *Cost:* two states.
41. **M3c-41 · A narrow pane is a console tile** (editor over its own results, the strip at its foot); the status bar follows the focused pane. *Cost:* none (mock structure; Task 41's sweep).
42. **M3c-42 · Footer order is M2a's** in every inline question: Default first, the Primary last (go-to fetch, text view). *Cost:* none.
43. **M3c-43 · Every fold range is at least three lines,** block comments included. *Cost:* one constant.
44. **M3c-44 · One chord, one name:** ⌃⌘G is "Select all occurrences" (the shipped `hv-cmd-editor-select-all-occurrences`) in the find bars and the editor menu. *Cost:* none.

## Plan notes (M3c)

Each ruling that changes plan text, as a note on the task that owns it (the T2a precedent: rulings 10, 21, 29). The controller accepted every one (`progress.md`, T2b and fix round 1).

- **Task 41 (M3c-35):** the Files line's "a `status_right` slot placing the editor segment right of the version per the mock" reads "the editor segment in the right-aligned group, before the version, per the mock"; the test `editor_segment_sits_right_of_the_version` is renamed `editor_segment_sits_before_the_version`.
- **Task 41 (M3c-36, M3c-38):** adds the `text-disabled` theme token (dark #636A76, light #A0A0A8) and the PopupMenu keyboard-active row (selection tint, 1 px blue left edge, `text` ink); the M3a/M3b shots that show a disabled control or a keyboard-active menu row are re-baselined, listed PNG by PNG. **Task 43** adds both DESIGN.md rows.
- **Task 41 (M3c-34):** the narrow forms are named exceptions N1–N4 to T2a cross-cutting 7 / surface 4 (NOTES surface 24); `trust_items_whole_down_to_120px_in_every_locale` also asserts the mode chip shows its mode word.
- **Task 33 (M3c-11):** "invalid regex turns the field red" reads "an invalid regex keeps the field border `text-2` and shows the ink glyph and 'Invalid pattern'; nothing is searched".
- **Tasks 38 and 42 (M3c-32):** "Re-run with a 64 MiB value limit" and "64 KiB–64 MiB" are displayed "64 MB" and "64 KB–64 MB" through the `exec-size-*` messages; the values stay binary units.
- **Task 38 (M3c-29):** "a one-row result highlights the toggle" reads "a one-row result shows the Record toggle with its word". Also (M3c-40): the text-view build shows progress with Cancel.
- **Task 37 (M3c-39, M3c-26, M3c-40, M3c-44):** grid Select all occurrences as drawn; Task 37 owns its own grid command row, `grid.select_all_occurrences` with the title `hv-cmd-grid-select-all-occurrences` (EN "Grid: select all occurrences", TR "Tablo: tüm eşleşmeleri seç"; the button shows the words without the "Grid:" prefix, like the other grid commands' surface labels), bound to ⌃⌘G in the [Grid] context, and stops reusing the editor's `hv-cmd-editor-select-all-occurrences` (the mock still borrows the editor title until then; one chord, one name, two namespaced ids); Filter rows adds a note row and never replaces the cursor strip; the go-to fetch shows progress with Stop fetching.
- **Task 36 (M3c-20):** a partial result with an unsorted tail shows both facts in one note row.
- **Task 34 (M3c-15):** ▶ only while Run would be enabled, never over awaiting / escalated / running / cancelling / queued marks.
- **Task 32 (M3c-7, M3c-43):** the fold-header severity order includes the M3b marks; every fold range is at least three lines.
- **Task 31 (controller ruling 8):** adds the four icons (`arrow-up`, `case-sensitive`, `whole-word`, `regex`) as a named exception like RC-4; Tasks 33 and 36 consume them. Also (controller ruling 2): Run with several carets runs only the main caret's statement.

### Open items for the PO (M3c)

1. **Turkish copy:** every M3c TR string is `Review: PO`; in particular "Türkçe esnek" (Turkish-tolerant), "Devrik" (Transposed; "Döndürülmüş" is the alternative), "Biçimli" / "Ham" (Pretty / Raw), "Onaltılık" (Hex), "harmanlama" (collation), "Satırları süz" (Filter rows), "Ham görünüm" (Raw display), "Karışık satır sonları", "Hepsini LF yap" (Normalize to LF), "Dondurmayı kaldır" (Unfreeze), "Otomatik" (the narrow Auto).
2. **Fold chords:** none by default (ruling M3c-8, controller ruling 7: avoid zoom-chord clashes).

Ruled by the controller (no longer open): Run follows the main caret (2); gutter buttons reachable in "Tab moves focus" mode (3); the disabled ink token (4, M3c-38); the keyboard-active row in every menu (5, M3c-36); the status-bar order (6, M3c-35); four icons by Task 31 (8); invalid regex in ink (9, M3c-11); sizes in MB (M3c-32).

### Verification (M3c)

See the Task 2b report for the run log. In short:
- The harness (rebuilt in the session scratchpad, not committed; Node 20 `--experimental-websocket` + headless Chrome over DevTools) rendered every M3c scenario (12 surfaces, 100 scenarios) × EN/TR × dark/light × 1280 × 800 and 960 × 600 with `bar=0&shot=1`: **800 renders (100 scenarios × 8), 0 findings.**
- Checks per render: console errors and exceptions; sideways overflow of every chrome container (toolbars, find bars and fields, strips, note rows, tabs, capsule, menus, popovers, the value viewer, the go-to popover, the inspector header, settings rows, chips, the crumb); chrome text that ellipsizes (data exempt: cells, names, crumbs, tab titles, the value viewer's title); floating layers inside the window and off the title bar and status bar; exactly one cp-fill block when the focused console is production, none otherwise; the 11 px floor; `text-3` on `raised` in dark; `NaN`, `undefined`, unfilled placeholders and raw Fluent ids in text and tooltips; every trust signal (capsule block, status environment segment, transaction segment, badge, mode chip — which must also show its mode word —, read-only chips, crumb marks, tab swatches, run states, Reduced mode, Cursor open) drawn whole inside every clipping ancestor; the production hairline on every production editor and pane; Danger only in dialogs; an alertdialog opening on its safe answer; no duplicated Fluent id across PRIOR / M3a / M3b / app / M3c; the value viewer and the go-to popover holding the focus.
- Regression: surfaces 1–13 were rendered from the M3b file and from this file (186 renders each: every scenario in EN · dark · 1280 and TR · light · 960) and compared pixel by pixel: after fix round 1, 54 of the 186 pairs differ, each only where a disabled control is drawn (Execute before the typed name, Run while a prompt or the error bar is up, the parameter prompt's Run, Count rows while counting, the history footer with no entry, the read-only mode chip, menu items disabled with a reason) or a keyboard-active menu row (the mode, isolation, tab, chooser and status menus): rulings M3c-36 and M3c-38. Every other pixel is identical; the checker flags the same two renders on both files (the template scenario literally shows `${tablo}`, data).
- The PNGs were looked at (EN · dark · 1280 and TR · light · 960, crops at 3× where detail mattered). That review caught, and the design fixed: a pinyin composition with an apostrophe lexed as a string (now `shuju`); a mock note over the candidate window; the 960 px narrow-pane layout leaving a 19 px fifth pane (now the widest pane fills); the badge's short form clipped in a 120 px pane (now wraps); the strip's Fetch more wider than a 120 px pane (now an icon); the width label over the strip; grid find's current match and the unsorted-tail boundary scrolled out of view; a gutter fixture whose end-of-line status ran under long text; the value viewer's subtitle spaced by its flex gap; the resize guide not drawn; an undo-selection fixture selecting a word that wasn't there; the keyboard-active menu row invisible in dark (ruling M3c-36).
- **Fix round 1** (design review: 1 C / 11 I / 12 M; controller rulings): the narrow mode chip keeps its word (and the checker now fails a chip without its mode word); the disabled ink token and the keyboard-active menu row (system-wide); Filter rows as a note row with the strip kept; partial + tail in one note; grid Select all occurrences; the narrow status bar following the focused pane (panes are console tiles); hex at 8 bytes per row with a drawn thumb; ▶ hidden while Run is off and never over M3b marks; the fold severity order with M3b marks; progress states for the go-to fetch and the text build; the narrow relaxations named N1–N4 ("read-only (guarded)" whole, badge buttons kept); and every minor (main-caret Run scenario, popovers above the strip, `hui-dialog-close`, EN singulars, one name for ⌃⌘G, M2a button order, NOTES drift, the history note, XML / invalid JSON / code-point / Unfreeze / KB / Task 41 carries drawn, the IME note, the text view's scroll cue, F6, TR nits).
- **Fix round 2** (re-check R1-I1, R1-M1…M3): the value viewer and the go-to popover stay below the console toolbar (the well shrinks to 120 px first, then scrolls) and the checker now fails any popover over a console toolbar; one token name, `text-disabled`; the narrow badge keeps its stamp on the label's first line; Task 37 owns a grid command id (Plan notes). Surfaces 20 and 22 re-rendered in all eight variants (128 renders), surface 24 in all eight (80), and every M3c surface re-checked in EN · dark · 1280 and TR · light · 960 (200 renders): 0 findings.
- **M3b surfaces after round 1:** the regression line above is the round-1 result; Task 41 lists these shots for re-baselining (Plan notes).
- The copy table below is generated from the mock's `i18n-m3c` block (156 ids), so the two cannot drift.

### Copy strings (EN / TR), M3c

Placeholders, plurals and isolation follow the M3b table's rules. The `i18n-app` block reuses, verbatim, strings the app already ships (Task 1b's command titles, `hv-console-ro-pending`, `hv-console-ro-pending-short`, the Settings page strings); they are listed in the mock, not here, and M3c redefines none of them.

| Prefix | File | Owner (own `## m3b-tNN-*` block) |
|---|---|---|
| `hv-editor-carets`, `hv-editor-ime-*` | `locales/{en,tr}/hacivat-ui.ftl` | Task 31 |
| `hv-editor-fold-*`, `hv-editor-unfold-name` | `locales/{en,tr}/hacivat-ui.ftl` | Task 32 |
| `hv-find-*` | `locales/{en,tr}/hacivat-ui.ftl` | Task 33 (`## m3b-t33-find`; Task 37 reuses them) |
| `hv-gutter-*`, `hv-eol-*`, `hv-history-recall` | `locales/{en,tr}/hacivat-ui.ftl` | Task 34 (`## m3b-t34-editor`) |
| `hv-grid-frozen-tip`, `hv-grid-resize-tip`, `hv-grid-width`, `hv-strip-ranges` | `locales/{en,tr}/hacivat-ui.ftl` | Task 35 |
| `hv-grid-sort*`, `hv-grid-sorted-*`, `hv-grid-clear-sort`, `hv-grid-unsorted-tail`, `hv-grid-resort`, `hv-grid-dir-*` | `locales/{en,tr}/hacivat-ui.ftl` | Task 36 (`## m3b-t36-sort`) |
| `hv-gfind-*`, `hv-goto-*` | `locales/{en,tr}/hacivat-ui.ftl` | Task 37 (`## m3b-t37-find-goto`) |
| `hv-view-*`, `hv-transposed-col`, `hv-record*`, `hv-text-*`, `hv-vv-*` | `locales/{en,tr}/hacivat-ui.ftl` | Task 38 (`## m3b-t38-views`) |
| `hv-gmenu-*` | `locales/{en,tr}/hacivat-ui.ftl` | Task 39 (`## m3b-t39-grid-menu`) |
| `hv-console-waiting-l1`, `hv-console-waiting-l2` | `locales/{en,tr}/hacivat-ui.ftl` | Task 41 |
| `hv-settings-*` | `locales/{en,tr}/hacivat-ui.ftl` | Task 42 (`## m3b-t42-settings`) |

| Id | EN | TR | Status |
|---|---|---|---|
| `hv-tx-chip-auto-short` | Auto | Otomatik | Review: PO |
| `hv-editor-carets` | { $n } carets | { $n } imleç | Review: PO |
| `hv-editor-ime-primary-only` | Input method: typing at the main caret only, because another caret was inside a string or comment | Giriş yöntemi: başka bir imleç bir dizenin ya da yorumun içinde olduğu için yalnızca ana imleçte yazılıyor | Review: PO |
| `hv-editor-fold-lines` | { $n } lines (one: { $n } line) | { $n } satır | Review: PO |
| `hv-editor-fold-tip` | Lines { $from }–{ $to } are folded. Click to unfold. | { $from }–{ $to }. satırlar katlandı. Açmak için tıklayın. | Review: PO |
| `hv-editor-fold-name` | Fold lines { $from }–{ $to } | { $from }–{ $to }. satırları katla | Review: PO |
| `hv-editor-unfold-name` | Unfold lines { $from }–{ $to } | { $from }–{ $to }. satırların katlamasını aç | Review: PO |
| `hv-editor-fold-failed` | failed inside the fold · line { $line } | katlanmış bölümde başarısız · satır { $line } | Review: PO |
| `hv-find-placeholder` | Find | Bul | Review: PO |
| `hv-find-match-case` | Match case | Büyük/küçük harf eşleşsin | Review: PO |
| `hv-find-words` | Whole words | Tam sözcük | Review: PO |
| `hv-find-regex` | Regular expression | Düzenli ifade | Review: PO |
| `hv-find-tolerant` | Turkish-tolerant | Türkçe esnek | Review: PO |
| `hv-find-tolerant-tip` | Turkish-tolerant: İ, I, ı and i find each other, and accents are ignored. | Türkçe esnek: İ, I, ı ve i birbirini bulur, aksanlar yok sayılır. | Review: PO |
| `hv-find-count` | { $i } of { $n } | { $i }/{ $n } | Review: PO |
| `hv-find-count-more` | { $i } of { $n }+ | { $i }/{ $n }+ | Review: PO |
| `hv-find-counting-tip` | Still counting; the number grows as the rest of the document is searched. | Sayım sürüyor; belgenin geri kalanı arandıkça sayı artar. | Review: PO |
| `hv-find-none` | No matches | Eşleşme yok | Review: PO |
| `hv-find-invalid` | Invalid pattern | Geçersiz desen | Review: PO |
| `hv-find-invalid-tip` | The regular expression can't be read, so nothing is searched: { $error } | Düzenli ifade okunamadığı için hiçbir şey aranmıyor: { $error } | Review: PO |
| `hv-find-prev` | Previous match | Önceki eşleşme | Review: PO |
| `hv-find-next` | Next match | Sonraki eşleşme | Review: PO |
| `hv-find-capped` | Selected the first { $max } of { $n } matches | { $n } eşleşmenin ilk { $max } tanesi seçildi | Review: PO |
| `hv-gutter-run` | Run this statement | Bu ifadeyi çalıştır | Review: PO |
| `hv-gutter-last-runs` | Last runs | Son çalıştırmalar | Review: PO |
| `hv-gutter-never-run` | Not run yet. | Henüz çalıştırılmadı. | Review: PO |
| `hv-gutter-same-path` | Runs through the same checks as Run. | Çalıştır ile aynı denetimlerden geçer. | Review: PO |
| `hv-eol-mixed` | Mixed line endings | Karışık satır sonları | Review: PO |
| `hv-eol-mixed-tip` | Lines end in both LF and CRLF. Click to normalize them. | Satırlar hem LF hem CRLF ile bitiyor. Eşitlemek için tıklayın. | Review: PO |
| `hv-eol-counts` | Line endings: LF { $lf }, CRLF { $crlf } | Satır sonları: LF { $lf }, CRLF { $crlf } | Review: PO |
| `hv-eol-to` | Normalize to { $ending } | Hepsini { $ending } yap | Review: PO |
| `hv-eol-most` | most lines | çoğu satır | Review: PO |
| `hv-history-recall` | From history: statement { $i } of { $n } · { $older } older · { $newer } newer | Geçmişten: ifade { $i }/{ $n } · { $older } daha eski · { $newer } daha yeni | Review: PO |
| `hv-grid-sorted-locally` | Sorted locally: the database might order differently. | Yerel olarak sıralandı: veritabanı farklı sıralayabilir. | Review: PO |
| `hv-grid-sorted-locally-more` | Only the { $rows } fetched rows are sorted. | Yalnızca getirilen { $rows } satır sıralandı. | Review: PO |
| `hv-grid-clear-sort` | Clear sort | Sıralamayı kaldır | Review: PO |
| `hv-grid-unsorted-tail` | { $rows } new rows at the end (not sorted) (one: { $rows } new row at the end (not sorted)) | Sonda { $rows } yeni satır (sıralanmadı) | Review: PO |
| `hv-grid-resort` | Re-sort | Yeniden sırala | Review: PO |
| `hv-grid-sorting` | Sorting { $rows } rows… (one: Sorting { $rows } row…) | { $rows } satır sıralanıyor… | Review: PO |
| `hv-grid-sorted-tail` | Only the { $sorted } fetched rows are sorted; { $rows } new rows at the end are not. (one: Only the { $sorted } fetched rows are sorted; { $rows } new row at the end is not.) | Yalnızca getirilen { $sorted } satır sıralandı; sondaki { $rows } yeni satır sıralanmadı. | Review: PO |
| `hv-grid-sort-cancel` | Cancel | Vazgeç | Review: PO |
| `hv-grid-sort-tip` | Sort level { $lvl }, { $dir }. Text compares by the column's collation, { $collation }. | Sıralama düzeyi { $lvl }, { $dir }. Metin, sütunun harmanlamasına ({ $collation }) göre karşılaştırılır. | Review: PO |
| `hv-grid-sort-tip-cp` | Sort level { $lvl }, { $dir }. The column's collation { $collation } has no equivalent here, so text compares by code point. | Sıralama düzeyi { $lvl }, { $dir }. Sütunun { $collation } harmanlamasının burada karşılığı yok; metin kod noktasına göre karşılaştırılır. | Review: PO |
| `hv-grid-dir-asc` | ascending | artan | Review: PO |
| `hv-grid-dir-desc` | descending | azalan | Review: PO |
| `hv-grid-frozen-tip` | Frozen: stays in place when you scroll sideways | Donduruldu: yana kaydırınca yerinde kalır | Review: PO |
| `hv-grid-resize-tip` | Drag to resize · double-click to fit | Boyutlandırmak için sürükleyin · sığdırmak için çift tıklayın | Review: PO |
| `hv-grid-width` | { $px } px | { $px } px | Review: PO |
| `hv-strip-ranges` | { $n } ranges · { $cells } cells | { $n } aralık · { $cells } hücre | Review: PO |
| `hv-gfind-placeholder` | Find in the shown values | Görünen değerlerde bul | Review: PO |
| `hv-gfind-filter` | Filter rows | Satırları süz | Review: PO |
| `hv-gfind-filter-tip` | Filter rows: show only the rows with a match | Satırları süz: yalnızca eşleşen satırları göster | Review: PO |
| `hv-gfind-incomplete` | may be incomplete | eksik olabilir | Review: PO |
| `hv-gfind-incomplete-tip` | { $n } values were cut at { $size }; text past the cut isn't searched. (one: { $n } value was cut at { $size }; text past the cut isn't searched.) | { $n } değer { $size } sınırında kesildi; kesilen kısım aranmaz. | Review: PO |
| `hv-gfind-filter-note` | Showing the { $shown } rows that match, of the { $rows } fetched. Rows not fetched yet aren't filtered. | Getirilen { $rows } satırdan eşleşen { $shown } satır gösteriliyor. Henüz getirilmeyen satırlar süzülmez. | Review: PO |
| `hv-gfind-show-all` | Show all rows | Tüm satırları göster | Review: PO |
| `hv-gfind-null-tip` | Searches what the grid shows, row by row from the focused cell. NULL never matches. | Tablonun gösterdiği değerlerde, odaktaki hücreden başlayarak arar. NULL hiçbir zaman eşleşmez. | Review: PO |
| `hv-gfind-close` | Close find | Bulmayı kapat | Review: PO |
| `hv-goto-title` | Go to row | Satıra git | Review: PO |
| `hv-goto-placeholder` | row or row:column | satır ya da satır:sütun | Review: PO |
| `hv-goto-hint` | A row number, then an optional column by name or position: 1500:city or 1500:3 | Satır numarası, ardından isteğe bağlı sütun adı ya da sırası: 1500:city ya da 1500:3 | Review: PO |
| `hv-goto-target` | Row { $row }, column { $col } | { $row }. satır, { $col } sütunu | Review: PO |
| `hv-goto-target-row` | Row { $row } | { $row }. satır | Review: PO |
| `hv-goto-past-end` | There's no row { $row }: the result has { $total } rows. | { $row }. satır yok: sonuçta { $total } satır var. | Review: PO |
| `hv-goto-unknown-col` | No column is named { $name }. | { $name } adında bir sütun yok. | Review: PO |
| `hv-goto-hidden-col` | Column { $name } is hidden. | { $name } sütunu gizli. | Review: PO |
| `hv-goto-show-col` | Show it and go | Gösterip git | Review: PO |
| `hv-goto-fetch` | Row { $row } isn't fetched yet. Fetching { $more } more rows takes a while, and the result's memory limit still applies. | { $row }. satır henüz getirilmedi. { $more } satır daha getirmek zaman alır; sonucun bellek sınırı yine geçerlidir. | Review: PO |
| `hv-goto-fetching` | Fetching to row { $row }: { $done } of { $more } rows | { $row }. satıra kadar getiriliyor: { $more } satırın { $done } tanesi | Review: PO |
| `hv-goto-fetching-note` | Escape hides this; fetching goes on in the strip. | Esc bunu gizler; getirme şeritte sürer. | Review: PO |
| `hv-goto-stop` | Stop fetching | Getirmeyi durdur | Review: PO |
| `hv-goto-fetch-ok` | Fetch and go | Getir ve git | Review: PO |
| `hv-goto-cancel` | Cancel | Vazgeç | Review: PO |
| `hv-goto-key-go` | Go | Git | Review: PO |
| `hv-view-table` | Table | Tablo | Review: PO |
| `hv-view-transposed` | Transposed | Devrik | Review: PO |
| `hv-view-text` | Text | Metin | Review: PO |
| `hv-view-label` | View | Görünüm | Review: PO |
| `hv-view-transposed-off` | Transposed shows up to { $max } rows; this result has { $rows }. | Devrik görünüm en çok { $max } satır gösterir; bu sonuçta { $rows } satır var. | Review: PO |
| `hv-transposed-col` | Column | Sütun | Review: PO |
| `hv-record` | Record | Kayıt | Review: PO |
| `hv-record-tip` | Show the focused row as a record | Odaktaki satırı kayıt olarak göster | Review: PO |
| `hv-record-pos` | Row { $i } of { $n } | Satır { $i }/{ $n } | Review: PO |
| `hv-record-prev` | Previous row | Önceki satır | Review: PO |
| `hv-record-next` | Next row | Sonraki satır | Review: PO |
| `hv-record-show-more` | Show more | Devamını göster | Review: PO |
| `hv-record-dup-tip` | The result has { $n } columns named { $name }; this is number { $i }. | Sonuçta { $name } adında { $n } sütun var; bu { $i }. sütun. | Review: PO |
| `hv-text-big-title` | Show { $rows } rows as text? | { $rows } satır metin olarak gösterilsin mi? | Review: PO |
| `hv-text-big-body` | Building the text view for this many rows takes a while and uses about { $size } of memory. | Bu kadar satır için metin görünümünü oluşturmak zaman alır ve yaklaşık { $size } bellek kullanır. | Review: PO |
| `hv-text-building` | Building the text view: { $done } of { $rows } rows | Metin görünümü oluşturuluyor: { $rows } satırın { $done } tanesi | Review: PO |
| `hv-text-cancel` | Cancel | Vazgeç | Review: PO |
| `hv-text-big-ok` | Show as text | Metin olarak göster | Review: PO |
| `hv-text-big-back` | Back to the table | Tabloya dön | Review: PO |
| `hv-vv-sub` | { $type } · row { $row } | { $type } · { $row }. satır | Review: PO |
| `hv-vv-raw` | Raw | Ham | Review: PO |
| `hv-vv-pretty` | Pretty | Biçimli | Review: PO |
| `hv-vv-hex` | Hex | Onaltılık | Review: PO |
| `hv-vv-pretty-tip` | Display only: Copy raw value copies the server's text unchanged. | Yalnızca görünüm: Ham değeri kopyala, sunucunun metnini değiştirmeden kopyalar. | Review: PO |
| `hv-vv-chars` | { $size } · { $n } characters (one: { $size } · { $n } character) | { $size } · { $n } karakter | Review: PO |
| `hv-vv-bytes` | { $n } bytes (one: { $n } byte) | { $n } bayt | Review: PO |
| `hv-vv-truncated` | Showing the first { $shown } of { $size }. | Değerin ilk { $shown } kısmı gösteriliyor (toplam { $size }). | Review: PO |
| `hv-vv-rerun` | Re-run with a { $limit } value limit | { $limit } değer sınırıyla yeniden çalıştır | Review: PO |
| `hv-vv-rerun-tip` | Runs this statement again with a larger limit for every value. It goes through the same checks as Run. | Bu ifadeyi her değer için daha büyük bir sınırla yeniden çalıştırır. Çalıştır ile aynı denetimlerden geçer. | Review: PO |
| `hv-vv-copy` | Copy raw value | Ham değeri kopyala | Review: PO |
| `hv-vv-not-json` | Not valid JSON, so Pretty is off and the value shows as the server sent it. | Geçerli JSON değil; Biçimli kapalı, değer sunucunun gönderdiği gibi gösteriliyor. | Review: PO |
| `hv-gmenu-copy` | Copy | Kopyala | Review: PO |
| `hv-gmenu-copy-headers` | Copy with headers | Başlıklarla kopyala | Review: PO |
| `hv-gmenu-copy-rows` | Copy rows | Satırları kopyala | Review: PO |
| `hv-gmenu-copy-name` | Copy column name | Sütun adını kopyala | Review: PO |
| `hv-gmenu-open-value` | Open value viewer | Değer görüntüleyiciyi aç | Review: PO |
| `hv-gmenu-sort-asc` | Sort ascending | Artan sırala | Review: PO |
| `hv-gmenu-sort-desc` | Sort descending | Azalan sırala | Review: PO |
| `hv-gmenu-add-sort` | Add as sort level { $lvl } | { $lvl }. sıralama düzeyi olarak ekle | Review: PO |
| `hv-gmenu-freeze` | Freeze up to here | Buraya kadar dondur | Review: PO |
| `hv-gmenu-unfreeze` | Unfreeze columns | Dondurmayı kaldır | Review: PO |
| `hv-gmenu-hide` | Hide column | Sütunu gizle | Review: PO |
| `hv-gmenu-show-all` | Show all columns | Tüm sütunları göster | Review: PO |
| `hv-gmenu-autofit` | Fit width | Genişliği sığdır | Review: PO |
| `hv-gmenu-raw` | Raw display | Ham görünüm | Review: PO |
| `hv-gmenu-raw-hint` | this column | bu sütun | Review: PO |
| `hv-gmenu-goto` | Go to row… | Satıra git… | Review: PO |
| `hv-gmenu-record` | Show as record | Kayıt olarak göster | Review: PO |
| `hv-gmenu-why-one-cell` | Select one cell | Tek hücre seçin | Review: PO |
| `hv-gmenu-why-not-sorted` | Not sorted | Sıralı değil | Review: PO |
| `hv-gmenu-why-no-hidden` | None hidden | Gizli sütun yok | Review: PO |
| `hv-gmenu-why-ranges` | Ranges don't line up | Aralıklar hizalı değil | Review: PO |
| `hv-gmenu-why-last` | Last shown column | Görünen son sütun | Review: PO |
| `hv-console-waiting-l1` | Waiting for a connection | Bağlantı bekleniyor | Review: PO |
| `hv-console-waiting-l2` | { $inUse } of { $max } in use | kullanımda: { $inUse }/{ $max } | Review: PO |
| `hv-settings-nav-editor` | Editor | Düzenleyici | Review: PO |
| `hv-settings-nav-execution` | Execution | Çalıştırma | Review: PO |
| `hv-settings-nav-results` | Results | Sonuçlar | Review: PO |
| `hv-settings-nav-history` | History | Geçmiş | Review: PO |
| `hv-settings-section-editor` | EDITOR | DÜZENLEYİCİ | Review: PO |
| `hv-settings-section-execution` | EXECUTION | ÇALIŞTIRMA | Review: PO |
| `hv-settings-section-results` | RESULTS | SONUÇLAR | Review: PO |
| `hv-settings-section-history` | HISTORY | GEÇMİŞ | Review: PO |
| `hv-settings-on` | On | Açık | Review: PO |
| `hv-settings-off` | Off | Kapalı | Review: PO |
| `hv-settings-tab-size` | Tab size | Sekme genişliği | Review: PO |
| `hv-settings-insert-spaces` | Indent with spaces | Boşlukla girintile | Review: PO |
| `hv-settings-copy-line` | Copy the line when nothing is selected | Seçim yokken satırı kopyala | Review: PO |
| `hv-settings-caret-blink` | Blinking caret | Yanıp sönen imleç | Review: PO |
| `hv-settings-confirm-unsafe-where` | Confirm UPDATE or DELETE without WHERE off production | Üretim dışında WHERE içermeyen UPDATE ya da DELETE için onay iste | Review: PO |
| `hv-settings-confirm-unsafe-where-note` | Production always asks, with the table's name typed. | Üretim her zaman sorar ve tablonun adını yazdırır. | Review: PO |
| `hv-settings-error-policy` | Ask what to do when a statement fails | Bir ifade başarısız olunca ne yapılacağını sor | Review: PO |
| `hv-settings-error-policy-note` | Off: a run of several statements stops at the first failure. | Kapalı: birden çok ifadelik çalıştırma ilk hatada durur. | Review: PO |
| `hv-settings-max-value` | Largest value kept per cell | Hücre başına saklanan en büyük değer | Review: PO |
| `hv-settings-max-value-note` | From 64 KB to 64 MB. Larger values are cut; the value viewer can re-run with the largest limit. | 64 KB ile 64 MB arası. Daha büyük değerler kesilir; değer görüntüleyici en büyük sınırla yeniden çalıştırabilir. | Review: PO |
| `hv-settings-max-value-clamped` | { $typed } is above the largest allowed, so it was set to { $max }. | { $typed } izin verilen en büyük değerin üstünde; { $max } yapıldı. | Review: PO |
| `hv-settings-unit-kb` | KB | KB | Review: PO |
| `hv-settings-unit-mb` | MB | MB | Review: PO |
| `hv-settings-raw-display` | Show grid values raw | Tablo değerlerini ham göster | Review: PO |
| `hv-settings-raw-display-note` | Numbers and dates as the server sent them, without regional formatting. Copy is always raw. | Sayılar ve tarihler bölgesel biçim olmadan, sunucunun gönderdiği gibi. Kopyalama her zaman hamdır. | Review: PO |
| `hv-settings-history` | Record query history | Sorgu geçmişini kaydet | Review: PO |
| `hv-settings-history-note` | Up to 10,000 statements per connection are kept for 90 days; recognised passwords and secrets are masked. | Bağlantı başına en çok 10.000 ifade 90 gün saklanır; tanınan parolalar ve gizli bilgiler maskelenir. | Review: PO |
