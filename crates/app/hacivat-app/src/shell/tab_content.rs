//! `TabContentRegistry`: tab kinds other tasks register (Task 17's settings tab), so the shell
//! opens them without knowing them (F3).

use std::rc::Rc;

use hacivat_ui::text::TextFn;
use hacivat_ui_core::gpui_surface::{AnyView, App, Global, Window};

/// What a tab holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TabKind {
    /// A SQL console, `Console <n>`.
    Console { n: u32 },
    /// Content registered under this kind in the [`TabContentRegistry`].
    Registered(&'static str),
}

/// Builds a registered tab's view (once per tab).
pub type BuildFn = Rc<dyn Fn(&mut Window, &mut App) -> AnyView>;

/// A registered tab kind: its title, how to build its view, and its key context.
#[derive(Clone)]
pub struct TabContent {
    /// Re-read on every render, so the tab follows a language switch.
    pub title: TextFn,
    pub build: BuildFn,
    /// The key context while the tab is active (`Workspace::context_stack`).
    pub context: &'static str,
}

/// Registered tab kinds, in registration order.
#[derive(Clone, Default)]
pub struct TabContentRegistry {
    contents: Vec<(&'static str, TabContent)>,
}

impl Global for TabContentRegistry {}

impl TabContentRegistry {
    /// Registers `kind`, replacing an earlier registration of the same kind.
    pub fn register(&mut self, kind: &'static str, content: TabContent) {
        match self.contents.iter_mut().find(|(k, _)| *k == kind) {
            Some(entry) => entry.1 = content,
            None => self.contents.push((kind, content)),
        }
    }

    pub fn get(&self, kind: &str) -> Option<&TabContent> {
        self.contents
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, c)| c)
    }

    /// The registered spelling of `kind` (for restoring a saved tab).
    pub(crate) fn kind_named(&self, kind: &str) -> Option<&'static str> {
        self.contents
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|(k, _)| *k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacivat_ui_core::gpui_surface::SharedString;

    fn content(context: &'static str) -> TabContent {
        TabContent {
            title: Rc::new(|_: &App| SharedString::default()),
            build: Rc::new(|_: &mut Window, _: &mut App| unreachable!("not built in this test")),
            context,
        }
    }

    #[test]
    fn a_second_registration_replaces_the_first() {
        let mut reg = TabContentRegistry::default();
        reg.register("settings", content("A"));
        reg.register("settings", content("B"));
        assert_eq!(reg.get("settings").map(|c| c.context), Some("B"));
        assert_eq!(reg.kind_named("settings"), Some("settings"));
        assert!(reg.get("other").is_none());
    }
}
