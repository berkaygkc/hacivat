//! Tabs: what each tab kind is called and which key context it adds. Tabs store kinds (IDs and
//! numbers), never rendered titles, so a language switch renames them on the next frame.

use hacivat_l10n::{Count, ids};
use hacivat_ui_core::gpui_surface::{App, SharedString};

use crate::l10n_global::{tr, tr_id};
use crate::shell::tab_content::{TabContentRegistry, TabKind};

/// The key context of a console tab.
pub const CONSOLE_CONTEXT: &str = "Console";

/// The title of a tab of `kind` in the current language. A registered kind whose registration
/// is gone shows its kind name.
pub fn title_of(kind: TabKind, cx: &App) -> SharedString {
    match kind {
        TabKind::Console { n, .. } => tr(
            cx,
            ids::hv::TAB_CONSOLE_UNTITLED,
            ids::hv::HvTabConsoleUntitled {
                n: Count(u64::from(n)),
            },
        ),
        TabKind::Registered(name) => match registered(cx, name) {
            Some(content) => (content.title)(cx),
            None => tr_id(cx, name),
        },
    }
}

/// The key context a tab of `kind` adds while active (none for a kind nobody registered).
pub fn context_of(kind: TabKind, cx: &App) -> Option<&'static str> {
    match kind {
        TabKind::Console { .. } => Some(CONSOLE_CONTEXT),
        TabKind::Registered(name) => registered(cx, name).map(|c| c.context),
    }
}

fn registered<'a>(cx: &'a App, name: &str) -> Option<&'a crate::shell::tab_content::TabContent> {
    cx.try_global::<TabContentRegistry>()?.get(name)
}
