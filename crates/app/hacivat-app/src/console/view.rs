//! The console view (Task 32): the `console_view` hook's body, the focused console and the
//! editor status segment.
//!
//! Task 1b stub: a console tab shows the editor of the console registered for it, else exactly
//! today's M2 home (`editor_empty`; R-M3-37 keeps it for an unbound console). No segment.

use hacivat_proto::ProfileId;
use hacivat_ui::panes::TabId;
use hacivat_ui_core::gpui_surface::{
    AnyView, App, AppContext as _, Context, IntoElement, Render, Window,
};
use hacivat_ui_core::theme::Theme;

use crate::console::registry::{ConsoleId, Consoles};

/// The view of one console tab.
struct ConsoleHost {
    tab: TabId,
    n: u32,
    profile: Option<ProfileId>,
}

impl Render for ConsoleHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = (self.n, self.profile);
        let editor = Consoles::by_tab(cx, self.tab)
            .and_then(|id| Consoles::get(cx, id))
            .and_then(|s| s.editor.clone());
        match editor {
            Some(editor) => editor.into_any_element(),
            None => {
                let theme = *Theme::get(cx);
                crate::shell::workspace::editor_empty(&theme, cx)
            }
        }
    }
}

/// The `console_view` hook: the view of console `tab` (`Console <n>`, bound to `profile`),
/// built once per tab.
pub fn build(
    tab: TabId,
    n: u32,
    profile: Option<ProfileId>,
    window: &mut Window,
    cx: &mut App,
) -> AnyView {
    let _ = window;
    cx.new(|_| ConsoleHost { tab, n, profile }).into()
}

/// The console whose editor has the focus, else the one last focused.
pub fn focused_console(window: &Window, cx: &App) -> Option<ConsoleId> {
    let by_editor = Consoles::ids(cx).into_iter().find(|id| {
        Consoles::get(cx, *id)
            .and_then(|s| s.editor.as_ref())
            .is_some_and(|editor| {
                hacivat_ui_core::gpui_surface::Focusable::focus_handle(editor.read(cx), cx)
                    .contains_focused(window, cx)
            })
    });
    by_editor.or_else(|| Consoles::focused(cx))
}

/// The status bar's editor segment of `console` ("Ln 14, Col 49"), its own entity (R-M3-11;
/// Task 32 fills it, Task 34 places it). Task 1b stub: none.
pub fn editor_segment(console: ConsoleId, cx: &App) -> Option<AnyView> {
    let _ = (console, cx);
    None
}
