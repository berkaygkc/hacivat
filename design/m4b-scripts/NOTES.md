# M4b "Script workspace (local)": design notes

**Status: awaiting review (fix round 1 applied).** Task 2 Steps 1–2 (mocks and these notes) are done, and the design review's findings and the controller's rulings (a)–(e) are in (see "Fix round 1" under Open questions and rulings); the controller reviews against DESIGN.md (PO away) and the PO signs off at the gate. Tasks 13–18 and 21 build from these notes once approved (plan "New UI surfaces": "binding for every UI task once the controller or the PO approves").

Clickable mock: `design/m4b-scripts/index.html` (one file, one app, a surface switcher). It is the M4 mock's world, unchanged: the same CSS (copied, with the M4b additions after it), the same Lucide 0.460 sprite (ISC) plus two icons, the same mock bar, Notes drawer, toggles and deep links. It runs with no network except Google Fonts. The M2a/M2b strings (`i18n-prior`), the shipped Fluent strings (`i18n-app`, `i18n-app-m4b`) and the M4 copy table (`i18n-m4`, shipped in `locales/`) are reused verbatim; M4b never redefines an earlier Fluent id.

**How to drive it**
- The dashed **mock bar** is not product UI. Row 1 holds the thirteen surfaces (1–12 and the pre-flight's 6b). Row 2 holds the scenarios of the current surface (`[` / `]`), the window size (Fit, 1440 × 900, 1280 × 800, 960 × 600; `W`), EN/TR (`L`), dark/light (`T`) and Notes (`N`).
- Keys `1`–`9`, `0`, `-`, `=` jump to the first twelve entries of the bar; the bar reaches all. They work while no field or the tree has the focus.
- Deep links: `?s=8&sc=chooser&theme=light&lang=tr&size=960&bar=0` (`s` is the bar position, 1-based; 6b is position 7). `bar=0` hides the mock bar. `shot=1` freezes timers for screenshots. `check=all` runs the verification pass (see Verification).
- Every scenario named "960 · TR" pins Turkish at 960 × 600; an explicit `lang` or `size` in the link wins.
- Numbers and dates follow the language toggle as a stand-in for the format locale (`1,204` / `1.204`, `Nov 21, 14:05` / `21 Kasım 14:05`). The product follows the OS region (L3, L15(d)).
- The mock shows macOS words and chords; the Windows/Linux rows are in the key table below.

**Flows that run**
- **Tree.** Click selects; chevron or double-click toggles; ↵ or a double-click on a file opens a script tab bound to the connection and database the row is under; ↵ on the problems row opens the diagnostics popover (Escape returns to the row). Right-click (or ⇧F10) opens the menus; Rename and New script edit the name in the row with live validation; Move to Trash removes the file and says so.
- **Tabs.** Click switches (the capsule, hairline, crumb and status bar follow); right-click opens the tab menu; closing a dirty tab asks (6b); "Open under another connection…" opens the chooser without the tab's own connection, then opens the script under the chosen one in its own tab (or focuses the tab already open there).
- **Changes on disk.** Reload asks first; Keep mine removes the notice and the next ⌘S asks Overwrite?; Save on a tab with secrets asks first.
- **Go to script.** ⇧⌘O opens it; type (Turkish-tolerant); ↓↑ ↵; Escape goes back from the chooser to the results, then closes.
- **Attach.** Typing the database name or aliases rewrites the file shown; an empty name disables Attach. The mapping dialog's "share as alias" box shows the line it would add.
- **Settings.** Search filters libraries; Remove… asks; Attach a folder… opens the attach dialog.
- **Encoding.** Open / Open read-only; the encoding Select and the status segment open their menus.
- **Toasts.** F6 moves the focus into the newest toast (its buttons become Tab stops only then).

**Product truth used:** the rewritten plan (`plan-m4b.md` "New UI surfaces" 1–12 and 6b, Task 2, the Global Constraints "Design rules", "Keys" and "The tab's connection…", rulings R-M4b-3 to -46), `po-questions-m4b.md` (Q1–Q23 defaults in force), `preflight-m4b.md` U-2 and `carries-w0.md` "T2", the shipped M4 explorer (`crates/app/hacivat-app/src/explorer/{contrib,actions,badges}.rs`, `crates/ui/hacivat-ui/src/tree/mod.rs` row slots and `tree/view/row.rs` status colours, `crates/ui/hacivat-ui/src/palette.rs`), the shipped keymaps and AltGr layers (`crates/ui/hacivat-ui/keymaps/`), the shipped Fluent files, DESIGN.md (Named Rules, Explorer Tree, the M2–M4 component sections), `design/m4-explorer/` (the base world and its review), `design/m3b-safe/` and `design/m3a-query/NOTES.md:169-171` (the T2 and paste sketches).

**Pre-flight carry (U-2) applied:** (1) surface 1 uses only M4's row slots (badge = "shared", status = git, meta = count, dim, tooltip); (2) the Scripts items are appended after M4's database menu items, and the SQLite root menu gets them; (3) surface 6b, the unsaved-changes prompt; (4) the Run-refusal notice (R-M4b-27, R-M4b-30) on surface 5; (5) no Save-all chord on Windows/Linux (R-M4b-37: ⇧⌘O / Ctrl+Shift+O for Go to script, ⌘S / Ctrl+S, ⌥⌘S macOS only; Reveal has no chord); (6) Go to script is its own palette layer; (7) no Scripts node under SQLite attached databases; (8) focus order per R-T33-1 (buttons are Tab stops; listed per surface). Controller rule R-C0-4: the secret mock holds no vendor-shaped token (the token line reads `ghp_••••••••`).

---

## Cross-cutting decisions

1. **No new hue, no new filled block.** Red: production marks (the root's and an `@production` folder's 7 px swatch, the production tab's mark, the capsule, the status segment, the editor hairline) and destructive answers (Delete… on a connection, "Discard and reload", "Overwrite"). Amber: an uncommitted change (git M and N) or something running (the static 7 px run marker: downloading, switching, Run as script). Green: a check that something succeeded. Blue: selection, focus, the primary button, a highlighted match. Every refusal, problem, conflict or unknown is an ink glyph and words. The capsule's cp-fill block stays the window's one filled badge (checked in every render). The "shared" badge and git glyphs are never filled and never red.
2. **No ambient motion.** The watcher never animates anything; a reload is a one-line status message for 3 s; progress is the static run marker and counts.
3. **The Raised Ink Rule holds.** Menus, tooltips, toasts, dialogs, the palette, toolbars and strips map text-3 to text-2; the explorer and the notices' field wells keep text-3 where allowed. Disabled items use the shipped `text-disabled`.
4. **Names are user data.** File, folder, library and connection names are shown verbatim (NFC for display, bidi-isolated, invisible characters escaped as `<U+XXXX>`, R-M4b-23), in mono where they are literal; Turkish names (`müşteri_segment.sql`, `haftalık_rapor.sql`) sort by the UI collator; counts are Number arguments formatted in the format locale.
5. **Where you open a script decides where it runs.** Every surface that shows a script tab names its connection: the tab title (whose connection part never truncates, ruling d), the crumb, the capsule, the status segment, the tab tooltip, every dialog subtitle. No surface offers to switch a tab's connection. One tab per (file, connection) (R-M4b-4): every path that opens a script under a connection where it is already open focuses that tab instead of opening a second buffer.
6. **Production is never chosen for the user.** The chooser never preselects it unless it is the **active tab's** connection (the "focused connection" is always the active tab's, never a tree row's); "Open under another connection…" never offers the connection it starts from; a file that only production shows opens nothing on ↵ until the user moves onto it.
7. **The safe answer is the default.** Every alertdialog opens on Cancel (Primary): reload, overwrite, secret warning, paste, remove, unsaved changes. A notice's buttons are Default.
8. **Focus (R-T33-1, R-M4b-43).** Buttons are Tab stops (notices, strips, dialogs, popovers, settings rows); toast buttons only after F6. When a pressed control disappears (a notice's button that removes the notice, a strip's Close, a dismissed toast, a closed dialog), the focus returns to the surface's main area: the tab's code area, or the tree for tree surfaces. Every notice's focus home is the code area of its tab.
9. **GPUI-reproducible.** Solid fills, 1 px borders, radii 0/3/4/6, the popover shadow only; no transforms, filters, blur or masks. Every tree row is the shipped 23 px row.
10. **Fits 960 × 600 in Turkish.** Every surface has a "960 · TR" scenario, checked in dark and light (see Verification).

---

## Surfaces

### 1 · Scripts node (HV-SCRIPT-05)
**Purpose.** A bound library shows as a Scripts node under every connection that shows its database, so where the user opens a script is where it runs (L14).

**Anatomy** (the shipped row, `TreeRowData`: chevron · icon · name · type text · badge · flags · status · pill · spacer · meta mark + meta).
- **One row tooltip** (`TreeRowData::tooltip`, carry T2 item 1): the shipped row has no per-cell tooltips (a badge showing its text has none, `RowStatus.text` is an accessible description, `RowFlag::Env` has no text), so every fact a cell carries is folded into the row's one tooltip, which is also its accessible description.
- **Scripts row:** icon `scroll-text`; label "Scripts" (Fluent); **badge** "shared" (outlined, micro 11.5, text-2, lowercase: controller ruling b; only when the library shows under at least one other connection node); **meta** the file count in mono (`13`). No pill, no status. Row tooltip: "13 scripts listed under this connection." / "Also under orders-staging and orders-dev: every connection with a database of this name shows the same files."
- **Placement** (R-M4b-25, -31, -32): first child of a PostgreSQL database row (only the session's or profile's database), of every MySQL/MariaDB database row, and of a SQLite profile root above `main` (never under an attached database). A profile without a catalog yet shows none (Go to script still offers it).
- **One library:** its folders and files directly. **Several:** one folder row per library (icon `folder`, the folder name, its own count as meta; row tooltip "Folder ~/work/… . Binds to the database orders.").
- **Folders** (icon `folder`) before files, each sorted by the UI collator. An `@<tag>` folder carries its environment mark in **flags** (red square only for `@production`); row tooltip "Only connections tagged production see this folder." Folders that hold no listed script are not shown; an unknown `@name` is never listed.
- **Files:** `file-code-2` for `.sql`, `file-text` for `.md`. **Status** (`RowStatus { kind, glyph, text }`): M (modified) and N (new) in amber, C (conflicted) in ink, ? (unknown) in text-3, `mono_micro` 11 at weight 400 (controller ruling a); a folder that contains changes shows `•` in text-2. The text alternative is the row's accessible description and a line of the row tooltip ("M Modified: differs from the last commit"). Clean and ignored files have none.
- **Disabled rows** (`dim`): a link pointing outside the library (icon `link`, text-3; row tooltip "Points outside the library, so Hacivat never opens it."; Open disabled). A cloud placeholder: the meta mark only, `MetaMark::Glyph(Download)` (row tooltip "Kept in the cloud, not downloaded. Opening it downloads it first." plus the git `?` line); while downloading, `MetaMark::Running` only (row tooltip "Downloading… The tab shows the text when the download ends."), and its tab shows the downloading notice instead of text (Run disabled). **No words in the meta slot** (review I4): the shipped meta ellipsizes first, so a state word there would be cut.
- **Problems row:** the library's first child when it has diagnostics: the ink `circle-alert` and "2 problems"; a cursor stop; ↵ or a click opens surface 11's popover.
- **Folder missing:** the Scripts node's child is the shipped failed row "Folder not found" with the Retry icon (tooltip names the path).

**States.** Under orders-prod (`@production` listed) · under orders-staging (`@staging` listed, `@production` not) · two libraries (library row tooltip) · outside link + downloading (link row tooltip) · Scripts row tooltip · file row tooltip · MySQL (under `kampanya`, not the default `api`) · SQLite (above `main`) · keyboard cursor · folder row tooltip (`@production`) · 960 × 600 in Turkish.

**Keyboard.** The shipped tree keys: ↑ ↓, → expands or enters, ← collapses or goes to the parent, ↵ opens a file (or the popover), ⇧F10 / the menu key opens the menu, F5 refreshes. Type-ahead and the filter match file names (`ContributedNode::filter_text`). No new chord (R-M4b-37).

**Focus order.** The tree is one Tab stop (the shipped `TreeView`); the problems row is a cursor stop inside it; the Retry icon is pointer-only, its keyboard path is Refresh (as M4).

**Out of scope.** Drag and drop (v1.x); a "Scripts" node with no library (Q9); file templates; diff of changed files.

### 2 · Context menus (R-M4b-18, R-M4b-34)
**Purpose.** The ways in (attach, map) on the database node, and the file operations in the tree.

**Anatomy** (the shipped PopupMenu: no icon column; a disabled row keeps its reason as the hint; no chords except the shipped ones).
- **Database row** (PG and MySQL/MariaDB): M4's items unchanged (Refresh F5, Edit scope…, Introspection level › | Copy qualified name ⌥⌘C, Expand all children, Collapse all children), then, after all of them and a separator (R-M4b-34): **Attach script library…**, **Map to a script library…** (disabled with "No other library is attached" when every library already shows here). **SQLite root:** the M2a connection items, then a separator and the same two, then Delete… last (ruling 17: the destructive row stays last; R-M4b-34's "append" reads "before the destructive group" for the SQLite root).
- **Scripts node / library row:** New script, New folder | Refresh F5, Reveal in Finder, Copy path | Script library settings…, Collapse all children. With several libraries the Scripts node keeps only Refresh, settings and Collapse; New… lives on each library row.
- **Folder:** New script, New folder | Rename, Move to Trash | Reveal in Finder, Copy path, Copy relative path | Refresh.
- **File:** Open ↵, Open under another connection… | Rename F2, Move to Trash | Reveal in Finder, Copy path, Copy relative path. **Link outside:** Open (disabled: "Points outside the library") | Reveal, Copy path, Copy relative path.
- **Open under another connection…** (review Critical 1, I1): the chooser's focused connection is the **active tab's**, never the row's; the connection the row is under is **not offered**; production is preselected only when it is the active tab's connection. When no other connection shows the file, the item is disabled with "No other connection shows this file". Picking a connection where the file is already open focuses that tab.
- Words per host through an `$os` selector: "Reveal in Finder" / "Reveal in File Explorer" / "Open Containing Folder"; "Move to Trash" / "Move to Recycle Bin".
- **Move to Trash is not red** (Default, controller ruling e) and asks nothing: the OS trash restores it (Q11); a tab on the file then shows Deleted on disk; a toast says where it went. The tree cursor moves to the next sibling, else the previous one, else the parent (review Minor 9).
- **New script / Rename** edit in the row: a 19 px mono field with the blue focus ring, the name selected without its extension. A bad name keeps the field, turns its border text-2 and shows the reason under it in the tooltip anatomy with the ink alert (slash, reserved Windows name, `.`/`..`/trailing dot or space, already exists, missing `.sql`/`.md` for a new script). ↵ commits; Escape cancels and returns the focus to the row.

**States.** Database · SQLite root · Scripts node · folder · file · outside link · New script inline · Rename invalid · Moved to Trash · 960 × 600 in Turkish.

**Keyboard.** ⇧F10 / menu key opens the menu on its first enabled row; ↑ ↓ ↵ Escape (shipped). **F2 renames** a script file or folder row (all hosts, `ExplorerTree`; controller ruling c). Move to Trash and Copy path have no default chord in M4b (menu and palette only; a Trash chord is a PO item).

**Focus order.** Menu rows (shipped). The inline field takes the focus while editing; Escape/↵ return it to the tree row.

### 3 · Attach dialog and mapping (R-M4b-8, R-M4b-21, Q5, Q6)
**Purpose.** Attach a folder, showing exactly what is written into it; or map an attached library to a differently named database on this computer.

**Anatomy.**
- Opened from a database's menu after the OS folder chooser ("Choose a folder of scripts"). The **480 form dialog**: title "Attach a script library", subtitle the env mark, connection and database it was opened from.
- **Folder** (static mono path, Change…), **Database** (mono TextInput, prefilled with the database it was opened on; hint "The database these scripts are for. Every connection with a database of this name shows them."), **Aliases** (mono, comma-separated).
- "Hacivat creates this file in the folder:" and the **file inset** (fingerprint-inset anatomy): its full path, then exactly `format = 1`, `database = "orders"`, `aliases = [...]` in syntax colours; the note "It holds only the database name and its aliases: never a connection, a host or a password. Commit it so your team gets the same binding."
- **Shows under:** every connection that will list it (mark + mono name), or "No connection has a database named … yet."
- **Found:** an inline notice "Found .hacivat/workspace.toml — binds to orders" / "Hacivat reads this file and never changes it.", then its content (comments included) read-only; Database and Aliases are not editable.
- **Invalid name:** the field border text-2, an ink line with the reason (empty, control characters, over 128 bytes, folder already attached, 32 libraries), Attach disabled.
- Footer: Cancel · **Attach** (Primary).
- **Mapping** (480 dialog, from "Map to a script library…"): title "Show a script library under orders_p", subtitle the connection and database; Library Select ("orders-scripts binds to orders"); "The mapping is kept on this computer only. Your teammates don't see it."; checkbox **unchecked** "Also add orders_p as an alias for everyone who uses this folder" / "Adds it to .hacivat/workspace.toml and keeps the file's comments"; checked, the file inset shows the result with the added line on the selection tint. Footer: Attach another folder… (left) · Cancel · **Show here** (Primary).

**States.** New library · with aliases · found workspace.toml · invalid name · mapping · mapping with share-as-alias · 960 × 600 in Turkish.

**Keyboard and focus order.** Tab is trapped in the card; the header's close × is not a Tab stop (Escape closes, as every M2–M4 dialog). New: Change… → Database (initial focus) → Aliases → Cancel → Attach (document order). Found: Change… → Cancel → Attach (initial focus on Attach: nothing is written into the folder). Mapping: Library → the checkbox (Space) → Attach another folder… → Cancel → Show here (initial focus). Escape cancels; ↵ on a focused button presses it.

### 4 · Settings › Script libraries
**Purpose.** Every library in one place, with its binding and its watcher state; removing never touches the folder.

**Anatomy.** The shipped Settings page; a new navigation entry "Script libraries" and its small-caps section title; the note "A library is a folder of .sql and .md files. Removing one never changes the folder or its files." with **Attach a folder…** (Default). A bordered list (1 px seam-strong, 4 px radius, rows divided by seam): folder icon, the name (mono), then on its own line "binds to orders" (and "· aliases orders_p"; this line wraps, never ellipsizes: review I4), the path (mono 11 text-2), the state line — Watching for changes (`eye`) · Checking every 2 seconds (`timer`, tooltip why: network drive / low watch limit) · Folder not found (`circle-alert`, ink) · Stopped at 20,000 files (`triangle-alert`, ink) — the script count and "git repository" / "git status off"; **Reveal in Finder** (ghost; disabled when the folder is missing) and **Remove…** (Default). **Local mappings** follow under their own small-caps title: env mark, connection › database, "shows", the library, Remove.
- **Remove…** opens a 420 alertdialog: "Remove orders-scripts from Hacivat?", the path as subtitle, "The folder and its files stay as they are, including .hacivat/workspace.toml. Its Scripts nodes go away, and so does its 1 local mapping." (without mappings the last clause is left out: `hv-scripts-remove-body-none`). **Remove** is Default (not Danger: nothing on disk changes); **Cancel** is Primary and focused.
- Search filters by name, path and database (Turkish-tolerant).

**States.** List (watching, polling, folder not found, stopped at 20,000 files) · with a mapping · Remove… · search · no libraries · 960 × 600 in Turkish.

**Focus order.** Search → Attach a folder… → per row Reveal, Remove… → per mapping Remove. Removing a row returns the focus to the search field's next stop (the list's first button), never to a vanished button.

### 5 · Script tab (HV-SCRIPT-06, R-M4b-4, R-M4b-27, R-M4b-30)
**Purpose.** A console whose text comes from a file, bound to the connection and database it was opened under, for its whole life.

**Anatomy.**
- **Tab:** `file-code-2` (or `file-text`), the title "orders_report.sql · orders-prod" in mono (one Fluent message; the file name ellipsizes down to four characters, **the connection never truncates**; an inactive script tab that still doesn't fit collapses to its icon, its title in the tooltip and the accessible name; controller ruling (d): if the **active** tab's connection alone still overflows once the file name is at its floor, the connection is shortened **in the middle**, both ends kept (`orders-rep…read-staging`), its full name in the tooltip, the accessible name, the capsule, the crumb and the status segment), the production mark on production tabs, the dirty dot (the close icon on hover). Under a MySQL database other than the profile's: "kampanya_özet.sql · api-dev · kampanya".
- **Tooltip:** the full path (mono), "From the library orders-scripts", the env mark and "Runs on orders-prod, database orders", "Unsaved changes".
- **Toolbar, capsule, hairline, crumb, status bar:** M3's, unchanged; they follow the active tab. The status bar adds the encoding segment (surface 10).
- **Tab menu:** Save ⌘S, Save all ⌥⌘S | Open under another connection… | Reveal in Finder, Copy path | Close tab ⌘W. There is no way to switch the tab's connection. "Open under another connection…" opens the chooser (surface 7) **without the tab's own connection**, then opens the script under the chosen connection in its own tab, or focuses the tab already open there (R-M4b-4); with no other candidate it is disabled ("No other connection shows this file").
- **MySQL under another database:** the tab opens its own session and switches it with `USE`; until the switch is read back, Run is disabled and the toolbar shows the run marker and "Switching to kampanya…"; Output logs "This tab's session now uses kampanya  USE kampanya".
- **Run refused (R-M4b-27):** before every Run the tab checks its session's database. MySQL: one switch back; if that fails, an alert notice under the toolbar: "Run refused: this tab runs on kampanya, but its session is on api." / "Hacivat switched the session back once and it still isn't on kampanya. Nothing was sent. Try again, or open the script under another connection." with **Switch to kampanya and run** and **Open under another connection…**; Output logs the refusal. PostgreSQL/SQLite (the profile was edited to another database): "orders-prod now opens orders_v2: the connection was edited. Nothing was sent. Open the script under the connection that shows orders." with Open under another connection….
- **Connection deleted / unbound (R-M4b-30):** capsule "No connection", title "… · no connection", Run disabled (tooltip "This tab's connection was deleted, so it runs nothing. Open it under another connection."), and the notice "The connection this tab was opened under was deleted." / "The text is kept. It runs nowhere until you open it under a connection." with Open under another connection…. A connect never binds it.
- **Markdown:** Run disabled, "Markdown notes do not run".

**States.** Under orders-prod · two tabs, one file · unsaved (+ the "Saved daily_revenue.sql" status message) · tab tooltip · tab menu · MySQL USE · connection deleted · Markdown · Run refused (MySQL) · Run refused (edited) · long connection name (a 47-character connection: the file name gives way, the connection stays whole; the capsule's name ellipsizes after the palette field collapses to its magnifier) · 960 × 600 in Turkish.

**Keyboard.** ⌘S / Ctrl+S save (script tab context), ⌥⌘S save all (macOS; Windows/Linux palette and menu only), ⌘W close (asks when dirty, 6b), ⌘↵ Run (unchanged).

**Focus order.** Tab strip → toolbar (Run) → the notice's buttons in order → the code area. Pressing a notice button that removes the notice returns the focus to the code area (the notice's focus home).

### 6 · Changes on disk (HV-SCRIPT-08, R-M4b-16, Q17)
**Purpose.** External edits arrive without a reload button and never cost the user their text.

**Anatomy** (every bar is the inline notice under the toolbar; its focus home is the tab's code area).
- **Clean tab:** reloaded in place (caret and selection mapped, undo cleared, held during an IME composition); one status-bar message for 3 s: "Reloaded orders_report.sql: it changed on disk". A followed rename: "orders_report.sql was renamed to haftalık_rapor.sql; this tab follows it".
- **Changed on disk** (dirty): `file-diff` glyph, "orders_report.sql changed on disk." (strong) / "Your edits in this tab aren't saved. Reload discards them; Keep mine keeps them, and saving then asks before it overwrites." — **Reload**, **Keep mine** (Default).
- **Reload** asks: 420 alertdialog "Discard your edits and reload orders_report.sql?", subtitle connection · database, "The file on disk replaces the text in this tab. Your edits are discarded." — **Discard and reload** (Danger: it destroys edits; alone at the footer's far left, 28 px plus a seam from the rest: DESIGN's isolation) · **Cancel** (Primary, focused, far right).
- **Keep mine** removes the notice (focus → code); the next save asks **Overwrite?**: "The file changed on disk since you opened it. Overwrite?" / "Saving replaces the version on disk, including the changes made outside Hacivat." — **Overwrite** (Danger, isolated the same way: it destroys the changes made outside Hacivat) · **Cancel** (Primary, focused).
- **Deleted on disk:** "orders_report.sql was deleted on disk." / "The text stays in this tab. Save writes the file again." — **Save ⌘S**, **Close tab**.
- **No longer listed:** "orders_report.sql is no longer listed under orders-staging." / "It is in @production, a folder for another environment." (or "It moved to a place this connection doesn't show.") / "Run and Save are off in this tab, so it never runs or writes the file where it no longer belongs." — **Close tab**, **Open under another connection…** (disabled with its reason where no other connection lists the file). *Controller ruling R-T14-4 (T14 fix round 1, overrides the earlier "This tab still runs only on …"): the tab is fail-closed; Run and Save are off.*

**States.** Changed on disk · Reload asks · silent reload · renamed · deleted · Overwrite? · no longer listed · 960 × 600 in Turkish.

**Focus order.** Toolbar → the notice's buttons → code. Dialogs: Danger → Cancel (initial), trapped; the header's close × is not a Tab stop (Escape answers Cancel).

### 6b · Unsaved changes (R-M4b-5, R-M4b-39; new in M4b)
**Purpose.** Closing a dirty script tab, or quitting or restarting with dirty script tabs, asks before anything is lost. M3's close prompt asks only about open transactions; this one runs after it in the same guard chain.

**Anatomy.** A 420 alertdialog per dirty script tab, one at a time: title "Save changes to orders_report.sql?" (file in mono); subtitle the env mark, connection and database, and "1 of 2 unsaved scripts" when several wait; body "Your changes are lost if you don't save them." Footer: **Don't save** (Default, far left) · **Save** (Default) · **Cancel** (Primary, focused). Escape and the window's close button answer Cancel. Save runs the normal save, so the secret warning (8) or Overwrite? (6) can follow it; Cancel stops the close or the quit.

**Ruling (controller ruling e):** Don't save is Default, not Danger: the answers to an app-raised question are never Danger (M3 ruling 6), Cancel protects the text, and it is the platform convention; the red outline belongs to the affirmative of a confirmation for a destructive action the user chose ("Discard and reload", "Overwrite", the recovered tab's "Discard…").

**States.** Closing one tab · quitting with two · 960 × 600 in Turkish.

**Focus order.** Don't save → Save → Cancel (initial), trapped in the card (the header × is not a Tab stop); after the prompt the focus returns to the tab's code area (or the next prompt).

### 7 · Go to script and the connection chooser (HV-SCRIPT-09, R-M4b-9, Q14)
**Purpose.** Find any script by name, then choose where it runs; production is never chosen for the user.

**Anatomy.**
- **Its own palette layer** over the shipped `Palette` (536 px, 8 px from the title bar, raised, 6 px radius, popover shadow), not a `>` mode of the command palette. Placeholder "Go to a script by name". Rows (23 px): file icon, the name in mono 12 with the match underlined in blue (Turkish-tolerant: `musteri` finds `müşteri_segment.sql`), the library-relative folder in mono 11 text-2 with its environment mark, and the library at the right in text-2. Empty query: recent/first scripts. No match: "No script matches". No library: "No script libraries yet" + how to attach one. Files under an unknown `@name` never appear.
- **↵ on a file:** one non-production candidate opens at once (or focuses its open tab); production or two or more open the **chooser** in the same surface.
- **Chooser:** a header "Open daily_revenue.sql under" (file in mono); rows: env mark, connection (mono 12), database (mono 11 text-2), "current" (outlined) on the active tab's connection, the engine at the right. Order: the active tab's connection first, then local, dev, test, staging, production, then untagged, then name. **The focused connection is always the active tab's** (`capConn`), whichever path opened the chooser (Go to script, the tree's or the tab's "Open under another connection…"). The active row is never production unless production is the active tab's connection; opened by "Open under another connection…", the chooser **leaves out the connection it starts from** (the row's or the tab's: R-M4b-4), and picking a connection where the file is already open focuses that tab; with only production candidates **no row is active** and ↵ does nothing until the user moves onto it. The footer adds "Production is never chosen for you." when a production row is listed.
- **No candidate:** "No connection shows this file." and the single row **Open without a connection** ("It opens unbound and runs nothing until you open it under a connection.").
- Footer (24 px, head tone): ↑↓ Navigate · ↵ Open/Choose · esc Back (opened from Go to script's results) or esc Close (opened from a menu).

**States.** Results · empty query · no match · chooser · chooser with production focused · production only · no candidate · 960 × 600 in Turkish.

**Keyboard.** ⇧⌘O / Ctrl+Shift+O opens it (R-M4b-37); ↑ ↓ ↵; Escape: chooser → results → closed (focus back where it was). No binding fires during an IME composition.

**Focus order.** The query field holds the focus; rows are an active-descendant list (the shipped palette's model); the palette has no buttons.

### 8 · Secret warning (HV-SCRIPT-10, R-M4b-17, Q7)
**Purpose.** Warn before a password or token is saved into a folder others read, without ever showing the value.

**Anatomy.**
- **Gutter:** a `key-round` glyph in text-2 in the mark column of each finding's line (no hue). Tooltip (popover anatomy): "Looks like a password" (or an access token, a private key, a connection URL with a password) and "Saving asks first. A comment on this line such as -- hacivat:allow-secret (with a space after the dashes) keeps it quiet."
- **Save dialog** (480 alertdialog): "3 lines in create_reader.sql look like secrets", subtitle connection · database; "Hacivat found text that looks like a password or a token. The values aren't shown here."; a field well listing **Line n · kind**, never values; "If you commit this file, the value is shared with everyone who can read the repository." (shown when the library is a git repository); the hint "To keep a line on purpose, add the comment -- hacivat:allow-secret (with a space after the dashes) or /* hacivat:allow-secret */ on it."; "Save anyway is remembered for this tab until you quit." Footer: **Save anyway** (Default) · **Cancel** (Primary, focused). Escape = Cancel.
- The sample's line 9 carries `-- hacivat:allow-secret` and is not reported. The token line is an obviously fake shape (R-C0-4).

**States.** Gutter glyph (tooltip) · save dialog · 960 × 600 in Turkish.

**Focus order.** Save anyway → Cancel (initial), trapped; after either answer the focus returns to the code.

### 9 · Large files (HV-SCRIPT-12, R-M4b-20; the M3a sketch)
**Purpose.** Files over 20 MB open as a read-only preview and run as a stream; very large pastes ask first.

**Anatomy.**
- **T2 banner** (info notice): "Read-only preview: this file is 148 MB." / "The editor shows its first 2.5 MB. Run as script sends the whole file." The code area is read-only.
- **Run as script** is the toolbar's one Primary (play icon; tooltip "Sends the whole file from disk, one statement at a time, each through Run's checks. It stops at the first error."); plain Run and ⌘↵ are absent on a preview.
- **Running:** a 26 px strip under the notice (the cursor-strip anatomy, raised): the amber run marker, "Running as script", "1,204 statements sent" (mono), "line 48213", "38 MB of 148 MB", **Stop** (Default, Tab stop). Run as script is disabled meanwhile (tooltip "Running as script: wait for it, or stop it"). Each statement still goes through the classifier, the gate and its confirmations.
- **Stopped on error:** the strip wraps: ink alert, "Stopped at statement 1,205: the server returned an error. Nothing after it was sent." + **Close**; the server text is in Output verbatim. **Stopped by the user:** "Stopped after 1,204 statements. Nothing after them was sent."
- **Paste 20–100 MB:** 420 alertdialog "Paste 42.6 MB?" / "That makes this document 61 MB, too large to edit. Paste it as a read-only preview, or cancel." — **Paste as preview** (Default) · **Cancel** (Primary, focused).
- *Task 11 fix round 1 (controller rulings):* there is no editable "Paste" past 20 MB (R-T11-2, R-M3-49), as the dialog above shows. **Mock error:** sizes follow the format locale, which follows the OS region (spec L15(d)), not the UI language; the mock's `nf()` follows the language toggle. A tr-TR region writes "42,6 MB" in either language; the product formats sizes with one decimal ("148,0 MB"; the shared app formatter, the same as the T1 banner's).

**States.** T2 preview · running · stopped on error · paste · 960 × 600 in Turkish.

**Focus order.** Toolbar → strip Stop/Close → code; Stop or Close removes the control and returns the focus to the code.

### 10 · Encoding (UI-EDIT-02, HV-SCRIPT-07)
**Purpose.** A file that isn't UTF-8 opens only once the user names its encoding, and saves back in it.

**Anatomy.**
- **Unknown encoding:** in place of the text, the notice "tr_rapor.sql isn't UTF-8 text." / "Choose the encoding it was saved in, or open it read-only to look first.", an **Encoding** Select (Windows-1254 (Turkish), ISO-8859-9 (Turkish), Windows-1252 (Western), UTF-16 LE …; a Turkish locale lists the Turkish ones first), **Open** and **Open read-only**. Run is disabled until the text is shown.
- **Read-only:** the text decoded as UTF-8 with � in text-2 where bytes can't be read, the read-only chip in the toolbar, and the notice "Shown as UTF-8; characters it can't read show as �. Reopen with an encoding to edit." with Reopen with encoding….
- **Status segment** (right group, a pointer button; **not a Tab stop**, like every shipped status-bar segment; its commands are in the palette: Reopen with encoding…, Save with encoding…, Normalize line endings): "Windows-1254 · CRLF" (`UTF-8 with BOM` when there is one); tooltip "Encoding and line endings. Saving keeps both."; its menu (above it): "Encoding: Windows-1254" header, Reopen with encoding…, Save with encoding…, then LF / CRLF with the current one checked (M3c's line-ending rows).
- **Can't encode:** a wide alert toast "Couldn't save tr_rapor.sql" / "漢 on line 4 can't be written in Windows-1254. Remove it, or use Save with encoding…."; the file on disk is unchanged and the tab stays dirty.

**States.** Unknown encoding · encoding Select open · read-only · status segment menu · can't encode · 960 × 600 in Turkish.

**Focus order.** Encoding Select → Open → Open read-only → code (Open and Open read-only remove the notice: focus → code).

### 11 · Library diagnostics popover
**Purpose.** Every library problem in one place, in plain words, saying what Hacivat does about it.

**Anatomy.** The explanation popover (raised, seam-strong, 6 px radius, popover shadow), 320 px, under the problems row; title "2 problems in orders-scripts" after the ink alert glyph; one item per problem with a text-2 glyph:
- unknown `@` folder: "@prod is not an environment tag — files under it are hidden (did you mean @production?)" + "Environment folders are @local, @dev, @test, @staging and @production.";
- contradictory scopes: "@production/@staging/ names two environments, so the files under it are hidden everywhere.";
- workspace.toml: unknown keys named in a mono box (names only, never values), invalid TOML with the parser's line, column and message in its own mono box ("Parser message"; never the source line, so a mistyped `password = "…` is not echoed), written by a newer Hacivat, larger than 64 KB — each "so the library binds by its folder name, orders-scripts";
- limits ("Listing stopped at 20,000 files…", "Folders deeper than 32 levels aren't listed."), polling (network drive / low watch limit);
- repository owned by another user: "Git status is off: the repository belongs to another user." + the safe.directory sentence;
- folder missing (also the tree's failed row and Settings).
- Footer: **Script library settings…** (Default).

**States.** Two problems · workspace.toml · repository untrusted · limits and polling · folder missing · 960 × 600 in Turkish.

**Focus order.** Opening it from the keyboard moves the focus to its button; Escape closes it and returns the focus to the problems row.

### 12 · Recovery (R-M4b-5, R-M4b-22)
**Purpose.** Unsaved script text survives a crash, kept in Hacivat's own folder, never in the library.

**Anatomy.**
- **On launch:** a wide sticky toast (`history` glyph) "Recovered unsaved changes to 2 scripts" / "Hacivat closed before they were saved. Their tabs are open again with your changes; save to keep them.", the tabs as a list (mono "orders_report.sql · orders-prod"), **Show** · **Dismiss**. Its buttons are Tab stops only after F6.
- **Recovered tab:** dirty, with the notice "Recovered unsaved changes from Nov 21, 14:05." / "Save keeps them. Discard goes back to the file on disk." — **Save ⌘S**, **Discard…** (Default here; it asks with Reload's confirmation, whose "Discard and reload" is Danger and isolated).
- **File or library gone:** "eski_rapor.sql isn't in the library any more." or "The library shared-reports isn't attached any more." / "Your recovered text is in this tab. Save writes it back to the same place." — **Save**.

**States.** On launch · recovered tab · file missing · library missing · 960 × 600 in Turkish.

**Focus order.** F6 → the toast's Show, Dismiss, close; dismissing returns the focus to the code. The tab notice: Save → Discard… → code.

---

## Keys (R-M4b-37; Task 1b's keymap rows)

Checked against the AltGr layers in `crates/ui/hacivat-ui/keymaps/altgr/` (TR-Q, TR-F, DE, FR, PL): a Ctrl+Alt chord on Windows/Linux is AltGr. Nothing starts with the palette's ⌘K/Ctrl+K; nothing takes Alt+Shift+F.

| Command | macOS | Windows / Linux | Note |
|---|---|---|---|
| Go to script (`scripts.goto`) | ⇧⌘O | Ctrl+Shift+O | Ctrl+Shift never collides with AltGr; free on all three hosts |
| Save (`file.save`, script tab context) | ⌘S | Ctrl+S | today bound only in `ConnectionDialog` |
| Save all (`file.save_all`) | ⌥⌘S | palette and menu only | `ctrl-alt-s` is AltGr+S = "ś" on PL; Ctrl+Shift+S is VS Code's Save As; Ctrl+K S starts with the palette chord (Q22) |
| Reveal (`scripts.reveal`) | palette and menu only | palette and menu only | R-M4b-37 |
| Rename (`scripts.rename`, `ExplorerTree`, script file and folder rows only) | F2 | F2 | controller ruling c; free in all three shipped keymaps |
| Move to Trash, Copy path, Copy relative path, New script, New folder | menu and palette only | menu and palette only | controller ruling c: a Trash chord is a PO item |
| Open / activate | ↵ | Enter | shipped tree key |
| Context menu | ⇧F10, menu key | Shift+F10, menu key | shipped |
| Refresh (Explorer) | F5 | F5 | shipped |
| Run | ⌘↵ | Ctrl+Enter | unchanged; absent on a T2 preview |
| Focus a toast | F6 | F6 | shipped `workspace.focus_notification` |

---

## New primitives

All from existing tokens; DESIGN.md rows are Task 21's.
- **Git status glyph** in the tree's status slot: `mono_micro` 11 at weight 400, M/N amber, C ink, ? text-3, the folder `•` text-2; never red; its text alternative is required (the shipped `RowStatus`).
- **Inline name field** in a tree row (19 px, mono 12, 3 px radius, the blue focus ring) and its **error tip** (the tooltip anatomy pinned under the field, ink alert glyph).
- **Problems row:** a tree row with the ink alert glyph instead of a kind icon; a cursor stop that opens a popover.
- **Script tab title** in two parts: the file name ellipsizes (≥ 4 characters), the separator and the connection never do; an inactive script tab that still doesn't fit collapses to its icon; the active tab's connection alone too long is shortened in the middle (ruling d).
- **Row tooltip** composed of lines: the facts of the row's cells (count, shared, env, link, cloud) and the git line (glyph + its text alternative).
- **Danger footer group:** a Danger button alone at the footer's far left, then 28 px and a seam before the rest (DESIGN's isolation).
- **Editor notices** for script states (changed, deleted, unlisted, unbound, Run refused, recovered, downloading, encoding, read-only, T2): the shipped inline notice under the toolbar with a strong first line and Default buttons in its action row.
- **Run-as-script strip:** the cursor-strip anatomy (26 px, raised) under the toolbar, wrapping when it reports a stop.
- **Go to script palette rows** (file + folder + library) and **chooser rows** (mark + connection + database + "current" outline tag + engine), with the palette's footer gaining a note.
- **File inset** for `workspace.toml` (the fingerprint inset with syntax colours; an added line on the selection tint).
- **Library list** in Settings (bordered list, rows divided by seam, a state line with a glyph).
- **Icons** (Lucide 0.460, ISC): `folder` and `file-text` are new; `scroll-text`, `link`, `download`, `file-diff`, `key-round`, `timer`, `eye`, `unplug`, `history`, `languages`, `trash-2` are in the mock sprite (Task 1b checks each against `crates/ui/hacivat-ui-core/assets/icons/lucide/`; `download`, `file-diff`, `key-round`, `timer`, `eye`, `unplug`, `history`, `languages`, `trash-2`, `link`, `scroll-text` already ship there).

---

## Open questions and rulings

Each is ruled so the build is not blocked; the PO can overrule any. Items 1–4 are conflicts with the shipped M4 code. **Controller rulings (fix round 1) override items 1, 2, 5, 6, 9 and 10:**
- **(a) Git colours:** New is amber and Conflicted is ink; Task 13 changes `crates/ui/hacivat-ui/src/tree/view/row.rs:528-531` (and its doc line) as a named exception, the glyph at `mono_micro` weight 400.
- **(b) "shared":** stays lowercase and outlined (the shipped badge); its tooltip moves into the row tooltip.
- **(c) Chords:** F2 renames (script file and folder rows, `ExplorerTree`, all hosts). Move to Trash has no chord in M4b (menu and palette only); a Trash chord is a PO item (if the PO wants it: ⌘⌫ on macOS, never plain ⌫; Delete on Windows/Linux; script rows only; a test that a profile root still routes to `connection.delete`). The earlier collision note is withdrawn: `Explorer` is pushed only on a profile root, plain `backspace` is also `connection.delete` on macOS and `cmd-backspace` is `Editor`-only; the risk is semantic (the same key with two weights a cursor step apart).
- **(d) Tab titles:** a script tab's connection name does not truncate; if the active tab's connection alone overflows at the file name's 4-character floor, it is shortened in the middle, both ends kept, with the full name in the tooltip (fixture: `orders-reporting-replica-eu-west-1-read-staging`, 47 characters, surface 5 "Long connection name").
- **(e) Button variants:** Danger: "Discard and reload", "Overwrite" and the recovered tab's "Discard…" confirm (all with DESIGN's 28 px isolation); Default: "Don't save", "Remove" and "Move to Trash".


1. **Git "New" is amber, not green** (conflict: `crates/ui/hacivat-ui/src/tree/view/row.rs:528-531` draws `StatusKind::New` green and `Conflicted` amber). *Ruling:* Modified and New are amber (Pending Amber: "uncommitted edits"; a new file is an uncommitted change, not a success), Conflicted is ink (a refusal-like state: Errors Without A New Hue), Unknown text-3, ContainsChanges text-2. *Cost:* two match arms in `row.rs` and one shot each (a named exception in Task 13).
2. **The "shared" badge is the shipped outlined badge, lowercase** (the plan says small-caps "SHARED"). `TreeRowData::badge` has no small-caps variant, and small caps at the badge's 11.5 px draw 8 px capitals, under the 11 px floor in spirit; M4's precedent badges are lowercase ("partial", "version 17"). *Ruling:* "shared" / "paylaşılan", outlined, glyph-less (so on a crowded row it gives way whole, as the shipped widget does). *Cost if wrong:* a small-caps badge variant in the tree widget.
3. **SQLite: under the profile root above `main`,** as M4 shipped the seam (`contrib.rs` module docs, `adapter.rs:1287`), not "under main and each attached database" (now R-M4b-32). *Cost:* none (applied).
4. **The database menu needs the contributor's items** (`actions.rs:637-700`: the Database/Schema arm consults no provider, and `ActionSlot` has no scripts slot; `node_menu` answers `None` for contributed nodes). *Ruling:* Task 1b's named exception appends the two Scripts items after M4's items (R-M4b-34); the mock shows that order. *Cost:* the R-M4b-34 lines.
5. **Move to Trash is not red and asks nothing:** recoverable from the OS trash (Q11); a toast says where it went. *Cost if wrong:* a confirmation dialog.
6. **Remove a library is Default, not Danger:** nothing on disk changes. **Don't save** (6b) is Default too; **Discard and reload** and **Overwrite** are Danger (they confirm destroying text). *Cost if wrong:* button variants.
7. **The problems row is the library's first child** (not last), so it is visible without scrolling; it is a cursor stop. *Cost if wrong:* row order.
8. **Run as script replaces Run on a T2 preview** (one primary per toolbar) and has no ⌘↵, so a reflex never streams 148 MB. *Cost if wrong:* a key row.
9. **Chords for Rename and Move to Trash:** superseded by controller ruling (c).
10. **A script tab's connection never ellipsizes:** adopted with the bounded middle fallback, controller ruling (d).
11. **The chooser footer says "Production is never chosen for you."** whenever a production row is listed and not focused. *Cost if wrong:* one string.
12. **"Open without a connection" is the only row when no connection shows the file** (R-M4b-9). *Cost if wrong:* copy.
13. **Git letters are the same in Turkish** (M, N, C, ?): they are what git users read everywhere; the Turkish text alternatives carry the meaning. *Cost if wrong:* three strings.
14. **Turkish terms** (all `# Review: PO`): betik / Betikler (script), betik kitaplığı (script library), paylaşılan (shared), eşle (map), diskte değişti, Çöp Sepeti / Geri Dönüşüm Kutusu, kodlama, satır sonu, Üretim bağlantısı for production in prose; transaction, commit and rollback stay English ("commit'ten", "commit edin").
15. **Counts without a plural selector:** `hv-scripts-unsaved-count` (shown only when two or more wait), `hv-scripts-run-stopped` (an ordinal position), `hv-scripts-size-mb` (a unit), and the limits inside `hv-scripts-set-status` / `hv-scripts-diag-limit` (20,000 files, 32 levels). Every other EN message with a count has `[one]` / `*[other]`. *Cost if wrong:* variants.
16. **The encoding prompt replaces the text** (nothing is decoded until the user names an encoding); read-only shows UTF-8 with �. *Cost if wrong:* a decode guess (no auto-detection in v1).
17. **SQLite root menu order** (review Minor 7): the two Scripts items follow the M2a items after a separator and come **before** Delete…, which stays last (DESIGN: the destructive row goes last). R-M4b-34's "append after a separator" should read, for the SQLite root, "after the M2a items, before the destructive group", so Task 1b does not append after Delete…. *Cost if wrong:* row order.
18. **The chooser's focused connection is the active tab's, and the source connection is excluded** (review Critical 1, I1): applied on every path; the check compares the active row with the rule (`capConn()`), not with a value the chooser computed. *Cost:* none.
19. **The capsule at 960 with a long name** (found while adding ruling d's fixture): the path word goes first, then the palette field collapses to its magnifier (DESIGN Minimum window), then the capsule's connection name (data) ellipsizes; the environment and access words never give way. *Cost if wrong:* the shipped capsule's own collapse order already rules this; the mock now follows it.
20. **Tab-stop declarations** (review Minor 3): every dialog's header × is not a Tab stop (Escape closes); the status bar's encoding segment is not a Tab stop (its commands are in the palette); toast buttons only after F6. Everything else with a button is a Tab stop.

### For the controller (Task 1b / 13–18 / 21 pick-up)
1. Two new icons for Task 1b's icon addendum: `folder`, `file-text`.
2. The `row.rs` status colours and weight (ruling a) as a named exception in Task 13, with its shot.
3. The key rows in the table above (R-M4b-37 plus ruling c's F2); a Trash chord goes to the PO list.
6. R-M4b-34's wording for the SQLite root (ruling 17).
4. The hui strings (`hui-editor-t2-*`, `hui-editor-paste-confirm-*`) belong to the editor crate's `hui.ftl` (Task 11's block); everything else to Task 1b's `## m4b-t01b-seams` block in `hacivat-ui.ftl`.
5. The menu label is "Map to a script library…" (the rewritten plan); the mapping dialog's title stays "Show a script library under {database}".

### Open items for the PO
1. Turkish copy (every TR string below is a draft).
2. Controller rulings (a)–(e) for sign-off; a Move to Trash chord (ruling c).
3. Go to script's chord ⇧⌘O / Ctrl+Shift+O (R-M4b-37) instead of VS Code's ⌘P.

---

## Verification

Run on the committed file (the harness is the mock's own `?check=all` mode, driven by headless Chrome with `--dump-dom`, the same driver M4 used; the driver scripts live in the session scratchpad and are not committed):
- **950 renders, 0 findings** (after fix round 1): 90 scenarios of the thirteen surfaces × EN/TR × dark/light × 1440 × 900, **1280 × 800** and 960 × 600 (the thirteen pinned "960 · TR" scenarios render dark and light only).
- Each render is checked for: exceptions; the number of cp-fill blocks larger than a mark (exactly one when the capsule is production, else none); **red outside production/destructive** (every element whose text, fill or border is a red token must be an environment mark, the capsule's block, the production status segment, a Danger button, a destructive menu row, the sheet's production word or the production hairline); **git badges in red** and git badges without a text alternative; text below 11 px (small caps excepted); text-3 on raised (dark; disabled exempt); `undefined`, `NaN`, `null` or an unfilled `{placeholder}`; horizontal overflow in toolbars, the status bar, the title bar, dialog heads and footers, menus, tooltips, the name-error tip, toasts, tree rows, notices, panel headers, the filter, tabs, the settings navigation, palette rows, header and footer, library and mapping rows, strips, popovers, form rows and the secret list; chrome text that ellipsizes (data exempt: only spans that hold names, paths, types or SQL; Fluent state words in the tree's meta, in Settings and on tabs are not exempt, review I4); trust flags clipped; the capsule's production block, the status segment, the active production tab's mark and the crumb's mark hidden; **a script tab's connection clipped or ellipsized**; a production tab without the hairline; **a Danger button within 28 px of another footer button**; **a per-cell tooltip in a tree row**; **the capsule running under the palette field**; **a tab connection shortened before its file name gave way, or without both ends**; floating layers outside the window or over the status bar; a modal without the focus inside it; **a button that is not a Tab stop** in a dialog footer, notice, popover, settings row or strip; **a toast button that is a Tab stop before F6**; **the chooser preselecting production** (compared with the active tab's connection, the rule itself); **a secret value in any dialog, toast, tooltip, menu, popover or the status bar**.
- **58 behaviour probes, all PASS** (7 added in fix round 1: the tree's Open under another connection… with the active tab on orders-dev and a file row under orders-prod preselects no production row and leaves out orders-prod; a tab's Open under another connection… leaves out its own connection; choosing a connection where the file is already open focuses that tab, no duplicate; the database menu appends the Scripts items after all of M4's items; a 47-character connection stays whole on the tab at 1440 and 960 and in the capsule; in a pane too narrow for it, it is shortened in the middle with both ends after the file name reached its floor), driven through real DOM events where a user would act: the scope table (`@PRODUCTION` scopes, `@prod` and `@productİon` hide, `@production/@staging/` hides); an untagged profile sees only unscoped files; orders-prod lists `@production/`, orders-staging hides it (E3) and lists `@staging/`; `@prod` is listed nowhere; the count equals the listed files; Scripts sits above the schemas; the Scripts row tooltip carries the count and names the other connections; one file opened under orders-prod and orders-staging gives two tabs bound to each (E1) and the capsule follows; the tab menu has no switch; Open under another connection… opens a new tab and leaves the first bound; the chooser lists the focused connection first and never preselects production; production-only has no active row, ↵ opens nothing, ↓ moves onto it; production is preselected only when focused; Escape goes back to the results; `musteri` finds `müşteri_segment.sql`; no hidden file in Go to script; the secret dialog lists lines and kinds but no value, opens on Cancel, Escape keeps the tab unsaved, the allow line is not reported; Reload asks on Cancel with a Danger button, Escape keeps the edits, Keep mine returns the focus to the code, the next save asks Overwrite? on Cancel; the paste confirmation opens on Cancel with Paste as preview Default; a T2 preview has one primary and no ⌘↵; the attach file is exactly format/database/aliases, follows the aliases typed, an empty name disables Attach, no connection detail reaches it; the alias box is unchecked; the file menu has Move to Trash (not red) and Open under another connection…; `/` in a name is refused, Escape returns to the tree; Remove asks on Cancel and is not Danger; the recovery toast's buttons are not Tab stops before F6, F6 makes them so, dismissing returns the focus to the code; a MySQL tab off its database refuses Run naming both databases, and the notice's button returns the focus to the code; the unsaved prompt has Save / Don't save / Cancel with Cancel focused and Don't save not red, Escape keeps the tab; closing a dirty tab asks; EN singular variants; enum variants; no TR plural variants; every EN key has a TR draft.
- Fix round 1 PNG review: the long-name capsule ran under the palette field at 960 (ruling 19), the active tab's connection was shortened while its file name kept full width (fixed: the file name gives way first), the sheet header overflowed with a long name.
- PNG review (1440 dark/light and every "960 · TR" in light) caught, and the design fixed: the dark theme tokens lost to a stray comment end in the CSS header (every dark render had no capsule fill; found by the filled-block check); a pinned tooltip lost when the screenshot resized the page; the tree selection under a different connection than the open tab on four surfaces; an inactive script tab's connection ellipsized at 960 in Turkish (now never; ruling 10); a downloading tab showing placeholder text; a link tooltip repeating its title; the found-file dialog opening with the focus on its close icon.
- `grep -nE "(sk|rk)_(live|test)_[A-Za-z0-9]{10,}|gh[pousr]_[A-Za-z0-9]{20,}|xox[abpr]-[A-Za-z0-9-]{10,}|AIza[0-9A-Za-z_-]{20,}"` over `design/m4b-scripts/` finds nothing (R-C0-4).
- The copy tables and FTL below are generated from the mock's own `i18n` block, so the two cannot drift.

---

## Copy strings (EN / TR)

The id is the Fluent id. Variables are Fluent variables (`{ $name }`). **Counts are Number arguments** (M4 ruling 22): one Number both selects the plural and is shown, formatted by `hacivat_l10n` in the format locale; line numbers use `NUMBER($line, useGrouping: "false")`, the recovery time `DATETIME($date, …)`; sizes are String arguments made from `hv-scripts-size-mb`. **Every EN message with a count has a `[one]` / `*[other]` selector** except those in ruling 15; Turkish needs none. **No sentence is assembled from fragments:** host words, reasons, kinds and states are `$os` / `$reason` / `$kind` / `$state` / `$enc` / `$what` / `$limit` selectors holding whole sentences. Names inside a message (`{ $file }`, `{ $conn }`, `{ $database }`, `{ $library }`, `{ $path }`, `{ $folder }`) are user data, FSI/PDI-isolated by the caller; the mock shows them in mono. In the tables, `· [one] …` and `· [value] …` list a message's other variants; the FTL is the authoritative shape.

| Prefix | File | Owner (own block) |
|---|---|---|
| `hv-scripts-*`, `hv-explorer-menu-attach-library`, `hv-explorer-menu-map-library*`, `hv-cmd-scripts-*`, `hv-settings-nav-scripts`, `hv-settings-section-scripts` | `locales/{en,tr}/hacivat-ui.ftl` | Task 1b, `## m4b-t01b-seams` |
| `hui-editor-t2-*`, `hui-editor-paste-confirm-*` | `locales/{en,tr}/hui.ftl` | Task 11, `## m4b-t11-editor` |

Reused verbatim, not redefined: `hv-explorer-*` (M4), `hv-conn-menu-*`, `hv-capsule-latency`, `hv-console-run`, `hv-console-stop`, `hv-tab-console-untitled`, `hv-editor-lncol`, `hv-results-output`, `hv-results-empty`, `hv-editor-empty`, `hv-settings-*`, `hv-find-tolerant*`, `hui-tab-close`, `hui-toast-dismiss`, `hui-palette-hint-*`, and the M2a strings (`dlg-cancel`, `env-*`, `sb-*`, `kv-*`).

<!-- generated from index.html's i18n block (t02 gen.py); do not edit by hand -->
#### Surface 1 · Scripts node

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-scripts-node` | Scripts | Betikler |
| `hv-scripts-node-shared` | shared | paylaşılan |
| `hv-scripts-node-shared-tip` | Also under {conns}: every connection with a database of this name shows the same files. | Ayrıca şunların altında: {conns}. Bu adda veritabanı olan her bağlantı aynı dosyaları gösterir. |
| `hv-scripts-lib-tip` | Folder {path}. Binds to the database {database}. | Klasör: {path}. {database} veritabanına bağlanır. |
| `hv-scripts-env-folder-tip` | Only connections tagged {env} see this folder. | Bu klasörü yalnızca {env} etiketli bağlantılar görür. |
| `hv-scripts-git-modified` | M | M |
| `hv-scripts-git-modified-tip` | Modified: differs from the last commit | Değiştirildi: son commit'ten farklı |
| `hv-scripts-git-new` | N | N |
| `hv-scripts-git-new-tip` | New: not in the last commit | Yeni: son commit'te yok |
| `hv-scripts-git-conflicted` | C | C |
| `hv-scripts-git-conflicted-tip` | Conflicted: the merge isn't resolved | Çakışma: birleştirme çözülmedi |
| `hv-scripts-git-unknown` | ? | ? |
| `hv-scripts-git-unknown-tip` | Git status unknown: a .gitattributes filter applies to it, and Hacivat never runs filters. · [dataless] Git status unknown: the file isn't downloaded, and Hacivat doesn't download it to compare. | Git durumu bilinmiyor: dosyaya bir .gitattributes filtresi uygulanıyor; Hacivat filtreleri hiç çalıştırmaz. · [dataless] Git durumu bilinmiyor: dosya indirilmedi; Hacivat karşılaştırmak için onu indirmez. |
| `hv-scripts-git-contains` | • | • |
| `hv-scripts-git-contains-tip` | Contains changed files | Değişmiş dosyalar içeriyor |
| `hv-scripts-outside-link-tip` | Points outside the library, so Hacivat never opens it. | Kitaplığın dışına işaret ediyor; Hacivat bunu hiç açmaz. |
| `hv-scripts-dataless` | not downloaded | indirilmedi |
| `hv-scripts-downloading` | Downloading… | İndiriliyor… |
| `hv-scripts-diag-row` | {n} problems · [one] {n} problem | {n} sorun |
| `hv-scripts-missing-row` | Folder not found | Klasör bulunamadı |
| `hv-scripts-loading` | Listing scripts… | Betikler listeleniyor… |
| `hv-scripts-downloading-tab` | Downloading {file}… | {file} indiriliyor… |
| `hv-scripts-downloading-body` | It is kept in the cloud; the text shows when the download ends. Nothing runs until then. | Bulutta tutuluyor; indirme bitince metin görünür. O zamana dek hiçbir şey çalışmaz. |
| `hv-scripts-node-row-tip` | {n} scripts listed under this connection. · [one] {n} script listed under this connection. | Bu bağlantının altında {n} betik listeleniyor. |
| `hv-scripts-dataless-row-tip` | Kept in the cloud, not downloaded. Opening it downloads it first. | Bulutta tutuluyor, indirilmedi. Açınca önce indirilir. |
| `hv-scripts-downloading-row-tip` | Downloading… The tab shows the text when the download ends. | İndiriliyor… İndirme bitince sekme metni gösterir. |

#### Surface 2 · Context menus

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-explorer-menu-attach-library` | Attach script library… | Betik kitaplığı bağla… |
| `hv-explorer-menu-map-library` | Map to a script library… | Bir betik kitaplığına eşle… |
| `hv-explorer-menu-map-library-none` | No other library is attached | Bağlı başka kitaplık yok |
| `hv-scripts-menu-new-script` | New script | Yeni betik |
| `hv-scripts-menu-new-folder` | New folder | Yeni klasör |
| `hv-scripts-menu-open` | Open | Aç |
| `hv-scripts-menu-open-under` | Open under another connection… | Başka bir bağlantıda aç… |
| `hv-scripts-menu-rename` | Rename | Yeniden adlandır |
| `hv-scripts-menu-trash` | Move to Trash · [windows] Move to Recycle Bin | Çöp Sepeti'ne taşı · [windows] Geri Dönüşüm Kutusu'na taşı |
| `hv-scripts-menu-reveal` | Reveal in Finder · [windows] Reveal in File Explorer · [linux] Open Containing Folder | Finder'da göster · [windows] Dosya Gezgini'nde göster · [linux] Bulunduğu klasörü aç |
| `hv-scripts-menu-copy-path` | Copy path | Yolu kopyala |
| `hv-scripts-menu-copy-rel` | Copy relative path | Göreli yolu kopyala |
| `hv-scripts-menu-settings` | Script library settings… | Betik kitaplığı ayarları… |
| `hv-scripts-menu-outside-tip` | Points outside the library | Kitaplığın dışına işaret ediyor |
| `hv-scripts-new-name` | untitled.sql | adsız.sql |
| `hv-scripts-new-folder-name` | new folder | yeni klasör |
| `hv-scripts-name-err` | Type a name. · [slash] A name can't contain / or \. · [reserved] Windows keeps this name for devices (CON, PRN, AUX, NUL, COM1…). · [exists] {name} is already in this folder. · [dots] A name can't be . or .., and can't end with a dot or a space. · [ext] A script's name ends in .sql or .md. | Bir ad yazın. · [slash] Ad / ya da \ içeremez. · [reserved] Windows bu adı aygıtlar için ayırır (CON, PRN, AUX, NUL, COM1…). · [exists] {name} bu klasörde zaten var. · [dots] Ad . ya da .. olamaz; nokta ya da boşlukla bitemez. · [ext] Betik adı .sql ya da .md ile biter. |
| `hv-scripts-trashed` | Moved {name} to the Trash. You can put it back from there. · [windows] Moved {name} to the Recycle Bin. You can restore it from there. | {name} Çöp Sepeti'ne taşındı. Oradan geri alabilirsiniz. · [windows] {name} Geri Dönüşüm Kutusu'na taşındı. Oradan geri yükleyebilirsiniz. |
| `hv-scripts-menu-open-under-none` | No other connection shows this file | Bu dosyayı gösteren başka bağlantı yok |

#### Surface 3 · Attach dialog

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-scripts-attach-choose` | Choose a folder of scripts | Betik klasörü seçin |
| `hv-scripts-attach-title` | Attach a script library | Betik kitaplığı bağla |
| `hv-scripts-attach-folder` | Folder | Klasör |
| `hv-scripts-attach-change` | Change… | Değiştir… |
| `hv-scripts-attach-database` | Database | Veritabanı |
| `hv-scripts-attach-database-hint` | The database these scripts are for. Every connection with a database of this name shows them. | Bu betiklerin ait olduğu veritabanı. Bu adda veritabanı olan her bağlantı onları gösterir. |
| `hv-scripts-attach-aliases` | Aliases | Takma adlar |
| `hv-scripts-attach-aliases-ph` | Other names for it, separated by commas | Başka adları, virgülle ayırarak |
| `hv-scripts-attach-shows-under` | Shows under | Görüneceği yerler |
| `hv-scripts-attach-shows-none` | No connection has a database named {database} yet. | Henüz {database} adlı veritabanı olan bağlantı yok. |
| `hv-scripts-attach-will-write` | Hacivat creates this file in the folder: | Hacivat klasörde şu dosyayı oluşturur: |
| `hv-scripts-attach-file-note` | It holds only the database name and its aliases: never a connection, a host or a password. Commit it so your team gets the same binding. | Yalnızca veritabanı adını ve takma adlarını içerir; bağlantı, sunucu ya da parola asla yazılmaz. Ekibiniz aynı bağlamayı alsın diye commit edin. |
| `hv-scripts-attach-found` | Found .hacivat/workspace.toml — binds to {database} | .hacivat/workspace.toml bulundu — {database} veritabanına bağlar |
| `hv-scripts-attach-found-note` | Hacivat reads this file and never changes it. | Hacivat bu dosyayı okur, hiç değiştirmez. |
| `hv-scripts-attach-btn` | Attach | Bağla |
| `hv-scripts-attach-err` | Type the database name. · [control] A database name can't contain control characters. · [long] A database name can be at most 128 bytes. · [already] This folder is already attached. · [many] Hacivat holds up to 32 libraries. Remove one in Settings › Script libraries first. | Veritabanının adını yazın. · [control] Veritabanı adı denetim karakteri içeremez. · [long] Veritabanı adı en çok 128 bayt olabilir. · [already] Bu klasör zaten bağlı. · [many] Hacivat en çok 32 kitaplık tutar. Önce Ayarlar › Betik kitaplıkları'ndan birini kaldırın. |
| `hv-scripts-map-title` | Show a script library under {database} | {database} altında bir betik kitaplığı göster |
| `hv-scripts-map-library` | Library | Kitaplık |
| `hv-scripts-map-binds` | binds to {database} | {database} veritabanına bağlı |
| `hv-scripts-map-note` | The mapping is kept on this computer only. Your teammates don't see it. | Eşleme yalnızca bu bilgisayarda tutulur. Ekip arkadaşlarınız görmez. |
| `hv-scripts-map-alias` | Also add {database} as an alias for everyone who uses this folder | {database} adını, bu klasörü kullanan herkes için takma ad olarak da ekle |
| `hv-scripts-map-alias-sub` | Adds it to .hacivat/workspace.toml and keeps the file's comments | .hacivat/workspace.toml dosyasına ekler, dosyadaki yorumları korur |
| `hv-scripts-map-btn` | Show here | Burada göster |
| `hv-scripts-map-other` | Attach another folder… | Başka klasör bağla… |

#### Surface 4 · Settings › Script libraries

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-settings-nav-scripts` | Script libraries | Betik kitaplıkları |
| `hv-settings-section-scripts` | Script libraries | Betik kitaplıkları |
| `hv-scripts-set-attach` | Attach a folder… | Klasör bağla… |
| `hv-scripts-set-note` | A library is a folder of .sql and .md files. Removing one never changes the folder or its files. | Kitaplık, .sql ve .md dosyalarından oluşan bir klasördür. Kaldırmak klasöre ve dosyalarına hiç dokunmaz. |
| `hv-scripts-set-status` | Watching for changes · [polling] Checking every 2 seconds · [unavailable] Folder not found · [limit] Stopped at {n} files | Değişiklikler izleniyor · [polling] 2 saniyede bir denetleniyor · [unavailable] Klasör bulunamadı · [limit] {n} dosyada durdu |
| `hv-scripts-set-status-tip` | This folder is on a network drive, so Hacivat checks it every 2 seconds instead of being told about changes. · [watches] The system's file-watch limit is low, so Hacivat checks this folder every 2 seconds instead of being told about changes. | Bu klasör bir ağ sürücüsünde; Hacivat değişiklik bildirimi beklemek yerine onu 2 saniyede bir denetler. · [watches] Sistemin dosya izleme sınırı düşük; Hacivat değişiklik bildirimi beklemek yerine bu klasörü 2 saniyede bir denetler. |
| `hv-scripts-set-files` | {n} scripts · [one] {n} script | {n} betik |
| `hv-scripts-set-aliases` | aliases {aliases} | takma adlar: {aliases} |
| `hv-scripts-set-git` | git repository | git deposu |
| `hv-scripts-set-git-off` | git status off | git durumu kapalı |
| `hv-scripts-set-remove` | Remove… | Kaldır… |
| `hv-scripts-set-mappings` | Local mappings | Yerel eşlemeler |
| `hv-scripts-set-mapping-shows` | shows | gösterir: |
| `hv-scripts-set-mapping-remove` | Remove | Kaldır |
| `hv-scripts-set-empty` | No script libraries yet. Attach a folder of .sql and .md files to see its scripts under the databases they are for. | Henüz betik kitaplığı yok. Betiklerini ait oldukları veritabanlarının altında görmek için .sql ve .md dosyalarından oluşan bir klasör bağlayın. |
| `hv-scripts-remove-title` | Remove {library} from Hacivat? | {library} Hacivat'tan kaldırılsın mı? |
| `hv-scripts-remove-body` | The folder and its files stay as they are, including .hacivat/workspace.toml. Its Scripts nodes go away, and so do its {n} local mappings. · [one] The folder and its files stay as they are, including .hacivat/workspace.toml. Its Scripts nodes go away, and so does its {n} local mapping. | Klasör ve dosyaları, .hacivat/workspace.toml dahil, olduğu gibi kalır. Betikler düğümleri ve {n} yerel eşlemesi kaldırılır. |
| `hv-scripts-remove-btn` | Remove | Kaldır |
| `hv-scripts-remove-body-none` | The folder and its files stay as they are, including .hacivat/workspace.toml. Its Scripts nodes go away. | Klasör ve dosyaları, .hacivat/workspace.toml dahil, olduğu gibi kalır. Betikler düğümleri kaldırılır. |

#### Surface 5 · Script tab

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-scripts-tab-title` | {file} · {conn} | {file} · {conn} |
| `hv-scripts-tab-title-db` | {file} · {conn} · {database} | {file} · {conn} · {database} |
| `hv-scripts-tab-title-unbound` | {file} · no connection | {file} · bağlantı yok |
| `hv-scripts-tab-tip-library` | From the library {library} | {library} kitaplığından |
| `hv-scripts-tab-tip-runs` | Runs on {conn}, database {database} | {conn} üzerinde, {database} veritabanında çalışır |
| `hv-scripts-tab-tip-unbound` | Runs nowhere: its connection was deleted | Hiçbir yerde çalışmaz: bağlantısı silindi |
| `hv-scripts-tab-tip-unsaved` | Unsaved changes | Kaydedilmemiş değişiklikler |
| `hv-scripts-tab-save` | Save | Kaydet |
| `hv-scripts-tab-save-all` | Save all | Tümünü kaydet |
| `hv-scripts-tab-saved` | Saved {file} | {file} kaydedildi |
| `hv-scripts-run-md` | Markdown notes do not run | Markdown notları çalışmaz |
| `hv-scripts-run-unbound` | This tab's connection was deleted, so it runs nothing. Open it under another connection. | Bu sekmenin bağlantısı silindi; hiçbir şey çalıştırmaz. Başka bir bağlantıda açın. |
| `hv-scripts-run-switching` | Switching to {database}… | {database} veritabanına geçiliyor… |
| `hv-scripts-unbound-title` | The connection this tab was opened under was deleted. | Bu sekmenin açıldığı bağlantı silindi. |
| `hv-scripts-unbound-body` | The text is kept. It runs nowhere until you open it under a connection. | Metin korunuyor. Bir bağlantıda açana kadar hiçbir yerde çalışmaz. |
| `hv-scripts-use-log` | This tab's session now uses {database} | Bu sekmenin oturumu artık {database} veritabanını kullanıyor |
| `hv-scripts-run-refused` | Run refused: this tab runs on {bound}, but its session is on {current}. | Çalıştırma reddedildi: bu sekme {bound} üzerinde çalışır, ama oturumu {current} üzerinde. |
| `hv-scripts-run-refused-body` | Hacivat switched the session back once and it still isn't on {bound}. Nothing was sent. Try again, or open the script under another connection. · [edited] {conn} now opens {current}: the connection was edited. Nothing was sent. Open the script under the connection that shows {bound}. | Hacivat oturumu bir kez geri geçirdi, yine de {bound} üzerinde değil. Hiçbir şey gönderilmedi. Yeniden deneyin ya da betiği başka bir bağlantıda açın. · [edited] {conn} artık {current} veritabanını açıyor: bağlantı düzenlendi. Hiçbir şey gönderilmedi. Betiği {bound} veritabanını gösteren bağlantıda açın. |
| `hv-scripts-run-refused-retry` | Switch to {bound} and run | {bound} veritabanına geç ve çalıştır |
| `hv-scripts-run-refused-unbound` | Run refused: this tab has no connection. | Çalıştırma reddedildi: bu sekmenin bağlantısı yok. |

#### Surface 6 · Changes on disk

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-scripts-disk-changed` | {file} changed on disk. | {file} diskte değişti. |
| `hv-scripts-disk-changed-body` | Your edits in this tab aren't saved. Reload discards them; Keep mine keeps them, and saving then asks before it overwrites. | Bu sekmedeki düzenlemeleriniz kaydedilmedi. Yeniden yükle onları atar; Benimkini koru onları tutar ve kaydederken üzerine yazmadan önce sorar. |
| `hv-scripts-disk-reload` | Reload | Yeniden yükle |
| `hv-scripts-disk-keep` | Keep mine | Benimkini koru |
| `hv-scripts-disk-reload-title` | Discard your edits and reload {file}? | Düzenlemeleriniz atılıp {file} yeniden yüklensin mi? |
| `hv-scripts-disk-reload-body` | The file on disk replaces the text in this tab. Your edits are discarded. | Diskteki dosya bu sekmedeki metnin yerini alır. Düzenlemeleriniz atılır. |
| `hv-scripts-disk-reload-yes` | Discard and reload | At ve yeniden yükle |
| `hv-scripts-disk-reloaded` | Reloaded {file}: it changed on disk | {file} diskte değiştiği için yeniden yüklendi |
| `hv-scripts-disk-renamed` | {file} was renamed to {new}; this tab follows it | {file} dosyasının adı {new} oldu; bu sekme onu izliyor |
| `hv-scripts-disk-deleted` | {file} was deleted on disk. | {file} diskten silindi. |
| `hv-scripts-disk-deleted-body` | The text stays in this tab. Save writes the file again. | Metin bu sekmede duruyor. Kaydet dosyayı yeniden yazar. |
| `hv-scripts-disk-overwrite-title` | The file changed on disk since you opened it. Overwrite? | Dosya siz açtıktan sonra diskte değişti. Üzerine yazılsın mı? |
| `hv-scripts-disk-overwrite-body` | Saving replaces the version on disk, including the changes made outside Hacivat. | Kaydetmek, Hacivat dışında yapılan değişiklikler dahil, diskteki sürümün yerini alır. |
| `hv-scripts-disk-overwrite` | Overwrite | Üzerine yaz |
| `hv-scripts-unlisted` | {file} is no longer listed under {conn}. | {file} artık {conn} altında listelenmiyor. |
| `hv-scripts-unlisted-body` | It moved into {folder}, a folder for another environment. This tab still runs only on {conn}. · [binding] Its library no longer binds to this database. This tab still runs only on {conn}. | Başka bir ortamın klasörüne, {folder} içine taşındı. Bu sekme yine yalnızca {conn} üzerinde çalışır. · [binding] Kitaplığı artık bu veritabanına bağlı değil. Bu sekme yine yalnızca {conn} üzerinde çalışır. |

#### Surface 6b · Unsaved changes

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-scripts-unsaved-title` | Save changes to {file}? | {file} dosyasındaki değişiklikler kaydedilsin mi? |
| `hv-scripts-unsaved-body` | Your changes are lost if you don't save them. | Kaydetmezseniz değişiklikleriniz kaybolur. |
| `hv-scripts-unsaved-count` | {n} of {total} unsaved scripts | {total} kaydedilmemiş betikten {n}. betik |
| `hv-scripts-unsaved-save` | Save | Kaydet |
| `hv-scripts-unsaved-discard` | Don't save | Kaydetme |

#### Surface 7 · Go to script

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-cmd-scripts-goto` | Go to script | Betiğe git |
| `hv-cmd-scripts-save` | Save script | Betiği kaydet |
| `hv-cmd-scripts-save-all` | Save all scripts | Tüm betikleri kaydet |
| `hv-cmd-scripts-attach` | Attach script library… | Betik kitaplığı bağla… |
| `hv-cmd-scripts-reveal` | Reveal script in Finder · [windows] Reveal script in File Explorer · [linux] Open the script's folder | Betiği Finder'da göster · [windows] Betiği Dosya Gezgini'nde göster · [linux] Betiğin klasörünü aç |
| `hv-scripts-goto-ph` | Go to a script by name | Ada göre betiğe git |
| `hv-scripts-goto-none` | No script matches | Eşleşen betik yok |
| `hv-scripts-goto-empty` | No script libraries yet | Henüz betik kitaplığı yok |
| `hv-scripts-goto-empty-body` | Attach a folder from a database's menu or Settings › Script libraries. | Bir veritabanının menüsünden ya da Ayarlar › Betik kitaplıkları'ndan klasör bağlayın. |
| `hv-scripts-goto-hint-open` | Open | Aç |
| `hv-scripts-choose-title` | Open {file} under | {file} şu bağlantıda açılsın: |
| `hv-scripts-choose-current` | current | geçerli |
| `hv-scripts-choose-prod-note` | Production is never chosen for you. | Üretim bağlantısı sizin yerinize hiç seçilmez. |
| `hv-scripts-choose-none` | No connection shows this file. | Bu dosyayı gösteren bağlantı yok. |
| `hv-scripts-choose-none-row` | Open without a connection | Bağlantı olmadan aç |
| `hv-scripts-choose-none-body` | It opens unbound and runs nothing until you open it under a connection. | Bağlantısız açılır; bir bağlantıda açana kadar hiçbir şey çalıştırmaz. |
| `hv-scripts-choose-hint-choose` | Choose | Seç |
| `hv-scripts-choose-hint-back` | Back | Geri |
| `hv-scripts-choose-hint-close` | Close | Kapat |

#### Surface 8 · Secret warning

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-scripts-secret-gutter` | Looks like a password · [token] Looks like an access token · [key] Looks like a private key · [url] Looks like a connection URL with a password | Parolaya benziyor · [token] Erişim belirtecine benziyor · [key] Özel anahtara benziyor · [url] Parola içeren bir bağlantı adresine benziyor |
| `hv-scripts-secret-gutter-hint` | Saving asks first. A comment on this line such as -- {marker} (with a space after the dashes) keeps it quiet. | Kaydederken önce sorulur. Bu satırdaki -- {marker} gibi bir yorum (tirelerden sonra bir boşlukla) onu susturur. |
| `hv-scripts-secret-title` | {n} lines in {file} look like secrets · [one] {n} line in {file} looks like a secret | {file} içinde {n} satır gizli bilgiye benziyor |
| `hv-scripts-secret-body` | Hacivat found text that looks like a password or a token. The values aren't shown here. | Hacivat parolaya ya da belirtece benzeyen metin buldu. Değerler burada gösterilmez. |
| `hv-scripts-secret-line` | Line {line} | Satır {line} |
| `hv-scripts-secret-git` | If you commit this file, the value is shared with everyone who can read the repository. | Bu dosyayı commit ederseniz değer, depoyu okuyabilen herkesle paylaşılır. |
| `hv-scripts-secret-allow` | To keep a line on purpose, add the comment -- {marker} (with a space after the dashes) or /* {marker} */ on it. | Bir satırı bilerek tutmak için o satıra -- {marker} (tirelerden sonra bir boşlukla) ya da /* {marker} */ yorumunu ekleyin. |
| `hv-scripts-secret-remember` | Save anyway is remembered for this tab until you quit. | Yine de kaydet, siz çıkana kadar bu sekme için hatırlanır. |
| `hv-scripts-secret-save` | Save anyway | Yine de kaydet |

#### Surface 9 · Large files

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hui-editor-t2-title` | Read-only preview: this file is {size}. | Salt okunur önizleme: bu dosya {size}. |
| `hui-editor-t2-body` | The editor shows its first {preview}. Run as script sends the whole file. | Düzenleyici dosyanın ilk {preview} kısmını gösterir. Betik olarak çalıştır dosyanın tamamını gönderir. |
| `hui-editor-paste-confirm-title` | Paste {size}? | {size} yapıştırılsın mı? |
| `hui-editor-paste-confirm-body` | That makes this document {total}, too large to edit. Paste it as a read-only preview, or cancel. | Bu, belgeyi düzenlenemeyecek kadar büyütür ({total}). Salt okunur önizleme olarak yapıştırın ya da vazgeçin. |
| `hui-editor-paste-confirm-preview` | Paste as preview | Önizleme olarak yapıştır |
| `hv-scripts-run-as-script` | Run as script | Betik olarak çalıştır |
| `hv-scripts-run-as-script-tip` | Sends the whole file from disk, one statement at a time, each through Run's checks. It stops at the first error. | Dosyanın tamamını diskten, her seferinde bir ifade olmak üzere ve her birini Çalıştır'ın denetimlerinden geçirerek gönderir. İlk hatada durur. |
| `hv-scripts-run-running` | Running as script | Betik olarak çalışıyor |
| `hv-scripts-run-sent` | {n} statements sent · [one] {n} statement sent | {n} ifade gönderildi |
| `hv-scripts-run-line` | line {line} | satır {line} |
| `hv-scripts-run-read` | {read} of {size} | {read} / {size} |
| `hv-scripts-run-busy` | Running as script: wait for it, or stop it | Betik olarak çalışıyor: bitmesini bekleyin ya da durdurun |
| `hv-scripts-run-stopped` | Stopped at statement {n}: the server returned an error. Nothing after it was sent. | {n}. ifadede durdu: sunucu hata döndürdü. Ondan sonrası gönderilmedi. |
| `hv-scripts-run-done` | Ran {n} statements from {file} · [one] Ran {n} statement from {file} | {file} dosyasından {n} ifade çalıştırıldı |
| `hv-scripts-run-cancelled` | Stopped after {n} statements. Nothing after them was sent. · [one] Stopped after {n} statement. Nothing after it was sent. | {n} ifadeden sonra durduruldu. Sonrası gönderilmedi. |
| `hv-scripts-size-mb` | {n} MB | {n} MB |

#### Surface 10 · Encoding

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-scripts-enc-title` | {file} isn't UTF-8 text. | {file} UTF-8 metin değil. |
| `hv-scripts-enc-body` | Choose the encoding it was saved in, or open it read-only to look first. | Kaydedildiği kodlamayı seçin ya da önce bakmak için salt okunur açın. |
| `hv-scripts-enc-label` | Encoding | Kodlama |
| `hv-scripts-enc-open` | Open | Aç |
| `hv-scripts-enc-ro` | Open read-only | Salt okunur aç |
| `hv-scripts-enc-ro-chip` | read-only | salt okunur |
| `hv-scripts-enc-ro-note` | Shown as UTF-8; characters it can't read show as �. Reopen with an encoding to edit. | UTF-8 olarak gösteriliyor; okunamayan karakterler � olarak görünür. Düzenlemek için bir kodlamayla yeniden açın. |
| `hv-scripts-enc-name` | {encoding} · [windows-1254] Windows-1254 (Turkish) · [iso-8859-9] ISO-8859-9 (Turkish) · [windows-1252] Windows-1252 (Western) · [utf-16le] UTF-16 LE | {encoding} · [windows-1254] Windows-1254 (Türkçe) · [iso-8859-9] ISO-8859-9 (Türkçe) · [windows-1252] Windows-1252 (Batı) · [utf-16le] UTF-16 LE |
| `hv-scripts-enc-bom` | {encoding} with BOM | {encoding}, BOM'lu |
| `hv-scripts-enc-seg-tip` | Encoding and line endings. Saving keeps both. | Kodlama ve satır sonları. Kaydetmek ikisini de korur. |
| `hv-scripts-enc-menu-head` | Encoding: {encoding} | Kodlama: {encoding} |
| `hv-scripts-enc-reopen` | Reopen with encoding… | Başka kodlamayla yeniden aç… |
| `hv-scripts-enc-save-with` | Save with encoding… | Başka kodlamayla kaydet… |
| `hv-scripts-enc-unencodable-title` | Couldn't save {file} | {file} kaydedilemedi |
| `hv-scripts-enc-unencodable-body` | {char} on line {line} can't be written in {encoding}. Remove it, or use Save with encoding…. | {line}. satırdaki {char} karakteri {encoding} ile yazılamaz. Silin ya da Başka kodlamayla kaydet… seçeneğini kullanın. |

#### Surface 11 · Diagnostics

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-scripts-diag-title` | {n} problems in {library} · [one] {n} problem in {library} | {library} içinde {n} sorun |
| `hv-scripts-diag-unknown-env` | {folder} is not an environment tag — files under it are hidden (did you mean {suggestion}?) | {folder} bir ortam etiketi değil; altındaki dosyalar gizlendi ({suggestion} mı demek istediniz?) |
| `hv-scripts-diag-unknown-env-nosug` | {folder} is not an environment tag — files under it are hidden. | {folder} bir ortam etiketi değil; altındaki dosyalar gizlendi. |
| `hv-scripts-diag-env-help` | Environment folders are @local, @dev, @test, @staging and @production. | Ortam klasörleri @local, @dev, @test, @staging ve @production'dır. |
| `hv-scripts-diag-contradict` | {path} names two environments, so the files under it are hidden everywhere. | {path} iki ortam adlandırıyor; altındaki dosyalar her yerde gizlendi. |
| `hv-scripts-diag-limit` | Listing stopped at {n} files; the rest of the folder isn't shown. · [depth] Folders deeper than {n} levels aren't listed. | Listeleme {n} dosyada durdu; klasörün geri kalanı gösterilmiyor. · [depth] {n} düzeyden derin klasörler listelenmez. |
| `hv-scripts-diag-polling` | This folder is on a network drive, so changes are checked every 2 seconds. · [watches] The system's file-watch limit is low, so changes are checked every 2 seconds. | Bu klasör bir ağ sürücüsünde; değişiklikler 2 saniyede bir denetleniyor. · [watches] Sistemin dosya izleme sınırı düşük; değişiklikler 2 saniyede bir denetleniyor. |
| `hv-scripts-diag-toml` | {file} isn't valid TOML, so the library binds by its folder name, {folder}. · [newer] {file} was written by a newer Hacivat, so the library binds by its folder name, {folder}. · [large] {file} is larger than 64 KB, so it isn't read; the library binds by its folder name, {folder}. | {file} geçerli bir TOML değil; kitaplık klasör adıyla, {folder} olarak bağlanıyor. · [newer] {file} daha yeni bir Hacivat ile yazılmış; kitaplık klasör adıyla, {folder} olarak bağlanıyor. · [large] {file} 64 KB'tan büyük olduğu için okunmadı; kitaplık klasör adıyla, {folder} olarak bağlanıyor. |
| `hv-scripts-diag-toml-keys` | {file} has keys Hacivat doesn't use. They are ignored. | {file} Hacivat'ın kullanmadığı anahtarlar içeriyor. Bunlar yok sayılır. |
| `hv-scripts-diag-parser` | Parser message | Ayrıştırıcı mesajı |
| `hv-scripts-diag-untrusted` | Git status is off: the repository belongs to another user. | Git durumu kapalı: depo başka bir kullanıcıya ait. |
| `hv-scripts-diag-untrusted-help` | Git itself refuses such a repository until it is marked safe (safe.directory). Hacivat shows no badges for it. | Git de böyle bir depoyu güvenli olarak işaretlenene kadar (safe.directory) reddeder. Hacivat onun için rozet göstermez. |
| `hv-scripts-diag-missing` | The folder isn't there: {path}. It may be on a drive that isn't connected. | Klasör yerinde yok: {path}. Bağlı olmayan bir sürücüde olabilir. |

#### Surface 12 · Recovery

| Fluent id | EN (final) | TR (draft, `# Review: PO`) |
|---|---|---|
| `hv-scripts-recover-title` | Recovered unsaved changes to {n} scripts · [one] Recovered unsaved changes to {n} script | {n} betikteki kaydedilmemiş değişiklikler kurtarıldı |
| `hv-scripts-recover-body` | Hacivat closed before they were saved. Their tabs are open again with your changes; save to keep them. | Hacivat bunlar kaydedilmeden kapandı. Sekmeleri değişikliklerinizle yeniden açık; korumak için kaydedin. |
| `hv-scripts-recover-show` | Show | Göster |
| `hv-scripts-recover-dismiss` | Dismiss | Kapat |
| `hv-scripts-recover-tab` | Recovered unsaved changes from {date}. | {date} tarihli kaydedilmemiş değişiklikler kurtarıldı. |
| `hv-scripts-recover-tab-body` | Save keeps them. Discard goes back to the file on disk. | Kaydet onları korur. At, diskteki dosyaya geri döner. |
| `hv-scripts-recover-discard` | Discard… | Değişiklikleri at… |
| `hv-scripts-recover-missing` | {file} isn't in the library any more. · [library] The library {library} isn't attached any more. | {file} artık kitaplıkta yok. · [library] {library} kitaplığı artık bağlı değil. |
| `hv-scripts-recover-missing-body` | Your recovered text is in this tab. Save writes it back to the same place. | Kurtarılan metniniz bu sekmede. Kaydet onu aynı yere geri yazar. |

### Proposed FTL, English (`locales/en/hacivat-ui.ftl`)

```ftl
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
hv-cmd-scripts-save = Save script
# Surface 7 (Go to script).
hv-cmd-scripts-save-all = Save all scripts
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
```

### Proposed FTL, English (`locales/en/hui.ftl`, the editor crate)

```ftl
## m4b-t11-editor
# Surface 9 (Large files).
# Variables:
#   $size (String) - a size (an hv-scripts-size-mb message), e.g. "148 MB".
hui-editor-t2-title = Read-only preview: this file is { $size }.
# Surface 9 (Large files).
# Variables:
#   $preview (String) - the preview size (an hv-scripts-size-mb message), e.g. "2.5 MB".
hui-editor-t2-body = The editor shows its first { $preview }. Run as script sends the whole file.
# Surface 9 (Large files).
# Variables:
#   $size (String) - a size (an hv-scripts-size-mb message), e.g. "148 MB".
hui-editor-paste-confirm-title = Paste { $size }?
# Surface 9 (Large files).
# Variables:
#   $total (Number) - the total, e.g. 2.
hui-editor-paste-confirm-body = That makes this document { $total }, too large to edit. Paste it as a read-only preview, or cancel.
# Surface 9 (Large files).
hui-editor-paste-confirm-preview = Paste as preview
```

### Proposed FTL, Turkish (`locales/tr/hacivat-ui.ftl`)

```ftl
## m4b-t01b-seams
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-node = Betikler
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-node-shared = paylaşılan
# Surface 1 (Scripts node).
# Variables:
#   $conns (String) - the other connections, a locale list of names (data), e.g. "orders-staging and orders-dev".
# Review: PO
hv-scripts-node-shared-tip = Ayrıca şunların altında: { $conns }. Bu adda veritabanı olan her bağlantı aynı dosyaları gösterir.
# Surface 1 (Scripts node).
# Variables:
#   $path (String) - a folder or file path, verbatim (data), e.g. "~/work/orders-scripts".
#   $database (String) - a database name, verbatim (data), e.g. "orders".
# Review: PO
hv-scripts-lib-tip = Klasör: { $path }. { $database } veritabanına bağlanır.
# Surface 1 (Scripts node).
# Variables:
#   $env (String) - the environment word (an env-* message), e.g. "production".
# Review: PO
hv-scripts-env-folder-tip = Bu klasörü yalnızca { $env } etiketli bağlantılar görür.
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-git-modified = M
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-git-modified-tip = Değiştirildi: son commit'ten farklı
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-git-new = N
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-git-new-tip = Yeni: son commit'te yok
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-git-conflicted = C
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-git-conflicted-tip = Çakışma: birleştirme çözülmedi
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-git-unknown = ?
# Surface 1 (Scripts node).
# Variables:
#   $reason (String) - selector: the reason; one of dataless, filter (anything else: the default).
# Review: PO
hv-scripts-git-unknown-tip =
    { $reason ->
        [dataless] Git durumu bilinmiyor: dosya indirilmedi; Hacivat karşılaştırmak için onu indirmez.
       *[filter] Git durumu bilinmiyor: dosyaya bir .gitattributes filtresi uygulanıyor; Hacivat filtreleri hiç çalıştırmaz.
    }
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-git-contains = •
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-git-contains-tip = Değişmiş dosyalar içeriyor
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-outside-link-tip = Kitaplığın dışına işaret ediyor; Hacivat bunu hiç açmaz.
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-dataless = indirilmedi
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-downloading = İndiriliyor…
# Surface 1 (Scripts node).
# Variables:
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-diag-row = { $n } sorun
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-missing-row = Klasör bulunamadı
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-loading = Betikler listeleniyor…
# Surface 2 (Context menus).
# Review: PO
hv-explorer-menu-attach-library = Betik kitaplığı bağla…
# Surface 2 (Context menus).
# Review: PO
hv-explorer-menu-map-library = Bir betik kitaplığına eşle…
# Surface 2 (Context menus).
# Review: PO
hv-explorer-menu-map-library-none = Bağlı başka kitaplık yok
# Surface 2 (Context menus).
# Review: PO
hv-scripts-menu-new-script = Yeni betik
# Surface 2 (Context menus).
# Review: PO
hv-scripts-menu-new-folder = Yeni klasör
# Surface 2 (Context menus).
# Review: PO
hv-scripts-menu-open = Aç
# Surface 2 (Context menus).
# Review: PO
hv-scripts-menu-open-under = Başka bir bağlantıda aç…
# Surface 2 (Context menus).
# Review: PO
hv-scripts-menu-rename = Yeniden adlandır
# Surface 2 (Context menus).
# Variables:
#   $os (String) - selector: the host (macos, windows, linux); one of other, windows (anything else: the default).
# Review: PO
hv-scripts-menu-trash =
    { $os ->
        [windows] Geri Dönüşüm Kutusu'na taşı
       *[other] Çöp Sepeti'ne taşı
    }
# Surface 2 (Context menus).
# Variables:
#   $os (String) - selector: the host (macos, windows, linux); one of linux, macos, windows (anything else: the default).
# Review: PO
hv-scripts-menu-reveal =
    { $os ->
        [windows] Dosya Gezgini'nde göster
        [linux] Bulunduğu klasörü aç
       *[macos] Finder'da göster
    }
# Surface 2 (Context menus).
# Review: PO
hv-scripts-menu-copy-path = Yolu kopyala
# Surface 2 (Context menus).
# Review: PO
hv-scripts-menu-copy-rel = Göreli yolu kopyala
# Surface 2 (Context menus).
# Review: PO
hv-scripts-menu-settings = Betik kitaplığı ayarları…
# Surface 2 (Context menus).
# Review: PO
hv-scripts-menu-outside-tip = Kitaplığın dışına işaret ediyor
# Surface 2 (Context menus).
# Review: PO
hv-scripts-new-name = adsız.sql
# Surface 2 (Context menus).
# Review: PO
hv-scripts-new-folder-name = yeni klasör
# Surface 2 (Context menus).
# Variables:
#   $reason (String) - selector: the reason; one of dots, empty, exists, ext, reserved, slash (anything else: the default).
#   $name (String) - a file or folder name, verbatim (data), e.g. "archive_old_orders.sql".
# Review: PO
hv-scripts-name-err =
    { $reason ->
        [slash] Ad / ya da \ içeremez.
        [reserved] Windows bu adı aygıtlar için ayırır (CON, PRN, AUX, NUL, COM1…).
        [exists] { $name } bu klasörde zaten var.
        [dots] Ad . ya da .. olamaz; nokta ya da boşlukla bitemez.
        [ext] Betik adı .sql ya da .md ile biter.
       *[empty] Bir ad yazın.
    }
# Surface 2 (Context menus).
# Variables:
#   $os (String) - selector: the host (macos, windows, linux); one of other, windows (anything else: the default).
#   $name (String) - a file or folder name, verbatim (data), e.g. "archive_old_orders.sql".
# Review: PO
hv-scripts-trashed =
    { $os ->
        [windows] { $name } Geri Dönüşüm Kutusu'na taşındı. Oradan geri yükleyebilirsiniz.
       *[other] { $name } Çöp Sepeti'ne taşındı. Oradan geri alabilirsiniz.
    }
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-choose = Betik klasörü seçin
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-title = Betik kitaplığı bağla
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-folder = Klasör
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-change = Değiştir…
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-database = Veritabanı
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-database-hint = Bu betiklerin ait olduğu veritabanı. Bu adda veritabanı olan her bağlantı onları gösterir.
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-aliases = Takma adlar
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-aliases-ph = Başka adları, virgülle ayırarak
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-shows-under = Görüneceği yerler
# Surface 3 (Attach dialog).
# Variables:
#   $database (String) - a database name, verbatim (data), e.g. "orders".
# Review: PO
hv-scripts-attach-shows-none = Henüz { $database } adlı veritabanı olan bağlantı yok.
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-will-write = Hacivat klasörde şu dosyayı oluşturur:
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-file-note = Yalnızca veritabanı adını ve takma adlarını içerir; bağlantı, sunucu ya da parola asla yazılmaz. Ekibiniz aynı bağlamayı alsın diye commit edin.
# Surface 3 (Attach dialog).
# Variables:
#   $database (String) - a database name, verbatim (data), e.g. "orders".
# Review: PO
hv-scripts-attach-found = .hacivat/workspace.toml bulundu — { $database } veritabanına bağlar
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-found-note = Hacivat bu dosyayı okur, hiç değiştirmez.
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-btn = Bağla
# Surface 3 (Attach dialog).
# Variables:
#   $reason (String) - selector: the reason; one of already, control, empty, long, many (anything else: the default).
# Review: PO
hv-scripts-attach-err =
    { $reason ->
        [control] Veritabanı adı denetim karakteri içeremez.
        [long] Veritabanı adı en çok 128 bayt olabilir.
        [already] Bu klasör zaten bağlı.
        [many] Hacivat en çok 32 kitaplık tutar. Önce Ayarlar › Betik kitaplıkları'ndan birini kaldırın.
       *[empty] Veritabanının adını yazın.
    }
# Surface 3 (Attach dialog).
# Variables:
#   $database (String) - a database name, verbatim (data), e.g. "orders".
# Review: PO
hv-scripts-map-title = { $database } altında bir betik kitaplığı göster
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-map-library = Kitaplık
# Surface 3 (Attach dialog).
# Variables:
#   $database (String) - a database name, verbatim (data), e.g. "orders".
# Review: PO
hv-scripts-map-binds = { $database } veritabanına bağlı
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-map-note = Eşleme yalnızca bu bilgisayarda tutulur. Ekip arkadaşlarınız görmez.
# Surface 3 (Attach dialog).
# Variables:
#   $database (String) - a database name, verbatim (data), e.g. "orders".
# Review: PO
hv-scripts-map-alias = { $database } adını, bu klasörü kullanan herkes için takma ad olarak da ekle
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-map-alias-sub = .hacivat/workspace.toml dosyasına ekler, dosyadaki yorumları korur
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-map-btn = Burada göster
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-map-other = Başka klasör bağla…
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-settings-nav-scripts = Betik kitaplıkları
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-settings-section-scripts = Betik kitaplıkları
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-scripts-set-attach = Klasör bağla…
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-scripts-set-note = Kitaplık, .sql ve .md dosyalarından oluşan bir klasördür. Kaldırmak klasöre ve dosyalarına hiç dokunmaz.
# Surface 4 (Settings › Script libraries).
# Variables:
#   $state (String) - selector: the watcher state; one of limit, polling, unavailable, watching (anything else: the default).
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-set-status =
    { $state ->
        [polling] 2 saniyede bir denetleniyor
        [unavailable] Klasör bulunamadı
        [limit] { $n } dosyada durdu
       *[watching] Değişiklikler izleniyor
    }
# Surface 4 (Settings › Script libraries).
# Variables:
#   $reason (String) - selector: the reason; one of network, watches (anything else: the default).
# Review: PO
hv-scripts-set-status-tip =
    { $reason ->
        [watches] Sistemin dosya izleme sınırı düşük; Hacivat değişiklik bildirimi beklemek yerine bu klasörü 2 saniyede bir denetler.
       *[network] Bu klasör bir ağ sürücüsünde; Hacivat değişiklik bildirimi beklemek yerine onu 2 saniyede bir denetler.
    }
# Surface 4 (Settings › Script libraries).
# Variables:
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-set-files = { $n } betik
# Surface 4 (Settings › Script libraries).
# Variables:
#   $aliases (String) - the aliases, a locale list (data), e.g. "orders_p".
# Review: PO
hv-scripts-set-aliases = takma adlar: { $aliases }
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-scripts-set-git = git deposu
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-scripts-set-git-off = git durumu kapalı
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-scripts-set-remove = Kaldır…
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-scripts-set-mappings = Yerel eşlemeler
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-scripts-set-mapping-shows = gösterir:
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-scripts-set-mapping-remove = Kaldır
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-scripts-set-empty = Henüz betik kitaplığı yok. Betiklerini ait oldukları veritabanlarının altında görmek için .sql ve .md dosyalarından oluşan bir klasör bağlayın.
# Surface 4 (Settings › Script libraries).
# Variables:
#   $library (String) - a library's folder name, verbatim (data), e.g. "orders-scripts".
# Review: PO
hv-scripts-remove-title = { $library } Hacivat'tan kaldırılsın mı?
# Surface 4 (Settings › Script libraries).
# Variables:
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-remove-body = Klasör ve dosyaları, .hacivat/workspace.toml dahil, olduğu gibi kalır. Betikler düğümleri ve { $n } yerel eşlemesi kaldırılır.
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-scripts-remove-btn = Kaldır
# Surface 5 (Script tab).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
#   $conn (String) - a connection name, verbatim (data), e.g. "orders-prod".
# Review: PO
hv-scripts-tab-title = { $file } · { $conn }
# Surface 5 (Script tab).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
#   $conn (String) - a connection name, verbatim (data), e.g. "orders-prod".
#   $database (String) - a database name, verbatim (data), e.g. "orders".
# Review: PO
hv-scripts-tab-title-db = { $file } · { $conn } · { $database }
# Surface 5 (Script tab).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-tab-title-unbound = { $file } · bağlantı yok
# Surface 5 (Script tab).
# Variables:
#   $library (String) - a library's folder name, verbatim (data), e.g. "orders-scripts".
# Review: PO
hv-scripts-tab-tip-library = { $library } kitaplığından
# Surface 5 (Script tab).
# Variables:
#   $conn (String) - a connection name, verbatim (data), e.g. "orders-prod".
#   $database (String) - a database name, verbatim (data), e.g. "orders".
# Review: PO
hv-scripts-tab-tip-runs = { $conn } üzerinde, { $database } veritabanında çalışır
# Surface 5 (Script tab).
# Review: PO
hv-scripts-tab-tip-unbound = Hiçbir yerde çalışmaz: bağlantısı silindi
# Surface 5 (Script tab).
# Review: PO
hv-scripts-tab-tip-unsaved = Kaydedilmemiş değişiklikler
# Surface 5 (Script tab).
# Review: PO
hv-scripts-tab-save = Kaydet
# Surface 5 (Script tab).
# Review: PO
hv-scripts-tab-save-all = Tümünü kaydet
# Surface 5 (Script tab).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-tab-saved = { $file } kaydedildi
# Surface 5 (Script tab).
# Review: PO
hv-scripts-run-md = Markdown notları çalışmaz
# Surface 5 (Script tab).
# Review: PO
hv-scripts-run-unbound = Bu sekmenin bağlantısı silindi; hiçbir şey çalıştırmaz. Başka bir bağlantıda açın.
# Surface 5 (Script tab).
# Variables:
#   $database (String) - a database name, verbatim (data), e.g. "orders".
# Review: PO
hv-scripts-run-switching = { $database } veritabanına geçiliyor…
# Surface 5 (Script tab).
# Review: PO
hv-scripts-unbound-title = Bu sekmenin açıldığı bağlantı silindi.
# Surface 5 (Script tab).
# Review: PO
hv-scripts-unbound-body = Metin korunuyor. Bir bağlantıda açana kadar hiçbir yerde çalışmaz.
# Surface 5 (Script tab).
# Variables:
#   $database (String) - a database name, verbatim (data), e.g. "orders".
# Review: PO
hv-scripts-use-log = Bu sekmenin oturumu artık { $database } veritabanını kullanıyor
# Surface 6 (Changes on disk).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-disk-changed = { $file } diskte değişti.
# Surface 6 (Changes on disk).
# Review: PO
hv-scripts-disk-changed-body = Bu sekmedeki düzenlemeleriniz kaydedilmedi. Yeniden yükle onları atar; Benimkini koru onları tutar ve kaydederken üzerine yazmadan önce sorar.
# Surface 6 (Changes on disk).
# Review: PO
hv-scripts-disk-reload = Yeniden yükle
# Surface 6 (Changes on disk).
# Review: PO
hv-scripts-disk-keep = Benimkini koru
# Surface 6 (Changes on disk).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-disk-reload-title = Düzenlemeleriniz atılıp { $file } yeniden yüklensin mi?
# Surface 6 (Changes on disk).
# Review: PO
hv-scripts-disk-reload-body = Diskteki dosya bu sekmedeki metnin yerini alır. Düzenlemeleriniz atılır.
# Surface 6 (Changes on disk).
# Review: PO
hv-scripts-disk-reload-yes = At ve yeniden yükle
# Surface 6 (Changes on disk).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-disk-reloaded = { $file } diskte değiştiği için yeniden yüklendi
# Surface 6 (Changes on disk).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
#   $new (String) - the new file name, verbatim (data), e.g. "haftalık_rapor.sql".
# Review: PO
hv-scripts-disk-renamed = { $file } dosyasının adı { $new } oldu; bu sekme onu izliyor
# Surface 6 (Changes on disk).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-disk-deleted = { $file } diskten silindi.
# Surface 6 (Changes on disk).
# Review: PO
hv-scripts-disk-deleted-body = Metin bu sekmede duruyor. Kaydet dosyayı yeniden yazar.
# Surface 6 (Changes on disk).
# Review: PO
hv-scripts-disk-overwrite-title = Dosya siz açtıktan sonra diskte değişti. Üzerine yazılsın mı?
# Surface 6 (Changes on disk).
# Review: PO
hv-scripts-disk-overwrite-body = Kaydetmek, Hacivat dışında yapılan değişiklikler dahil, diskteki sürümün yerini alır.
# Surface 6 (Changes on disk).
# Review: PO
hv-scripts-disk-overwrite = Üzerine yaz
# Surface 6 (Changes on disk).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
#   $conn (String) - a connection name, verbatim (data), e.g. "orders-prod".
# Review: PO
hv-scripts-unlisted = { $file } artık { $conn } altında listelenmiyor.
# Surface 6 (Changes on disk).
# Variables:
#   $reason (String) - selector: the reason; one of binding, env (anything else: the default).
#   $folder (String) - a folder name, verbatim (data), e.g. "@prod".
#   $conn (String) - a connection name, verbatim (data), e.g. "orders-prod".
# Review: PO
hv-scripts-unlisted-body =
    { $reason ->
        [binding] Kitaplığı artık bu veritabanına bağlı değil. Bu sekme yine yalnızca { $conn } üzerinde çalışır.
       *[env] Başka bir ortamın klasörüne, { $folder } içine taşındı. Bu sekme yine yalnızca { $conn } üzerinde çalışır.
    }
# Surface 7 (Go to script).
# Review: PO
hv-cmd-scripts-goto = Betiğe git
# Surface 7 (Go to script).
# Review: PO
hv-cmd-scripts-save = Betiği kaydet
# Surface 7 (Go to script).
# Review: PO
hv-cmd-scripts-save-all = Tüm betikleri kaydet
# Surface 7 (Go to script).
# Review: PO
hv-cmd-scripts-attach = Betik kitaplığı bağla…
# Surface 7 (Go to script).
# Variables:
#   $os (String) - selector: the host (macos, windows, linux); one of linux, macos, windows (anything else: the default).
# Review: PO
hv-cmd-scripts-reveal =
    { $os ->
        [windows] Betiği Dosya Gezgini'nde göster
        [linux] Betiğin klasörünü aç
       *[macos] Betiği Finder'da göster
    }
# Surface 7 (Go to script).
# Review: PO
hv-scripts-goto-ph = Ada göre betiğe git
# Surface 7 (Go to script).
# Review: PO
hv-scripts-goto-none = Eşleşen betik yok
# Surface 7 (Go to script).
# Review: PO
hv-scripts-goto-empty = Henüz betik kitaplığı yok
# Surface 7 (Go to script).
# Review: PO
hv-scripts-goto-empty-body = Bir veritabanının menüsünden ya da Ayarlar › Betik kitaplıkları'ndan klasör bağlayın.
# Surface 7 (Go to script).
# Review: PO
hv-scripts-goto-hint-open = Aç
# Surface 7 (Go to script).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-choose-title = { $file } şu bağlantıda açılsın:
# Surface 7 (Go to script).
# Review: PO
hv-scripts-choose-current = geçerli
# Surface 7 (Go to script).
# Review: PO
hv-scripts-choose-prod-note = Üretim bağlantısı sizin yerinize hiç seçilmez.
# Surface 7 (Go to script).
# Review: PO
hv-scripts-choose-none = Bu dosyayı gösteren bağlantı yok.
# Surface 7 (Go to script).
# Review: PO
hv-scripts-choose-none-row = Bağlantı olmadan aç
# Surface 7 (Go to script).
# Review: PO
hv-scripts-choose-none-body = Bağlantısız açılır; bir bağlantıda açana kadar hiçbir şey çalıştırmaz.
# Surface 7 (Go to script).
# Review: PO
hv-scripts-choose-hint-choose = Seç
# Surface 7 (Go to script).
# Review: PO
hv-scripts-choose-hint-back = Geri
# Surface 8 (Secret warning).
# Variables:
#   $kind (String) - selector: the finding's kind; one of key, password, token, url (anything else: the default).
# Review: PO
hv-scripts-secret-gutter =
    { $kind ->
        [token] Erişim belirtecine benziyor
        [key] Özel anahtara benziyor
        [url] Parola içeren bir bağlantı adresine benziyor
       *[password] Parolaya benziyor
    }
# Surface 8 (Secret warning).
# Variables:
#   $marker (String) - the literal hacivat:allow-secret (never translated).
# Review: PO
hv-scripts-secret-gutter-hint = Kaydederken önce sorulur. Bu satırdaki -- { $marker } gibi bir yorum (tirelerden sonra bir boşlukla) onu susturur.
# Surface 8 (Secret warning).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-secret-title = { $file } içinde { $n } satır gizli bilgiye benziyor
# Surface 8 (Secret warning).
# Review: PO
hv-scripts-secret-body = Hacivat parolaya ya da belirtece benzeyen metin buldu. Değerler burada gösterilmez.
# Surface 8 (Secret warning).
# Variables:
#   $line (Number) - a line number, shown without grouping, e.g. 48213.
# Review: PO
hv-scripts-secret-line = Satır { NUMBER($line, useGrouping: "false") }
# Surface 8 (Secret warning).
# Review: PO
hv-scripts-secret-git = Bu dosyayı commit ederseniz değer, depoyu okuyabilen herkesle paylaşılır.
# Surface 8 (Secret warning).
# Variables:
#   $marker (String) - the literal hacivat:allow-secret (never translated).
# Review: PO
hv-scripts-secret-allow = Bir satırı bilerek tutmak için o satıra -- { $marker } (tirelerden sonra bir boşlukla) ya da /* { $marker } */ yorumunu ekleyin.
# Surface 8 (Secret warning).
# Review: PO
hv-scripts-secret-remember = Yine de kaydet, siz çıkana kadar bu sekme için hatırlanır.
# Surface 8 (Secret warning).
# Review: PO
hv-scripts-secret-save = Yine de kaydet
# Surface 9 (Large files).
# Review: PO
hv-scripts-run-as-script = Betik olarak çalıştır
# Surface 9 (Large files).
# Review: PO
hv-scripts-run-as-script-tip = Dosyanın tamamını diskten, her seferinde bir ifade olmak üzere ve her birini Çalıştır'ın denetimlerinden geçirerek gönderir. İlk hatada durur.
# Surface 9 (Large files).
# Review: PO
hv-scripts-run-running = Betik olarak çalışıyor
# Surface 9 (Large files).
# Variables:
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-run-sent = { $n } ifade gönderildi
# Surface 9 (Large files).
# Variables:
#   $line (Number) - a line number, shown without grouping, e.g. 48213.
# Review: PO
hv-scripts-run-line = satır { NUMBER($line, useGrouping: "false") }
# Surface 9 (Large files).
# Variables:
#   $read (String) - bytes read so far (an hv-scripts-size-mb message), e.g. "38 MB".
#   $size (String) - a size (an hv-scripts-size-mb message), e.g. "148 MB".
# Review: PO
hv-scripts-run-read = { $read } / { $size }
# Surface 9 (Large files).
# Review: PO
hv-scripts-run-busy = Betik olarak çalışıyor: bitmesini bekleyin ya da durdurun
# Surface 9 (Large files).
# Variables:
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-run-stopped = { $n }. ifadede durdu: sunucu hata döndürdü. Ondan sonrası gönderilmedi.
# Surface 9 (Large files).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-run-done = { $file } dosyasından { $n } ifade çalıştırıldı
# Surface 9 (Large files).
# Variables:
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-run-cancelled = { $n } ifadeden sonra durduruldu. Sonrası gönderilmedi.
# Surface 9 (Large files).
# Variables:
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-size-mb = { $n } MB
# Surface 10 (Encoding).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-enc-title = { $file } UTF-8 metin değil.
# Surface 10 (Encoding).
# Review: PO
hv-scripts-enc-body = Kaydedildiği kodlamayı seçin ya da önce bakmak için salt okunur açın.
# Surface 10 (Encoding).
# Review: PO
hv-scripts-enc-label = Kodlama
# Surface 10 (Encoding).
# Review: PO
hv-scripts-enc-open = Aç
# Surface 10 (Encoding).
# Review: PO
hv-scripts-enc-ro = Salt okunur aç
# Surface 10 (Encoding).
# Review: PO
hv-scripts-enc-ro-chip = salt okunur
# Surface 10 (Encoding).
# Review: PO
hv-scripts-enc-ro-note = UTF-8 olarak gösteriliyor; okunamayan karakterler � olarak görünür. Düzenlemek için bir kodlamayla yeniden açın.
# Surface 10 (Encoding).
# Variables:
#   $enc (String) - selector: the encoding id; one of iso-8859-9, other, utf-16le, windows-1252, windows-1254 (anything else: the default).
#   $encoding (String) - an encoding's name (data), e.g. "Windows-1254".
# Review: PO
hv-scripts-enc-name =
    { $enc ->
        [windows-1254] Windows-1254 (Türkçe)
        [iso-8859-9] ISO-8859-9 (Türkçe)
        [windows-1252] Windows-1252 (Batı)
        [utf-16le] UTF-16 LE
       *[other] { $encoding }
    }
# Surface 10 (Encoding).
# Variables:
#   $encoding (String) - an encoding's name (data), e.g. "Windows-1254".
# Review: PO
hv-scripts-enc-bom = { $encoding }, BOM'lu
# Surface 10 (Encoding).
# Review: PO
hv-scripts-enc-seg-tip = Kodlama ve satır sonları. Kaydetmek ikisini de korur.
# Surface 10 (Encoding).
# Variables:
#   $encoding (String) - an encoding's name (data), e.g. "Windows-1254".
# Review: PO
hv-scripts-enc-menu-head = Kodlama: { $encoding }
# Surface 10 (Encoding).
# Review: PO
hv-scripts-enc-reopen = Başka kodlamayla yeniden aç…
# Surface 10 (Encoding).
# Review: PO
hv-scripts-enc-save-with = Başka kodlamayla kaydet…
# Surface 10 (Encoding).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-enc-unencodable-title = { $file } kaydedilemedi
# Surface 10 (Encoding).
# Variables:
#   $line (Number) - a line number, shown without grouping, e.g. 48213.
#   $char (String) - the character that cannot be encoded (data), e.g. "漢".
#   $encoding (String) - an encoding's name (data), e.g. "Windows-1254".
# Review: PO
hv-scripts-enc-unencodable-body = { NUMBER($line, useGrouping: "false") }. satırdaki { $char } karakteri { $encoding } ile yazılamaz. Silin ya da Başka kodlamayla kaydet… seçeneğini kullanın.
# Surface 11 (Diagnostics).
# Variables:
#   $library (String) - a library's folder name, verbatim (data), e.g. "orders-scripts".
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-diag-title = { $library } içinde { $n } sorun
# Surface 11 (Diagnostics).
# Variables:
#   $folder (String) - a folder name, verbatim (data), e.g. "@prod".
#   $suggestion (String) - the tag it probably meant, e.g. "@production".
# Review: PO
hv-scripts-diag-unknown-env = { $folder } bir ortam etiketi değil; altındaki dosyalar gizlendi ({ $suggestion } mı demek istediniz?)
# Surface 11 (Diagnostics).
# Variables:
#   $folder (String) - a folder name, verbatim (data), e.g. "@prod".
# Review: PO
hv-scripts-diag-unknown-env-nosug = { $folder } bir ortam etiketi değil; altındaki dosyalar gizlendi.
# Surface 11 (Diagnostics).
# Review: PO
hv-scripts-diag-env-help = Ortam klasörleri @local, @dev, @test, @staging ve @production'dır.
# Surface 11 (Diagnostics).
# Variables:
#   $path (String) - a folder or file path, verbatim (data), e.g. "~/work/orders-scripts".
# Review: PO
hv-scripts-diag-contradict = { $path } iki ortam adlandırıyor; altındaki dosyalar her yerde gizlendi.
# Surface 11 (Diagnostics).
# Variables:
#   $limit (String) - selector: which limit; one of depth, files (anything else: the default).
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-diag-limit =
    { $limit ->
        [depth] { $n } düzeyden derin klasörler listelenmez.
       *[files] Listeleme { $n } dosyada durdu; klasörün geri kalanı gösterilmiyor.
    }
# Surface 11 (Diagnostics).
# Variables:
#   $reason (String) - selector: the reason; one of network, watches (anything else: the default).
# Review: PO
hv-scripts-diag-polling =
    { $reason ->
        [watches] Sistemin dosya izleme sınırı düşük; değişiklikler 2 saniyede bir denetleniyor.
       *[network] Bu klasör bir ağ sürücüsünde; değişiklikler 2 saniyede bir denetleniyor.
    }
# Surface 11 (Diagnostics).
# Variables:
#   $reason (String) - selector: the reason; one of invalid, large, newer (anything else: the default).
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
#   $folder (String) - a folder name, verbatim (data), e.g. "@prod".
# Review: PO
hv-scripts-diag-toml =
    { $reason ->
        [newer] { $file } daha yeni bir Hacivat ile yazılmış; kitaplık klasör adıyla, { $folder } olarak bağlanıyor.
        [large] { $file } 64 KB'tan büyük olduğu için okunmadı; kitaplık klasör adıyla, { $folder } olarak bağlanıyor.
       *[invalid] { $file } geçerli bir TOML değil; kitaplık klasör adıyla, { $folder } olarak bağlanıyor.
    }
# Surface 11 (Diagnostics).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-diag-toml-keys = { $file } Hacivat'ın kullanmadığı anahtarlar içeriyor. Bunlar yok sayılır.
# Surface 11 (Diagnostics).
# Review: PO
hv-scripts-diag-parser = Ayrıştırıcı mesajı
# Surface 11 (Diagnostics).
# Review: PO
hv-scripts-diag-untrusted = Git durumu kapalı: depo başka bir kullanıcıya ait.
# Surface 11 (Diagnostics).
# Review: PO
hv-scripts-diag-untrusted-help = Git de böyle bir depoyu güvenli olarak işaretlenene kadar (safe.directory) reddeder. Hacivat onun için rozet göstermez.
# Surface 11 (Diagnostics).
# Variables:
#   $path (String) - a folder or file path, verbatim (data), e.g. "~/work/orders-scripts".
# Review: PO
hv-scripts-diag-missing = Klasör yerinde yok: { $path }. Bağlı olmayan bir sürücüde olabilir.
# Surface 12 (Recovery).
# Variables:
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-recover-title = { $n } betikteki kaydedilmemiş değişiklikler kurtarıldı
# Surface 12 (Recovery).
# Review: PO
hv-scripts-recover-body = Hacivat bunlar kaydedilmeden kapandı. Sekmeleri değişikliklerinizle yeniden açık; korumak için kaydedin.
# Surface 12 (Recovery).
# Review: PO
hv-scripts-recover-show = Göster
# Surface 12 (Recovery).
# Review: PO
hv-scripts-recover-dismiss = Kapat
# Surface 12 (Recovery).
# Variables:
#   $date (DateTime) - when the recovery copy was written, e.g. 2026-11-21 14:05.
# Review: PO
hv-scripts-recover-tab = { DATETIME($date, dateStyle: "medium", timeStyle: "short") } tarihli kaydedilmemiş değişiklikler kurtarıldı.
# Surface 12 (Recovery).
# Review: PO
hv-scripts-recover-tab-body = Kaydet onları korur. At, diskteki dosyaya geri döner.
# Surface 12 (Recovery).
# Review: PO
hv-scripts-recover-discard = Değişiklikleri at…
# Surface 12 (Recovery).
# Variables:
#   $what (String) - selector: what is missing; one of file, library (anything else: the default).
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
#   $library (String) - a library's folder name, verbatim (data), e.g. "orders-scripts".
# Review: PO
hv-scripts-recover-missing =
    { $what ->
        [library] { $library } kitaplığı artık bağlı değil.
       *[file] { $file } artık kitaplıkta yok.
    }
# Surface 12 (Recovery).
# Review: PO
hv-scripts-recover-missing-body = Kurtarılan metniniz bu sekmede. Kaydet onu aynı yere geri yazar.
# Surface 5 (Script tab).
# Variables:
#   $bound (String) - the database the tab is bound to (data), e.g. "kampanya".
#   $current (String) - the database the session is on (data), e.g. "api".
# Review: PO
hv-scripts-run-refused = Çalıştırma reddedildi: bu sekme { $bound } üzerinde çalışır, ama oturumu { $current } üzerinde.
# Surface 5 (Script tab).
# Variables:
#   $reason (String) - selector: the reason; one of edited, mysql (anything else: the default).
#   $bound (String) - the database the tab is bound to (data), e.g. "kampanya".
#   $conn (String) - a connection name, verbatim (data), e.g. "orders-prod".
#   $current (String) - the database the session is on (data), e.g. "api".
# Review: PO
hv-scripts-run-refused-body =
    { $reason ->
        [edited] { $conn } artık { $current } veritabanını açıyor: bağlantı düzenlendi. Hiçbir şey gönderilmedi. Betiği { $bound } veritabanını gösteren bağlantıda açın.
       *[mysql] Hacivat oturumu bir kez geri geçirdi, yine de { $bound } üzerinde değil. Hiçbir şey gönderilmedi. Yeniden deneyin ya da betiği başka bir bağlantıda açın.
    }
# Surface 5 (Script tab).
# Variables:
#   $bound (String) - the database the tab is bound to (data), e.g. "kampanya".
# Review: PO
hv-scripts-run-refused-retry = { $bound } veritabanına geç ve çalıştır
# Surface 5 (Script tab).
# Review: PO
hv-scripts-run-refused-unbound = Çalıştırma reddedildi: bu sekmenin bağlantısı yok.
# Surface 6b (Unsaved changes).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-unsaved-title = { $file } dosyasındaki değişiklikler kaydedilsin mi?
# Surface 6b (Unsaved changes).
# Review: PO
hv-scripts-unsaved-body = Kaydetmezseniz değişiklikleriniz kaybolur.
# Surface 6b (Unsaved changes).
# Variables:
#   $total (Number) - the total, e.g. 2.
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-unsaved-count = { $total } kaydedilmemiş betikten { $n }. betik
# Surface 6b (Unsaved changes).
# Review: PO
hv-scripts-unsaved-save = Kaydet
# Surface 6b (Unsaved changes).
# Review: PO
hv-scripts-unsaved-discard = Kaydetme
# Surface 1 (Scripts node).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-downloading-tab = { $file } indiriliyor…
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-downloading-body = Bulutta tutuluyor; indirme bitince metin görünür. O zamana dek hiçbir şey çalışmaz.
# Surface 2 (Context menus).
# Review: PO
hv-scripts-menu-open-under-none = Bu dosyayı gösteren başka bağlantı yok
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-scripts-remove-body-none = Klasör ve dosyaları, .hacivat/workspace.toml dahil, olduğu gibi kalır. Betikler düğümleri kaldırılır.
# Surface 1 (Scripts node).
# Variables:
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-node-row-tip = Bu bağlantının altında { $n } betik listeleniyor.
# Surface 7 (Go to script).
# Review: PO
hv-scripts-choose-hint-close = Kapat
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-dataless-row-tip = Bulutta tutuluyor, indirilmedi. Açınca önce indirilir.
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-downloading-row-tip = İndiriliyor… İndirme bitince sekme metni gösterir.
```

### Proposed FTL, Turkish (`locales/tr/hui.ftl`)

```ftl
## m4b-t11-editor
# Surface 9 (Large files).
# Variables:
#   $size (String) - a size (an hv-scripts-size-mb message), e.g. "148 MB".
# Review: PO
hui-editor-t2-title = Salt okunur önizleme: bu dosya { $size }.
# Surface 9 (Large files).
# Variables:
#   $preview (String) - the preview size (an hv-scripts-size-mb message), e.g. "2.5 MB".
# Review: PO
hui-editor-t2-body = Düzenleyici dosyanın ilk { $preview } kısmını gösterir. Betik olarak çalıştır dosyanın tamamını gönderir.
# Surface 9 (Large files).
# Variables:
#   $size (String) - a size (an hv-scripts-size-mb message), e.g. "148 MB".
# Review: PO
hui-editor-paste-confirm-title = { $size } yapıştırılsın mı?
# Surface 9 (Large files).
# Variables:
#   $total (Number) - the total, e.g. 2.
# Review: PO
hui-editor-paste-confirm-body = Bu, belgeyi düzenlenemeyecek kadar büyütür ({ $total }). Salt okunur önizleme olarak yapıştırın ya da vazgeçin.
# Surface 9 (Large files).
# Review: PO
hui-editor-paste-confirm-preview = Önizleme olarak yapıştır
```
