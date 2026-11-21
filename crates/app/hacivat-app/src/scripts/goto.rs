//! Go to script (HV-SCRIPT-09, R-M4b-9; NOTES surface 7): its own layer over M4's
//! [`Palette`] (there is no palette mode), listing scripts by name across the libraries, with
//! the connection chooser after a pick.
//!
//! ## m4b-t01b-seams: the stub opens an empty Go-to-script layer: its placeholder, no row, and
//! Escape closes it. Task 15 fills the rows from the "Go to script" index (Turkish-tolerant,
//! `commands::palette::search_opts`) and the chooser ([`super::chooser`]). The `scripts.goto`
//! command stays disabled until then, so nothing on a user path opens this layer.

use hacivat_ui::palette::{Palette, PaletteEvent};
use hacivat_ui_core::gpui_surface::{
    App, AppContext as _, Context, Entity, Focusable as _, Global, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Styled as _, Subscription, WeakEntity, Window, div,
    px,
};
use hacivat_ui_core::theme::Theme;

use crate::commands::palette::PALETTE_CONTEXT;
use crate::l10n_global::text_fn_id;
use crate::shell::{Layer, Workspace};

/// The Go-to-script layer's ID.
pub const GOTO_LAYER: &str = "scripts-goto";

/// The layer view around the palette (placed as the command palette is).
pub struct GotoHost {
    palette: Entity<Palette>,
    _subscription: Subscription,
}

impl GotoHost {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let palette = cx.new(|cx| Palette::new(cx, text_fn_id("hv-scripts-goto-ph")));
        let subscription = cx.subscribe_in(
            &palette,
            window,
            |_, _, event: &PaletteEvent, window, cx| match event {
                // No row to confirm and nothing to rank in the stub.
                PaletteEvent::QueryChanged(_) | PaletteEvent::Confirmed(_) => {}
                PaletteEvent::Dismissed => close(window, cx),
            },
        );
        Self {
            palette,
            _subscription: subscription,
        }
    }

    /// The layer's palette (tests read its rows).
    pub fn palette(&self) -> &Entity<Palette> {
        &self.palette
    }
}

impl Render for GotoHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx).tokens;
        let width = (t.spacing.completion_list_width + t.spacing.completion_detail_width)
            .min(f32::from(window.viewport_size().width) - 2.0 * t.spacing.lg);
        div()
            .absolute()
            .top(px(t.spacing.titlebar_height + t.spacing.sm))
            .left_0()
            .w_full()
            .flex()
            .justify_center()
            .child(
                div()
                    .id("scripts-goto-layer")
                    .w(px(width.max(0.0)))
                    .on_mouse_down_out(cx.listener(|_, _, window, cx| close(window, cx)))
                    .child(self.palette.clone()),
            )
    }
}

/// The open Go-to-script host (one per app, as the command palette's).
#[derive(Default)]
struct OpenGoto(Option<WeakEntity<GotoHost>>);

impl Global for OpenGoto {}

/// Opens the Go-to-script layer (an open one just takes the focus). Stub: no row.
pub fn open(window: &mut Window, cx: &mut App) {
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    if ws.read(cx).layer_ids().contains(&GOTO_LAYER)
        && let Some(host) = open_host(cx)
    {
        let focus = host.read(cx).palette.focus_handle(cx);
        window.focus(&focus, cx);
        return;
    }
    let host = cx.new(|cx| GotoHost::new(window, cx));
    let focus = host.read(cx).palette.focus_handle(cx);
    ws.update(cx, |ws, cx| {
        ws.push_layer(
            Layer {
                id: GOTO_LAYER,
                context: PALETTE_CONTEXT,
                view: host.clone().into(),
                restore_focus: None,
                modal: false,
            },
            window,
            cx,
        );
    });
    window.focus(&focus, cx);
    cx.set_global(OpenGoto(Some(host.downgrade())));
}

/// The open Go-to-script layer's view, if any.
pub fn open_host(cx: &App) -> Option<Entity<GotoHost>> {
    cx.try_global::<OpenGoto>()?.0.as_ref()?.upgrade()
}

/// Pops the layer when it is the topmost one (the focus goes back to where it was).
fn close(window: &mut Window, cx: &mut App) {
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    if ws.read(cx).layer_ids().last() == Some(&GOTO_LAYER) {
        ws.update(cx, |ws, cx| ws.pop_layer(window, cx));
    }
}
