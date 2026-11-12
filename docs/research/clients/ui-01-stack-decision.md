# UI-01 — UI Stack Decision for Hacivat (Rust desktop DB client)

Status: research input for an architecture decision record (ADR). Date of research: 2026-11-11.
Companion documents: `ui-02-code-editor-and-data-grid-components.md` (components + grid algorithms), `oss-clients-beekeeper-and-tui.md`, `oss-clients-redisinsight-compass.md`.

---

## 1. Scope and sources

### 1.1 Question

Which UI technology should Hacivat use so that it can (a) render virtualized 2-axis grids with millions of rows, (b) host a rich SQL/query code editor, (c) offer IDE-style docking/tabs/splits, (d) meet i18n requirements (English + Turkish in v1, trivial addition of more languages, which implies IME, RTL and font fallback must work even if v1 does not ship an RTL locale), (e) be accessible to screen readers, (f) start fast and stay lean, (g) package/sign/auto-update on macOS, Windows and Linux, and (h) remain maintainable by a small team — all while keeping the database, MCP and networking core in Rust.

### 1.2 Candidates studied

| Candidate | Version / commit studied | License | Source location |
|---|---|---|---|
| Tauri v2 (`tauri` crate) | 2.12.5, commit `66b29d1b03bdc08e7275f8da40cc05432a50e1f4` (2026-11-10) | Apache-2.0 OR MIT | `.refs/repos/tauri` (sparse: `crates/tauri`, `crates/tauri-runtime-wry`, `crates/tauri-bundler/src`) |
| gpui (Zed) | `gpui` 0.2.2, Zed commit `912159c7ad77ce6296a93be77efc298b0b00fd50` (2026-11-11) | gpui/gpui_* platform crates: Apache-2.0; Zed `editor`, `ui`, `workspace`, `rope`, `text`, `picker`: **GPL-3.0-or-later** | `.refs/repos/zed/crates` |
| gpui-component (Longbridge) | commit `3c5320b7c0508c288a6be0ddddd9fb38e906f196` | Apache-2.0 | `.refs/repos/gpui-component` |
| egui / egui_extras | 0.37.0, commit `38f6bfa3d6802eee61bbca78ca30a1413a0a8e34` | MIT OR Apache-2.0 | `.refs/repos/egui` |
| iced | 0.15.0-dev (0.14.0 released 2025-12-07), commit `c3d2cfd0a8503fd4b13754467953b427be259c55` | MIT | `.refs/repos/iced` (sparse) |
| Slint | 1.20.0, commit `1871c9af106f1bd44aebccc676b153dfb61ea77d` | Triple: GPL-3.0-only / Slint Royalty-free 2.0 / commercial | `.refs/repos/slint` (sparse) |
| Floem (Lapce UI toolkit) | 0.2.0, commit `1351ffb162faeb8be983f1301d718a0d5fb59c27` (2026-09-23) | MIT | `.refs/repos/floem` |
| Lapce | commit `f66ffaa95387794205c35864517de70958b9ea8d` (2026-09-30) | Apache-2.0 | `.refs/repos/lapce` (sparse) |
| Dioxus (desktop + native/Blitz) | 0.8.0-alpha.3, commit `97adf7915e1adfef4e2da436c9bd0b6add0dfa39` | MIT OR Apache-2.0 | `.refs/repos/dioxus` (sparse: `packages/desktop`, `packages/native`, `packages/native-dom`) |

Repository popularity/activity snapshot (GitHub API, 2026-11-11): tauri 112.7k stars; zed 94.7k; dioxus 39.7k; egui 32.2k; iced 32.0k; slint 24.6k; gpui-component 20.3k; floem 4.3k; blitz 4.4k. All except floem/lapce pushed within the last 48 hours; floem last pushed 2026-09-24, lapce 2026-10-09 **[Observed: `gh api repos/<r>`]**.

### 1.3 Method

- Read framework source for the specific mechanisms that matter (IPC transport and thresholds, list/table virtualization, text system and bidi, accessibility integration, input method handling).
- Read license files and per-crate `Cargo.toml` license fields (critical for gpui/Zed).
- No builds or scripts were run from any cloned repository. Performance numbers that are not directly observed in source are tagged **[Inferred]** and listed as benchmarks to run in a spike (section 7).

---

## 2. Executive summary

1. **Recommendation: Tauri v2 shell + TypeScript UI + Rust core**, with a canvas-rendered data grid and CodeMirror 6, connected by a **windowed, binary (Arrow IPC or compact columnar) data protocol**. Weighted score 90/100 vs. 71 (Dioxus desktop), 70 (gpui + gpui-component), 59 (Slint, egui), 52 (iced, Floem), 50 (Dioxus native/Blitz). See section 5.
2. The decisive factors are **text and i18n** (IME, bidi/RTL, font fallback, shaping, locale-aware formatting are mature in every browser engine), **editor availability** (CodeMirror 6 / Monaco have no native-Rust equal under a permissive license), **accessibility**, and **code reuse with Perde's web panel**.
3. The main cost is **memory and per-OS webview variance** (WKWebView on macOS, WebView2/Chromium on Windows, WebKitGTK on Linux). Mitigation: never ship full result sets to the webview; render the grid on canvas; keep all heavy state in Rust.
4. Tauri v2 IPC has a **binary path**: `InvokeResponseBody::Raw(Vec<u8>)` arrives in JS as an `ArrayBuffer`; channel messages ≥ 1024 bytes (raw) or ≥ 8192 bytes (JSON) are staged in a per-webview queue and pulled by the webview through a custom-protocol `fetch` instead of `eval` **[Observed: tauri/crates/tauri/src/ipc/channel.rs:34-38, 292-320]**.
5. On Android the custom-protocol IPC is not used and on any OS it **falls back to `postMessage`** if the custom protocol fails (CSP, blocked scheme) **[Observed: tauri/crates/tauri/scripts/ipc-protocol.js:15-23, 61-84]**. Hacivat must test binary throughput on both paths.
6. **gpui is the strongest native alternative**: GPU-rendered, very fast, Apache-2.0 framework crates, and since 2026 has **AccessKit integration** **[Observed: zed/crates/gpui/src/_accessibility.rs:1-60; accesskit in gpui, gpui_macos, gpui_windows, gpui_linux Cargo.toml]**. Longbridge's **gpui-component** adds a table with row *and* column virtualization, dock, and an editor with LSP (completions, code actions, hover, semantic tokens) built on `ropey` **[Observed: gpui-component/crates/component/src/table/state.rs:129-134; crates/base/src/input/editor/lsp/]**.
7. **gpui licensing trap**: Zed's `editor`, `ui`, `workspace`, `picker`, `rope`, `text` crates are **GPL-3.0-or-later** **[Observed: zed/crates/<crate>/Cargo.toml license fields]**. Hacivat (permissive) may depend only on `gpui`, `gpui_*` platform crates, `sum_tree` (Apache-2.0) and third-party Apache/MIT crates such as gpui-component.
8. gpui risks: pre-1.0 with frequent breaking changes (its README says so) **[Observed: zed/crates/gpui/README.md:7]**, partial RTL (layout code contains explicit RTL workarounds) **[Observed: zed/crates/gpui/src/text_system/line_layout.rs:253]**, no packaging/updater, and `Pixels(f32)` coordinates **[Observed: zed/crates/gpui/src/geometry.rs:2736]** which require scroll-space scaling beyond ~16M px.
9. **egui** is excellent for tools but lacks bidi (`// TODO(emilk): heed bidi characters`) **[Observed: egui/crates/epaint/src/text/unicode.rs:15]** and `TableBuilder` virtualizes rows only with a fixed-height fast path **[Observed: egui/crates/egui_extras/src/table.rs:1027-1081]**; no rich code editor exists.
10. **iced** gained IME support in 0.14 and a `table` widget, but the table takes all rows eagerly (no virtualization) **[Observed: iced/widget/src/table.rs:13-27; CHANGELOG.md:18-24]** and there is no AccessKit integration in mainline **[Observed: no accesskit in iced Cargo manifests]**.
11. **Slint** has the best built-in i18n story of the native toolkits (`@tr()` + gettext, translations bundling, parley-based shaping with complex scripts, AccessKit) **[Observed: slint/internal/core/Cargo.toml:94-105,188; internal/core/translations.rs]**, but only a plain `TextEdit` (no code editor) and a basic `StandardTableView`; its triple license adds friction for a permissive project.
12. **Floem** (Lapce) has a code editor core and Fluent-based localization **[Observed: floem/src/views/localization.rs:1-30; floem/editor-core]**, but no accessibility, small community, and slowing activity.
13. **Dioxus desktop** is still a webview (wry) **[Observed: dioxus/packages/desktop/Cargo.toml:31]**; it trades TypeScript for Rust RSX but loses direct access to the JS editor/grid ecosystem. **Dioxus native (Blitz)** is promising (stylo + taffy + parley + AccessKit feature flags) **[Observed: dioxus/packages/native/Cargo.toml:17-33]** but too young for a v1.
14. Regardless of toolkit, the **grid must not hold all rows in UI memory**: Rust owns result sets (Arrow record batches, spill-to-disk) and serves **windows** (row range × column range) on demand. This is also what makes a future native frontend possible.
15. **i18n synergy**: use **Fluent (.ftl)** as the single message format — `fluent-rs` in Rust (errors, MCP, CLI, native menus) and `@fluent/bundle` in the webview, sharing one set of resource files **[Inferred: both are Mozilla Project Fluent implementations; fluent-rs and fluent.js are present in .refs/repos]**.
16. **Keep the door open**: define the UI ↔ core boundary as a versioned message API (commands, events, binary windows). A later gpui frontend (e.g., "Hacivat Lite") can reuse 100% of the core.
17. Run a **two-week spike** with hard pass/fail benchmarks (section 7) before freezing the ADR, primarily to validate WebKitGTK performance and binary IPC throughput on Linux.

---

## 3. Detailed findings

### 3.1 Tauri v2

#### 3.1.1 Architecture

- Tauri uses **WRY** (webview abstraction) and **TAO** (windowing) **[Observed: tauri/ARCHITECTURE.md:15, 43-47]**. Webviews: WKWebView (macOS/iOS), WebView2 (Windows, Chromium-based, evergreen), WebKitGTK (Linux), Android System WebView **[Inferred: WRY documentation; consistent with `tauri-runtime-wry`]**.
- The Rust process is the app; the UI is HTML/CSS/JS served from a custom protocol. Commands are Rust functions exposed through `#[tauri::command]`; capability/permission files restrict which commands each window may call **[Observed: tauri/crates/tauri/src/ipc/authority.rs, capability_builder.rs exist]**.

#### 3.1.2 IPC transport and binary payloads (critical for large result sets)

Observed mechanics:

- **Request path**: the JS side `fetch`es `ipc://localhost/<cmd>` (custom protocol) with `POST` body; headers carry `Tauri-Callback`, `Tauri-Error`, `Tauri-Invoke-Key`. Response `content-type` decides decoding: `application/json` → `response.json()`, `text/plain` → text, otherwise `arrayBuffer()` **[Observed: tauri/crates/tauri/scripts/ipc-protocol.js:15-50]**.
- Raw request bodies arrive in Rust as `InvokeBody::Raw(Vec<u8>)`; on Android only JSON is supported **[Observed: tauri/crates/tauri/src/ipc/mod.rs:55-63]**.
- Raw responses `InvokeResponseBody::Raw` are sent as `application/octet-stream` **[Observed: tauri/crates/tauri/src/ipc/protocol.rs:344]**.
- **Fallback**: if the custom protocol fails (blocked or CSP error), Tauri permanently switches to `window.ipc.postMessage` for that page **[Observed: ipc-protocol.js:61-84]**. On the postMessage path, raw bytes are serialized as JSON arrays — expensive **[Inferred: protocol.rs:69 serializes `InvokeBody::Raw` with `serde_json::to_string`]**.
- **Channels** (`tauri::ipc::Channel<T>`) stream ordered messages from Rust to JS. Each send increments an index; small payloads are delivered by `webview.eval(...)` directly; larger payloads are inserted into a `ChannelDataIpcQueue` keyed by webview label, and the webview is told (via eval) to `invoke('plugin:__TAURI_CHANNEL__|fetch')` with a header naming the data id **[Observed: channel.rs:270-320, 485-505]**.
  - Thresholds: JSON `< 8192` bytes goes through eval ("8192 byte JSON payload runs roughly 2x faster through eval than through fetch on WebView2 v135"); raw `< 1024` bytes goes through eval ("1024 byte payload runs roughly 30% faster through eval than through fetch on macOS") **[Observed: channel.rs:34-38]**.
  - The queue is scoped per webview and entries are single-use **[Observed: channel.rs:525-540 tests]**; if the webview was closed the payload is dropped **[Observed: channel.rs:306-316]**.
  - Small raw payloads on the eval path are encoded as a JSON number array and rebuilt into `Uint8Array` — fine for ≤1 KB, terrible beyond, which is why the threshold exists **[Observed: channel.rs:302-305]**.

Implications:

- Large result windows should be returned as **`tauri::ipc::Response` with raw bytes** from a command (pull model), or streamed over a `Channel<InvokeResponseBody>` with `Raw` chunks (push model). Both avoid JSON.
- Each window should be **≥ 1 KB** and ideally 64 KB–1 MB to amortize per-message overhead (eval of a bootstrap snippet + a fetch round trip) **[Inferred]**.
- Arrow IPC stream bytes decode in JS with `apache-arrow`'s `tableFromIPC` without per-cell parsing; string columns become `Utf8` vectors with offset buffers **[Inferred: Arrow JS design]**. For a canvas grid only the visible cells' strings need materializing.

#### 3.1.3 Webview differences per OS

| Aspect | macOS (WKWebView) | Windows (WebView2) | Linux (WebKitGTK) |
|---|---|---|---|
| Engine | WebKit (system, tied to OS version) | Chromium (evergreen runtime, can be fixed-version) | WebKit (distro package; version varies widely) |
| Canvas 2D perf | Good, GPU-accelerated | Very good | Variable; GPU compositing depends on driver/distro; historically the weakest **[Inferred: widespread community reports; must be measured]** |
| IME | Native NSTextInputClient via WebKit | TSF via Chromium | IBus/Fcitx via GTK; quality depends on GTK IM module **[Inferred]** |
| Screen readers | VoiceOver via WebKit AX tree | Narrator/NVDA/JAWS via Chromium UIA/IA2 | Orca via ATK/AT-SPI; weakest of the three **[Inferred]** |
| Devtools | Safari Web Inspector | Edge DevTools | WebKit inspector |
| CSS/JS features | Lags Chromium for some APIs (e.g., EditContext unavailable) | Latest Chromium features | Lags; depends on distro |
| Memory baseline | ~moderate (shared system WebKit) | Higher (separate Chromium processes) | Moderate |

Concrete consequences for Hacivat:
- Use only broadly supported web APIs (CodeMirror 6 already handles `EditContext` opportunistically **[Observed: codemirror-view/src/docview.ts:153-160 `updateEditContextFormatting`]**).
- Treat Linux/WebKitGTK as the performance floor in benchmarks.
- Ship an AppImage/Flatpak that pins a known-good WebKitGTK where possible **[Inferred]**.

#### 3.1.4 Packaging, signing, auto-update

- The bundler covers macOS (`.app`, `.dmg`), Windows (MSI/NSIS), Linux (deb, rpm, AppImage) and produces **updater bundles** **[Observed: tauri/crates/tauri-bundler/src/bundle/{macos,windows,linux,updater_bundle.rs}]**. The official updater plugin verifies signatures with a public key embedded in the app **[Inferred: Tauri updater plugin docs]**.
- Code signing hooks exist for macOS notarization and Windows Authenticode (including custom sign commands for cloud HSMs) **[Inferred: Tauri docs; bundler macos/windows modules]**.
- This is a large, often underestimated workload that gpui/egui/iced/Slint/Floem leave to the application (via `cargo-packager`, `cargo-bundle`, `cargo-wix`, custom scripts).

#### 3.1.5 Plugin/extensibility

- Web UI makes **sandboxed plugins** natural: a plugin can be an iframe/webview with a postMessage API and a capability manifest (Beekeeper Studio's plugin system uses this pattern — see `oss-clients-beekeeper-and-tui.md`). Rust-side plugins can be WASM (wasmtime) behind the same capability model **[Inferred]**.

#### 3.1.6 Startup and memory

- Binary size is small (no bundled browser) but runtime memory includes the webview process(es). Typical Tauri hello-world RSS on Windows is dominated by WebView2 processes (tens to low hundreds of MB) **[Inferred: community measurements; to be measured in spike]**.
- Cold start is dominated by webview creation + JS bundle parse; keeping the initial bundle small (code-split editor/grid) matters **[Inferred]**.

### 3.2 gpui (Zed) and gpui-component

#### 3.2.1 Licensing (read this first)

| Crate | License | Usable by permissive Hacivat? |
|---|---|---|
| `gpui` | Apache-2.0 **[Observed: zed/crates/gpui/Cargo.toml:9]** | Yes |
| `gpui_macos`, `gpui_windows`, `gpui_linux`, `gpui_wgpu`, `gpui_platform` | Apache-2.0 **[Observed: respective Cargo.toml]** | Yes |
| `sum_tree` | Apache-2.0 | Yes |
| `editor`, `ui`, `workspace`, `picker` | GPL-3.0-or-later | **No** |
| `rope`, `text` | GPL-3.0-or-later | **No** (note: even the rope is GPL) |
| gpui-component (`gpui-base`, `gpui-component`, …) | Apache-2.0 **[Observed: gpui-component/LICENSE-APACHE; crates/component/Cargo.toml:4]** | Yes |

Clean-room rule: do not read Zed GPL crates for implementation ideas beyond public behavior; do not copy.

#### 3.2.2 Rendering and layout

- "Hybrid immediate and retained mode, GPU accelerated" **[Observed: zed/crates/gpui/README.md:3]**. Layout uses taffy (flexbox/grid) **[Observed: zed/crates/gpui/src/taffy.rs]**. Platform backends: Metal on macOS, wgpu-based renderer for Linux/Windows (`gpui_wgpu`) **[Observed: crate list]**. There is an experimental `gpui_web` (canvas + IME mirror) **[Observed: zed/crates/gpui_web/src/{canvas_text.rs,ime_mirror.rs}, version 0.1.0]**.
- Coordinates are `Pixels(pub(crate) f32)` **[Observed: geometry.rs:2736]**. With f32, integer pixels are exact up to 2^24 = 16,777,216; a 10M-row grid at 24 px/row is 240M px. A grid must therefore virtualize in **row-index space** and only convert the local viewport to pixels (see ALG-GRID-01 in ui-02).

#### 3.2.3 Lists and tables

- `uniform_list` virtualizes fixed-height items and supports scroll strategies (Top/Center/Bottom) and decorations **[Observed: zed/crates/gpui/src/elements/uniform_list.rs:22-253]**.
- `list` supports variable-height items stored in a `SumTree<ListItem>` with an `overdraw` margin and `splice` for incremental updates **[Observed: zed/crates/gpui/src/elements/list.rs:19-68, 310-321, 490]**.
- gpui-component `Table`: `TableVisibleRange { rows, cols }` → **2-axis virtualization**, fixed left/right columns, horizontal scroll handle shared by header and body, three selection modes (row, column, cell), cell selection events, `has_more`/`load_more_threshold`/`load_more` for incremental loading **[Observed: gpui-component/crates/component/src/table/state.rs:57-243, 276-335; delegate.rs:219-274]**.

#### 3.2.4 Text, IME, RTL, font fallback

- `EntityInputHandler` mirrors the platform text input protocol (UTF-16 ranges, marked text, selection rects, bounds for range), and asks apps to notify text services after programmatic edits **[Observed: zed/crates/gpui/src/input.rs:1-40]** — this is the right shape for robust IME (Japanese/Chinese/Korean, Turkish dead keys).
- Text system: CoreText on macOS (`font-kit` feature), DirectWrite on Windows, cosmic-text + swash + `unicode-bidi` on the wgpu path **[Observed: gpui_apple/Cargo.toml:20-47; gpui_wgpu/Cargo.toml:26-36]**. Font fallback API exists (`font_fallbacks`) **[Observed: gpui/src/text_system.rs:1-71]**.
- RTL: shaping can reorder glyphs ("Bidirectional shaping can put glyphs out of byte order") and line layout contains a workaround "to accommodate RTL text … lack of complete" support **[Observed: gpui/src/text_system/line.rs:270; line_layout.rs:253]**. Conclusion: **RTL works for display in common cases but is not complete** (cursor movement, selection, mixed-direction editing).

#### 3.2.5 Accessibility

- gpui integrates **AccessKit**; elements with IDs get `GlobalElementId`s which map to accessibility nodes; there is an a11y example **[Observed: gpui/src/_accessibility.rs:1-60]**. gpui-component has `a11y_disabled.rs` and `macos_accessibility.rs` modules **[Observed: crates/base/src]**. This is new; coverage of complex widgets (grid cells, editor text ranges) must be validated.

#### 3.2.6 Editor

- Zed's editor is GPL — not usable. gpui-component provides `Editor`/`EditorState` with display map, highlighting (tree-sitter optional, regex fallback), search, diagnostics, decorations, indentation and LSP integration: completions, code actions, definitions, document colors, hover, semantic tokens **[Observed: crates/base/src/input/README.md; editor/lsp/*.rs; crates/component/Cargo.toml:22-47]**. Buffer: `ropey::Rope` **[Observed: crates/component/src/input/state.rs:4]**. Multi-cursor and inlay hints: **[Unknown]** — no `inlay` symbol found; verify in spike.

#### 3.2.7 Docking, i18n, theming

- gpui-component has `dock` modules, tabs, resizable panels **[Observed: crates/base/src/dock, crates/component/src/dock]**.
- i18n via `rust-i18n` 4.2 with YAML locale files (`locales/ui.yml`, languages en, pt-BR, ru, zh-CN/HK/TW, it) **[Observed: gpui-component/Cargo.toml:102; crates/component/locales/ui.yml]**. No Turkish yet; adding means contributing YAML keys upstream or overriding.
- Theming: JSON theme schema and themes directory **[Observed: repo root `.theme-schema.json`, `themes/`]**.

### 3.3 egui / egui_extras

- `TableBuilder::body().rows(row_height, total_rows, …)` computes `min_row = floor(scroll_offset_y / row_height_with_spacing)`, `max_row = ceil((scroll_offset_y + max_height)/row_height) + 1`, inserts a top spacer of `min_row × h`, renders only those rows and adds a bottom spacer **[Observed: egui/crates/egui_extras/src/table.rs:1027-1081]**. `heterogeneous_rows` iterates all heights to find the visible window (O(n) per frame) **[Observed: table.rs:1112-1201]**. Columns are **not** virtualized (every column is laid out per visible row) **[Inferred: column loop in TableRow]**.
- `ScrollArea::show_rows` uses the same fixed-height scheme and calls `skip_ahead_auto_ids` for stable IDs **[Observed: egui/crates/egui/src/containers/scroll_area.rs:1087-1118]**.
- Text: shaping via `harfrust` (kerning/ligatures) **[Observed: epaint/Cargo.toml:89; CHANGELOG.md:330]**, **no bidi** **[Observed: epaint/src/text/unicode.rs:15; text_layout.rs:1499 "once RTL/bidi support is added"]**.
- Accessibility via AccessKit **[Observed: egui/crates/egui/Cargo.toml:99; src/accessibility.rs]**. IME supported through winit **[Inferred]**.
- Immediate mode: whole UI re-evaluated each frame on interaction; fine for tools, but a large IDE layout with thousands of widgets costs CPU every frame **[Inferred]**.
- Ecosystem: `egui_dock` (docking), `egui_code_editor` (basic) **[Inferred]**; no LSP-grade editor.

### 3.4 iced

- 0.14.0 (2025-12-07) added reactive rendering, input method support, `table`, `grid`, hot reloading, headless testing, primitive culling in `column`/`row` **[Observed: iced/CHANGELOG.md:9-40]**.
- `table(columns, rows)` takes `impl IntoIterator<Item = T>` and builds a cell per row × column → not virtualized **[Observed: iced/widget/src/table.rs:13-27]**.
- Text: cosmic-text fork (bidi and shaping) **[Observed: iced/Cargo.toml:206]**. Accessibility: none in mainline **[Observed: no accesskit dependency]** (the COSMIC fork of iced has AccessKit work **[Inferred]**).
- `text_editor` exists (basic, highlighter support) **[Observed: widget/src/text_editor.rs]**; `pane_grid` for splits **[Observed: widget/src/pane_grid.rs]**.

### 3.5 Slint

- Triple license: GPL-3.0-only, Royalty-free 2.0 (requires disclosure/attribution, excludes embedded), commercial **[Observed: slint/LICENSE.md:1-40]**. For Hacivat (permissive), the Royalty-free license would be used with an "AboutSlint"/badge disclosure; contributors and downstream forks inherit that obligation.
- Text: parley with `complex-scripts`, skrifa, rustybuzz, ICU normalizer **[Observed: internal/core/Cargo.toml:94-105, 159, 188-204]**. Accessibility: AccessKit in the winit backend; `accessibility-text` feature **[Observed: internal/backends/winit/Cargo.toml; internal/core/Cargo.toml:105]**.
- i18n: `@tr("…")` in `.slint` markup with gettext extraction and bundled translations **[Observed: internal/core/translations.rs; docs reference lexical-structure/expressions mention @tr]**.
- Widgets: `StandardTableView` with sortable columns and rows of `StandardListViewItem` **[Observed: docs/.../standardtableview.mdx]**; `ListView` is virtualized **[Inferred: Slint docs]**. No code editor widget (only `TextEdit`).
- Strengths: tiny footprint, declarative markup, live preview tooling, stable 1.x API.

### 3.6 Floem / Lapce

- Floem: reactive signals, `virtual_list`/`virtual_stack`, text editor view, `localization` view built on `fluent-bundle` + `unic-langid` + `sys-locale` **[Observed: floem/src/views/*.rs; floem/Cargo.toml:113,188]**; text via parley 0.7; renderers vello/vger/tiny-skia **[Observed: floem/Cargo.toml:6,48-61,150-152]**.
- `editor-core` (buffer on `lapce-xi-rope`, cursor, selection, movement, registers, modal/vim mode) **[Observed: floem/editor-core/src; Cargo.toml:15,52]**.
- No AccessKit. Activity slowing (last push 2026-09-24).

### 3.7 Dioxus desktop and Dioxus native (Blitz)

- `dioxus-desktop` uses `wry` with `os-webview` → same webview engines/variance as Tauri, without Tauri's capability system, bundler and updater maturity **[Observed: dioxus/packages/desktop/Cargo.toml:31,63]** (Dioxus CLI `dx bundle` exists **[Inferred]**).
- `dioxus-native`: renderer based on Blitz with features `system-fonts`, `complex-scripts`, `writing-mode`, `accessibility`, `clipboard`, `file-dialog` **[Observed: dioxus/packages/native/Cargo.toml:17-33]**. Promising long-term (real CSS via stylo, parley text, AccessKit) but alpha.

### 3.8 Cross-cutting: what the grid needs from any toolkit

1. A way to draw thousands of text runs per frame cheaply (canvas 2D, GPU text atlas).
2. Scroll position independent of pixel precision (row-index anchoring).
3. Hit testing and keyboard focus for a virtual surface.
4. An accessibility shadow for the virtual surface (ARIA grid with `aria-rowcount`/`aria-rowindex` in web; AccessKit table nodes natively) — see ui-02.
5. Clipboard read/write of multiple formats (text/plain TSV, text/html, custom MIME) — browsers expose `ClipboardItem` with `text/html` + `text/plain`; native toolkits vary (arboard supports text + HTML on major OSes **[Inferred]**).

---

## 4. Algorithm specifications

The grid/editor algorithms are specified in `ui-02-code-editor-and-data-grid-components.md` (ALG-GRID-01..04). This document specifies the **data-window transport** between Rust core and UI, which is the stack-dependent piece.

### ALG-UI-01: Windowed binary result transport (core → UI)

- **Problem:** Display result sets of arbitrary size (millions of rows, hundreds of columns, wide text/BLOB cells) in a UI process/webview without copying the whole result into UI memory or paying JSON costs.
- **Inputs / outputs:**
  - Input (UI → core): `WindowRequest { result_id, generation, row_start, row_end, col_ids[], max_cell_bytes, format: ArrowIpc | Compact, request_id }`.
  - Output (core → UI): `WindowResponse { request_id, result_id, generation, row_start, row_count, columns: [col_id], payload: bytes, truncated_cells: bitmap, row_count_known: bool, total_rows: Option<u64> }`.
- **Algorithm:**
  1. Core keeps each result set as an append-only list of Arrow `RecordBatch`es (in memory up to a budget, then spilled to an on-disk Arrow IPC file with an index of batch offsets). A `generation` counter increments whenever the result is replaced (re-run) or mutated (edits applied, sort changed).
  2. UI computes the visible window (ALG-GRID-01) plus overscan, then **rounds the row range outward to a page grid** (e.g., 256-row pages) so requests are cacheable and overlapping scrolls re-use pages.
  3. UI de-duplicates: if a page is cached for `(result_id, generation, page_no, col_set_hash)` it is not requested again. Column sets are requested as the set of visible columns plus overscan columns, rounded to column pages (e.g., 32 columns).
  4. UI sends at most `K` in-flight requests (K = 2–4); newer requests for the same result **cancel** older not-yet-served ones (scroll fling).
  5. Core resolves the page: locate batches covering `[row_start, row_end)`, slice them (zero-copy in Arrow), project requested columns, and for each variable-length cell longer than `max_cell_bytes` truncate to a prefix and set the bit in `truncated_cells` (full value fetched separately on demand by `CellRequest`).
  6. Serialize: Arrow IPC stream (schema message + one record batch) or a compact custom format (`u32` row count, per column: type tag, validity bitmap, offsets/values). Use `tauri::ipc::Response::new(bytes)` (raw) for pull requests.
  7. UI decodes lazily: it keeps the Arrow buffers and materializes strings only for cells actually painted.
  8. If `generation` in the response differs from the UI's current generation, discard the response.
  9. Streaming while a query is still running: core pushes `RowCountUpdate { result_id, generation, rows_so_far, done }` events over a `Channel` (small JSON, eval path); UI extends the scrollable extent; pages beyond `rows_so_far` are requested only once available.
- **Data structures:** LRU page cache in UI keyed by `(result_id, generation, page_no, col_page_no)`, bounded by bytes (e.g., 64 MB); core-side batch index `Vec<(first_row: u64, batch_ref)>` searched by binary search; spill file with offsets.
- **Complexity / performance notes:** Page lookup O(log B) batches. Serialization O(cells in window). With 256 rows × 32 cols × ~16 B ≈ 128 KB per page, a full viewport (≈60 rows × 20 cols) needs 1 page; flinging through 1M rows only requests pages for frames actually displayed (requests coalesced at ~60 Hz). Raw payloads above 1 KB take Tauri's fetch path **[Observed: channel.rs:37-38]**; keep pages in the 64 KB–1 MB band **[Inferred]**.
- **Edge cases (exhaustive list):**
  1. Result has zero columns (DDL/DML) → no window requests; show affected-rows summary.
  2. Result has zero rows but known columns → header only; `total_rows = 0`, `row_count_known = true`.
  3. Row count unknown while streaming → scroll extent grows; scrollbar shows "≥ n" state.
  4. Query cancelled mid-stream → `done = true` with `cancelled` flag; already fetched rows remain browsable.
  5. Result re-run while pages are in flight → generation mismatch, discard.
  6. Column reorder/hide in UI → column ids are stable; reorder only changes the requested set order; cache keyed by id set.
  7. Extremely wide rows (1000+ columns) → column pages; header metadata sent once per result.
  8. Huge single cells (multi-MB JSON/BLOB) → truncated prefix + `truncated_cells` bit; full value via `CellRequest` with explicit user action and size confirmation.
  9. Binary data → hex/base64 preview generated in core, never raw bytes in text paths.
  10. Non-UTF-8 text from drivers → core converts with replacement character and flags cell as lossy.
  11. NULL vs empty string → validity bitmap distinguishes; UI renders NULL with a distinct style.
  12. Numeric precision (DECIMAL/NUMERIC with 38+ digits, i64/u64 beyond 2^53) → transmit as strings or Decimal128/Int64 with BigInt decoding; never JS `number`.
  13. Timestamps with time zones → transmit epoch + tz id or the original string; format in UI with user locale.
  14. Window closed while request pending → Tauri drops queued payloads for unregistered webviews **[Observed: channel.rs:306-316]**; core should also cancel work.
  15. postMessage fallback active (custom protocol blocked) → detect and reduce page size or show diagnostic; raw bytes become JSON arrays **[Observed: ipc-protocol.js:61-84; protocol.rs:69]**.
  16. Android/iOS (future) → JSON only on Android for requests **[Observed: mod.rs:55]**.
  17. Memory pressure in core → spill batches to disk; pages served from mmap.
  18. Sort/filter applied client-side vs server-side → client-side sort of a partially fetched result is disallowed (would be misleading); sorting re-runs the query or sorts the fully materialized core-side result and bumps generation.
- **Test cases (input → expected):**
  1. 10,000,000-row synthetic result, request rows [9,999,744, 10,000,000) cols [0..20) → 256 rows returned, `row_start` correct, latency < 20 ms in core (spike target).
  2. Two requests for same page in flight → one core call, both UI consumers satisfied.
  3. Re-run query during scroll → stale responses discarded (no flicker of old data).
  4. Cell with 5 MB JSON, `max_cell_bytes = 4096` → 4096-byte prefix, truncated bit set; `CellRequest` returns full value.
  5. DECIMAL `12345678901234567890.123456789` → displayed exactly.
  6. Column with all NULLs → bitmap all zero; no strings materialized.
  7. Custom protocol blocked by CSP in test → fallback path still renders (slower), diagnostic logged.
- **Prior art and evidence:** Tauri channel/raw IPC **[Observed: channel.rs]**; Glide Data Grid `getCellsForSelection` async pull model **[Observed: glide-data-grid/packages/core/src/data-editor/use-column-sizer.ts:117-150]**; Beekeeper uses `limit+1` paging on the backend and a utility process with MessagePorts **[Observed: see oss-clients-beekeeper-and-tui.md]**; harlequin keeps result sets as Arrow tables behind `textual-fastdatatable` **[Observed: harlequin/pyproject.toml:26-28; src/harlequin/query.py:89-93]**.
- **Used by:** Hacivat (UI ↔ core); the same window protocol can serve the local MCP server's paged result tool.

### ALG-UI-02: UI ↔ core API versioning (to keep a future native frontend possible)

- **Problem:** Avoid locking the core to one UI toolkit.
- **Inputs / outputs:** Typed command/event schema (e.g., generated with `specta`/`ts-rs` to TypeScript) with an API version.
- **Algorithm:**
  1. All UI-visible operations are commands `{name, version, args}` returning `Result<T, CoreError{code, fluent_id, args}>`.
  2. Errors carry a **Fluent message id + arguments**, not English text, so both web and native frontends localize identically.
  3. Events are published on topics (`result.rows`, `connection.state`, `job.progress`) with monotonically increasing sequence numbers per topic so a UI can detect gaps.
  4. Binary payloads (ALG-UI-01) are separate from typed JSON envelopes.
  5. On startup the UI sends its supported API range; core rejects incompatible UIs with a clear error.
- **Data structures:** schema registry; per-topic sequence counters.
- **Complexity / performance notes:** negligible.
- **Edge cases:** unknown command (UI newer than core) → `E_UNSUPPORTED`; event gap → UI requests a snapshot; error with missing Fluent id in a locale → fall back to English bundle, then to id.
- **Test cases:** UI v2 against core v1 → handshake error; error id present only in `en` → Turkish UI shows English text, never the raw id.
- **Prior art and evidence:** Tauri commands + capabilities **[Observed: tauri/crates/tauri/src/ipc/{command.rs,authority.rs}]**; rainfrog's `Database` trait isolates drivers behind async tasks polled by the UI loop **[Observed: rainfrog/src/database/mod.rs:101-160]**.
- **Used by:** Hacivat.

---

## 5. Weighted decision matrix

### 5.1 Criteria and weights

| # | Criterion | Weight | Why |
|---|---|---|---|
| C1 | Data grid: 2-axis virtualization, millions of rows, selection, copy/paste | 14 | Core of a DB client |
| C2 | Code editor: completion popups, inlays, gutters, multi-cursor, IME, large files, LSP | 14 | Core of a DB client |
| C3 | Text & i18n: IME, bidi/RTL, font fallback, shaping, locale formatting | 12 | Hard requirement (EN+TR v1, trivial new languages) |
| C4 | Accessibility (screen readers) | 8 | Product quality; legal requirements in some markets |
| C5 | Docking/tabs/split panes | 6 | IDE layout |
| C6 | Startup time & memory & frame performance | 8 | "Feels native/fast" |
| C7 | Packaging, code signing, auto-update | 7 | 3 OSes, small team |
| C8 | Maturity, API stability, community | 10 | Multi-year maintenance |
| C9 | Theming | 4 | Light/dark/high contrast |
| C10 | Plugin/extensibility | 5 | Ecosystem |
| C11 | Code reuse with Perde web panel | 7 | Shared grid/editor/i18n |
| C12 | License fit (permissive Hacivat) | 5 | Distribution freedom |

### 5.2 Scores (1 = poor, 5 = excellent) with evidence

| Criterion (weight) | Tauri v2 + TS | gpui + gpui-component | egui | iced | Slint | Floem | Dioxus desktop | Dioxus native |
|---|---|---|---|---|---|---|---|---|
| C1 Grid (14) | 4 — canvas grids exist (Glide, MIT); windowed fetch needed | 4 — Table with row+col virtualization, fixed cols, load_more | 3 — rows-only virtualization | 2 — table not virtualized | 3 — ListView virtualized, basic table | 3 — virtual_list/stack | 3 — web grids via JS interop | 2 |
| C2 Editor (14) | 5 — CodeMirror 6 / Monaco | 4 — editor with LSP; inlays/multi-cursor unverified | 2 | 2 | 1 | 3 — Lapce-derived core | 3 — JS editors via interop | 1 |
| C3 Text/i18n (12) | 5 — browser engines | 3 — IME strong, RTL partial | 2 — no bidi | 3 — cosmic-text, IME new | 4 — parley + gettext | 3 — parley + fluent | 5 | 3 |
| C4 A11y (8) | 4 — DOM/ARIA; canvas needs shadow | 3 — AccessKit, new | 3 — AccessKit | 1 | 4 — AccessKit | 1 | 4 | 3 |
| C5 Docking (6) | 4 — JS libraries (e.g., dockview) | 4 — gpui-component dock | 4 — egui_dock | 3 — pane_grid | 2 | 2 | 3 | 2 |
| C6 Perf (8) | 3 — webview RSS, Linux variance | 5 | 4 | 4 | 5 | 4 | 3 | 4 |
| C7 Packaging (7) | 5 — bundler + updater | 3 — DIY | 3 | 3 | 3 | 2 | 4 | 3 |
| C8 Maturity (10) | 5 | 3 — pre-1.0, churn | 4 | 3 | 4 | 2 | 3 | 1 |
| C9 Theming (4) | 5 | 4 | 3 | 4 | 4 | 3 | 5 | 4 |
| C10 Plugins (5) | 4 — iframe sandboxes, WASM | 2 | 2 | 2 | 2 | 2 | 3 | 2 |
| C11 Reuse with web (7) | 5 | 2 — gpui_web experimental | 2 | 1 | 1 | 1 | 3 | 3 |
| C12 License (5) | 5 | 5 (avoid GPL crates) | 5 | 5 | 3 | 5 | 5 | 5 |

### 5.3 Weighted totals (max 500 → normalized to 100)

| Candidate | Weighted sum | Normalized |
|---|---|---|
| **Tauri v2 + TypeScript UI** | 56+70+60+32+24+24+35+50+20+20+35+25 = **451** | **90.2** |
| Dioxus desktop | 42+42+60+32+18+24+28+30+20+15+21+25 = 357 | 71.4 |
| gpui + gpui-component | 56+56+36+24+24+40+21+30+16+10+14+25 = 352 | 70.4 |
| Slint | 42+14+48+32+12+40+21+40+16+10+7+15 = 297 | 59.4 |
| egui | 42+28+24+24+24+32+21+40+12+10+14+25 = 296 | 59.2 |
| Floem | 42+42+36+8+12+32+14+20+12+10+7+25 = 260 | 52.0 |
| iced | 28+28+36+8+18+32+21+30+16+10+7+25 = 259 | 51.8 |
| Dioxus native (Blitz) | 28+14+36+24+12+32+21+10+16+10+21+25 = 249 | 49.8 |

### 5.4 Sensitivity analysis

- **"Native purist" weights** (C11 reuse → 0, C6 perf 8 → 15): Tauri 437 → 87.4; gpui 373 → 74.6. Ranking unchanged.
- **"gpui matures" scenario** (gpui C3 → 4, C8 → 4, C7 → 4 if `cargo-packager` + custom updater proven): gpui = 352 + 12 + 10 + 7 = 381 → 76.2. Still below Tauri unless C11 is zeroed *and* C6 doubled *and* gpui's editor proves to be at CodeMirror level (C2 → 5: +14 → 395 → 79). Tauri remains ahead because C2/C3/C4/C7 stay at or above gpui.
- **"Grid is everything"** (C1 weight 30, others scaled down proportionally): Tauri and gpui tie on C1 (4 each); outcome unchanged.

---

## 6. Recommendation, architecture and risks

> **Superseded (2026-11-11):** the product decision is native Rust on GPUI with our own component library — see `../../decisions/0001-native-ui-on-gpui.md`. The analysis below remains valid as research; product positioning (developer trust in "native Rust") was not a criterion in this matrix.

### 6.1 Recommendation

Adopt **Tauri v2** with a **TypeScript UI** and a **Rust core** split into UI-agnostic crates. Use:

| Layer | Choice | Notes |
|---|---|---|
| Shell | Tauri v2 (2.12.x) | Capabilities per window; updater plugin; bundler for all 3 OSes |
| UI framework | React (largest component ecosystem; Glide Data Grid and dockview are React-first) or SolidJS (smaller/faster) | Pick in spike; React default because Perde web panel reuse |
| Data grid | Start with **Glide Data Grid (MIT)** behind an adapter implementing ALG-UI-01; plan an in-house canvas grid ("hgrid") implementing ALG-GRID-01..04 if Glide's limits bite | see ui-02 |
| Editor | **CodeMirror 6** + `@codemirror/lang-sql` + custom completion sources fed by the Rust schema cache; optional LSP bridge to a Rust SQL language server | Monaco as fallback only |
| Docking | dockview (MIT) or golden-layout | evaluate a11y/keyboard |
| i18n | **Fluent** `.ftl` files shared between Rust (`fluent-rs`) and UI (`@fluent/bundle`, `@fluent/react` or equivalent); `Intl.*` for number/date formatting in UI | see section 6 of this doc |
| Data protocol | ALG-UI-01 windows, Arrow IPC (or compact columnar) via raw `Response`; events via `Channel` | |
| Core | `hacivat-core` (drivers, result store, schema cache, query runner), `hacivat-mcp`, `hacivat-net` (Karagöz P2P) | UI-agnostic |

### 6.2 Why not gpui (yet)

gpui is the most credible native path and scored second-tier only because of (1) no permissive editor equal to CodeMirror 6 (Zed's is GPL; gpui-component's is promising but unproven on SQL ergonomics, inlays, multi-cursor and very large files), (2) partial RTL, (3) brand-new accessibility, (4) packaging/updater DIY, (5) pre-1.0 churn, (6) no reuse with Perde's web panel. Revisit at Hacivat v2 with the same benchmark suite; the core/UI boundary (ALG-UI-02) keeps that option cheap.

### 6.3 Risks and mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| WebKitGTK slow canvas / compositing on some Linux distros | Medium | High on Linux | Spike benchmark on Ubuntu LTS, Fedora, Arch, with Intel/AMD/NVIDIA; Flatpak with pinned runtime; reduce overdraw; offer "reduced effects" mode |
| Webview memory footprint criticized vs. "native" apps | High | Medium | Keep JS heap small (no full result sets), lazy-load editor/grid bundles, measure RSS budget (target < 250 MB with 5 tabs) |
| Binary IPC falls back to postMessage (CSP misconfig, Linux quirks) | Low | High | CSP allows `ipc:` scheme; startup self-test sends a 64 KB raw ping and reports transport path |
| Divergent behavior between WKWebView / WebView2 / WebKitGTK (IME, clipboard formats, fonts) | Medium | Medium | Cross-OS E2E tests (WebDriver) for IME composition, clipboard HTML/TSV, font fallback (Turkish, CJK, Arabic samples) |
| Canvas grid inaccessible to screen readers | High if ignored | High | ARIA shadow grid (ui-02 §3.3.6) and full keyboard model |
| Supply-chain risk of npm ecosystem | Medium | Medium | Lockfile, `npm audit`/`osv-scanner`, minimal deps, vendored critical packages |
| Tauri v2 breaking changes | Low | Medium | Pin minor versions, upgrade quarterly |
| Glide Data Grid limits (e.g., astral-plane paste bug, React coupling) | Medium | Medium | Adapter boundary; in-house grid fallback plan |

---

## 7. Spike plan (pass/fail benchmarks before ADR freeze)

| # | Benchmark | Pass criterion |
|---|---|---|
| B1 | Scroll 10M-row × 50-col synthetic result with fling | ≥ 55 fps on macOS/Windows; ≥ 45 fps on Linux reference machine; no blank frames > 100 ms |
| B2 | Raw IPC throughput: 1 MB Arrow page round trip | ≤ 15 ms p50, ≤ 40 ms p95 on all 3 OSes |
| B3 | Editor with 50k-line SQL file | typing latency ≤ 16 ms p95; completion popup ≤ 50 ms after keystroke |
| B4 | IME: Japanese (Kotoeri/MS-IME/Fcitx5), Turkish Q/F keyboards with dead keys, Korean | composition correct in editor, grid cell editor, filter inputs |
| B5 | RTL smoke: Arabic/Hebrew strings in grid cells and editor comments | correct visual order, caret movement, selection |
| B6 | Screen readers: VoiceOver, NVDA, Orca navigate grid cells and announce row/col/header/value | all three announce coordinates and values |
| B7 | Cold start to interactive main window | ≤ 1.0 s macOS/Windows on SSD, ≤ 1.5 s Linux |
| B8 | RSS with 5 open tabs and 1M-row result | ≤ 300 MB total (app + webview) |
| B9 | Signed + notarized macOS build, signed Windows MSI/NSIS, AppImage + Flatpak; updater round trip | automated in CI |

Run B1–B6 also against a gpui + gpui-component prototype for a documented comparison.

---

## 8. Implications for Hacivat / Karagöz / Perde

- **Hacivat**: adopt the layered architecture above; enforce that no UI code calls drivers directly; all result data flows through ALG-UI-01.
- **Perde**: build the web panel with the same UI framework (React or Solid) and share packages: `@hacivat/grid`, `@hacivat/editor`, `@hacivat/i18n` (Fluent loader + `.ftl` files), `@hacivat/design-tokens`.
- **Karagöz**: no UI; but its audit events should carry Fluent ids so Perde and Hacivat render them localized.
- **Local MCP server**: reuses the core's result store and window API (ALG-UI-01) to page large results for agents.

---

## 9. i18n considerations

1. **Single message format (Fluent)** for Rust and TS; files under `locales/<bcp47>/*.ftl`; adding a language = adding a folder + registering the locale in a manifest file read at runtime (no code changes). Native menus (Tauri menu API) are built from the same bundles in Rust.
2. **Formatting**: use `Intl.NumberFormat`, `Intl.DateTimeFormat`, `Intl.RelativeTimeFormat`, `Intl.Collator` in the UI; use ICU4X in Rust for anything formatted server-side (exports, MCP text). Never format numbers in Rust for display in the UI.
3. **Turkish pitfalls** that the stack must handle: dotted/dotless i casing (`"I".toLocaleLowerCase("tr") === "ı"`), so *identifier* case-folding (SQL keywords, completion filters) must use locale-independent folding while *UI text* uses locale-aware functions. CodeMirror's fuzzy matcher uses `toUpperCase()/toLowerCase()` without locale **[Observed: codemirror-autocomplete/src/filter.ts:33-39]** — correct for identifiers. RedisInsight sorts tree nodes with `localeCompare(b, 'en')` **[Observed: RedisInsight/redisinsight/ui/src/helpers/constructKeysToTree.ts:124-128]** — Hacivat should sort *data* by code point or a user-selectable collation, and *UI lists* with `Intl.Collator(uiLocale)`.
4. **Pseudo-localization** build (accented, 30–40% longer strings, RTL-mirrored pseudo locale) to catch truncation and hard-coded strings in CI.
5. **Fonts**: bundle a UI font with Latin Extended (Turkish ğ, ş, ı, İ), and rely on system fallback for CJK/Arabic; verify fallback in all three webviews (B5).
6. **RTL readiness**: use CSS logical properties (`margin-inline-start`), `dir="auto"` on data cells (cell content direction is data-dependent, independent of UI locale), mirrored icons only where semantically directional.
7. **Editor phrases**: CodeMirror UI strings come from the `EditorState.phrases` facet **[Observed: codemirror-state/src/state.ts:305-330]**; Hacivat feeds it from Fluent (see ui-02).

---

## 10. Risks, pitfalls, open questions

- **[Unknown]** Actual Linux WebKitGTK canvas throughput on the reference matrix — must be measured (B1).
- **[Unknown]** Whether gpui-component's editor supports inlay hints and multi-cursor (not found in a symbol search) and how it handles 50k-line buffers.
- **[Unknown]** Tauri updater behavior for Linux deb/rpm installs (AppImage is the supported updater target for Linux **[Inferred]**) — decide distribution channels per OS.
- **[Unknown]** Screen reader quality of a canvas grid with ARIA shadow on WebKitGTK/Orca.
- Pitfall: `postMessage` fallback silently degrades binary performance; add telemetry-free local diagnostics.
- Pitfall: f32 coordinates in native toolkits (gpui, egui) break pixel-anchored virtualization above ~16.7M px; browsers also cap element heights (~33.5M px in Chromium, 7M px is the conservative CodeMirror limit **[Observed: glide-data-grid infinite-scroller.tsx:74; codemirror-view/src/viewstate.ts:52-60]**). Always anchor scroll by row index.
- Pitfall: Slint's royalty-free license requires attribution in the shipped app; GPL path would complicate permissive distribution.
- Open question: React vs SolidJS for the UI — decide by measuring re-render cost of the shell (tabs, tree with 100k nodes using virtualization) in the spike.
