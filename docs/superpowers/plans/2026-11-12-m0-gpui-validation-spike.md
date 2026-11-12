# M0: GPUI Validation Spike Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove or disprove that GPUI at a pinned commit can carry Hacivat. The spike is a go/no-go gate (B1 grid, B3 editor, IME, VF visual fidelity, PKG) that ends in an ADR addendum. It also leaves behind the two permanent pieces: the `hacivat-ui-core` adapter boundary and the design-token codegen.

**Architecture:** This is a new Cargo workspace. Only `hacivat-ui-core` imports `gpui`. It owns app bootstrap, generated theme tokens, bundled fonts and the frame/latency recorder, and re-exports a curated GPUI surface. Four throwaway spike binaries depend on `hacivat-ui-core`, never on `gpui` directly:
- `spike-grid` (B1)
- `spike-editor` (B3 + IME)
- `spike-harman` (VF)
- `spike-probes` (non-gating measurements)

`xtask` holds the permanent token codegen plus the bench and VF tooling.

**Tech Stack:**
- Rust 1.98.1, edition 2024
- `gpui` and `gpui_platform` from zed-industries/zed at rev `912159c7ad77ce6296a93be77efc298b0b00fd50` (Apache-2.0)
- `ropey` (spike-only rope)
- `yaml-rust2` and `serde_json` (codegen)
- `image` (VF)
- `cargo-packager` and `cargo-packager-updater` (PKG)
- `minisign`
- `cargo-deny`

**Spec:** `docs/superpowers/specs/2026-11-12-hacivat-v1-design.md`. Read §3.2, §3.3, §3.4, §5.1, §5.2, §5.4, §6 M0 and §7.3. Also read `docs/decisions/0001-native-ui-on-gpui.md`, `DESIGN.md` and `.impeccable/design.json`.

## Global Constraints

- **GPUI pin:** `git = "https://github.com/zed-industries/zed", rev = "912159c7ad77ce6296a93be77efc298b0b00fd50"`, crates `gpui` (0.2.2) and `gpui_platform` (0.1.0). Never depend on Zed's `editor`, `ui`, `workspace`, `rope` or `text` crates (GPL-3.0-or-later). Never read them for ideas either (ADR 0001 §1).
- **Toolchain:** `rust-toolchain.toml` channel `1.98.1`, matching the pin's own `rust-toolchain.toml`. Edition 2024.
- **Licensing:**
  - Every workspace crate is `license = "Apache-2.0"`, `publish = false`.
  - `cargo-deny` denies `GPL-*`, `AGPL-*`, `LGPL-*`, `SSPL-*` and `BUSL-*` licenses.
  - Fonts are OFL-1.1 (Source Sans 3, Fragment Mono). Icons are Lucide (ISC). License texts are bundled next to the assets.
- **Adapter rule:** no crate except `crates/ui/hacivat-ui-core` may list `gpui` or `gpui_platform` as a dependency. `cargo xtask check-deps` enforces this (Task 1).
- **Token source of truth:**
  - Colours, typography, radii and spacing come from the YAML frontmatter of `DESIGN.md`.
  - Shadows and motion come from `.impeccable/design.json` → `extensions.shadows` and `extensions.motion`.
  - Components use semantic token names only, never hex literals.
- **Design rules enforced in code:**
  - No text below 11 px.
  - Exactly one `cp-fill` quad per window.
  - Radii come only from `{0, 3, 4, 6, 9999}` px.
  - Only popovers have shadows.
- **Pass criteria (spec §6 M0, verbatim numbers):**
  - **B1:** ≥ 55 fps on macOS and Windows, ≥ 45 fps on the Linux reference, and no frame gap > 100 ms (p99 frame-time capture), with 10,000,000 rows × 50 columns.
  - **B3:** typing ≤ 16 ms p95 and completion popup ≤ 50 ms on a 50,000-line (~2 MB) SQL file.
  - **IME:** correct composition, caret and candidate window placement for Turkish Q, Turkish F, dead keys, and Japanese and Chinese IME.
  - **VF:** geometry within ±1 px of the Chromium render, colour ΔE2000 ≤ 1, real `smcp`/`c2sc` and `tnum`, Fragment Mono for code, nothing under 11 px, 3/4/6 px radii, popover shadow only, Turkish strings fit at 1440 px, and PO sign-off.
- **Grid metrics:** rows are 21 px in the result grid and 23 px in the explorer; toolbars are 32/34 px. Read the exact values from `DESIGN.md` `spacing`.
- **Authoritative numbers:**
  - Manual runs on the PO's Mac (Apple M4 Pro, macOS 26.6.2; record the RAM).
  - Manual runs on a rented or VM x86_64 Windows 11 host and an Ubuntu 24.04 host. Record the host type, because software-rendered VMs cannot reach the budgets.
  - Hosted CI is a relative regression signal only.
- **Packaging:**
  - macOS: Developer ID signed and notarized.
  - Windows: unsigned.
  - Linux: minisign-signed.
  - Updates are verified with minisign/ed25519, with the public key compiled in.
- **Time box:** one week for B1/B3/IME/PKG, plus at most one week for VF (ADR 0001, spec §6).
- **Git:** the project directory is not yet a git repository. Task 1 runs `git init` only after the PO approves committing (PO rule: commit only when asked). CI (Task 11) needs a GitHub repository that the PO creates.

## Review Focus

1. **Scroll precision at the far end of 10M rows.** 10M × 21 px = 210M px, which is beyond f32 integer precision. Scrolling to the last row must show row 9,999,999 exactly, with no jitter. → test `scroll_to_last_row_is_exact` in Task 5.
2. **Fractional display scale (Windows 150 %, Linux 1.25×).** 1 px seams must stay crisp and must not blur across two device pixels. → test `snap_to_device_px_is_pixel_aligned` in Task 2; VF also runs at 1.5× (Task 8).
3. **Turkish small caps and casing.** The explorer header renders "GEZGİN" (dotted İ). `smcp` on "i" without Turkish shaping, or a `str::to_uppercase`, produces "GEZGIN". → probe `turkish_small_caps` in Task 9. The renderer feeds pre-cased strings from Fluent, and casing never happens in render code.
4. **IME interrupted mid-composition (click elsewhere, focus loss, window switch).** The marked text must either commit or vanish, never linger. → test `unmark_after_focus_loss_commits_marked_text` in Task 7.
5. **One very long line in the SQL file (minified SQL, 100k-char line).** Typing must stay within budget. → bench case `typing_long_line` in Task 6. The shaping cache works on a visible-column window, not the whole line.

---

## File Structure

```
hacivat/
  Cargo.toml                      # workspace: members, [workspace.dependencies] (gpui pin), [patch.crates-io] copied from zed@pin as gpui needs
  rust-toolchain.toml             # channel = "1.98.1"
  deny.toml                       # license + ban policy
  .github/workflows/m0.yml        # build all OSes, tests, token staleness, check-deps, nightly bench (regression signal)
  crates/ui/hacivat-ui-core/
    Cargo.toml
    assets/fonts/{SourceSans3-*.ttf, FragmentMono-*.ttf, OFL.txt}
    assets/icons/lucide/*.svg + LICENSE (ISC)
    src/lib.rs                    # pub mod app, theme, fonts, metrics, frames; pub use of the curated gpui surface (mod gpui_surface)
    src/gpui_surface.rs           # the ONLY place `use gpui::…` is re-exported for other crates
    src/app.rs                    # run_app, open_window
    src/theme/mod.rs              # Color, TypeStyle, ThemeTokens, ThemeMode, Theme global
    src/theme/generated.rs        # written by `cargo xtask tokens`, never edited by hand
    src/fonts.rs                  # embed + register fonts; feature sets
    src/metrics.rs                # device-pixel snapping
    src/frames.rs                 # FrameRecorder, LatencyProbe, FrameStats, JSON dump
  xtask/
    src/main.rs                   # subcommands: tokens [--check], check-deps, gen-sql, bench-report, vf
    src/tokens.rs  src/check_deps.rs  src/gen_sql.rs  src/bench_report.rs  src/vf.rs
  spikes/
    spike-grid/src/{main.rs, source.rs, scroll.rs, element.rs, bench.rs}
    spike-editor/src/{main.rs, buffer.rs, element.rs, input.rs, completion.rs, bench.rs}
    spike-harman/src/{main.rs, screen/*.rs}          # static reproduction of the harman workspace
    spike-probes/src/main.rs                          # non-gating probes (X row)
  design/vf/samples.json          # VF colour sample points + geometry scanlines (Task 8)
  packaging/{Packager.toml, update-manifest.template.json, keys/README.md}
  docs/benchmarks/m0.md           # gate results (Task 12)
  docs/benchmarks/m0-ime-checklist.md
  docs/decisions/0001-addendum-m0.md
```

---

### Task 1: Workspace, GPUI pin, adapter skeleton, dependency guard

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `deny.toml`, `.gitignore`
- Create: `crates/ui/hacivat-ui-core/{Cargo.toml, src/lib.rs, src/gpui_surface.rs, src/app.rs}`
- Create: `xtask/{Cargo.toml, src/main.rs, src/check_deps.rs}`, `.cargo/config.toml` (alias `xtask = "run -p xtask --"`)
- Test: `crates/ui/hacivat-ui-core/tests/app_smoke.rs`, `xtask/src/check_deps.rs` (unit tests)

**Interfaces:**
- Produces:
  - `hacivat_ui_core::app::run_app(f: impl FnOnce(&mut App) + 'static)`. It wraps `gpui_platform::application().run`, registers fonts (Task 3) and installs the theme global (Task 2).
  - `hacivat_ui_core::app::open_window<V: Render>(cx: &mut App, title: &str, size_px: (f32, f32), build: impl FnOnce(&mut Window, &mut App) -> Entity<V>) -> WindowHandle<V>`
  - `hacivat_ui_core::gpui_surface::*`: re-exports of the GPUI types the spikes use. Start with `App, Context, Window, Entity, Render, IntoElement, ParentElement, Styled, div, px, Pixels, Bounds, Point, Size, canvas, ScrollWheelEvent, KeyDownEvent, EntityInputHandler, UTF16Selection, FocusHandle, Focusable, ClipboardItem, SharedString, Hsla`, and extend it as later tasks need.
  - `xtask::check_deps::offending_crates(metadata: &cargo_metadata::Metadata) -> Vec<String>`: names of workspace crates other than `hacivat-ui-core` that depend directly on `gpui` or `gpui_platform`.

- [ ] **Step 0: Ask the PO to approve `git init` and committing.** Without approval, skip every "Commit" step and keep going.
- [ ] **Step 1: Write the failing tests.**

```rust
// xtask/src/check_deps.rs
#[test] fn flags_spike_that_depends_on_gpui() {
    let md = fixture_metadata(&[("spike-x", &["gpui"]), ("hacivat-ui-core", &["gpui", "gpui_platform"])]);
    assert_eq!(offending_crates(&md), vec!["spike-x".to_string()]);
}
#[test] fn adapter_itself_is_allowed() {
    let md = fixture_metadata(&[("hacivat-ui-core", &["gpui"])]);
    assert!(offending_crates(&md).is_empty());
}
```
```rust
// crates/ui/hacivat-ui-core/tests/app_smoke.rs  (uses gpui's test-support feature)
#[gpui::test] fn opens_a_window(cx: &mut gpui::TestAppContext) {
    let handle = cx.update(|cx| hacivat_ui_core::app::open_window(cx, "smoke", (800.0, 600.0), |_, cx| cx.new(|_| Empty)));
    assert!(handle.is_ok());
}
```
- [ ] **Step 2:** Run `cargo test -p xtask -p hacivat-ui-core`. Expected: compile failure (items not defined).
- [ ] **Step 3: Write the workspace manifest and `rust-toolchain.toml`.**
  - Add `gpui`/`gpui_platform` to `[workspace.dependencies]` at the pin. `gpui_platform` gets the `font-kit`/`wayland`/`x11` features the pin enables by default.
  - Copy into the workspace `[patch.crates-io]` the entries from `zed@pin/Cargo.toml` that gpui's dependency tree resolves through. Known so far: `async-task`, `async-process`. Find the rest by building, and record every copied patch with a comment pointing to the zed line.
- [ ] **Step 4: Implement `run_app`, `open_window`, `gpui_surface` and `offending_crates`.** Use the `cargo_metadata` crate. `cargo xtask check-deps` exits 1 and prints the offenders.
- [ ] **Step 5: Write `deny.toml`.**
  - License allow-list: Apache-2.0, MIT, BSD-2/3, ISC, Zlib, Unicode-3.0, OFL-1.1, MPL-2.0, CC0-1.0.
  - Deny the copyleft licenses listed in Global Constraints.
  - Under `[bans]`, ban the zed GPL crate names `editor`, `ui`, `workspace`, `rope`, `text` when sourced from `github.com/zed-industries/zed`.
- [ ] **Step 6:** Run `cargo test -p xtask -p hacivat-ui-core && cargo xtask check-deps && cargo deny check licenses bans`. Expected: all pass. A window opens in the smoke test (headless test platform).
- [ ] **Step 7: Commit** with message `chore: workspace, gpui pin 912159c, ui-core adapter skeleton`.

### Task 2: Token codegen (`xtask tokens`) and theme API

**Files:**
- Create: `xtask/src/tokens.rs`
- Create: `crates/ui/hacivat-ui-core/src/theme/{mod.rs, generated.rs}`, `src/metrics.rs`
- Test: `xtask/src/tokens.rs` (unit tests), `crates/ui/hacivat-ui-core/src/theme/mod.rs` and `src/metrics.rs` (unit tests)

**Interfaces:**
- Consumes: the `DESIGN.md` frontmatter keys `colors`, `typography`, `rounded`, `spacing`, and the `design.json` keys `extensions.shadows[]{name, layers[]{x,y,blur,spread,color}}` and `extensions.motion[]{name, durationMs | value}`.
- Produces:
  - `pub struct Color(pub u32)`, packed as `0xRRGGBBAA`, plus `Color::to_hsla(self) -> Hsla`.
  - `pub struct TypeStyle { pub family: &'static str, pub size_px: f32, pub weight: u16, pub line_height: f32, pub features: &'static [&'static str] }`
  - `pub struct ShadowLayer { pub x: f32, pub y: f32, pub blur: f32, pub spread: f32, pub color: Color }`
  - `pub struct ThemeTokens { pub colors: Colors, pub type_ramp: TypeRamp, pub radii: Radii, pub spacing: Spacing, pub popover_shadow: &'static [ShadowLayer], pub motion: Motion }`
  - `generated.rs` defines `pub const DARK: ThemeTokens` and `pub const LIGHT: ThemeTokens`, plus `pub struct Colors` (one `Color` field per token, kebab→snake with the `dark-`/`light-` prefix stripped, e.g. `text_2`, `cp_fill`, `on_cp_fill`), `TypeRamp` (one `TypeStyle` per typography key), `Radii { none, xs, sm, md, full }`, `Spacing` (one `f32` per key; keys that start with a digit become `s_2xs`, …) and `Motion` (one `u32` ms per duration token, plus easing tokens as `[f32; 4]` cubic-bezier).
  - Unprefixed colour keys (window controls) go into `pub const SHARED: SharedColors`.
  - `pub enum ThemeMode { Dark, Light }`; `pub struct Theme { pub mode: ThemeMode, pub tokens: &'static ThemeTokens }`, installed as a GPUI global; `Theme::get(cx: &App) -> &Theme`.
  - `pub fn snap_to_device_px(logical: f32, scale: f32) -> f32` in `metrics.rs`.
  - `xtask tokens [--check]`: `--check` exits 1 with a unified diff when `generated.rs` is stale.

- [ ] **Step 1: Write the failing tests.**

```rust
// xtask/src/tokens.rs
#[test] fn parses_dark_ground() { let t = load(DESIGN_MD_FIXTURE, DESIGN_JSON_FIXTURE).unwrap(); assert_eq!(t.dark.color("ground"), Some(0x16181DFF)); }
#[test] fn parses_rgba_tint() { let t = load(..).unwrap(); assert_eq!(t.dark.color("blue-tint"), Some(0x7AA2F71F)); } // 0.12*255 = 30.6 -> round = 31 = 0x1F
#[test] fn dark_and_light_keysets_match() { let t = load(..).unwrap(); assert_eq!(t.dark.keys(), t.light.keys()); }
#[test] fn rejects_mismatched_keysets() { assert!(load(MD_MISSING_LIGHT_GROUND, JSON).is_err()); }
#[test] fn generated_file_is_current() { assert_eq!(render(&load_repo_sources().unwrap()), std::fs::read_to_string(GENERATED_RS).unwrap()); }
```
```rust
// hacivat-ui-core theme tests (Named Rules)
#[test] fn eleven_px_floor() { for s in DARK.type_ramp.all() { assert!(s.size_px >= 11.0, "{}", s.family); } }
#[test] fn radii_on_scale() { assert_eq!([DARK.radii.none, DARK.radii.xs, DARK.radii.sm, DARK.radii.md], [0.0, 3.0, 4.0, 6.0]); }
#[test] fn cp_fill_is_saturated_red_with_white_text_in_both_themes() {
    assert_eq!(DARK.colors.cp_fill.0, 0xCE2C31FF); assert_eq!(LIGHT.colors.cp_fill.0, 0xCE2C31FF);
    assert_eq!(DARK.colors.on_cp_fill.0, 0xFFFFFFFF);
}
// metrics.rs
#[test] fn snap_to_device_px_is_pixel_aligned() {
    for scale in [1.0, 1.25, 1.5, 2.0] { for v in [0.3, 1.0, 20.5, 276.0] {
        let d = snap_to_device_px(v, scale) * scale; assert!((d - d.round()).abs() < 1e-4);
    } }
}
```
- [ ] **Step 2:** Run `cargo test -p xtask tokens && cargo test -p hacivat-ui-core theme metrics`. Expected: FAIL (items not defined).
- [ ] **Step 3: Implement `tokens::load`, `tokens::render` and the `tokens` subcommand.**
  - Parse the YAML with `yaml-rust2`, taking the frontmatter between the first two `---` lines.
  - Accepted colour forms: `#RRGGBB`, `#RRGGBBAA`, `rgba(r,g,b,a)`. Alpha is rounded half-up to a byte.
  - Size and length values arrive as `"13.5px"`; strip the unit, and reject anything that is not `px`.
  - Motion easing tokens arrive as `cubic-bezier(a,b,c,d)`.
  - Output is deterministic, with keys sorted and rustfmt applied after writing.
- [ ] **Step 4:** Run `cargo xtask tokens` to write `generated.rs`. Implement `Theme`, `ThemeMode`, `Color::to_hsla` and `snap_to_device_px` (`(logical * scale).round() / scale`).
- [ ] **Step 5:** Run `cargo test -p xtask -p hacivat-ui-core && cargo xtask tokens --check`. Expected: PASS, exit 0.
- [ ] **Step 6: Commit** with message `feat(ui-core): design-token codegen and theme API`.

### Task 3: Bundled fonts and OpenType features

**Files:**
- Create: `crates/ui/hacivat-ui-core/src/fonts.rs`
- Create: `crates/ui/hacivat-ui-core/assets/fonts/*` and `OFL.txt`. Download Source Sans 3 (weights 400/500/600/700, with the `smcp`, `c2sc`, `tnum` features) from the adobe-fonts/source-sans GitHub release, and Fragment Mono Regular and Italic from the weiweihuanghuang/fragment-mono release. Record the URLs and SHA-256 in `assets/fonts/SOURCES.md`.
- Test: covered by probe `fonts` in Task 9 (needs a real platform text system).

**Interfaces:**
- Produces:
  - `pub fn register_fonts(cx: &mut App) -> anyhow::Result<()>`, called by `run_app`. It embeds the files with `include_bytes!` and passes them to `cx.text_system().add_fonts`.
  - `pub fn font_for(style: &TypeStyle) -> Font`: maps family, weight and features to a GPUI `Font` with `FontFeatures` (e.g. `[("tnum",1)]`, or `[("smcp",1),("c2sc",1)]` for `label-sc`).
  - `pub const UI_FAMILY: &str = "Source Sans 3"; pub const MONO_FAMILY: &str = "Fragment Mono";`

- [ ] **Step 1: Write the failing unit test.**
```rust
#[test] fn label_sc_requests_small_caps() { let f = font_for(&DARK.type_ramp.label_sc); assert!(f.features.contains("smcp") && f.features.contains("c2sc")); }
#[test] fn body_requests_tabular_figures() { assert!(font_for(&DARK.type_ramp.body).features.contains("tnum")); }
```
  Use the accessor GPUI's `FontFeatures` exposes at the pin. If none exists, add a small `has_feature(&FontFeatures, &str) -> bool` helper in `fonts.rs`.
- [ ] **Step 2:** Run `cargo test -p hacivat-ui-core fonts`. Expected: FAIL.
- [ ] **Step 3: Implement `register_fonts` and `font_for`.** Feature lists come from `TypeStyle.features`, which Task 2 parses from `fontFeature: "tnum"` or `"smcp, c2sc"`.
- [ ] **Step 4:** Run `cargo test -p hacivat-ui-core fonts`. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(ui-core): bundle Source Sans 3 and Fragment Mono with feature mapping`.

### Task 4: Frame and latency recorder; bench report

**Files:**
- Create: `crates/ui/hacivat-ui-core/src/frames.rs`, `xtask/src/bench_report.rs`
- Test: unit tests in both files

**Interfaces:**
- Produces:
  - `pub struct FrameRecorder`, with:
    - `new(label: &str)`
    - `on_frame(&mut self, now: Instant)`, called from a `window.on_next_frame` loop that re-arms itself while recording
    - `stats(&self) -> FrameStats`
    - `dump_json(&self, dir: &Path, os: &str) -> io::Result<PathBuf>`, which writes `target/bench/<label>-<os>-<unix_ts>.json` with the raw intervals.
  - `pub struct FrameStats { pub frames: usize, pub mean_fps: f64, pub p50_ms: f64, pub p95_ms: f64, pub p99_ms: f64, pub max_gap_ms: f64, pub gaps_over_100ms: usize }`. Intervals are the deltas between consecutive `on_frame` calls. Percentiles use nearest-rank.
  - `pub struct LatencyProbe`, with:
    - `mark_input(&mut self, at: Instant)`
    - `on_frame(&mut self, now: Instant)`: when an input is pending, it records `now - input_at` as one sample.
    - `stats() -> LatencyStats { samples, p50_ms, p95_ms, max_ms }`
  - `cargo xtask bench-report <files…> --milestone m0` appends a markdown table to `docs/benchmarks/m0.md`, one row per file: label, OS, host, build hash (`git rev-parse --short HEAD`, or `nogit`), frames, mean fps, p95, p99, max gap, gaps > 100 ms, and PASS/FAIL against the thresholds in Global Constraints.

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn stats_from_uniform_16ms() { let s = stats_of(&[16.0; 600]); assert!((s.mean_fps - 62.5).abs() < 0.01); assert_eq!(s.gaps_over_100ms, 0); }
#[test] fn detects_long_gap() { let mut v = vec![16.0; 100]; v[50] = 140.0; let s = stats_of(&v); assert_eq!(s.gaps_over_100ms, 1); assert_eq!(s.max_gap_ms, 140.0); }
#[test] fn p95_nearest_rank() { let v: Vec<f64> = (1..=100).map(|i| i as f64).collect(); assert_eq!(stats_of(&v).p95_ms, 95.0); }
#[test] fn latency_pairs_input_with_next_frame() { /* mark at t0, frames at t0+5ms and t0+20ms -> one sample of 5ms */ }
#[test] fn report_marks_b1_fail_below_55fps_on_macos() { assert_eq!(verdict("b1-fling", "macos", &FrameStats{ mean_fps: 54.9, ..ok() }), Verdict::Fail); }
#[test] fn report_marks_b1_pass_45fps_on_linux() { assert_eq!(verdict("b1-fling", "linux", &FrameStats{ mean_fps: 45.0, ..ok() }), Verdict::Pass); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-ui-core frames && cargo test -p xtask bench_report`. Expected: FAIL.
- [ ] **Step 3: Implement the recorder, the probe, `stats_of(&[f64]) -> FrameStats` and `verdict(label, os, &FrameStats | &LatencyStats) -> Verdict`.** Thresholds:
  - `b1-fling`: mean ≥ 55 (macos/windows), ≥ 45 (linux), and `gaps_over_100ms == 0`.
  - `b3-typing*`: p95 ≤ 16.
  - `b3-completion`: p95 ≤ 50.
- [ ] **Step 4:** Run the same tests. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat: frame/latency recorder and bench report`.

### Task 5: B1 grid spike (10M × 50, fling)

**Files:**
- Create: `spikes/spike-grid/src/{main.rs, source.rs, scroll.rs, element.rs, bench.rs}`
- Test: unit tests in `source.rs` and `scroll.rs`

**Interfaces:**
- Consumes: `run_app`, `open_window`, `Theme`, `font_for`, `FrameRecorder` (Tasks 1–4).
- Produces (spike-local; it is not the HV-GRID API, which M3 designs properly):
  - `pub trait GridDataSource { fn row_count(&self) -> u64; fn col_count(&self) -> usize; fn column(&self, col: usize) -> ColumnMeta; fn write_cell(&self, row: u64, col: usize, out: &mut String); }`
  - `pub struct ColumnMeta { pub name: SharedString, pub type_label: SharedString, pub width_px: f32, pub numeric: bool }`
  - `pub struct SyntheticSource { rows: u64, cols: usize }`, built with `SyntheticSource::new(10_000_000, 50)`. Cells are deterministic, and column kinds cycle int, text, numeric(14,2) with thousands formatting, timestamp, NULL every 97th row. Values come from a hash of `(row, col)`. There is no storage.
  - `pub struct ScrollPos { pub top_row: u64, pub offset_px: f32 }` with `offset_px` in `[0, row_h)`. Scroll state never uses an absolute f32 pixel offset (Review Focus 1).
  - `pub fn scroll_by(pos: ScrollPos, delta_px: f64, row_h: f32, total_rows: u64, viewport_rows: u64) -> ScrollPos`
  - `pub fn visible_rows(pos: ScrollPos, viewport_h: f32, row_h: f32, total_rows: u64) -> std::ops::Range<u64>`
  - `pub fn scrollbar_thumb(pos: ScrollPos, viewport_h: f32, row_h: f32, total_rows: u64) -> (f32 /*top*/, f32 /*height, min 24*/)`
  - The binary has `--bench fling`, which drives a synthetic fling (initial velocity 12,000 px/s, exponential decay τ = 0.325 s, re-flung every 1.5 s, for 20 s), records with `FrameRecorder("b1-fling")` and dumps JSON. Without `--bench`, it runs interactively with real trackpad momentum events.

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn scroll_to_last_row_is_exact() {
    let p = scroll_by(ScrollPos{top_row:0, offset_px:0.0}, 1e12, 21.0, 10_000_000, 40);
    assert_eq!(p.top_row, 10_000_000 - 40); assert_eq!(p.offset_px, 0.0);
    assert_eq!(visible_rows(p, 40.0*21.0, 21.0, 10_000_000).end, 10_000_000);
}
#[test] fn small_steps_accumulate_without_drift() {
    let mut p = ScrollPos{top_row: 9_000_000, offset_px: 0.0};
    for _ in 0..21_000 { p = scroll_by(p, 1.0, 21.0, 10_000_000, 40); }
    assert_eq!(p, ScrollPos{top_row: 9_001_000, offset_px: 0.0});
}
#[test] fn cannot_scroll_above_zero() { assert_eq!(scroll_by(ScrollPos{top_row:0,offset_px:5.0}, -100.0, 21.0, 100, 10), ScrollPos{top_row:0, offset_px:0.0}); }
#[test] fn visible_rows_includes_partial_last_row() { assert_eq!(visible_rows(ScrollPos{top_row:10, offset_px:7.0}, 100.0, 21.0, 1000), 10..16); }
#[test] fn thumb_has_min_height() { assert!(scrollbar_thumb(ScrollPos{top_row:0,offset_px:0.0}, 800.0, 21.0, 10_000_000).1 >= 24.0); }
#[test] fn synthetic_cells_are_deterministic() { let s = SyntheticSource::new(10_000_000, 50); let (mut a, mut b) = (String::new(), String::new()); s.write_cell(9_999_999, 49, &mut a); s.write_cell(9_999_999, 49, &mut b); assert_eq!(a, b); }
```
- [ ] **Step 2:** Run `cargo test -p spike-grid`. Expected: FAIL.
- [ ] **Step 3: Implement `scroll_by`, `visible_rows` and `scrollbar_thumb`.** Do the arithmetic in `f64`/`u64`. Clamp to `total_rows - viewport_rows`.
- [ ] **Step 4: Implement the grid element in `element.rs` as a `canvas`.** It does the following:
  - Paints only the visible rows × visible columns: row backgrounds, 1 px seams snapped with `snap_to_device_px`, a header with name and type, row numbers, NULL rendered as `label-sc` "NULL" in `text-3`, and right-aligned numerics.
  - Shapes cells with `window.text_system().shape_line`.
  - Caches shaped lines in a frame-generation LRU of 4,096 entries keyed by `(string, font, size)`.
  - Supports horizontal scroll in plain f32 (columns are bounded).
  - Takes all colours and metrics from `Theme`. The grid uses 21 px rows.
- [ ] **Step 5: Implement `bench.rs`.** It drives the fling on `on_next_frame`, records frames and writes JSON on exit.
- [ ] **Step 6:** Run `cargo test -p spike-grid && cargo run -p spike-grid --release -- --bench fling`. Expected: tests PASS, and the bench prints `FrameStats` and writes `target/bench/b1-fling-macos-*.json`.
- [ ] **Step 7:** Run `cargo xtask bench-report target/bench/b1-fling-macos-*.json --milestone m0`. Expected: a row appended with PASS or FAIL. FAIL is a valid outcome and gets recorded, not hidden.
- [ ] **Step 8: Commit** with message `spike(grid): 10M x 50 virtualized grid with row-anchored scroll and fling bench`.

### Task 6: B3 editor spike (50k lines, typing and completion latency)

**Files:**
- Create: `xtask/src/gen_sql.rs`
- Create: `spikes/spike-editor/src/{main.rs, buffer.rs, element.rs, completion.rs, bench.rs}`
- Test: unit tests in `buffer.rs`, `completion.rs` and `gen_sql.rs`

**Interfaces:**
- Consumes: Tasks 1–4.
- Produces:
  - `cargo xtask gen-sql --lines 50000 --out target/fixtures/b3-50k.sql`: a deterministic mix of the harman query shapes, CTEs, comments, string literals with Turkish text (`'İstanbul'`, `'Iğdır'`) and long `IN (…)` lists. Size ≈ 2 MB (±10 %). `--long-line 100000` appends one minified 100k-char line (Review Focus 5).
  - `pub struct Buffer { rope: ropey::Rope, version: u64 }`, with:
    - `insert(&mut self, char_idx: usize, text: &str)`
    - `delete(&mut self, range: Range<usize>)`
    - `line_count()`
    - `line_slice(line: usize) -> RopeSlice`
    - `char_to_utf16(&self, char_idx) -> usize`
    - `utf16_to_char(&self, u16_idx) -> usize`
  - `element.rs`: paints only the visible lines. Each line is shaped over a visible-column window of at most 512 chars around the viewport, never the whole line. A per-line layout cache, keyed by `(line, buffer.version_of_line)`, is invalidated only for edited lines. Keyword colouring uses a single ASCII-case-insensitive scan over SQL keywords (spike-only). Gutter line numbers.
  - `completion.rs`: `pub fn complete(prefix: &str) -> Vec<CompletionItem>` returns up to 41 items filtered from a static list of the harman popup items plus 200 synthetic columns. The popup is painted with `popover_shadow` and `radii.md`.
  - `bench.rs`, run as `--bench typing`, `--bench typing-long-line` or `--bench completion`:
    - `typing`: replays 2,000 keystrokes at 60 Hz at line 25,000, calling `LatencyProbe::mark_input` in the key handler. Recorder label `b3-typing`.
    - `typing-long-line`: the same on the 100k-char line. Label `b3-typing-long-line`.
    - `completion`: replays 300 identifier keystrokes, each opening or refiltering the popup, measured to the frame that paints the popup. Label `b3-completion`.

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn gen_sql_is_deterministic_and_sized() { let a = gen(50_000, None); assert_eq!(a, gen(50_000, None)); assert_eq!(a.lines().count(), 50_000); assert!((1_800_000..=2_200_000).contains(&a.len())); }
#[test] fn utf16_roundtrip_with_turkish_and_emoji() { let mut b = Buffer::from("İı😀x"); assert_eq!(b.char_to_utf16(3), 4); assert_eq!(b.utf16_to_char(4), 3); }
#[test] fn insert_bumps_version() { let mut b = Buffer::from("a"); let v = b.version; b.insert(1, "b"); assert!(b.version > v); }
#[test] fn completion_filters_by_prefix_case_insensitively() { assert!(complete("CUS").iter().any(|i| i.label == "customer_id")); }
#[test] fn completion_caps_at_41() { assert!(complete("").len() <= 41); }
```
- [ ] **Step 2:** Run `cargo test -p spike-editor -p xtask gen_sql`. Expected: FAIL.
- [ ] **Step 3: Implement `gen_sql`, `Buffer` and `complete`.**
- [ ] **Step 4: Implement the editor element and the bench modes.**
- [ ] **Step 5:** Run `cargo test -p spike-editor && cargo xtask gen-sql --lines 50000 --long-line 100000 --out target/fixtures/b3-50k.sql && cargo run -p spike-editor --release -- --file target/fixtures/b3-50k.sql --bench typing` (then `typing-long-line`, then `completion`). Expected: three JSON files and printed stats.
- [ ] **Step 6:** Run `cargo xtask bench-report target/bench/b3-*.json --milestone m0`. Expected: rows appended with verdicts.
- [ ] **Step 7: Commit** with message `spike(editor): 50k-line editor with typing/completion latency benches`.

### Task 7: IME on the editor spike

**Files:**
- Create: `spikes/spike-editor/src/input.rs`, `docs/benchmarks/m0-ime-checklist.md`
- Test: unit tests in `input.rs`

**Interfaces:**
- Consumes: `Buffer`, its UTF-16 conversions and the editor element (Task 6).
- Produces: `impl EntityInputHandler for EditorView`, with every method in UTF-16 offsets:
  - `text_for_range`
  - `selected_text_range`
  - `marked_text_range`
  - `unmark_text`
  - `replace_text_in_range`
  - `replace_and_mark_text_in_range`
  - `bounds_for_range`: returns the caret or marked-range rectangle from the cached line layout, so the candidate window sits under the composition.
  - `character_index_for_point`

  Editor state: `marked: Option<Range<usize>>` (char indices) and `selection: Range<usize>`. On focus loss (`on_blur`) the view calls `unmark_text`, which commits the marked text.

- [ ] **Step 1: Write the failing tests** against a pure `ImeState` struct, so the tests need no window.
```rust
#[test] fn replace_and_mark_sets_marked_range() { let mut s = ImeState::new("SELECT "); s.replace_and_mark(None, "ç", None); assert_eq!(s.marked_utf16(), Some(7..8)); }
#[test] fn unmark_after_focus_loss_commits_marked_text() { let mut s = ImeState::new("x"); s.replace_and_mark(None, "ş", None); s.unmark(); assert_eq!(s.text(), "xş"); assert_eq!(s.marked_utf16(), None); }
#[test] fn replace_in_marked_range_replaces_composition() { let mut s = ImeState::new(""); s.replace_and_mark(None, "k", None); s.replace_and_mark(None, "か", None); s.replace(None, "感"); assert_eq!(s.text(), "感"); }
#[test] fn dead_key_sequence_produces_single_char() { let mut s = ImeState::new(""); s.replace_and_mark(None, "^", None); s.replace(None, "â"); assert_eq!(s.text(), "â"); }
```
- [ ] **Step 2:** Run `cargo test -p spike-editor input`. Expected: FAIL.
- [ ] **Step 3: Implement `ImeState`, the handler methods and `bounds_for_range`.**
- [ ] **Step 4:** Run `cargo test -p spike-editor input`. Expected: PASS.
- [ ] **Step 5: Write `docs/benchmarks/m0-ime-checklist.md`.** It has one row per OS × input source, with columns for input, expected result, observed, and pass/fail:
  - Turkish Q: `ğüşıöçİ`, plus `Shift+i` → `İ` and `i` → `i`
  - Turkish F
  - US with dead keys: `^`+`a` → `â`, `` ` ``+`e` → `è`
  - Japanese Kotoeri/MS-IME/Mozc: `kanji` → candidate window under the caret, commit with Enter
  - Chinese Pinyin: `zhongwen` → `中文`

  Check: composition underline drawn, caret position, candidate window placement, and mid-composition click-away (Review Focus 4).
- [ ] **Step 6: Commit** with message `spike(editor): IME input handler and manual IME checklist`.

### Task 8: VF: harman workspace reproduction and comparison tooling

**Files:**
- Create: `spikes/spike-harman/src/{main.rs, screen/{titlebar.rs, explorer.rs, editor.rs, grid.rs, inspector.rs, statusbar.rs, data.rs}}`
- Create: `crates/ui/hacivat-ui-core/assets/icons/lucide/*.svg` (only the icons the harman mockup uses), `xtask/src/vf.rs`, `design/vf/samples.json`
- Test: unit tests in `xtask/src/vf.rs`

**Interfaces:**
- Consumes: Theme, fonts and the gpui surface (Tasks 1–3). The static data is copied from `design/directions/harman/index.html` (Result 1, the explorer tree, the completion popup and the status bar). EN and TR strings are hard-coded in `data.rs`; Fluent arrives in M1.
- Produces:
  - `spike-harman [--theme dark|light] [--lang en|tr] [--scale 1|1.5|2]`: opens a 1440×900 logical window that reproduces the harman workspace: title bar with the capsule, explorer with the connection panel, editor with gutter, execution band and completion popup, grid with the inspector and next-page strip, and status bar. On macOS the native traffic lights replace the mock's drawn ones, so the title-bar content is inset to match.
  - `design/vf/samples.json`: `{ "colors": [{ "name", "x", "y", "token", "theme" }], "scanlines": [{ "name", "axis": "x"|"y", "at", "from", "to", "expect_edges_px": [..] }] }`, in logical pixels on the 1440×900 frame. Write at least 40 colour samples covering every surface token and the `cp-fill` capsule, plus scanlines through the title-bar bottom seam, the explorer/editor seam (x = 276), the editor/results seam, grid rows 1–5 (21 px pitch), the toolbar heights (32/34) and the inspector edge.
  - `cargo xtask vf --reference <png> --candidate <png> --scale <s>`: prints one line per sample (`ok`/`FAIL` with ΔE) and per scanline (expected vs found edges, ±1 px), and exits 1 on any failure. The reference PNGs come from headless Chromium using the command in `design/directions/BRIEF.md` at `--force-device-scale-factor` 1, 1.5 and 2, with `?theme=…&lang=…&view=workspace`.
  - `pub fn delta_e2000(a: [f64; 3], b: [f64; 3]) -> f64` (Lab), `pub fn srgb_to_lab(rgb: [u8; 3]) -> [f64; 3]` (D65), and `pub fn find_edges(img: &RgbaImage, line: &Scanline, tol_de: f64) -> Vec<u32>` (positions where ΔE to the previous pixel exceeds `tol_de`).

- [ ] **Step 1: Write the failing tests.**
```rust
// Sharma, Wu, Dalal (2005) CIEDE2000 test data, pairs 1, 7, 17 and 34
#[test] fn de2000_sharma_pair_1()  { assert!((delta_e2000([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485]) - 2.0425).abs() < 1e-4); }
#[test] fn de2000_sharma_pair_7()  { assert!((delta_e2000([50.0, 0.0, 0.0], [50.0, -1.0, 2.0]) - 2.3669).abs() < 1e-4); }
#[test] fn de2000_sharma_pair_17() { assert!((delta_e2000([50.0, 2.5, 0.0], [73.0, 25.0, -18.0]) - 27.1492).abs() < 1e-4); }
#[test] fn de2000_sharma_pair_34() { assert!((delta_e2000([2.0776, 0.0795, -1.1350], [0.9033, -0.0636, -0.5514]) - 0.9082).abs() < 1e-4); }
#[test] fn srgb_white_is_l100() { let l = srgb_to_lab([255,255,255]); assert!((l[0]-100.0).abs() < 1e-3); }
#[test] fn finds_one_px_seam() { let img = synthetic_with_vertical_seam(276, 0x2A2E36, 0x1E2127); assert_eq!(find_edges(&img, &Scanline::x(450, 200, 400), 2.0), vec![276, 277]); }
```
- [ ] **Step 2:** Run `cargo test -p xtask vf`. Expected: FAIL.
- [ ] **Step 3: Implement `vf.rs`** (CIEDE2000 per Sharma et al. 2005, sRGB → XYZ D65 → Lab) and the `vf` subcommand. Run `cargo test -p xtask vf`. Expected: PASS.
- [ ] **Step 4: Build the `spike-harman` screens from tokens only.** Any hex literal outside `generated.rs` is a review failure. Draw the capsule as exactly one `cp-fill` quad and add a debug assertion that counts `cp-fill` quads per frame (== 1). Render icons with GPUI `svg()` at 12/14 px.
- [ ] **Step 5: Capture the candidate frames.** On macOS, run `screencapture -o -l <window-id> target/vf/cand-dark-en-2x.png`, using a `--print-window-id` flag that the spike adds. Capture dark/light × en/tr at 2×, plus dark-en at 1× and 1.5× (Windows 150 % on the rented host). Then run `cargo xtask vf` against each Chromium reference. Expected: per-sample output. Fix the spike until it passes, or record why it can't (an adapter limitation is an Engine-gate input).
- [ ] **Step 6:** Write `target/vf/side-by-side-*.png`: the reference and the candidate concatenated horizontally (the `image` crate in xtask, `--side-by-side` flag). This goes to the PO for sign-off (VF f).
- [ ] **Step 7: Commit** with message `spike(harman): GPUI reproduction of the harman workspace and VF tooling`.

### Task 9: Non-gating probes (X row) and font checks

**Files:**
- Create: `spikes/spike-probes/src/main.rs`
- Test: the probes are the tests. Each subcommand prints `probe <name>: ok|FAIL <detail>` and exits non-zero on FAIL.

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces these subcommands:
  - `fonts`: shapes `"0000"` and `"1111"` with `body` (`tnum`) and asserts equal widths. Shapes `"Ab"` with and without `smcp` and asserts the glyph ids differ. Asserts that Fragment Mono advance widths are equal across `"iW"`.
  - `turkish_small_caps`: shapes the TR explorer header string as Fluent will deliver it (pre-cased "GEZGİN") with `label-sc`, and asserts that the glyph for `İ` is the dotted capital. Then shapes lower-case `"gezgin"` with `smcp` and records, without asserting, whether the font's `locl` TRK applies under GPUI's shaping. Record the outcome in the addendum, because it decides whether casing must always happen upstream (Review Focus 3).
  - `fallback`: shapes `"İıŞşĞğÜüÖöÇç ✓ → ≈"` and asserts that no glyph resolves to `.notdef` (glyph id 0).
  - `bidi`: asserts that `"\u{2068}Ada\u{2069}"` and `"Ada"` have equal widths (D-075).
  - `a11y`: exposes one custom element with an AccessKit node (role `Grid`, label "Result 1"), following the gpui `examples/a11y.rs` pattern at the pin. Prints the node tree. Verify manually with VoiceOver, Narrator and Orca.
  - `clipboard`: writes `ClipboardItem` text with metadata, reads it back, and compares (all OSes). On Linux, it also writes the primary selection.
  - `startup`: prints the ms from `main()` to the first `on_next_frame`, and RSS after 5 s idle (`proc_pid_rusage` on macOS, `GetProcessMemoryInfo` on Windows, `/proc/self/statm` on Linux).

- [ ] **Step 1: Implement each subcommand.** Expected output is `ok` for fonts, fallback and bidi. The others record values.
- [ ] **Step 2:** Run `cargo run -p spike-probes --release -- all`. Expected: the fonts, fallback and bidi lines print `ok`. Record the other values for the addendum.
- [ ] **Step 3: Commit** with message `spike(probes): font, Turkish casing, fallback, bidi, a11y, clipboard, startup probes`.

### Task 10: PKG: signed bundles and an update round trip

**Files:**
- Create: `packaging/Packager.toml`, `packaging/update-manifest.template.json`, `packaging/keys/README.md` (key generation; the private key never enters the repo)
- Modify: `spikes/spike-harman/src/main.rs`: add `--check-update <manifest-url>` using `cargo-packager-updater` with the minisign public key `include_str!("../../../packaging/keys/update.pub")`. The window title shows the version.
- Test: the round trip below, run manually per OS.

**Interfaces:**
- Produces:
  - Bundles of `spike-harman` as product "Hacivat Spike", version `0.0.1` and `0.0.2`:
    - macOS: universal `.app` in a `.dmg`, Developer ID signed, hardened runtime, notarized with `xcrun notarytool submit --wait`, stapled.
    - Windows: unsigned NSIS per-user installer.
    - Linux: AppImage, `.deb` and `.rpm`, each with a `.minisig`.
  - An update manifest per platform, signed with minisign.

- [ ] **Step 1:** Generate the update keypair with `minisign -G -p packaging/keys/update.pub -s ~/.hacivat-keys/update.key`. Commit only the `.pub`.
- [ ] **Step 2:** Build and package `0.0.1` and `0.0.2` with `cargo packager --release -c packaging/Packager.toml`. The macOS signing identity, Apple ID and app password come from env vars (`APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`); ask the PO for them, never write them to disk in the repo.
- [ ] **Step 3:** Run `spctl -a -vvv -t install "Hacivat Spike.dmg"` and `xcrun stapler validate`. Expected: `accepted`, `source=Notarized Developer ID`, and `The validate action worked!`.
- [ ] **Step 4:** Serve `0.0.2` and its manifest with `python3 -m http.server` from a temp dir. Launch the installed `0.0.1` with `--check-update http://127.0.0.1:8000/manifest.json`. Expected: it downloads, verifies the minisign signature, installs and relaunches with the title showing `0.0.2`. Repeat with a tampered artifact. Expected: the update is refused with a signature error.
- [ ] **Step 5:** Repeat Steps 2–4 on the Windows and Linux hosts (unsigned on Windows; the Linux AppImage self-update). Record the results.
- [ ] **Step 6: Commit** with message `spike(pkg): signed packaging and minisign-verified update round trip`.

### Task 11: CI workflow (regression signal)

**Files:**
- Create: `.github/workflows/m0.yml`

**Interfaces:**
- Produces jobs on `macos-14`, `windows-2022` and `ubuntu-24.04`, using the pinned toolchain:
  - `cargo build --workspace --release`
  - `cargo test --workspace`
  - `cargo xtask tokens --check`
  - `cargo xtask check-deps`
  - `cargo deny check licenses bans`
  - Nightly: `spike-grid --bench fling` and the `spike-editor` benches under a virtual display on Linux (`xvfb-run`). The JSON is uploaded as artifacts and labelled "regression signal only, not authoritative".

- [ ] **Step 1: Write the workflow.** The PO must create the GitHub repository first. Until then, validate the file with `actionlint` locally. Expected: no findings.
- [ ] **Step 2: Commit** with message `ci: m0 build, tests, token staleness, dependency and license guards`.

### Task 12: Gate run, results and ADR addendum

**Files:**
- Create or complete: `docs/benchmarks/m0.md`, `docs/decisions/0001-addendum-m0.md`

- [ ] **Step 1: Run the authoritative benches** (B1, B3 typing, B3 long line, B3 completion), each three times on the PO's Mac with the release build, on AC power, with other apps closed. Repeat on the Windows 11 and Ubuntu 24.04 hosts. Append everything with `cargo xtask bench-report`.
- [ ] **Step 2:** Complete the IME checklist on all three OSes with a human at the keyboard (the PO on macOS).
- [ ] **Step 3: Run VF on every capture.** Put the side-by-side images in front of the PO, and record the sign-off (VF f) or the fix list.
- [ ] **Step 4: Write `docs/decisions/0001-addendum-m0.md`** with:
  - the pinned commit `912159c7ad77ce6296a93be77efc298b0b00fd50`
  - the machine specs
  - a results table (B1, B3, IME, VF a–f, PKG), each PASS or FAIL against the Global Constraints numbers
  - the X-row values from Task 9, including the Turkish `smcp`/`locl` finding
  - the copied `[patch.crates-io]` list
  - the decision: **GO** (proceed to M1 on GPUI), **GO with fix list** (a PKG or VF fix list that doesn't fail VF a–d), or **NO-GO** (any B1/B3/IME/VF a–d failure; re-evaluate with the Linebender fallback per ADR 0001)
- [ ] **Step 5: Present the addendum to the PO** for the go/no-go decision. Do not start M1 without it.
- [ ] **Step 6: Commit** with message `docs: M0 benchmark results and ADR 0001 addendum`.

---

## Self-review notes

- **Spec coverage:**
  - M0 rows B1 → Task 5; B3 → Task 6; IME → Task 7; PKG → Task 10; VF a–f → Task 8 + Task 12; X → Task 9.
  - Surviving pieces: adapter → Tasks 1–3; token codegen → Task 2.
  - §5.2's One Filled Badge and 11 px rules → Tasks 2 and 8.
  - §3.2 reference machines → Global Constraints + Task 12.
  - CI regression signal → Task 11.
- **Not in M0 by design:** Fluent and l10n (M1), the real HV-GRID and UI-EDIT APIs (M3), and Windows Authenticode (before 1.0).
