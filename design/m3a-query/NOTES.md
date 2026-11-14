# M3a "Run a query": design notes

Clickable mock: `design/m3a-query/index.html` (one file, one app, a surface switcher). It is the M2b mock's world, unchanged: the same CSS (copied, with the M3a additions after it), the same Lucide 0.460 sprite (ISC) plus five icons, the same mock bar idea, Notes drawer, toggles and deep links. It runs with no network except Google Fonts.

This file is the binding component reference for M3a (R-M3-30). The DESIGN.md "Query, Results and Grid" section is written once, in M3b.

**How to drive it**
- The dashed **mock bar** is not product UI. Row 1 holds the fourteen surfaces. Row 2 holds the scenarios of the current surface (`[` / `]`), the window size (Fit, 1280 × 800, 960 × 600; `W`), the caret blink (Blink / Static, open question 3), EN/TR (`L`), dark/light (`T`) and Notes (`N`).
- Keys `1`–`9`, `0`, `-`, `=` jump to surfaces 1–12. Surfaces 13 and 14 are reached with the bar or a deep link.
- Deep links: `?s=11&sc=memory&theme=light&lang=tr&size=960&bar=0`. `bar=0` hides the mock bar, so the window is exactly the viewport. `shot=1` freezes the clocks for screenshots.
- The Notes drawer repeats these notes per surface and lists the surface's strings (Fluent ids, EN, TR).
- In the mock, numbers and dates follow the language toggle as a stand-in for the format locale. The product follows the OS region (L3, L15(d)), so `tr-TR` digits can appear with an English UI.

**Flows that run**
- **Run.** Run (or ⌘↵) with one candidate runs at once; with two or more it opens the chooser. Hover and ↑/↓ preview each candidate; ↵ runs it, Esc restores the preview. Running shows the clock in the toolbar and the gutter (once a second), then rows stream, then the cursor suspends at 10,000+ rows.
- **Stop** (or ⌘.) while running: Cancelling…, then Cancelled in the gutter, the strip and the log.
- **Production.** Run on the `UPDATE` line of the `orders-prod` console: nothing is sent; the gutter shows the ink glyph with "not sent: read-only" and the Output tab shows the denial. The notice opens its popover by click or from the ⋯ menu.
- **Disconnected console** (Preview › No connection): Run starts the connect and runs nothing (R-M3-45).
- **Strip.** Fetch more adds 10,000 rows. Fetch all streams up to the memory banner; Fetch more anyway completes the result at 1,000,000 rows. Close cursor (strip or status bar) closes it with its reason. Re-run runs again.
- **Grid.** Click a cell, Shift-click to extend, click a row number, a header or the corner; ⌘C or Copy shows the copy toast (or the refusal over 1,000,000 cells).
- **Output.** Clicking an error line selects its statement in the editor. Tabs switch, and the status bar follows the active result tab.
- **Close a console tab** whose console has an open transaction (Close tx surface, or the × on the tab there) opens the prompt; Enter and Esc both cancel.
- The editor's context menu opens on right-click in the code.

**Product truth used:** the M3a plan (surfaces 1–14 and the M3b sketches in "New UI surfaces", rulings R-M3-6 REVISED, -8, -16 amended, -22 to -49, P15, P16, P25), its Global Constraints, DESIGN.md, `design/m2-connect/NOTES.md` and `design/m2b-secure/NOTES.md` (cross-cutting rules), harman (`design/directions/harman/index.html`: editor band, gutter, grid, strip, caret), D-011, D-014, D-035, D-065, the `CloseReason` and `DenyLayer` enums, and the shipped Fluent strings where a label already exists (`hv-tab-console-untitled`, `hv-editor-empty`, `hv-results-empty`, `hv-capsule-access-read-only`, the M2b access labels, `hui-palette-hint-*`, `hui-menu-more`).

---

## Cross-cutting decisions

1. **No new hue, no new filled block.** Blue means "this runs" or "this is selected" (the preview band, the gutter bar, the caret, the grid selection, the chooser's active row). Amber means running or pending (the run marker, Cancelling, the open-transaction marker). Green means succeeded (checks, the executed bar, the loaded scroll band). Red means production only (the hairline, swatches, locks on production). A failure is an ink glyph. The capsule's cp-fill block stays the window's one filled badge (checked in every render).
2. **The preview never shows a range Run would not send.** Anything uncertain or offline is drawn in neutral fills (`hover`) with text-2/text-3 bars, never in blue.
3. **The Raised Ink Rule holds.** Toolbars, the strip, active tabs, menus, popovers, tooltips and dialogs map text-3 to text-2. The editor and the grid sit on the panel, where text-3 is allowed (line numbers, end-of-line statuses, type subtitles, NULL). Field wells restore text-3.
4. **Server text is never inside a translated sentence.** The server's message sits verbatim in its own mono field: on its own row in the Output log, in a captioned box in the gutter tooltip. Sentences only interpolate values Hacivat chose or measured (connection names, counts, durations, sizes, SQLSTATE codes as chips).
5. **No ambient motion.** Skeleton rows are static. Clocks repaint at most once a second. The caret blinks at the OS rate and goes solid after 10 s idle and under reduced motion (open question 3). Nothing shimmers or spins.
6. **Every count and duration is one Fluent message with its unit inside** (`exec-dur-*`, `exec-size-*`, `hui-editor-size-mb`), formatted by the format locale. IDs and row and line numbers stay raw (no grouping). NULL is the constant `NULL` (D-065).
7. **The Overflow Rule decides, not ellipsis.** Toolbars hide low-priority items before any label truncates; trust signals (environment, read-only, transaction, cursor) never collapse. Only data ellipsizes (SQL labels, cell text, verbatim server text), always with the whole value in a tooltip.
8. **GPUI-reproducible.** Solid fills, 1 px borders (solid or dashed), radii 0/3/4/6, the popover shadow only. No transforms, filters, blur or masks. Positions in the code area are in whole Fragment Mono advances.
9. **Fits 960 × 600 in Turkish.** A headless pass rendered every scenario in EN/TR × dark/light at 1280 × 800 and 960 × 600 (see Verification).

---

## Surfaces

### 1 · Console editor area
**Intent.** A real editor bound to one connection. You always see where you are, what you typed and where the caret is, with no ambient motion.

**Anatomy.**
- Panel tone; code in Fragment Mono 12.5/18 with 6 px top padding. Lexical colours are DESIGN.md's syntax tokens; comments italic. A MySQL executable comment (`/*! … */`) is code: its tokens keep code colours on the inlay well (`seam-soft`), its delimiters (`/*!40101`, `*/`) are comment ink, not italic.
- Turkish identifiers (`"müşteri adı"`) and strings (`'İzmir'`, `'Şanlıurfa'`) are lexed as written; nothing is case-folded (INV-3).
- **Gutter**: 58 px for up to 3 digits, then + 1 Fragment Mono advance per extra digit (open question 4). Numbers right-aligned with 14 px right padding; the current line's number in `text`, the others `text-3`. Status glyphs sit at 8 px from the left; range bars are 1 px at 4 px (the preview bar moves to 2 px when an executed bar is already there).
- **Caret**: 2 × 16 px `blue`, 1 px above the line box. **Selection**: `blue-tint-strong` when focused; when unfocused, no caret and a neutral `seam-strong` selection (macOS's inactive selection), so it never reads as the Run preview.
- **Long line**: the text scrolls horizontally under a fixed gutter; a 4 px `seam-strong` thumb (radius 3) sits at the bottom of the code area.
- **IME**: the composition carries a 1 px `text` underline 3 px under the baseline; the active clause 2 px. The OS draws the candidate window under the composed range (from `bounds_for_range`); the dashed box in the mock only marks where it lands. The mock uses Pinyin (Latin marked text) so the advance grid stays exact.
- **Empty bound console**: one placeholder line in `text-3`, Source Sans 13: "Write SQL here. ⌘↵ runs the statement at the caret." (the chord comes from the keymap). It disappears with the first keystroke.
- **Production**: a 1 px `red` hairline across the top of the pane (over the tab strip's top edge), the 7 px swatch on the tab, the connection name in mono, and the read-only notice in the toolbar (surface 2). No filled block.
- **Read-only connection**: the editor stays editable; the toolbar carries the lock chip.
- **Unbound console**: exactly the M2 home, "Connect to a database to start writing SQL" (R-M3-37).
- **Tabs**: 30 px; a bound console's title is the connection name in mono (data), with ` (n)` from the second console of a profile; an unbound console is "Console 1". The active tab takes the raised tone and joins the toolbar.

**States.** Focused · Unfocused · Selection on one line · across lines · Long line · Line numbers at 1, 4, 5 and 7 digits · IME · Empty · Read-only connection · Production · MySQL executable comment · Unbound.

**Keyboard.** Typing, movement and selection as Task 25. Escape collapses the selection; Escape then Tab leaves the editor. No binding fires during a composition (F15).

### 2 · Editor toolbar
**Intent.** Run and Stop, where it runs and whether it can write, on one 32 px raised bar that never truncates a label.

**Anatomy** (left to right).
- **Run** (primary, play icon, `⌘↵` hint in on-accent ink). While running it becomes **Stop** (outline, square icon, `⌘.`); the slot keeps its width.
- **Run state**: the 7 px amber run marker, "Running" in `text`, and a mono 11.5 `text-2` clock ("3 s", repainted once a second). After Stop: a hollow amber marker and "Cancelling…"; Stop is disabled until the server answers.
- **Connection label**: a seam, the env mark, the connection name in mono 11.5 `text`, then a chevron and the database in mono `text-2`.
- **Waiting** (R-M3-43): a hollow `seam-strong` marker and "Waiting for a free connection to orders-dev (4 of 4 in use)", then Cancel (ghost). Run is disabled. When the bar is short the sentence becomes "Waiting for a connection (4 of 4 in use)" (the connection is already in the label; the tooltip keeps the whole).
- Right: a read-only connection shows its M2b access label as a lock chip (22 px, lock in `text-2`). **Production** shows the notice "Production is read-only in this version" with a `red` lock; it is a button that opens a 300 px explanation popover (title, why, what still works).
- A ⋯ button (ghost icon, "More") appears only when something moved into the overflow menu.

**States.** Idle · Running · Cancelling · Read-only chip · Production notice (popover open) · Waiting 4 of 4 · 960 × 600 in Turkish: a second production console waiting for a session, with the overflow menu open.

**Overflow Rule.** First the database crumb hides; then the production notice collapses to the lock chip "read-only" (still red, still a button) and "Why this console is read-only…" moves into the ⋯ menu; then the waiting sentence takes its short form. Run/Stop, the run state, the connection name, Cancel and the lock never hide. At 1280 × 800 nothing collapses in either language.

**Keyboard.** ⌘↵ runs, ⌘. stops (Windows/Linux: Ctrl+Break and Ctrl+F2, R-M3-38). The notice and the ⋯ button are Tab stops; Enter or Space opens them; Esc closes the popover and returns the focus to its trigger.

### 3 · Run preview band and current-statement bar
**Intent.** Before you press Run you can see exactly what it would send (R-M3-8). An uncertain boundary is drawn as uncertain and sends nothing (R-M3-24).

**Anatomy.**
- **One statement** that owns its lines: harman's full-row band, `blue-tint` at .55 (dark) / .8 (light), from the gutter's edge to the right edge; a 1 px `blue` bar in the gutter over its lines. A leading comment line is outside the band (it is the title, not sent).
- **Two statements on one line**: the exact-range tint, same colour, on the statement's characters only, without the terminator (open question 1). A statement that spans lines gets the full-row band on the lines it owns and the exact tint on the lines it shares.
- **A selection runs as separate statements**: the selection tint, one gutter bar per unit with a 2 px gap between units, and "1 of 3", "2 of 3", "3 of 3" at the end of each unit's first line (`text-3`).
- **Ask, preselected** (D-011): the band shows the candidate Enter would run; "2 choices" with the chevrons-up-down glyph sits at the end of its first line.
- **Nothing**: no band; the status bar says "Nothing to run here".
- **Uncertain**: the heuristic range in the neutral `hover` fill, a dashed 1 px `text-2` gutter bar, and "boundary uncertain" with the ink alert glyph at the end of the line; the status bar says "Boundary uncertain: select the statement to run it".
- **No connection**: the same geometry in the neutral fill, a solid `text-3` bar and "not connected" with the unplug glyph. Run connects first and runs nothing.
- **Chooser hover**: the band follows the highlighted candidate.
- **Composing**: the band is frozen at its last resolved range while marked text exists; it resolves again after the commit.

**States.** One statement · Two on a line · Two on a line, full row (the rejected option, for comparison) · Selection → 3 · Ask, preselected · Nothing · Uncertain · No connection · Chooser hover · Composing.

**Keyboard.** The band follows the caret and the selection; there is no key of its own. "Describe execution range" (surface 8) says it in words.

### 4 · Statement chooser (D-011)
**Intent.** When Run has more than one candidate, a popup at the caret lists them in resolve order and previews each; Enter runs the highlighted one.

**Anatomy.**
- The PopupMenu (raised, 1 px `seam-strong`, radius 6, popover shadow), 460 px wide (narrower when the window is), anchored under `EditorView::caret_bounds()`, flipped above when it would cross the status bar.
- A 12 px `text-2` header, "Choose what to run".
- 24 px rows: a 12 px check column (the blue check marks the kind remembered for this console), the kind in Source Sans 13 `text` (128 px column: Statement, Largest statement, From caret, Whole script), the whitespace-collapsed SQL in Fragment Mono 12 `text-2` (ellipsized; leading comments are not part of the label), and for multi-statement items "2 statements" in `text-2`.
- The active row is `blue-tint-strong` (the completion list's active item), and its label goes to `text`.
- A 24 px footer on `head`: ↑↓ Navigate, ↵ Run, esc Close.
- **Boundary uncertain**: no run item at all. The ink alert glyph and "Boundary uncertain" (13 px, 600), "Hacivat can't tell exactly where this statement ends, so it sent nothing.", the reason ("A ; sits inside an executable comment (/*! … */)."), and what to do ("Select exactly the text to run, then press ⌘↵."). Footer: esc Close.

- **Which kinds appear** (Task 13's dedupe): with the caret in the first of two statements, Statement and Whole script (From caret equals Whole script, so it is labelled Whole script). With the caret in the middle of six, Statement, From caret (5 statements) and Whole script (6). Largest statement appears only when the caret's statement sits inside a larger batch; no M3a fixture has one, so its row is the same anatomy.

**States.** Candidates · Three kinds · Remembered kind · Hover previews · Boundary uncertain · 960 in Turkish.

**Keyboard.** The popup takes the focus. ↑/↓ move and preview; ↵ (key-down, no repeat) runs the highlighted item; Esc closes and restores the preview. **No access keys** (P15): letters do nothing. The chosen kind is remembered per console for the session (R-M3-36).

### 5 · Gutter execution status
**Intent.** Every statement you ran keeps its outcome in the gutter and at the end of its first line, in words as well as glyphs.

**Anatomy.**

| State | Gutter glyph (12 px slot) | Range bar | End of line (micro 11.5 `text-3`, numbers mono 11) |
|---|---|---|---|
| Queued | 7 px hollow `seam-strong` square | — | "queued" |
| Running (after 50 ms) | the 7 px amber run marker | — | "running · 3 s" in amber, once a second |
| Succeeded | check, `green` | 1 px `green` over the statement | check + "executed · 42 ms · 100 rows" |
| Succeeded DML | check, `green` | 1 px `green` | check + "12 rows affected · 38 ms" |
| Failed | triangle-alert, `text` (ink) | — | "failed" in `text` + the SQLSTATE chip (mono 11, 1 px `seam-strong`, radius 3) |
| Not sent (read-only) | triangle-alert, `text` | — | "not sent: read-only" in `text` |
| Cancelling | 7 px hollow amber square | — | "cancelling…" in amber |
| Cancelled | ban, `text-2` | — | "cancelled after 12 s 408 ms" |
| Skipped | skip-forward, `text-2` | — | "skipped: an earlier statement failed" |
| Stale | check, `text-3` | 1 px `text-3` | "edited since execution" |

- **Failed tooltip** (popover anatomy, 320 px): "Statement 3 failed" with the SQLSTATE chip, then a captioned box "Server message" with the server's text verbatim (mono 11.5/17), then "Click to show it in Output". A click on the glyph opens it; Esc closes it.
- **Two marks on one row**: the worse glyph with a 3 px `text-2` dot at its lower right; the end of line lists both, divided by a `|`; the tooltip lists both statements with their outcomes.
- **A long line**: whenever the status would come within 12 px of its line's text, it sits on a panel pad (1 px `seam-soft` left edge) over the line's tail and switches to its short form: check + duration, the amber clock, or the SQLSTATE chip alone; queued, skipped and stale drop their words (the gutter glyph stays). The full text is in the tooltip and the Output tab. This happens often at 960 px.

**States.** Run in progress (live clock) · Stopped on error (StopOnError, R-M3-26) · Cancelling · Cancelled · Failed tooltip · Stale · Two on one row · Long line · Read-only denial (production).

**Keyboard.** Execution status is announced politely to screen readers (Task 29). The tooltip is pointer-first; the same text is in the Output tab.

### 6 · Status bar segments
**Intent.** The status bar follows the focused console and its active result tab (Status Follows The Tab), and says when a transaction or a cursor is open.

**Anatomy** (24 px, ground, caption `text-2`, seam-divided).
- Left: the environment segment (production: `red-tint`, red 600 text, swatch and lock; others: plain, mark, and the connection in mono), the path with latency, the encryption, then:
  - **Transaction open · 3 statements** (R-M3-27): the 7 px amber marker (pending), text in `text`; tooltip "The statements since BEGIN aren't committed yet. Commit or roll back from the command palette." Read-only.
  - **Transaction failed · roll back to end it** (R-M3-16 amended): the ink alert glyph, text in `text`; tooltip explains that only a rollback is accepted.
  - **Rows · timing** of the active result, in mono ("10,000+ rows · 131 ms").
  - **Cursor open**: a button with the lock-open glyph that closes the cursor; tooltip "Holding a database snapshot. Click to close the cursor."
  - A transient message in `text` ("Nothing to run here", the boundary-uncertain line, Describe execution range, the connect-first line).
- Right: the editor segment (its own entity, R-M3-11): Reduced mode · "186 selected" (characters, line breaks counted once, format locale) · "Ln 14, Col 49" (raw numbers, like the gutter: "Ln 1048570"); then the server version, then the language toggle.

**Collapse order** at narrow widths (first hidden first): server version, encryption, selected count, path, Ln/Col, rows · timing. The environment, transaction, cursor, message, Reduced mode and language never collapse.

**States.** Ln, Col · Selected · Reduced mode · Transaction open · Transaction failed · Cursor open · Production result · Staging result · 960 in Turkish with everything on.

**Keyboard.** Cursor open is a button (Tab stop in the status bar's focus group); Enter or Space closes the cursor.

### 7 · Large-file and clipboard notices
**Intent.** Large documents and clipboard problems are explained where they happen, calmly, with ink glyphs.

**Anatomy.**
- **T1 banner** (reduced mode, 4–20 MB): the inline notice (padding 6/10, radius 4, 1 px `seam-strong`, panel/ground fill, info glyph `text-2`) between the toolbar and the code: "This document is large (18.4 MB), so the editor is in reduced mode." and, in `text-2`, "Reduced mode keeps typing fast. It ends when the document is smaller than 3.2 MB." The status bar shows "Reduced mode".
- **Paste refused** (R-M3-49): a wide toast with the ink circle-alert glyph: "Couldn't paste" / "The pasted text is 31.4 MB, which would take this document past 20 MB. Paste a smaller part."
- **NUL pasted**: each NUL is inserted visibly as a `NUL` control chip (the grid's chip: Fragment Mono 11 `text-2` on the inlay well, radius 3; it takes its own width in the line); a wide toast with the ink triangle-alert glyph says how many and that most servers reject them.
- **Clipboard unavailable**: a toast with the clipboard-x glyph and Try again.
- *M4b sketch:* the T2 read-only preview banner ("Read-only preview: this file is 148 MB." / "The editor shows its first 2.5 MB. Run as script sends the whole file.") with a disabled Run as script button; and the 20–100 MB paste confirmation (420 alertdialog, Cancel primary and focused, "Paste as preview" outline).

**States.** T1 banner · Paste refused · NUL pasted · Clipboard unavailable · M4b T2 preview · M4b 20–100 MB paste.

**Keyboard.** Toasts never take the focus unless asked (F6); Esc dismisses. The banner is not focusable; its text is in the editor's accessible description.

### 8 · Editor context menu
**Intent.** The editor's context menu on the PopupMenu anatomy: clipboard first, then Run.

**Anatomy.** 250 px PopupMenu: Cut ⌘X · Copy ⌘C · Paste ⌘V · Select all ⌘A · separator · Run ⌘↵ · Describe execution range · separator · *M3b sketch:* Find… ⌘F (disabled). Key hints mono 11 `text-2` (raised). Cut and Copy stay enabled without a selection: they take the line (`copy_line_without_selection`).

**Describe execution range** puts one sentence in the status bar and announces it: "Run executes lines 12–14: SELECT o.id, c.full_name, …" (one statement) or "Run executes 3 statements on lines 2–4".

**States.** With selection · No selection · Describe range.

**Keyboard.** Shift+F10 or the context-menu key opens it at the caret; ↑/↓, ↵, Esc. The editor's Edit-menu commands are the same entries.

### 9 · Results tile
**Intent.** The results tile belongs to the focused pane's active console (R-M3-41): Output first, then one tab per result, a minimal toolbar, the grid and the cursor strip.

**Anatomy.**
- A console that never ran keeps the **M1 empty state**, unchanged ("Run a query to see results here", no header).
- **Tab strip** (30 px): Output (scroll-text glyph) first, then "Result 1", "Result 2" (table glyph). Production results carry the 7 px swatch; staging ones the hollow swatch and "(staging)" in `text-2`. A 6 px `text-2` dot on Output means a line arrived while another tab was active.
- **Toolbar** (34 px raised, Task 30): Copy (ghost, ⌘C) and, on the right, rows · timing in mono `text-2`. On the Output tab: Clear, Copy, Scroll to end.
- **More than 2,000 columns**: an info row under the toolbar (raised, padding 5/12, 12/16 `text-2`, info glyph; prose, so it wraps): "2,412 columns. Wide results scroll sideways; select fewer columns to read them more easily." It is not in the toolbar, so the Overflow Rule never has to hide it.
- **First rows arriving**: the header from the row description, static `seam-soft` skeleton bars (no shimmer), "Fetching… 0 rows" in the strip.
- **Zero rows**: the header, "No rows" (12.5 `text-3`) in the body, "0 rows" with the green check in the strip.
- **Update count only**: no result tab; the Output tab is active with "12 rows affected in 38 ms".
- **An error**: the Output tab is active with the error line (surface 12).

**States.** Empty (M1) · First rows · Rows · 0 rows · Update count · Unread dot · Error · Result 1, 2 · Staging tab · 2,412 columns.

**Keyboard.** ⌘3 focuses the tile; Ctrl+Tab / Ctrl+Shift+Tab move between result tabs; the grid takes the focus inside a result tab.

### 10 · Grid cells and header
**Intent.** A dense, database-agnostic grid: names in body-strong with mono types, numbers by the format locale, IDs raw, every odd value visible.

**Anatomy.**
- **Header** 36 px on `head`: the name in 13/600 `text`, the type in Fragment Mono 11 `text-3` under it; numeric columns right-aligned. 1 px `seam-strong` under the header.
- **Rows** 21 px with `seam-soft` lines; the row-number column on `gutter`, 11.5 `text-3`, right-aligned, **raw** (no grouping), 46 px up to 5 digits and wider by one digit advance beyond (8-digit scenario).
- **Cells**: body 13 `text`. `NULL` is the constant in synthesized small caps, 600, `text-3`; the text `'NULL'` is ordinary text; `''` is mono 11.5 `text-3`. Numbers right-aligned in the format locale (`1,240.00` / `1.240,00`); ID-like columns (`id`, `*_id`) raw. Timestamps ISO in mono 11.5 `text-2` (open question 2). UUIDs, bytes (`\x89504e47…`, a 16-byte value as a UUID) and JSON in mono 11.5 `text`. Booleans `true`/`false`, centred, never translated.
- **Controls** are chips (mono 11 `text-2` on the inlay well, radius 3): `↵` after the first line of a multi-line value, `→` for a tab, `⟨RLO⟩` for a bidi override. Copy keeps them verbatim.
- **Truncated** values end with a mono 11 `text-2` chip "… 5.2 MB" (1 px `seam-strong`, radius 3); the tooltip says how much is shown.
- **Decode error**: the ink triangle-alert glyph and "Can't decode" in `text-2`; the tooltip names the type.
- **Long Turkish text** ellipsizes in the cell; the whole value is in the tooltip (and the M3b value viewer).
- **Selection**: range `blue-tint`, active cell `blue-tint-strong`, a 1 px `blue` box with a 5 px square handle, selected row numbers in `blue`. Row set (from the row numbers), column set (from the header; the header cell tints too), select all (the corner). Hover: a 1 px `seam-strong` inset outline.
- **Scroll track** (9 px, `gutter`): the thumb (`text-3` at 60 %, radius 3) and the rows already loaded beyond the viewport as a square `green-fill` band.
- *M3b sketch:* the sort indicator (blue arrow and the level, mono 11) and a frozen-left column (`seam-strong` right edge).

**States.** Cell types · Bytes, JSON · Range · Row set · Column set · Select all · Hover · Numbers and IDs (toggle L for TR digits) · 8-digit rows · M3b sketch.

**Keyboard.** Arrows move the active cell; Shift extends; ⌘A selects all; ⌘C copies. Select column has no chord in M3a (`grid.select_column` in the palette, P25).

### 11 · Cursor strip
**Intent.** The 26 px strip says how complete a result is and what holding it costs, and offers the next step.

**Anatomy** (26 px, raised, caption 12 `text-2`, compact 20 px buttons).
- **Streaming**: the amber run marker and "Fetching… 3,500 rows" (count in mono `text`).
- **Suspended**: "10,000+ rows" in `text`, a seam, "Cursor open — holding a database snapshot", then Close cursor · Fetch more · Fetch all. When the strip is short the sentence becomes "Cursor open" (tooltip keeps the whole); the buttons never truncate.
- **Complete**: the green check and "1,000,000 rows".
- **Closed**: the info glyph (`text-2`), the reason sentence in `text` (`cursor-closed-*`, with "Hacivat's transaction was rolled back." when that happened, R-M3-23), and Re-run.
- **Cancelled**: the ban glyph and "Cancelled after 3,500 rows", Re-run.
- **Memory limit** (R-M3-33): the ink circle-alert glyph, "Memory limit for this result reached (412,880 rows, 256 MB)" in `text`, Fetch more anyway · Close cursor. That banner is the confirmation; no dialog.
- "2 more result sets not shown" with the layers glyph after a seam (R-M3-25).
- Right: "Selection" and the dimensions in mono ("3 × 2").
- **When the strip is short** (960 px, Turkish): first the word "Selection" hides, then the cursor sentence becomes "Cursor open"; a reason sentence that still does not fit wraps and the strip grows to two lines (padding 4, line height 16). Buttons and counts never truncate.

**States.** Streaming · Suspended · Complete · Closed (session needed, idle, transaction ended, connection lost) · Cancelled · Memory limit · More result sets.

**Keyboard.** The strip's buttons are in the results tile's Tab order after the grid; palette commands `results.close_cursor`, `results.fetch_more`, `results.fetch_all`, `results.rerun`.

### 12 · Output log
**Intent.** The plain record of what ran: one sentence per event, ink glyphs, the server's text verbatim and separate.

**Anatomy.**
- 21 px rows, 12 px left padding, 12.5 `text`. Glyphs 12 px in `text-2`: check (rows, counts, completed), git-commit (transaction), ban (cancelled), lock (read-only denial), ellipsis (events not shown), list (summary), info (notes). Errors use triangle-alert in `text`.
- **Error**: "Statement 3 failed" at 600, the SQLSTATE chip, then the server's text verbatim on its own 21 px row (mono 11.5 `text-2`, indented 32 px, `lang` set to the server's language), ellipsized with the whole in its tooltip. A click on either row selects the statement in the editor.
- **Summary**: "2 of 6 statements executed, 1 failed, in 1 s 20 ms" at 600.
- Lines for rows retrieved (with "more are available" while a cursor is open), rows affected, completed, transaction committed / rolled back, cancelled, more result sets, read-only denials (one sentence per `DenyLayer`), COPY not supported, and "312 events not shown" in `text-2`.
- Timestamps are off by default (setting); copy is invariant (ISO, ASCII digits).
- **Empty**: "No output yet. Messages from the statements you run appear here." centred in `text-3`.

**States.** Run log · Error · Commit, roll back · Read-only denial · Events not shown · Empty.

**Keyboard.** The list takes the focus; ↑/↓ move between lines; Enter on an error line selects its statement; ⌘C copies the selected lines.

### 13 · Close with an open transaction
**Intent.** Closing a console or quitting never discards a user transaction silently (D-014). The safe answer is the default.

**Anatomy.**
- 420 px **alertdialog** over the scrim (below the title bar). Title (13/600): "orders-dev has an open transaction with 3 statements". Subtitle: the env mark, the console's title in mono 11.5 (the connection name, "orders-dev (2)" for a second console) and the engine.
- Body: "Closing this console ends its session. Commit keeps the changes; Roll back discards them." (quit: "Quitting ends the session. …").
- Footer: **Commit and close** (outline, check icon), **Roll back and close** (outline, undo icon) on the left; **Cancel** (primary, focused) on the right. Quit: "Commit and quit", "Roll back and quit".
- **Failed** transaction (R-M3-16 amended, P16): title "The transaction on orders-dev failed"; body "An error or a cancel stopped it, so it can only be rolled back. Its 3 statements won't be kept."; footer **Roll back and close** … **Cancel** (primary, focused). No Commit.

**States.** Console close · Quit · Failed · Failed, quit.

**Keyboard.** Initial focus on Cancel. Enter (key-down, no repeat, no modifier) and Esc both cancel. Tab is trapped in the card. Closing the window answers Cancel.

### 14 · Copy feedback
**Intent.** Copy confirms what it took, and refuses calmly when the selection is too large.

**Anatomy.**
- **Copied**: the 320 px toast with the copy-check glyph: "Copied 3 × 8 cells" and, in `text-2`, "Tab-separated, raw values".
- **Copying** (over 10,000 cells, background): "Copying 48,000 cells…".
- **Too much** (over 1,000,000 cells or 100 MB): a wide toast with the ink circle-alert glyph: "Too much to copy" / "The selection has 8,000,000 cells. Copy takes at most 1,000,000 cells or 100 MB; select fewer rows or columns."

**States.** Copied · Copying · Too much.

**Keyboard.** ⌘C in the grid; the toast never takes the focus.

---

## NEW primitives

All use existing tokens only and are reproducible in GPUI.

| Primitive | Anatomy and exact tokens |
|---|---|
| **Preview band / exact tint** | Full row: `blue-tint` at .55 (dark) / .8 (light) across the code area. Exact: the same fill on the statement's characters only (whole advances, no terminator). Neutral variants (uncertain, no connection): `hover`. |
| **Gutter bar** | 1 px at x = 4: `green` executed, `blue` preview, `text-3` stale or offline, dashed `text-2` uncertain (GPUI dashed border). Units of a multi-statement selection leave a 2 px gap. |
| **Gutter glyph slot** | 12 × 18 px at x = 8. Marks: 7 px amber run marker, 7 px hollow amber square, 7 px hollow `seam-strong` square; icons 12 px. A 3 px `text-2` dot for "more on this row". |
| **End-of-line status** | Right-aligned at 14 px from the code area's edge; micro 11.5 `text-3`, numbers mono 11; green check for success, amber words for running and cancelling, `text` words and a SQLSTATE chip for failure. Collision: a panel pad with a 1 px `seam-soft` left edge and the short form. |
| **Executable-comment tint** | `seam-soft` behind the comment's tokens; delimiters in comment ink, not italic. |
| **Control chip** | Fragment Mono 11 `text-2` on `seam-soft`, radius 3, padding 0/2: `↵`, `→`, `⟨RLO⟩` in the grid; `NUL` in the editor, where the chip takes its own width (a display fragment, not an advance). |
| **Chooser** | PopupMenu, 460 px, a header line, kind + SQL label + count rows, `blue-tint-strong` active row, the remembered kind's blue check, a 24 px key-hint footer on `head`. |
| **Gutter tooltip** | Popover anatomy, 320 px, padding 8/10: title with chip, a captioned server-message box (panel/ground, 1 px `seam-strong`, radius 4, mono 11.5/17), a hint line. |
| **Strip buttons** | The Button at 20 px height, padding 0/7, 12 px label. |
| **Truncation chip** | "… 5.2 MB" in Fragment Mono 11 `text-2`, 1 px `seam-strong`, radius 3, padding 0/4. |
| **Skeleton bar** | 7 px tall, square, `seam-soft` (dark) / `seam` (light), static. |
| **Loaded band** | In the 9 px scroll track: square `green-fill` from the thumb to the last loaded row. |

**Icons.** The mock adds five Lucide 0.460 icons to the sprite: `scroll-text` (Output tab), `skip-forward` (Skipped), `arrow-down-to-line` (Scroll to end), `eraser` (Clear), `clipboard-x` (clipboard unavailable). The app's icon set (`crates/ui/hacivat-ui-core/assets/icons/lucide/`) also lacks `ban` and `copy-check`, which the M2b sprite already had. Tasks 28, 30 and 27 add those seven SVGs with the set's licence notice.

---

## Open questions for the PO

Each was answered by the ruling noted, so the build is not blocked; the PO can overrule any.

1. **Two statements on one line: full-row band or exact-range tint?**
   - *Ruling:* the exact-range tint on the statement's characters, without the terminator, whenever a unit shares its line; the full-row band on lines a unit owns. A full row would claim the other statement runs too, which is exactly the wrong-range anti-goal (§2.3). The rejected variant is in the mock (Preview › Two · full row) for comparison.
   - *Cost if wrong:* a paint rule in Task 28 (two quad shapes instead of one).
2. **Timestamp cell style: mono 11.5 `text-2` or body?**
   - *Ruling:* mono 11.5 `text-2`, as approved in harman (`td.ts`). Timestamps are ISO literals (Mono Means Copyable), and the dimmer ink keeps a column of near-identical prefixes from dominating the row. UUIDs, bytes and JSON are mono in `text`.
   - *Cost if wrong:* one style per cell kind in Task 16's `CellKind` mapping.
3. **Caret blink.**
   - *Ruling:* blink at the OS rate (macOS: the system insertion-point period), go solid after 10 s without input, and never blink under reduced motion. A caret blink is a platform affordance for finding the caret, not ambient motion, and stopping it after 10 s keeps an idle window from repainting. The mock bar's Blink / Static toggle shows both.
   - *Cost if wrong:* one setting default.
4. **The gutter growth rule.**
   - *Ruling:* confirmed: 58 px + (digits − 3) × one Fragment Mono advance, never narrower than 58 px. The width follows the document's last line number, not the visible ones, so scrolling never moves the text. The grid's row-number column follows the same idea: 46 px up to 5 digits, one tabular-digit advance more per digit, raw numbers.
   - *Cost if wrong:* none; it is a constant in Task 28.
5. **Failed = ink glyph, Cancelling = amber, never red.**
   - *Ruling:* confirmed. Failed is the ink triangle-alert glyph with the SQLSTATE chip; Cancelling is a hollow amber square; Cancelled is the ban glyph in `text-2`. Red stays for production.
   - *Additional ruling:* **Running uses the 7 px amber run marker, not a spinner.** The plan says "spinner"; a spinner is ambient motion, and the run marker is already the app's running glyph (capsule, splash, toolbar, strip). The clock beside it ("running · 3 s", once a second) carries the progress. Nothing is drawn under 50 ms.
   - *Cost if wrong:* swapping one glyph in Task 28.
6. **The production read-only notice copy (no milestone names).**
   - *Ruling:* the toolbar says "Production is read-only in this version" ("Üretim bu sürümde salt okunur"), with a red lock, as a button. Its popover: "Why this console is read-only" / "Writing to production needs statement confirmations, and this version of Hacivat doesn't have them yet. Until it does, every production console runs read-only, whatever the connection allows." / "Reads run as usual. A statement that would change data or schema isn't sent, and the Output tab says why." It promises nothing and names no milestone; M3b deletes the strings with the flag.
   - *Cost if wrong:* copy only.
7. **The chooser's "boundary uncertain" copy.**
   - *Ruling:* title "Boundary uncertain"; "Hacivat can't tell exactly where this statement ends, so it sent nothing."; the reason (an executable comment, or an unclosed quote or comment above); "Select exactly the text to run, then press ⌘↵." There is no run item, so there is nothing to press by mistake (R-M3-24).
   - *Cost if wrong:* copy only.
8. **The copy-refusal copy.**
   - *Ruling:* "Too much to copy" / "The selection has 8,000,000 cells. Copy takes at most 1,000,000 cells or 100 MB; select fewer rows or columns." It states both limits and the way out, and does not promise export ("export arrives later" in the plan's sketch was dropped: a promise of a later feature is not a fix the user can apply now).
   - *Cost if wrong:* copy only.
9. **The close-with-open-transaction copy, the button order and the Failed forms.**
   - *Ruling:* title "orders-dev has an open transaction with 3 statements"; buttons that name the result: "Commit and close", "Roll back and close" (outline, left), "Cancel" (primary, focused, right); quit uses "… and quit". Neither Commit nor Roll back is Danger red: neither destroys stored data, and the protection is that neither is the default. The Failed prompt drops Commit and says why. The status segment's Failed form is "Transaction failed · roll back to end it" with the ink alert glyph.
   - *Cost if wrong:* copy and button order in Task 33.
10. **Unfocused selection colour.** *Ruling:* neutral `seam-strong` (macOS's inactive selection), not a fainter blue, so it never looks like the Run preview. *Cost if wrong:* one token.
11. **Uncertain and offline previews.** *Ruling:* drawn in neutral `hover` with a dashed or `text-3` bar, never blue, because blue means "this runs". *Cost if wrong:* a paint variant in Task 28.
12. **Executable comments' "own tint".** *Ruling:* the inlay well (`seam-soft`) behind the comment's code-coloured tokens; no new colour. *Cost if wrong:* one decoration class in Task 24.
13. **End of line on a long line.** *Ruling:* the status keeps its place on a panel pad over the line's tail and shows its short form; the tooltip has the rest. *Cost if wrong:* Task 28 layout.
14. **Collapse orders.** *Ruling:* toolbar: the database crumb, then the notice's text (into ⋯), then the waiting sentence's short form. Status bar: version, encryption, selected count, path, Ln/Col, rows · timing; the environment, transaction, cursor, message, Reduced mode and language never collapse. Strip: the "Selection" word, then the cursor sentence becomes "Cursor open", then a reason sentence wraps to a second line. The 2,000-column hint is a wrapping info row, not a toolbar item. *Cost if wrong:* priority numbers in Tasks 21, 30, 34.
15. **The open-transaction marker.** *Ruling:* the 7 px amber marker: an open transaction is pending work (Hue Is Meaning: amber = pending). *Cost if wrong:* one glyph.
16. **Turkish "imleç" for both the text caret and the database cursor.** "From caret" is "İmleçten sona" in the chooser and "Cursor open" is "İmleç açık" in the results; the context separates them, and "kürsör" is not idiomatic. *Ruling:* keep "imleç" in both until the D-030 glossary decides. *Cost if wrong:* a handful of strings.
17. **Error titles in the log.** *Ruling:* "Statement 3 failed" with the SQLSTATE chip, not a curated per-code title: M3a has no curated error catalog, and the server's own sentence follows verbatim on the next row. *Cost if wrong:* M3b may add `db-pg-sqlstate-*` titles on top.
18. **The chooser's active row.** *Ruling:* `blue-tint-strong`, as the completion list's active item: Enter runs that row, so it reads as a selection, not a hover. *Cost if wrong:* one token on one menu.
19. **"Roll back" in English, "Geri al" in Turkish; "Commit" stays "Commit et".** The shipped M2 strings already use "commit edilir" and "geri al". *Ruling:* follow them. *Cost if wrong:* copy only.

---

## Verification

- Headless Chrome, driven over the DevTools protocol, rendered every scenario of the fourteen surfaces (108 scenarios) × EN/TR × dark/light × 1280 × 800 and 960 × 600: 864 renders, with the mock bar hidden (`bar=0`) so the window is exactly that size, and clocks frozen (`shot=1`).
- Each render was checked for: console errors and exceptions; a toolbar, strip, status bar, tab strip, capsule, dialog, popover, menu, tooltip, toast or notice that overflows sideways; ellipsized chrome text inside them (data is exempt: SQL labels, cell text, verbatim server text, the connection crumb); a floating layer outside the window; more than one cp-fill block larger than a mark; text below 11 px (the approved synthesized small caps excepted); `text-3` ink on the raised tone (dark theme, where raised differs from panel).
- Result: **864 renders, 0 findings.** The first pass found 56 (the cursor sentence and a closed-reason sentence ellipsized in the strip at 960, the production toolbar overflowing at 960 with the waiting state, the 2,000-column hint overflowing the toolbar, chips at 10.5 px); each was fixed in the design (short forms, a wrapping strip, the hint as its own row, 11 px chips) and is recorded above.
- The PNGs were looked at for four slices: EN · light · 1280, TR · dark · 960, EN · dark · 1280 and TR · light · 960. That review caught what the checker could not, and each was fixed: syntax classes leaking into the status bar (now scoped to the code area), the placeholder borrowing the panel-header class, an EN plural ("1 rows"), a Running scenario that kept the previous cursor open (contrary to R-M3-23), an unreadable `␀` control picture (now the `NUL` chip), a read-only scenario whose capsule said "No connection", an invalid timestamp hour, and code ligatures (`>=` drawn as one glyph; ligatures are off, as in the product).
- Counting the lines of this file that carry the Status value of the copy table gives 162, the number of Turkish strings: the value appears once per row and nowhere else.
- The Impeccable detector reports three advisory items, all inherited or parser noise: the traffic lights' translucent border (M2b CSS), and Source Sans 3 and Fragment Mono read as undeclared families.

---

## Copy strings (EN / TR)

The id is the Fluent id. Placeholders are Fluent variables (`{ $rows }`); counts, durations and sizes take pre-formatted values from the format locale (`exec-dur-*`, `exec-size-*`, `hui-editor-size-mb`). Every EN message with a count has a plural selector; where the singular changes more than the noun's ending, its `one` form follows in the EN cell. Turkish needs no plural after a numeral.

| Prefix | File | Owner |
|---|---|---|
| `hv-console-*`, `hv-editor-*`, `hv-chooser-*`, `hv-results-*`, `hv-grid-*`, `hv-strip-*`, `hv-tx-*`, `hv-copy-*` | `locales/{en,tr}/hacivat-ui.ftl` | Tasks 30, 32, 33, 34 (each in its own `## m3a-tNN-*` block) |
| `hui-editor-*` | `locales/{en,tr}/hui.ftl` | Tasks 25, 27, 29 |
| `exec-*`, `cursor-*` | `locales/{en,tr}/hacivat-core.ftl` | Task 23 |

Reused, not repeated here: the shipped `hv-tab-console-untitled`, `hv-editor-empty`, `hv-results-empty`, `hv-capsule-access-read-only`, `hv-conn-access-ro-guarded` and `-enforced`, `hui-menu-more`, `hui-tab-close`, `hui-toast-dismiss`, `hui-palette-hint-navigate`, `-run`, `-close`, and the M2a/M2b capsule, explorer and sheet strings. Literal data is not translated: SQL, connection names, SQLSTATE codes, server text, `NULL`, `true`/`false`.

None of the Turkish below has been signed off yet; the Status column says so for each row.

| id | EN | TR | Status |
|---|---|---|---|
| `exec-dur-lt-ms` | <1 ms | <1 ms | PO review |
| `exec-dur-ms` | { $ms } ms | { $ms } ms | PO review |
| `exec-dur-s` | { $s } s | { $s } sn | PO review |
| `exec-dur-s-ms` | { $s } s { $ms } ms | { $s } sn { $ms } ms | PO review |
| `exec-dur-min` | { $min } min | { $min } dk | PO review |
| `exec-dur-min-s` | { $min } min { $s } s | { $min } dk { $s } sn | PO review |
| `exec-size-kb` | { $n } KB | { $n } KB | PO review |
| `exec-size-mb` | { $n } MB | { $n } MB | PO review |
| `exec-size-gb` | { $n } GB | { $n } GB | PO review |
| `hv-console-tab-title` | { $name } ({ $n }) | { $name } ({ $n }) | PO review |
| `hv-console-run` | Run | Çalıştır | PO review |
| `hv-console-stop` | Stop | Durdur | PO review |
| `hv-console-running` | Running | Çalışıyor | PO review |
| `hv-console-cancelling` | Cancelling… | İptal ediliyor… | PO review |
| `hv-console-waiting` | Waiting for a free connection to { $name } ({ $inUse } of { $max } in use) | { $name } için boş bağlantı bekleniyor (kullanımda: { $inUse }/{ $max }) | PO review |
| `hv-console-waiting-short` | Waiting for a connection ({ $inUse } of { $max } in use) | Bağlantı bekleniyor (kullanımda: { $inUse }/{ $max }) | PO review |
| `hv-console-waiting-cancel` | Cancel | Vazgeç | PO review |
| `hv-console-prod-notice` | Production is read-only in this version | Üretim bu sürümde salt okunur | PO review |
| `hv-console-prod-pop-title` | Why this console is read-only | Bu konsol neden salt okunur | PO review |
| `hv-console-prod-pop-body` | Writing to production needs statement confirmations, and this version of Hacivat doesn't have them yet. Until it does, every production console runs read-only, whatever the connection allows. | Üretimde yazmak ifade onayları gerektirir ve Hacivat'ın bu sürümünde bu onaylar henüz yok. O zamana kadar her üretim konsolu, bağlantı neye izin verirse versin salt okunur çalışır. | PO review |
| `hv-console-prod-pop-reads` | Reads run as usual. A statement that would change data or schema isn't sent, and the Output tab says why. | Okuma ifadeleri her zamanki gibi çalışır. Veriyi ya da şemayı değiştirecek bir ifade gönderilmez; nedenini Çıktı sekmesi söyler. | PO review |
| `hv-console-connect-first` | Connecting to { $name }. Run again once it's connected. | { $name } bağlantısı kuruluyor. Bağlandıktan sonra yeniden çalıştırın. | PO review |
| `hv-editor-placeholder` | Write SQL here. { $key } runs the statement at the caret. | SQL'i buraya yazın. { $key }, imlecin bulunduğu ifadeyi çalıştırır. | PO review |
| `hv-editor-lncol` | Ln { $line }, Col { $col } | Sat { $line }, Süt { $col } | PO review |
| `hv-editor-selected` | { $n } selected | { $n } seçili | PO review |
| `hv-editor-reduced` | Reduced mode | Kısıtlı mod | PO review |
| `hv-editor-nothing-to-run` | Nothing to run here | Burada çalıştırılacak bir şey yok | PO review |
| `hv-editor-uncertain` | Boundary uncertain: select the statement to run it | Sınır belirsiz: çalıştırmak için ifadeyi seçin | PO review |
| `hv-editor-preview-choices` | { $n } choices | { $n } seçenek | PO review |
| `hv-editor-preview-unit` | { $i } of { $n } | { $i }/{ $n } | PO review |
| `hv-editor-preview-offline` | not connected | bağlı değil | PO review |
| `hv-editor-preview-uncertain` | boundary uncertain | sınır belirsiz | PO review |
| `hv-editor-mark-queued` | queued | sırada | PO review |
| `hv-editor-mark-running` | running · { $duration } | çalışıyor · { $duration } | PO review |
| `hv-editor-mark-rows` | executed · { $duration } · { $rows } rows (one: executed · { $duration } · { $rows } row) | çalıştı · { $duration } · { $rows } satır | PO review |
| `hv-editor-mark-affected` | { $rows } rows affected · { $duration } (one: { $rows } row affected · { $duration }) | { $rows } satır etkilendi · { $duration } | PO review |
| `hv-editor-mark-done` | executed · { $duration } | çalıştı · { $duration } | PO review |
| `hv-editor-mark-failed` | failed | başarısız | PO review |
| `hv-editor-mark-cancelling` | cancelling… | iptal ediliyor… | PO review |
| `hv-editor-mark-cancelled` | cancelled after { $duration } | { $duration } sonra iptal edildi | PO review |
| `hv-editor-mark-skipped` | skipped: an earlier statement failed | atlandı: önceki bir ifade başarısız oldu | PO review |
| `hv-editor-mark-stale` | edited since execution | çalıştırıldıktan sonra düzenlendi | PO review |
| `hv-editor-mark-denied` | not sent: read-only | gönderilmedi: salt okunur | PO review |
| `hv-editor-tip-show` | Click to show it in Output | Çıktı sekmesinde görmek için tıklayın | PO review |
| `hv-editor-tip-stale` | Edited since execution. The marks show the last run. | Çalıştırıldıktan sonra düzenlendi. İşaretler son çalıştırmayı gösterir. | PO review |
| `hv-editor-tip-two` | Two statements on this line | Bu satırda iki ifade var | PO review |
| `hv-editor-describe-range` | Describe execution range | Çalıştırma aralığını açıkla | PO review |
| `hv-editor-describe-one` | Run executes lines { $from }–{ $to }: { $label } | Çalıştır, { $from }–{ $to }. satırları çalıştırır: { $label } | PO review |
| `hv-editor-describe-many` | Run executes { $n } statements on lines { $from }–{ $to } | Çalıştır, { $from }–{ $to }. satırlardaki { $n } ifadeyi çalıştırır | PO review |
| `hui-editor-cut` | Cut | Kes | PO review |
| `hui-editor-copy` | Copy | Kopyala | PO review |
| `hui-editor-paste` | Paste | Yapıştır | PO review |
| `hui-editor-select-all` | Select all | Tümünü seç | PO review |
| `hui-editor-find` | Find… | Bul… | PO review |
| `hui-editor-size-mb` | { $n } MB | { $n } MB | PO review |
| `hui-editor-t1-title` | This document is large ({ $size }), so the editor is in reduced mode. | Bu belge büyük ({ $size }); düzenleyici kısıtlı modda. | PO review |
| `hui-editor-t1-body` | Reduced mode keeps typing fast. It ends when the document is smaller than { $size }. | Kısıtlı mod yazmayı hızlı tutar. Belge { $size } boyutunun altına inince sona erer. | PO review |
| `hui-editor-paste-refused-title` | Couldn't paste | Yapıştırılamadı | PO review |
| `hui-editor-paste-refused-body` | The pasted text is { $size }, which would take this document past { $limit }. Paste a smaller part. | Yapıştırılan metin { $size }; belge { $limit } sınırını aşardı. Daha küçük bir bölüm yapıştırın. | PO review |
| `hui-editor-nul-title` | The pasted text contains NUL characters | Yapıştırılan metinde NUL karakteri var | PO review |
| `hui-editor-nul-body` | { $n } NUL characters are marked in the text so you can find them. Most servers reject them. (one: { $n } NUL character is marked in the text so you can find it. Most servers reject it.) | { $n } NUL karakteri, bulabilmeniz için metinde işaretlendi. Çoğu sunucu bunları reddeder. | PO review |
| `hui-editor-clipboard-title` | Couldn't read the clipboard | Pano okunamadı | PO review |
| `hui-editor-clipboard-body` | Another app may be holding it, or the system didn't allow access. | Başka bir uygulama panoyu tutuyor olabilir ya da sistem erişime izin vermedi. | PO review |
| `hui-editor-try-again` | Try again | Yeniden dene | PO review |
| `hui-editor-t2-title` | Read-only preview: this file is { $size }. | Salt okunur önizleme: bu dosya { $size }. | PO review |
| `hui-editor-t2-body` | The editor shows its first { $shown }. Run as script sends the whole file. | Düzenleyici dosyanın ilk { $shown } kısmını gösterir. Betik olarak çalıştır, dosyanın tamamını gönderir. | PO review |
| `hui-editor-t2-run` | Run as script | Betik olarak çalıştır | PO review |
| `hui-editor-paste-big-title` | Paste { $size }? | { $size } yapıştırılsın mı? | PO review |
| `hui-editor-paste-big-body` | The document becomes a read-only preview. You can still run it as a script. | Belge salt okunur bir önizlemeye dönüşür. Yine de betik olarak çalıştırabilirsiniz. | PO review |
| `hui-editor-paste-big-ok` | Paste as preview | Önizleme olarak yapıştır | PO review |
| `hv-chooser-title` | Choose what to run | Neyin çalışacağını seçin | PO review |
| `hv-chooser-statement` | Statement | İfade | PO review |
| `hv-chooser-largest` | Largest statement | En geniş ifade | PO review |
| `hv-chooser-from-caret` | From caret | İmleçten sona | PO review |
| `hv-chooser-whole` | Whole script | Tüm betik | PO review |
| `hv-chooser-count` | { $n } statements | { $n } ifade | PO review |
| `hv-chooser-uncertain-title` | Boundary uncertain | Sınır belirsiz | PO review |
| `hv-chooser-uncertain-body` | Hacivat can't tell exactly where this statement ends, so it sent nothing. | Hacivat bu ifadenin tam olarak nerede bittiğini anlayamıyor; bu yüzden hiçbir şey göndermedi. | PO review |
| `hv-chooser-uncertain-exec-comment` | A ; sits inside an executable comment (/\*! … \*/). | Yürütülebilir bir yorumun (/\*! … \*/) içinde ; var. | PO review |
| `hv-chooser-uncertain-unclosed` | A quote or comment above isn't closed. | Yukarıdaki bir tırnak ya da yorum kapanmamış. | PO review |
| `hv-chooser-uncertain-fix` | Select exactly the text to run, then press { $key }. | Çalıştırılacak metni tam olarak seçin, sonra { $key } tuşuna basın. | PO review |
| `hv-results-output` | Output | Çıktı | PO review |
| `hv-results-result-n` | Result { $n } | Sonuç { $n } | PO review |
| `hv-results-env-suffix` | ({ $env }) | ({ $env }) | PO review |
| `hv-results-unread` | New output | Yeni çıktı | PO review |
| `hv-results-copy` | Copy | Kopyala | PO review |
| `hv-results-rows-timing` | { $rows } rows · { $duration } (one: { $rows } row · { $duration }) | { $rows } satır · { $duration } | PO review |
| `hv-results-rows-more` | { $rows }+ rows | { $rows }+ satır | PO review |
| `hv-results-log-clear` | Clear | Temizle | PO review |
| `hv-results-log-end` | Scroll to end | Sona kaydır | PO review |
| `hv-results-log-empty` | No output yet. Messages from the statements you run appear here. | Henüz çıktı yok. Çalıştırdığınız ifadelerin iletileri burada görünür. | PO review |
| `hv-results-status-cursor` | Cursor open | İmleç açık | PO review |
| `hv-results-status-cursor-tip` | Holding a database snapshot. Click to close the cursor. | Veritabanı anlık görüntüsü tutuluyor. İmleci kapatmak için tıklayın. | PO review |
| `hv-grid-no-rows` | No rows | Satır yok | PO review |
| `hv-grid-too-many-columns` | { $n } columns. Wide results scroll sideways; select fewer columns to read them more easily. | { $n } sütun. Geniş sonuçlar yana kayar; daha rahat okumak için daha az sütun seçin. | PO review |
| `hv-grid-truncated` | … { $size } | … { $size } | PO review |
| `hv-grid-truncated-tip` | Showing the first { $shown } of { $size } | Değerin ilk { $shown } kısmı gösteriliyor (toplam { $size }) | PO review |
| `hv-grid-decode-error` | Can't decode | Çözümlenemedi | PO review |
| `hv-grid-decode-error-tip` | Hacivat couldn't decode this { $type } value. | Hacivat bu { $type } değerini çözümleyemedi. | PO review |
| `hv-grid-loading` | Loading | Yükleniyor | PO review |
| `hv-strip-fetching` | Fetching… { $rows } rows (one: Fetching… { $rows } row) | Getiriliyor… { $rows } satır | PO review |
| `hv-strip-cursor-open` | Cursor open — holding a database snapshot | İmleç açık — veritabanı anlık görüntüsü tutuluyor | PO review |
| `hv-strip-close-cursor` | Close cursor | İmleci kapat | PO review |
| `hv-strip-fetch-more` | Fetch more | Daha fazla getir | PO review |
| `hv-strip-fetch-all` | Fetch all | Tümünü getir | PO review |
| `hv-strip-complete` | { $rows } rows (one: { $rows } row) | { $rows } satır | PO review |
| `hv-strip-rerun` | Re-run | Yeniden çalıştır | PO review |
| `hv-strip-cancelled` | Cancelled after { $rows } rows (one: Cancelled after { $rows } row) | { $rows } satırdan sonra iptal edildi | PO review |
| `hv-strip-memory` | Memory limit for this result reached ({ $rows } rows, { $size }) | Bu sonucun bellek sınırına ulaşıldı ({ $rows } satır, { $size }) | PO review |
| `hv-strip-fetch-anyway` | Fetch more anyway | Yine de getir | PO review |
| `hv-strip-more-sets` | { $n } more result sets not shown (one: { $n } more result set not shown) | { $n } sonuç kümesi daha gösterilmedi | PO review |
| `hv-strip-selection` | Selection | Seçim | PO review |
| `hv-strip-dims` | { $rows } × { $cols } | { $rows } × { $cols } | PO review |
| `cursor-closed-session-needed` | Cursor closed after { $rows } rows: another statement needed the session. | { $rows } satırdan sonra imleç kapandı: oturum başka bir ifade için gerekiyordu. | PO review |
| `cursor-closed-idle-timeout` | Cursor closed after { $rows } rows: idle for { $duration }. | { $rows } satırdan sonra imleç kapandı: { $duration } boyunca boşta kaldı. | PO review |
| `cursor-closed-tab-closed` | Cursor closed after { $rows } rows: its tab was closed. | { $rows } satırdan sonra imleç kapandı: sekmesi kapatıldı. | PO review |
| `cursor-closed-tx-ended` | Cursor closed after { $rows } rows: its transaction ended. | { $rows } satırdan sonra imleç kapandı: ait olduğu işlem sona erdi. | PO review |
| `cursor-closed-user-closed` | Cursor closed after { $rows } rows. | { $rows } satırdan sonra imleç kapatıldı. | PO review |
| `cursor-closed-memory-limit` | Cursor closed after { $rows } rows: the memory limit was reached. | { $rows } satırdan sonra imleç kapandı: bellek sınırına ulaşıldı. | PO review |
| `cursor-closed-connection-lost` | Cursor closed after { $rows } rows: the connection was lost. | { $rows } satırdan sonra imleç kapandı: bağlantı koptu. | PO review |
| `cursor-closed-server-terminated` | Cursor closed after { $rows } rows: the server ended the session. | { $rows } satırdan sonra imleç kapandı: sunucu oturumu sonlandırdı. | PO review |
| `cursor-closed-cancelled` | Cursor closed after { $rows } rows: fetching was stopped. | { $rows } satırdan sonra imleç kapandı: getirme durduruldu. | PO review |
| `cursor-closed-enforcement-gap` | Cursor closed after { $rows } rows: the read-only guard closed the session. | { $rows } satırdan sonra imleç kapandı: salt okunur koruma oturumu kapattı. | PO review |
| `cursor-closed-rolled-back` | Hacivat's transaction was rolled back. | Hacivat'ın işlemi geri alındı. | PO review |
| `exec-rows-retrieved` | { $rows } rows retrieved in { $duration } (one: { $rows } row retrieved in { $duration }) | { $rows } satır alındı ({ $duration }) | PO review |
| `exec-rows-retrieved-more` | { $rows } rows retrieved in { $duration }; more are available (one: { $rows } row retrieved in { $duration }; more are available) | { $rows } satır alındı ({ $duration }); devamı var | PO review |
| `exec-rows-affected` | { $rows } rows affected in { $duration } (one: { $rows } row affected in { $duration }) | { $rows } satır etkilendi ({ $duration }) | PO review |
| `exec-completed` | Completed in { $duration } | Tamamlandı ({ $duration }) | PO review |
| `exec-tx-committed` | Transaction committed | İşlem commit edildi | PO review |
| `exec-tx-rolled-back` | Transaction rolled back | İşlem geri alındı | PO review |
| `exec-summary` | { $executed } of { $total } statements executed, { $failed } failed, in { $duration } | { $total } ifadeden { $executed } tanesi çalıştı, { $failed } tanesi başarısız oldu ({ $duration }) | PO review |
| `exec-error` | Statement { $n } failed | { $n }. ifade başarısız oldu | PO review |
| `exec-cancelled` | Cancelled after { $duration } | { $duration } sonra iptal edildi | PO review |
| `exec-more-result-sets` | { $skipped } more result sets were read and not shown; only the first one is kept. (one: { $skipped } more result set was read and not shown; only the first one is kept.) | { $skipped } sonuç kümesi daha okundu ve gösterilmedi; yalnızca ilki tutulur. | PO review |
| `exec-deny-read-only-input` | Not sent: this console is read-only, and the statement can change data or schema. | Gönderilmedi: bu konsol salt okunur ve ifade veriyi ya da şemayı değiştirebilir. | PO review |
| `exec-deny-read-only-transaction` | Not sent: a read-only console can't start or end transactions. | Gönderilmedi: salt okunur bir konsol işlem başlatamaz ya da bitiremez. | PO review |
| `exec-deny-read-only-protocol` | Stopped: the server reported a write inside the read-only transaction. | Durduruldu: sunucu, salt okunur işlemin içinde bir yazma bildirdi. | PO review |
| `exec-deny-read-only-engine` | The server refused the statement because the transaction is read-only. | Sunucu, işlem salt okunur olduğu için ifadeyi reddetti. | PO review |
| `exec-deny-read-only-function` | Not sent: the statement calls a function whose effects a rollback can't undo. | Gönderilmedi: ifade, etkileri geri alınamayan bir fonksiyon çağırıyor. | PO review |
| `exec-events-dropped` | { $n } events not shown (one: { $n } event not shown) | { $n } olay gösterilmedi | PO review |
| `exec-copy-unsupported` | Not sent: COPY … FROM STDIN and TO STDOUT need the client to stream data, which consoles don't do. | Gönderilmedi: COPY … FROM STDIN ve TO STDOUT, istemcinin veri aktarmasını gerektirir; konsollar bunu yapmaz. | PO review |
| `hv-tx-open` | Transaction open · { $n } statements (one: Transaction open · { $n } statement) | İşlem açık · { $n } ifade | PO review |
| `hv-tx-failed` | Transaction failed · roll back to end it | İşlem başarısız · bitirmek için geri alın | PO review |
| `hv-tx-open-tip` | The statements since BEGIN aren't committed yet. Commit or roll back from the command palette. | BEGIN'den bu yana çalışan ifadeler henüz commit edilmedi. Komut paletinden commit edin ya da geri alın. | PO review |
| `hv-tx-failed-tip` | An error or a cancel stopped the transaction. The server now accepts only a rollback. | Bir hata ya da iptal işlemi durdurdu. Sunucu artık yalnızca geri almayı kabul eder. | PO review |
| `hv-tx-cmd-commit` | Commit transaction | İşlemi commit et | PO review |
| `hv-tx-cmd-rollback` | Roll back transaction | İşlemi geri al | PO review |
| `hv-tx-close-title` | { $name } has an open transaction with { $n } statements (one: { $name } has an open transaction with { $n } statement) | { $name } üzerinde { $n } ifadelik açık bir işlem var | PO review |
| `hv-tx-close-body` | Closing this console ends its session. Commit keeps the changes; Roll back discards them. | Bu konsolu kapatmak oturumunu sonlandırır. Commit değişiklikleri korur, geri alma onları atar. | PO review |
| `hv-tx-quit-body` | Quitting ends the session. Commit keeps the changes; Roll back discards them. | Çıkmak oturumu sonlandırır. Commit değişiklikleri korur, geri alma onları atar. | PO review |
| `hv-tx-commit-close` | Commit and close | Commit et ve kapat | PO review |
| `hv-tx-rollback-close` | Roll back and close | Geri al ve kapat | PO review |
| `hv-tx-commit-quit` | Commit and quit | Commit et ve çık | PO review |
| `hv-tx-rollback-quit` | Roll back and quit | Geri al ve çık | PO review |
| `hv-tx-cancel` | Cancel | Vazgeç | PO review |
| `hv-tx-failed-title` | The transaction on { $name } failed | { $name } üzerindeki işlem başarısız oldu | PO review |
| `hv-tx-failed-body` | An error or a cancel stopped it, so it can only be rolled back. Its { $n } statements won't be kept. (one: An error or a cancel stopped it, so it can only be rolled back. Its { $n } statement won't be kept.) | Bir hata ya da iptal onu durdurdu; artık yalnızca geri alınabilir. İçindeki { $n } ifade korunmayacak. | PO review |
| `hv-copy-done` | Copied { $rows } × { $cols } cells | { $rows } × { $cols } hücre kopyalandı | PO review |
| `hv-copy-done-line` | Tab-separated, raw values | Sekmeyle ayrılmış, ham değerler | PO review |
| `hv-copy-working` | Copying { $cells } cells… | { $cells } hücre kopyalanıyor… | PO review |
| `hv-copy-refused-title` | Too much to copy | Kopyalanamayacak kadar büyük | PO review |
| `hv-copy-refused-body` | The selection has { $cells } cells. Copy takes at most { $max } cells or { $size }; select fewer rows or columns. | Seçimde { $cells } hücre var. Kopyalama en çok { $max } hücre ya da { $size } alır; daha az satır ya da sütun seçin. | PO review |

162 strings, each with a Turkish translation. Not included: mock-only labels (mock bar, Notes drawer, sketch tags) and literal data.

