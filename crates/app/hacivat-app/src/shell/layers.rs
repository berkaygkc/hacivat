//! `Layers`: views drawn above the tiles in stack order (dialogs, popup menus, the palette).
//! The workspace owns the stack; Escape pops the topmost layer unless the layer's view handles
//! it first, and popping restores the focus the layer recorded.

use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, EntityId, FocusHandle, InteractiveElement, IntoElement, ParentElement,
    Styled, Subscription, div, px,
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

    /// The layers as elements, bottom first. A modal layer sits centred over a dimmed ground
    /// that takes every click; the scrim starts below the title bar, so the capsule (the
    /// environment and the connection state) is never dimmed (NOTES cross-cutting #1, PO Q1),
    /// while the title bar turns inert (`TitleBar::inert`). Other layers cover the window and
    /// place themselves (a popup menu at its anchor, dismissing itself on a press outside).
    /// Debug selectors: `layer-<id>`.
    pub fn elements(&self, theme: &Theme) -> Vec<AnyElement> {
        let t = theme.tokens;
        let c = &t.colors;
        self.stack
            .iter()
            .map(|entry| {
                let layer = &entry.layer;
                let view = layer.view.entity_id();
                let id = layer.id;
                let overlay = div()
                    .id(("layer", view.as_u64()))
                    .debug_selector(move || format!("layer-{id}"))
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0();
                let mut overlay = if layer.modal {
                    overlay.top(px(t.spacing.titlebar_height))
                } else {
                    overlay.top_0()
                };
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        AppContext as _, Bounds, Context, Pixels, Render, TestAppContext, Window, px, size,
    };
    use hacivat_ui_core::theme::ThemeMode;

    struct Card;

    impl Render for Card {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .debug_selector(|| "probe-card".to_string())
                .w(px(200.0))
                .h(px(100.0))
        }
    }

    struct Host {
        layers: Layers,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let theme = *Theme::get(cx);
            div()
                .relative()
                .size_full()
                .children(self.layers.elements(&theme))
        }
    }

    /// The overlay and card bounds of one layer in an 800 × 600 window.
    fn overlay_bounds(cx: &mut TestAppContext, modal: bool) -> (Bounds<Pixels>, Bounds<Pixels>) {
        cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
        let (_, vcx) = cx.add_window_view(|_, cx| {
            let mut layers = Layers::default();
            layers.push(
                Layer {
                    id: "probe",
                    context: DIALOG_CONTEXT,
                    view: cx.new(|_| Card).into(),
                    restore_focus: None,
                    modal,
                },
                None,
            );
            Host { layers }
        });
        vcx.simulate_resize(size(px(800.0), px(600.0)));
        vcx.run_until_parked();
        (
            vcx.debug_bounds("layer-probe").expect("overlay drawn"),
            vcx.debug_bounds("probe-card").expect("card drawn"),
        )
    }

    fn modal_overlay_bounds(cx: &mut TestAppContext) -> Bounds<Pixels> {
        overlay_bounds(cx, true).0
    }

    #[gpui::test]
    fn modal_scrim_starts_below_the_title_bar(cx: &mut TestAppContext) {
        let b = modal_overlay_bounds(cx);
        assert_eq!(b.origin.y, px(40.0));
        assert_eq!((b.left(), b.bottom()), (px(0.0), px(600.0)));
        assert_eq!(b.size.width, px(800.0));
        // The dialog is centred in the area under the title bar.
        let (overlay, card) = overlay_bounds(cx, true);
        assert_eq!(card.center(), overlay.center());
        // A non-modal layer (a popup menu) still covers the window and places itself.
        let (popover, _) = overlay_bounds(cx, false);
        assert_eq!(popover.origin.y, px(0.0));
    }
}
