# GPUI carry patches (not applied)

These are patches against GPUI at `zed@912159c7ad77ce6296a93be77efc298b0b00fd50`. They are
kept here for review. **Nothing in the build applies them.** To use one, apply it in a fork
of zed, or to a vendored copy of `gpui_macos`, and point the `gpui_platform` dependency at
that copy.

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
- **Status:** written against the pinned source, but not compiled. Nothing may be built
  inside `.refs/`. Compile it and test it manually in the fork before relying on it.

## Apache-2.0 obligations when carrying a patch

- Keep zed's `LICENSE-APACHE` with the patched sources. At this pin, zed has no `NOTICE`
  file. If a later pin adds one, carry it too.
- Modified files must say they were changed (section 4(b)). The patch adds a header
  comment to `window.rs` saying so.
- The patched crate stays Apache-2.0. Do not copy anything from zed's GPL crates
  (`editor`, `text`, `rope`, `ui`, `workspace`) into a patch.
