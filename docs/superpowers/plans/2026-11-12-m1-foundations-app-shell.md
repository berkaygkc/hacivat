# M1: Foundations and App Shell Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the M0 adapter into the foundations of the real app: a launchable `hacivat` binary on all three OSes whose shell (title bar with capsule, tiled layout, tabs and splits, status bar, palette, toasts, dialogs, settings) looks like harman, takes every visible string from Fluent, and switches language and theme at runtime. CI, nightly packaging and the M0 carry-overs land with it.

**Architecture:** The workspace grows from `crates/ui/hacivat-ui-core` and `xtask` into the canonical §5.1 layout:
- `crates/ui`: `hacivat-ui-core` (the only GPUI importer), `hacivat-bench` (gpui-free frame stats), `hacivat-ui` (widgets, keymap data, UI text provider).
- `crates/core`: `hacivat-l10n` and `hacivat-l10n-schema` (Fluent + ICU4X), `hacivat-settings` (settings store and §5.6 paths).
- `crates/app/hacivat-app` (workspace shell, commands, settings UI) and `crates/bin/hacivat` (the executable).

Core never sees UI types. UI widgets receive rendered strings or resolve `hui-*` IDs through a provider trait. The app joins the two. Language and theme are GPUI globals swapped at runtime, followed by `refresh_windows`.

**Tech Stack:**
- Rust 1.98.1, edition 2024
- `gpui`/`gpui_platform` at zed `912159c` (moving to a fork at the same base in Task 15)
- `fluent-bundle` 0.16, `fluent-syntax` 0.12, `intl-memoizer` 0.5.3, `unic-langid` 0.9
- ICU4X `~2.4` components (`icu_locale`, `icu_decimal`, `icu_datetime`, `icu_time`, `icu_list`, `icu_plurals`, `icu_casemap`, `icu_normalizer`, `icu_properties`, `icu_segmenter`) and `fixed_decimal`
- `arc-swap` 1, `rust-embed` 8, `sys-locale`, `directories`, `serde`, `toml`, `toml_edit`, `serde_json`, `unicode-segmentation`
- `tracing` + `tracing-subscriber` (json) + `tracing-appender`
- `trybuild` (dev), `cargo-packager`, `minisign`, `cargo-deny`, `actionlint`

**Spec:** `docs/superpowers/specs/2026-11-12-hacivat-v1-design.md`. Read §3.1–3.4, §4 rows i18n/Settings/Keyboard/Accessibility, §5.1–5.4, §5.6, §6 M1, §7, the decision log (L1, L2, L11, L12) and §10 C8, C15, C17, C18.

Also read:
- `docs/decisions/0001-addendum-m0.md` ("GPUI capability findings", "Open items")
- `docs/benchmarks/m0-rulings.md` (deferred minors)
- `docs/benchmarks/m0-ime-checklist.md` ("Known gaps")
- `docs/research/algorithms/26-i18n.md` (I18N-01..07, -09, -16, -17, -23)
- `docs/research/algorithms/25-sql-editor-component.md` (UI-EDIT-13 only)
- `DESIGN.md`, `.impeccable/design.json`, and `design/directions/harman/index.html` (the shell reference)

## Global Constraints

- **GPUI pin:** `912159c7ad77ce6296a93be77efc298b0b00fd50` (gpui 0.2.2, gpui_platform 0.1.0). After Task 15 the source is the fork `https://github.com/berkaygkc/zed` at the commit that is `912159c` plus the carried IME patch; nothing else differs. Never depend on Zed's `editor`, `ui`, `workspace`, `rope` or `text`, and never read them.
- **Toolchain:** 1.98.1, edition 2024. Every crate is `license = "Apache-2.0"`, `publish = false`, `[lints] workspace = true`.
- **Licences:** cargo-deny allow-list as in `deny.toml`. Fonts are OFL-1.1 and icons Lucide v0.460.0 (ISC), with texts bundled next to the assets.
- **Layering (§5.1, enforced by `xtask check-deps`):**
  - Only `hacivat-ui-core` may reach `gpui`, `gpui_platform` or any zed-sourced package, directly or transitively.
  - `crates/ui/*` never depends on `crates/core|sql|ai`.
  - `crates/core|sql|ai` never depend on `crates/ui/*`.
  - `hacivat-app` and `hacivat` may depend on both.
- **No `unsafe`** except in `hacivat-ui-core/src/platform/**` (workspace lint `unsafe_code = "deny"`). Every unsafe block carries a `// SAFETY:` comment.
- **Casing (INV-3, L11):** `str::to_lowercase`, `str::to_uppercase`, `char::to_lowercase` and `char::to_uppercase` are `disallowed-methods` workspace-wide (`clippy.toml`). Human-text casing goes through `hacivat_l10n::case_for_display`. Small-caps text arrives pre-cased from Fluent (`GEZGİN`, not `to_uppercase("gezgin")`).
- **Text (INV-1, I18N-23):**
  - Machine text (action IDs, settings keys, log lines, file names) is locale-free English.
  - Every visible chrome string comes from Fluent. The only exceptions are user/catalog data (`TextSource::Data`/`Catalog`) and generated shortcut labels (`TextSource::Shortcut`).
  - No string concatenation of messages, and no suffix on a placeable in Turkish.
- **Fluent IDs:** grammar `^(cm|hui|hv|hc|kg|ps|pw|pe|db|error)-[a-z0-9]+(-[a-z0-9]+)*$`, with prefix ⇔ file: `cm-` → `common.ftl`, `hui-` → `hui.ftl`, `hv-` → `hacivat-ui.ftl`. Every English message with variables carries a `# Variables:` block.
- **Tokens:** components use semantic token names only; a hex literal outside `theme/generated.rs` fails review.
- **Named rules:**
  - Text is never below 11 px nominal.
  - At most one `cp-fill` quad per window frame, and exactly one when the capsule shows production.
  - Radii come from {0, 3, 4, 6, 9999} px.
  - Only floating surfaces cast the popover shadow: popover, menu, palette, toast, dialog.
  - Focus is a 1 px blue outline at a 1 px offset.
  - No ambient motion: tab, popover and selection changes are instant.
- **Shell metrics (DESIGN.md):**
  - Title bar 40 px (padding `0 10px 0 14px`); status bar 24 px; panel header 30 px.
  - Explorer 276 px over a 205 px connection panel; editor area 322 px over results; inspector 252 px.
  - Tabs 30 px; capsule 26 px; palette field 26 px; buttons 24 px; segmented controls 26 px.
  - macOS traffic lights at (14, 13).
- **Threading:** the UI thread never awaits I/O. Catalog building for a language switch runs on the background executor.
- **Budgets:**
  - First frame ≤ 500 ms on the macOS reference; idle RSS ≤ 200 MB.
  - Cold parse of the en+tr desktop catalogs ≤ 10 ms; language switch-to-repaint ≤ 100 ms.
  - Simple message render ≤ 3 µs p50.
- **Local state (§5.6):** paths come from `directories`. Directories are `0700` and files `0600` on Unix. Settings live in `<config>/settings.toml`, which is TOML with `version = 1`.
- **Packaging (L2):**
  - macOS is Developer ID signed and notarized (`NOTARY_PROFILE`, verified at `82dadcc` with `mystat-notary`).
  - Windows is unsigned.
  - Linux is minisign-signed.
  - Nightlies sign only with the **passworded** update key held in CI secrets.
- **Execution (L1):**
  - Feature branch `m1-foundations` off `main`. Each task runs in its own git worktree on branch `m1/<task-slug>`, created from `m1-foundations`.
  - Each agent starts with `git merge --ff-only m1-foundations`.
  - All worktrees share `CARGO_TARGET_DIR=<main checkout>/target`.
  - Before the final verification run, `cargo clean -p <crates the task touched>` and `cargo clean --release -p <same>`.
  - The controller merges into `m1-foundations` after each task's review.
  - FTL files are append-only. Each task appends a block headed `## <task slug>`, and the controller resolves append-only conflicts by keeping both blocks.
  - Resolve a `Cargo.lock` conflict by taking the branch tip and running `cargo check --workspace`.

## Review Focus

1. **Turkish and AltGr keyboards.**
   - On TR-Q, Ctrl plus the key labelled I produces `ı`, and a `ctrl-i` binding must still fire.
   - On Windows, AltGr (Ctrl+Alt) chords such as Ctrl+Alt+Q (`@`) must type into the focused input and never fire a command.
   - Tests: `tr_q_dotless_i_matches_ctrl_i` and `altgr_char_passes_through` (Task 5); `altgr_at_sign_reaches_palette_input` (Task 16).
2. **Language switch mid-interaction.** With the palette or a dialog open and text typed, or a toast showing, switching language keeps the typed text and re-renders every label and the toast in the new language, without a restart.
   - Tests: `switch_language_with_palette_open_keeps_query` (Task 16); `toast_rerenders_after_language_switch` (Task 13).
3. **Hand-edited or foreign state files.** `settings.toml` and `workspace.json` may arrive with typos, wrong types, unknown keys, keys from a newer version, unparsable content, or a newer format version. The app starts with defaults for bad values, keeps unknown keys and comments, and never silently destroys the user's file.
   - Tests in Task 7.
4. **Narrow window in a long locale.** At 1024 px wide in `tr-XT`, the title-bar segments collapse into an overflow menu before the palette label truncates (the Overflow Rule), and no label is clipped.
   - Tests: `titlebar_collapses_segments_before_truncating_palette` (Task 12); shots surface `shell-narrow` (Task 18).
5. **Stacked layers and focus.** With the palette open over the settings tab, Esc closes only the palette and focus returns to the settings search field. A dialog traps focus.
   - Tests: `escape_closes_only_topmost_layer_and_restores_focus` (Task 16); `dialog_traps_focus_and_escape_cancels` (Task 12).

## Rulings (controller calls; the PO can override each)

- **R1. IME patch carriage: a fork of zed pinned at `912159c` plus the patch** (Task 15).
  - **Why not vendoring:** vendoring `gpui_macos` means de-workspacing about 40 inherited dependencies in a 10k-line crate, and it reaches only `gpui_macos`. The second gap (blur for inactive windows) needs a change in `gpui` itself.
  - **Why not a `[patch]` override of just `gpui_macos`:** it would pull a second `gpui` from the fork and break type identity. Patching every zed crate amounts to switching the source, with more noise.
  - **What the fork is:** one commit on top of `912159c`, Apache-2.0, with `LICENSE-APACHE` kept and a "modified" header in `window.rs`.
  - **Scheduling:** the inactive-window blur change is scheduled for M5 (editor IME, UI-EDIT-12) on the same fork. If the PO has not created the fork by Wave 3, Task 15 is deferred to M2 and recorded, never worked around.
  - **Cost if wrong:** reverting is one `Cargo.toml` line.
- **R2. `hacivat-settings` is added as a core crate.** It holds the settings schema and store, the §5.6 paths and the workspace-state file. §5.1 has no home for these, and `alg/26` I18N-06 already names `hacivat-settings`.
- **R3. `hacivat-bench` is added under `crates/ui/` as a gpui-free crate.** It is the new home of `frames.rs`. `hacivat-ui-core` re-exports it behind the `bench` feature.
- **R4. `L10nGlobal` lives in `hacivat-app`, not `hacivat-ui`.** `alg/26` I18N-07 places it in `hacivat-ui`, but that would break §5.1 rule 1. `hacivat-ui` instead owns `UiText(Arc<dyn UiTextProvider>)`.
- **R5. Command titles use `hv-cmd-<action id with . and _ as ->` IDs.** UI-EDIT-13's `cmd-*` prefix fails the I18N-01 ID grammar.
- **R6. The casing ban is workspace-wide, not limited to the `sql-*`/driver/introspect/MCP crates.** Those crates do not exist yet, and L11 bans casing in renderers too. ASCII methods stay allowed. An exception needs `#[expect(clippy::disallowed_methods, reason = "...")]` and review.
- **R7. Own keystroke dispatcher.** It sits in the workspace root's `capture_key_down` and implements UI-EDIT-13 normalization. GPUI's keymap is used only to show macOS menu key equivalents: menu commands are bound under a context that no element sets, and AppKit dispatches those chords through the menu, so there is still a single handler.
- **R8. Screenshots come from GPUI's `HeadlessAppContext` with `gpui_platform::current_headless_renderer`.** This covers the pseudo-screens job and the VF capture of the shell, on macOS (Metal) and Linux (wgpu). Windows has no headless renderer at the pin, so Windows screens come from `design/vf/capture.ps1` at the gate. If headless capture fails the VF reference self-check, fall back to real-window capture (`design/vf/capture.sh app`).
- **R9. M1 pulls in the I18N-09 search key** (Contains / WordPrefix / Fuzzy with highlight ranges). The palette (§10 C18) and settings search need it, although §6 M1 lists only I18N-01..07, -16, -17 and -23. Per D-074 (the register wins over I18N-09's notes), Turkish-tolerant folding is on only when the UI language or regional format is Turkish. The toggle UI arrives with the explorer filter in M4.
- **R10. `l10n-check` in M1 implements a subset of the rules.**
  - Errors: L001–L008, L011, L013, L015, L017, L018, L024, L027, L029.
  - Warnings: L009, L012, L021, L023, L025.
  - Deferred: L010, L014, L016, L019, L020 and L022 to M9 string freeze; L026 to M2 (error registries); L028 to M9; L030 to M8 (MCP crates).
- **R11. Text rendering applies on restart only.** Live switching is not feasible at the pin:
  - `gpui_apple/src/text_system.rs:240-260` caches `AppleFontSmoothing` in a process-wide `OnceLock` on the first glyph rasterized.
  - Rasterized glyphs stay in the atlas with the dilation baked in.
  - L12 forbids changing GPUI for this.
  
  Settings shows "Applies after restart" and a "Restart now" button (`cx.restart()`).
- **R12. The One Filled Badge assertion is paint-level.** It counts `Scene::quads` whose `background.as_solid()` equals `cp_fill`, read through `Window::rendered_scene()` (gpui `bench-support`) behind the ui-core feature `scene-check`. Tests and the shots harness enable it; release builds do not.
- **R13. Shell contents in M1.**
  - The capsule shows "No connection". A `--vf-demo` flag shows the harman production capsule (data `orders-prod`, `PostgreSQL 16`).
  - The explorer and connection panel show Fluent empty states.
  - The editor area holds console tabs (`Console 1`) with an empty state; the results pane shows an empty state.
  - Settings opens as a tab.
  - Windows and Linux get an in-window app menu button after the brand; macOS uses `set_menus`.

## Waves

Interface-first: Task 1 commits every cross-task type and signature in this plan (bodies `unimplemented!()` where a later task owns the logic), the crate skeletons with their dependency lists, the FTL file skeletons and the `pub mod` lines. After Task 1, each wave's tasks own disjoint files.

| Wave | Runs | Tasks (parallel agents) | Unblocked by |
|---|---|---|---|
| W0 | serial | Task 1 | — |
| W1 | 5 agents | A: Task 2 then Task 3 · B: Task 4 · C: Task 5 · D: Task 6 · E: Task 7 | Task 1 merged |
| W2 | 5 agents | Task 8 · Task 9 · Task 10 · Task 11 · Task 12 | W1 merged (Task 9 needs 6; Task 10 needs 6; Tasks 11/12 need 4) |
| W3 | 3 agents | Task 13 · Task 14 · Task 15 | W2 merged |
| W4 | 3 agents | Task 16 · Task 17 · Task 18 (merges last) | Task 13 merged |
| W5 | serial | Task 19 (gate) | everything |

File ownership:
- W1:
  - Task 2: `xtask/src/check_deps.rs`
  - Task 3: `crates/ui/hacivat-bench/**`, `xtask/src/bench_report.rs`
  - Task 4: `crates/ui/hacivat-ui-core/**` except `src/keymap/**`, `src/window.rs`, `src/named_rules.rs`; plus `spikes/*/Cargo.toml` and the spike `use` lines
  - Task 5: `crates/ui/hacivat-ui-core/src/keymap/**`, `crates/ui/hacivat-ui/keymaps/**`, `crates/ui/hacivat-ui/src/keymaps.rs`, `crates/ui/hacivat-ui/tests/keymap_collisions.rs`
  - Task 6: `crates/core/hacivat-l10n/**` except `src/{format,pseudo,search,effective,casing}.rs`; plus `crates/core/hacivat-l10n-schema/**` and `locales/**/common.ftl`
  - Task 7: `crates/core/hacivat-settings/**`
- W2:
  - Task 8: `hacivat-ui-core/src/{window,named_rules}.rs`
  - Task 9: the five l10n modules Task 6 does not own
  - Task 10: `xtask/src/l10n/**`, `locales/_schema/literal-allowlist.toml`
  - Task 11: `hacivat-ui/src/{l10n,text,input,palette}/**`
  - Task 12: `hacivat-ui/src/{button,segmented,icon,popover,panes,dialog,toast,menu,accesskey,statusbar,capsule,titlebar}.rs`
- W3:
  - Task 13: `crates/app/hacivat-app/src/{lib,shell/**,l10n_global,theme_pref}.rs`, `crates/bin/hacivat/**`
  - Task 14: `.github/**`, `packaging/**`, `xtask/src/update_key.rs`
  - Task 15: root `Cargo.toml` (gpui source lines), `deny.toml`, `Cargo.lock`, `crates/ui/hacivat-ui-core/patches/README.md`
- W4:
  - Task 16: `hacivat-app/src/commands/**`, `hacivat-app/src/menus.rs`
  - Task 17: `hacivat-app/src/settings_ui/**`
  - Task 18: `hacivat-app/tests/shots.rs`, `design/vf/**`, `design/baselines/**`, `xtask/src/shot_diff.rs`

`xtask/src/main.rs` gets one dispatch arm per task (Tasks 10, 14, 18), which is an append-only merge.

---

## File Structure

```
hacivat/
  Cargo.toml                         # members += crates/core/*, crates/app/*, crates/bin/*; [workspace.lints]; new workspace deps
  clippy.toml                        # disallowed-methods (casing ban, R6)
  deny.toml                          # + fork URL in allow-git (Task 15)
  locales/
    en/{locale.toml, common.ftl, hui.ftl, hacivat-ui.ftl, hacivat-core.ftl}   # reference catalog
    tr/{locale.toml, common.ftl, hui.ftl, hacivat-ui.ftl}                      # stub: harman-approved strings + drafts for PO review
    _schema/{binaries.toml, dynamic-ids.toml, literal-allowlist.toml}
  crates/ui/hacivat-ui-core/src/
    app.rs            # Boot, run_app (the single bootstrap), open_window
    assets.rs         # BundledAssets (was shell.rs), icons
    fonts.rs          # + verify_bundled_fonts (was shell.rs)
    platform/{mod.rs, text_rendering.rs}   # AppleFontSmoothing (only unsafe module)
    window.rs         # open_workspace_window, chrome_kind
    named_rules.rs    # [scene-check] cp-fill quad count
    keymap/{mod.rs, parse.rs, normalize.rs, resolve.rs, altgr.rs, display.rs}
    spike_support.rs  # [spike-support] display_sizes, native_window_id, open_unified_window
    frame_loop.rs     # [bench] + `pub use hacivat_bench as frames`
  crates/ui/hacivat-bench/src/lib.rs          # frames.rs moved here (gpui-free)
  crates/ui/hacivat-ui/
    keymaps/{default-macos,default-windows,default-linux}.toml, presets/{hacivat,datagrip}.toml,
            layouts/{tr-q,tr-f,de,fr,pl}.toml, tests/altgr/{tr-q,tr-f,de,fr,pl}.toml
    src/{l10n.rs, text.rs, input/{mod.rs, ime.rs}, palette.rs, keymaps.rs, actions.rs, button.rs, segmented.rs, icon.rs,
         popover.rs, panes.rs, dialog.rs, toast.rs, menu.rs, accesskey.rs, statusbar.rs, capsule.rs, titlebar.rs}
  crates/core/hacivat-l10n-schema/src/lib.rs  # FTL signature extraction (build.rs + xtask share it)
  crates/core/hacivat-l10n/{build.rs, src/{lib, catalog, negotiate, runtime, render, args, format, pseudo, search, effective, casing}.rs}
  crates/core/hacivat-settings/src/{lib, schema, store, paths, workspace_state, fsio}.rs
  crates/app/hacivat-app/src/{lib, l10n_global, theme_pref, menus}.rs, shell/{mod, workspace, tiles, tabs}.rs,
                             commands/{mod, registry, dispatch, palette}.rs, settings_ui/{mod, rows, search}.rs
  crates/app/hacivat-app/tests/shots.rs
  crates/bin/hacivat/src/{main.rs, logging.rs, smoke.rs}
  xtask/src/{check_deps.rs, bench_report.rs, l10n/{mod, rules, code_scan}.rs, update_key.rs, shot_diff.rs}
  .github/workflows/ci.yml           # was m0.yml
  packaging/{Packager.toml, common.sh, package-*.sh|ps1, keys/{update.pub, README.md}}
  design/vf/{gen_samples.py (+ --set m1-shell), samples-m1-shell.json, capture.sh (+ app kind)}
  docs/benchmarks/m1.md              # gate record (Task 19)
```

---

### Task 1: Interface commit (workspace, guards, crate skeletons, catalog skeleton)

**Files:**
- Modify: `Cargo.toml`, `crates/ui/hacivat-ui-core/{Cargo.toml, src/lib.rs, src/gpui_surface.rs}`, `xtask/Cargo.toml`, `xtask/src/bench_report.rs` (swap the `#[path]` include for the crate), `spikes/*/Cargo.toml` (`[lints] workspace = true`)
- Create: `clippy.toml`, `crates/ui/hacivat-bench/` (`git mv crates/ui/hacivat-ui-core/src/frames.rs crates/ui/hacivat-bench/src/lib.rs`), skeletons of every crate in File Structure, and `locales/**`
- Test: `xtask/src/check_deps.rs` (one new test), `crates/core/hacivat-settings/src/schema.rs`

**Interfaces (Produces; every later task consumes these names):**
- `hacivat-ui-core`:
  - `pub mod keymap` holding types only:
    - `Mods { ctrl, alt, shift, cmd: bool }`
    - `NamedKey { Enter, Escape, Tab, Backspace, Delete, Insert, Space, Up, Down, Left, Right, Home, End, PageUp, PageDown, F(u8) }`
    - `NormKey { Named(NamedKey), Char(char) }`
    - `KeyPress { mods: Mods, key: NormKey }`
    - `Binding { chord: Vec<KeyPress>, command: String, context: String, when_layout: Option<LayoutId>, during_composition: bool, source: BindingSource }`
    - `BindingSource { Default, Preset, Layout, User }`
    - `KeymapPlatform { MacOs, Windows, Linux }`
    - `LayoutId(pub String)`; the values used are `"us" | "tr-q" | "tr-f" | "de" | "fr" | "pl"`
    - `ContextStack(pub Vec<&'static str>)`, innermost last
    - `Resolution { Command(String), Pending, PassThrough }`
    - `KeymapDiag`
    - `Keymap`
  - `pub mod window` and `pub mod named_rules` declared with signature stubs (see Task 8 and the bullet on `ChromeKind` below).
  - Features in `Cargo.toml`:
    - `bench`
    - `spike-support`
    - `scene-check = ["gpui/bench-support"]`
    - `test-support = ["gpui/test-support", "gpui_platform/test-support"]`
  - `gpui_surface` gains: `Global, Action, KeyBinding, Menu, MenuItem, OsAction, SystemMenuType, WindowControlArea, WindowDecorations, Decorations, WindowAppearance, AnyWindowHandle, Subscription, Task, MouseButton, MouseDownEvent, ClickEvent, Axis, Keystroke, KeyDownEvent, Overflow, AnyView, FocusOutEvent, TestAppContext, VisualTestContext, HeadlessAppContext` (the last three under `test-support`).
- `hacivat-ui`, with `lib.rs` declaring every module in File Structure:
  - `l10n`:
    - `pub trait UiTextProvider: Send + Sync + 'static { fn text(&self, id: &str, args: &[(&'static str, UiArg)]) -> SharedString; fn attr(&self, id: &str, attr: &str) -> Option<SharedString>; fn generation(&self) -> u64; }`
    - `pub enum UiArg { Count(u64), Text(SharedString) }`
    - `pub struct UiText(pub Arc<dyn UiTextProvider>)`, which is a `Global`
    - `pub fn ui_text(cx: &App, id: &str) -> SharedString`
    - `pub struct EnglishDefaults`
  - `text`:
    - `pub enum TextSource { Fluent, Catalog, Data, Shortcut }`
    - `pub struct Label`, built with `Label::new(text: impl Into<SharedString>, source: TextSource, style: TypeStyle, color: Hsla)`, `.truncate(bool)` and `.id(impl Into<ElementId>)`, and implementing `IntoElement`
  - `actions`:
    - `pub struct ActionDef { pub id: &'static str, pub title_id: &'static str /* hv-cmd-* (R5) */, pub context: &'static str, pub run: Rc<dyn Fn(&mut Window, &mut App)>, pub enabled: fn(&App) -> bool }`
    - `pub struct ActionRegistry`, a `Global`, with `register(&mut self, def: ActionDef) -> Result<(), DuplicateAction>`, `get(&self, id: &str) -> Option<&ActionDef>`, `all(&self) -> &[ActionDef]` and `run(id: &str, window: &mut Window, cx: &mut App) -> bool`
- `hacivat-l10n` (signatures only):
  - `MsgRef<A> { pub id: &'static str, .. }` with `const fn new(id)`
  - `trait IntoFluentArgs { fn into_args(self) -> Vec<(&'static str, ArgValue)>; }`
  - `ArgValue { Count(u64), Text(String), Decimal(ExactDecimal), When(WhenValue), List(Vec<String>) }`, plus `NoArgs`, `Count(pub u64)`, `Text(pub String)`
  - `L10n`: `new(Runtime)`, `t`, `t_attr`, `t_machine`, `t_dynamic(&self, id: &str, args: Vec<(&'static str, ArgValue)>) -> String`, `t_english(&self, id: &str) -> String` (the `en` bundle only, for the English search layer), `snapshot() -> Arc<Runtime>`, `install_if_newer(&self, rt: Runtime) -> bool`, `generation() -> u64`
  - `Runtime { pub generation: u64, pub ui_chain: Vec<LanguageIdentifier>, pub format: FormatCtx, pub diags: Vec<LoadDiag>, .. }`
  - `build_runtime(cat: &Catalogs, neg: &Negotiated, pseudo: Option<PseudoMode>, generation: u64) -> Runtime`
  - `Catalogs`, `CatalogLocale`, `LoadDiag`, `negotiate`, `NegotiationCtx`, `PrefSource`, `Negotiated` (bodies in Task 6)
  - `FormatCtx`, `install_functions`, `PseudoMode`, `transform_literal`, `wrap_message`, `SearchOpts`, `MatchMode`, `SearchKey`, `Match`, `MatchKind`, `build_key`, `find`, `turkish_tolerant_for`, `effective`, `OsLocale`, `EffectiveLocale`, `case_for_display` (bodies in Task 9)
- `hacivat-settings` (complete data types; logic in Task 7):
  - `Settings { version: u32, ui: UiSettings, keymap: KeymapSettings }`
  - `UiSettings { language: LanguagePref, regional_format: LanguagePref, time_zone: TimeZonePref, theme: ThemePref, text_rendering: TextRenderingPref }`
  - `LanguagePref { System, Tag(String) }`, `TimeZonePref { System, Iana(String) }`, `ThemePref { Dark, Light, System }`, `TextRenderingPref { Crisp, Standard }`
  - `KeymapSettings { preset: KeymapPreset }`, `KeymapPreset { Hacivat, Datagrip }`
  - TOML keys: `version`, `ui.language`, `ui.regional_format`, `ui.time_zone`, `ui.theme`, `ui.text_rendering`, `keymap.preset`. Values: `"system"`, `"dark"`, `"light"`, `"crisp"`, `"standard"`, `"hacivat"`, `"datagrip"`, or a BCP-47 tag or IANA zone.
- `hacivat-app`: module stubs `shell`, `commands`, `settings_ui`, `l10n_global`, `theme_pref`, `menus`, and `pub struct PendingChord(pub Option<SharedString>)`, which is a `Global`. Features: `scene-check = ["hacivat-ui-core/scene-check"]`, `pseudo = ["hacivat-l10n/pseudo"]`, `shots = ["scene-check", "pseudo", "hacivat-ui-core/test-support"]`.
- Feature `pseudo` on `hacivat-l10n`. Feature `scene-check = ["hacivat-ui-core/scene-check"]` on `hacivat-ui`. `hacivat-l10n` depends on `hacivat-settings` (core → core).
- `hacivat-ui-core::window`: `ChromeKind { MacTrafficLights, WindowsCaption, LinuxClient, LinuxServer }` and `TRAFFIC_LIGHTS_INSET_PX` (value copied from `spikes/spike-harman/src/screen/titlebar.rs`) are complete in this commit, because Task 12 runs in the same wave as Task 8.
- `locales/_schema/dynamic-ids.toml` lists the families `hui-*` and `hv-cmd-*`.
- `hacivat` (bin): `main` calls `run_app` and opens an empty window, so CI and packaging can target the crate from W1 on.
- `locales/en/locale.toml`: `tag = "en"`, `native_name = "English"`, `english_name = "English"`, `short_name = "EN"`, `direction = "ltr"`, `status = "stable"`. `tr` uses `"Türkçe"`, `"Turkish"`, `"TR"`, `status = "beta"`.
- `locales/en/common.ftl`: `-brand-hacivat = Hacivat` and `-brand-hacivat-caps = HACIVAT` (the brand is never Turkish-cased).

- [ ] **Step 1: Write the failing tests.**
```rust
// xtask/src/check_deps.rs
#[test] fn clippy_toml_bans_locale_blind_casing() {
    let t = std::fs::read_to_string(workspace_root().join("clippy.toml")).unwrap();
    for m in ["str::to_lowercase", "str::to_uppercase", "char::to_lowercase", "char::to_uppercase"] { assert!(t.contains(m), "{m}"); }
}
// hacivat-settings/src/schema.rs
#[test] fn defaults_match_spec() {
    let s = Settings::default();
    assert_eq!((s.version, s.ui.theme, s.ui.text_rendering, s.keymap.preset), (1, ThemePref::Dark, TextRenderingPref::Crisp, KeymapPreset::Hacivat));
    assert_eq!(s.ui.language, LanguagePref::System);
}
```
- [ ] **Step 2:** Run `cargo test -p xtask clippy_toml && cargo test -p hacivat-settings defaults`. Expected: FAIL (file or crate missing).
- [ ] **Step 3: Write the manifests.**
  - Root `Cargo.toml`: `[workspace.lints.rust] unsafe_code = "deny"` and `[workspace.lints.clippy] disallowed_methods = "deny"`.
  - Add the Tech Stack dependencies to `[workspace.dependencies]` with exact versions resolved now. Record them in the commit message.
  - Add `#[allow(unsafe_code)]` with SAFETY comments on the existing `native_window_id` until Task 4 moves it.
- [ ] **Step 4: Write the skeletons and the catalog files with the interfaces above.**
- [ ] **Step 5:** Run `cargo check --workspace --all-targets && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo xtask check-deps && cargo xtask tokens --check`. Expected: all pass, and every existing M0 test still passes.
- [ ] **Step 6: Commit** with message `chore(m1): interface commit — crate skeletons, lints, casing ban, catalog skeleton`.

### Task 2: Dependency guard: transitive GPUI/zed reach, layers, lints

**Files:**
- Modify: `xtask/src/check_deps.rs`

**Interfaces:**
- Produces:
  - `pub enum Rule { ZedOutsideAdapter, CoreReachesUi, UiDependsOnCore, AdapterFeatureInProduct, MissingWorkspaceLints }`
  - `pub struct Violation { pub krate: String, pub rule: Rule, pub via: Vec<String> }`
  - `pub fn violations(md: &Metadata, manifest: &dyn Fn(&Path) -> String) -> Vec<Violation>`
  - `pub const ZED_SOURCES: [&str; 2] = ["https://github.com/zed-industries/zed", "https://github.com/berkaygkc/zed"]`
  - Layer comes from the manifest path: `crates/ui/` → Ui, `crates/{core,sql,ai}/` → Core, `crates/{app,bin}/` → App, `spikes/` → Spike, `xtask` → Tool.

- [ ] **Step 1: Write the failing tests.** Extend `fixture_metadata` to emit `resolve.nodes[].deps[].dep_kinds` and package `source`.
```rust
#[test] fn flags_transitive_gpui_through_helper() { let md = fx(&[("spike-x", &["helper"]), ("helper", &["gpui"])]); assert_eq!(names(&violations(&md, &lints_ok)), ["helper", "spike-x"]); }
#[test] fn gpui_through_adapter_is_allowed() { let md = fx(&[("spike-x", &["hacivat-ui-core"]), ("hacivat-ui-core", &["gpui", "gpui_platform"])]); assert!(violations(&md, &lints_ok).is_empty()); }
#[test] fn flags_dev_dependency_on_gpui() { let md = fx_dev(&[("hacivat-ui", &["gpui"])]); assert_eq!(rule_of(&md, "hacivat-ui"), Rule::ZedOutsideAdapter); }
#[test] fn flags_gpui_platform_only() { let md = fx(&[("hacivat-app", &["gpui_platform"])]); assert_eq!(rule_of(&md, "hacivat-app"), Rule::ZedOutsideAdapter); }
#[test] fn flags_any_zed_sourced_crate() { let md = fx_src(&[("hacivat-ui", &[("collections", ZED_SOURCES[0])])]); assert_eq!(rule_of(&md, "hacivat-ui"), Rule::ZedOutsideAdapter); }
#[test] fn flags_fork_sourced_crate() { let md = fx_src(&[("hacivat-ui", &[("gpui_util", ZED_SOURCES[1])])]); assert_eq!(rule_of(&md, "hacivat-ui"), Rule::ZedOutsideAdapter); }
#[test] fn flags_core_reaching_adapter() { let md = fx_at(&[("hacivat-l10n", "crates/core", &["hacivat-ui-core"])]); assert_eq!(rule_of(&md, "hacivat-l10n"), Rule::CoreReachesUi); }
#[test] fn flags_ui_depending_on_core() { let md = fx_at(&[("hacivat-ui", "crates/ui", &["hacivat-l10n"]), ("hacivat-l10n", "crates/core", &[])]); assert_eq!(rule_of(&md, "hacivat-ui"), Rule::UiDependsOnCore); }
#[test] fn app_may_use_core_and_ui() { let md = fx_at(&[("hacivat-app", "crates/app", &["hacivat-ui", "hacivat-l10n"])]); assert!(violations(&md, &lints_ok).is_empty()); }
#[test] fn flags_product_enabling_spike_features() { let md = fx_feat("hacivat", "hacivat-ui-core", &["spike-support"]); assert_eq!(rule_of(&md, "hacivat"), Rule::AdapterFeatureInProduct); }
#[test] fn flags_member_without_workspace_lints() { let md = fx(&[("hacivat-ui", &[])]); let v = violations(&md, &|_| "[package]\nname=\"hacivat-ui\"\n".into()); assert_eq!(v[0].rule, Rule::MissingWorkspaceLints); }
```
- [ ] **Step 2:** Run `cargo test -p xtask check_deps`. Expected: FAIL.
- [ ] **Step 3: Implement `violations`.**
  - BFS over `resolve.nodes` from each workspace crate across normal, build and dev edges.
  - Do not expand through `hacivat-ui-core` for Ui, App, Spike and Tool crates.
  - Core crates are not allowed to reach `hacivat-ui-core` at all.
  - A package counts as zed-sourced when `source` starts with `git+` followed by any entry of `ZED_SOURCES`.
  - `AdapterFeatureInProduct` applies to `hacivat` and `hacivat-app` when they request `spike-support` or `bench` from `hacivat-ui-core` (`dependencies[].features`).
  - `MissingWorkspaceLints`: the manifest parsed with `toml` lacks `lints.workspace = true`.
  - `run()` prints `check-deps: ok` or one line per violation (`<crate>: <rule> via a -> b -> gpui`) and exits 1.
- [ ] **Step 4:** Run `cargo test -p xtask check_deps && cargo xtask check-deps`. Expected: PASS and `check-deps: ok`.
- [ ] **Step 5: Commit** with message `feat(xtask): transitive GPUI/zed reach, layer and lint guards`.

### Task 3: `hacivat-bench` and bench-report hygiene

**Files:**
- Modify: `crates/ui/hacivat-bench/src/lib.rs`, `xtask/src/bench_report.rs`, `xtask/Cargo.toml` (drop the duplicate `serde_json` dev-dependency)

**Interfaces:**
- Consumes: the moved `FrameRecorder`, `stats_of`, `verdict`, `build_label(head: &str, porcelain_z: &str) -> String`, `build_stamp(root: &Path) -> String`.
- Produces:
  - `build_stamp` now runs `git status --porcelain -z` and ignores paths under `docs/benchmarks/`.
  - `bench_report::escape_cell(s: &str) -> String`
  - `bench_report::parse_milestone(s: &str) -> Result<String>`, which accepts `^m[0-9]+$`
  - Rows are de-duplicated by source JSON file name.

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn bench_doc_edits_are_not_dirty() { assert_eq!(build_label("abc1234", " M docs/benchmarks/m1.md\0"), "abc1234"); }
#[test] fn non_ascii_path_is_dirty_without_quoting() { assert_eq!(build_label("abc1234", " M crates/ui/İçerik.rs\0"), "abc1234-dirty"); }
#[test] fn pipe_in_label_is_escaped() { assert_eq!(escape_cell("b1|x"), "b1\\|x"); }
#[test] fn milestone_rejects_paths() { assert!(parse_milestone("../m0").is_err()); assert_eq!(parse_milestone("m1").unwrap(), "m1"); }
#[test] fn rerun_does_not_duplicate_rows() { let doc = append_rows(HEADER, &[row("b1-fling-macos-1.json")]); assert_eq!(append_rows(&doc, &[row("b1-fling-macos-1.json")]), doc); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-bench -p xtask bench_report`. Expected: FAIL.
- [ ] **Step 3: Implement.** The source JSON file name goes in a trailing hidden HTML comment cell, so de-duplication needs no extra column.
- [ ] **Step 4:** Run the same tests, then `cargo build -p spike-grid -p spike-editor`. Expected: PASS, and the spikes still compile.
- [ ] **Step 5: Commit** with message `refactor(bench): gpui-free hacivat-bench; bench-report escaping, milestone check, de-dup`.

### Task 4: ui-core single bootstrap, spike/bench features, text rendering (L12)

**Files:**
- Modify: `crates/ui/hacivat-ui-core/src/{app.rs, fonts.rs, lib.rs, theme/mod.rs}`, `Cargo.toml`
- Create: `src/assets.rs` (moved from `shell.rs`), `src/platform/{mod.rs, text_rendering.rs}`, `src/spike_support.rs` (moved from `shell.rs`)
- Delete: `src/shell.rs`
- Add: Lucide v0.460.0 `menu`, `minus`, `square`, `copy`, `plus`, `settings`, `info`, `triangle-alert`, `circle-check`, `ellipsis`, `keyboard`, `rows-2`, `panel-left` to `assets/icons/lucide/`, plus the `ICONS` list
- Modify: `spikes/*/Cargo.toml` (enable `spike-support`/`bench`) and their `use` lines

**Interfaces:**
- Produces:
  - `pub enum TextRendering { Crisp, Standard }`
  - `pub struct Boot { pub text_rendering: TextRendering }`, with `Default` = `Crisp`
  - `pub fn run_app(boot: Boot, f: impl FnOnce(&mut App) + 'static)`. This is the only bootstrap. It calls `platform::apply_text_rendering(boot.text_rendering)` before `gpui_platform::application()`, then installs assets, registers and verifies fonts (exit 3 on failure), installs the `Dark` theme and calls `f`.
  - `platform::apply_text_rendering(mode: TextRendering)`:
    - macOS: `CFPreferencesSetAppValue("AppleFontSmoothing", 0 for Crisp | NULL for Standard, kCFPreferencesCurrentApplication)`. Standard removes our override, so the user's global preference applies.
    - Other OSes: no-op.
  - `platform::app_font_smoothing_override() -> Option<i64>`: reads back the app-domain value on macOS, `None` elsewhere.
  - `pub const TEXT_RENDERING_IS_LIVE: bool = false` (R11).
  - `platform::native_window_id(window: &Window) -> Option<u32>` (moved from `shell.rs`; not feature-gated, because the app's `--print-window-id` uses it)
  - `fonts::verify_bundled_fonts(cx: &App) -> Result<()>`
  - `assets::BundledAssets`
  - `spike_support::{display_sizes, open_unified_window}` under `spike-support`, re-exporting `platform::native_window_id` for the spikes.
  - `frame_loop` and `pub use hacivat_bench as frames` under `bench`.

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn boot_default_is_crisp() { assert_eq!(Boot::default().text_rendering, TextRendering::Crisp); }
#[cfg(target_os = "macos")] #[test] fn crisp_then_standard_toggles_the_app_override() {
    platform::apply_text_rendering(TextRendering::Crisp);   assert_eq!(platform::app_font_smoothing_override(), Some(0));
    platform::apply_text_rendering(TextRendering::Standard); assert_eq!(platform::app_font_smoothing_override(), None);
}
#[test] fn text_rendering_is_restart_only() { assert!(!TEXT_RENDERING_IS_LIVE); }
#[test] fn radii_on_scale_in_both_themes() { for t in [&DARK, &LIGHT] { assert_eq!([t.radii.none, t.radii.xs, t.radii.sm, t.radii.md], [0.0, 3.0, 4.0, 6.0]); } }
#[test] fn bundled_icons_include_m1_set() { for n in ["menu", "minus", "square", "settings", "ellipsis", "keyboard"] { assert!(BundledAssets.load(&format!("icons/lucide/{n}.svg")).unwrap().is_some(), "{n}"); } }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-ui-core`. Expected: FAIL.
- [ ] **Step 3: Implement the split and the platform module.**
  - `platform/mod.rs` is the only module with `#![allow(unsafe_code)]`. It uses `core-foundation`/`core-foundation-sys`.
  - Add a comment in `fonts.rs` saying that `tnum` is a no-op because Source Sans 3 digits are tabular by default (deferred minor T3).
- [ ] **Step 4:** Run:
  ```
  cargo test -p hacivat-ui-core
  cargo check -p hacivat-ui-core
  cargo check -p hacivat-ui-core --features spike-support,bench
  cargo build -p spike-harman -p spike-grid -p spike-editor -p spike-probes
  ```
  Expected: PASS. Without features, the crate exposes no `spike_support`/`frame_loop`.
- [ ] **Step 5: Commit** with message `feat(ui-core): single bootstrap, crisp text by default (L12), spike/bench features`.

### Task 5: Keymap model, data and AltGr collision CI (UI-EDIT-13)

**Files:**
- Create:
  - `crates/ui/hacivat-ui-core/src/keymap/{parse,normalize,resolve,altgr,display}.rs`
  - `crates/ui/hacivat-ui/keymaps/**`
  - `crates/ui/hacivat-ui/src/keymaps.rs`
  - `crates/ui/hacivat-ui/tests/keymap_collisions.rs`

**Interfaces:**
- Consumes: the Task 1 keymap types.
- Produces:
  - `parse_chord(s: &str) -> Result<Vec<KeyPress>, KeymapError>`, for example `"ctrl-shift-7"` or `"cmd-k cmd-s"`
  - `normalize_char(c: char) -> char`: NFC, then `ı`/`İ`/`I` → `i`, then ASCII lowercase
  - `normalize(k: &Keystroke) -> Option<KeyPress>`: `None` for a dead key (no `key_char` and not a named key)
  - `Keymap::from_sources(platform: KeymapPlatform, default: &str, preset: &str, layout_overlays: &[(&LayoutId, &str)], user: Option<&str>) -> (Keymap, Vec<KeymapDiag>)`
  - `Keymap::resolve(&self, ctx: &ContextStack, pending: &[KeyPress], press: &KeyPress, layout: &LayoutId, composing: bool) -> Resolution`
  - `Keymap::bindings_for(&self, command: &str, layout: &LayoutId) -> Vec<&Binding>`
  - `AltGrTable::parse(toml: &str) -> AltGrTable`
  - `altgr_collisions(km: &Keymap, platform: KeymapPlatform, tables: &[AltGrTable]) -> Vec<Collision { chord: String, layout: String, produces: char }>`
  - `display_chord(chord: &[KeyPress], platform: KeymapPlatform) -> String`: macOS `⌃⌥⇧⌘` glyphs, otherwise `Ctrl+Shift+7` with English modifier names
  - `layout_id_from_platform(id: &str) -> LayoutId`: maps macOS/Windows/Linux layout IDs such as `com.apple.keylayout.Turkish` and `00000041F` to `tr-q`
  - `hacivat_ui::keymaps::load(platform, preset: &str, user: Option<&str>) -> (Keymap, Vec<KeymapDiag>)`, which embeds the data files
- Data. The default (`hacivat` preset) M1 bindings, in context `Workspace`:

  | Command | macOS | Windows/Linux |
  |---|---|---|
  | `workspace.command_palette` | cmd-k | ctrl-k |
  | `workspace.open_settings` | cmd-, | ctrl-, |
  | `workspace.new_console_tab` | cmd-t | ctrl-t |
  | `workspace.close_tab` | cmd-w | ctrl-w (Windows also ctrl-f4) |
  | `workspace.next_tab` | ctrl-tab | ctrl-tab |
  | `workspace.prev_tab` | ctrl-shift-tab | ctrl-shift-tab |
  | `workspace.toggle_explorer` | cmd-1 | alt-1 |
  | `app.quit` | cmd-q | Linux ctrl-q |

  The `datagrip` preset uses `workspace.command_palette` = cmd-shift-a/ctrl-shift-a and `workspace.open_settings` = cmd-,/ctrl-alt-s.

  AltGr tables (`keymaps/tests/altgr/*.toml`) list the AltGr+key → char pairs for TR-Q, TR-F, DE, FR (AZERTY) and PL (Programmer). Each file has a header citing its source layout (KBDTUQ, KBDTUF, KBDGR, KBDFR, KBDPL1).

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn tr_q_dotless_i_matches_ctrl_i() { let km = fixture("[[binding]]\nchord=\"ctrl-i\"\ncommand=\"t.x\"\ncontext=\"Workspace\""); assert_eq!(km.resolve(&ws(), &[], &kp(ctrl(), 'ı'), &tr_q(), false), Resolution::Command("t.x".into())); }
#[test] fn shift_dotless_i_gives_capital_i_matches_ctrl_shift_i() { assert_eq!(normalize_char('I'), 'i'); assert_eq!(normalize_char('İ'), 'i'); }
#[test] fn altgr_char_passes_through() { let km = defaults(KeymapPlatform::Windows); assert_eq!(km.resolve(&ws(), &[], &kp(ctrl_alt(), 'q'), &tr_q(), false), Resolution::PassThrough); }
#[test] fn dead_key_never_matches() { assert_eq!(normalize(&Keystroke { modifiers: Modifiers::none(), key: "dead_circumflex".into(), key_char: None }), None); }
#[test] fn composing_blocks_bindings() { let km = defaults(KeymapPlatform::MacOs); assert_eq!(km.resolve(&ws(), &[], &kp(cmd(), 'k'), &us(), true), Resolution::PassThrough); }
#[test] fn two_key_chord_is_pending_then_fires() { let km = fixture_chord("ctrl-k ctrl-s", "t.save"); assert_eq!(km.resolve(&ws(), &[], &kp(ctrl(), 'k'), &us(), false), Resolution::Pending); assert_eq!(km.resolve(&ws(), &[kp(ctrl(), 'k')], &kp(ctrl(), 's'), &us(), false), Resolution::Command("t.save".into())); }
#[test] fn innermost_context_wins() { /* "Palette" binding on escape shadows "Workspace" escape */ }
#[test] fn tr_q_overlay_displays_shift_7_for_slash() { let km = fixture_with_overlay("ctrl-/", "tr-q", "ctrl-shift-7"); assert_eq!(display_chord(&km.bindings_for("t.c", &tr_q())[0].chord, KeymapPlatform::Windows), "Ctrl+Shift+7"); }
#[test] fn mac_display_uses_glyphs() { assert_eq!(display_chord(&parse_chord("cmd-shift-p").unwrap(), KeymapPlatform::MacOs), "⇧⌘P"); }
#[test] fn duplicate_chord_in_context_is_diagnosed() { let (_, d) = Keymap::from_sources(KeymapPlatform::Windows, DUP, "", &[], None); assert_eq!(d.len(), 1); }
// hacivat-ui/tests/keymap_collisions.rs
#[test] fn default_keymaps_have_no_altgr_collisions() { for p in [KeymapPlatform::Windows, KeymapPlatform::Linux] { assert_eq!(altgr_collisions(&load(p, "hacivat", None).0, p, &tables()), vec![]); } }
#[test] fn adding_ctrl_alt_l_fails_naming_polish_l_stroke() { let km = with_extra(KeymapPlatform::Windows, "ctrl-alt-l"); let c = altgr_collisions(&km, KeymapPlatform::Windows, &tables()); assert!(c.iter().any(|c| c.layout == "pl" && c.produces == 'ł')); }
#[test] fn mac_defaults_never_use_bare_option_on_printable() { assert!(option_only_printables(&load(KeymapPlatform::MacOs, "hacivat", None).0).is_empty()); }
#[test] fn known_altgr_facts() { let t = tables(); assert_eq!(t.get("tr-q", 'q'), Some('@')); assert_eq!(t.get("de", 'm'), Some('µ')); assert_eq!(t.get("pl", 'l'), Some('ł')); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-ui-core keymap && cargo test -p hacivat-ui --test keymap_collisions`. Expected: FAIL.
- [ ] **Step 3: Implement parsing, normalization, resolution and the collision check.** Resolution walks the context stack innermost first, matches on a `(Mods, NormKey)` hash, and applies layout-overlay alternatives for the active `LayoutId` (UI-EDIT-13 steps 2–5). For a symbol chord, ignore Shift when the active layout needs Shift to type that symbol.
- [ ] **Step 4: Write the data files.**
- [ ] **Step 5:** Run the same tests. Expected: PASS.
- [ ] **Step 6: Commit** with message `feat(keymap): UI-EDIT-13 model, M1 default keymaps, AltGr collision CI`.

### Task 6: `hacivat-l10n` core: catalogs, negotiation, typed IDs, render, swap

**Files:**
- Create:
  - `crates/core/hacivat-l10n-schema/src/lib.rs`
  - `crates/core/hacivat-l10n/build.rs`
  - `crates/core/hacivat-l10n/src/{lib,catalog,negotiate,runtime,render,args}.rs`
  - `crates/core/hacivat-l10n/tests/{render.rs, negotiate.rs, ui/wrong_args.rs}`
- Modify: `locales/{en,tr}/common.ftl`

**Interfaces:**
- Consumes: the Task 1 signatures, and `format::install_functions` / `pseudo` stubs (Task 9 fills them; until then the bundle uses Fluent's default formatting).
- Produces:
  - Schema crate:
    - `Signature { id: String, vars: Vec<(String, VarType)>, attrs: Vec<String>, selector_vars: Vec<String> }`
    - `VarType { String, Number, DateTime, List, Enum(Vec<String>) }`
    - `extract(ftl: &str, file: &str) -> Result<Vec<Signature>, Vec<SchemaError>>`, which reads the `# Variables:` blocks
  - `build.rs` turns `locales/en/*.ftl` into `ids.rs`, exposed as `hacivat_l10n::ids::{cm, hv, hui}`:
    - each message becomes a const `hv::TITLEBAR_PALETTE: MsgRef<NoArgs>`;
    - a message with variables gets an args struct named after it in PascalCase (`HvTabConsoleUntitled { pub n: Count }`).
    - The build fails on an undocumented variable. Dynamic families are listed in `locales/_schema/dynamic-ids.toml` (`hui-*`).
  - Catalogs:
    - `Catalogs::embedded() -> Catalogs` uses `rust-embed` over `locales/` and skips `_`-prefixed folders.
    - `Catalogs::with_overlay(self, dir: &Path) -> Catalogs`
    - `Catalogs::from_memory(files: &[(&str, &str)]) -> Catalogs`
    - `locales() -> &[CatalogLocale]`, `diags() -> &[LoadDiag]`
  - `CatalogLocale { tag, max, native_name, english_name, short_name, direction, status, source }`
  - `LoadDiag { NonCanonicalTag{folder}, MissingManifest{tag}, Junk{file, line}, ReservedPseudo{folder}, OverlaySkipped{path, reason} }`
  - Negotiation:
    - `negotiate(ctx: &NegotiationCtx, avail: &[CatalogLocale]) -> Negotiated`
    - `NegotiationCtx { sources: Vec<PrefSource>, format_pref: Option<String>, os_format: Option<String>, allow_experimental: bool, allow_pseudo: bool }`
    - `PrefSource { Setting(String), Env(&'static str, String), OsList(Vec<String>) }`
    - `Negotiated { chain: Vec<LanguageIdentifier>, format_locale: String, diags: Vec<NegDiag> }`
  - `L10n` per the Task 1 signatures. It is backed by `ArcSwap<Runtime>`, holds two bundle sets per locale (Ui isolating, Machine not), and falls back per message (I18N-04 steps 5–9).

- [ ] **Step 1: Write the failing tests** (the normative rows of I18N-02, -03 and -04).
```rust
#[test] fn embedded_has_en_stable_and_tr_beta() { let c = Catalogs::embedded(); assert_eq!(tags(&c), ["en", "tr"]); assert_eq!(status(&c, "tr"), Status::Beta); }
#[test] fn overlay_without_manifest_is_experimental() { let c = Catalogs::embedded().with_overlay(&tmp_overlay(&[("de/hacivat-ui.ftl", "hv-x = X")])); assert_eq!(status(&c, "de"), Status::Experimental); assert_eq!(english_name(&c, "de"), "de"); }
#[test] fn overlay_overrides_per_id() { /* overlay tr/hacivat-ui.ftl redefines hv-titlebar-palette = Komut ara → [tr] render == "Komut ara" */ }
#[test] fn junk_line_keeps_other_messages() { let c = Catalogs::from_memory(&[("en/locale.toml", EN_TOML), ("en/hacivat-ui.ftl", "hv-a = A\n\n\n\n\n\n!!junk\nhv-b = B\n")]); assert!(c.diags().contains(&LoadDiag::Junk { file: "en/hacivat-ui.ftl".into(), line: 7 })); }
#[test] fn pseudo_folder_on_disk_is_rejected() { /* en-XA/ → LoadDiag::ReservedPseudo */ }
#[test] fn os_tr_tr_gives_tr_chain() { let n = neg(&[os(&["tr-TR", "en-US"])], None); assert_eq!(chain(&n), ["tr", "en"]); assert_eq!(n.format_locale, "tr-TR"); }
#[test] fn english_setting_keeps_turkish_region() { let n = neg(&[set("en"), os(&["tr-TR"])], Some("tr-TR")); assert_eq!(chain(&n), ["en"]); assert_eq!(n.format_locale, "tr-TR"); }
#[test] fn lang_c_is_english_with_diag() { let n = neg(&[env("LANG", "C")], None); assert_eq!(chain(&n), ["en"]); assert!(!n.diags.is_empty()); }
#[test] fn codeset_and_modifier_are_stripped() { assert_eq!(chain(&neg(&[env("LANG", "tr_TR.ISO-8859-9")], None)), ["tr", "en"]); assert_eq!(chain(&neg(&[env("LANG", "tr_TR@euro")], None)), ["tr", "en"]); }
#[test] fn never_across_languages() { assert_eq!(chain(&neg(&[set("az")], None)), ["en"]); }
#[test] fn numbering_extension_kept_in_format_only() { let n = neg(&[os(&["tr-TR-u-nu-arab"])], None); assert_eq!(chain(&n), ["tr", "en"]); assert_eq!(n.format_locale, "tr-TR-u-nu-arab"); }
#[test] fn zh_hk_picks_hant() { assert_eq!(chain_with(&["zh-Hans", "zh-Hant"], &[os(&["zh-HK"])]), ["zh-Hant", "en"]); }
#[test] fn pseudo_needs_permission() { assert_eq!(chain(&neg(&[set("en-XA")], None)), ["en"]); assert_eq!(chain(&neg_pseudo(&[set("en-XA")])), ["en-XA", "en"]); }
// render.rs  (format locale en-US until Task 9)
#[test] fn missing_tr_message_falls_back_to_en() { let l = l10n(&["tr", "en"], &[("tr", "hv-a = A-tr")], &[("en", "hv-a = A\nhv-b = B")]); assert_eq!(l.t_dynamic("hv-b", vec![]), "B"); }
#[test] fn tr_typo_variable_falls_back_in_release() { /* tr "{ $cuont } satır", en "{ $count } rows", count 2 → "2 rows" */ }
#[test] fn ui_sink_isolates_and_machine_sink_does_not() { /* tr "Bağlantı { $name } kaydedildi." name "prod" → t: "Bağlantı \u{2068}prod\u{2069} kaydedildi.", t_machine: "Bağlantı prod kaydedildi." */ }
#[test] fn stale_build_is_discarded() { let l = L10n::new(rt(1)); assert!(l.install_if_newer(rt(3))); assert!(!l.install_if_newer(rt(2))); assert_eq!(l.generation(), 3); }
#[test] fn typed_ids_reject_wrong_args() { trybuild::TestCases::new().compile_fail("tests/ui/wrong_args.rs"); }
#[test] fn cold_parse_en_tr_under_10ms() { /* release-only (#[cfg(not(debug_assertions))]): median of 20 builds of [tr,en] runtime ≤ 10 ms */ }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-l10n -p hacivat-l10n-schema`. Expected: FAIL.
- [ ] **Step 3: Implement `extract` and `build.rs`.** Declare `cargo:rerun-if-changed=../../../locales`.
- [ ] **Step 4: Implement `Catalogs`, `negotiate` and `L10n`.**
  - Overlay safety limits (I18N-02 step 5): 1 MB per file, UTF-8 required, no symlinks leaving the root, only `*.ftl` and `locale.toml`.
  - Release builds fall back to the next bundle on a structural error. Debug builds return `⚠[text]`.
- [ ] **Step 5:** Run `cargo test -p hacivat-l10n -p hacivat-l10n-schema` (and `--release` for the budget test). Expected: PASS.
- [ ] **Step 6: Commit** with message `feat(l10n): catalogs, negotiation, typed IDs, per-message fallback, atomic swap`.

### Task 7: `hacivat-settings`: store, local paths, workspace state

**Files:**
- Create: `crates/core/hacivat-settings/src/{store,paths,workspace_state,fsio}.rs`
- Modify: `crates/core/hacivat-settings/src/{lib,schema}.rs`

**Interfaces:**
- Produces:
  - Store:
    - `load(path: &Path) -> LoadOutcome { settings: Settings, diags: Vec<SettingsDiag>, quarantined: Option<PathBuf> }`
    - `SettingsDiag { code: &'static str, key: String }` (codes only; the app renders them via Fluent)
    - `save_changes(path: &Path, new: &Settings) -> io::Result<()>`: a `toml_edit` merge that keeps comments and unknown keys, then an atomic write
  - File I/O:
    - `fsio::atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()>`: temp file plus rename, `0600` on Unix
    - `fsio::quarantine(path: &Path, tag: &str, now: SystemTime) -> io::Result<PathBuf>`, which renames to `<name>.<tag>-<unix_secs>`
  - `LocalPaths { config, data, cache, runtime: Option<PathBuf> }`:
    - `from_os() -> io::Result<Self>`, via `directories::ProjectDirs::from("dev", "hacivat", "Hacivat")`
    - `under(root: &Path) -> Self`, for tests
    - `ensure_dirs(&self) -> io::Result<()>`
    - one accessor per §5.6 row: `settings_toml`, `connections_toml`, `keymaps_dir`, `profile_toml(uuid)`, `vault_bin`, `catalog_db(uuid, major)`, `history_db`, `consoles_dir(uuid)`, `workspace_json`, `ai_broker_db`, `ai_audit_db`, `ipc_endpoint(user_sid: Option<&str>)`, `logs_dir`, `locales_overlay_dir`
  - Workspace state:
    - `workspace_state::load(path: &Path, supported: u32) -> Option<serde_json::Value>`: a newer `version` is set aside with tag `newer`; an unparsable file with tag `corrupt`
    - `workspace_state::save(path: &Path, version: u32, value: &serde_json::Value) -> io::Result<()>`

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn defaults_when_file_missing() { let o = load(&tmp().join("settings.toml")); assert_eq!(o.settings, Settings::default()); assert!(o.diags.is_empty()); }
#[test] fn round_trip_keeps_unknown_keys_and_comments() {
    let p = write_tmp("# mine\nversion = 1\n[ui]\ntheme = \"light\"\nfuture_key = 3\n");
    let mut s = load(&p).settings; s.ui.theme = ThemePref::Dark; save_changes(&p, &s).unwrap();
    let t = std::fs::read_to_string(&p).unwrap(); assert!(t.contains("# mine") && t.contains("future_key = 3") && t.contains("theme = \"dark\""));
}
#[test] fn invalid_language_tag_falls_back_to_system() { let o = load(&write_tmp("[ui]\nlanguage = \"klingon!!\"\n")); assert_eq!(o.settings.ui.language, LanguagePref::System); assert_eq!(o.diags[0].key, "ui.language"); }
#[test] fn wrong_type_uses_default() { let o = load(&write_tmp("[ui]\ntheme = 3\n")); assert_eq!(o.settings.ui.theme, ThemePref::Dark); assert_eq!(o.diags[0].code, "settings-invalid-value"); }
#[test] fn unparsable_file_is_quarantined_not_overwritten() { let p = write_tmp("[[["); let o = load(&p); assert!(o.quarantined.unwrap().to_string_lossy().contains("settings.toml.corrupt-")); assert!(!p.exists()); }
#[cfg(unix)] #[test] fn permissions_are_0700_and_0600() { /* ensure_dirs + save → mode & 0o777 == 0o700 dir, 0o600 file */ }
#[test] fn local_paths_follow_spec_table() {
    let p = LocalPaths::under(Path::new("/r")); let u = "6f1c…";
    assert_eq!(p.settings_toml(), Path::new("/r/config/settings.toml")); assert_eq!(p.history_db(), Path::new("/r/data/history.sqlite"));
    assert_eq!(p.catalog_db(u, 1), Path::new("/r/data/catalog").join(format!("{u}.v1.sqlite"))); assert_eq!(p.workspace_json(), Path::new("/r/data/workspace.json"));
    assert_eq!(p.logs_dir(), Path::new("/r/cache/logs")); assert_eq!(p.locales_overlay_dir(), Path::new("/r/config/locales"));
    assert_eq!(p.ipc_endpoint(Some("S-1-5-21-1")).to_string_lossy(), if cfg!(windows) { r"\\.\pipe\hacivat-S-1-5-21-1" } else { "/r/runtime/hacivat/broker.sock" });
}
#[test] fn future_workspace_version_is_set_aside() { let p = write_tmp_named("workspace.json", r#"{"version":2,"panes":{}}"#); assert!(workspace_state::load(&p, 1).is_none()); assert!(sibling_with_prefix(&p, "workspace.json.newer-")); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-settings`. Expected: FAIL.
- [ ] **Step 3: Implement.** Validate each key independently, so one bad value never resets the others. Check language tags with `unic-langid` parsing only; this crate does not depend on l10n.
- [ ] **Step 4:** Run `cargo test -p hacivat-settings`. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(settings): versioned settings store, §5.6 paths, workspace state with set-aside`.

### Task 8: Window chrome and the paint-level One Filled Badge check

**Files:**
- Create: `crates/ui/hacivat-ui-core/src/{window.rs, named_rules.rs}`, `crates/ui/hacivat-ui-core/tests/{window.rs, named_rules.rs}`

**Interfaces:**
- Produces:
  - `pub struct WorkspaceWindowSpec { pub title: SharedString, pub size: (f32, f32), pub min_size: (f32, f32) }`. The default is 1440×900 with a minimum of 960×600.
  - `pub fn open_workspace_window<V: Render + 'static>(cx: &mut App, spec: WorkspaceWindowSpec, build: impl FnOnce(&mut Window, &mut App) -> Entity<V>) -> anyhow::Result<WindowHandle<V>>`:
    - macOS: transparent title bar, traffic lights at (14, 13)
    - Windows: `appears_transparent` with a custom caption
    - Linux: `window_decorations: Some(WindowDecorations::Client)`
  - `pub enum ChromeKind { MacTrafficLights, WindowsCaption, LinuxClient, LinuxServer }` and `pub fn chrome_kind(window: &Window) -> ChromeKind`
  - `pub const TRAFFIC_LIGHTS_INSET_PX: f32`: the width the title bar reserves on macOS, copied from `spikes/spike-harman/src/screen/titlebar.rs`
  - Under `scene-check`:
    - `pub fn count_solid_quads(window: &Window, color: Hsla) -> usize`
    - `pub fn assert_one_filled_badge(window: &Window, cp_fill: Hsla, expect_production: bool)`, which panics with the count

- [ ] **Step 1: Write the failing tests** (VisualTestContext, `--features test-support,scene-check`).
```rust
#[gpui::test] fn workspace_window_opens_with_min_size(cx: &mut TestAppContext) { let w = open_spec(cx, WorkspaceWindowSpec::default()); assert!(w.is_ok()); }
#[gpui::test] fn one_cp_fill_quad_counts_one(cx: &mut TestAppContext) { let (_, vcx) = draw(cx, |_| CpFillProbe { blocks: 1 }); vcx.update(|w, _| assert_eq!(count_solid_quads(w, DARK.colors.cp_fill.to_hsla()), 1)); }
#[gpui::test] #[should_panic(expected = "cp-fill quads: 2")] fn two_cp_fill_quads_fail(cx: &mut TestAppContext) { let (_, vcx) = draw(cx, |_| CpFillProbe { blocks: 2 }); vcx.update(|w, _| assert_one_filled_badge(w, DARK.colors.cp_fill.to_hsla(), true)); }
#[gpui::test] fn no_production_means_zero_quads(cx: &mut TestAppContext) { let (_, vcx) = draw(cx, |_| CpFillProbe { blocks: 0 }); vcx.update(|w, _| assert_one_filled_badge(w, DARK.colors.cp_fill.to_hsla(), false)); }
#[test] fn chrome_kind_per_os() { /* cfg-matched: macOS → MacTrafficLights; windows → WindowsCaption */ }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-ui-core --features test-support,scene-check --test window --test named_rules`. Expected: FAIL.
- [ ] **Step 3: Implement.** `count_solid_quads` iterates over `window.rendered_scene().quads` and compares `background.as_solid()` within 1/255 per channel.
- [ ] **Step 4:** Run the same command. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(ui-core): per-OS workspace window chrome; paint-level One Filled Badge check`.

### Task 9: l10n formatting, settings model, pseudo-locales, search key, display casing

**Files:**
- Create: `crates/core/hacivat-l10n/src/{format,effective,pseudo,search,casing}.rs`, `crates/core/hacivat-l10n/tests/{format.rs, pseudo.rs, search.rs}`

**Interfaces:**
- Consumes: `build_runtime` and the bundle constructor (Task 6), and `hacivat_settings::Settings`.
- Produces:
  - Formatting:
    - `FormatCtx { pub format_locale: String, pub time_zone: String, pub generation: u64 }`
    - `with_format_ctx<R>(ctx: &FormatCtx, f: impl FnOnce() -> R) -> R` (re-entrant)
    - `install_functions(bundle: &mut FluentBundle<..>)`: the `NUMBER`, `DATETIME` and `LIST` functions and the `hv_format` formatter
  - Settings model:
    - `OsLocale { languages: Vec<String>, format: Option<String>, time_zone: Option<String>, env: Vec<(&'static str, String)> }` with `OsLocale::detect() -> OsLocale` (`sys-locale`, env, `iana-time-zone`)
    - `effective(s: &Settings, os: &OsLocale, allow_pseudo: bool) -> EffectiveLocale { ctx: NegotiationCtx, time_zone: String }`
  - Pseudo-locales:
    - `PseudoMode { EnXa, TrXt }`
    - `transform_literal(s: &str, m: PseudoMode) -> Cow<'_, str>`
    - `wrap_message(s: String) -> String` (adds `[` and `]`)
    - The pseudo locales are built from the `en` source when `Negotiated.chain[0]` is `en-XA` or `tr-XT`.
  - Search key:
    - `SearchOpts { pub turkish_tolerant: bool, pub mode: MatchMode }`
    - `MatchMode { Contains, WordPrefix, Fuzzy }`
    - `build_key(text: &str, o: &SearchOpts) -> SearchKey`
    - `find(text: &SearchKey, query: &str, o: &SearchOpts) -> Option<Match { ranges: Vec<Range<usize>>, score: u32, kind: MatchKind }>`, with byte ranges in the original text extended to grapheme boundaries
    - `MatchKind { ExactWord, Prefix, Substring, Fuzzy }`
    - `turkish_tolerant_for(ui_lang: &str, format_locale: &str) -> bool`
  - `case_for_display(text: &str, lang: &LanguageIdentifier) -> String`, using ICU4X `CaseMapper::uppercase`

- [ ] **Step 1: Write the failing tests** (the I18N-05 and I18N-06 normative values, the ALG-I18N-06 rows, and the I18N-16 rows).
```rust
#[test] fn number_tr_and_en() { assert_eq!(fmt("tr-TR", "{ NUMBER($n) }", n(1234.5)), "1.234,5"); assert_eq!(fmt("en-US", "{ NUMBER($n) }", n(1234.5)), "1,234.5"); }
#[test] fn percent_patterns() { assert_eq!(fmt("tr-TR", "{ NUMBER($n, style: \"percent\") }", n(0.25)), "%25"); assert_eq!(fmt("en-US", "{ NUMBER($n, style: \"percent\") }", n(0.25)), "25%"); }
#[test] fn ports_never_grouped() { assert_eq!(fmt("tr-TR", "{ NUMBER($n, useGrouping: \"false\") }", n(5432.0)), "5432"); }
#[test] fn dates_in_istanbul() { assert_eq!(fmt_tz("tr-TR", "Europe/Istanbul", "{ DATETIME($t, dateStyle: \"long\") }", t("2026-11-11T14:05:09Z")), "11 Kasım 2026"); assert_eq!(fmt_tz("tr-TR", "Europe/Istanbul", "{ DATETIME($t, dateStyle: \"short\") }", t("2026-11-11T14:05:09Z")), "11.11.2026"); }
#[test] fn lists() { assert_eq!(fmt("tr-TR", "{ LIST($l) }", l(&["a", "b", "c"])), "a, b ve c"); assert_eq!(fmt("en-US", "{ LIST($l) }", l(&["a", "b", "c"])), "a, b, and c"); }
#[test] fn plural_by_ui_language_digits_by_format_locale() { assert_eq!(render(&["en"], "tr-TR", "hv-r", "{ $count -> [one] { $count } row *[other] { $count } rows }", 1234), "1.234 rows"); assert_eq!(render(&["en"], "tr-TR", "hv-r", SAME, 1), "1 row"); }
#[test] fn turkish_count_message() { assert_eq!(render_tr("{ $count } satır", 1234.5, "tr-TR"), "1.234,5 satır"); }
#[test] fn unknown_option_keeps_default_formatting() { assert_eq!(fmt("en-US", "{ NUMBER($n, currency: \"TRY\") }", n(3.0)), "3"); }
#[test] fn english_ui_turkish_region_is_primary_case() { let e = effective(&settings_lang("en"), &os(&["tr-TR"], Some("tr-TR")), false); assert_eq!(negotiate(&e.ctx, &avail()).format_locale, "tr-TR"); }
#[test] fn pseudo_doubles_short_strings() { let s = transform_literal("Save", PseudoMode::EnXa); assert!(graphemes(&s) >= 8 && s.chars().any(|c| !c.is_ascii())); }
#[test] fn pseudo_skips_access_keys_markup_urls() { assert_eq!(transform_literal("S", PseudoMode::EnXa), "S"); assert_eq!(transform_literal("<code>SELECT 1</code>", PseudoMode::EnXa), "<code>SELECT 1</code>"); assert_eq!(transform_literal("https://hacivat.dev", PseudoMode::EnXa), "https://hacivat.dev"); }
#[test] fn tr_xt_injects_turkish_letters() { let s = transform_literal("Find in table", PseudoMode::TrXt); assert!(s.chars().any(|c| "ıİşğçöü".contains(c))); assert!(graphemes(&s) >= 21); }
#[test] fn pseudo_keeps_placeables_and_brackets() { let s = render_pseudo("hv-n", "{ $count } rows", 5); assert!(s.starts_with("[5") && s.ends_with(']')); assert!(!s.contains("rows")); }
#[test] fn search_rows() {
    let tol = SearchOpts { turkish_tolerant: true, mode: MatchMode::Contains }; let off = SearchOpts { turkish_tolerant: false, ..tol };
    assert!(hit("user_id", "ID", &tol)); assert_eq!(hl("İSTANBUL_SUBE", "istanbul", &tol), "İSTANBUL"); assert!(hit("ILIK", "ılık", &tol));
    assert!(hit("müşteri", "musteri", &tol)); assert!(!hit("müşteri", "musteri", &off)); assert_eq!(hl("Straße", "stras", &tol), "Straß");
    assert!(!hit("orders", "ıd", &tol)); assert!(hit("ad\u{2067}min", "admin", &tol));
}
#[test] fn tolerant_only_for_turkish() { assert!(turkish_tolerant_for("tr", "en-US")); assert!(turkish_tolerant_for("en", "tr-TR")); assert!(!turkish_tolerant_for("en", "en-US")); }
#[test] fn display_casing_is_turkish_aware() { assert_eq!(case_for_display("bilgi", &langid!("tr")), "BİLGİ"); assert_eq!(case_for_display("bilgi", &langid!("en")), "BILGI"); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-l10n --features pseudo`. Expected: FAIL.
- [ ] **Step 3: Implement.**
  - Formatting follows I18N-05 steps 1–8 (exact `Decimal` from the shortest round-trip text; formatters memoized per generation).
  - The pseudo transform follows ALG-I18N-03 with the length-budget table.
  - The search key follows ALG-I18N-06 steps 1–6. Turkish-tolerant folding maps `{I, i, İ, ı}` (plus a following U+0307) to `i`, plus accent folding.
- [ ] **Step 4:** Run `cargo test -p hacivat-l10n --features pseudo` and `LANG=tr_TR.UTF-8 LC_ALL=tr_TR.UTF-8 cargo test -p hacivat-l10n --features pseudo`. Expected: PASS both ways.
- [ ] **Step 5: Commit** with message `feat(l10n): ICU4X NUMBER/DATETIME/LIST, locale settings model, pseudo-locales, Turkish-tolerant search`.

### Task 10: `xtask l10n check` (I18N-17) and the literal-string lint (I18N-23)

**Files:**
- Create: `xtask/src/l10n/{mod.rs, rules.rs, code_scan.rs}`, `xtask/tests/fixtures/l10n/**`, `locales/_schema/literal-allowlist.toml`
- Modify: `xtask/src/main.rs` (arm `l10n`), `xtask/Cargo.toml` (`hacivat-l10n-schema`, `fluent-syntax`, `icu_plurals`)

**Interfaces:**
- Consumes: `hacivat_l10n_schema::extract`.
- Produces:
  - `cargo xtask l10n check [--root <dir>] [--json <out>]` checks the rule set in R10. It writes the human report to stdout, writes `target/l10n-report.json`, and exits 1 on any error.
  - `pub struct Finding { pub rule: &'static str, pub severity: Severity, pub locale: String, pub file: String, pub line: u32, pub id: String }`
  - `pub fn check(root: &Path, code_roots: &[&Path]) -> Vec<Finding>`
  - `code_scan` covers the following:
    - **L024:** string literals that match the ID grammar but are not in `en`, and `ids::<ns>::<CONST>` references to missing messages.
    - **L025:** English IDs with no reference.
    - **I18N-23 rule 1:** `format!("{} {}", …t(…), …)`-shaped concatenation.
    - **Hard-coded UI literals:** a string literal with an alphabetic character passed to `.child(`, `Label::new(` with `TextSource::Fluent`, or `SharedString::from(`, in `crates/ui/hacivat-ui/src` and `crates/app/*/src`, unless the literal is listed in `literal-allowlist.toml`. Reported as rule `L900`, error.

- [ ] **Step 1: Write the failing tests** (the normative I18N-17 rows as fixtures).
```rust
#[test] fn rules_fire_on_fixtures() {
    for (fixture, rule) in [("typo-var", "L011"), ("obsolete", "L008"), ("missing", "L009"), ("tr-few", "L013"), ("rlo", "L018"),
        ("dropped-code-tag", "L017"), ("undocumented-var", "L006"), ("bad-id", "L004"), ("tr_TR-folder", "L001"),
        ("nfd", "L002"), ("pseudo-folder", "L029"), ("junk", "L003"), ("dup-across-domains", "L005"), ("suffix-after-placeable", "L021")] {
        assert!(rules_of(&check(&fx(fixture), &[])).contains(&rule), "{fixture} → {rule}");
    }
}
#[test] fn clean_fixture_exits_zero() { assert!(errors(&check(&fx("clean"), &[])).is_empty()); }
#[test] fn code_scan_flags_unknown_id_unused_id_concat_and_literal() {
    let f = check(&fx("clean"), &[&fx("code")]);
    for r in ["L024", "L025", "L900"] { assert!(rules_of(&f).contains(&r), "{r}"); }
    assert!(f.iter().any(|x| x.rule == "L900" && x.id.contains("format!")));
}
#[test] fn repo_catalogs_are_clean() { assert!(errors(&check(&repo_root().join("locales"), &repo_code_roots())).is_empty()); }
```
- [ ] **Step 2:** Run `cargo test -p xtask l10n`. Expected: FAIL.
- [ ] **Step 3: Implement the rules and the scanner.** The code scanner is a token scan; `syn` is not needed. L013 reads CLDR cardinal categories from `icu_plurals`.
- [ ] **Step 4:** Run `cargo test -p xtask l10n && cargo xtask l10n check`. Expected: PASS, and `l10n-check: 0 errors`.
- [ ] **Step 5: Commit** with message `feat(xtask): l10n check (I18N-17 subset) and hard-coded string lint`.

### Task 11: `hacivat-ui` text and input: provider, Label registry, TextInput (IME), Palette view

**Files:**
- Create: `crates/ui/hacivat-ui/src/{l10n.rs, text.rs, input/{mod.rs, ime.rs}, palette.rs}`, `crates/ui/hacivat-ui/tests/{text.rs, input.rs, palette.rs}`
- Modify: `locales/{en,tr}/hui.ftl` (append)

**Interfaces:**
- Consumes: `Theme`, `font_for`, `small_caps` (ui-core) and the Task 1 types.
- Produces:
  - `EnglishDefaults: UiTextProvider` parses the embedded `locales/en/hui.ftl` once, with no ICU dependency. Unknown IDs render `⟦id⟧` in debug builds.
  - Text registry:
    - `RenderedText { text, source: TextSource, size_px: f32, clipped: bool, truncated: bool, path: SharedString }`
    - `TextRegistry { pub enabled: bool, pub frame: Vec<RenderedText> }`, a `Global`
    - `Label` becomes a custom `Element`. It shapes with the window text system and ellipsizes with `…` when `truncate`. It records into the registry when enabled, with `clipped = !truncate && shaped_width > bounds.width + 0.5`. A truncated label carries its full text as tooltip. A `Caps::SynthesizedSmallCaps` style routes through `fonts::small_caps`.
  - IME:
    - `ime::ImeState`, a pure model: `new(text)`, `replace(range_utf16: Option<Range<usize>>, text)`, `replace_and_mark(range, text, sel)`, `unmark()`, `marked_utf16()`, `selection_utf16()`, `text()`, `backspace()` (one grapheme), `move_left()`, `move_right()`, `select_all()`. The caret starts at the end of the text (M0 ruling R4).
  - `TextInput`, an entity with `new(cx, placeholder_id: &'static str)`:
    - `value() -> &str`, `set_value(&mut self, s)`
    - `is_composing() -> bool`
    - `impl EntityInputHandler`, with `bounds_for_range` taken from the shaped line
    - `on_blur` calls `unmark()`, which commits
    - emits `InputEvent::{Changed, Submitted, Cancelled}`
    - 26 px tall, radius 4, seam-strong outline, blue focus outline
  - Palette:
    - `PaletteItem { id: SharedString, title: SharedString, keys: Option<SharedString>, highlight: Vec<Range<usize>> }`
    - `Palette`, an entity holding a `TextInput` plus the result list. Rows are 23 px with the active row in `blue_tint_strong`; there is a 24 px footer with key hints. The popover surface is raised, radius 6, with `popover_shadow`.
    - `Palette::set_items(&mut self, items: Vec<PaletteItem>)`, `query() -> String`
    - emits `PaletteEvent::{QueryChanged(String), Confirmed(SharedString), Dismissed}`
    - Up/Down/Enter/Escape. The app does the ranking.
  - `hui.ftl` (EN): `hui-palette-no-results = No matching commands`, `hui-input-clear = Clear`.

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn english_defaults_render_hui() { assert_eq!(EnglishDefaults::new().text("hui-palette-no-results", &[]), "No matching commands"); }
#[test] fn replace_and_mark_sets_marked_range() { let mut s = ImeState::new("SELECT "); s.replace_and_mark(None, "ç", None); assert_eq!(s.marked_utf16(), Some(7..8)); }
#[test] fn unmark_commits_marked_text() { let mut s = ImeState::new("x"); s.replace_and_mark(None, "ş", None); s.unmark(); assert_eq!((s.text(), s.marked_utf16()), ("xş", None)); }
#[test] fn composition_replaced_by_commit() { let mut s = ImeState::new(""); s.replace_and_mark(None, "k", None); s.replace_and_mark(None, "か", None); s.replace(None, "感"); assert_eq!(s.text(), "感"); }
#[test] fn utf16_with_turkish_and_emoji() { let s = ImeState::new("İı😀x"); assert_eq!(s.selection_utf16(), 5..5); }
#[test] fn backspace_removes_one_grapheme() { let mut s = ImeState::new("ai\u{0307}"); s.backspace(); assert_eq!(s.text(), "a"); }
#[gpui::test] fn typing_turkish_through_input_handler(cx: &mut TestAppContext) { let (input, vcx) = focused_input(cx); vcx.simulate_input("ğüşıöçİ"); input.read_with(vcx, |i, _| assert_eq!(i.value(), "ğüşıöçİ")); }
#[gpui::test] fn blur_commits_composition(cx: &mut TestAppContext) { /* replace_and_mark "ş" via handler, focus another handle, run_until_parked → value "ş", not composing */ }
#[gpui::test] fn clipped_label_is_recorded(cx: &mut TestAppContext) { let reg = draw_label_in(cx, "A very long label", 40.0, false); assert!(reg.frame[0].clipped); let reg = draw_label_in(cx, "A very long label", 40.0, true); assert!(reg.frame[0].truncated && !reg.frame[0].clipped); }
#[gpui::test] fn palette_keyboard_navigation(cx: &mut TestAppContext) { /* 3 items, "down down enter" → Confirmed(id of item 2); "escape" → Dismissed */ }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-ui --test text --test input --test palette`. Expected: FAIL.
- [ ] **Step 3: Implement.** Re-implement the M0 `ImeState` ideas (do not depend on `spikes/`).
- [ ] **Step 4:** Run the same tests. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(ui): UI text provider, Label registry, IME-correct TextInput, palette view`.

### Task 12: `hacivat-ui` chrome and containers

**Files:**
- Create: `crates/ui/hacivat-ui/src/{button,segmented,icon,popover,panes,dialog,toast,menu,accesskey,statusbar,capsule,titlebar}.rs`, `crates/ui/hacivat-ui/tests/{panes.rs, chrome.rs, dialog.rs, accesskey.rs}`
- Modify: `locales/{en,tr}/hui.ftl` (append)

**Interfaces:**
- Consumes: `Label`, `TextSource`, `ui_text` (Task 11); `Theme`, `small_caps`, `BundledAssets`, `ChromeKind`, `TRAFFIC_LIGHTS_INSET_PX` (Tasks 1 and 4).
- Produces (metrics per Global Constraints and DESIGN.md Components):
  - `Button::new(id, label: SharedString)`, with `.kind(ButtonKind::{Default, Primary, Ghost, Danger, Icon})`, `.icon(IconName)`, `.key_hint(SharedString)`, `.disabled(bool)` and `.on_click(..)`
  - `Segmented::new(id).item(SegmentItem { content: SegmentContent::{Label(SharedString), Icon(IconName)}, pressed: bool, tooltip: SharedString, on_click })`
  - `Icon::new(IconName).size(IconSize::{Chrome /*12*/, Default /*14*/})`
  - `popover_surface() -> Div`: raised, radius 6, seam-strong border, `popover_shadow`
  - Panes:
    - `PaneId(u32)`, `TabId(u32)`, `PaneNode { Leaf(PaneId), Split { axis: Axis, ratio: f32, a: Box<PaneNode>, b: Box<PaneNode> } }`
    - `Pane { tabs: Vec<TabId>, active: usize }`
    - `PaneTree { root: PaneNode, panes: BTreeMap<PaneId, Pane>, focused: PaneId }`, serde-serializable
    - Methods: `new(first_tab)`, `split(pane, axis, new_tab) -> PaneId`, `close_tab(pane, tab) -> Option<TabId>` (the newly active tab), `move_focus(dir: FocusDir)`, `set_ratio(path: &[bool], ratio)` (clamped to [0.1, 0.9]; the view additionally keeps each side ≥ 120 px), `activate(pane, idx)`, `next_tab()`, `prev_tab()`
  - `TabStrip::new(pane, tabs: Vec<TabSpec { id, title: SharedString, title_source: TextSource, dirty: bool, env: Option<EnvMark> }>)`, with 30 px tabs; the active tab is raised
  - Dialogs:
    - `DialogSpec { title: SharedString, body: SharedString, buttons: Vec<DialogButton { label, kind, accesskey: Option<char>, english_accesskey: Option<char>, on_press }>, cancel: usize, primary: usize }`
    - `Dialog::open(window, cx, spec)`, a modal layer: focus trap, Esc → cancel, Enter → primary, Alt+access key on Windows/Linux
  - Toasts:
    - `Toast { text: Rc<dyn Fn(&App) -> SharedString>, kind: ToastKind::{Info, Error}, ttl: Duration }`
    - `ToastQueue`, a `Global`, with `push(cx, toast)`: at most 3 visible, bottom right above the status bar, 6 s TTL, no animation
  - Menus:
    - `MenuItemSpec { label, keys: Option<SharedString>, accesskey: Option<char>, disabled: bool, action: SharedString }` plus separators
    - `Menu::open(window, cx, anchor, items)`, with arrow, Enter and Esc navigation
  - `AccessKeyAssigner::assign(labels: &[(&str, Option<char>, Option<char>)]) -> Vec<AccessKey::{InLabel(usize), Suffix(char), None}>` (I18N-23 rule 5)
  - `StatusBar::new().left(seg).right(seg)`: 24 px on ground, 12 px `text_2`, seams between segments
  - Capsule:
    - `CapsuleModel { Placeholder { label: SharedString }, Connection { env: EnvKind, env_label: SharedString, name: SharedString, engine: SharedString, access: SharedString, path: SharedString } }`
    - `EnvKind { Production, Staging, Local, Other }`
    - `Capsule::new(model)`: 26 px, radius 4. The production block is the only `cp_fill` quad; its `capsule_env` small caps come from a pre-cased label.
  - Title bar:
    - `TitleBar::new(chrome: ChromeKind)` with `.brand(word_caps: SharedString)`, `.capsule(Capsule)`, `.palette_field(placeholder: SharedString, keys: SharedString, on_click)`, `.right(Vec<AnyElement>)` and `.app_menu(on_click)` (Windows/Linux)
    - 40 px tall. Draggable via `start_window_move`; double-click zooms. Windows caption buttons use `WindowControlArea`.
  - `titlebar_fit(width: f32, fixed: f32, palette_min: f32, segments: f32) -> TitleBarFit { segments_in_overflow: bool, palette_width: f32 }`
  - `hui.ftl` (EN): `hui-dialog-close = Close`, `hui-toast-dismiss = Dismiss`, `hui-menu-more = More`, `hui-tab-close = Close tab`, `hui-window-minimize = Minimize`, `hui-window-maximize = Maximize`, `hui-window-close = Close window`.

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn split_then_close_collapses() { let mut t = PaneTree::new(TabId(1)); let p2 = t.split(PaneId(0), Axis::Horizontal, TabId(2)); assert_eq!(t.close_tab(p2, TabId(2)), None); assert_eq!(t.root, PaneNode::Leaf(PaneId(0))); }
#[test] fn closing_active_tab_activates_left_neighbour() { let mut t = tree_with_tabs(&[1, 2, 3], 2); assert_eq!(t.close_tab(PaneId(0), TabId(3)), Some(TabId(2))); }
#[test] fn pane_tree_round_trips_json() { let t = sample_tree(); assert_eq!(serde_json::from_value::<PaneTree>(serde_json::to_value(&t).unwrap()).unwrap(), t); }
#[test] fn ratio_is_clamped() { let mut t = sample_tree(); t.set_ratio(&[], 0.01); assert!(ratio(&t) >= 0.1); }
#[test] fn titlebar_collapses_segments_before_truncating_palette() { let f = titlebar_fit(1024.0, 600.0, 220.0, 180.0); assert!(f.segments_in_overflow); assert!(f.palette_width >= 220.0); let f = titlebar_fit(1440.0, 600.0, 220.0, 180.0); assert!(!f.segments_in_overflow); }
#[test] fn access_keys_turkish_menu() { assert_eq!(keys(&[("Kaydet", None, None), ("Kapat", None, None), ("Kopyala", None, None)]), ['K', 'a', 'o']); assert_eq!(keys(&[("Kaydet", None, None), ("Kopyala", Some('p'), None)]), ['K', 'p']); }
#[test] fn access_key_suffix_when_no_letters() { assert_eq!(AccessKeyAssigner::assign(&[("保存", None, Some('S'))]), vec![AccessKey::Suffix('S')]); }
#[gpui::test] fn dialog_traps_focus_and_escape_cancels(cx: &mut TestAppContext) { /* 2 buttons; "tab tab tab" keeps focus inside; "escape" → cancel callback ran once; focus back on the opener */ }
#[gpui::test] fn status_bar_and_tabs_use_tokens(cx: &mut TestAppContext) { /* status bar height 24, tab height 30 via layout bounds */ }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-ui --test panes --test chrome --test dialog --test accesskey`. Expected: FAIL. (The capsule's single `cp-fill` quad is asserted in Task 13, once Task 8's counter has merged.)
- [ ] **Step 3: Implement.** Every string comes in as a parameter or a `hui-*` ID; none is literal (Task 10's L900 lint applies).
- [ ] **Step 4:** Run the same tests. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(ui): buttons, segmented, panes/tabs, dialog, toast, menu, access keys, status bar, capsule, title bar`.

### Task 13: App shell and the `hacivat` binary

**Files:**
- Create: `crates/app/hacivat-app/src/{l10n_global.rs, theme_pref.rs, shell/{mod,workspace,tiles,tabs}.rs}`, `crates/app/hacivat-app/tests/shell.rs`, `crates/bin/hacivat/src/{main.rs, logging.rs, smoke.rs}`
- Modify: `locales/{en,tr}/hacivat-ui.ftl` (append)

**Interfaces:**
- Consumes: Tasks 4 and 6–12.
- Produces:
  - L10n global:
    - `L10nGlobal { pub l10n: Arc<L10n>, pub catalogs: Arc<Catalogs> }`, a `Global`
    - `install_l10n(cx: &mut App, l10n: Arc<L10n>, catalogs: Arc<Catalogs>)`, which also installs `UiText(Arc::new(L10nUiProvider))` resolving `hui-*` through the chain
    - `switch_locale(cx: &mut App, settings: &Settings) -> Task<()>`: builds the runtime on `cx.background_spawn`, then `install_if_newer`, `cx.refresh_windows()`, re-sets window titles and calls `menus::rebuild(cx)` (a stub until Task 16)
  - Theme:
    - `theme_mode_for(pref: ThemePref, appearance: WindowAppearance) -> ThemeMode`
    - `apply_theme(cx: &mut App, pref: ThemePref)`, which observes the appearance for `System`
  - `AppState { paths: LocalPaths, settings: Settings }`, a `Global`
  - `open_workspace(cx: &mut App, demo: Option<DemoCapsule>) -> Result<WindowHandle<Workspace>>`
  - `Workspace` (`Render`), which owns `TileLayout { explorer_w: 276.0, conn_panel_h: 205.0, editor_h: 322.0, inspector_w: 252.0, explorer_visible: true }`, `PaneTree`, `tabs: BTreeMap<TabId, TabKind::{Console { n: u32 }, Settings}>` and `CapsuleModel`. Seams can be dragged; the layout is saved through `workspace_state::save` (version 1) on change, debounced to 500 ms on the background executor.
  - The shell is laid out like harman:
    - **Title bar:** brand `-brand-hacivat-caps`, capsule, palette field (`hv-titlebar-palette`, keys from the keymap), and on the right the language segmented control (catalog `short_name`s) and the theme segmented control (moon/sun).
    - **Tiles:** explorer and connection panel with 30 px headers and empty states; the editor area with the tab strip and an empty state; the results pane with an empty state.
    - **Status bar:** left "No connection" plus the `PendingChord` segment; right a language button `EN / TR`.
  - Bin:
    - `main`: `LocalPaths::from_os` → `ensure_dirs` → `settings::load` → `logging::init(paths.logs_dir())` (JSON lines, daily rotation, keep 7 files) → `Catalogs::embedded().with_overlay(..)` → initial `L10n` built synchronously → `run_app(Boot { text_rendering }, ..)`. A quarantined settings file shows an error toast (`hv-settings-file-quarantined`).
    - Flags: `--smoke` prints `first-frame-ms=<n>` at the first `on_next_frame`, then `idle-rss-mib=<n>` 5 s later, and exits 0. `--vf-demo` shows the harman production capsule. `--print-window-id` prints `platform::native_window_id` and `WINDOW_TITLE=<title>` for `design/vf/capture.sh app`.
  - FTL (EN / TR):

    | ID | EN | TR |
    |---|---|---|
    | `hv-titlebar-palette` | Search or run a command | Ara veya komut çalıştır |
    | `hv-capsule-no-connection` | No connection | Bağlantı yok |
    | `hv-capsule-env-production` | PRODUCTION | PROD |
    | `hv-capsule-access-read-only` | read-only | salt okunur |
    | `hv-capsule-path-direct` | Direct | Doğrudan |
    | `hv-panel-explorer` | EXPLORER | GEZGİN |
    | `hv-panel-connection` | CONNECTION | BAĞLANTI |
    | `hv-explorer-filter` | Filter connections | Bağlantılarda ara |
    | `hv-explorer-empty` | No connections yet | Henüz bağlantı yok |
    | `hv-tab-console-untitled` | Console { $n } | Konsol { $n } |
    | `hv-editor-empty` | Connect to a database to start writing SQL | — |
    | `hv-results-empty` | Run a query to see results here | — |
    | `hv-status-no-connection` | No connection | Bağlantı yok |
    | `hv-status-pending-chord` | Waiting for the next key: { $chord } | — |
    | `hv-theme-dark` | Dark | Koyu |
    | `hv-theme-light` | Light | Açık |
    | `hv-settings-file-quarantined` | Your settings file could not be read. It was saved as { $file } and defaults are in use. | — |

    TR drafts not in harman get a `# Review: PO` comment. Missing TR entries fall back to EN.

- [ ] **Step 1: Write the failing tests** (`tests/shell.rs`, VisualTestContext with `scene-check`).
```rust
#[gpui::test] fn switching_language_rerenders_without_restart(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en"); assert_eq!(palette_label(&vcx), "Search or run a command");
    switch_to(&vcx, "tr"); vcx.run_until_parked(); assert_eq!(palette_label(&vcx), "Ara veya komut çalıştır"); assert_eq!(panel_title(&vcx), "GEZGİN");
}
#[gpui::test] fn two_rapid_switches_last_wins(cx: &mut TestAppContext) { let vcx = shell(cx, "en"); switch_to(&vcx, "tr"); switch_to(&vcx, "en"); vcx.run_until_parked(); assert_eq!(palette_label(&vcx), "Search or run a command"); }
#[gpui::test] fn toast_rerenders_after_language_switch(cx: &mut TestAppContext) { /* push toast from hv-status-no-connection, switch to tr → toast text "Bağlantı yok" */ }
#[gpui::test] fn theme_switch_is_live(cx: &mut TestAppContext) { let vcx = shell(cx, "en"); apply(&vcx, ThemePref::Light); vcx.run_until_parked(); assert_eq!(root_bg(&vcx), LIGHT.colors.ground.to_hsla()); }
#[test] fn system_theme_follows_appearance() { assert_eq!(theme_mode_for(ThemePref::System, WindowAppearance::VibrantLight), ThemeMode::Light); assert_eq!(theme_mode_for(ThemePref::Dark, WindowAppearance::Light), ThemeMode::Dark); }
#[gpui::test] fn one_filled_badge_holds(cx: &mut TestAppContext) { let vcx = shell_demo(cx); vcx.update(|w, _| assert_one_filled_badge(w, DARK.colors.cp_fill.to_hsla(), true)); let vcx = shell(cx, "en"); vcx.update(|w, _| assert_one_filled_badge(w, DARK.colors.cp_fill.to_hsla(), false)); }
#[gpui::test] fn layout_round_trips_through_workspace_json(cx: &mut TestAppContext) { /* split right, new tab, save; reopen from the same LocalPaths::under(tmp) → identical PaneTree and TileLayout */ }
#[gpui::test] fn corrupt_overlay_keeps_language_and_reports_diag(cx: &mut TestAppContext) { /* overlay tr file with Junk → language still tr for valid ids; L10n runtime diags count 1 */ }
#[gpui::test] fn every_label_in_shell_is_from_fluent_or_exempt(cx: &mut TestAppContext) { /* en-XA with registry enabled → every TextSource::Fluent label starts with '[' and ends with ']' */ }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-app --features scene-check,pseudo --test shell`. Expected: FAIL.
- [ ] **Step 3: Implement `l10n_global`, `theme_pref` and the shell.** Components store IDs plus arguments, or closures, never rendered text (I18N-07 step 4).
- [ ] **Step 4: Implement the bin.**
- [ ] **Step 5:** Run the tests, then `cargo run -p hacivat --release -- --smoke`. Expected: PASS, and output lines `first-frame-ms=<n>` and `idle-rss-mib=<n>` with exit 0.
- [ ] **Step 6: Visual check.** Run `cargo run -p hacivat -- --vf-demo` and compare it side by side with harman (title bar, tiles, status bar) against DESIGN.md Layout/Components. Fix any deviation; the formal VF numbers come in Task 18.
- [ ] **Step 7: Commit** with message `feat(app): harman shell, runtime language/theme switch, layout restore; hacivat binary`.

### Task 14: CI, nightly packaging, passworded update key

**Files:**
- Move: `.github/workflows/m0.yml` → `.github/workflows/ci.yml`
- Modify: `packaging/{Packager.toml, package-macos.sh, package-linux.sh, package-windows.ps1, make-manifest.py, keys/README.md}`
- Create: `packaging/common.sh`, `xtask/src/update_key.rs`
- Modify: `xtask/src/main.rs` (arm `check-update-key`)

**Interfaces:**
- Produces:
  - `xtask check-update-key <secret-key-file>` exits 1 unless the minisign secret key is encrypted.
    - `pub fn key_is_encrypted(sk_file: &str) -> Result<bool>`: base64-decode line 2 and check that bytes 2..4 (the KDF algorithm) equal `b"Sc"`. `[0, 0]` means unencrypted.
  - CI jobs (pinned toolchain, `--locked`):
    - `build-test` (3 OSes; adds `cargo clippy --workspace --all-targets -- -D warnings`)
    - `l10n-check` (`cargo xtask l10n check`)
    - `turkey-test` (Ubuntu; `locale-gen tr_TR.UTF-8`; `LANG=tr_TR.UTF-8 LC_ALL=tr_TR.UTF-8 cargo test --workspace`)
    - `launch-smoke` (3 OSes; `cargo run -p hacivat --release -- --smoke`, under `xvfb-run -a` on Linux; asserts exit 0 and a `first-frame-ms=` line)
    - `deny` (licenses, bans, sources)
    - `tokens`, `check-deps`
    - Nightly `bench` (unchanged spike benches)
    - Nightly `nightly-package` (3 OSes):
      - macOS: universal, signed if `APPLE_CERT_P12_BASE64` is set, notarized if `APPLE_API_KEY_P8_BASE64` is set (`notarytool --key/--key-id/--issuer`), else a warning annotation.
      - Windows: unsigned NSIS.
      - Linux: AppImage and `.deb`.
      - Every artifact plus the update manifest is minisigned with `MINISIGN_SECRET_KEY` and `MINISIGN_PASSWORD`, after `check-update-key` passes.
    - Nightly `pseudo-screens` (macOS + Ubuntu; runs Task 18's harness and uploads `target/shots/**`). Until Task 18 merges, it is guarded with `if: hashFiles('crates/app/hacivat-app/tests/shots.rs') != ''`.
  - Packager config:
    - `name = "hacivat"`, `productName = "Hacivat"`, `identifier = "dev.hacivat.Hacivat"`, `version = "@VERSION@"` (nightly `0.1.0-nightly.<yyyymmdd>`), `minimumSystemVersion = "13.0"`.
    - Windows and Linux render their own configs without the signing placeholder.
    - The DMG is named `Hacivat_<version>_universal.dmg`.
  - `packaging/common.sh` defines the key path once, a truthy parser for `REQUIRE_NOTARIZATION` (`1|true|yes`), and probe output kept on failure.

- [ ] **Step 0 (PO):**
  - Generate the passworded key: `minisign -G -p packaging/keys/update.pub -s ~/.hacivat-keys/update.key`, without `-W`.
  - Add the repository secrets `MINISIGN_SECRET_KEY` (the file content) and `MINISIGN_PASSWORD`.
  - Optionally add `APPLE_CERT_P12_BASE64`, `APPLE_CERT_PASSWORD`, `APPLE_API_KEY_P8_BASE64`, `APPLE_API_KEY_ID`, `APPLE_API_ISSUER`.
  - Until the key exists, nightly packaging builds but signs nothing and fails the job.
- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn unencrypted_key_is_refused() { assert!(!key_is_encrypted(&sk_fixture(*b"Ed", [0, 0])).unwrap()); }
#[test] fn scrypt_key_is_accepted() { assert!(key_is_encrypted(&sk_fixture(*b"Ed", *b"Sc")).unwrap()); }
#[test] fn garbage_is_an_error() { assert!(key_is_encrypted("untrusted comment: x\n!!!\n").is_err()); }
```
- [ ] **Step 2:** Run `cargo test -p xtask update_key`. Expected: FAIL.
- [ ] **Step 3: Implement the subcommand, the workflow and the packaging changes** (deferred minors: Task 10 ×2, Task 10 REQUIRE_NOTARIZATION/probe).
- [ ] **Step 4:** Run `cargo test -p xtask update_key && actionlint .github/workflows/ci.yml && bash -n packaging/*.sh`. Locally, run `packaging/package-macos.sh 0.1.0-m1`. Expected: tests PASS, actionlint finds nothing, and a signed DMG is produced. With `RELEASE=1 NOTARY_PROFILE=<profile>`, `spctl -a -vvv -t install` reports `source=Notarized Developer ID`.
- [ ] **Step 5:** After the controller pushes the branch, run `gh workflow run ci.yml --ref m1-foundations`. Expected: every PR job is green, and `launch-smoke` passes on all three OSes.
- [ ] **Step 6: Commit** with message `ci(m1): clippy, l10n-check, turkey-test, launch smoke, nightly signed packages; passworded update key`.

### Task 15: Carry the GPUI IME patch on a zed fork (R1)

**Files:**
- Modify: `Cargo.toml` (the gpui/gpui_platform `git`/`rev`), `deny.toml` (`allow-git` += the fork), `Cargo.lock`, `crates/ui/hacivat-ui-core/patches/README.md`
- Create (in the fork, not in this repo): branch `hacivat/912159c` with the commit "gpui_macos: discard marked text on FocusLost (Hacivat carry)", plus `HACIVAT-PATCHES.md` at the root listing the patch, its rationale and the Apache-2.0 §4(b) notice

**Interfaces:**
- Produces: `[workspace.dependencies] gpui/gpui_platform = { git = "https://github.com/berkaygkc/zed", rev = "<fork sha>" }`. `ZED_SOURCES` (Task 2) already covers the fork.

- [ ] **Step 0 (PO):** Run `gh repo fork zed-industries/zed --clone=false`, or approve the agent running it. If the fork does not exist at dispatch, mark this task deferred to M2 in `docs/benchmarks/m1.md` and stop.
- [ ] **Step 1:** Create the branch at `912159c`, then apply the carried patch with `git am` or `git apply` plus a commit. Check that `LICENSE-APACHE` is untouched and that `crates/gpui_macos/src/window.rs` carries the "modified by Hacivat" header. Push.
- [ ] **Step 2:** Switch the workspace pin, then run `cargo update -p gpui -p gpui_platform`, `cargo build --workspace`, `cargo test --workspace`, `cargo deny check licenses bans sources` and `cargo xtask check-deps`. Expected: all pass, and the `Cargo.lock` diff touches only zed-sourced packages, whose source becomes the fork.
- [ ] **Step 3:** Update `patches/README.md`: the patch is applied via the fork at `<sha>`, and Gap 2 (focus move while the window is inactive) stays open, assigned to M5. The manual IME check needs the palette and settings inputs from W4, so it runs in Task 19, Step 3.
- [ ] **Step 4: Commit** with message `build(gpui): carry FocusLost discardMarkedText via fork at 912159c (R1)`.

### Task 16: Commands: registry, dispatcher, palette, menus

**Files:**
- Create: `crates/app/hacivat-app/src/commands/{mod,registry,dispatch,palette}.rs`, `crates/app/hacivat-app/src/menus.rs`, `crates/app/hacivat-app/tests/commands.rs`
- Modify: `locales/{en,tr}/hacivat-ui.ftl` (append `hv-cmd-*` and `hv-menu-*`)

**Interfaces:**
- Consumes: `ActionRegistry` (Task 1), `hacivat_ui::keymaps::load` and `Keymap` (Task 5), `Palette` (Task 11), `search` (Task 9), `Workspace` (Task 13).
- Produces:
  - `register_workspace_actions(reg: &mut ActionRegistry)` registers exactly: `workspace.command_palette`, `workspace.open_settings`, `workspace.new_console_tab`, `workspace.close_tab`, `workspace.next_tab`, `workspace.prev_tab`, `workspace.split_right`, `workspace.split_down`, `workspace.focus_next_pane`, `workspace.toggle_explorer`, `workspace.toggle_theme`, `workspace.next_language`, `workspace.reload_translations`, `app.about`, `app.restart`, `app.quit`. Each `title_id` is `hv-cmd-<id with . and _ → ->`, for example `hv-cmd-workspace-open-settings = Open settings` / `Ayarları aç`.
  - Dispatcher:
    - `Dispatcher { keymap: Keymap, pending: Vec<KeyPress>, since: Option<Instant>, layout: LayoutId }`
    - `on_key_down(&mut self, ev: &KeyDownEvent, ctx: &ContextStack, composing: bool, now: Instant) -> DispatchOutcome::{Run(String), Pending(SharedString), PassThrough}`
    - The pending state expires after 1,000 ms.
    - It is wired into the workspace root's `capture_key_down`, updates the `PendingChord` global, and re-reads the layout through `cx.on_keyboard_layout_change`.
  - Debug builds only: `HACIVAT_DEBUG_FOCUS_MOVE_MS=<n>` moves focus to the palette input `n` ms after the first IME `replace_and_mark` in any `TextInput`. This is the programmatic focus move for the IME check in Task 19.
  - `CommandEntry { id: &'static str, title: SharedString /* l10n.t_dynamic(title_id) */, english_title: SharedString /* l10n.t_english(title_id) */, keys: Option<SharedString> }`
  - `rank_commands(query: &str, entries: &[CommandEntry], recent: &[String], opts: &SearchOpts) -> Vec<PaletteItem>` (§8.13):
    - Match the localized title, and the English title as a second layer.
    - Order: ExactWord > Prefix > Substring > Fuzzy > English-only.
    - Ties: recent first (most recent first), then registry order.
    - An empty query lists every enabled action, recent first.
  - `menus::build_menus(reg: &ActionRegistry, cx: &App) -> Vec<Menu>` (macOS `set_menus`; also rebuilt by `switch_locale`), with `hv-menu-*` names. Menu commands' chords are bound as GPUI `KeyBinding`s under context `HacivatMenu` (R7).

- [ ] **Step 1: Write the failing tests.**
```rust
#[gpui::test] fn primary_k_opens_palette_with_every_action(cx: &mut TestAppContext) { let vcx = shell(cx, "en"); vcx.simulate_keystrokes(primary("k")); assert_eq!(palette_items(&vcx).len(), registry_len(&vcx)); assert!(palette_item(&vcx, "workspace.open_settings").keys.is_some()); }
#[gpui::test] fn altgr_at_sign_reaches_palette_input(cx: &mut TestAppContext) { /* palette open; KeyDown{ctrl+alt, key "q", key_char "@"} with windows keymap + tr-q layout → no action ran; then simulate_input("@") → query "@" */ }
#[gpui::test] fn bindings_do_not_fire_while_composing(cx: &mut TestAppContext) { /* settings search composing (replace_and_mark) → primary-w does not close the tab */ }
#[gpui::test] fn switch_language_with_palette_open_keeps_query(cx: &mut TestAppContext) { let vcx = shell(cx, "tr"); open_palette(&vcx); vcx.simulate_input("ayar"); switch_to(&vcx, "en"); vcx.run_until_parked(); assert_eq!(palette_query(&vcx), "ayar"); assert_eq!(palette_item(&vcx, "workspace.open_settings").title, "Open settings"); }
#[gpui::test] fn escape_closes_only_topmost_layer_and_restores_focus(cx: &mut TestAppContext) { /* open settings tab, focus search; open palette; "escape" → palette closed, focused == settings search; settings tab still open */ }
#[test] fn ranking_prefers_word_match_then_english_layer() { let r = rank_commands("ayar", &fixture_items_tr(), &[], &tol()); assert_eq!(r[0].id, "workspace.open_settings"); let r = rank_commands("settings", &fixture_items_tr(), &[], &tol()); assert_eq!(r[0].id, "workspace.open_settings"); let r = rank_commands("AYARLARI", &fixture_items_tr(), &[], &tol()); assert_eq!(r[0].id, "workspace.open_settings"); }
#[test] fn recent_breaks_ties() { let r = rank_commands("", &fixture_items_en(), &["app.about".into()], &off()); assert_eq!(r[0].id, "app.about"); }
#[test] fn every_action_title_exists_in_en() { let (reg, l) = (registry(), l10n_en()); for d in reg.all() { assert!(!l.t_english(d.title_id).starts_with('⟦'), "{}", d.title_id); } }
#[test] fn keymap_commands_are_registered() { let reg = registry(); for p in ALL_PLATFORMS { for b in load(p, "hacivat", None).0.bindings() { assert!(reg.get(&b.command).is_some(), "{}", b.command); } } }
#[gpui::test] fn menus_follow_language(cx: &mut TestAppContext) { /* build_menus after switch to tr → View menu name == "Görünüm" */ }
#[gpui::test] fn pending_chord_shows_in_status_bar(cx: &mut TestAppContext) { /* user keymap "ctrl-k ctrl-s" → after ctrl-k the PendingChord global is Some("Ctrl+K") / "⌃K"; after 1,000 ms (advance_clock) it is None */ }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-app --test commands`. Expected: FAIL.
- [ ] **Step 3: Implement the registry population, the dispatcher, the palette glue and the menus.**
- [ ] **Step 4:** Run `cargo test -p hacivat-app --test commands && cargo xtask l10n check`. Expected: PASS, and `0 errors`.
- [ ] **Step 5: Visual check.** Open the palette in en and tr in both themes and compare against DESIGN.md (raised surface, 6 px radius, popover shadow, 23 px rows, active `blue_tint_strong`, `text_2` metadata on raised).
- [ ] **Step 6: Commit** with message `feat(app): action registry, UI-EDIT-13 dispatcher, command palette, localized menus`.

### Task 17: Settings UI

**Files:**
- Create: `crates/app/hacivat-app/src/settings_ui/{mod,rows,search}.rs`, `crates/app/hacivat-app/tests/settings_ui.rs`
- Modify: `locales/{en,tr}/hacivat-ui.ftl` (append `hv-settings-*`)

**Interfaces:**
- Consumes: `save_changes` (Task 7), `switch_locale` and `apply_theme` (Task 13), `effective` and `search` (Task 9), `TEXT_RENDERING_IS_LIVE` and `app_font_smoothing_override` (Task 4), `Dialog` and `Segmented` (Task 12), `TextInput` (Task 11).
- Produces:
  - `SettingsPage`, an entity shown as `TabKind::Settings`. It has a left nav (General, Appearance, Keyboard) and a search field (I18N-09 with the English layer, as in ALG-L10N-07).
  - Rows:
    - Language: System plus catalog native names (`English`, `Türkçe`). Live.
    - Regional format: System, `en-US`, `tr-TR`. Live, with a preview line `NUMBER(1234.5) · DATETIME(2026-11-11T14:05, short date + short time)`.
    - Theme: Dark / Light / System. Live.
    - Text rendering: Crisp (default) / macOS standard. macOS only. Saved immediately; shows "Applies after restart" and a "Restart now" button, which opens a dialog and then calls `cx.restart()`.
    - Keymap preset: Hacivat / DataGrip. Live keymap reload; a warning toast lists collisions on the current layout.
  - `settings_rows() -> Vec<SettingRow { key: &'static str, title: MsgRef<NoArgs> }>`; the English layer is `l10n.t_english(row.title.id)`
  - `filter_rows(query, rows, opts) -> Vec<&'static str>`
  - Every change goes through `save_changes(paths.settings_toml(), ..)`, and the I/O runs on the background executor.

- [ ] **Step 1: Write the failing tests.**
```rust
#[gpui::test] fn theme_change_persists_and_applies(cx: &mut TestAppContext) { let (vcx, paths) = settings_tab(cx); click_segment(&vcx, "ui.theme", "light"); vcx.run_until_parked(); assert_eq!(Theme::get_mode(&vcx), ThemeMode::Light); assert!(read(paths.settings_toml()).contains("theme = \"light\"")); }
#[gpui::test] fn language_change_is_live(cx: &mut TestAppContext) { let (vcx, _) = settings_tab(cx); choose(&vcx, "ui.language", "tr"); vcx.run_until_parked(); assert_eq!(nav_label(&vcx, 0), "Genel"); assert_eq!(tab_title(&vcx), "Ayarlar"); }
#[cfg(target_os = "macos")] #[gpui::test] fn text_rendering_change_needs_restart(cx: &mut TestAppContext) { let (vcx, paths) = settings_tab(cx); click_segment(&vcx, "ui.text_rendering", "standard"); vcx.run_until_parked(); assert!(read(paths.settings_toml()).contains("text_rendering = \"standard\"")); assert!(restart_note_visible(&vcx)); assert_eq!(app_font_smoothing_override(), Some(0)); }
#[cfg(not(target_os = "macos"))] #[gpui::test] fn text_rendering_row_hidden(cx: &mut TestAppContext) { assert!(!row_visible(&settings_tab(cx).0, "ui.text_rendering")); }
#[test] fn search_matches_english_layer_in_turkish() { assert_eq!(filter_rows("theme", &rows_tr(), &tol()), ["ui.theme"]); assert_eq!(filter_rows("tema", &rows_tr(), &tol()), ["ui.theme"]); assert_eq!(filter_rows("dil", &rows_tr(), &tol()), ["ui.language"]); }
#[gpui::test] fn regional_preview_is_turkish(cx: &mut TestAppContext) { /* regional tr-TR, tz Europe/Istanbul → preview "1.234,5 · 11.11.2026 14:05"; en-US → starts with "1,234.5" */ }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-app --test settings_ui`. Expected: FAIL.
- [ ] **Step 3: Implement.** Fixed copy: `hv-settings-title = Settings` / `Ayarlar`, `hv-settings-nav-general = General` / `Genel`, `hv-settings-restart-note = Applies after restart` / `Yeniden başlatınca uygulanır` (`# Review: PO`). Row and control metrics follow DESIGN.md: 26 px fields and segmented controls, 22 px key/value rhythm, 12 px left padding, `text_2` labels.
- [ ] **Step 4:** Run `cargo test -p hacivat-app --test settings_ui && cargo xtask l10n check`. Expected: PASS.
- [ ] **Step 5: Visual check.** Capture the settings tab in en/tr, dark/light, and check it against DESIGN.md (no cards, seams only, 11 px floor, Raised Ink Rule).
- [ ] **Step 6: Commit** with message `feat(app): settings tab — language, regional format, theme, text rendering (restart), keymap preset`.

### Task 18: Shots harness: pseudo-screens, VF of the shell, DESIGN.md review

**Files:**
- Create: `crates/app/hacivat-app/tests/shots.rs` (feature `shots`), `design/vf/samples-m1-shell.json` (generated), `xtask/src/shot_diff.rs`
- Modify: `design/vf/gen_samples.py` (`--set m1-shell`), `design/vf/capture.sh` (kind `app`: runs `hacivat --vf-demo --print-window-id`), `design/vf/README.md`, `xtask/src/main.rs` (arm `shot-diff`)

**Interfaces:**
- Consumes: everything above. It uses `HeadlessAppContext::with_platform(gpui_platform::current_platform(true).text_system(), Arc::new(BundledAssets), gpui_platform::current_headless_renderer)` (R8) and calls `apply_text_rendering(Crisp)` first.
- Produces:
  - `cargo test -p hacivat-app --features shots --test shots -- --ignored` renders these surfaces:
    - `shell`, `shell-demo-production`, `shell-narrow` (1024×700), `palette-open`, `settings-general`, `dialog-about`, `toast-info`, `menu-app`
    - at 1280×800 (except `shell-narrow`), × locales `en`, `tr`, `en-XA`, `tr-XT`, × themes `dark`, `light`
    - and `shell-demo-production` at 1440×900, scale 2, × en/tr × dark/light, for VF
  - It writes `target/shots/<surface>-<theme>-<locale>-<w>x<h>@<s>x.png`. For every frame it asserts:
    - (a) in pseudo locales, every `TextSource::Fluent` label is bracketed;
    - (b) no label has `clipped == true`;
    - (c) every `size_px ≥ 11.0` except synthesized small-caps glyph runs;
    - (d) `assert_one_filled_badge` holds (`expect_production` only on demo surfaces).
  - On Windows it prints `shots: no headless renderer (R8)` and passes.
  - `cargo xtask shot-diff --baseline <dir> --candidate <dir> [--max-de 2.0 --max-share 0.001]` compares same-named PNGs pixel by pixel with ΔE2000 and exits 1 on a breach.

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn shot_diff_identical_is_zero() { assert_eq!(diff_share(&img(0x16181D), &img(0x16181D), 2.0), 0.0); }
#[test] fn shot_diff_counts_changed_pixels() { assert!((diff_share(&img(0x16181D), &half(0x16181D, 0xCE2C31), 2.0) - 0.5).abs() < 1e-9); }
#[test] fn m1_shell_samples_validate_against_reference() { /* python-generated JSON parses with vf::parse_samples and every sample lies in the title bar (y < 40), status bar (y ≥ 876), a panel header or a seam */ }
```
  The harness test itself, `#[ignore] fn render_all_surfaces()`, is the main check.
- [ ] **Step 2:** Run `cargo test -p xtask shot_diff vf`. Expected: FAIL.
- [ ] **Step 3: Implement `shot_diff`, the samples set, the `app` capture kind and the harness.** First confirm that a headless capture of `shell-demo-production` passes `xtask vf`'s reference self-check. If it does not, switch the VF path to `capture.sh app` (R8 fallback) and record why.
- [ ] **Step 4:** Run `cargo test -p hacivat-app --features shots --test shots -- --ignored`, then:
  ```
  cargo xtask vf --reference target/vf/ref-<theme>-<lang>-2x.png \
    --candidate target/shots/shell-demo-production-<theme>-<lang>-1440x900@2x.png \
    --samples design/vf/samples-m1-shell.json --scale 2 --theme <theme>
  ```
  Run the VF compare for each of the 4 combinations. Expected: the harness passes, and every VF sample and scanline reads `ok`.
- [ ] **Step 5: DESIGN.md review per surface.** Open each PNG (en, tr-XT, dark, light) and check it against:
  - tokens only, seams, radii, the 11 px floor, popover-only shadow, the Raised Ink Rule;
  - the Overflow Rule (`shell-narrow`);
  - focus ring visibility (`palette-open`).
  
  List the fixes and apply them in the owning code. Re-run Step 4.
- [ ] **Step 6: Commit** with message `test(shots): pseudo-screens harness, M1 shell VF samples, shot-diff`.

### Task 19: M1 gate

**Files:**
- Create: `docs/benchmarks/m1.md`, `design/baselines/m1/macos/*.png` (after the PO signs off)

- [ ] **Step 1:** On `m1-foundations`, run `cargo clean -p hacivat -p hacivat-app` (and `--release`), then:
  ```
  cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings \
    && cargo xtask check-deps && cargo xtask l10n check && cargo xtask tokens --check \
    && cargo deny check licenses bans sources
  ```
  Expected: all green.
- [ ] **Step 2:** On the PO's Mac, run `cargo run -p hacivat --release -- --smoke` 3 times. Record `first-frame-ms` (≤ 500) and `idle-rss-mib` (≤ 200 MB) in `m1.md` with the build hash.
- [ ] **Step 3: Demo for the PO (macOS):**
  - Switch EN↔TR from the title bar, the status bar, the palette and Settings, and dark/light/system, all without a restart.
  - Run the palette with Turkish Q.
  - Do the IME rows (Task 15) in the palette and the settings search: Japanese composition with Enter; Turkish Q `ğüşıöçİ`; `HACIVAT_DEBUG_FOCUS_MOVE_MS=3000` with a debug build, composing in settings search. Expected: no duplicated text and no stuck underline. Then mark Known gap 1 in `docs/benchmarks/m0-ime-checklist.md` as closed.
  - Change text rendering and restart.
  - Restore the layout after quit.
  - Install the notarized DMG (`RELEASE=1 NOTARY_PROFILE=<profile> packaging/package-macos.sh 0.1.0-m1`) and confirm `spctl` reports `accepted`.
- [ ] **Step 4: Windows and Linux.** Record the CI `launch-smoke` results and the nightly artifacts. When the PO provides the hosts, install the nightly packages there, launch them, capture `capture.ps1`/`capture.sh app` shell screens, and record them in `m1.md`. Otherwise list them as open items.
- [ ] **Step 5: PO DESIGN.md review** of every surface PNG from Task 18, plus the Turkish strings (L1). After sign-off, copy the macOS PNGs to `design/baselines/m1/macos/` (nightly `shot-diff` uses them).
- [ ] **Step 6:** Write `docs/benchmarks/m1.md`:
  - the exit criteria table (launch on all OSes, notarized macOS, runtime switch, pseudo-screens job, Fluent-only strings) with PASS/FAIL and evidence;
  - the R1–R13 outcomes;
  - the IME rows (Task 15);
  - open items with owners.
- [ ] **Step 7: Commit** with message `docs(m1): gate record, approved shell baselines`.

---

## Self-review notes

- **Spec coverage (§6 M1):**
  - Workspace and CI → Tasks 1, 2, 14. Lint → Tasks 1, 14. `cargo-deny` → Task 14. `turkey-test` → Task 14. `l10n-check` → Tasks 10, 14. Nightlies → Task 14.
  - ui-core and themes → Tasks 4, 8.
  - I18N-01..04 → Task 6. I18N-05/06 → Task 9. I18N-07 → Tasks 6 and 13. I18N-16 → Tasks 9 and 18. I18N-17 → Task 10. I18N-23 → Tasks 10 and 12. I18N-09 per R9 → Task 9.
  - Action registry, keymaps and collision CI → Tasks 5, 16; palette → Tasks 11, 16.
  - Shell → Tasks 12, 13. Toasts and dialogs → Task 12.
  - Settings → Tasks 7, 17. §5.6 paths → Task 7.
  - Exit criteria → Tasks 13, 14, 18, 19.
- **M0 carry-overs:**
  - single bootstrap → Task 4; transitive check-deps → Task 2; paint-level cp-fill → Task 8;
  - `frames.rs` gpui-free → Tasks 1, 3; text weight → Tasks 4, 17; IME patch → Task 15;
  - passworded update key → Task 14;
  - deferred minors that touch kept code → Tasks 2, 3, 4, 14 (spike-only minors stay with the spikes).
- **Not in M1 by design:**
  - single-instance lock and IPC (§5.3) → M8 with the broker;
  - the updater client and first-run flow (§4, §7.3) → M9;
  - crash reports → M9;
  - grid and connection settings keys → M2/M3;
  - the D-074 toggle UI → M4;
  - inactive-window IME blur → M5.
