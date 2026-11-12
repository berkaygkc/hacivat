//! The workspace shell: joins the UI crates and the core crates (§5.1). Shell and globals in
//! Task 13, commands and menus in Task 16, the settings UI in Task 17.

pub mod commands;
pub mod l10n_global;
pub mod menus;
pub mod settings_ui;
pub mod shell;
pub mod theme_pref;

use hacivat_ui_core::gpui_surface::{App, Global, SharedString};

/// The pending first chord of a multi-key binding, shown in the status bar (`None` when idle).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct PendingChord(pub Option<SharedString>);

impl Global for PendingChord {}

/// Installs the app's globals, then runs every task's registration.
pub fn init(cx: &mut App) {
    cx.set_global(PendingChord::default());
    // registrations (append-only)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn init_installs_an_idle_pending_chord(cx: &mut TestAppContext) {
        cx.update(|cx| {
            init(cx);
            menus::rebuild(cx);
            assert_eq!(cx.global::<PendingChord>(), &PendingChord(None));
        });
    }
}
