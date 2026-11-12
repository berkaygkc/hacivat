//! The workspace shell: joins the UI crates and the core crates (§5.1). Shell and globals in
//! Task 13, commands and menus in Task 16, the settings UI in Task 17.

pub mod commands;
pub mod l10n_global;
pub mod menus;
pub mod settings_ui;
pub mod shell;
pub mod theme_pref;

pub use shell::{AppState, SessionOverrides, Startup, start};

use hacivat_ui_core::gpui_surface::{App, Global, SharedString};

/// The pending first chord of a multi-key binding, shown in the status bar (`None` when idle).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct PendingChord(pub Option<SharedString>);

impl Global for PendingChord {}

/// Installs the app's globals (the pending chord, the tab-content registry, the shell hooks,
/// the action registry and the toast queue), then runs every task's registration. Settings,
/// localization, theme and keymap come from [`start`], which calls this once per app.
pub fn init(cx: &mut App) {
    cx.set_global(PendingChord::default());
    shell::install_globals(cx);
    // registrations (append-only)
    commands::install(cx);
    settings_ui::install(cx);
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
            assert!(cx.has_global::<shell::TabContentRegistry>());
            assert!(cx.has_global::<shell::ShellHooks>());
        });
    }
}
