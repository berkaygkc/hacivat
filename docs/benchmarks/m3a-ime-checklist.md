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
- **Windows and Linux rows** are open until those hosts exist (R-M3-7).
