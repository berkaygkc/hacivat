//! The command registry. Every command (palette entry, menu item, key binding) is an
//! [`ActionDef`] registered here; menus reach it through `RunCommand` (F1).

use std::fmt;
use std::rc::Rc;

use hacivat_ui_core::gpui_surface::{App, Global, Window};

/// The body of a command.
pub type ActionFn = Rc<dyn Fn(&mut Window, &mut App)>;

/// One command.
#[derive(Clone)]
pub struct ActionDef {
    /// Locale-free English ID, for example `workspace.open_settings`.
    pub id: &'static str,
    /// The title's Fluent ID: `hv-cmd-<id with . and _ as ->` (R5).
    pub title_id: &'static str,
    /// The key context the command belongs to (`Workspace`, `Palette`, ...).
    pub context: &'static str,
    pub run: ActionFn,
    pub enabled: fn(&App) -> bool,
}

impl fmt::Debug for ActionDef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionDef")
            .field("id", &self.id)
            .field("title_id", &self.title_id)
            .field("context", &self.context)
            .finish_non_exhaustive()
    }
}

/// An action ID registered twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DuplicateAction {
    pub id: &'static str,
}

impl fmt::Display for DuplicateAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "action {} is already registered", self.id)
    }
}

impl std::error::Error for DuplicateAction {}

/// Every registered command, in registration order.
#[derive(Debug, Clone, Default)]
pub struct ActionRegistry {
    defs: Vec<ActionDef>,
}

impl Global for ActionRegistry {}

impl ActionRegistry {
    pub fn register(&mut self, def: ActionDef) -> Result<(), DuplicateAction> {
        if self.get(def.id).is_some() {
            return Err(DuplicateAction { id: def.id });
        }
        self.defs.push(def);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&ActionDef> {
        self.defs.iter().find(|d| d.id == id)
    }

    pub fn all(&self) -> &[ActionDef] {
        &self.defs
    }

    /// Runs the registered, enabled command `id`. Returns whether it ran.
    pub fn run(id: &str, window: &mut Window, cx: &mut App) -> bool {
        let Some(run) = cx
            .try_global::<ActionRegistry>()
            .and_then(|reg| reg.get(id))
            .filter(|def| (def.enabled)(cx))
            .map(|def| Rc::clone(&def.run))
        else {
            return false;
        };
        run(window, cx);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use std::cell::Cell;

    fn def(id: &'static str, hits: Rc<Cell<u32>>, enabled: fn(&App) -> bool) -> ActionDef {
        ActionDef {
            id,
            title_id: "hv-cmd-test",
            context: "Workspace",
            run: Rc::new(move |_, _| hits.set(hits.get() + 1)),
            enabled,
        }
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let hits = Rc::new(Cell::new(0));
        let mut reg = ActionRegistry::default();
        assert_eq!(reg.register(def("t.a", hits.clone(), |_| true)), Ok(()));
        assert_eq!(
            reg.register(def("t.a", hits.clone(), |_| true)),
            Err(DuplicateAction { id: "t.a" })
        );
        assert_eq!(reg.all().len(), 1);
        assert_eq!(reg.get("t.a").map(|d| d.context), Some("Workspace"));
        assert!(reg.get("t.b").is_none());
    }

    #[gpui::test]
    fn run_calls_enabled_actions_only(cx: &mut TestAppContext) {
        let hits = Rc::new(Cell::new(0));
        let mut reg = ActionRegistry::default();
        reg.register(def("t.on", hits.clone(), |_| true)).unwrap();
        reg.register(def("t.off", hits.clone(), |_| false)).unwrap();
        let vcx = cx.add_empty_window();
        vcx.update(|window, cx| {
            assert!(
                !ActionRegistry::run("t.on", window, cx),
                "no registry installed"
            );
            cx.set_global(reg);
            assert!(ActionRegistry::run("t.on", window, cx));
            assert!(!ActionRegistry::run("t.off", window, cx));
            assert!(!ActionRegistry::run("t.missing", window, cx));
        });
        assert_eq!(hits.get(), 1);
    }
}
