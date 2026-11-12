//! `ShellHooks`: behaviour a later task plugs into the shell without editing it (F3).

use std::rc::Rc;

use hacivat_ui_core::gpui_surface::{App, Global, KeyDownEvent, Window};
use hacivat_ui_core::keymap::ContextStack;

/// Sees every key down before any element of the workspace does, with the workspace's context
/// stack. Returning `true` consumes the key (propagation stops). Task 16's dispatcher fills it.
pub type KeyCaptureHook = Rc<dyn Fn(&KeyDownEvent, &ContextStack, &mut Window, &mut App) -> bool>;

/// The shell's hook slots.
#[derive(Clone, Default)]
pub struct ShellHooks {
    pub key_capture: Option<KeyCaptureHook>,
}

impl Global for ShellHooks {}

/// Fills the key-capture slot (replacing any earlier hook).
pub fn set_key_capture(cx: &mut App, hook: KeyCaptureHook) {
    cx.default_global::<ShellHooks>().key_capture = Some(hook);
}

/// The installed key-capture hook, if any.
pub(crate) fn key_capture(cx: &App) -> Option<KeyCaptureHook> {
    cx.try_global::<ShellHooks>()
        .and_then(|h| h.key_capture.clone())
}
