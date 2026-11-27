# M3a IME checklist (manual, PO)

The M0 checklist (`m0-ime-checklist.md`) retargeted from `spike-editor` to the real console editor in the app, plus UI-EDIT-12 test 3 (Windows Turkish Q, AltGr dead `~` then Space). Gate item E7 (R-M3-7): the macOS rows are re-run on the real editor; the Windows and Linux rows stay open until hosts exist. The Observed and Pass/Fail columns are empty on purpose: the PO fills them in.

## Setup

1. Build and start the app: `cargo run --release -p hacivat`.
2. A console needs a connection before it shows an editor (an unbound console is the connections home, R-M3-37). Any connection works; the quickest needs no server: create a SQLite connection to a new file in the connection dialog, then connect it. The focused console binds to it and shows the editor.
3. Click into the editor so it has the keyboard focus, and type on line 1. For check 3 at line 49,000, paste the B3 fixture into the console (`cargo xtask gen-sql --lines 50000 --long-line 100000 --out target/fixtures/b3-50k.sql`).

Host: ______ (OS version, input source versions, VM or bare metal), build: ______ (`git rev-parse --short HEAD`)

## For every row, also check

1. Composition underline: the in-progress text is underlined while composing, and the underline disappears on commit.
2. Caret position: the caret sits at the end of the composition (or at the IME's chosen position inside it), never at the line start.
3. Candidate window placement: the candidate window sits directly under the composition, and follows it when the line scrolls or the window moves. Repeat once at line 49,000 of the B3 fixture (`b3 --bench ime-bounds` times the same question: UI-EDIT-12 test 8, < 1 ms).
4. Mid-composition click-away: start a composition (for example `kanji`, do not commit), click another app and come back. Match the native editor (TextEdit on macOS): the composition survives, Esc, Enter or the next keystroke acts on it only, and no text is lost or duplicated.
5. No key binding fires during a composition (F15): `⌘↵` (Run) and the editor commands do nothing until the composition is committed; the toolbar's Run commits the composition first and keeps the editor focused.
6. The status bar's `Ln, Col` follows the caret through the composition, and the Run preview band never shows a range over half-composed text.
7. Undo: one `⌘Z` removes one whole composition (one undo group), never part of it.
8. Tab outside a composition inserts a tab character.

There is no completion popup in M3a, so the M0 check "the completion popup must not open during a composition" does not apply until M3b.

## macOS

| Input source | Input | Expected result | Observed | Pass/Fail |
|---|---|---|---|---|
| Turkish Q | `ğ` `ü` `ş` `ı` `ö` `ç` `İ` (the dedicated keys) | `ğüşıöçİ` | | |
| Turkish Q | `Shift+i` | `İ` (dotted capital) | | |
| Turkish Q | `i` | `i` (dotted lowercase) | | |
| Turkish Q | `ı` key, then `Shift+ı` | `ı`, then `I` | | |
| Turkish F | the F-layout keys for `ğüşıöçİ` | `ğüşıöçİ` | | |
| Turkish F | `Shift+i`, then `i` | `İ`, then `i` | | |
| ABC – Extended (dead keys) | `Option+6` then `a` | `â` | | |
| ABC – Extended (dead keys) | `` Option+` `` then `e` | `è` | | |
| ABC – Extended (dead keys) | `Option+6` then Space | `^` | | |
| U.S. (UI-EDIT-12 test 4) | `Option+e` then `e` | `é`, one undo group, no binding fired for `Option+e` | | |
| Japanese (Kotoeri) | type `kanji`, check the candidate window, commit with Enter | `漢字` (or the chosen candidate), candidate window under the composition | | |
| Japanese (Kotoeri) | type `kanji`, press Escape | composition cancelled, no text left (as native: the first Esc may revert to kana) | | |
| Chinese Pinyin | type `zhongwen`, pick the first candidate with Space or Enter | `中文`, candidate window under the composition | | |
| Any IME | start a composition, click away, return | see check 4 | | |
| Any IME | start a composition, press `⌘↵` | nothing runs; the composition stays (check 5) | | |

### macOS: the T37 list from Task 27 (`task-27-report.md` items 1–7 and the fix round's (a), (b))

| # | Input source / setup | Input | Expected result | Observed | Pass/Fail |
|---|---|---|---|---|---|
| 1a | Japanese, a console connected to a server; a statement that fails, run earlier | start a composition, then let the failed run's error arrive and use its "go to error" (T33's `select_range`) while still composing | no duplicated or lost text; the selection moves only after the composition commits (UI-EDIT-12 test 7) | | |
| 1b | Japanese | while composing, choose **Edit ▸ Paste** from the macOS menu bar | nothing runs (F15); the composition stays intact; no duplicate | | |
| 1c | Japanese | while composing, choose **Edit ▸ Undo** from the macOS menu bar | nothing runs (F15); the composition stays intact | | |
| 2a | Japanese, Chinese Pinyin, Korean (2-Set) | compose after scrolling the editor | the candidate window sits at the composition start, within 2 px | | |
| 2b | Japanese, Chinese Pinyin, Korean | compose at line 49,000 of the 50k-line fixture | same | | |
| 2c | Japanese, Chinese Pinyin, Korean | compose on a line of CJK wide characters (for example `中文中文中文`) | same; any jump before the next frame is recorded (the estimate counts UTF-16 columns × the advance) | | |
| 2d | Japanese | scroll the caret out of view, then start a composition | the editor reveals the caret; the candidate window sits at the composition | | |
| 3a | Japanese | while composing, click in the results tile, or open the palette (in-window focus move) | the text commits once; no duplicate | | |
| 3b | Japanese | while composing, ⌘Tab away and back | the composition is kept, then commits once | | |
| 3c (fix (a)) | Japanese | while composing, switch apps, move the focus with the mouse, then come back | the marked text is as it was; the next keystroke starts fresh | | |
| 4a | U.S. | `Option+e` then `e` | `é`, one undo group, no binding fired | | |
| 4b | Turkish Q | type `ı` and `İ` among plain text, then undo | they type plainly; the undo groups match the typing (IME commits use the executor clock) | | |
| 5a | Japanese | while composing, click inside the marked text | AppKit commits first; no duplicate | | |
| 5b | Japanese | while composing, click outside the marked text | commits once; the caret goes to the click | | |
| 5c (fix (b)) | any | click the left half of a line's last character | the caret goes **before** that character | | |
| 6a | Japanese | reconversion: select committed text and press the Kana key twice | the reconversion candidates show for that text; the result replaces it once | | |
| 6b | Emoji & Symbols (`⌃⌘Space`) | insert astral characters (for example `😀🇹🇷`) | inserted whole; the caret steps over each as one character | | |
| 6c | Dictation | dictate a long sentence | the text lands once, at the caret | | |
| 7a | clipboard (another app → Hacivat) | paste CRLF text from a Windows-made file, text with a leading BOM, and text with NUL characters | every CRLF/CR/LF becomes the document's dominant ending; one leading BOM is stripped; each NUL is kept and drawn as a `NUL` chip, with the warning toast | | |
| 7b | clipboard | paste more than 20 MB | refused, with the toast; the text is unchanged | | |
| 7c | clipboard (Hacivat → Hacivat through the pasteboard) | copy a whole line with no selection, then paste on another line | it pastes as a line above the caret's line, the caret's column kept (the metadata survives the pasteboard) | | |
| 7d | clipboard | paste through the platform's own paste path (for example the Edit menu while a text input handles it) | the same normalisation and guards as the Paste command | | |

## Windows 11 (x86_64) — open until a host exists

| Input source | Input | Expected result | Observed | Pass/Fail |
|---|---|---|---|---|
| Turkish Q | `ğüşıöçİ`, `Shift+i`, `i` | `ğüşıöçİ`, `İ`, `i` | | |
| Turkish Q (UI-EDIT-12 test 3) | in `WHERE name ` type `AltGr+ü` (dead `~`), then Space | exactly one `~` inserted; no marked text left; no binding fired | | |
| Turkish F | the F-layout keys for `ğüşıöçİ`, `Shift+i`, `i` | `ğüşıöçİ`, `İ`, `i` | | |
| US-International (dead keys) | `^` then `a`; `` ` `` then `e` | `â`; `è` | | |
| Japanese (MS-IME) | type `kanji`, commit with Enter | `漢字`, candidate window under the composition | | |
| Chinese Pinyin | type `zhongwen`, commit | `中文`, candidate window under the composition | | |
| Any IME | start a composition, click away, return | see check 4 | | |

## Ubuntu 24.04 — open until a host exists

| Input source | Input | Expected result | Observed | Pass/Fail |
|---|---|---|---|---|
| Turkish Q | `ğüşıöçİ`, `Shift+i`, `i` | `ğüşıöçİ`, `İ`, `i` | | |
| Turkish F | the F-layout keys for `ğüşıöçİ`, `Shift+i`, `i` | `ğüşıöçİ`, `İ`, `i` | | |
| English US, intl. with dead keys | `^` then `a`; `` ` `` then `e` | `â`; `è` | | |
| Japanese (Mozc via IBus or Fcitx5) | type `kanji`, commit with Enter | `漢字`, candidate window under the composition | | |
| Chinese Pinyin (IBus or Fcitx5) | type `zhongwen`, commit | `中文`, candidate window under the composition | | |
| Any IME | start a composition, click away, return | see check 4 | | |

Pass criterion (spec §6, IME gate): correct composition, caret and candidate-window placement for Turkish Q, Turkish F, dead keys, and Japanese and Chinese IME, on the real editor.

## Scripted traces (run in CI)

The manual rows above sit on top of the scripted IME traces, which run on every PR:

- `crates/ui/hacivat-text/tests/ime_traces.rs` (Task 10): the composition model, UTF-16 offsets, one undo group per composition.
- `crates/ui/hacivat-ui-editor/tests/ime_view.rs` (Task 27): the editor's input handler — `bounds_for_range` within 2 px of the caret and between an edit and the next frame, `character_index_for_point`, marked text after an emoji, the blur rule for active and inactive windows, app edits held while composing, notify only after non-IME edits; `bounds_for_range_at_line_49000_under_1ms` is a perf test (`HACIVAT_PERF=1`, the perf lines).

## Known gaps

- **Focus move while the window is inactive** (M0, still open → M5): GPUI delivers no blur for an inactive window, so a composition can stay on the old editor. Task 27's `blur_rule_active_and_inactive_window` and `a_composition_dropped_while_the_window_was_inactive_ends` pin what the editor does today.
- **Windows and Linux rows** are open until those hosts exist (R-M3-7), and so is whether the clipboard's line-wise metadata survives on those platforms.
- **Fallback if 1a–1c duplicate text** (Task 27): a fork carry that discards the marked text on `ContentChanged` while composing.

<!-- ## m3c-t45 -->
## M3c: several carets (Task 31 mirroring; M3c gate E2)

Re-run the macOS table above on the M3c build, then these rows. Setup: as above. Add carets with
⌥⌘↓ (`editor.add_caret_below`) or Option-click; one undo is ⌘Z. "Mirroring" means the IME's
composition is typed at every caret while the OS sees only the main caret (R-M3-64). If any other
caret sits inside a string or a comment, the composition goes to the main caret only and the status
line says "Input method: typing at the main caret only, because another caret was inside a string
or comment" / "Giriş yöntemi: başka bir imleç bir dizenin ya da yorumun içinde olduğu için yalnızca
ana imleçte yazılıyor".

| # | Input source / setup | Input | Expected result | Observed | Pass/Fail |
|---|---|---|---|---|---|
| M1 | Japanese; three carets on three plain lines (`SELECT 1` ×3, carets at line ends) | type `kanji`, commit with Enter | `漢字` (or the chosen candidate) at all three carets; the underline shows at every caret while composing; one ⌘Z removes all three | | |
| M2 | Japanese; three carets, one of them inside `'…'` | type `kanji`, commit | the fallback sentence shows; text goes in at the main caret only; the other carets stay | | |
| M3 | Japanese, Chinese Pinyin; three carets on lines far apart | compose | the candidate window sits under the **main** caret only | | |
| M4 | U.S.; three carets | press and hold `e`, choose `é` | `é` at all three carets (the platform range moves every mirror); no extra `e` left | | |
| M5 | Japanese; three carets | while composing, press ⌘↵ | nothing runs (F15); the composition stays at every caret | | |
| M6 | Japanese; three carets | type `'` at all carets, then start a composition right away | the fallback applies (the carets are now inside strings) | | |
| M7 | Japanese; fallback active | start a composition, press Escape | composition cancelled; all three carets come back as they were | | |
| M8 | Turkish Q; three carets | `ğüşıöçİ`, `Shift+i`, `i` | typed at every caret, `İ`/`i` exact; one undo group per run of typing | | |
| M9 | any | Option-click on a caret's own spot while the pointer jitters by a pixel | the carets stay (no stray column selection) | | |
| M10 | any; a line of CJK wide characters | Option-drag a column across it | the column lands on whole characters (a wide glyph is two advances) | | |

Linux (open until a host exists): primary selection on X11 and Wayland, middle-click paste at
several carets; whether the clipboard's per-caret pieces survive on Windows and Linux.
