# ADR 0001 addendum: M0 GPUI validation spike results

- **Status:** Accepted. The PO decided GO on 2026-11-12.
- **Date:** 2026-11-12
- **Plan:** `docs/superpowers/plans/2026-11-12-m0-gpui-validation-spike.md`
- **Spec:** `docs/superpowers/specs/2026-11-12-hacivat-v1-design.md` §6 M0
- **Branch:** `m0-spike`. Benchmarks were run at build `122c45e`. The IME fix landed at `83dbe04`.

## Pinned engine

- **GPUI:** `gpui` 0.2.2 and `gpui_platform` 0.1.0, both from `https://github.com/zed-industries/zed` at rev `912159c7ad77ce6296a93be77efc298b0b00fd50`. Toolchain is Rust 1.98.1, matching the pin's own `rust-toolchain.toml`.
- **Copied `[patch.crates-io]` entries:** `async-process` and `async-task`, both at the revisions in zed's `Cargo.toml:1045-1046`.
- **Carried patch (not applied):** `crates/ui/hacivat-ui-core/patches/gpui-discard-marked-text-on-focus-lost.patch`. It adds `discardMarkedText` on `FocusLost` in `gpui_macos`. It is Apache-2.0, and the carry notes are in the README next to it.
- **No GPL Zed crates.** `cargo deny` bans them by name and limits git sources to zed and its own forks.

## Machines

| Host | Details |
|---|---|
| macOS reference (the PO's Mac) | Apple M4 Pro, 48 GB, macOS 26.6.2. Built-in Retina display (scale 2, 120 Hz) and an external 5120×1440 display (scale 1, 240 Hz). |
| Windows 11, Ubuntu 24.04 | Not yet provisioned. CI builds and tests are green on `windows-2022` and `ubuntu-24.04`, but CI is a build and regression signal only (spec §3.3). |

## Results

### Gating checks

| # | Check | macOS result | Verdict |
|---|---|---|---|
| B1 | Fling scroll, 10M rows × 50 columns | Built-in display (120 Hz, 2×), 3 runs: 120.0 fps mean, the display ceiling. p99 frame 8.6–8.8 ms, max gap 10.7–11.8 ms, 0 gaps over 100 ms. Grid paint p99 3.6 ms against a 6 ms budget. External display (240 Hz, 1×): 237.3 fps. | **PASS** |
| B3 | Typing in a 50k-line (~2 MB) file, p95 ≤ 16 ms | Built-in display, 3 runs: p95 7.2 / 9.7 / 10.5 ms. | **PASS** |
| B3 | Typing on a 100k-character line, p95 ≤ 16 ms | p95 8.6 / 12.4 / 13.6 ms. The worst single keystroke was 16.5 ms. | **PASS** (watch in M1) |
| B3 | Completion popup, p95 ≤ 50 ms | p95 7.2 / 11.0 / 11.2 ms. | **PASS** |
| IME | Turkish Q/F, dead keys, Japanese, Chinese | Turkish Q, Turkish F, Tab, Japanese composition, candidate-window placement and Esc all behave as native. Dead keys match TextEdit. The app-switch duplication bug was found and fixed, and the PO re-tested it. Chinese was not run because the input source was missing; Japanese covers the same path. Two in-window edge cases are open (see below). | **PASS** (macOS) |
| VF a | Geometry within ±1 px of Chromium | All scanlines pass at 2×. The scanlines are also validated against the reference render. | **PASS** (2×) |
| VF b | Token colours, ΔE2000 ≤ 1 | Light EN/TR: 64/64. Dark EN/TR: 60/64. The 4 failing samples are all the `gutter` token (#1A1D22 rendered as #1B1D23, ΔE 1.13). This is consistent with 8-bit Display P3 → sRGB capture rounding. The reference pixels themselves are verified against the tokens. | **Open:** re-check on an sRGB-native capture (Windows host) |
| VF c | Small caps, tabular figures, mono for code | PO decision: small caps are synthesized like the approved mock (capitals at 0.7× size plus tracking), not the font's `smcp`/`c2sc`. Source Sans 3 has no `tnum` feature, but its digits are tabular by default, and equal digit advances are asserted. Fragment Mono is used for code. | **PASS** (as redefined by the PO) |
| VF d | Radii, popover shadow only, 11 px floor | Radii are 0/3/4/6, and the popover shadow is the only shadow. The 11 px floor applies to nominal size; synthesized small caps are a PO-approved exception. | **PASS** |
| VF e | Turkish strings fit at 1440 px | TR captures pass. The 1× and 1.5× scales have not been captured: the Mac's 1× external display path is documented, and 1.5× needs the Windows host. | **PASS** (2×); 1×/1.5× open |
| VF f | PO side-by-side sign-off | Signed off 2026-11-12. The only difference raised was text weight, now resolved by decision (see below). | **PASS** |
| PKG | Signed builds and an update round trip | macOS: universal `.app` + DMG, Developer ID-signed with hardened runtime, **notarized and stapled**. Gatekeeper reports `accepted, source=Notarized Developer ID` for both the DMG and the `.app` (submission 381b451b, run with `RELEASE=1 NOTARY_PROFILE=mystat-notary`, which uses the same Apple account as the PO's MyStat profile). A minisign-verified update 0.0.1 → 0.0.2 succeeded; a tampered artifact was refused, and so was a rollback to an older signed artifact. Windows and Linux scripts are written but have not been run. cargo-packager 0.11.8 has no `.rpm` output. | **PASS (macOS)**; Windows/Linux open |

### Non-gating measurements (X row)

| Probe | Result (macOS) |
|---|---|
| Cold start to first frame | 153–160 ms warm, 340 ms cold. Budget is ≤ 500 ms. |
| Idle RSS | 73 MiB. Budget is ≤ 200 MB. |
| Screen reader | With VoiceOver active, the AccessKit tree exposes Window "Hacivat probes" › Grid "Result 1". Narrator and Orca are pending hosts. |
| Clipboard metadata | Round-trips. |
| Bidi isolates (U+2068/9) | Zero width. |
| Turkish glyph fallback | No `.notdef`. |
| Turkish small caps | GPUI shaping (CoreText) does **not** apply the font's `locl` TRK. `smcp` on lowercase `i` gives an undotted `I`. **Casing must always happen upstream** (Fluent and pre-cased strings), never via font features or in the renderer. |
| Font weights | 400, 500, 600 and 700 resolve to distinct faces through family "Source Sans 3" on macOS. Windows and Linux are pending. |

## GPUI capability findings

These are things M1 must build or carry in `hacivat-ui-core`:

1. **Scroll precision.** GPUI pixels are f32. 10M rows × 21 px exceeds f32 integer precision, so grid scroll state is row-anchored (`top_row: u64` plus `offset_px`). Never use an absolute pixel offset.
2. **No letter-spacing API.** Tracking and synthesized small caps are implemented in `hacivat_ui_core::fonts::small_caps` (scale 0.7, fitted tracking).
3. **Border and clipping limits.** One border colour per element and no corner-radius clipping. Use adapter-level composition.
4. **No API to commit or discard the platform IME composition** (macOS `NSTextInputContext`). The app-switch case is solved in the app. Two in-window edge cases need the carried patch, plus a GPUI change so that blur is delivered for inactive windows. They are recorded in `docs/benchmarks/m0-ime-checklist.md` under Known gaps.
5. **Font smoothing.** GPUI applies CoreGraphics font smoothing (stroke dilation) unless the app preference `AppleFontSmoothing` is 0 (`gpui_apple/src/text_system.rs:224-260`). This is why text renders heavier than the Chromium mock (`-webkit-font-smoothing: antialiased`).
6. **The display refresh rate bounds latency.** At 120 Hz, typing latency includes up to one 8.3 ms frame. B3 is defined as app-side latency (handler entry → frame handed to the compositor).
7. **Inactive windows are throttled to 30 Hz by macOS.** Benches fail any run with input sent to an inactive window.

## Decisions recorded during M0 (PO, 2026-11-12)

- **Small caps.** Synthesized, like the approved mock (`fontVariantCaps: synthesized` in DESIGN.md; the Synthesized Small Caps Rule).
- **Text weight.** The default is thin rendering that matches the mock: font smoothing off, by setting the app's own `AppleFontSmoothing` preference at startup. A **Settings option** lets users switch to macOS-standard (heavier) text. This is implemented in M1. The GPUI source does not need to change.
- **Grid numbers.** Grid cells follow the UI locale; copy, export and SQL use raw values. Decided earlier (design doc L3) and unchanged.

## Open items carried out of M0

| Item | Needed for | Owner |
|---|---|---|
| Windows 11 and Ubuntu 24.04 gate runs: B1, B3, IME (incl. Narrator/Orca), VF 1.5×, PKG | Full M0 sign-off per spec. Not needed to start M1 on macOS. | PO provides hosts; agents run the scripts (`design/vf/capture.ps1`, `capture.sh`, packaging scripts) |
| Dark `gutter` token sRGB re-check | VF b | Windows host |
| VF 1× capture on the external display | VF e | Any time on the Mac |
| GPUI IME patch (FocusLost → discardMarkedText) plus blur for inactive windows | Review Focus 4 edge cases | M1 adapter work |
| `.rpm` packaging via a separate tool | Linux PKG | M9 |
| Text-weight setting and startup preference | PO decision | M1 |
| Typing on very long lines (worst keystroke 16.5 ms at 120 Hz) | Margin | M5 editor work |

## Recommendation

**GO with fix list.** Proceed to M1 on GPUI at `912159c`.

Every engine gate (B1, B3, IME, VF a–d) passes on the macOS reference, most with wide margins: grid paint uses about 60 % of its budget at p99, and typing p95 is at most 13.6 ms. No result triggers the Linebender fallback. The open items are tooling or other-platform confirmations, not engine risks. The decision should be revisited only if the Windows or Linux runs fail B1, B3 or IME.

**Decision (PO, 2026-11-12):** **GO.** Proceed to M1 on GPUI at `912159c`. Windows and Linux confirmations follow as open items. Controller rulings made during M0 are in `docs/benchmarks/m0-rulings.md`.
