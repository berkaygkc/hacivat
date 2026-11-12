# M0 IME checklist (manual, PO)

Run `cargo run -p spike-editor -- --file <any.sql>`, click into the window so it has focus, and type on line 1. One table per OS; copy the tables for Windows 11 and Ubuntu 24.04. The Observed and Pass/Fail columns are empty on purpose: the PO fills them in.

Host: ______ (OS version, input source versions, VM or bare metal)

## For every row, also check

1. Composition underline: the in-progress text is underlined while composing, and the underline disappears on commit.
2. Caret position: the caret sits at the end of the composition (or at the IME's chosen position inside it), never at the line start.
3. Candidate window placement: the candidate window sits directly under the composition, and follows it when the line scrolls or the window moves.
4. Mid-composition click-away (Review Focus 4): start a composition (for example `kanji`, do not commit), then click another app and come back. Match the native editor (TextEdit on macOS). The composition survives the round-trip, still owned by the IME, and the candidate list may reappear. Esc, Enter or the next keystroke acts on that composition only. The text must not be lost or duplicated, and the underline must be gone once the composition is committed.
5. The completion popup must not open during a composition.
6. CJK rows (Japanese and Chinese): right after each composition keystroke, before the next frame is drawn, the candidate window x is estimated from monospace character widths, not the exact shaped line. Watch whether the candidate window jumps or sits visibly misplaced (horizontally off the composition) and record it in Observed.
7. Tab: press Tab outside a composition; a tab character must be inserted.

## macOS (Apple M4 Pro, macOS 26.6.2)

| Input source | Input | Expected result | Observed | Pass/Fail |
|---|---|---|---|---|
| Turkish Q | `ğ` `ü` `ş` `ı` `ö` `ç` `İ` (the dedicated keys) | `ğüşıöçİ` | | |
| Turkish Q | `Shift+i` | `İ` (dotted capital) | | |
| Turkish Q | `i` | `i` (dotted lowercase) | | |
| Turkish Q | `ı` key, then `Shift+ı` | `ı`, then `I` | | |
| Turkish F | the F-layout keys for `ğüşıöçİ` | `ğüşıöçİ` | | |
| Turkish F | `Shift+i`, then `i` | `İ`, then `i` | | |
| US with dead keys (U.S. International) | `^` then `a` | `â` | | |
| US with dead keys | `` ` `` then `e` | `è` | | |
| US with dead keys | `^` then space | `^` | | |
| Japanese (Kotoeri) | type `kanji`, check the candidate window, commit with Enter | `漢字` (or the chosen candidate), candidate window under the caret | | |
| Japanese (Kotoeri) | type `kanji`, press Escape | composition cancelled, no text left | | |
| Chinese Pinyin | type `zhongwen`, pick the first candidate with Space or Enter | `中文`, candidate window under the caret | | |
| Any IME | start a composition, click away, return | see check 4 | | |

## Windows 11 (x86_64)

| Input source | Input | Expected result | Observed | Pass/Fail |
|---|---|---|---|---|
| Turkish Q | `ğüşıöçİ`, `Shift+i`, `i` | `ğüşıöçİ`, `İ`, `i` | | |
| Turkish F | the F-layout keys for `ğüşıöçİ`, `Shift+i`, `i` | `ğüşıöçİ`, `İ`, `i` | | |
| US with dead keys (US-International) | `^` then `a`; `` ` `` then `e` | `â`; `è` | | |
| Japanese (MS-IME) | type `kanji`, commit with Enter | `漢字`, candidate window under the caret | | |
| Chinese Pinyin | type `zhongwen`, commit | `中文`, candidate window under the caret | | |
| Any IME | start a composition, click away, return | see check 4 | | |

## Ubuntu 24.04

| Input source | Input | Expected result | Observed | Pass/Fail |
|---|---|---|---|---|
| Turkish Q | `ğüşıöçİ`, `Shift+i`, `i` | `ğüşıöçİ`, `İ`, `i` | | |
| Turkish F | the F-layout keys for `ğüşıöçİ`, `Shift+i`, `i` | `ğüşıöçİ`, `İ`, `i` | | |
| US with dead keys (English US, intl. with dead keys) | `^` then `a`; `` ` `` then `e` | `â`; `è` | | |
| Japanese (Mozc via IBus or Fcitx5) | type `kanji`, commit with Enter | `漢字`, candidate window under the caret | | |
| Chinese Pinyin (IBus or Fcitx5) | type `zhongwen`, commit | `中文`, candidate window under the caret | | |
| Any IME | start a composition, click away, return | see check 4 | | |

Pass criterion (spec section 6, M0): correct composition, caret and candidate window placement for Turkish Q, Turkish F, dead keys, and Japanese and Chinese IME.

## Known gaps (M0)

Both gaps fall under Review Focus 4. Both are pending the GPUI patch `crates/ui/hacivat-ui-core/patches/gpui-discard-marked-text-on-focus-lost.patch`, which is not applied. At zed@912159c, GPUI on macOS gives the app no way to make `NSTextInputContext` drop its composition.

- **In-window programmatic focus move during composition.** The editor commits the marked text locally, but the IME keeps its own copy of the composition. The next IME delivery can then land beside the committed text. Clicks and keystrokes do not trigger this, because the IME sees them first. The patch closes this gap.
- **Focus move while the window is inactive.** GPUI delivers no blur for this, because the previous focus path of an inactive window is empty (`crates/gpui/src/window.rs:3773-3777`). The underline can stay stuck on the old editor. The patch alone does not close this gap: GPUI would also need to deliver the blur, or `FocusLost`, for inactive windows.

Not a gap: in ABC – Extended, Option+I is the horn dead key, so Option+I then A gives `ʼa`. That matches TextEdit. Circumflex in that layout is Option+6.
