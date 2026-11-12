//! Debug builds only: the programmatic focus move for the IME check (Task 19).
//! `HACIVAT_DEBUG_FOCUS_MOVE_MS=<n>` makes the first IME composition schedule a focus move to
//! the palette's query field `n` ms later, opening the palette when needed.

use std::time::Duration;

use hacivat_ui::input::FocusedInput;
use hacivat_ui_core::gpui_surface::{App, Global};

use crate::commands::palette::open_palette;
use crate::shell::Workspace;

/// The environment variable that turns the focus move on (milliseconds).
pub const FOCUS_MOVE_ENV: &str = "HACIVAT_DEBUG_FOCUS_MOVE_MS";

/// Whether the focus move still waits for its composition.
struct FocusMoveArmed(bool);

impl Global for FocusMoveArmed {}

/// Installs the focus move from `HACIVAT_DEBUG_FOCUS_MOVE_MS`, when it is set to a number.
pub fn install_from_env(cx: &mut App) {
    let delay = std::env::var(FOCUS_MOVE_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok());
    if let Some(ms) = delay {
        install_focus_move(cx, Duration::from_millis(ms));
    }
}

/// On the first `FocusedInput.composing == true`, moves the focus to the palette's query field
/// `delay` later (opening the palette in the workspace window if it is not open).
pub fn install_focus_move(cx: &mut App, delay: Duration) {
    cx.set_global(FocusMoveArmed(true));
    cx.observe_global::<FocusedInput>(move |cx| {
        let composing = cx.global::<FocusedInput>().composing;
        let armed = cx.try_global::<FocusMoveArmed>().is_some_and(|a| a.0);
        if !(composing && armed) {
            return;
        }
        cx.set_global(FocusMoveArmed(false));
        let timer = cx.background_executor().timer(delay);
        cx.spawn(async move |cx| {
            timer.await;
            cx.update(|cx| {
                let window = cx
                    .active_window()
                    .into_iter()
                    .chain(cx.windows())
                    .find(|w| w.downcast::<Workspace>().is_some());
                if let Some(window) = window {
                    let _ = window.update(cx, |_, window, cx| open_palette(window, cx));
                }
            });
        })
        .detach();
    })
    .detach();
}
