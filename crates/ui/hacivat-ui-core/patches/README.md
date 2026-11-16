# GPUI carry patches

These are patches against GPUI at `zed@912159c7ad77ce6296a93be77efc298b0b00fd50`. The files
here are the reviewable record. The build does not apply them itself: it gets them through
the Hacivat zed fork (ruling R1).

## How they are applied

- **Fork:** `https://github.com/berkaygkc/zed`, branch `hacivat/912159c`.
- **Pinned commit:** `6607045170abc9654afad74be2e868a829744737` (M3a Task 1a). It is
  `912159c` plus three commits, each carrying one patch below:
  1. `4858c09` "gpui_macos: discard marked text on FocusLost (Hacivat carry)". It also adds
     `HACIVAT-PATCHES.md` at the fork root.
  2. `a1eaa20` "gpui: report FocusLost then FocusGained on a text input to text input move
     (Hacivat carry)".
  3. `6607045` "scheduler: take state before rng in block_until (Hacivat carry)". It also
     widens the fork notice's scope sentence to "`crates/gpui*` and `crates/scheduler`".
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
- **When it acts:** only on `TextInputStateChange::FocusLost`. At `912159c`, GPUI sends that
  only when the window's focused text input goes from Some to None
  (`crates/gpui/src/window.rs:3733-3741`). With the second patch it also sends it, followed
  by `FocusGained`, when focus moves directly from one text input to another.
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
  - It fixes the in-window programmatic focus move from a text input to a non-input target.
  - Together with the second patch, it also fixes the in-window move from one text input
    to another, e.g. the settings search to the command palette (M1 gate, Known gap 1).
  - It does **not** fix a focus move while the window is inactive. That case would also
    need GPUI to deliver the blur, or to send `FocusLost`, for an inactive window.
- **Status:** applied via the fork since `4858c09`. It applied cleanly to `912159c`. The fork
  commit differs from this file in one way only: `rustfmt` reflowed the `use gpui::{…}`
  import list, which this file leaves unformatted. Behaviour is identical, and the fork
  commit is the authoritative copy. `cargo check -p gpui_macos` and
  `cargo clippy -p gpui_macos` pass in the fork checkout with Rust 1.98.1, without warnings.
- **Still open:**
  - Gap 2, a focus move while the window is inactive, stays open. It needs a change in
    `gpui` itself, on the same fork branch, and is assigned to M5 (editor IME, UI-EDIT-12).
  - The manual IME check of this patch needs the palette and settings inputs from Wave 4,
    so it runs in Task 19, Step 3.

## `gpui-focus-lost-on-input-to-input-move.patch`

- **Target:** `crates/gpui/src/window.rs`, in `Window::draw`. The patch also touches
  `crates/gpui/src/input.rs` (tests only) and a comment in `crates/gpui_macos/src/window.rs`.
- **The gap it closes:** at `912159c`, `Window::draw` calls `text_input_state_changed` only
  when `previous.is_some() != focused_text_input.is_some()`. A move from one text input
  straight to a different one (Some(a) to Some(b)) sent nothing, so the first patch never
  discarded. Found at the M1 gate: compose in the settings search, then let a programmatic
  move focus the palette's input. The palette was empty, but the candidate window stayed
  open and the input method kept the composition live.
- **What it does:** for Some(a) to Some(b) with a different focus id, it sends `FocusLost`
  and then `FocusGained`.
  - Some to None and None to Some are unchanged.
  - Some(a) to Some(a), a redraw or a refocus of the same input, sends nothing.
- **Order in Hacivat, for an a to b move during a composition:**
  1. In the draw, GPUI installs b's input handler and sends `FocusLost` then `FocusGained`.
     The macOS carry schedules `discardMarkedText`; `FocusGained` is a no-op on macOS.
  2. The focus listeners run. a's `TextInput` `on_blur` commits its marked text locally
     (`ImeState::blur` with an active window), so a keeps the composed text.
  3. The deferred `discardMarkedText` runs. The input method drops its copy, so nothing is
     replayed into b.

  This is the same order as the Some to None case. `TextInput` needs no change.
- **Other platforms:** Linux and Windows use GPUI's default no-op. Hacivat does not build
  `gpui_web` or `gpui_ios`.
- **Tests (in the fork):** a new `input::tests::focus_move_between_text_inputs_reports_loss_then_gain`
  asserts the a to b sequence. `text_notifications_require_a_focused_input_and_track_field_changes`
  moves focus between two text inputs, so its expected sequences gain the pair. Both fail
  without the `window.rs` change. `cargo test -p gpui --features test-support --lib`
  (423 passed, 2 ignored) and `cargo clippy -p gpui -p gpui_macos` (no warnings) pass with
  Rust 1.98.1.
- **Format:** the diff of fork commit `a1eaa20` against `4858c09`, limited to the three
  source files (`HACIVAT-PATCHES.md` is left out). It applies on top of the first patch.
- **Status:** applied via the fork at `a1eaa20`. The manual IME re-check (settings search
  to palette with `HACIVAT_DEBUG_FOCUS_MOVE_MS`) is for the PO.

## `scheduler-block-until-lock-order.patch`

- **Target:** `crates/scheduler/src/test_scheduler.rs`, in `TestScheduler::block_until`. The
  `scheduler` crate is Apache-2.0 (`license = "Apache-2.0"`, `LICENSE-APACHE` links to the
  root licence; checked at the fork head before the patch, R-M3-13). It is GPUI's executor
  crate; `gpui` depends on it, and only its test scheduler changes.
- **The deadlock it fixes** (M2b Task 22 round 1, again in Task 24; the shots harness comment
  in `crates/app/hacivat-app/tests/shots.rs`, `stop_core_runtime`): for a timed block,
  `block_until` computed its tick budget with
  `self.rng.lock().random_range(self.state.lock().timeout_ticks.clone())`. The `rng` guard is
  live while `state` is locked. `schedule_local` and `schedule_background_with_priority` lock
  `state` and then `rng`. `App::shutdown` blocks on the quit futures with a timeout; a core
  worker that wakes a GPUI task from its own thread at that moment schedules it there, and
  the two threads deadlock. Tests with parking allowed hit it intermittently when they quit
  while core workers still run. Shipped code is not affected: production runs GPUI's platform
  scheduler.
- **What it does:** it reads `timeout_ticks` under `state`, drops that guard, then locks `rng`.
  Every path in the file now takes `state` before `rng` (line numbers at `a1eaa20`: `:132`
  holds a temporary `rng` guard only in the `while` condition, `:252` takes `rng` alone,
  `:388`/`:391` take `rng` under `state`), and no `rng` guard is held while `state` is
  locked. The draw is the same single `random_range` call over the same range, so every
  seed's schedule is unchanged.
- **Tests:** in the fork, `cargo +1.98.1 test -p scheduler` (31 passed), `clippy -p scheduler
  --tests` (no warnings). In Hacivat, `crates/app/hacivat-app/tests/teardown_stress.rs`, under
  a 60 s watchdog per cycle:
  - `timed_blocks_while_another_thread_schedules_never_hang` runs timed blocks while another
    thread spawns tasks without pause. It hung in 5 of 5 local runs at `a1eaa20` and passes
    at `6607045`.
  - `fifty_core_boot_and_quit_cycles_never_hang` boots the app with the core runtime 50 times,
    connects through `FakeDriver` and quits for real. It guards the whole teardown path,
    though it rarely hits the one-expression window.
- **Format:** the diff of fork commit `6607045` against `a1eaa20`, limited to
  `test_scheduler.rs` (`HACIVAT-PATCHES.md` is left out).
- **Status:** applied via the fork at `6607045`, which the workspace `Cargo.toml` pins. The
  harness's own `stop_core_runtime` stays: it stops the runtime outside the quit block, which
  is still the cheaper teardown for the shots.

## Apache-2.0 obligations when carrying a patch

- Keep zed's `LICENSE-APACHE` with the patched sources. At this pin, zed has no `NOTICE`
  file. If a later pin adds one, carry it too.
- Modified files must say they were changed (section 4(b)). Each patch adds a header
  comment to the files it modifies saying so.
- The patched crate stays Apache-2.0. Do not copy anything from zed's GPL crates
  (`editor`, `text`, `rope`, `ui`, `workspace`) into a patch.
