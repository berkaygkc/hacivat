//! `RunCommand`, the one generic GPUI action (F1).
//!
//! Hacivat dispatches its own commands (R7); GPUI's action system is used only where the
//! platform needs a GPUI action: application-menu items and macOS menu key equivalents carry a
//! `RunCommand`, and its handler calls `hacivat_ui::actions::ActionRegistry::run(&id, ..)`.

use gpui::SharedString;

/// Runs the Hacivat command `id` (an action ID such as `workspace.open_settings`).
///
/// Built only in code (menus, key bindings), never from JSON keymaps, hence `no_json`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, gpui::Action)]
#[action(namespace = hacivat, no_json)]
pub struct RunCommand {
    pub id: SharedString,
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Action as _;

    #[test]
    fn run_command_is_namespaced_and_compares_by_id() {
        let a = RunCommand {
            id: "app.quit".into(),
        };
        assert_eq!(a.name(), "hacivat::RunCommand");
        assert!(a.partial_eq(&RunCommand {
            id: "app.quit".into()
        }));
        assert!(!a.partial_eq(&RunCommand {
            id: "app.about".into()
        }));
    }
}
