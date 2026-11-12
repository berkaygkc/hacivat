//! The key dispatcher (UI-EDIT-13): resolves each key down against the active keymap with the
//! workspace's context stack, keeps the pending first presses of a multi-key chord (they expire
//! after [`PENDING_TIMEOUT`]) and never fires a binding during IME composition (F15).

use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_ui_core::gpui_surface::{KeyDownEvent, SharedString};
use hacivat_ui_core::keymap::{
    ContextStack, KeyPress, KeyboardLayoutId, Keymap, Resolution, display_chord, normalize,
};

/// How long the first presses of a multi-key chord wait for the next key.
pub const PENDING_TIMEOUT: Duration = Duration::from_millis(1000);

/// What a key down does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// Run the command with this action ID.
    Run(String),
    /// A chord is under way; the label shows the presses so far (for the status bar).
    Pending(SharedString),
    /// Not a binding: the key goes on to the focused element (text, AltGr text, the IME).
    PassThrough,
}

/// The dispatcher state: the keymap in effect, its layout and a chord in progress.
#[derive(Debug, Clone)]
pub struct Dispatcher {
    pub keymap: Arc<Keymap>,
    pub pending: Vec<KeyPress>,
    /// When the last pending press came in.
    pub since: Option<Instant>,
    pub layout: KeyboardLayoutId,
}

impl Dispatcher {
    pub fn new(keymap: Arc<Keymap>, layout: KeyboardLayoutId) -> Self {
        Self {
            keymap,
            pending: Vec::new(),
            since: None,
            layout,
        }
    }

    /// Follows a new active keymap (a preset switch or a keyboard layout change, F11). A chord in
    /// progress is dropped.
    pub fn set_keymap(&mut self, keymap: Arc<Keymap>, layout: KeyboardLayoutId) {
        self.keymap = keymap;
        self.layout = layout;
        self.clear_pending();
    }

    /// Drops a chord in progress.
    pub fn clear_pending(&mut self) {
        self.pending.clear();
        self.since = None;
    }

    /// Resolves one key down at `now`. `ctx` is the workspace's context stack and `composing`
    /// whether the focused input is composing (then only `during_composition` bindings run).
    pub fn on_key_down(
        &mut self,
        ev: &KeyDownEvent,
        ctx: &ContextStack,
        composing: bool,
        now: Instant,
    ) -> DispatchOutcome {
        if self
            .since
            .is_some_and(|since| now.saturating_duration_since(since) >= PENDING_TIMEOUT)
        {
            self.clear_pending();
        }
        // A dead key never matches a binding (and ends a chord in progress).
        let Some(press) = normalize(&ev.keystroke) else {
            self.clear_pending();
            return DispatchOutcome::PassThrough;
        };
        match self
            .keymap
            .resolve(ctx, &self.pending, &press, &self.layout, composing)
        {
            Resolution::Command(command) => {
                self.clear_pending();
                DispatchOutcome::Run(command)
            }
            Resolution::Pending => {
                self.pending.push(press);
                self.since = Some(now);
                DispatchOutcome::Pending(
                    display_chord(&self.pending, self.keymap.platform()).into(),
                )
            }
            Resolution::PassThrough => {
                self.clear_pending();
                DispatchOutcome::PassThrough
            }
        }
    }
}
