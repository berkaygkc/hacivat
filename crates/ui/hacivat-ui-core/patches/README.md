# GPUI carry patches

These are patches against GPUI at `zed@912159c7ad77ce6296a93be77efc298b0b00fd50`. The files
here are the reviewable record. The build does not apply them itself: it gets them through
the Hacivat zed fork (ruling R1).

## How they are applied

- **Fork:** `https://github.com/berkaygkc/zed`, branch `hacivat/912159c`.
- **Pinned commit:** `4858c0911c04a11c47255d9cd17338f407e6a1af`. It is `912159c` plus one
  commit, "gpui_macos: discard marked text on FocusLost (Hacivat carry)", which carries the
  patch below and adds `HACIVAT-PATCHES.md` at the fork root.
- The workspace `Cargo.toml` pins `gpui` and `gpui_platform` to that commit. `deny.toml`
  allows the fork as a git source, and `cargo xtask check-deps` treats it as zed-sourced
  (`ZED_SOURCES`).
- **To add a patch:** commit it on the fork branch, list it in `HACIVAT-PATCHES.md`, add
  the patch file here, then move the pin in `Cargo.toml` and the `Cargo.lock` sources to the
  new commit.

## `gpui-discard-marked-text-on-focus-lost.patch`

- **Target:** `crates/gpui_macos/src/window.rs`.
- **What it does:** it implements `PlatformWindow::text_input_state_changed`. At this pin,
  macOS uses the default no-op (`crates/gpui/src/platform.rs:1464`).
- **When it acts:** only on `TextInputStateChange::FocusLost`. GPUI sends that when the
  window's focused text input goes from Some to None (`crates/gpui/src/window.rs:3733-3741`).
  - That is a real focus change. An app switch never triggers it.
  - It never acts on `ContentChanged` or `SelectionChanged`. `notify_text_input_changed`
    sends those after every IME edit (`window.rs:5663-5679`), so acting on them would end
    every composition.
- **What it does on `FocusLost`:** it calls `-[[view inputContext] discardMarkedText]`. The
  call is deferred to the foreground executor, so it runs after the current draw's focus
  listeners. By then the app's `on_blur` has committed the marked text locally. This
  follows the AppKit contract: the client unmarks first, then the input method drops its
  copy.
- **Which M0 gaps it closes** (see `docs/benchmarks/m0-ime-checklist.md`, "Known gaps"):
  - It fixes the in-window programmatic focus move.
  - It does **not** fix a focus move while the window is inactive. That case would also
    need GPUI to deliver the blur, or to send `FocusLost`, for an inactive window.
- **Status:** applied via the fork at `4858c09`. It applied cleanly to `912159c`. The fork
  commit differs from this file in one way only: `rustfmt` reflowed the `use gpui::{…}`
  import list, which this file leaves unformatted. Behaviour is identical, and the fork
  commit is the authoritative copy. `cargo check -p gpui_macos` and
  `cargo clippy -p gpui_macos` pass in the fork checkout with Rust 1.98.1, without warnings.
- **Still open:**
  - Gap 2, a focus move while the window is inactive, stays open. It needs a change in
    `gpui` itself, on the same fork branch, and is assigned to M5 (editor IME, UI-EDIT-12).
  - The manual IME check of this patch needs the palette and settings inputs from Wave 4,
    so it runs in Task 19, Step 3.

## Apache-2.0 obligations when carrying a patch

- Keep zed's `LICENSE-APACHE` with the patched sources. At this pin, zed has no `NOTICE`
  file. If a later pin adds one, carry it too.
- Modified files must say they were changed (section 4(b)). The patch adds a header
  comment to `window.rs` saying so.
- The patched crate stays Apache-2.0. Do not copy anything from zed's GPL crates
  (`editor`, `text`, `rope`, `ui`, `workspace`) into a patch.
