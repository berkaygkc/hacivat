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
- `arc-swap` 1, `rust-embed` 8, `sys-locale`, `iana-time-zone`, `directories`, `serde`, `toml`, `toml_edit`, `serde_json`, `unicode-segmentation`
- Platform (ui-core `platform/**` only): `core-foundation`, `core-foundation-sys`, `mach2` (macOS); `windows-sys` (Windows)
- `tracing` + `tracing-subscriber` (json) + `tracing-appender`
- xtask: `toml`, `base64`
- `trybuild` and `tempfile` (dev), `cargo-packager`, `minisign`, `cargo-deny`, `actionlint`
- No crate outside this list is added without a ruling (no `sysinfo`: RSS comes from `platform::resident_set_mib`).

**PO decisions at approval (2026-11-12):** plan approved; execute subagent-driven with 3–5 parallel agents. R1 confirmed: fork zed to `berkaygkc/zed` and carry the IME patch there. CI does not sign anything (unsigned nightlies; releases signed/notarized locally). Recorded as spec decision-log row L13.

**Spec:** `docs/superpowers/specs/2026-11-12-hacivat-v1-design.md`. Read §3.1–3.4, §4 rows i18n/Settings/Keyboard/Accessibility, §5.1–5.4, §5.6, §6 M1, §7, the decision log (L1, L2, L11, L12, L13) and §10 C8, C15, C17, C18. L13 amends L2, §6 M1 and §7.3: where they say nightlies are signed, this plan follows L13.

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
  - **Test-only exception (F1):** a crate under `crates/ui` or `crates/app` may declare `gpui = { workspace = true, features = ["test-support"] }` as a **dev-dependency**, because `#[gpui::test]` expands to hard-coded `gpui::` paths. A normal or build dependency on `gpui`/`gpui_platform`, or a dev-dependency without `test-support`, still fails `check-deps`. Production code reaches GPUI only through `hacivat_ui_core::gpui_surface`.
- **Dependencies (F23):** Task 1 writes every manifest with every dependency the crate needs in M1 (Task 1, Step 3). A later task never edits a manifest it does not own; if it needs a dependency that Task 1 missed, the controller adds it in a one-line commit on `m1-foundations`.
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
- **Packaging (L2 as amended by L13):**
  - **CI never signs.** Nightlies on all three OSes are unsigned build artifacts. No Apple certificate, notarization key or minisign key goes into GitHub secrets.
  - **Releases are signed locally on the PO's Mac.** macOS: Developer ID signed and notarized (`RELEASE=1 NOTARY_PROFILE=<profile> packaging/package-macos.sh`; notarization verified at `82dadcc`). Windows: unsigned (no identity yet, L2) but minisigned. Linux: minisign-signed. Windows and Linux release packages come from CI as unsigned artifacts and are minisigned locally (`packaging/sign-artifacts.sh`).
  - The minisign update key is passworded and lives in `~/.hacivat-keys/`; `cargo xtask check-update-key` refuses an unencrypted key.
- **Test bodies (F9):** an empty or comment-only test body is a defect. Every test in this plan lists its assertions with exact values; copy them literally. Helpers named in a test (`shell`, `switch_to`, `fixture`, …) are written by the task that owns the test file, except the shared app helpers in `crates/app/hacivat-app/tests/support/` (Task 13).
- **Full test set (F2, F10):** these commands, not `cargo test --workspace` alone, are what "all tests pass" means. Each task runs the lines for the crates it touches; PR CI and the gate (Task 19) run lines 1–4; nightly CI and the gate run line 5.
  ```
  cargo test --workspace
  cargo test -p hacivat-ui-core --features test-support,scene-check
  cargo test -p hacivat-l10n --features pseudo
  cargo test -p hacivat-app --features scene-check,pseudo
  cargo test --release -p hacivat-l10n --features pseudo
  ```
- **Stubs (F31):** a Task 1 stub never panics. It returns the documented neutral value (an empty `Vec`, `None`, the input unchanged, or a no-op), so a later wave can call it before its owner fills it in.
- **Execution (L1):**
  - Feature branch `m1-foundations` off `main`. Each task runs in its own git worktree on branch `m1/<task-slug>`, created from `m1-foundations`.
  - Each agent starts with `git merge --ff-only m1-foundations`.
  - All worktrees share `CARGO_TARGET_DIR=<main checkout>/target`.
  - Before the final verification run, `cargo clean -p <crates the task touched>` and `cargo clean --release -p <same>`.
  - The controller merges into `m1-foundations` after each task's review.
  - FTL files are append-only. Each task appends a block headed `## <task slug>`, and the controller resolves append-only conflicts by keeping both blocks.
  - The same append-only rule covers `locales/_schema/literal-allowlist.toml` (any task may append entries for its own literals, F19), the `xtask/src/main.rs` dispatch arms, and the registration block in `hacivat_app::init` (one line per task, F3).
  - Resolve a `Cargo.lock` conflict by taking the branch tip and running `cargo check --workspace`.

## Review Focus

1. **Turkish and AltGr keyboards.**
   - On TR-Q, Ctrl plus the key labelled I produces `ı`, and a `ctrl-i` binding must still fire.
   - On Windows, AltGr (Ctrl+Alt) chords such as Ctrl+Alt+Q (`@`) must type into the focused input and never fire a command.
   - Tests: `tr_q_dotless_i_matches_ctrl_i` and `altgr_char_passes_through` (Task 5); `altgr_at_sign_reaches_palette_input` (Task 16).
2. **Language switch mid-interaction.** With the palette or a dialog open and text typed, or a toast showing, switching language keeps the typed text and re-renders every label and the toast in the new language, without a restart.
   - Tests: `switch_language_with_palette_open_keeps_query` (Task 16); `toast_rerenders_after_language_switch` and `dialog_rerenders_after_language_switch` (Task 13).
3. **Hand-edited or foreign state files.** `settings.toml` and `workspace.json` may arrive with typos, wrong types, unknown keys, keys from a newer version, unparsable content, or a newer format version. The app starts with defaults for bad values, keeps unknown keys and comments, and never silently destroys the user's file.
   - Tests in Task 7.
4. **Narrow window in a long locale.** At 1024 px wide in `tr-XT`, the title-bar segments collapse into an overflow menu before the palette label truncates (the Overflow Rule), and no label is clipped.
   - Tests: `titlebar_collapses_segments_before_truncating_palette` (Task 12); shots surface `shell-narrow` (Task 18).
5. **Stacked layers and focus.** With the palette open over the settings tab, Esc closes only the palette and focus returns to the settings search field. A dialog traps focus.
   - Tests: `layers_escape_pops_topmost_and_restores_focus` (Task 13); `dialog_traps_focus_and_escape_cancels` (Task 12); `escape_closes_only_topmost_layer_and_restores_focus` (Task 19, once palette and settings have both merged).

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
- **R7. Own keystroke dispatcher.** It sits in the workspace root's `capture_key_down` (through Task 13's key-capture hook slot) and implements UI-EDIT-13 normalization. GPUI's keymap is used only to show macOS menu key equivalents: menu commands are bound under a context that no element sets, and AppKit dispatches those chords through the menu, so there is still a single handler.
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
  - Windows and Linux get an in-window app menu button after the brand; macOS uses `set_menus`. Both are built from one registry-derived menu model (Task 16).

## Waves

Interface-first: Task 1 commits every cross-task type and signature in this plan (non-panicking stub bodies where a later task owns the logic, F31), working minimal versions of the few helpers a same-wave task consumes (`Label`, `ui_text`, `popover_surface`, `focus_outline`; F4), the crate skeletons with their complete dependency lists (F23), the FTL file skeletons and the `pub mod` lines. After Task 1, each wave's tasks own disjoint files.

| Wave | Runs | Tasks (parallel agents) | Unblocked by |
|---|---|---|---|
| W0 | serial | Task 1 | — |
| W1 | 5 agents | A: Task 2 then Task 3 · B: Task 4 · C: Task 5 · D: Task 6 · E: Task 7 | Task 1 merged |
| W2 | 5 agents | Task 8 · Task 9 · Task 10 · Task 11 · Task 12 | W1 merged (Tasks 9 and 10 need 6; Tasks 11 and 12 need 4; Task 12 uses Task 1's working `Label`/`ui_text`, not Task 11's) |
| W3 | 3 agents | Task 13 · Task 14 · Task 15 | W2 merged. Task 14 merges after Task 13, and its Step 5 runs after that merge (`launch-smoke` needs `--smoke`, F17) |
| W4 | 2 agents | Task 16 · Task 17 | Task 13 merged (both plug into Task 13's `Layers`, `TabContent` registry and key-capture hook, F3) |
| W5 | serial | Task 18 | Tasks 16 and 17 merged (it renders the palette, settings, menu, About dialog and info toast) |
| W6 | serial | Task 19 (gate) | everything |

File ownership:
- W0: Task 1 as listed in its Files.
- W1:
  - Task 2: `xtask/src/check_deps.rs`
  - Task 3: `crates/ui/hacivat-bench/**`, `xtask/src/bench_report.rs`
  - Task 4: `crates/ui/hacivat-ui-core/**` except `src/keymap/**`, `src/window.rs`, `src/named_rules.rs`, `src/theme/surfaces.rs`; plus `spikes/*/Cargo.toml`, the spike `use` lines and the four `run_app` call sites in `spikes/spike-{editor,harman,grid,probes}/src/main.rs`
  - Task 5: `crates/ui/hacivat-ui-core/src/keymap/**`, `crates/ui/hacivat-ui/keymaps/**`, `crates/ui/hacivat-ui/src/keymaps.rs`, `crates/ui/hacivat-ui/tests/keymap_collisions.rs`
  - Task 6: `crates/core/hacivat-l10n/**` except `src/{format,pseudo,search,effective,casing}.rs`; plus `crates/core/hacivat-l10n-schema/**` and `locales/**/common.ftl`
  - Task 7: `crates/core/hacivat-settings/**`
- W2:
  - Task 8: `hacivat-ui-core/src/{window,named_rules}.rs`, `hacivat-ui-core/tests/{window,named_rules}.rs`
  - Task 9: the five l10n modules Task 6 does not own, and `hacivat-l10n/tests/{format,pseudo,search}.rs`
  - Task 10: `xtask/src/l10n/**`, `xtask/tests/fixtures/l10n/**`, `locales/_schema/literal-allowlist.toml` (creates it; append-only afterwards)
  - Task 11: `hacivat-ui/src/{l10n.rs, text.rs, input/**, palette.rs}`, `hacivat-ui/tests/{text,input,palette}.rs`
  - Task 12: `hacivat-ui/src/{button,segmented,icon,popover,panes,dialog,toast,menu,accesskey,statusbar,capsule,titlebar}.rs`, `hacivat-ui/tests/{panes,chrome,dialog,accesskey,a11y}.rs`
- W3:
  - Task 13: `crates/app/hacivat-app/src/{lib.rs, shell/**, l10n_global.rs, theme_pref.rs}`, `crates/app/hacivat-app/tests/{shell.rs, support/**}`, `crates/bin/hacivat/**`
  - Task 14: `.github/**`, `packaging/**`, `xtask/src/update_key.rs`
  - Task 15: root `Cargo.toml` (gpui source lines), `deny.toml`, `Cargo.lock`, `crates/ui/hacivat-ui-core/patches/README.md`
- W4:
  - Task 16: `hacivat-app/src/commands/**`, `hacivat-app/src/menus.rs`, `hacivat-app/tests/commands.rs`; one registration line in `hacivat_app::init`
  - Task 17: `hacivat-app/src/settings_ui/**`, `hacivat-app/tests/settings_ui.rs`; one registration line in `hacivat_app::init`
- W5:
  - Task 18: `hacivat-app/tests/shots.rs`, `design/vf/**`, `xtask/src/shot_diff.rs`, and the nightly `shot-diff` step in `.github/workflows/ci.yml`
- W6:
  - Task 19: `docs/benchmarks/{m1.md, m0-ime-checklist.md}`, `design/baselines/**`, `hacivat-app/tests/cross_surface.rs`

Append-only shared files (Global Constraints, Execution): FTL files, `literal-allowlist.toml`, the `xtask/src/main.rs` dispatch arms (Tasks 10, 14, 18) and the `hacivat_app::init` registration block (Tasks 16, 17).

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
    platform/{mod.rs, text_rendering.rs, memory.rs}   # AppleFontSmoothing, resident_set_mib (only unsafe module)
    theme/surfaces.rs # popover_surface, focus_outline (Task 1, complete)
    actions.rs        # RunCommand, the one generic GPUI action (menus)
    window.rs         # open_workspace_window, chrome_kind
    named_rules.rs    # [scene-check] cp-fill quad count
    keymap/{mod.rs, parse.rs, normalize.rs, resolve.rs, altgr.rs, display.rs, reach.rs}
    spike_support.rs  # [spike-support] display_sizes, native_window_id, open_unified_window
    frame_loop.rs     # [bench] + `pub use hacivat_bench as frames`
  crates/ui/hacivat-bench/src/lib.rs          # frames.rs moved here (gpui-free)
  crates/ui/hacivat-ui/
    keymaps/{default-macos,default-windows,default-linux}.toml, presets/{hacivat,datagrip}.toml,
            layouts/{tr-q,tr-f,de,fr,pl}.toml, altgr/{tr-q,tr-f,de,fr,pl}.toml (embedded, F11)
    src/{l10n.rs, text.rs, input/{mod.rs, ime.rs}, palette.rs, keymaps.rs, actions.rs, button.rs, segmented.rs, icon.rs,
         popover.rs, panes.rs, dialog.rs, toast.rs, menu.rs, accesskey.rs, statusbar.rs, capsule.rs, titlebar.rs}
  crates/core/hacivat-l10n-schema/src/lib.rs  # FTL signature extraction (build.rs + xtask share it)
  crates/core/hacivat-l10n/{build.rs, src/{lib, catalog, negotiate, runtime, render, args, format, pseudo, search, effective, casing}.rs}
  crates/core/hacivat-settings/src/{lib, schema, store, paths, workspace_state, fsio}.rs
  crates/app/hacivat-app/src/{lib, l10n_global, theme_pref, menus}.rs, shell/{mod, workspace, tiles, tabs, layers, tab_content, hooks, keymap_state}.rs,
                             commands/{mod, registry, dispatch, palette, debug}.rs, settings_ui/{mod, rows, search}.rs
  crates/app/hacivat-app/tests/{support/mod.rs, shell.rs, commands.rs, settings_ui.rs, shots.rs, cross_surface.rs}
  crates/bin/hacivat/src/{main.rs, logging.rs, smoke.rs}
  xtask/src/{check_deps.rs, bench_report.rs, l10n/{mod, rules, code_scan}.rs, update_key.rs, shot_diff.rs}
  .github/workflows/ci.yml           # was m0.yml
  packaging/{Packager.toml, common.sh, package-*.sh|ps1, sign-artifacts.sh, keys/{update.pub, README.md}}
  design/vf/{gen_samples.py (+ --set m1-shell), samples-m1-shell.json, capture.sh (+ app kind)}
  docs/benchmarks/m1.md              # gate record (Task 19)
```

---

### Task 1: Interface commit (workspace, guards, crate skeletons, catalog skeleton)

**Files:**
- Modify: `Cargo.toml`, `crates/ui/hacivat-ui-core/{Cargo.toml, src/lib.rs, src/gpui_surface.rs, src/app.rs, src/theme/mod.rs}`, `xtask/{Cargo.toml, src/main.rs, src/check_deps.rs}`, `xtask/src/bench_report.rs` (swap the `#[path]` include for the crate), `spikes/*/Cargo.toml` (`[lints] workspace = true`)
- Modify (lint fallout, F20): `spikes/spike-harman/src/screen/data.rs` (line 163), `spikes/spike-probes/src/main.rs` (crate attribute), and the four `run_app` call sites (`spikes/spike-editor/src/main.rs:67`, `spikes/spike-harman/src/main.rs:118`, `spikes/spike-grid/src/main.rs:120`, `spikes/spike-probes/src/main.rs:45`)
- Create: `clippy.toml`, `crates/ui/hacivat-bench/` (`git mv crates/ui/hacivat-ui-core/src/frames.rs crates/ui/hacivat-bench/src/lib.rs`), `crates/ui/hacivat-ui-core/src/{theme/surfaces.rs, actions.rs}`, skeletons of every crate in File Structure, and `locales/**`
- Test: `xtask/src/check_deps.rs` (two new tests), `crates/core/hacivat-settings/src/schema.rs`, `crates/ui/hacivat-ui/src/{l10n.rs, text.rs}` (unit tests)

**Interfaces (Produces; every later task consumes these names):**
- **Derives (F31).** Every data type below derives `Debug, Clone, PartialEq`; also `Eq, Hash` where the fields allow, `Copy` for fieldless enums, and `Default` where a default is named. Stub bodies follow the Stubs rule in Global Constraints.
- `hacivat-ui-core`:
  - `pub mod keymap` holding types only:
    - `Mods { ctrl, alt, shift, cmd: bool }` (`Default` = all false)
    - `NamedKey { Enter, Escape, Tab, Backspace, Delete, Insert, Space, Up, Down, Left, Right, Home, End, PageUp, PageDown, F(u8) }`
    - `NormKey { Named(NamedKey), Char(char) }`
    - `KeyPress { mods: Mods, key: NormKey, typed: Option<char> }`. `typed` is the character the OS would insert (`Keystroke::key_char`) when it differs from `key`; it is `None` in parsed chords and is ignored by chord equality and hashing. Resolution uses it to detect AltGr text (Task 5).
    - `Binding { chord: Vec<KeyPress>, command: String, context: String, when_layout: Option<KeyboardLayoutId>, during_composition: bool, source: BindingSource }`
    - `BindingSource { Default, Preset, Layout, User }`
    - `KeymapPlatform { MacOs, Windows, Linux }`
    - `KeyboardLayoutId(pub String)` (F22: not `LayoutId`, which `gpui_surface` already exports); the values used are `"us" | "tr-q" | "tr-f" | "de" | "fr" | "pl"`
    - `ContextStack(pub Vec<&'static str>)`, innermost last
    - `Resolution { Command(String), Pending, PassThrough }`
    - `KeymapDiag { DuplicateChord { chord: String, context: String }, Parse { source: BindingSource, line: u32, message: String } }`, `KeymapError { message: String }`
    - `Keymap { platform: KeymapPlatform, bindings: Vec<Binding> }` (fields private; accessors in Task 5)
  - `pub mod actions`: `RunCommand { pub id: SharedString }`, a GPUI `Action` in namespace `hacivat` (F1). It is the only GPUI action type: menu items and macOS key equivalents carry it, and its handler calls `ActionRegistry::run(&id, ..)`. Re-exported from `gpui_surface`.
  - `theme::surfaces` (complete, F4/F26; the only implementations in the workspace):
    - `pub fn popover_surface(theme: &Theme) -> Div`: raised, radius 6, seam-strong border, `popover_shadow`
    - `pub fn focus_outline<E: Styled>(el: E, theme: &Theme, focused: bool) -> E`: a 1 px blue outline at a 1 px offset when `focused`
  - `app`: `pub enum TextRendering { Crisp, Standard }`, `pub struct Boot { pub text_rendering: TextRendering }` (`Default` = `Crisp`), and the final `pub fn run_app(boot: Boot, f: impl FnOnce(&mut App) + 'static)` (F21). In this commit `run_app` ignores `boot` and keeps the M0 behaviour; Task 4 fills it in. Task 1 changes the four spike call sites and the bin to `run_app(Boot::default(), ..)`; Task 4 owns those call sites afterwards.
  - `pub mod window` and `pub mod named_rules` declared with signature stubs (see Task 8 and the bullet on `ChromeKind` below).
  - Features in `Cargo.toml`:
    - `bench`
    - `spike-support`
    - `scene-check = ["gpui/bench-support"]`
    - `test-support = ["gpui/test-support", "gpui_platform/test-support"]`
  - `gpui_surface` gains: `Global, Action, RunCommand, KeyBinding, Menu, MenuItem, OsAction, SystemMenuType, WindowControlArea, WindowDecorations, Decorations, WindowAppearance, AnyWindowHandle, Subscription, Task, MouseButton, MouseDownEvent, ClickEvent, Axis, Keystroke, KeyDownEvent, Overflow, AnyView, EntityId, FocusOutEvent, EventEmitter, TestAppContext, VisualTestContext, HeadlessAppContext` (the last three under `test-support`). `Menu` stays GPUI's application menu; the in-window popup is `hacivat_ui::menu::PopupMenu` (F22).
- `hacivat-ui`, with `lib.rs` declaring every module in File Structure:
  - `l10n`:
    - `pub trait UiTextProvider: Send + Sync + 'static { fn text(&self, id: &str, args: &[(&'static str, UiArg)]) -> SharedString; fn attr(&self, id: &str, attr: &str) -> Option<SharedString>; fn generation(&self) -> u64; }`
    - `pub enum UiArg { Count(u64), Text(SharedString) }`
    - `pub struct UiText(pub Arc<dyn UiTextProvider>)`, which is a `Global`
    - `pub fn ui_text(cx: &App, id: &str) -> SharedString`, working (F4): the provider's `text(id, &[])`, or the ID itself when no `UiText` is installed
    - `pub struct EnglishDefaults` (stub `text` returns the ID; Task 11 fills it in)
  - `text`:
    - `pub enum TextSource { Fluent, Catalog, Data, Shortcut }`
    - `pub type TextFn = Rc<dyn Fn(&App) -> SharedString>` and `pub type KeyFn = Rc<dyn Fn(&App) -> Option<char>>`: the stored form of every label on a stateful overlay (dialog, popup menu, toast), so it re-renders after a language switch (F16)
    - `pub struct Label`, built with `Label::new(text: impl Into<SharedString>, source: TextSource, style: TypeStyle, color: Hsla)`, `.truncate(bool)` and `.id(impl Into<ElementId>)`, and implementing `IntoElement`. Working minimal version (F4): a `div` with the text in `style` and `color`, no registry. Task 11 replaces the body with the custom element.
  - `input`:
    - `pub enum InputEvent { Changed, Submitted, Cancelled, CompositionChanged(bool) }` (F15)
    - `pub struct FocusedInput { pub entity: Option<EntityId>, pub composing: bool }`, a `Global` (`Default` = none, false). Every `TextInput` updates it on focus, blur, mark and unmark (Task 11); the dispatcher reads `composing` from it (Task 16).
  - `actions`:
    - `pub struct ActionDef { pub id: &'static str, pub title_id: &'static str /* hv-cmd-* (R5) */, pub context: &'static str, pub run: Rc<dyn Fn(&mut Window, &mut App)>, pub enabled: fn(&App) -> bool }`
    - `pub struct ActionRegistry`, a `Global`, with `register(&mut self, def: ActionDef) -> Result<(), DuplicateAction>`, `get(&self, id: &str) -> Option<&ActionDef>`, `all(&self) -> &[ActionDef]` and `run(id: &str, window: &mut Window, cx: &mut App) -> bool`
- `hacivat-l10n` (signatures only):
  - `MsgRef<A> { pub id: &'static str, .. }` with `const fn new(id)`
  - `trait IntoFluentArgs { fn into_args(self) -> Vec<(&'static str, ArgValue)>; }`
  - `ArgValue { Count(u64), Text(String), Decimal(ExactDecimal), When(WhenValue), List(Vec<String>) }`, plus `NoArgs`, `Count(pub u64)`, `Text(pub String)`
  - `WhenValue { Instant { unix_ms: i64 }, Local { year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8 } }`. `Instant` is converted to the format time zone; `Local` is a wall-clock value formatted as given (F30).
  - `L10n` (F31, F8):
    - `new(rt: Runtime) -> L10n`
    - `t<A: IntoFluentArgs>(&self, msg: MsgRef<A>, args: A) -> String` (UI sink, isolating)
    - `t_attr<A: IntoFluentArgs>(&self, msg: MsgRef<A>, attr: &str, args: A) -> Option<String>`
    - `t_machine<A: IntoFluentArgs>(&self, msg: MsgRef<A>, args: A) -> String` (machine sink, not isolating)
    - `t_dynamic(&self, id: &str, args: Vec<(&'static str, ArgValue)>) -> String`, `t_english(&self, id: &str) -> String` (the `en` bundle only, for the English search layer)
    - `snapshot(&self) -> Arc<Runtime>`, `install_if_newer(&self, rt: Runtime) -> bool`, `generation(&self) -> u64`
    - `reserve_generation(&self) -> u64`: backed by an `AtomicU64`; every call returns a value greater than every earlier reservation and than the installed generation. A language switch reserves at call time, so the last switch requested always wins (F8).
  - `Runtime { pub generation: u64, pub ui_chain: Vec<LanguageIdentifier>, pub format: FormatCtx, pub diags: Vec<LoadDiag>, .. }`
  - `FormatInputs { pub format_locale: String, pub time_zone: String }` with `FormatInputs::new(neg: &Negotiated, time_zone: &str) -> FormatInputs` (F7)
  - `build_runtime(cat: &Catalogs, neg: &Negotiated, fmt: &FormatInputs, generation: u64) -> Runtime`. The pseudo mode is derived from `neg.chain[0]` through `PseudoMode::for_tag(&LanguageIdentifier) -> Option<PseudoMode>`; there is no separate pseudo parameter.
  - `Catalogs`, `CatalogLocale`, `Status { Stable, Beta, Experimental }`, `Direction { Ltr, Rtl }`, `LoadDiag`, `negotiate`, `NegotiationCtx`, `PrefSource`, `Negotiated`, `NegDiag { CLocale { var: String }, Unparsable { source: String, value: String }, NoMatch { requested: String } }` (bodies in Task 6)
  - `FormatCtx`, `with_format_ctx`, `install_functions`, `PseudoMode`, `transform_literal`, `transform_resource`, `wrap_message`, `SearchOpts`, `MatchMode`, `SearchKey`, `Match`, `MatchKind`, `build_key`, `find`, `turkish_tolerant_for`, `effective`, `OsLocale`, `EffectiveLocale`, `case_for_display` (bodies in Task 9; stubs: `install_functions` and `transform_resource` are no-ops, `with_format_ctx` just calls `f`, `transform_literal` and `wrap_message` return the input, `find` returns `None`)
- `hacivat-settings` (complete data types; logic in Task 7; all derive `Debug, Clone, PartialEq, Default`):
  - `Settings { version: u32, ui: UiSettings, keymap: KeymapSettings }`
  - `UiSettings { language: LanguagePref, regional_format: LanguagePref, time_zone: TimeZonePref, theme: ThemePref, text_rendering: TextRenderingPref }`
  - `LanguagePref { System, Tag(String) }`, `TimeZonePref { System, Iana(String) }`, `ThemePref { Dark, Light, System }`, `TextRenderingPref { Crisp, Standard }`
  - `KeymapSettings { preset: KeymapPreset }`, `KeymapPreset { Hacivat, Datagrip }`
  - TOML keys: `version`, `ui.language`, `ui.regional_format`, `ui.time_zone`, `ui.theme`, `ui.text_rendering`, `keymap.preset`. Values: `"system"`, `"dark"`, `"light"`, `"crisp"`, `"standard"`, `"hacivat"`, `"datagrip"`, or a BCP-47 tag or IANA zone.
- `hacivat-app`: module stubs `shell`, `commands`, `settings_ui`, `l10n_global`, `theme_pref`, `menus`; `menus::rebuild(cx: &mut App)` (no-op stub; Task 13 calls it, Task 16 fills it, F15); `pub fn init(cx: &mut App)` with an empty registration block; and `pub struct PendingChord(pub Option<SharedString>)`, which is a `Global`. Features: `scene-check = ["hacivat-ui-core/scene-check"]`, `pseudo = ["hacivat-l10n/pseudo"]`, `shots = ["scene-check", "pseudo", "hacivat-ui-core/test-support"]`.
- Feature `pseudo` on `hacivat-l10n`. Feature `scene-check = ["hacivat-ui-core/scene-check"]` on `hacivat-ui`. `hacivat-l10n` depends on `hacivat-settings` (core → core).
- `hacivat-ui-core::window`: `ChromeKind { MacTrafficLights, WindowsCaption, LinuxClient, LinuxServer }` and `TRAFFIC_LIGHTS_INSET_PX` (value copied from `spikes/spike-harman/src/screen/titlebar.rs`) are complete in this commit, because Task 12 runs in the same wave as Task 8.
- `xtask`: `pub fn workspace_root() -> PathBuf` in `main.rs` (the parent of `CARGO_MANIFEST_DIR`), used by every xtask test that reads repo files (F20).
- `locales/_schema/dynamic-ids.toml` lists the families `hui-*` and `hv-cmd-*`.
- `hacivat` (bin): `main` calls `run_app(Boot::default(), ..)` and opens an empty window, so CI and packaging can target the crate from W1 on.
- `locales/en/locale.toml`: `tag = "en"`, `native_name = "English"`, `english_name = "English"`, `short_name = "EN"`, `direction = "ltr"`, `status = "stable"`. `tr` uses `"Türkçe"`, `"Turkish"`, `"TR"`, `status = "beta"`.
- `locales/en/common.ftl`: `-brand-hacivat = Hacivat`, `-brand-hacivat-caps = HACIVAT` (the brand is never Turkish-cased), and the message `cm-brand-hacivat-caps = { -brand-hacivat-caps }` (F6: a term has no message ID, so the title bar renders this message). `tr/common.ftl` repeats the terms unchanged.

- [ ] **Step 1: Write the failing tests.**
```rust
// xtask/src/check_deps.rs
#[test] fn clippy_toml_bans_locale_blind_casing() {
    let t = std::fs::read_to_string(workspace_root().join("clippy.toml")).unwrap();
    for m in ["str::to_lowercase", "str::to_uppercase", "char::to_lowercase", "char::to_uppercase"] { assert!(t.contains(m), "{m}"); }
}
#[test] fn test_support_dev_dependency_is_not_an_offender() {
    let md = fixture_metadata_kinds(&[("hacivat-ui", &[("gpui", DepKind::Development, &["test-support"])])]); assert!(offending_crates(&md).is_empty());
    let md = fixture_metadata_kinds(&[("hacivat-ui", &[("gpui", DepKind::Normal, &["test-support"])])]); assert_eq!(offending_crates(&md), ["hacivat-ui"]);
}
// hacivat-settings/src/schema.rs
#[test] fn defaults_match_spec() {
    let s = Settings::default();
    assert_eq!((s.version, s.ui.theme, s.ui.text_rendering, s.keymap.preset), (1, ThemePref::Dark, TextRenderingPref::Crisp, KeymapPreset::Hacivat));
    assert_eq!((s.ui.language, s.ui.regional_format, s.ui.time_zone), (LanguagePref::System, LanguagePref::System, TimeZonePref::System));
}
// hacivat-ui/src/l10n.rs
#[gpui::test] fn ui_text_without_provider_returns_the_id(cx: &mut TestAppContext) { cx.update(|cx| assert_eq!(ui_text(cx, "hui-x"), "hui-x")); }
```
- [ ] **Step 2:** Run `cargo test -p xtask clippy_toml test_support && cargo test -p hacivat-settings defaults`. Expected: FAIL (file or crate missing).
- [ ] **Step 3: Write the manifests.**
  - Root `Cargo.toml`: `[workspace.lints.rust] unsafe_code = "deny"` and `[workspace.lints.clippy] disallowed_methods = "deny"`.
  - Add the Tech Stack dependencies to `[workspace.dependencies]` with exact versions resolved now. Record them in the commit message.
  - Write each crate's dependency list in full now (F23), so no later task edits a manifest it does not own:

    | Crate | Dependencies (`[dev-dependencies]` in brackets) |
    |---|---|
    | `hacivat-ui-core` | `gpui`, `gpui_platform`, `anyhow`, `rust-embed`, `serde`, `toml` (keymap data), `icu_normalizer` (keymap NFC), `hacivat-bench` (optional, `bench`); macOS `core-foundation`, `core-foundation-sys`, `mach2`; Windows `windows-sys` |
    | `hacivat-bench` | what `frames.rs` uses today, no `gpui` |
    | `hacivat-ui` | `hacivat-ui-core`, `rust-embed`, `fluent-syntax` (EnglishDefaults), `unicode-segmentation`, `serde`, `toml`; [`gpui` with `test-support` (F1), `serde_json`] |
    | `hacivat-l10n-schema` | `fluent-syntax` |
    | `hacivat-l10n` | `fluent-bundle`, `fluent-syntax`, `intl-memoizer`, `unic-langid`, the ICU4X components, `fixed_decimal`, `arc-swap`, `rust-embed`, `sys-locale`, `iana-time-zone`, `unicode-segmentation`, `serde`, `toml`, `hacivat-settings`; build: `hacivat-l10n-schema`; [`trybuild`, `tempfile`] |
    | `hacivat-settings` | `serde`, `toml`, `toml_edit`, `serde_json`, `directories`, `unic-langid`; [`tempfile`] |
    | `hacivat-app` | `hacivat-ui-core`, `hacivat-ui`, `hacivat-l10n`, `hacivat-settings`, `serde`, `serde_json`, `anyhow`; [`gpui` with `test-support` (F1), `hacivat-ui-core` with `test-support`, `tempfile`] |
    | `hacivat` | `hacivat-app`, `hacivat-ui-core`, `hacivat-l10n`, `hacivat-settings`, `anyhow`, `tracing`, `tracing-subscriber`, `tracing-appender` |
    | `xtask` | existing, plus `toml`, `base64`, `hacivat-l10n-schema`, `fluent-syntax`, `icu_plurals` |
  - Make the M0 `check-deps` accept the F1 exception: `offending_crates` skips a dependency whose kind is `Development` and whose `features` contain `test-support`. Task 2 replaces this function.
  - Fix the existing code that the new lints reject (F20):
    - `#[allow(unsafe_code)]` with SAFETY comments on the existing `native_window_id` until Task 4 moves it.
    - `spikes/spike-harman/src/screen/data.rs:163`: `#[expect(clippy::disallowed_methods, reason = "M0 spike demo data, ASCII-only; not product code")]`.
    - `spikes/spike-probes/src/main.rs`: crate attribute `#![allow(unsafe_code, reason = "M0 probes call platform APIs directly; spike only")]`. (A manifest-level `[lints.rust]` override cannot coexist with `[lints] workspace = true`, which Task 2's `MissingWorkspaceLints` requires.)
- [ ] **Step 4: Write the skeletons and the catalog files with the interfaces above.** Keep `hacivat_ui_core::frames` and `shell` working for the spikes (`spike-grid/src/bench.rs:13`, `spike-editor/src/bench.rs:12`).
- [ ] **Step 5:** Run `cargo check --workspace --all-targets && cargo clippy --workspace --all-targets -- -D warnings`, then lines 1–4 of the full test set, then `cargo xtask check-deps && cargo xtask tokens --check`. Expected: all pass, and every existing M0 test still passes.
- [ ] **Step 6: Commit** with message `chore(m1): interface commit — crate skeletons, lints, casing ban, catalog skeleton`.

### Task 2: Dependency guard: transitive GPUI/zed reach, layers, lints

**Files:**
- Modify: `xtask/src/check_deps.rs` (`toml` is already an xtask dependency from Task 1)

**Interfaces:**
- Produces:
  - `pub enum Rule { ZedOutsideAdapter, CoreReachesUi, UiDependsOnCore, AdapterFeatureInProduct, MissingWorkspaceLints }`
  - `pub struct Violation { pub krate: String, pub rule: Rule, pub via: Vec<String> }`
  - `pub fn violations(md: &Metadata, manifest: &dyn Fn(&Path) -> String) -> Vec<Violation>`
  - `pub const ZED_SOURCES: [&str; 2] = ["https://github.com/zed-industries/zed", "https://github.com/berkaygkc/zed"]`
  - Layer comes from the manifest path: `crates/ui/` → Ui, `crates/{core,sql,ai}/` → Core, `crates/{app,bin}/` → App, `spikes/` → Spike, `xtask` → Tool.

- [ ] **Step 1: Write the failing tests.** Extend `fixture_metadata` to emit `resolve.nodes[].deps[].dep_kinds`, `dependencies[].features` and package `source`. The `fx*` helpers synthesize a package for every dependency name that the fixture does not define.
```rust
#[test] fn flags_transitive_gpui_through_helper() { let md = fx(&[("spike-x", &["helper"]), ("helper", &["gpui"])]); assert_eq!(names(&violations(&md, &lints_ok)), ["helper", "spike-x"]); }
#[test] fn gpui_through_adapter_is_allowed() { let md = fx(&[("spike-x", &["hacivat-ui-core"]), ("hacivat-ui-core", &["gpui", "gpui_platform"])]); assert!(violations(&md, &lints_ok).is_empty()); }
#[test] fn allows_test_support_dev_dependency_on_gpui() { let md = fx_kind_at("hacivat-ui", "crates/ui", "gpui", DepKind::Development, &["test-support"]); assert!(violations(&md, &lints_ok).is_empty()); let md = fx_kind_at("hacivat-app", "crates/app", "gpui", DepKind::Development, &["test-support"]); assert!(violations(&md, &lints_ok).is_empty()); }
#[test] fn flags_dev_dependency_on_gpui_without_test_support() { let md = fx_kind_at("hacivat-ui", "crates/ui", "gpui", DepKind::Development, &[]); assert_eq!(rule_of(&md, "hacivat-ui"), Rule::ZedOutsideAdapter); }
#[test] fn flags_normal_dependency_on_gpui_even_with_test_support() { let md = fx_kind_at("hacivat-ui", "crates/ui", "gpui", DepKind::Normal, &["test-support"]); assert_eq!(rule_of(&md, "hacivat-ui"), Rule::ZedOutsideAdapter); }
#[test] fn flags_build_dependency_on_gpui() { let md = fx_kind_at("hacivat-app", "crates/app", "gpui", DepKind::Build, &["test-support"]); assert_eq!(rule_of(&md, "hacivat-app"), Rule::ZedOutsideAdapter); }
#[test] fn test_support_exception_is_limited_to_ui_and_app() { let md = fx_kind_at("hacivat-l10n", "crates/core", "gpui", DepKind::Development, &["test-support"]); assert_eq!(rule_of(&md, "hacivat-l10n"), Rule::ZedOutsideAdapter); }
#[test] fn flags_gpui_platform_only() { let md = fx(&[("hacivat-app", &["gpui_platform"])]); assert_eq!(rule_of(&md, "hacivat-app"), Rule::ZedOutsideAdapter); }
#[test] fn flags_any_zed_sourced_crate() { let md = fx_src(&[("hacivat-ui", &[("collections", ZED_SOURCES[0])])]); assert_eq!(rule_of(&md, "hacivat-ui"), Rule::ZedOutsideAdapter); }
#[test] fn flags_fork_sourced_crate() { let md = fx_src(&[("hacivat-ui", &[("gpui_util", ZED_SOURCES[1])])]); assert_eq!(rule_of(&md, "hacivat-ui"), Rule::ZedOutsideAdapter); }
#[test] fn flags_core_reaching_adapter() { let md = fx_at(&[("hacivat-l10n", "crates/core", &["hacivat-ui-core"])]); assert_eq!(rule_of(&md, "hacivat-l10n"), Rule::CoreReachesUi); }
#[test] fn flags_ui_depending_on_core() { let md = fx_at(&[("hacivat-ui", "crates/ui", &["hacivat-l10n"]), ("hacivat-l10n", "crates/core", &[])]); assert_eq!(rule_of(&md, "hacivat-ui"), Rule::UiDependsOnCore); }
#[test] fn app_may_use_core_and_ui() { let md = fx_at(&[("hacivat-app", "crates/app", &["hacivat-ui", "hacivat-l10n"])]); assert!(violations(&md, &lints_ok).is_empty()); }
#[test] fn flags_product_enabling_adapter_only_features() { for f in ["spike-support", "bench", "scene-check", "test-support"] { let md = fx_feat("hacivat", "hacivat-ui-core", &[f]); assert_eq!(rule_of(&md, "hacivat"), Rule::AdapterFeatureInProduct, "{f}"); } }
#[test] fn app_dev_dependency_may_enable_adapter_test_support() { let md = fx_feat_kind("hacivat-app", "hacivat-ui-core", DepKind::Development, &["test-support"]); assert!(violations(&md, &lints_ok).is_empty()); }
#[test] fn flags_member_without_workspace_lints() { let md = fx(&[("hacivat-ui", &[])]); let v = violations(&md, &|_| "[package]\nname=\"hacivat-ui\"\n".into()); assert_eq!(v[0].rule, Rule::MissingWorkspaceLints); }
```
- [ ] **Step 2:** Run `cargo test -p xtask check_deps`. Expected: FAIL.
- [ ] **Step 3: Implement `violations`.**
  - BFS over `resolve.nodes` from each workspace crate across normal, build and dev edges.
  - Do not expand through `hacivat-ui-core` for Ui, App, Spike and Tool crates.
  - F1 exception: a direct dev edge from a Ui or App crate to `gpui` whose `dependencies[].features` contain `test-support` is allowed and not expanded. Every other edge to `gpui`, `gpui_platform` or a zed-sourced package is a `ZedOutsideAdapter` violation.
  - Core crates are not allowed to reach `hacivat-ui-core` at all.
  - A package counts as zed-sourced when `source` starts with `git+` followed by any entry of `ZED_SOURCES`.
  - `AdapterFeatureInProduct` applies to `hacivat` and `hacivat-app` when a normal (non-dev) dependency on `hacivat-ui-core` requests `spike-support`, `bench`, `scene-check` or `test-support` (`dependencies[].features`, F33). Release builds must never carry `Window::rendered_scene` (R12).
  - `MissingWorkspaceLints`: the manifest parsed with `toml` lacks `lints.workspace = true`.
  - `run()` prints `check-deps: ok` or one line per violation (`<crate>: <rule> via a -> b -> gpui`) and exits 1.
- [ ] **Step 4:** Run `cargo test -p xtask check_deps && cargo xtask check-deps`. Expected: PASS and `check-deps: ok`.
- [ ] **Step 5: Commit** with message `feat(xtask): transitive GPUI/zed reach, layer and lint guards`.

### Task 3: `hacivat-bench` and bench-report hygiene

**Files:**
- Modify: `crates/ui/hacivat-bench/src/lib.rs`, `xtask/src/bench_report.rs`

**Interfaces:**
- Consumes: the moved `FrameRecorder`, `stats_of`, `verdict`, `build_label(head: &str, porcelain: &str) -> String` (today it splits on `.lines()`, `frames.rs:218`), `build_stamp(root: &Path) -> String`; in `bench_report.rs`, `insert_rows(doc: &str, rows: &str) -> String` and `row_for(json: &str) -> Result<String, String>` (F20: these are the real names).
- Produces:
  - `build_label(head: &str, porcelain_z: &str) -> String` parses NUL-separated `git status --porcelain -z` entries. A rename or copy entry (`R` or `C` status) is followed by a second NUL-terminated path with no status prefix: the parser consumes it and ignores it.
  - `build_stamp` now runs `git status --porcelain -z` and ignores paths under `docs/benchmarks/`.
  - `bench_report::row_for(json: &str, source_file: &str) -> Result<String, String>`: the row ends with a hidden cell `<!-- src:<source_file> -->`.
  - `bench_report::insert_rows(doc: &str, rows: &str) -> String` de-duplicates by that source cell: a row whose source is already in `doc` replaces the old row instead of being added.
  - `bench_report::escape_cell(s: &str) -> String`
  - `bench_report::parse_milestone(s: &str) -> Result<String>`, which accepts `^m[0-9]+$`
  - The existing `frames.rs` label tests (around `frames.rs:695`, `\n`-separated input) are rewritten to `-z` input.

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn bench_doc_edits_are_not_dirty() { assert_eq!(build_label("abc1234", " M docs/benchmarks/m1.md\0"), "abc1234"); }
#[test] fn non_ascii_path_is_dirty_without_quoting() { assert_eq!(build_label("abc1234", " M crates/ui/İçerik.rs\0"), "abc1234-dirty"); }
#[test] fn rename_source_path_is_not_parsed_as_an_entry() { assert_eq!(build_label("abc1234", "R  docs/benchmarks/new.md\0docs/benchmarks/old.md\0 M docs/benchmarks/m1.md\0"), "abc1234"); assert_eq!(build_label("abc1234", "R  crates/new.rs\0docs/benchmarks/old.md\0"), "abc1234-dirty"); }
#[test] fn pipe_in_label_is_escaped() { assert_eq!(escape_cell("b1|x"), "b1\\|x"); }
#[test] fn milestone_rejects_paths() { assert!(parse_milestone("../m0").is_err()); assert_eq!(parse_milestone("m1").unwrap(), "m1"); }
#[test] fn rerun_does_not_duplicate_rows() { let r = row_for(B1_FIXTURE_JSON, "b1-fling-macos-1.json").unwrap(); let doc = insert_rows(DOC_FIXTURE, &r); assert_eq!(insert_rows(&doc, &r), doc); assert_eq!(doc.matches("src:b1-fling-macos-1.json").count(), 1); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-bench -p xtask bench_report`. Expected: FAIL.
- [ ] **Step 3: Implement.** The source JSON file name goes in the trailing hidden HTML comment cell, so de-duplication needs no extra column.
- [ ] **Step 4:** Run the same tests, then `cargo build -p spike-grid -p spike-editor`. Expected: PASS, and the spikes still compile.
- [ ] **Step 5: Commit** with message `refactor(bench): gpui-free hacivat-bench; bench-report escaping, milestone check, de-dup`.

### Task 4: ui-core single bootstrap, spike/bench features, text rendering (L12)

**Files:**
- Modify: `crates/ui/hacivat-ui-core/src/{app.rs, fonts.rs, lib.rs}`, `Cargo.toml` (features only; dependencies are in place from Task 1)
- Create: `src/assets.rs` (moved from `shell.rs`), `src/platform/{mod.rs, text_rendering.rs, memory.rs}`, `src/spike_support.rs` (moved from `shell.rs`)
- Delete: `src/shell.rs`
- Add: Lucide v0.460.0 `menu`, `minus`, `square`, `plus`, `settings`, `info`, `triangle-alert`, `circle-check`, `ellipsis`, `keyboard`, `rows-2`, `panel-left` to `assets/icons/lucide/`, plus the `ICONS` list (`copy` is already bundled)
- Modify: `spikes/*/Cargo.toml` (enable `spike-support`/`bench`), their `use` lines, and the four `run_app` call sites listed in Task 1 (owned by this task from W1 on, F21)

**Interfaces:**
- Consumes: `TextRendering`, `Boot` and the `run_app(boot, f)` signature (Task 1).
- Produces:
  - `run_app` body. This is the only bootstrap. It calls `platform::apply_text_rendering(boot.text_rendering)` before `gpui_platform::application()`, then installs assets, registers and verifies fonts (exit 3 on failure), installs the `Dark` theme and calls `f`.
  - `platform::apply_text_rendering(mode: TextRendering)`:
    - macOS: `CFPreferencesSetAppValue("AppleFontSmoothing", 0 for Crisp | NULL for Standard, kCFPreferencesCurrentApplication)`. Standard removes our override, so the user's global preference applies.
    - Other OSes: no-op.
  - `platform::app_font_smoothing_override() -> Option<i64>`: reads back the app-domain value on macOS, `None` elsewhere.
  - `platform::resident_set_mib() -> Option<u64>` (F18): macOS `mach_task_basic_info.resident_size` via `mach2`; Linux `VmRSS` from `/proc/self/status`; Windows `GetProcessMemoryInfo().WorkingSetSize` via `windows-sys`. `None` on failure. The bin's `--smoke` uses it.
  - `pub const TEXT_RENDERING_IS_LIVE: bool = false` (R11; Task 17 reads it to show the restart note).
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
    platform::apply_text_rendering(TextRendering::Crisp);   assert_eq!(platform::app_font_smoothing_override(), Some(0));
}
#[test] fn resident_set_is_reported() { let m = platform::resident_set_mib().expect("RSS readable on every OS"); assert!(m >= 1 && m < 4096, "{m}"); let big = vec![1u8; 64 << 20]; assert!(platform::resident_set_mib().unwrap() >= m + 32, "64 MiB touched"); drop(big); }
#[test] fn radii_on_scale_in_both_themes() { for t in [&DARK, &LIGHT] { assert_eq!([t.radii.none, t.radii.xs, t.radii.sm, t.radii.md], [0.0, 3.0, 4.0, 6.0]); } }
#[test] fn bundled_icons_include_m1_set() { for n in ["menu", "minus", "square", "copy", "settings", "ellipsis", "keyboard"] { assert!(BundledAssets.load(&format!("icons/lucide/{n}.svg")).unwrap().is_some(), "{n}"); } }
```
  The tautological `text_rendering_is_restart_only` (it asserted a constant) is dropped; R11 is covered by Task 17's `text_rendering_change_needs_restart` (F9).
- [ ] **Step 2:** Run `cargo test -p hacivat-ui-core`. Expected: FAIL.
- [ ] **Step 3: Implement the split and the platform module.**
  - `platform/mod.rs` is the only module with `#![allow(unsafe_code)]`. It uses `core-foundation`/`core-foundation-sys` and `mach2` on macOS and `windows-sys` on Windows. Every unsafe block has a `// SAFETY:` comment.
  - `resident_set_is_reported` fills the 64 MiB buffer with ones so the pages are touched.
  - Add a comment in `fonts.rs` saying that `tnum` is a no-op because Source Sans 3 digits are tabular by default (deferred minor T3).
- [ ] **Step 4:** Run:
  ```
  cargo test -p hacivat-ui-core
  cargo check -p hacivat-ui-core
  cargo check -p hacivat-ui-core --features spike-support,bench
  cargo build -p spike-harman -p spike-grid -p spike-editor -p spike-probes
  ```
  Expected: PASS. Without features, the crate exposes no `spike_support`/`frame_loop`.
- [ ] **Step 5: Commit** with message `feat(ui-core): single bootstrap, crisp text by default (L12), RSS probe, spike/bench features`.

### Task 5: Keymap model, data and AltGr collision CI (UI-EDIT-13)

**Files:**
- Create:
  - `crates/ui/hacivat-ui-core/src/keymap/{parse,normalize,resolve,altgr,display,reach}.rs`
  - `crates/ui/hacivat-ui/keymaps/**`
  - `crates/ui/hacivat-ui/src/keymaps.rs`
  - `crates/ui/hacivat-ui/tests/keymap_collisions.rs`
- Modify: `crates/ui/hacivat-ui-core/src/keymap/mod.rs` (Task 1's types file; add the `pub mod`/`pub use` lines and the `Keymap` methods)

**Interfaces:**
- Consumes: the Task 1 keymap types. Dependencies (`toml`, `icu_normalizer`) are in place from Task 1.
- Produces:
  - `parse_chord(s: &str) -> Result<Vec<KeyPress>, KeymapError>`, for example `"ctrl-shift-7"` or `"cmd-k cmd-s"`
  - `normalize_char(c: char) -> char`: NFC, then `ı`/`İ`/`I` → `i`, then ASCII lowercase
  - `normalize(k: &Keystroke) -> Option<KeyPress>`: `None` for a dead key (no `key_char` and not a named key). `typed` is `key_char` when it differs from `key`.
  - `Keymap::from_sources(platform: KeymapPlatform, default: &str, preset: &str, layout_overlays: &[(&KeyboardLayoutId, &str)], user: Option<&str>) -> (Keymap, Vec<KeymapDiag>)`
  - `Keymap::resolve(&self, ctx: &ContextStack, pending: &[KeyPress], press: &KeyPress, layout: &KeyboardLayoutId, composing: bool) -> Resolution`. On Windows and Linux, a press with `ctrl` and `alt` and a printable `typed` character is AltGr text and resolves to `PassThrough`, whatever the bindings say (Review Focus 1).
  - `Keymap::bindings(&self) -> &[Binding]` (F15) and `Keymap::platform(&self) -> KeymapPlatform`
  - `Keymap::bindings_for(&self, command: &str, layout: &KeyboardLayoutId) -> Vec<&Binding>`
  - `AltGrTable::parse(toml: &str) -> AltGrTable`, `AltGrTable::layout(&self) -> &str`; `AltGrTables(pub Vec<AltGrTable>)` with `get(&self, layout: &str, key: char) -> Option<char>`
  - `altgr_collisions(km: &Keymap, tables: &AltGrTables) -> Vec<Collision>`, with `Collision { chord: String, layout: String, produces: char }` (derives `Debug, Clone, PartialEq, Eq`)
  - `option_only_printables(km: &Keymap) -> Vec<String>`: macOS chords that use Option alone on a printable key
  - `unreachable_chords(km: &Keymap, layout: &KeyboardLayoutId, shift_level: &[char]) -> Vec<String>` (F27): chords whose key character needs Shift on that layout, whose chord lacks Shift, and for which no overlay alternative exists
  - `display_chord(chord: &[KeyPress], platform: KeymapPlatform) -> String`: macOS `⌃⌥⇧⌘` glyphs, otherwise `Ctrl+Shift+7` with English modifier names
  - `layout_id_from_platform(id: &str) -> KeyboardLayoutId`: maps macOS/Windows/Linux layout IDs such as `com.apple.keylayout.Turkish` and the Windows KLID `0000041F` to `tr-q` (F27)
  - `hacivat_ui::keymaps::load(platform, preset: &str, user: Option<&str>) -> (Keymap, Vec<KeymapDiag>)`, which embeds the data files
  - `hacivat_ui::keymaps::layouts_altgr() -> AltGrTables` (F11): the embedded `keymaps/altgr/*.toml` tables, used at runtime by Settings and in CI
  - `hacivat_ui::keymaps::shift_level(layout: &KeyboardLayoutId) -> Vec<char>`: base-layer characters that need Shift on that layout (from `keymaps/layouts/<id>.toml`, key `shift_level`)
  - `hacivat_ui::keymaps::PRESETS: [&str; 2] = ["hacivat", "datagrip"]` and `accepted_altgr_shadowing(preset: &str) -> Vec<Collision>` (from the preset file's `[[accepted_altgr_shadowing]]` entries, F12)
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

  The `datagrip` preset uses `workspace.command_palette` = cmd-shift-a/ctrl-shift-a and `workspace.open_settings` = cmd-,/ctrl-alt-s. On PL (Programmer), AltGr+S types `ś`, so `ctrl-alt-s` is an **accepted, documented shadowing** (F12): AltGr chords always pass through (Review Focus 1), so on PL the chord types `ś` and the command stays reachable from the palette. `presets/datagrip.toml` lists it under `[[accepted_altgr_shadowing]]` (chord `ctrl-alt-s`, layout `pl`, produces `ś`, plus any other layout the tables report for that chord, each listed explicitly), with a comment giving this reason.

  AltGr tables (`keymaps/altgr/*.toml`, embedded, F11) list the AltGr+key → char pairs for TR-Q, TR-F, DE, FR (AZERTY) and PL (Programmer). Each file has a header citing its source layout (KBDTUQ, KBDTUF, KBDGR, KBDFR, KBDPL1). Layout overlays (`keymaps/layouts/*.toml`) carry `shift_level` and the overlay alternatives that make every default chord reachable (for example FR AZERTY digits need Shift, so `alt-1` gets an overlay).

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn tr_q_dotless_i_matches_ctrl_i() { let km = fixture("[[binding]]\nchord=\"ctrl-i\"\ncommand=\"t.x\"\ncontext=\"Workspace\""); assert_eq!(km.resolve(&ws(), &[], &kp(ctrl(), 'ı'), &tr_q(), false), Resolution::Command("t.x".into())); }
#[test] fn shift_dotless_i_gives_capital_i_matches_ctrl_shift_i() {
    assert_eq!(normalize_char('I'), 'i'); assert_eq!(normalize_char('İ'), 'i');
    let km = fixture("[[binding]]\nchord=\"ctrl-shift-i\"\ncommand=\"t.y\"\ncontext=\"Workspace\"");
    assert_eq!(km.resolve(&ws(), &[], &kp(ctrl_shift(), 'I'), &tr_q(), false), Resolution::Command("t.y".into()));
    assert_eq!(km.resolve(&ws(), &[], &kp(ctrl_shift(), 'İ'), &tr_q(), false), Resolution::Command("t.y".into()));
}
#[test] fn altgr_char_passes_through() {
    let km = fixture_on(KeymapPlatform::Windows, "[[binding]]\nchord=\"ctrl-alt-q\"\ncommand=\"t.q\"\ncontext=\"Workspace\"");
    assert_eq!(km.resolve(&ws(), &[], &kp_typed(ctrl_alt(), 'q', '@'), &tr_q(), false), Resolution::PassThrough);
    assert_eq!(km.resolve(&ws(), &[], &kp(ctrl_alt(), 'q'), &us(), false), Resolution::Command("t.q".into()));
}
#[test] fn dead_key_never_matches() { assert_eq!(normalize(&Keystroke { modifiers: Modifiers::none(), key: "dead_circumflex".into(), key_char: None }), None); }
#[test] fn composing_blocks_bindings() { let km = defaults(KeymapPlatform::MacOs); assert_eq!(km.resolve(&ws(), &[], &kp(cmd(), 'k'), &us(), true), Resolution::PassThrough); assert_eq!(km.resolve(&ws(), &[], &kp(cmd(), 'k'), &us(), false), Resolution::Command("workspace.command_palette".into())); }
#[test] fn two_key_chord_is_pending_then_fires() { let km = fixture_chord("ctrl-k ctrl-s", "t.save"); assert_eq!(km.resolve(&ws(), &[], &kp(ctrl(), 'k'), &us(), false), Resolution::Pending); assert_eq!(km.resolve(&ws(), &[kp(ctrl(), 'k')], &kp(ctrl(), 's'), &us(), false), Resolution::Command("t.save".into())); }
#[test] fn innermost_context_wins() {
    let km = fixture("[[binding]]\nchord=\"escape\"\ncommand=\"ws.esc\"\ncontext=\"Workspace\"\n[[binding]]\nchord=\"escape\"\ncommand=\"pal.esc\"\ncontext=\"Palette\"");
    assert_eq!(km.resolve(&ctx(&["Workspace", "Palette"]), &[], &kpn(Mods::default(), NamedKey::Escape), &us(), false), Resolution::Command("pal.esc".into()));
    assert_eq!(km.resolve(&ctx(&["Workspace"]), &[], &kpn(Mods::default(), NamedKey::Escape), &us(), false), Resolution::Command("ws.esc".into()));
}
#[test] fn tr_q_overlay_displays_shift_7_for_slash() { let km = fixture_with_overlay("ctrl-/", "tr-q", "ctrl-shift-7"); assert_eq!(display_chord(&km.bindings_for("t.c", &tr_q())[0].chord, KeymapPlatform::Windows), "Ctrl+Shift+7"); }
#[test] fn mac_display_uses_glyphs() { assert_eq!(display_chord(&parse_chord("cmd-shift-p").unwrap(), KeymapPlatform::MacOs), "⇧⌘P"); }
#[test] fn duplicate_chord_in_context_is_diagnosed() { let (_, d) = Keymap::from_sources(KeymapPlatform::Windows, DUP, "", &[], None); assert_eq!(d, vec![KeymapDiag::DuplicateChord { chord: "ctrl-k".into(), context: "Workspace".into() }]); }
#[test] fn bindings_lists_every_loaded_binding() { let km = fixture_chord("ctrl-k ctrl-s", "t.save"); assert_eq!(km.bindings().len(), 1); assert_eq!(km.bindings()[0].command, "t.save"); }
#[test] fn windows_turkish_klid_maps_to_tr_q() { assert_eq!(layout_id_from_platform("0000041F"), KeyboardLayoutId("tr-q".into())); assert_eq!(layout_id_from_platform("com.apple.keylayout.Turkish"), KeyboardLayoutId("tr-q".into())); }
// hacivat-ui/tests/keymap_collisions.rs
#[test] fn every_preset_has_exactly_its_accepted_altgr_collisions() { for preset in PRESETS { for p in [KeymapPlatform::Windows, KeymapPlatform::Linux] { assert_eq!(altgr_collisions(&load(p, preset, None).0, &layouts_altgr()), accepted_altgr_shadowing(preset), "{preset} {p:?}"); } } }
#[test] fn hacivat_preset_accepts_no_shadowing_and_datagrip_names_polish_s() { assert!(accepted_altgr_shadowing("hacivat").is_empty()); assert!(accepted_altgr_shadowing("datagrip").contains(&Collision { chord: "ctrl-alt-s".into(), layout: "pl".into(), produces: 'ś' })); }
#[test] fn adding_ctrl_alt_l_fails_naming_polish_l_stroke() { let km = with_extra(KeymapPlatform::Windows, "ctrl-alt-l"); let c = altgr_collisions(&km, &layouts_altgr()); assert!(c.iter().any(|c| c.layout == "pl" && c.produces == 'ł')); }
#[test] fn mac_defaults_never_use_bare_option_on_printable() { for preset in PRESETS { assert!(option_only_printables(&load(KeymapPlatform::MacOs, preset, None).0).is_empty(), "{preset}"); } }
#[test] fn known_altgr_facts() { let t = layouts_altgr(); assert_eq!(t.get("tr-q", 'q'), Some('@')); assert_eq!(t.get("de", 'm'), Some('µ')); assert_eq!(t.get("pl", 'l'), Some('ł')); assert_eq!(t.get("pl", 's'), Some('ś')); }
#[test] fn default_chords_are_reachable_on_every_layout() { for preset in PRESETS { for p in ALL_PLATFORMS { let km = load(p, preset, None).0; for l in ["us", "tr-q", "tr-f", "de", "fr", "pl"] { let l = KeyboardLayoutId(l.into()); assert!(unreachable_chords(&km, &l, &shift_level(&l)).is_empty(), "{preset} {p:?} {l:?}"); } } } }
#[test] fn azerty_digit_without_overlay_is_unreachable() { let km = with_extra(KeymapPlatform::Windows, "alt-2"); let fr = KeyboardLayoutId("fr".into()); assert_eq!(unreachable_chords(&km, &fr, &shift_level(&fr)), vec!["alt-2".to_string()]); }
```
- [ ] **Step 2:** Run `cargo test -p hacivat-ui-core keymap && cargo test -p hacivat-ui --test keymap_collisions`. Expected: FAIL.
- [ ] **Step 3: Implement parsing, normalization, resolution, reachability and the collision check.** Resolution walks the context stack innermost first, matches on a `(Mods, NormKey)` hash, and applies layout-overlay alternatives for the active `KeyboardLayoutId` (UI-EDIT-13 steps 2–5). For a symbol chord, ignore Shift when the active layout needs Shift to type that symbol.
- [ ] **Step 4: Write the data files**, including the overlays the reachability test demands.
- [ ] **Step 5:** Run the same tests. Expected: PASS.
- [ ] **Step 6: Commit** with message `feat(keymap): UI-EDIT-13 model, M1 default keymaps, AltGr collision and reachability CI`.

### Task 6: `hacivat-l10n` core: catalogs, negotiation, typed IDs, render, swap

**Files:**
- Create:
  - `crates/core/hacivat-l10n-schema/src/lib.rs`
  - `crates/core/hacivat-l10n/build.rs`
  - `crates/core/hacivat-l10n/src/{lib,catalog,negotiate,runtime,render,args}.rs`
  - `crates/core/hacivat-l10n/tests/{render.rs, negotiate.rs, ui/wrong_args.rs, ui/wrong_args.stderr}` (the `.stderr` is generated once with `TRYBUILD=overwrite` and checked in, F31)
- Modify: `locales/{en,tr}/common.ftl`

**Interfaces:**
- Consumes: the Task 1 signatures, including the non-panicking Task 9 stubs.
- Wiring (so Task 9 changes only its own five files, Part 1 row 12 of the pre-flight): `build_runtime` calls `format::install_functions` on every bundle; rendering runs inside `format::with_format_ctx(&rt.format, ..)`; when `PseudoMode::for_tag(&neg.chain[0])` is `Some`, the `en` resources go through `pseudo::transform_resource` and UI-sink results through `pseudo::wrap_message`. Until Task 9 lands, these are the identity, so the bundle uses Fluent's default formatting.
- Produces:
  - Schema crate:
    - `Signature { id: String, vars: Vec<(String, VarType)>, attrs: Vec<String>, selector_vars: Vec<String> }`
    - `VarType { String, Number, DateTime, List, Enum(Vec<String>) }`
    - `extract(ftl: &str, file: &str) -> Result<Vec<Signature>, Vec<SchemaError>>`, which reads the `# Variables:` blocks
  - `build.rs` turns `locales/en/*.ftl` into `ids.rs`, exposed as `hacivat_l10n::ids::{cm, hv, hui}`:
    - each message becomes a const `hv::TITLEBAR_PALETTE: MsgRef<NoArgs>`;
    - a message with variables gets an args struct named after it in PascalCase (`HvTabConsoleUntitled { pub n: Count }`).
    - The build fails on an undocumented variable. Dynamic families are listed in `locales/_schema/dynamic-ids.toml` (`hui-*`, `hv-cmd-*`); their messages still get constants.
  - Catalogs:
    - `Catalogs::embedded() -> Catalogs` uses `rust-embed` over `locales/` and skips `_`-prefixed folders.
    - `Catalogs::with_overlay(self, dir: &Path) -> Catalogs`
    - `Catalogs::from_memory(files: &[(&str, &str)]) -> Catalogs`
    - `locales() -> &[CatalogLocale]`, `diags() -> &[LoadDiag]`
  - `CatalogLocale { tag, max, native_name, english_name, short_name, direction: Direction, status: Status, source }`
  - `LoadDiag { NonCanonicalTag{folder}, MissingManifest{tag}, Junk{file, line}, ReservedPseudo{folder}, OverlaySkipped{path, reason} }`
  - Negotiation:
    - `negotiate(ctx: &NegotiationCtx, avail: &[CatalogLocale]) -> Negotiated`
    - `NegotiationCtx { sources: Vec<PrefSource>, format_pref: Option<String>, os_format: Option<String>, allow_experimental: bool, allow_pseudo: bool }`
    - `PrefSource { Setting(String), Env(&'static str, String), OsList(Vec<String>) }`
    - `Negotiated { chain: Vec<LanguageIdentifier>, format_locale: String, diags: Vec<NegDiag> }`
  - `L10n` per the Task 1 signatures, including `reserve_generation` (F8). It is backed by `ArcSwap<Runtime>`, holds two bundle sets per locale (Ui isolating, Machine not), and falls back per message (I18N-04 steps 5–9).

- [ ] **Step 1: Write the failing tests** (the normative rows of I18N-02, -03 and -04).
```rust
#[test] fn embedded_has_en_stable_and_tr_beta() { let c = Catalogs::embedded(); assert_eq!(tags(&c), ["en", "tr"]); assert_eq!(status(&c, "en"), Status::Stable); assert_eq!(status(&c, "tr"), Status::Beta); }
#[test] fn overlay_without_manifest_is_experimental() { let c = Catalogs::embedded().with_overlay(&tmp_overlay(&[("de/hacivat-ui.ftl", "hv-x = X")])); assert_eq!(status(&c, "de"), Status::Experimental); assert_eq!(english_name(&c, "de"), "de"); }
#[test] fn overlay_overrides_per_id() {
    let c = Catalogs::from_memory(&[("en/locale.toml", EN_TOML), ("tr/locale.toml", TR_TOML), ("en/hacivat-ui.ftl", "hv-a = A\nhv-b = B"), ("tr/hacivat-ui.ftl", "hv-a = A-tr\nhv-b = B-tr")])
        .with_overlay(&tmp_overlay(&[("tr/hacivat-ui.ftl", "hv-a = A-overlay")]));
    let l = l10n_from(&c, &["tr", "en"]); assert_eq!(l.t_dynamic("hv-a", vec![]), "A-overlay"); assert_eq!(l.t_dynamic("hv-b", vec![]), "B-tr");
}
#[test] fn junk_line_keeps_other_messages() { let c = Catalogs::from_memory(&[("en/locale.toml", EN_TOML), ("en/hacivat-ui.ftl", "hv-a = A\n\n\n\n\n\n!!junk\nhv-b = B\n")]); assert!(c.diags().contains(&LoadDiag::Junk { file: "en/hacivat-ui.ftl".into(), line: 7 })); let l = l10n_from(&c, &["en"]); assert_eq!((l.t_dynamic("hv-a", vec![]), l.t_dynamic("hv-b", vec![])), ("A".into(), "B".into())); }
#[test] fn pseudo_folder_on_disk_is_rejected() { let c = Catalogs::embedded().with_overlay(&tmp_overlay(&[("en-XA/hacivat-ui.ftl", "hv-a = A")])); assert!(c.diags().contains(&LoadDiag::ReservedPseudo { folder: "en-XA".into() })); assert!(!tags(&c).contains(&"en-XA")); }
#[test] fn os_tr_tr_gives_tr_chain() { let n = neg(&[os(&["tr-TR", "en-US"])], None); assert_eq!(chain(&n), ["tr", "en"]); assert_eq!(n.format_locale, "tr-TR"); }
#[test] fn english_setting_keeps_turkish_region() { let n = neg(&[set("en"), os(&["tr-TR"])], Some("tr-TR")); assert_eq!(chain(&n), ["en"]); assert_eq!(n.format_locale, "tr-TR"); }
#[test] fn lang_c_is_english_with_diag() { let n = neg(&[env("LANG", "C")], None); assert_eq!(chain(&n), ["en"]); assert_eq!(n.diags, vec![NegDiag::CLocale { var: "LANG".into() }]); }
#[test] fn codeset_and_modifier_are_stripped() { assert_eq!(chain(&neg(&[env("LANG", "tr_TR.ISO-8859-9")], None)), ["tr", "en"]); assert_eq!(chain(&neg(&[env("LANG", "tr_TR@euro")], None)), ["tr", "en"]); }
#[test] fn never_across_languages() { assert_eq!(chain(&neg(&[set("az")], None)), ["en"]); }
#[test] fn numbering_extension_kept_in_format_only() { let n = neg(&[os(&["tr-TR-u-nu-arab"])], None); assert_eq!(chain(&n), ["tr", "en"]); assert_eq!(n.format_locale, "tr-TR-u-nu-arab"); }
#[test] fn zh_hk_picks_hant() { assert_eq!(chain_with(&["zh-Hans", "zh-Hant"], &[os(&["zh-HK"])]), ["zh-Hant", "en"]); }
#[test] fn pseudo_needs_permission() { assert_eq!(chain(&neg(&[set("en-XA")], None)), ["en"]); assert_eq!(chain(&neg_pseudo(&[set("en-XA")])), ["en-XA", "en"]); }
// render.rs  (format locale en-US until Task 9)
#[test] fn missing_tr_message_falls_back_to_en() { let l = l10n(&["tr", "en"], &[("tr", "hv-a = A-tr")], &[("en", "hv-a = A\nhv-b = B")]); assert_eq!(l.t_dynamic("hv-b", vec![]), "B"); }
#[cfg(not(debug_assertions))] #[test] fn tr_typo_variable_falls_back_in_release() { let l = l10n(&["tr", "en"], &[("tr", "hv-r = { $cuont } satır")], &[("en", "hv-r = { $count } rows")]); assert_eq!(l.t_dynamic("hv-r", vec![("count", ArgValue::Count(2))]), "\u{2068}2\u{2069} rows"); }
#[cfg(debug_assertions)] #[test] fn tr_typo_variable_is_flagged_in_debug() { let l = l10n(&["tr", "en"], &[("tr", "hv-r = { $cuont } satır")], &[("en", "hv-r = { $count } rows")]); assert!(l.t_dynamic("hv-r", vec![("count", ArgValue::Count(2))]).starts_with("⚠[")); }
#[test] fn ui_sink_isolates_and_machine_sink_does_not() {
    const SAVED: MsgRef<NameArg> = MsgRef::new("hv-saved"); // test-local `struct NameArg(String)` implementing IntoFluentArgs as [("name", Text)]
    let l = l10n(&["tr", "en"], &[("tr", "hv-saved = Bağlantı { $name } kaydedildi.")], &[("en", "hv-saved = Connection { $name } saved.")]);
    assert_eq!(l.t(SAVED, NameArg("prod".into())), "Bağlantı \u{2068}prod\u{2069} kaydedildi.");
    assert_eq!(l.t_machine(SAVED, NameArg("prod".into())), "Bağlantı prod kaydedildi.");
}
#[test] fn stale_build_is_discarded() { let l = L10n::new(rt(1)); assert!(l.install_if_newer(rt(3))); assert!(!l.install_if_newer(rt(2))); assert_eq!(l.generation(), 3); }
#[test] fn reserved_generations_are_unique_and_increasing() { let l = L10n::new(rt(5)); let a = l.reserve_generation(); let b = l.reserve_generation(); assert!(a > 5 && b > a); assert!(l.install_if_newer(rt(b))); assert!(!l.install_if_newer(rt(a))); assert_eq!(l.generation(), b); }
#[test] fn typed_ids_reject_wrong_args() { trybuild::TestCases::new().compile_fail("tests/ui/wrong_args.rs"); }
#[cfg(not(debug_assertions))] #[test] fn cold_parse_en_tr_under_10ms() {
    let c = Catalogs::embedded(); let n = neg(&[os(&["tr-TR"])], None); let f = FormatInputs::new(&n, "UTC");
    let mut t: Vec<Duration> = (0..20u64).map(|g| { let s = Instant::now(); let _ = build_runtime(&c, &n, &f, g); s.elapsed() }).collect();
    t.sort(); assert!(t[10] <= Duration::from_millis(10), "median {:?}", t[10]);
}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-l10n -p hacivat-l10n-schema`. Expected: FAIL.
- [ ] **Step 3: Implement `extract` and `build.rs`.** Declare `cargo:rerun-if-changed=../../../locales`.
- [ ] **Step 4: Implement `Catalogs`, `negotiate` and `L10n`.**
  - Overlay safety limits (I18N-02 step 5): 1 MB per file, UTF-8 required, no symlinks leaving the root, only `*.ftl` and `locale.toml`.
  - Release builds fall back to the next bundle on a structural error. Debug builds return `⚠[text]`.
- [ ] **Step 5:** Run `cargo test -p hacivat-l10n -p hacivat-l10n-schema` and `cargo test --release -p hacivat-l10n` (the release-only fallback and budget tests, F10). Expected: PASS.
- [ ] **Step 6: Commit** with message `feat(l10n): catalogs, negotiation, typed IDs, per-message fallback, atomic swap`.

### Task 7: `hacivat-settings`: store, local paths, workspace state

**Files:**
- Create: `crates/core/hacivat-settings/src/{store,paths,workspace_state,fsio}.rs`
- Modify: `crates/core/hacivat-settings/src/{lib,schema}.rs`

**Interfaces:**
- Produces:
  - Store:
    - `load(path: &Path) -> LoadOutcome { settings: Settings, diags: Vec<SettingsDiag>, quarantined: Option<PathBuf> }`
    - `SettingsDiag { code: &'static str, key: String }`. Codes: `settings-invalid-value` (a known key with a bad value or type; the default is used) and `settings-newer-version` (`version` greater than 1). Unknown keys are kept and are not diagnosed. Task 13 shows each code as a toast (`hv-settings-invalid-value`, `hv-settings-newer-version`, F24).
    - `save_changes(path: &Path, new: &Settings) -> io::Result<()>`: a `toml_edit` merge that writes only keys whose value changed, keeps comments, unknown keys and a newer `version`, then an atomic write
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

- [ ] **Step 1: Write the failing tests** (Review Focus 3; F24 adds the last four).
```rust
#[test] fn defaults_when_file_missing() { let o = load(&tmp().join("settings.toml")); assert_eq!(o.settings, Settings::default()); assert!(o.diags.is_empty()); }
#[test] fn round_trip_keeps_unknown_keys_and_comments() {
    let p = write_tmp("# mine\nversion = 1\n[ui]\ntheme = \"light\"\nfuture_key = 3\n");
    let mut s = load(&p).settings; s.ui.theme = ThemePref::Dark; save_changes(&p, &s).unwrap();
    let t = std::fs::read_to_string(&p).unwrap(); assert!(t.contains("# mine") && t.contains("future_key = 3") && t.contains("theme = \"dark\""));
}
#[test] fn invalid_language_tag_falls_back_to_system() { let o = load(&write_tmp("[ui]\nlanguage = \"klingon!!\"\n")); assert_eq!(o.settings.ui.language, LanguagePref::System); assert_eq!(o.diags, vec![SettingsDiag { code: "settings-invalid-value", key: "ui.language".into() }]); }
#[test] fn wrong_type_uses_default() { let o = load(&write_tmp("[ui]\ntheme = 3\nlanguage = \"tr\"\n")); assert_eq!(o.settings.ui.theme, ThemePref::Dark); assert_eq!(o.settings.ui.language, LanguagePref::Tag("tr".into())); assert_eq!(o.diags, vec![SettingsDiag { code: "settings-invalid-value", key: "ui.theme".into() }]); }
#[test] fn unparsable_file_is_quarantined_not_overwritten() { let p = write_tmp("[[["); let o = load(&p); assert!(o.quarantined.unwrap().to_string_lossy().contains("settings.toml.corrupt-")); assert!(!p.exists()); assert_eq!(o.settings, Settings::default()); }
#[cfg(unix)] #[test] fn permissions_are_0700_and_0600() {
    let r = tmp(); let p = LocalPaths::under(&r); p.ensure_dirs().unwrap();
    assert_eq!(mode(&p.config), 0o700); assert_eq!(mode(&p.data), 0o700); assert_eq!(mode(&p.cache), 0o700);
    save_changes(&p.settings_toml(), &Settings::default()).unwrap(); assert_eq!(mode(&p.settings_toml()), 0o600); // mode(p) = metadata(p).permissions().mode() & 0o777
}
#[test] fn local_paths_follow_spec_table() {
    let p = LocalPaths::under(Path::new("/r")); let u = "6f1c…";
    assert_eq!(p.settings_toml(), Path::new("/r/config/settings.toml")); assert_eq!(p.history_db(), Path::new("/r/data/history.sqlite"));
    assert_eq!(p.catalog_db(u, 1), Path::new("/r/data/catalog").join(format!("{u}.v1.sqlite"))); assert_eq!(p.workspace_json(), Path::new("/r/data/workspace.json"));
    assert_eq!(p.logs_dir(), Path::new("/r/cache/logs")); assert_eq!(p.locales_overlay_dir(), Path::new("/r/config/locales"));
    assert_eq!(p.ipc_endpoint(Some("S-1-5-21-1")).to_string_lossy(), if cfg!(windows) { r"\\.\pipe\hacivat-S-1-5-21-1" } else { "/r/runtime/hacivat/broker.sock" });
}
#[test] fn future_workspace_version_is_set_aside() { let p = write_tmp_named("workspace.json", r#"{"version":2,"panes":{}}"#); assert!(workspace_state::load(&p, 1).is_none()); assert!(sibling_with_prefix(&p, "workspace.json.newer-")); assert!(!p.exists()); }
#[test] fn newer_settings_version_loads_known_keys_and_keeps_the_file() {
    let p = write_tmp("version = 2\n[ui]\ntheme = \"light\"\nnew_thing = true\n"); let o = load(&p);
    assert_eq!(o.settings.ui.theme, ThemePref::Light); assert_eq!(o.diags, vec![SettingsDiag { code: "settings-newer-version", key: "version".into() }]); assert!(o.quarantined.is_none());
    let mut s = o.settings; s.ui.theme = ThemePref::Dark; save_changes(&p, &s).unwrap();
    let t = std::fs::read_to_string(&p).unwrap(); assert!(t.contains("version = 2") && t.contains("new_thing = true") && t.contains("theme = \"dark\""));
}
#[test] fn wrong_typed_version_uses_default_and_keeps_the_file() { let p = write_tmp("version = \"one\"\n[ui]\ntheme = \"light\"\n"); let o = load(&p); assert_eq!(o.settings.version, 1); assert_eq!(o.settings.ui.theme, ThemePref::Light); assert_eq!(o.diags, vec![SettingsDiag { code: "settings-invalid-value", key: "version".into() }]); assert!(o.quarantined.is_none() && p.exists()); }
#[test] fn unparsable_workspace_json_is_set_aside_as_corrupt() { let p = write_tmp_named("workspace.json", "{not json"); assert!(workspace_state::load(&p, 1).is_none()); assert!(sibling_with_prefix(&p, "workspace.json.corrupt-")); assert!(!p.exists()); }
#[test] fn misspelled_key_is_kept_as_unknown() {
    let p = write_tmp("[ui]\nthem = \"light\"\n"); let o = load(&p); assert_eq!(o.settings.ui.theme, ThemePref::Dark); assert!(o.diags.is_empty());
    let mut s = o.settings; s.ui.theme = ThemePref::Light; save_changes(&p, &s).unwrap();
    let t = std::fs::read_to_string(&p).unwrap(); assert!(t.contains("them = \"light\"") && t.contains("theme = \"light\""));
}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-settings`. Expected: FAIL.
- [ ] **Step 3: Implement.** Validate each key independently, so one bad value never resets the others. Check language tags with `unic-langid` parsing only; this crate does not depend on l10n.
- [ ] **Step 4:** Run `cargo test -p hacivat-settings`. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(settings): versioned settings store, §5.6 paths, workspace state with set-aside`.

### Task 8: Window chrome and the paint-level One Filled Badge check

**Files:**
- Modify: `crates/ui/hacivat-ui-core/src/{window.rs, named_rules.rs}` (Task 1 created them with stubs and the complete `ChromeKind`)
- Create: `crates/ui/hacivat-ui-core/tests/{window.rs, named_rules.rs}`

**Interfaces:**
- Produces:
  - `pub struct WorkspaceWindowSpec { pub title: SharedString, pub size: (f32, f32), pub min_size: (f32, f32) }`. The default is 1440×900 with a minimum of 960×600.
  - `pub fn open_workspace_window<V: Render + 'static>(cx: &mut App, spec: WorkspaceWindowSpec, build: impl FnOnce(&mut Window, &mut App) -> Entity<V>) -> anyhow::Result<WindowHandle<V>>`:
    - macOS: transparent title bar, traffic lights at (14, 13)
    - Windows: `appears_transparent` with a custom caption
    - Linux: `window_decorations: Some(WindowDecorations::Client)`
  - `pub fn chrome_kind(window: &Window) -> ChromeKind` (`ChromeKind` and `TRAFFIC_LIGHTS_INSET_PX` are already complete from Task 1)
  - Under `scene-check`:
    - `pub fn count_solid_quads(window: &Window, color: Hsla) -> usize`
    - `pub fn assert_one_filled_badge(window: &Window, cp_fill: Hsla, expect_production: bool)`, which panics with the count

- [ ] **Step 1: Write the failing tests** (VisualTestContext, `--features test-support,scene-check`; line 2 of the full test set runs them, F2).
```rust
#[gpui::test] fn workspace_window_opens_with_min_size(cx: &mut TestAppContext) { let w = open_spec(cx, WorkspaceWindowSpec::default()).unwrap(); w.update(cx, |_, window, _| assert_eq!(window.bounds().size, size(px(1440.), px(900.)))).unwrap(); }
#[gpui::test] fn one_cp_fill_quad_counts_one(cx: &mut TestAppContext) { let (_, vcx) = draw(cx, |_| CpFillProbe { blocks: 1 }); vcx.update(|w, _| assert_eq!(count_solid_quads(w, DARK.colors.cp_fill.to_hsla()), 1)); }
#[gpui::test] #[should_panic(expected = "cp-fill quads: 2")] fn two_cp_fill_quads_fail(cx: &mut TestAppContext) { let (_, vcx) = draw(cx, |_| CpFillProbe { blocks: 2 }); vcx.update(|w, _| assert_one_filled_badge(w, DARK.colors.cp_fill.to_hsla(), true)); }
#[gpui::test] fn no_production_means_zero_quads(cx: &mut TestAppContext) { let (_, vcx) = draw(cx, |_| CpFillProbe { blocks: 0 }); vcx.update(|w, _| assert_one_filled_badge(w, DARK.colors.cp_fill.to_hsla(), false)); }
#[gpui::test] fn chrome_kind_per_os(cx: &mut TestAppContext) {
    let w = open_spec(cx, WorkspaceWindowSpec::default()).unwrap();
    w.update(cx, |_, window, _| { let k = chrome_kind(window);
        #[cfg(target_os = "macos")] assert_eq!(k, ChromeKind::MacTrafficLights);
        #[cfg(target_os = "windows")] assert_eq!(k, ChromeKind::WindowsCaption);
        #[cfg(target_os = "linux")] assert!(matches!(k, ChromeKind::LinuxClient | ChromeKind::LinuxServer), "{k:?}");
    }).unwrap();
}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-ui-core --features test-support,scene-check --test window --test named_rules`. Expected: FAIL.
- [ ] **Step 3: Implement.** `count_solid_quads` iterates over `window.rendered_scene().quads` and compares `background.as_solid()` within 1/255 per channel.
- [ ] **Step 4:** Run the same command. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(ui-core): per-OS workspace window chrome; paint-level One Filled Badge check`.

### Task 9: l10n formatting, settings model, pseudo-locales, search key, display casing

**Files:**
- Modify (Task 1 stubs): `crates/core/hacivat-l10n/src/{format,effective,pseudo,search,casing}.rs`
- Create: `crates/core/hacivat-l10n/tests/{format.rs, pseudo.rs, search.rs}`

**Interfaces:**
- Consumes: `build_runtime`, `FormatInputs` and the hook wiring (Task 6: `install_functions`, `with_format_ctx`, `transform_resource` and `wrap_message` are already called from `runtime.rs`/`render.rs`, so this task edits only its five files), and `hacivat_settings::Settings`. `iana-time-zone` is a dependency from Task 1.
- Produces:
  - Formatting:
    - `FormatCtx { pub format_locale: String, pub time_zone: String, pub generation: u64 }`, built by `build_runtime` from `FormatInputs` (F7)
    - `with_format_ctx<R>(ctx: &FormatCtx, f: impl FnOnce() -> R) -> R` (re-entrant)
    - `install_functions(bundle: &mut FluentBundle<..>)`: the `NUMBER`, `DATETIME` and `LIST` functions and the `hv_format` formatter. `DATETIME` converts `WhenValue::Instant` to `FormatCtx.time_zone` and formats `WhenValue::Local` as given (F30).
  - Settings model:
    - `OsLocale { languages: Vec<String>, format: Option<String>, time_zone: Option<String>, env: Vec<(&'static str, String)> }` with `OsLocale::detect() -> OsLocale` (`sys-locale`, env, `iana-time-zone`)
    - `effective(s: &Settings, os: &OsLocale, allow_pseudo: bool) -> EffectiveLocale { ctx: NegotiationCtx, time_zone: String }`; the caller builds `FormatInputs::new(&negotiate(&e.ctx, ..), &e.time_zone)`
  - Pseudo-locales:
    - `PseudoMode { EnXa, TrXt }` with `PseudoMode::for_tag(tag: &LanguageIdentifier) -> Option<PseudoMode>`
    - `transform_literal(s: &str, m: PseudoMode) -> Cow<'_, str>`
    - `transform_resource(res: &mut ast::Resource<String>, m: PseudoMode)`: transforms every text element of every message, and leaves terms whose ID starts with `-brand-` untouched (F6: the brand is never pseudo-transformed)
    - `wrap_message(s: String) -> String` (adds `[` and `]`)
    - The pseudo locales are built from the `en` source when `Negotiated.chain[0]` is `en-XA` or `tr-XT`; `for_tag` is the only switch.
  - Search key:
    - `SearchOpts { pub turkish_tolerant: bool, pub mode: MatchMode }`
    - `MatchMode { Contains, WordPrefix, Fuzzy }`
    - `build_key(text: &str, o: &SearchOpts) -> SearchKey`
    - `find(text: &SearchKey, query: &str, o: &SearchOpts) -> Option<Match { ranges: Vec<Range<usize>>, score: u32, kind: MatchKind }>`, with byte ranges in the original text extended to grapheme boundaries
    - `MatchKind { ExactWord, Prefix, Substring, Fuzzy }`
    - `turkish_tolerant_for(ui_lang: &str, format_locale: &str) -> bool`
  - `case_for_display(text: &str, lang: &LanguageIdentifier) -> String`, using ICU4X `CaseMapper::uppercase`. It has no M1 caller; it is the sanctioned casing path named in Global Constraints (INV-3) for M2's error renderers (F33).

- [ ] **Step 1: Write the failing tests** (the I18N-05 and I18N-06 normative values, the ALG-I18N-06 rows, and the I18N-16 rows).
```rust
#[test] fn number_tr_and_en() { assert_eq!(fmt("tr-TR", "{ NUMBER($n) }", n(1234.5)), "1.234,5"); assert_eq!(fmt("en-US", "{ NUMBER($n) }", n(1234.5)), "1,234.5"); }
#[test] fn percent_patterns() { assert_eq!(fmt("tr-TR", "{ NUMBER($n, style: \"percent\") }", n(0.25)), "%25"); assert_eq!(fmt("en-US", "{ NUMBER($n, style: \"percent\") }", n(0.25)), "25%"); }
#[test] fn ports_never_grouped() { assert_eq!(fmt("tr-TR", "{ NUMBER($n, useGrouping: \"false\") }", n(5432.0)), "5432"); }
#[test] fn dates_in_istanbul() { assert_eq!(fmt_tz("tr-TR", "Europe/Istanbul", "{ DATETIME($t, dateStyle: \"long\") }", t("2026-11-11T14:05:09Z")), "11 Kasım 2026"); assert_eq!(fmt_tz("tr-TR", "Europe/Istanbul", "{ DATETIME($t, dateStyle: \"short\") }", t("2026-11-11T14:05:09Z")), "11.11.2026"); }
#[test] fn instant_converts_to_zone_and_local_does_not() { assert_eq!(fmt_tz("tr-TR", "Europe/Istanbul", "{ DATETIME($t, timeStyle: \"short\") }", t("2026-11-11T14:05:09Z")), "17:05"); assert_eq!(fmt_tz("tr-TR", "Europe/Istanbul", "{ DATETIME($t, timeStyle: \"short\") }", local(2026, 11, 11, 14, 5)), "14:05"); }
#[test] fn lists() { assert_eq!(fmt("tr-TR", "{ LIST($l) }", l(&["a", "b", "c"])), "a, b ve c"); assert_eq!(fmt("en-US", "{ LIST($l) }", l(&["a", "b", "c"])), "a, b, and c"); }
#[test] fn plural_by_ui_language_digits_by_format_locale() { assert_eq!(render(&["en"], "tr-TR", "hv-r", "{ $count -> [one] { $count } row *[other] { $count } rows }", 1234), "1.234 rows"); assert_eq!(render(&["en"], "tr-TR", "hv-r", SAME, 1), "1 row"); }
#[test] fn turkish_count_message() { assert_eq!(render_tr("{ $count } satır", 1234.5, "tr-TR"), "1.234,5 satır"); }
#[test] fn unknown_option_keeps_default_formatting() { assert_eq!(fmt("en-US", "{ NUMBER($n, currency: \"TRY\") }", n(3.0)), "3"); }
#[test] fn english_ui_turkish_region_is_primary_case() { let e = effective(&settings_lang("en"), &os(&["tr-TR"], Some("tr-TR")), false); assert_eq!(negotiate(&e.ctx, &avail()).format_locale, "tr-TR"); }
#[test] fn time_zone_setting_beats_os() { let e = effective(&settings_tz("Europe/Istanbul"), &os_tz(&["en-US"], "America/New_York"), false); assert_eq!(e.time_zone, "Europe/Istanbul"); let e = effective(&Settings::default(), &os_tz(&["en-US"], "America/New_York"), false); assert_eq!(e.time_zone, "America/New_York"); }
#[test] fn pseudo_doubles_short_strings() { let s = transform_literal("Save", PseudoMode::EnXa); assert!(graphemes(&s) >= 8 && s.chars().any(|c| !c.is_ascii())); }
#[test] fn pseudo_skips_access_keys_markup_urls() { assert_eq!(transform_literal("S", PseudoMode::EnXa), "S"); assert_eq!(transform_literal("<code>SELECT 1</code>", PseudoMode::EnXa), "<code>SELECT 1</code>"); assert_eq!(transform_literal("https://hacivat.dev", PseudoMode::EnXa), "https://hacivat.dev"); }
#[test] fn tr_xt_injects_turkish_letters() { let s = transform_literal("Find in table", PseudoMode::TrXt); assert!(s.chars().any(|c| "ıİşğçöü".contains(c))); assert!(graphemes(&s) >= 21); }
#[test] fn pseudo_keeps_placeables_and_brackets() { let s = render_pseudo("hv-n", "{ $count } rows", 5); assert!(s.starts_with("[\u{2068}5\u{2069}") && s.ends_with(']')); assert!(!s.contains("rows")); }
#[test] fn pseudo_leaves_brand_terms() { assert_eq!(render_pseudo_with_terms("cm-brand-hacivat-caps", "-brand-hacivat-caps = HACIVAT\ncm-brand-hacivat-caps = { -brand-hacivat-caps }"), "[HACIVAT]"); }
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
- [ ] **Step 4:** Run `cargo test -p hacivat-l10n --features pseudo` and `LANG=tr_TR.UTF-8 LC_ALL=tr_TR.UTF-8 cargo test -p hacivat-l10n --features pseudo`. Then re-measure the parse budget now that the ICU formatters exist (F10): `cargo test --release -p hacivat-l10n --features pseudo cold_parse`. Expected: PASS all three ways.
- [ ] **Step 5: Commit** with message `feat(l10n): ICU4X NUMBER/DATETIME/LIST, locale settings model, pseudo-locales, Turkish-tolerant search`.

### Task 10: `xtask l10n check` (I18N-17) and the literal-string lint (I18N-23)

**Files:**
- Create: `xtask/src/l10n/{mod.rs, rules.rs, code_scan.rs}`, `xtask/tests/fixtures/l10n/**`, `locales/_schema/literal-allowlist.toml`
- Modify: `xtask/src/main.rs` (arm `l10n`). The dependencies (`hacivat-l10n-schema`, `fluent-syntax`, `icu_plurals`) are in place from Task 1.

**Interfaces:**
- Consumes: `hacivat_l10n_schema::extract` (each `SchemaError` becomes a finding; `extract` returns the signatures it could read alongside the errors, so one bad message does not hide the rest).
- Produces:
  - `cargo xtask l10n check [--root <dir>] [--json <out>]` checks the rule set in R10. It writes the human report to stdout, writes `target/l10n-report.json`, and exits 1 on any error.
  - `pub struct Finding { pub rule: &'static str, pub severity: Severity, pub locale: String, pub file: String, pub line: u32, pub id: String }`
  - `pub fn check(root: &Path, code_roots: &[&Path]) -> Vec<Finding>`
  - `code_scan` covers the following:
    - **L024:** string literals that match the ID grammar but are not in `en`, and `ids::<ns>::<CONST>` references to missing messages.
    - **L025:** English IDs with no reference.
    - **I18N-23 rule 1:** `format!("{} {}", …t(…), …)`-shaped concatenation.
    - **Hard-coded UI literals:** a string literal with an alphabetic character passed to `.child(`, `Label::new(` with `TextSource::Fluent`, or `SharedString::from(`, in `crates/ui/hacivat-ui/src` and `crates/app/*/src`, unless the literal is listed in `literal-allowlist.toml`. Reported as rule `L900`, error.
  - `literal-allowlist.toml` is created here and is append-only afterwards: each later task appends the entries for its own literals (for example the demo capsule's `orders-prod` and `PostgreSQL 16`, `…`, `×`) under a `# <task slug>` comment (F19).

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
#[test] #[ignore = "repo-wide: run by CI l10n-check and the gate (F19)"] fn repo_catalogs_are_clean() { assert!(errors(&check(&workspace_root().join("locales"), &repo_code_roots())).is_empty()); }
```
  `repo_catalogs_are_clean` is ignored by default because it scans code that later waves are still writing; the CI `l10n-check` job runs it with `--ignored`, and so does the gate.
- [ ] **Step 2:** Run `cargo test -p xtask l10n`. Expected: FAIL.
- [ ] **Step 3: Implement the rules and the scanner.** The code scanner is a token scan; `syn` is not needed. L013 reads CLDR cardinal categories from `icu_plurals`.
- [ ] **Step 4:** Run `cargo test -p xtask l10n && cargo test -p xtask l10n -- --ignored repo_catalogs_are_clean && cargo xtask l10n check`. Expected: PASS, and `l10n-check: 0 errors`.
- [ ] **Step 5: Commit** with message `feat(xtask): l10n check (I18N-17 subset) and hard-coded string lint`.

### Task 11: `hacivat-ui` text and input: provider, Label registry, TextInput (IME), Palette view

**Files:**
- Modify (Task 1 working minimal versions): `crates/ui/hacivat-ui/src/{l10n.rs, text.rs}`
- Create: `crates/ui/hacivat-ui/src/{input/{mod.rs, ime.rs}, palette.rs}`, `crates/ui/hacivat-ui/tests/{text.rs, input.rs, palette.rs}`
- Modify: `locales/{en,tr}/hui.ftl` (append)

**Interfaces:**
- Consumes: `Theme`, `font_for`, `small_caps`, `popover_surface`, `focus_outline` (ui-core; the surface and focus ring are not re-implemented here, F26) and the Task 1 types (`TextFn`, `InputEvent`, `FocusedInput`).
- Produces:
  - `EnglishDefaults: UiTextProvider` parses the embedded `locales/en/hui.ftl` once, with no ICU dependency. Unknown IDs render `⟦id⟧` in debug builds. Task 13 tests that it renders every `hui-*` message exactly as `L10n` does (F26).
  - Text registry:
    - `RenderedText { text, source: TextSource, size_px: f32, clipped: bool, truncated: bool, path: SharedString }`
    - `TextRegistry { pub enabled: bool, pub frame: Vec<RenderedText> }`, a `Global`
    - `Label` becomes a custom `Element` (replacing Task 1's minimal `div`, same constructor). It shapes with the window text system and ellipsizes with `…` when `truncate`. It records into the registry when enabled, with `clipped = !truncate && shaped_width > bounds.width + 0.5`. A truncated label carries its full text as tooltip. A `Caps::SynthesizedSmallCaps` style routes through `fonts::small_caps`.
  - IME:
    - `ime::ImeState`, a pure model: `new(text)`, `replace(range_utf16: Option<Range<usize>>, text)`, `replace_and_mark(range, text, sel)`, `unmark()`, `marked_utf16()`, `selection_utf16()`, `text()`, `backspace()` (one grapheme), `move_left()`, `move_right()`, `select_all()`. The caret starts at the end of the text (M0 ruling R4).
  - `TextInput`, an entity with `new(cx, placeholder: TextFn)` (F33: a closure, so `hv-*` placeholders work as well as `hui-*`):
    - `value() -> &str`, `set_value(&mut self, s)`
    - `is_composing() -> bool`
    - `impl EntityInputHandler`, with `bounds_for_range` taken from the shaped line
    - `on_blur` calls `unmark()`, which commits
    - emits `InputEvent::{Changed, Submitted, Cancelled, CompositionChanged(bool)}`; `CompositionChanged(true)` on the first mark of a composition, `CompositionChanged(false)` when it commits or is cancelled (F15)
    - keeps the `FocusedInput` global current: `entity` on focus and blur, `composing` on mark and unmark
    - 26 px tall, radius 4, seam-strong outline, `focus_outline` when focused
  - Palette:
    - `PaletteItem { id: SharedString, title: SharedString, keys: Option<SharedString>, highlight: Vec<Range<usize>> }`
    - `Palette`, an entity holding a `TextInput` plus the result list. Rows are 23 px with the active row in `blue_tint_strong`; there is a 24 px footer with key hints. The surface is `popover_surface(theme)`.
    - `Palette::set_items(&mut self, items: Vec<PaletteItem>)`, `query() -> String`, `input() -> &Entity<TextInput>`
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
#[gpui::test] fn blur_commits_composition(cx: &mut TestAppContext) {
    let (input, other, vcx) = focused_input_and_other(cx); let events = record_input_events(&input, vcx);
    input.update_in(vcx, |i, w, cx| i.replace_and_mark_text_in_range(None, "ş", None, w, cx));
    assert!(input.read_with(vcx, |i, _| i.is_composing())); assert!(vcx.read(|cx| cx.global::<FocusedInput>().composing));
    vcx.update(|w, _| other.focus(w)); vcx.run_until_parked();
    input.read_with(vcx, |i, _| { assert_eq!(i.value(), "ş"); assert!(!i.is_composing()); });
    assert!(!vcx.read(|cx| cx.global::<FocusedInput>().composing));
    let ev = events.take(); assert_eq!(ev.iter().filter(|e| matches!(e, InputEvent::CompositionChanged(_))).collect::<Vec<_>>(), [&InputEvent::CompositionChanged(true), &InputEvent::CompositionChanged(false)]);
}
#[gpui::test] fn clipped_label_is_recorded(cx: &mut TestAppContext) { let reg = draw_label_in(cx, "A very long label", 40.0, false); assert!(reg.frame[0].clipped); let reg = draw_label_in(cx, "A very long label", 40.0, true); assert!(reg.frame[0].truncated && !reg.frame[0].clipped); }
#[gpui::test] fn palette_keyboard_navigation(cx: &mut TestAppContext) {
    let (pal, vcx) = palette_with(cx, &["a", "b", "c"]); let events = record_palette_events(&pal, vcx);
    vcx.simulate_keystrokes("down down enter"); assert_eq!(events.take(), vec![PaletteEvent::Confirmed("c".into())]);
    vcx.simulate_keystrokes("up enter"); assert_eq!(events.take(), vec![PaletteEvent::Confirmed("b".into())]);
    vcx.simulate_keystrokes("escape"); assert_eq!(events.take(), vec![PaletteEvent::Dismissed]);
}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-ui --test text --test input --test palette`. Expected: FAIL.
- [ ] **Step 3: Implement.** Re-implement the M0 `ImeState` ideas (do not depend on `spikes/`).
- [ ] **Step 4:** Run the same tests. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(ui): UI text provider, Label registry, IME-correct TextInput, palette view`.

### Task 12: `hacivat-ui` chrome and containers

**Files:**
- Create: `crates/ui/hacivat-ui/src/{button,segmented,icon,popover,panes,dialog,toast,menu,accesskey,statusbar,capsule,titlebar}.rs`, `crates/ui/hacivat-ui/tests/{panes.rs, chrome.rs, dialog.rs, accesskey.rs, a11y.rs}`
- Modify: `locales/{en,tr}/hui.ftl` (append)

**Interfaces:**
- Consumes: `Label`, `TextSource`, `TextFn`, `KeyFn`, `ui_text` (Task 1's working versions; Task 11 refines `Label` in the same wave without changing its API, F4); `Theme`, `small_caps`, `popover_surface`, `focus_outline`, `BundledAssets`, `ChromeKind`, `TRAFFIC_LIGHTS_INSET_PX`, `Role` (Tasks 1 and 4).
- Produces (metrics per Global Constraints and DESIGN.md Components; focus rings use `focus_outline` and floating surfaces use `popover_surface`, F26):
  - `Button::new(id, label: SharedString)`, with `.kind(ButtonKind::{Default, Primary, Ghost, Danger, Icon})`, `.icon(IconName)`, `.key_hint(SharedString)`, `.disabled(bool)` and `.on_click(..)`
  - `Segmented::new(id).item(SegmentItem { content: SegmentContent::{Label(SharedString), Icon(IconName)}, pressed: bool, tooltip: SharedString, on_click })`
  - `Icon::new(IconName).size(IconSize::{Chrome /*12*/, Default /*14*/})`
  - Panes:
    - `PaneId(u32)`, `TabId(u32)`, `PaneNode { Leaf(PaneId), Split { axis: Axis, ratio: f32, a: Box<PaneNode>, b: Box<PaneNode> } }`
    - `Pane { tabs: Vec<TabId>, active: usize }`
    - `PaneTree { root: PaneNode, panes: BTreeMap<PaneId, Pane>, focused: PaneId }`, serde-serializable
    - Methods: `new(first_tab)`, `split(pane, axis, new_tab) -> PaneId`, `close_tab(pane, tab) -> Option<TabId>` (the newly active tab), `move_focus(dir: FocusDir)`, `set_ratio(path: &[bool], ratio)` (clamped to [0.1, 0.9]; the view additionally keeps each side ≥ 120 px), `activate(pane, idx)`, `next_tab()`, `prev_tab()`
  - `TabStrip::new(pane, tabs: Vec<TabSpec { id, title: SharedString, title_source: TextSource, dirty: bool, env: Option<EnvMark> }>)`, with 30 px tabs; the active tab is raised. (The strip is rebuilt every frame from IDs, so its titles may be rendered strings.)
  - Dialogs (a view; the host places it, which in the app is Task 13's `Layers`):
    - `DialogSpec { title: TextFn, body: TextFn, buttons: Vec<DialogButton { label: TextFn, kind: ButtonKind, accesskey: Option<KeyFn>, english_accesskey: Option<char> }>, cancel: usize, primary: usize }` (F16: closures, re-read on every render)
    - `Dialog::new(spec, window, cx) -> Entity<Dialog>`: focus trap (Tab and Shift-Tab cycle the buttons), initial focus on `primary`, Esc → cancel, Enter → primary, Alt+access key on Windows/Linux. Emits `DialogEvent::{Pressed(usize), Cancelled}`.
  - Toasts:
    - `Toast { text: TextFn, kind: ToastKind::{Info, Error}, ttl: Duration }`
    - `ToastQueue`, a `Global`, with `push(cx, toast)`: at most 3 visible, bottom right above the status bar, 6 s TTL, no animation
  - Popup menus (F22: `PopupMenu`, so it never collides with GPUI's application `Menu`):
    - `MenuItemSpec { label: TextFn, keys: Option<TextFn>, accesskey: Option<KeyFn>, disabled: bool, action: SharedString }` plus separators (F16)
    - `PopupMenu::new(anchor: Point<Pixels>, items: Vec<MenuItemSpec>, window, cx) -> Entity<PopupMenu>`, with arrow, Enter and Esc navigation. Emits `PopupMenuEvent::{Chosen(SharedString), Dismissed}`.
  - `AccessKeyAssigner::assign(labels: &[(&str, Option<char>, Option<char>)]) -> Vec<AccessKey::{InLabel(usize), Suffix(char), None}>` (I18N-23 rule 5). Letters compare with `char::eq_ignore_ascii_case` for ASCII and by exact match for non-ASCII (F32; the casing ban stays intact and `hacivat-ui` needs no l10n dependency).
  - `StatusBar::new().left(seg).right(seg)`: 24 px on ground, 12 px `text_2`, seams between segments
  - Capsule:
    - `CapsuleModel { Placeholder { label: SharedString }, Connection { env: EnvKind, env_label: SharedString, name: SharedString, engine: SharedString, access: SharedString, path: SharedString } }`
    - `EnvKind { Production, Staging, Local, Other }`
    - `Capsule::new(model)`: 26 px, radius 4. The production block is the only `cp_fill` quad; its `capsule_env` small caps come from a pre-cased label.
  - Title bar:
    - `TitleBar::new(chrome: ChromeKind)` with `.brand(word_caps: SharedString)`, `.capsule(Capsule)`, `.palette_field(placeholder: SharedString, keys: SharedString, on_click)`, `.right(Vec<AnyElement>)` and `.app_menu(on_click)` (Windows/Linux)
    - 40 px tall. Draggable via `start_window_move`; double-click zooms. Windows caption buttons use `WindowControlArea`.
  - `titlebar_fit(width: f32, fixed: f32, palette_pref: f32, palette_min: f32, segments: f32) -> TitleBarFit { segments_in_overflow: bool, palette_width: f32, palette_truncates: bool }` (F13). With `available = width - fixed`: if `available ≥ palette_pref + segments`, segments stay inline and `palette_width = palette_pref`. Otherwise the segments move into the overflow menu (a `OVERFLOW_BUTTON_PX = 24.0` button), `palette_width = min(palette_pref, available - 24)`, and `palette_truncates = palette_width < palette_min`. So the palette label truncates only after the segments have collapsed (the Overflow Rule).
  - Accessibility (F25; spec §4 Accessibility): every widget sets an AccessKit role and an accessible name through GPUI's `.role(..)`/`.aria_label(..)`: `Button` → `Role::Button` named by its label (an icon button by its tooltip); `Dialog` → `Role::Dialog` named by its title, modal; `PopupMenu` → `Role::Menu` with `Role::MenuItem` children named by their labels; `Toast` → `Role::Status` (Info) or `Role::Alert` (Error) named by its text; `TabStrip` → `Role::TabList` with `Role::Tab` children named by their titles, the active one selected.
  - `hui.ftl` (EN): `hui-dialog-close = Close`, `hui-toast-dismiss = Dismiss`, `hui-menu-more = More`, `hui-tab-close = Close tab`, `hui-window-minimize = Minimize`, `hui-window-maximize = Maximize`, `hui-window-close = Close window`.

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn split_then_close_collapses() { let mut t = PaneTree::new(TabId(1)); let p2 = t.split(PaneId(0), Axis::Horizontal, TabId(2)); assert_eq!(t.close_tab(p2, TabId(2)), None); assert_eq!(t.root, PaneNode::Leaf(PaneId(0))); }
#[test] fn closing_active_tab_activates_left_neighbour() { let mut t = tree_with_tabs(&[1, 2, 3], 2); assert_eq!(t.close_tab(PaneId(0), TabId(3)), Some(TabId(2))); }
#[test] fn pane_tree_round_trips_json() { let t = sample_tree(); assert_eq!(serde_json::from_value::<PaneTree>(serde_json::to_value(&t).unwrap()).unwrap(), t); }
#[test] fn ratio_is_clamped() { let mut t = sample_tree(); t.set_ratio(&[], 0.01); assert_eq!(ratio(&t), 0.1); t.set_ratio(&[], 0.99); assert_eq!(ratio(&t), 0.9); }
#[test] fn titlebar_collapses_segments_before_truncating_palette() {
    assert_eq!(titlebar_fit(1440.0, 600.0, 360.0, 220.0, 180.0), TitleBarFit { segments_in_overflow: false, palette_width: 360.0, palette_truncates: false });
    assert_eq!(titlebar_fit(1024.0, 600.0, 360.0, 220.0, 180.0), TitleBarFit { segments_in_overflow: true, palette_width: 360.0, palette_truncates: false });
    assert_eq!(titlebar_fit(1100.0, 600.0, 360.0, 220.0, 180.0), TitleBarFit { segments_in_overflow: true, palette_width: 360.0, palette_truncates: false });
    assert_eq!(titlebar_fit(800.0, 600.0, 360.0, 220.0, 180.0), TitleBarFit { segments_in_overflow: true, palette_width: 176.0, palette_truncates: true });
}
#[test] fn access_keys_turkish_menu() { assert_eq!(keys(&[("Kaydet", None, None), ("Kapat", None, None), ("Kopyala", None, None)]), ['K', 'a', 'o']); assert_eq!(keys(&[("Kaydet", None, None), ("Kopyala", Some('p'), None)]), ['K', 'p']); }
#[test] fn access_keys_ascii_ignore_case_non_ascii_exact() { assert_eq!(keys(&[("Save", None, None), ("save as", None, None)]), ['S', 'a']); assert_eq!(keys(&[("Işık", None, None), ("ışık", None, None)]), ['I', 'ı']); assert_eq!(keys(&[("Çık", None, None), ("çiz", None, None)]), ['Ç', 'ç']); }
#[test] fn access_key_suffix_when_no_letters() { assert_eq!(AccessKeyAssigner::assign(&[("保存", None, Some('S'))]), vec![AccessKey::Suffix('S')]); }
#[gpui::test] fn dialog_traps_focus_and_escape_cancels(cx: &mut TestAppContext) {
    let (d, vcx, events) = open_test_dialog(cx, spec_two_buttons(/* cancel */ 0, /* primary */ 1)); // test host renders the dialog alone in a window
    assert_eq!(focused_button(&d, &vcx), Some(1));
    vcx.simulate_keystrokes("tab"); assert_eq!(focused_button(&d, &vcx), Some(0));
    vcx.simulate_keystrokes("tab"); assert_eq!(focused_button(&d, &vcx), Some(1));
    vcx.simulate_keystrokes("shift-tab shift-tab shift-tab"); assert_eq!(focused_button(&d, &vcx), Some(0));
    vcx.simulate_keystrokes("escape"); assert_eq!(events.take(), vec![DialogEvent::Cancelled]);
}
#[gpui::test] fn dialog_enter_presses_primary(cx: &mut TestAppContext) { let (_, vcx, events) = open_test_dialog(cx, spec_two_buttons(0, 1)); vcx.simulate_keystrokes("enter"); assert_eq!(events.take(), vec![DialogEvent::Pressed(1)]); }
#[gpui::test] fn status_bar_and_tabs_use_tokens(cx: &mut TestAppContext) {
    let vcx = draw_chrome_probe(cx); // StatusBar with 2 segments and a TabStrip with 2 tabs, each tagged with debug_selector
    assert_eq!(vcx.debug_bounds("status-bar").unwrap().size.height, px(24.)); assert_eq!(vcx.debug_bounds("tab-strip").unwrap().size.height, px(30.));
    assert_eq!(vcx.debug_bounds("tab-1").unwrap().size.height, px(30.)); assert_eq!(vcx.debug_bounds("tab-2").unwrap().size.height, px(30.));
}
// tests/a11y.rs: a11y_nodes(vcx) parses window.debug_a11y_tree_json() (the M0 a11y probe's source) into (Role, name, selected) triples
#[gpui::test] fn chrome_exposes_roles_and_names(cx: &mut TestAppContext) {
    let vcx = draw(cx, |_, _| a11y_probe()); // Button "Save", TabStrip ["Console 1" (active), "Console 2"], Info toast "Saved", Error toast "Failed"
    let n = a11y_nodes(&vcx);
    assert!(n.contains(&(Role::Button, "Save".into(), false))); assert!(n.iter().any(|x| x.0 == Role::TabList));
    assert!(n.contains(&(Role::Tab, "Console 1".into(), true))); assert!(n.contains(&(Role::Tab, "Console 2".into(), false)));
    assert!(n.contains(&(Role::Status, "Saved".into(), false))); assert!(n.contains(&(Role::Alert, "Failed".into(), false)));
}
#[gpui::test] fn dialog_and_popup_menu_expose_roles_and_names(cx: &mut TestAppContext) {
    let vcx = open_test_dialog_titled(cx, "Delete?", &["Cancel", "Delete"]); let n = a11y_nodes(&vcx);
    assert!(n.contains(&(Role::Dialog, "Delete?".into(), false))); assert!(n.contains(&(Role::Button, "Cancel".into(), false))); assert!(n.contains(&(Role::Button, "Delete".into(), false)));
    let vcx = open_test_menu(cx, &["Copy", "Paste"]); let n = a11y_nodes(&vcx);
    assert!(n.iter().any(|x| x.0 == Role::Menu)); assert!(n.contains(&(Role::MenuItem, "Copy".into(), false))); assert!(n.contains(&(Role::MenuItem, "Paste".into(), false)));
}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-ui --test panes --test chrome --test dialog --test accesskey --test a11y`. Expected: FAIL. (The capsule's single `cp-fill` quad is asserted in Task 13, once Task 8's counter has merged.) If the test platform leaves the AccessKit tree inactive, the `a11y_nodes` helper activates it the way the M0 probe did; record that in the commit message.
- [ ] **Step 3: Implement.** Every string comes in as a parameter, a `TextFn` or a `hui-*` ID; none is literal (Task 10's L900 lint applies).
- [ ] **Step 4:** Run the same tests. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(ui): buttons, segmented, panes/tabs, dialog, toast, popup menu, access keys, status bar, capsule, title bar, AccessKit roles`.

### Task 13: App shell, its extension points, and the `hacivat` binary

**Files:**
- Create: `crates/app/hacivat-app/src/{l10n_global.rs, theme_pref.rs, shell/{mod,workspace,tiles,tabs,layers,tab_content,hooks,keymap_state}.rs}`, `crates/app/hacivat-app/tests/{shell.rs, support/mod.rs}`, `crates/bin/hacivat/src/{main.rs, logging.rs, smoke.rs}`
- Modify: `crates/app/hacivat-app/src/lib.rs` (`init` and its registration block), `locales/{en,tr}/hacivat-ui.ftl` (append)

**Interfaces:**
- Consumes: Tasks 4–12 (including `keymaps::load`, `layouts_altgr`, `altgr_collisions` from Task 5 and `platform::resident_set_mib` from Task 4).
- Produces:
  - L10n global:
    - `L10nGlobal { pub l10n: Arc<L10n>, pub catalogs: Arc<Catalogs> }`, a `Global`
    - `install_l10n(cx: &mut App, l10n: Arc<L10n>, catalogs: Arc<Catalogs>)`, which also installs `UiText(Arc::new(L10nUiProvider))` resolving `hui-*` through the chain
    - `switch_locale(cx: &mut App, settings: &Settings) -> Task<()>`: reserves a generation with `l10n.reserve_generation()` at call time (F8), builds the runtime with `build_runtime(.., &FormatInputs::new(&neg, &eff.time_zone), generation)` on `cx.background_spawn` (F7), then `install_if_newer`, `cx.refresh_windows()`, re-sets window titles and calls `menus::rebuild(cx)` (Task 1 stub; Task 16 fills it, F15)
  - Theme:
    - `theme_mode_for(pref: ThemePref, appearance: WindowAppearance) -> ThemeMode`
    - `apply_theme(cx: &mut App, pref: ThemePref)`, which observes the appearance for `System`
  - `AppState { paths: LocalPaths, settings: Settings }`, a `Global`
  - Keymap state (`shell/keymap_state.rs`, F11): `ActiveKeymap { pub preset: KeymapPreset, pub platform: KeymapPlatform, pub layout: KeyboardLayoutId, pub keymap: Arc<Keymap> }`, a `Global` installed at startup from `keymaps::load` and updated on `cx.on_keyboard_layout_change`; `set_keymap_preset(cx: &mut App, preset: KeymapPreset) -> Vec<Collision>` reloads it and returns the AltGr collisions on the current layout (`altgr_collisions(&km, &layouts_altgr())` filtered to `layout`). Task 16's dispatcher observes it; Task 17 calls `set_keymap_preset`.
  - **Extension points (F3)**, so Tasks 16 and 17 touch only their own files plus one registration line each:
    - `Layers` (`shell/layers.rs`), owned by `Workspace`: a stack of `Layer { id: &'static str, context: &'static str, view: AnyView, restore_focus: Option<FocusHandle> }` drawn above the tiles in stack order. `Workspace::push_layer(&mut self, layer: Layer, window, cx)` records the currently focused handle as `restore_focus` when the layer gives none; `Workspace::pop_layer(&mut self, window, cx) -> Option<&'static str>` removes the topmost layer and restores its focus; `Workspace::layer_ids(&self) -> Vec<&'static str>`. Esc with a non-empty stack pops the topmost layer, unless the layer's view handles Esc first (a dialog cancels, then its host pops it).
    - `Workspace::open_dialog(&mut self, spec: DialogSpec, window, cx)`: wraps `Dialog::new` in a layer (context `Dialog`) and pops it on `DialogEvent`; `Workspace::open_popup_menu(&mut self, anchor, items, window, cx)` does the same for `PopupMenu` (context `Menu`) and runs the chosen action through `ActionRegistry::run`.
    - `TabContentRegistry` (`shell/tab_content.rs`), a `Global`: `register(&mut self, kind: &'static str, content: TabContent { title: TextFn, build: Rc<dyn Fn(&mut Window, &mut App) -> AnyView>, context: &'static str })` and `get(&self, kind) -> Option<&TabContent>`. `TabKind { Console { n: u32 }, Registered(&'static str) }`; `Workspace::open_registered_tab(&mut self, kind, window, cx)` focuses an existing tab of that kind or opens one, and a kind with no registration renders an empty state.
    - `ShellHooks` (`shell/hooks.rs`), a `Global` with one slot: `key_capture: Option<KeyCaptureHook>`, where `KeyCaptureHook = Rc<dyn Fn(&KeyDownEvent, &ContextStack, &mut Window, &mut App) -> bool>`. The workspace root's `capture_key_down` calls it with `Workspace::context_stack`; `true` stops propagation. `set_key_capture(cx, hook)` fills the slot.
    - `Workspace::context_stack(&self, window: &Window) -> ContextStack`: `["Workspace", <active tab's context: "Console" or the registered content's>, <each layer's context, bottom to top>]`.
    - `Workspace` commands as methods, so Task 16's actions stay thin: `new_console_tab`, `close_active_tab`, `activate_next_tab`, `activate_prev_tab`, `split_focused(axis)`, `focus_next_pane`, `toggle_explorer`, `tab_count`, `active_tab_kind`.
    - `pub fn init(cx: &mut App)`: installs the globals above and ends with the registration block, which is append-only (one line per later task): `// registrations (append-only)`. Task 16 adds `commands::install(cx);`, Task 17 adds `settings_ui::install(cx);`.
  - `open_workspace(cx: &mut App, demo: Option<DemoCapsule>) -> Result<WindowHandle<Workspace>>`
  - `Workspace` (`Render`), which owns `TileLayout { explorer_w: 276.0, conn_panel_h: 205.0, editor_h: 322.0, inspector_w: 252.0, explorer_visible: true }`, `PaneTree`, `tabs: BTreeMap<TabId, TabKind>`, `CapsuleModel` and `Layers`. Seams can be dragged; the layout is saved through `workspace_state::save` (version 1) on change, debounced to 500 ms on the background executor.
  - The shell is laid out like harman:
    - **Title bar:** brand `ids::cm::BRAND_HACIVAT_CAPS` (F6: the message, not the term), capsule, palette field (`hv-titlebar-palette`, keys from `ActiveKeymap` via `bindings_for` and `display_chord`), and on the right the language segmented control (catalog `short_name`s) and the theme segmented control (moon/sun). On Windows and Linux, `.app_menu(..)` runs `ActionRegistry::run("app.show_menu", ..)` (registered by Task 16, F14).
    - **Tiles:** explorer and connection panel with 30 px headers and empty states; the editor area with the tab strip and an empty state; the results pane with an empty state.
    - **Status bar:** left "No connection" plus the `PendingChord` segment; right a language button `EN / TR`.
  - Startup diagnostics (F24): a quarantined settings file shows an error toast (`hv-settings-file-quarantined`); `SettingsDiag` code `settings-invalid-value` shows one warning toast `hv-settings-invalid-value` listing the keys; code `settings-newer-version` shows `hv-settings-newer-version`.
  - Bin:
    - `main`: `LocalPaths::from_os` → `ensure_dirs` → `settings::load` → `logging::init(paths.logs_dir())` (JSON lines, daily rotation, keep 7 files) → `Catalogs::embedded().with_overlay(..)` → initial `L10n` built synchronously → `run_app(Boot { text_rendering }, ..)` → `hacivat_app::init`.
    - Flags: `--smoke` prints `first-frame-ms=<n>` at the first `on_next_frame`, then `idle-rss-mib=<n>` from `platform::resident_set_mib()` 5 s later (F18), and exits 0 (exit 1 if RSS is unreadable). `--vf-demo` shows the harman production capsule. `--print-window-id` prints `platform::native_window_id` and `WINDOW_TITLE=<title>` for `design/vf/capture.sh app`.
  - Shared test helpers (`tests/support/mod.rs`, used by Tasks 16, 17, 18 and 19): `shell(cx, lang) -> VisualTestContext`, `shell_at(cx, root: &Path, lang)`, `shell_with_keymap(cx, platform, layout, user: Option<&str>)`, `switch_to(vcx, lang)`, `run(vcx, action_id)`, `palette_label`, `panel_title`, `explorer_empty`, `toast_texts`, `dialog_title`, `dialog_body`, `tab_count`, `tab_title`, `layer_ids`, `context_stack`, `status_bar_text`, `root_bg`, `apply(vcx, ThemePref)`, `registry_frame`, `l10n_diags`, `focused(vcx) -> Option<FocusHandle>`. Every text getter strips U+2068/U+2069.
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
    | `hv-settings-invalid-value` | Some settings could not be read and use their defaults: { $keys } | — |
    | `hv-settings-newer-version` | Your settings file is from a newer Hacivat. Settings this version does not know are kept. | — |

    TR drafts not in harman get a `# Review: PO` comment. Missing TR entries fall back to EN.

- [ ] **Step 1: Write the failing tests** (`tests/shell.rs`, VisualTestContext with `scene-check`; line 4 of the full test set).
```rust
#[gpui::test] fn switching_language_rerenders_without_restart(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en"); assert_eq!(palette_label(&vcx), "Search or run a command");
    switch_to(&vcx, "tr"); vcx.run_until_parked(); assert_eq!(palette_label(&vcx), "Ara veya komut çalıştır"); assert_eq!(panel_title(&vcx), "GEZGİN");
}
#[gpui::test] fn two_rapid_switches_last_wins(cx: &mut TestAppContext) { let vcx = shell(cx, "en"); switch_to(&vcx, "tr"); switch_to(&vcx, "en"); vcx.run_until_parked(); assert_eq!(palette_label(&vcx), "Search or run a command"); }
#[gpui::test] fn toast_rerenders_after_language_switch(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en"); push_toast(&vcx, ids::hv::STATUS_NO_CONNECTION.id, ToastKind::Info); assert_eq!(toast_texts(&vcx), ["No connection"]);
    switch_to(&vcx, "tr"); vcx.run_until_parked(); assert_eq!(toast_texts(&vcx), ["Bağlantı yok"]);
}
#[gpui::test] fn dialog_rerenders_after_language_switch(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en"); open_dialog_ids(&vcx, ids::hv::CAPSULE_NO_CONNECTION.id, ids::hv::EXPLORER_EMPTY.id); assert_eq!((dialog_title(&vcx), dialog_body(&vcx)), ("No connection".into(), "No connections yet".into()));
    switch_to(&vcx, "tr"); vcx.run_until_parked(); assert_eq!((dialog_title(&vcx), dialog_body(&vcx)), ("Bağlantı yok".into(), "Henüz bağlantı yok".into())); assert_eq!(layer_ids(&vcx), ["dialog"]);
}
#[gpui::test] fn theme_switch_is_live(cx: &mut TestAppContext) { let vcx = shell(cx, "en"); apply(&vcx, ThemePref::Light); vcx.run_until_parked(); assert_eq!(root_bg(&vcx), LIGHT.colors.ground.to_hsla()); assert_eq!(vcx.read(|cx| Theme::get(cx).mode), ThemeMode::Light); }
#[test] fn system_theme_follows_appearance() { assert_eq!(theme_mode_for(ThemePref::System, WindowAppearance::VibrantLight), ThemeMode::Light); assert_eq!(theme_mode_for(ThemePref::Dark, WindowAppearance::Light), ThemeMode::Dark); }
#[gpui::test] fn one_filled_badge_holds(cx: &mut TestAppContext) { let vcx = shell_demo(cx); vcx.update(|w, _| assert_one_filled_badge(w, DARK.colors.cp_fill.to_hsla(), true)); let vcx = shell(cx, "en"); vcx.update(|w, _| assert_one_filled_badge(w, DARK.colors.cp_fill.to_hsla(), false)); }
#[gpui::test] fn layout_round_trips_through_workspace_json(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap(); let vcx = shell_at(cx, root.path(), "en");
    workspace(&vcx).update_in(&vcx, |w, win, cx| { w.split_focused(Axis::Horizontal, win, cx); w.new_console_tab(win, cx); });
    vcx.executor().advance_clock(Duration::from_millis(600)); vcx.run_until_parked();
    let before = layout(&vcx); assert_eq!(before.0.panes.len(), 2); close_all_windows(&vcx);
    let vcx2 = shell_at(cx, root.path(), "en"); assert_eq!(layout(&vcx2), before); // layout() = (PaneTree, TileLayout)
}
#[gpui::test] fn corrupt_overlay_keeps_language_and_reports_diag(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap(); write(root.path().join("config/locales/tr/hacivat-ui.ftl"), "hv-explorer-empty = Bağlantı yok gibi\n!!junk\n");
    let vcx = shell_at(cx, root.path(), "tr"); assert_eq!(panel_title(&vcx), "GEZGİN"); assert_eq!(explorer_empty(&vcx), "Bağlantı yok gibi");
    assert_eq!(l10n_diags(&vcx).iter().filter(|d| matches!(d, LoadDiag::Junk { .. })).count(), 1);
}
#[gpui::test] fn every_label_in_shell_is_from_fluent_or_exempt(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en-XA"); enable_registry(&vcx); vcx.run_until_parked(); let f = registry_frame(&vcx);
    let fluent: Vec<_> = f.iter().filter(|r| r.source == TextSource::Fluent).collect(); assert!(fluent.len() >= 10, "{}", fluent.len());
    for r in fluent { assert!(r.text.starts_with('[') && r.text.ends_with(']'), "{}: {}", r.path, r.text); }
    assert!(f.iter().any(|r| r.text == "[HACIVAT]")); // the brand is wrapped but not transformed (F6)
}
#[gpui::test] fn layers_escape_pops_topmost_and_restores_focus(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en"); let base = focus_test_input(&vcx); // a TextInput inside a test-registered tab
    push_test_layer(&vcx, "a", "LayerA"); let a_focus = focused(&vcx); push_test_layer(&vcx, "b", "LayerB");
    assert_eq!(context_stack(&vcx).0, ["Workspace", "Test", "LayerA", "LayerB"]);
    vcx.simulate_keystrokes("escape"); assert_eq!(layer_ids(&vcx), ["a"]); assert_eq!(focused(&vcx), a_focus);
    vcx.simulate_keystrokes("escape"); assert!(layer_ids(&vcx).is_empty()); assert_eq!(focused(&vcx), Some(base));
}
#[gpui::test] fn registered_tab_content_renders_and_titles_follow_language(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en"); register_test_tab(&vcx, "test", ids::hv::PANEL_EXPLORER.id); // title TextFn from the ID
    workspace(&vcx).update_in(&vcx, |w, win, cx| w.open_registered_tab("test", win, cx)); assert_eq!(tab_title(&vcx), "EXPLORER"); assert!(vcx.debug_bounds("test-tab-content").is_some());
    workspace(&vcx).update_in(&vcx, |w, win, cx| w.open_registered_tab("test", win, cx)); assert_eq!(tab_count(&vcx), 2); // Console 1 + one test tab, not two
    switch_to(&vcx, "tr"); vcx.run_until_parked(); assert_eq!(tab_title(&vcx), "GEZGİN");
}
#[gpui::test] fn key_capture_hook_runs_before_children(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en"); let input = focus_test_input(&vcx); let seen = install_recording_hook(&vcx, "ctrl-j"); // returns true only for ctrl-j
    vcx.simulate_keystrokes("ctrl-j"); assert_eq!(seen.take(), vec![("ctrl-j".to_string(), vec!["Workspace", "Test"])]); assert_eq!(input_value(&vcx, &input), "");
    vcx.simulate_input("x"); assert_eq!(input_value(&vcx, &input), "x");
}
#[gpui::test] fn invalid_settings_value_shows_toast(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap(); write(root.path().join("config/settings.toml"), "[ui]\ntheme = 3\n");
    let vcx = shell_at(cx, root.path(), "en"); let t = toast_texts(&vcx); assert_eq!(t.len(), 1); assert!(t[0].starts_with("Some settings could not be read") && t[0].contains("ui.theme"), "{}", t[0]);
}
#[test] fn english_defaults_match_l10n_for_every_hui_message() {
    let (e, l) = (EnglishDefaults::new(), l10n_en());
    for id in message_ids_in(&workspace_file("locales/en/hui.ftl")) { assert_eq!(e.text(&id, &sample_ui_args(&id)), l.t_dynamic(&id, sample_arg_values(&id)), "{id}"); } // samples: counts = 2, text vars = "x"
}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-app --features scene-check,pseudo --test shell`. Expected: FAIL.
- [ ] **Step 3: Implement `l10n_global`, `theme_pref`, `keymap_state`, the extension points and the shell.** Components store IDs plus arguments, or closures, never rendered text (I18N-07 step 4).
- [ ] **Step 4: Implement the bin.**
- [ ] **Step 5:** Run the tests, then `cargo run -p hacivat --release -- --smoke`. Expected: PASS, and output lines `first-frame-ms=<n>` and `idle-rss-mib=<n>` with exit 0.
- [ ] **Step 6: Visual check.** Run `cargo run -p hacivat -- --vf-demo` and compare it side by side with harman (title bar, tiles, status bar) against DESIGN.md Layout/Components. Fix any deviation; the formal VF numbers come in Task 18.
- [ ] **Step 7: Commit** with message `feat(app): harman shell, layers/tab-content/key-capture extension points, runtime language/theme switch, layout restore; hacivat binary`.

### Task 14: CI, unsigned nightly packaging, local release signing, passworded update key

**Files:**
- Move: `.github/workflows/m0.yml` → `.github/workflows/ci.yml`, and change its `name: m0` to `name: ci` and its trigger `branches: [main, m0-spike]` to `branches: [main, m1-foundations]` (F28)
- Modify: `packaging/{Packager.toml, package-macos.sh, package-linux.sh, package-windows.ps1, make-manifest.py, keys/README.md}`
- Create: `packaging/common.sh`, `packaging/sign-artifacts.sh`, `xtask/src/update_key.rs`
- Modify: `xtask/src/main.rs` (arm `check-update-key`)

**Interfaces:**
- Produces:
  - `xtask check-update-key <secret-key-file>` exits 1 unless the minisign secret key is encrypted.
    - `pub fn key_is_encrypted(sk_file: &str) -> Result<bool>`: base64-decode line 2 and check that bytes 2..4 (the KDF algorithm) equal `b"Sc"`. `[0, 0]` means unencrypted.
  - CI jobs (pinned toolchain, `--locked`). No job reads a signing secret (L13).
    - `build-test` (3 OSes): `cargo clippy --workspace --all-targets -- -D warnings`, then lines 1–4 of the full test set (F2)
    - `l10n-check`: `cargo xtask l10n check` and `cargo test -p xtask l10n -- --ignored repo_catalogs_are_clean` (F19)
    - `turkey-test` (Ubuntu; `locale-gen tr_TR.UTF-8`; `LANG=tr_TR.UTF-8 LC_ALL=tr_TR.UTF-8`, lines 1–4 of the full test set)
    - `switch` (spec §7.1, F29; macOS + Ubuntu): `cargo test -p hacivat-app --features scene-check,pseudo -- switch language` (the runtime-switching tests of Tasks 13, 16 and 17)
    - `launch-smoke` (3 OSes; `cargo run -p hacivat --release -- --smoke`, under `xvfb-run -a` on Linux; asserts exit 0, a `first-frame-ms=` line and an `idle-rss-mib=` line). It goes green once Task 13 has merged; this task merges after Task 13 (F17).
    - `deny` (`cargo deny --all-features check licenses bans sources`, so the `scene-check` dependencies are covered)
    - `tokens`, `check-deps`
    - Nightly `release-tests`: line 5 of the full test set (`cargo test --release -p hacivat-l10n --features pseudo`: the release-only fallback and the 10 ms parse budget, F10)
    - Nightly `bench` (unchanged spike benches)
    - Nightly `nightly-package` (3 OSes), all **unsigned** (L13): macOS universal `.dmg`, Windows NSIS, Linux AppImage and `.deb`. No step signs, notarizes or minisigns.
    - Nightly `pseudo-screens` (macOS + Ubuntu; runs Task 18's harness and uploads `target/shots/**`). Until Task 18 merges, it is guarded with `if: hashFiles('crates/app/hacivat-app/tests/shots.rs') != ''`.
  - Local release (PO's Mac only, L13):
    - `RELEASE=1 NOTARY_PROFILE=<profile> packaging/package-macos.sh <version>`: signs with Developer ID, notarizes, staples, then runs `cargo xtask check-update-key ~/.hacivat-keys/update.key` and minisigns the DMG (minisign prompts for the key password).
    - `packaging/sign-artifacts.sh <dir>`: runs `check-update-key`, then minisigns every Windows and Linux package that CI built into `<dir>` and writes SHA-256 checksums.
    - Without `RELEASE=1`, `package-macos.sh` builds an unsigned DMG, as CI does.
  - Packager config:
    - `name = "hacivat"`, `productName = "Hacivat"`, `identifier = "dev.hacivat.Hacivat"`, `version = "@VERSION@"` (nightly `0.1.0-nightly.<yyyymmdd>`), `minimumSystemVersion = "13.0"`.
    - Windows and Linux render their own configs without the signing placeholder.
    - The DMG is named `Hacivat_<version>_universal.dmg`.
  - `packaging/common.sh` defines the key path once (`~/.hacivat-keys/update.key`), a truthy parser for `REQUIRE_NOTARIZATION` (`1|true|yes`), and probe output kept on failure.

- [ ] **Step 0 (PO):** Generate the passworded key: `minisign -G -p packaging/keys/update.pub -s ~/.hacivat-keys/update.key`, without `-W`. Add no CI secrets: the key and the Apple credentials stay on the PO's Mac (L13).
- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn unencrypted_key_is_refused() { assert!(!key_is_encrypted(&sk_fixture(*b"Ed", [0, 0])).unwrap()); }
#[test] fn scrypt_key_is_accepted() { assert!(key_is_encrypted(&sk_fixture(*b"Ed", *b"Sc")).unwrap()); }
#[test] fn garbage_is_an_error() { assert!(key_is_encrypted("untrusted comment: x\n!!!\n").is_err()); }
```
- [ ] **Step 2:** Run `cargo test -p xtask update_key`. Expected: FAIL.
- [ ] **Step 3: Implement the subcommand, the workflow and the packaging changes** (deferred minors: Task 10 ×2, Task 10 REQUIRE_NOTARIZATION/probe).
- [ ] **Step 4:** Run `cargo test -p xtask update_key && actionlint .github/workflows/ci.yml && bash -n packaging/*.sh`, and `grep -nE 'secrets\.(APPLE|MINISIGN|NOTARY)' .github/workflows/ci.yml` (expected: no output). Locally, run `packaging/package-macos.sh 0.1.0-m1` (expected: an unsigned DMG), then `RELEASE=1 NOTARY_PROFILE=<profile> packaging/package-macos.sh 0.1.0-m1` (expected: `spctl -a -vvv -t install` reports `source=Notarized Developer ID`, and `minisign -V -p packaging/keys/update.pub -m <dmg>` verifies).
- [ ] **Step 5:** After the controller has merged Task 13 and this task and pushed the branch, run `gh workflow run ci.yml --ref m1-foundations`. Expected: every PR job is green, and `launch-smoke` passes on all three OSes.
- [ ] **Step 6: Commit** with message `ci(m1): clippy, feature tests, l10n-check, turkey-test, switch, launch smoke, unsigned nightlies; local release signing`.

### Task 15: Carry the GPUI IME patch on a zed fork (R1)

**Files:**
- Modify: `Cargo.toml` (the gpui/gpui_platform `git`/`rev`), `deny.toml` (`allow-git` += the fork), `Cargo.lock`, `crates/ui/hacivat-ui-core/patches/README.md`
- Create (in the fork, not in this repo): branch `hacivat/912159c` with the commit "gpui_macos: discard marked text on FocusLost (Hacivat carry)", plus `HACIVAT-PATCHES.md` at the root listing the patch, its rationale and the Apache-2.0 §4(b) notice

**Interfaces:**
- Produces: `[workspace.dependencies] gpui/gpui_platform = { git = "https://github.com/berkaygkc/zed", rev = "<fork sha>" }`. `ZED_SOURCES` (Task 2) already covers the fork.

- [ ] **Step 0 (PO):** Run `gh repo fork zed-industries/zed --clone=false`, or approve the agent running it. If the fork does not exist at dispatch, mark this task deferred to M2 in `docs/benchmarks/m1.md` and stop.
- [ ] **Step 1:** Create the branch at `912159c`, then apply the carried patch with `git am` or `git apply` plus a commit. The patch has never been compiled (`patches/README.md`), so before pushing, build it in the fork checkout: `cargo check -p gpui_macos` (F28). If it does not apply or does not compile, fix it in this step and record the change in `HACIVAT-PATCHES.md`. Check that `LICENSE-APACHE` is untouched and that `crates/gpui_macos/src/window.rs` carries the "modified by Hacivat" header. Push.
- [ ] **Step 2:** Switch the workspace pin, then run `cargo update -p gpui -p gpui_platform`, `cargo build --workspace`, lines 1–4 of the full test set, `cargo deny --all-features check licenses bans sources` (this covers `scene-check`'s `gpui/bench-support` dependencies, such as criterion, F28) and `cargo xtask check-deps`. Expected: all pass, and the `Cargo.lock` diff touches only zed-sourced packages, whose source becomes the fork.
- [ ] **Step 3:** Update `patches/README.md`: the patch is applied via the fork at `<sha>`, and Gap 2 (focus move while the window is inactive) stays open, assigned to M5. The manual IME check needs the palette and settings inputs from W4, so it runs in Task 19, Step 3.
- [ ] **Step 4: Commit** with message `build(gpui): carry FocusLost discardMarkedText via fork at 912159c (R1)`.

### Task 16: Commands: registry, dispatcher, palette, menus

**Files:**
- Create: `crates/app/hacivat-app/src/commands/{mod,registry,dispatch,palette,debug}.rs`, `crates/app/hacivat-app/src/menus.rs` (replacing Task 1's stub), `crates/app/hacivat-app/tests/commands.rs`
- Modify: `crates/app/hacivat-app/src/lib.rs` (one line in the `init` registration block: `commands::install(cx);`), `locales/{en,tr}/hacivat-ui.ftl` (append `hv-cmd-*`, `hv-menu-*`, `hv-about-*`, `hv-toast-*`)

**Interfaces:**
- Consumes: `ActionRegistry`, `RunCommand`, `FocusedInput`, `InputEvent` (Task 1), `hacivat_ui::keymaps::load`, `Keymap::bindings` and `display_chord` (Task 5), `Palette` (Task 11), `Dialog`, `Toast`, `MenuItemSpec` (Task 12), `search` (Task 9), and from Task 13: `Workspace` and its command methods, `Layers` (`push_layer`/`pop_layer`), `open_dialog`, `open_popup_menu`, `context_stack`, `ShellHooks::set_key_capture`, `ActiveKeymap`, and the test helpers in `tests/support`.
- Produces:
  - `commands::install(cx: &mut App)`: registers the actions, installs the `Dispatcher` in the key-capture hook slot, observes `ActiveKeymap` and `FocusedInput`, and calls `menus::rebuild(cx)`.
  - `register_workspace_actions(reg: &mut ActionRegistry)` registers exactly: `workspace.command_palette`, `workspace.open_settings`, `workspace.new_console_tab`, `workspace.close_tab`, `workspace.next_tab`, `workspace.prev_tab`, `workspace.split_right`, `workspace.split_down`, `workspace.focus_next_pane`, `workspace.toggle_explorer`, `workspace.toggle_theme`, `workspace.next_language`, `workspace.reload_translations`, `app.about`, `app.show_menu` (Windows/Linux only; `enabled` is false on macOS), `app.restart`, `app.quit`. Each `title_id` is `hv-cmd-<id with . and _ → ->`, for example `hv-cmd-workspace-open-settings = Open settings` / `Ayarları aç`. `workspace.open_settings` calls `Workspace::open_registered_tab("settings")`, which Task 17 registers.
  - Dispatcher:
    - `Dispatcher { keymap: Arc<Keymap>, pending: Vec<KeyPress>, since: Option<Instant>, layout: KeyboardLayoutId }`
    - `on_key_down(&mut self, ev: &KeyDownEvent, ctx: &ContextStack, composing: bool, now: Instant) -> DispatchOutcome::{Run(String), Pending(SharedString), PassThrough}`
    - `set_keymap(&mut self, keymap: Arc<Keymap>, layout: KeyboardLayoutId)` (F11): clears `pending`. `install` calls it whenever the `ActiveKeymap` global changes (preset switch or keyboard layout change).
    - The pending state expires after 1,000 ms.
    - It runs in Task 13's key-capture hook with `Workspace::context_stack`, reads `composing` from the `FocusedInput` global (F15), and updates the `PendingChord` global.
  - Palette glue: `workspace.command_palette` pushes the `Palette` as a layer (`id: "palette"`, context `Palette`); `Confirmed` runs the action and pops the layer, `Dismissed` pops it. On a language switch the items are rebuilt from IDs and the query is kept.
  - About and info toast (F14): `app.about` opens a `Dialog` with `hv-about-title` (`About { -brand-hacivat }` / `{ -brand-hacivat } Hakkında`), `hv-about-version` (`Version { $version }` / `Sürüm { $version }`) and one `hui-dialog-close` button. `workspace.reload_translations` reloads the overlay and pushes an Info toast `hv-toast-translations-reloaded` (`Translations reloaded` / `Çeviriler yeniden yüklendi`).
  - Debug builds only (`commands/debug.rs`): `HACIVAT_DEBUG_FOCUS_MOVE_MS=<n>` makes the first `FocusedInput.composing == true` transition schedule a focus move to the palette input (`n` ms later, opening the palette if needed). Function: `install_focus_move(cx: &mut App, delay: Duration)`. This is the programmatic focus move for the IME check in Task 19.
  - `CommandEntry { id: &'static str, title: SharedString /* l10n.t_dynamic(title_id) */, english_title: SharedString /* l10n.t_english(title_id) */, keys: Option<SharedString> }`
  - `rank_commands(query: &str, entries: &[CommandEntry], recent: &[String], opts: &SearchOpts) -> Vec<PaletteItem>` (§8.13, spec C18):
    - Match the localized title first, the English title as the second layer, and the action ID as the third layer (F14).
    - Order: ExactWord > Prefix > Substring > Fuzzy on the localized title, then English-only matches, then ID-only matches.
    - Ties: recent first (most recent first), then registry order.
    - An empty query lists every enabled action, recent first.
  - Menus, one registry-derived model for both menus (F14):
    - `MenuGroup { title_id: &'static str, items: Vec<MenuEntry::{Action(&'static str), Separator}> }`
    - `menus::menu_model(reg: &ActionRegistry) -> Vec<MenuGroup>`: groups `hv-menu-app` (`{ -brand-hacivat }`), `hv-menu-view` (`View` / `Görünüm`), `hv-menu-window` (`Window` / `Pencere`), `hv-menu-help` (`Help` / `Yardım`)
    - `menus::build_menus(model: &[MenuGroup], cx: &App) -> Vec<Menu>` for macOS `set_menus`. Items carry `RunCommand { id }` (F1); their chords are bound as GPUI `KeyBinding`s under context `HacivatMenu` (R7).
    - `menus::popup_items(model: &[MenuGroup], cx: &App) -> Vec<MenuItemSpec>` for the Windows/Linux in-window menu (`app.show_menu` opens it with `Workspace::open_popup_menu` under the title-bar button). Labels are `TextFn`s.
    - `menus::rebuild(cx: &mut App)`: `cx.set_menus(build_menus(&menu_model(reg), cx))` on macOS; a no-op elsewhere, since the popup reads its labels when it renders.

- [ ] **Step 1: Write the failing tests.**
```rust
#[gpui::test] fn primary_k_opens_palette_with_every_action(cx: &mut TestAppContext) { let vcx = shell(cx, "en"); vcx.simulate_keystrokes(primary("k")); assert_eq!(layer_ids(&vcx), ["palette"]); assert_eq!(palette_items(&vcx).len(), enabled_action_count(&vcx)); assert!(palette_item(&vcx, "workspace.open_settings").keys.is_some()); }
#[gpui::test] fn altgr_at_sign_reaches_palette_input(cx: &mut TestAppContext) {
    let vcx = shell_with_keymap(cx, KeymapPlatform::Windows, "tr-q", Some("[[binding]]\nchord=\"ctrl-alt-q\"\ncommand=\"workspace.toggle_theme\"\ncontext=\"Workspace\""));
    open_palette(&vcx); let ran = record_actions(&vcx);
    vcx.dispatch_keystroke_event(KeyDownEvent { keystroke: Keystroke { modifiers: Modifiers { control: true, alt: true, ..Default::default() }, key: "q".into(), key_char: Some("@".into()) }, is_held: false });
    vcx.simulate_input("@"); vcx.run_until_parked();
    assert!(ran.take().is_empty()); assert_eq!(vcx.read(|cx| Theme::get(cx).mode), ThemeMode::Dark); assert_eq!(palette_query(&vcx), "@");
}
#[gpui::test] fn bindings_do_not_fire_while_composing(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en"); run(&vcx, "workspace.new_console_tab"); assert_eq!(tab_count(&vcx), 2);
    open_palette(&vcx); compose_in_palette(&vcx, "ş"); // replace_and_mark through the input handler
    assert!(vcx.read(|cx| cx.global::<FocusedInput>().composing));
    vcx.simulate_keystrokes(primary("w")); assert_eq!(tab_count(&vcx), 2);
    commit_palette_composition(&vcx); vcx.simulate_keystrokes("escape"); assert!(layer_ids(&vcx).is_empty());
    vcx.simulate_keystrokes(primary("w")); assert_eq!(tab_count(&vcx), 1);
}
#[gpui::test] fn switch_language_with_palette_open_keeps_query(cx: &mut TestAppContext) { let vcx = shell(cx, "tr"); open_palette(&vcx); vcx.simulate_input("ayar"); switch_to(&vcx, "en"); vcx.run_until_parked(); assert_eq!(palette_query(&vcx), "ayar"); assert_eq!(palette_item(&vcx, "workspace.open_settings").title, "Open settings"); }
#[gpui::test] fn dispatcher_follows_active_keymap(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en"); vcx.update(|_, cx| { set_keymap_preset(cx, KeymapPreset::Datagrip); }); vcx.run_until_parked();
    vcx.simulate_keystrokes(primary("shift-a")); assert_eq!(layer_ids(&vcx), ["palette"]);
}
#[test] fn ranking_prefers_word_match_then_english_layer() { let r = rank_commands("ayar", &fixture_items_tr(), &[], &tol()); assert_eq!(r[0].id, "workspace.open_settings"); let r = rank_commands("settings", &fixture_items_tr(), &[], &tol()); assert_eq!(r[0].id, "workspace.open_settings"); let r = rank_commands("AYARLARI", &fixture_items_tr(), &[], &tol()); assert_eq!(r[0].id, "workspace.open_settings"); }
#[test] fn action_id_is_the_third_layer() {
    assert_eq!(rank_commands("open_settings", &fixture_items_tr(), &[], &tol())[0].id, "workspace.open_settings");
    assert_eq!(rank_commands("workspace.open", &fixture_items_en(), &[], &off())[0].id, "workspace.open_settings");
    let r = rank_commands("app", &fixture_items_en(), &[], &off()); // fixture titles: app.show_menu "Show app menu", app.about "About Hacivat"
    assert_eq!(r[0].id, "app.show_menu"); assert!(r.iter().any(|i| i.id == "app.about")); // a title match outranks ID-only matches
}
#[test] fn recent_breaks_ties() { let r = rank_commands("", &fixture_items_en(), &["app.about".into()], &off()); assert_eq!(r[0].id, "app.about"); }
#[test] fn every_action_title_exists_in_en() { let (reg, l) = (registry(), l10n_en()); for d in reg.all() { assert!(!l.t_english(d.title_id).starts_with('⟦'), "{}", d.title_id); } }
#[test] fn keymap_commands_are_registered() { let reg = registry(); for p in ALL_PLATFORMS { for preset in PRESETS { for b in load(p, preset, None).0.bindings() { assert!(reg.get(&b.command).is_some(), "{}", b.command); } } } }
#[gpui::test] fn menus_follow_language(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en"); let names = |vcx: &VisualTestContext| vcx.update(|_, cx| build_menus(&menu_model(cx.global::<ActionRegistry>()), cx).iter().map(|m| m.name.to_string()).collect::<Vec<_>>());
    assert_eq!(names(&vcx), ["Hacivat", "View", "Window", "Help"]); switch_to(&vcx, "tr"); vcx.run_until_parked(); assert_eq!(names(&vcx), ["Hacivat", "Görünüm", "Pencere", "Yardım"]);
}
#[gpui::test] fn in_window_menu_matches_app_menu(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en"); let (app_menu, popup) = vcx.update(|_, cx| { let m = menu_model(cx.global::<ActionRegistry>()); (action_item_names(&build_menus(&m, cx)), popup_labels(&popup_items(&m, cx), cx)) });
    assert!(!popup.is_empty()); assert_eq!(popup, app_menu); // same actions, same order, same labels
}
#[gpui::test] fn about_dialog_is_localized(cx: &mut TestAppContext) { let vcx = shell(cx, "en"); run(&vcx, "app.about"); assert_eq!(dialog_title(&vcx), "About Hacivat"); switch_to(&vcx, "tr"); vcx.run_until_parked(); assert_eq!(dialog_title(&vcx), "Hacivat Hakkında"); }
#[gpui::test] fn reload_translations_shows_info_toast(cx: &mut TestAppContext) { let vcx = shell(cx, "en"); run(&vcx, "workspace.reload_translations"); vcx.run_until_parked(); assert_eq!(toast_texts(&vcx), ["Translations reloaded"]); }
#[gpui::test] fn pending_chord_shows_in_status_bar(cx: &mut TestAppContext) {
    let vcx = shell_with_keymap(cx, current_platform(), "us", Some("[[binding]]\nchord=\"ctrl-j ctrl-s\"\ncommand=\"workspace.toggle_theme\"\ncontext=\"Workspace\""));
    vcx.simulate_keystrokes("ctrl-j"); let want = if cfg!(target_os = "macos") { "⌃J" } else { "Ctrl+J" };
    assert_eq!(vcx.read(|cx| cx.global::<PendingChord>().0.clone()).as_deref(), Some(want)); assert!(status_bar_text(&vcx).contains(want));
    vcx.executor().advance_clock(Duration::from_millis(1001)); vcx.run_until_parked(); assert_eq!(vcx.read(|cx| cx.global::<PendingChord>().0.clone()), None);
}
#[cfg(debug_assertions)] #[gpui::test] fn debug_focus_move_fires_after_first_composition(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en"); vcx.update(|_, cx| install_focus_move(cx, Duration::from_millis(50))); let input = focus_test_input(&vcx);
    compose_in(&vcx, &input, "か"); vcx.executor().advance_clock(Duration::from_millis(60)); vcx.run_until_parked();
    assert_eq!(layer_ids(&vcx), ["palette"]); assert!(palette_input_focused(&vcx));
}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-app --features scene-check,pseudo --test commands`. Expected: FAIL.
- [ ] **Step 3: Implement the registry population, the dispatcher, the palette glue, About, the menus and `install`.**
- [ ] **Step 4:** Run `cargo test -p hacivat-app --features scene-check,pseudo --test commands && cargo xtask l10n check`. Expected: PASS, and `0 errors`.
- [ ] **Step 5: Visual check.** Open the palette in en and tr in both themes and compare against DESIGN.md (raised surface, 6 px radius, popover shadow, 23 px rows, active `blue_tint_strong`, `text_2` metadata on raised).
- [ ] **Step 6: Commit** with message `feat(app): action registry, UI-EDIT-13 dispatcher, command palette, localized menus and About`.

### Task 17: Settings UI

**Files:**
- Create: `crates/app/hacivat-app/src/settings_ui/{mod,rows,search}.rs`, `crates/app/hacivat-app/tests/settings_ui.rs`
- Modify: `crates/app/hacivat-app/src/lib.rs` (one line in the `init` registration block: `settings_ui::install(cx);`), `locales/{en,tr}/hacivat-ui.ftl` (append `hv-settings-*`)

**Interfaces:**
- Consumes: `save_changes` (Task 7), `switch_locale`, `apply_theme`, `set_keymap_preset`, `TabContentRegistry` and the test helpers (Task 13), `effective`, `search` and `WhenValue::Local` (Task 9), `TEXT_RENDERING_IS_LIVE`, `app_font_smoothing_override` and `apply_text_rendering` (Task 4), `Dialog` and `Segmented` (Task 12), `TextInput` (Task 11).
- Produces:
  - `settings_ui::install(cx: &mut App)`: registers `TabContent` kind `"settings"` (title `hv-settings-title`, context `Settings`) in `TabContentRegistry` (F3). The tab itself is opened by Task 16's `workspace.open_settings` or by a test helper.
  - `SettingsPage`, an entity rendered as the `"settings"` tab content. It has a left nav (General, Appearance, Keyboard) and a search field (`TextInput` with placeholder `hv-settings-search`; I18N-09 with the English layer, as in ALG-L10N-07).
  - Rows:
    - Language: System plus catalog native names (`English`, `Türkçe`). Live.
    - Regional format: System, `en-US`, `tr-TR`. Live, with a preview line `hv-settings-regional-preview = { $number } · { $when }` rendering `NUMBER(1234.5)` and `DATETIME(WhenValue::Local(2026-11-11 14:05), dateStyle short, timeStyle short)`. The value is a wall-clock time, so the time zone does not shift it (F30).
    - Theme: Dark / Light / System. Live.
    - Text rendering: Crisp (default) / macOS standard. macOS only. Saved immediately; shows "Applies after restart" (because `TEXT_RENDERING_IS_LIVE` is false) and a "Restart now" button, which opens a dialog and then calls `cx.restart()`.
    - Keymap preset: Hacivat / DataGrip. Live: calls `set_keymap_preset` (Task 13), which replaces the `ActiveKeymap` global that Task 16's dispatcher follows. If the returned collisions on the current layout are not empty, a warning toast `hv-settings-keymap-collisions` lists them (`{ $chords }`, each as `<display_chord> → <char>`) (F11).
  - `settings_rows() -> Vec<SettingRow { key: &'static str, title: MsgRef<NoArgs> }>`; the English layer is `l10n.t_english(row.title.id)`
  - `filter_rows(query, rows, opts) -> Vec<&'static str>`
  - Every change goes through `save_changes(paths.settings_toml(), ..)`, and the I/O runs on the background executor.
  - Test helper (in `tests/settings_ui.rs`): `settings_tab(cx)` and `settings_tab_with(cx, platform, layout, time_zone)` open the shell and the settings tab; both call `platform::apply_text_rendering(TextRendering::Crisp)` first, so the process starts from the shipped default (F21).

- [ ] **Step 1: Write the failing tests.**
```rust
#[gpui::test] fn theme_change_persists_and_applies(cx: &mut TestAppContext) { let (vcx, paths) = settings_tab(cx); click_segment(&vcx, "ui.theme", "light"); vcx.run_until_parked(); assert_eq!(vcx.read(|cx| Theme::get(cx).mode), ThemeMode::Light); assert!(read(paths.settings_toml()).contains("theme = \"light\"")); }
#[gpui::test] fn language_change_is_live(cx: &mut TestAppContext) { let (vcx, _) = settings_tab(cx); choose(&vcx, "ui.language", "tr"); vcx.run_until_parked(); assert_eq!(nav_label(&vcx, 0), "Genel"); assert_eq!(tab_title(&vcx), "Ayarlar"); }
#[cfg(target_os = "macos")] #[gpui::test] fn text_rendering_change_needs_restart(cx: &mut TestAppContext) {
    let (vcx, paths) = settings_tab(cx); assert_eq!(app_font_smoothing_override(), Some(0)); assert!(!restart_note_visible(&vcx));
    click_segment(&vcx, "ui.text_rendering", "standard"); vcx.run_until_parked();
    assert!(read(paths.settings_toml()).contains("text_rendering = \"standard\"")); assert!(restart_note_visible(&vcx));
    assert_eq!(app_font_smoothing_override(), Some(0)); // the running process is unchanged: restart-only (R11)
}
#[cfg(not(target_os = "macos"))] #[gpui::test] fn text_rendering_row_hidden(cx: &mut TestAppContext) { assert!(!row_visible(&settings_tab(cx).0, "ui.text_rendering")); }
#[test] fn search_matches_english_layer_in_turkish() { assert_eq!(filter_rows("theme", &rows_tr(), &tol()), ["ui.theme"]); assert_eq!(filter_rows("tema", &rows_tr(), &tol()), ["ui.theme"]); assert_eq!(filter_rows("dil", &rows_tr(), &tol()), ["ui.language"]); }
#[gpui::test] fn regional_preview_is_turkish(cx: &mut TestAppContext) {
    let (vcx, _) = settings_tab_with(cx, current_platform(), "us", "Europe/Istanbul");
    choose(&vcx, "ui.regional_format", "tr-TR"); vcx.run_until_parked(); assert_eq!(regional_preview(&vcx), "1.234,5 · 11.11.2026 14:05");
    choose(&vcx, "ui.regional_format", "en-US"); vcx.run_until_parked(); assert!(regional_preview(&vcx).starts_with("1,234.5 · "), "{}", regional_preview(&vcx));
}
#[gpui::test] fn keymap_preset_reloads_and_warns_on_polish(cx: &mut TestAppContext) {
    let (vcx, paths) = settings_tab_with(cx, KeymapPlatform::Windows, "pl", "UTC");
    choose(&vcx, "keymap.preset", "datagrip"); vcx.run_until_parked();
    assert_eq!(vcx.read(|cx| cx.global::<ActiveKeymap>().preset), KeymapPreset::Datagrip); assert!(read(paths.settings_toml()).contains("preset = \"datagrip\""));
    let t = toast_texts(&vcx); assert_eq!(t.len(), 1); assert!(t[0].contains("Ctrl+Alt+S") && t[0].contains('ś'), "{}", t[0]);
    choose(&vcx, "keymap.preset", "hacivat"); vcx.run_until_parked(); assert_eq!(toast_texts(&vcx).len(), 1); // no new warning: hacivat has no collisions
}
```
- [ ] **Step 2:** Run `cargo test -p hacivat-app --features scene-check,pseudo --test settings_ui`. Expected: FAIL.
- [ ] **Step 3: Implement.** Fixed copy: `hv-settings-title = Settings` / `Ayarlar`, `hv-settings-nav-general = General` / `Genel`, `hv-settings-search = Search settings` / `Ayarlarda ara`, `hv-settings-restart-note = Applies after restart` / `Yeniden başlatınca uygulanır`, `hv-settings-keymap-collisions = On this keyboard layout these shortcuts type a character instead: { $chords }` (all TR drafts `# Review: PO`). Row and control metrics follow DESIGN.md: 26 px fields and segmented controls, 22 px key/value rhythm, 12 px left padding, `text_2` labels.
- [ ] **Step 4:** Run `cargo test -p hacivat-app --features scene-check,pseudo --test settings_ui && cargo xtask l10n check`. Expected: PASS.
- [ ] **Step 5: Visual check.** Capture the settings tab in en/tr, dark/light, and check it against DESIGN.md (no cards, seams only, 11 px floor, Raised Ink Rule).
- [ ] **Step 6: Commit** with message `feat(app): settings tab — language, regional format, theme, text rendering (restart), keymap preset`.

### Task 18: Shots harness: pseudo-screens, VF of the shell, DESIGN.md review

Runs after Tasks 16 and 17 have merged (W5, F17): it renders the palette, the settings tab, the in-window menu, the About dialog and the info toast.

**Files:**
- Create: `crates/app/hacivat-app/tests/shots.rs` (feature `shots`), `design/vf/samples-m1-shell.json` (generated), `xtask/src/shot_diff.rs`
- Modify: `design/vf/gen_samples.py` (`--set m1-shell`), `design/vf/capture.sh` (kind `app`: runs `hacivat --vf-demo --print-window-id`), `design/vf/README.md`, `xtask/src/main.rs` (arm `shot-diff`), `.github/workflows/ci.yml` (the nightly `shot-diff` step only)

**Interfaces:**
- Consumes: everything above. It uses `HeadlessAppContext::with_platform(gpui_platform::current_platform(true).text_system(), Arc::new(BundledAssets), gpui_platform::current_headless_renderer)` (R8) and calls `apply_text_rendering(Crisp)` first.
- Produces:
  - `cargo test -p hacivat-app --features shots --test shots -- --ignored` renders these surfaces:
    - `shell`, `shell-demo-production`, `shell-narrow` (1024×700), `palette-open`, `settings-general`, `dialog-about` (Task 16's `app.about`), `toast-info` (Task 16's `workspace.reload_translations`), `menu-app` (Task 16's `app.show_menu` popup; on macOS the harness opens the same popup directly)
    - at 1280×800 (except `shell-narrow`), × locales `en`, `tr`, `en-XA`, `tr-XT`, × themes `dark`, `light`
    - and `shell-demo-production` at 1440×900, scale 2, × en/tr × dark/light, for VF
  - It writes `target/shots/<surface>-<theme>-<locale>-<w>x<h>@<s>x.png`. For every frame it asserts:
    - (a) in pseudo locales, every `TextSource::Fluent` label is bracketed;
    - (b) no label has `clipped == true`;
    - (c) every `size_px ≥ 11.0` except synthesized small-caps glyph runs;
    - (d) `assert_one_filled_badge` holds (`expect_production` only on demo surfaces).
  - On Windows it prints `shots: no headless renderer (R8)` and passes.
  - `cargo xtask shot-diff --baseline <dir> --candidate <dir> [--max-de 2.0 --max-share 0.001]` compares same-named PNGs pixel by pixel with ΔE2000 and exits 1 on a breach.
  - CI (F17): the nightly `pseudo-screens` job (macOS) gains a `shot-diff` step against `design/baselines/m1/macos`, guarded with `if: hashFiles('design/baselines/m1/macos/*.png') != ''` until Task 19 commits the baselines.

- [ ] **Step 1: Write the failing tests.**
```rust
#[test] fn shot_diff_identical_is_zero() { assert_eq!(diff_share(&img(0x16181D), &img(0x16181D), 2.0), 0.0); }
#[test] fn shot_diff_counts_changed_pixels() { assert!((diff_share(&img(0x16181D), &half(0x16181D, 0xCE2C31), 2.0) - 0.5).abs() < 1e-9); }
#[test] fn m1_shell_samples_validate_against_reference() {
    let s = vf::parse_samples(&std::fs::read_to_string(workspace_root().join("design/vf/samples-m1-shell.json")).unwrap()).unwrap();
    // zones at 1440×900, scale 1: title bar y < 40; status bar y ≥ 876; panel headers y ∈ [40, 70) and y ∈ [671, 701) for x < 276; seams at x ∈ {276, 1188} and y ∈ {362, 671} (±1 px)
    for p in &s { assert!(zone_of(p).is_some(), "{p:?} lies outside every zone"); }
    for z in [Zone::TitleBar, Zone::StatusBar, Zone::PanelHeader, Zone::Seam] { assert!(s.iter().any(|p| zone_of(p) == Some(z)), "no sample in {z:?}"); }
}
```
  The harness test itself, `#[ignore] fn render_all_surfaces()`, is the main check.
- [ ] **Step 2:** Run `cargo test -p xtask shot_diff vf`. Expected: FAIL.
- [ ] **Step 3: Implement `shot_diff`, the samples set, the `app` capture kind, the harness and the CI step.** First confirm that a headless capture of `shell-demo-production` passes `xtask vf`'s reference self-check. If it does not, switch the VF path to `capture.sh app` (R8 fallback) and record why.
- [ ] **Step 4:** Run `cargo test -p hacivat-app --features shots --test shots -- --ignored`, then:
  ```
  cargo xtask vf --reference target/vf/ref-<theme>-<lang>-2x.png \
    --candidate target/shots/shell-demo-production-<theme>-<lang>-1440x900@2x.png \
    --samples design/vf/samples-m1-shell.json --scale 2 --theme <theme>
  ```
  Run the VF compare for each of the 4 combinations. Expected: the harness passes, and every VF sample and scanline reads `ok`. Also run `actionlint .github/workflows/ci.yml`.
- [ ] **Step 5: DESIGN.md review per surface.** Open each PNG (en, tr-XT, dark, light) and check it against:
  - tokens only, seams, radii, the 11 px floor, popover-only shadow, the Raised Ink Rule;
  - the Overflow Rule (`shell-narrow`);
  - focus ring visibility (`palette-open`).
  
  List the fixes and apply them in the owning code. Re-run Step 4.
- [ ] **Step 6: Commit** with message `test(shots): pseudo-screens harness, M1 shell VF samples, shot-diff`.

### Task 19: M1 gate

**Files:**
- Create: `docs/benchmarks/m1.md`, `crates/app/hacivat-app/tests/cross_surface.rs`, `design/baselines/m1/macos/*.png` (after the PO signs off)
- Modify: `docs/benchmarks/m0-ime-checklist.md` (close Known gap 1, F33)

- [ ] **Step 1: Cross-surface tests (F3).** These need the palette (Task 16) and the settings tab (Task 17) together, so they live here. Write `tests/cross_surface.rs`:
```rust
#[gpui::test] fn escape_closes_only_topmost_layer_and_restores_focus(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en"); run(&vcx, "workspace.open_settings"); let search = focus_settings_search(&vcx);
    vcx.simulate_keystrokes(primary("k")); assert_eq!(layer_ids(&vcx), ["palette"]);
    vcx.simulate_keystrokes("escape"); assert!(layer_ids(&vcx).is_empty()); assert_eq!(focused(&vcx), Some(search));
    assert_eq!(active_tab_kind(&vcx), TabKind::Registered("settings")); assert_eq!(tab_count(&vcx), 2);
}
#[gpui::test] fn bindings_do_not_fire_while_composing_in_settings_search(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en"); run(&vcx, "workspace.open_settings"); let search = focus_settings_search(&vcx); assert_eq!(tab_count(&vcx), 2);
    compose_in(&vcx, &search, "ş"); vcx.simulate_keystrokes(primary("w")); assert_eq!(tab_count(&vcx), 2);
    commit_composition(&vcx, &search); vcx.simulate_keystrokes(primary("w")); assert_eq!(tab_count(&vcx), 1);
}
```
  Run `cargo test -p hacivat-app --features scene-check,pseudo --test cross_surface`. A failure is fixed in the owning task's code, not in the test.
- [ ] **Step 2:** On `m1-foundations`, run `cargo clean -p hacivat -p hacivat-app` (and `--release`), then:
  ```
  cargo clippy --workspace --all-targets -- -D warnings \
    && <all five lines of the full test set> \
    && cargo test -p xtask l10n -- --ignored repo_catalogs_are_clean \
    && cargo xtask check-deps && cargo xtask l10n check && cargo xtask tokens --check \
    && cargo deny --all-features check licenses bans sources
  ```
  Expected: all green, including the release-only l10n tests and the 10 ms parse budget (F10). Record the median parse time in `m1.md`.
- [ ] **Step 3:** On the PO's Mac, run `cargo run -p hacivat --release -- --smoke` 3 times. Record `first-frame-ms` (≤ 500) and `idle-rss-mib` (≤ 200 MB) in `m1.md` with the build hash.
- [ ] **Step 4: Demo for the PO (macOS):**
  - Switch EN↔TR from the title bar, the status bar, the palette and Settings, and dark/light/system, all without a restart.
  - Run the palette with Turkish Q.
  - Do the IME rows (Task 15) in the palette and the settings search: Japanese composition with Enter; Turkish Q `ğüşıöçİ`; `HACIVAT_DEBUG_FOCUS_MOVE_MS=3000` with a debug build, composing in settings search. Expected: no duplicated text and no stuck underline. Then mark Known gap 1 in `docs/benchmarks/m0-ime-checklist.md` as closed.
  - Change text rendering and restart.
  - Restore the layout after quit.
  - Build the release locally (L13): `RELEASE=1 NOTARY_PROFILE=<profile> packaging/package-macos.sh 0.1.0-m1`, install the notarized DMG and confirm `spctl` reports `accepted`.
- [ ] **Step 5: Windows and Linux.** Record the CI `launch-smoke` results and the unsigned nightly artifacts. When the PO provides the hosts, install the nightly packages there, launch them, capture `capture.ps1`/`capture.sh app` shell screens, and record them in `m1.md`. Otherwise list them as open items.
- [ ] **Step 6: PO DESIGN.md review** of every surface PNG from Task 18, plus the Turkish strings (L1). After sign-off, copy the macOS PNGs to `design/baselines/m1/macos/` (the nightly `shot-diff` step from Task 18 then starts using them).
- [ ] **Step 7:** Write `docs/benchmarks/m1.md`:
  - the exit criteria table (launch on all OSes, notarized macOS built locally per L13, runtime switch, pseudo-screens job, Fluent-only strings) with PASS/FAIL and evidence;
  - the R1–R13 outcomes;
  - the IME rows (Task 15);
  - open items with owners.
- [ ] **Step 8: Commit** with message `docs(m1): gate record, cross-surface tests, approved shell baselines`.

---

## Self-review notes

- **Spec coverage (§6 M1):**
  - Workspace and CI → Tasks 1, 2, 14. Lint → Tasks 1, 14. `cargo-deny` → Tasks 14, 15. `turkey-test` → Task 14. `l10n-check` → Tasks 10, 14. `switch` → Task 14. Nightlies (unsigned, L13) → Task 14; local release signing → Tasks 14, 19.
  - ui-core and themes → Tasks 1, 4, 8.
  - I18N-01..04 → Task 6. I18N-05/06 → Task 9. I18N-07 → Tasks 6 and 13. I18N-16 → Tasks 9 and 18. I18N-17 → Task 10. I18N-23 → Tasks 10 and 12. I18N-09 per R9 → Task 9.
  - Action registry, keymaps and collision CI → Tasks 5, 16; palette → Tasks 11, 16.
  - Shell → Tasks 12, 13. Toasts and dialogs → Tasks 12, 13. Accessibility roles → Task 12.
  - Settings → Tasks 7, 17. §5.6 paths → Task 7.
  - Exit criteria → Tasks 13, 14, 18, 19.
- **M0 carry-overs:**
  - single bootstrap → Tasks 1, 4; transitive check-deps → Task 2; paint-level cp-fill → Task 8;
  - `frames.rs` gpui-free → Tasks 1, 3; text weight → Tasks 4, 17; IME patch → Task 15;
  - passworded update key → Task 14;
  - deferred minors that touch kept code → Tasks 2, 3, 4, 14 (spike-only minors stay with the spikes).
- **Type and name consistency (re-checked after the pre-flight rulings):** `KeyboardLayoutId` everywhere (never `LayoutId`, which stays GPUI's layout handle); `PopupMenu` for the in-window menu and `Menu` only for GPUI's application menu; `altgr_collisions(km, tables)` takes the platform from `Keymap::platform()`; `build_runtime(cat, neg, fmt: &FormatInputs, generation)` everywhere; `TextFn`/`KeyFn` on every stateful overlay; `TabKind::Registered("settings")` for the settings tab; `workspace_root()` in every xtask test.
- **Not in M1 by design:**
  - single-instance lock and IPC (§5.3) → M8 with the broker;
  - the updater client and first-run flow (§4, §7.3) → M9;
  - crash reports → M9;
  - grid and connection settings keys → M2/M3;
  - the D-074 toggle UI → M4;
  - inactive-window IME blur → M5;
  - signing in CI (L13: releases are signed locally).

## Appendix: pre-flight rulings applied

Source: `.superpowers/sdd/2026-11-12-m1-foundations-app-shell/preflight.md`, Part 3. All rulings adopted by the controller on 2026-11-12.

| # | What changed | Where |
|---|---|---|
| F1 | `gpui` dev-dependency with `test-support` allowed in `crates/ui` and `crates/app`; normal, build and plain dev edges still fail; generic `RunCommand { id }` action | Global Constraints (Layering); T1 (interfaces, manifests, M0 guard); T2 tests and Step 3; T16 menus |
| F2 | "Full test set" defined with the feature-gated commands; CI and the gate run it | Global Constraints; T1 Step 5; T8; T14 jobs; T15 Step 2; T19 Step 2 |
| F3 | T13 ships `Layers`, the `TabContent` registry and the key-capture hook slot; T16 and T17 add one registration line each; cross-surface tests moved to T19; T18 after T16/T17 | Waves table and ownership; T13; T16; T17; T19 Step 1; Review Focus 5 |
| F4 | T1 ships working `Label`, `ui_text`, `popover_surface`, `focus_outline`; T12 no longer waits for T11 | Waves intro; T1; T11; T12 |
| F5 | Spec row L13 (unsigned CI nightlies, local release signing); conflicting T14 Step 0 lines deleted | Spec decision log; header; Global Constraints (Packaging); T14; T19 Steps 4, 7 |
| F6 | `cm-brand-hacivat-caps` message; title bar uses the typed ID; brand terms skip the pseudo transform | T1 (common.ftl); T9 (`transform_resource`, test); T13 |
| F7 | `FormatInputs { format_locale, time_zone }` passed to `build_runtime`; the separate pseudo parameter dropped | T1; T6 (wiring, budget test); T9; T13 `switch_locale` |
| F8 | `L10n::reserve_generation()` on an `AtomicU64`; `switch_locale` reserves at call time | T1; T6 test; T13 |
| F9 | Every comment-only or tautological test now has exact assertions; rule "an empty or comment-only test body is a defect"; T4's constant test dropped; T5's AltGr test uses an explicit `ctrl-alt-q` binding | Global Constraints (Test bodies); T4–T8, T11–T13, T16–T19 |
| F10 | Release-only l10n tests run nightly and at the gate; parse budget re-measured in T9 and T19 | Global Constraints (Full test set line 5); T6 Step 5; T9 Step 4; T14 `release-tests`; T19 Step 2 |
| F11 | AltGr tables embedded (`layouts_altgr()`); `ActiveKeymap` + `set_keymap_preset`; `Dispatcher::set_keymap`; T17 preset test | T5; T13; T16; T17 |
| F12 | Collision test runs over every preset; datagrip `ctrl-alt-s` on PL recorded as accepted, documented shadowing | T5 data and tests |
| F13 | `titlebar_fit` gains `palette_pref` and `palette_truncates`; numbers fixed (1024 overflows, 1440 does not) | T12 |
| F14 | Action ID is the palette's third search layer; one menu model feeds `set_menus` and the in-window menu; `app.show_menu`, `hv-about-*`, info toast | T13 title bar; T16; T18 surfaces; R13 |
| F15 | `Keymap::bindings()`, `menus::rebuild` (T1 stub), `InputEvent::CompositionChanged`, `FocusedInput` global | T1; T5; T11; T16 |
| F16 | Dialog, popup-menu and toast labels are `TextFn` closures; `dialog_rerenders_after_language_switch` | T1 (`TextFn`); T12; T13; Review Focus 2 |
| F17 | T14 merges after T13; T18 in its own wave after T16/T17; nightly `shot-diff` step owned by T18; baselines owned by T19 | Waves table and ownership; T14; T18 |
| F18 | `platform::resident_set_mib()` in T4 (no `sysinfo`); extra crates listed in the Tech Stack and T1 | Tech Stack; T1 manifests; T4; T13 `--smoke` |
| F19 | `literal-allowlist.toml` append-only for every task; `repo_catalogs_are_clean` is `#[ignore]` and runs in CI and the gate | Global Constraints (Execution); T10; T14; T19 |
| F20 | T1 fixes the lint fallout in the spikes and defines `workspace_root()`; T3 names the real functions and handles `-z` renames | T1 Files and Step 3; T3 |
| F21 | T1 declares `Boot`, `TextRendering` and the final `run_app`; T4 owns the spike call sites; T17's helper applies `Crisp` first and asserts the pre-state | T1; T4; T17 |
| F22 | `LayoutId` → `KeyboardLayoutId`; hacivat-ui `Menu` → `PopupMenu` | T1; T5; T12; T16; self-review notes |
| F23 | T1 writes every crate's full dependency list; no later task edits a manifest it does not own | Global Constraints; T1 Step 3; T2, T4, T10 Files |
| F24 | Four new settings tests; diag codes defined; `hv-settings-invalid-value` and `hv-settings-newer-version` toasts | T7; T13 |
| F25 | AccessKit roles and names on Button, Dialog, PopupMenu, Toast and the tab strip, with tests | T12 (`tests/a11y.rs`) |
| F26 | One `focus_outline()` and `popover_surface()` in ui-core; `EnglishDefaults` checked against `L10n` for every `hui-*` message | T1; T11; T12; T13 test |
| F27 | Reachability test per layout overlay; Windows Turkish KLID fixed to `0000041F` | T5 |
| F28 | `cargo deny --all-features`; `ci.yml` name and branches updated; T15 compiles the patch before pushing | T14; T15 |
| F29 | Spec `switch` job added | T14 |
| F30 | `WhenValue::Local` wall-clock value; the regional preview uses it | T1; T9 test; T17 |
| F31 | `L10n::t`/`t_attr`/`t_machine` signatures; `Status`, `NegDiag`, `Direction` named; derives stated; stubs never panic; `.stderr` file listed | Global Constraints (Stubs); T1; T6 Files |
| F32 | Access keys compare with `char::eq_ignore_ascii_case`, exact match for non-ASCII; test added | T12 |
| F33 | T19 lists the `m0-ime-checklist.md` edit; `TextInput` placeholder is a `TextFn`; `AdapterFeatureInProduct` also covers `scene-check` and `test-support`; L463's test now asserts Shift matching; `case_for_display` kept as the sanctioned casing path | T2; T5; T9; T11; T19 Files |
