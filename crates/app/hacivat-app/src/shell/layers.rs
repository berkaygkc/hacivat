//! `Layers`: views drawn above the tiles in stack order (dialogs, popup menus, the palette).
//! The workspace owns the stack; Escape pops the topmost layer unless the layer's view handles
//! it first, and popping restores the focus the layer recorded.

use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, EntityId, FocusHandle, InteractiveElement, IntoElement, ParentElement,
    Styled, Subscription, div,
};
use hacivat_ui_core::theme::Theme;

/// The key context of a dialog layer.
pub const DIALOG_CONTEXT: &str = "Dialog";
/// The key context of a popup-menu layer.
pub const MENU_CONTEXT: &str = "Menu";
/// How much of the ground shows through behind a dialog (the dialog host dims the workspace).
const SCRIM_ALPHA: f32 = 0.6;

/// One view above the tiles.
pub struct Layer {
    pub id: &'static str,
    /// Pushed onto `Workspace::context_stack` while the layer is open.
    pub context: &'static str,
    pub view: AnyView,
    /// Focused again when the layer is popped; `push_layer` records the focused element when
    /// this is `None`.
    pub restore_focus: Option<FocusHandle>,
    /// A modal layer dims and blocks what is under it, and while one is open only the top
    /// layer's bindings (and `app.quit`) resolve (P47). `open_dialog` sets it.
    pub modal: bool,
}

struct Entry {
    layer: Layer,
    /// Keeps the host's subscription to the layer's events alive while it is open.
    _subscription: Option<Subscription>,
}

/// The layer stack, bottom first.
#[derive(Default)]
pub struct Layers {
    stack: Vec<Entry>,
}

impl Layers {
    pub fn push(&mut self, layer: Layer, subscription: Option<Subscription>) {
        self.stack.push(Entry {
            layer,
            _subscription: subscription,
        });
    }

    /// Removes the topmost layer.
    pub fn pop(&mut self) -> Option<Layer> {
        self.stack.pop().map(|e| e.layer)
    }

    /// Removes the layer whose view is `view`.
    pub fn remove_view(&mut self, view: EntityId) -> Option<Layer> {
        let at = self
            .stack
            .iter()
            .position(|e| e.layer.view.entity_id() == view)?;
        Some(self.stack.remove(at).layer)
    }

    pub fn ids(&self) -> Vec<&'static str> {
        self.stack.iter().map(|e| e.layer.id).collect()
    }

    pub fn contexts(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.stack.iter().map(|e| e.layer.context)
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    /// Whether any open layer is modal.
    pub fn has_modal(&self) -> bool {
        self.stack.iter().any(|e| e.layer.modal)
    }

    /// The top layer's context while a modal layer is open (P47).
    pub fn modal_context(&self) -> Option<&'static str> {
        if !self.has_modal() {
            return None;
        }
        self.stack.last().map(|e| e.layer.context)
    }

    /// The layers as elements covering the window, bottom first. A modal layer sits centred
    /// over a dimmed ground that takes every click; other layers place themselves (a popup menu
    /// at its anchor, dismissing itself on a press outside).
    pub fn elements(&self, theme: &Theme) -> Vec<AnyElement> {
        let c = &theme.tokens.colors;
        self.stack
            .iter()
            .map(|entry| {
                let layer = &entry.layer;
                let view = layer.view.entity_id();
                let mut overlay = div()
                    .id(("layer", view.as_u64()))
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full();
                if layer.modal {
                    let mut scrim = c.ground.to_hsla();
                    scrim.a = SCRIM_ALPHA;
                    overlay = overlay
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(scrim)
                        .occlude();
                }
                overlay.child(layer.view.clone()).into_any_element()
            })
            .collect()
    }
}
