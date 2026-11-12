//! Explanation popover (NOTES surface 7): a titled card explaining a capsule badge, opened with
//! `Workspace::open_anchored`. Task 9 draws it; the Task 1 stub renders an empty `div`.

use hacivat_ui_core::gpui_surface::{
    AnyView, App, AppContext as _, Context, IntoElement, Render, SharedString, Window, div,
};

use crate::icon::IconName;

/// An explanation card (NOTES surface 7).
pub struct ExplainPopover {
    #[allow(dead_code, reason = "Task 9")]
    title: SharedString,
    #[allow(dead_code, reason = "Task 9")]
    icon: IconName,
    #[allow(dead_code, reason = "Task 9")]
    body: Vec<SharedString>,
}

impl ExplainPopover {
    /// The popover as a view, ready for a layer.
    pub fn view(
        title: SharedString,
        icon: IconName,
        body: Vec<SharedString>,
        cx: &mut App,
    ) -> AnyView {
        cx.new(|_| ExplainPopover { title, icon, body }).into()
    }
}

impl Render for ExplainPopover {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
