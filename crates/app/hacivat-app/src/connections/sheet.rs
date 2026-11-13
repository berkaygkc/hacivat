//! The connection panel sheet: the selected profile's details and actions (NOTES surface 8, Task 18).
//!
//! Key-value rows for the explorer's selection: environment (mark and word), engine, host or
//! file, user, access (`hv-conn-access-ro-guarded` when read-only, R10), path, encryption
//! (once connected) and where the password comes from. With nothing selected it says so.
//!
//! M2b (Task 22, NOTES M2b surface 6): the Access row sits under Environment. A read-only
//! connection reads "read-only · enforced" or "read-only · guarded" (short forms, so Turkish
//! fits the value column); while it is live the value is a borderless button and a Tab stop
//! (lock 12 px, red only on production, and the label, underlined on hover) that opens the
//! read-only explanation. Before the role probe has run (not connected) it says guarded, the
//! claim Hacivat can make without it. Read-write reads "read-write" in text-2. The Path row
//! says `SSH via <bastion>` for a tunnel. Debug selector of the button: `sheet-access`.

use std::cell::Cell;
use std::rc::Rc;

use hacivat_l10n::{NoArgs, ids};
use hacivat_proto::{AccessLabel, DbmsKind, EnvTag};
use hacivat_secrets::{AuthProviderId, Retention};
use hacivat_session::LiveState;
use hacivat_session::profile::{Profile, ProfileEndpoint};
use hacivat_ui::capsule::env_swatch;
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::popover::{A11yDecl, Declare as _, Tip};
use hacivat_ui::text::{Label, TextSource};
use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, App, AppContext as _, Bounds, Context, FocusHandle,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement, Pixels, Render, Role,
    ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled, Subscription, Window,
    canvas, div, point, px,
};
use hacivat_ui_core::theme::{Theme, focus_outline};

use crate::connections::capsule::{encryption_label, engine_label, env_kind, env_word, path_label};
use crate::connections::explorer::endpoint_text;
use crate::connections::model::ConnectionsModel;
use crate::connections::{access_popover, session_toasts};
use crate::l10n_global::tr;
use crate::shell::tiles::{self, empty_state_wrapping, panel_header, panel_title};
use hacivat_ui::text::scroll_viewport;

/// The width of the key column.
const KEY_PX: f32 = 92.0;

/// The sheet view (the `connection_panel` hook).
/// A key/value row's height: one ↑/↓ scroll step.
const ROW_PX: f32 = 22.0;
/// The scroll thumb's width and shortest length.
const THUMB_PX: f32 = 4.0;
const THUMB_MIN_PX: f32 = 16.0;

pub struct SheetView {
    _subscription: Subscription,
    // ## m2a-t21-shots
    /// The rows' scroll position, for the scroll thumb.
    scroll: ScrollHandle,
    /// The rows take the focus (a Tab stop while they overflow) and scroll from the keyboard.
    focus: FocusHandle,
    // ## m2b-t22-live
    /// The Access value's button (a Tab stop while the connection is live and read-only).
    access_focus: FocusHandle,
    /// Where the Access value was painted: the explanation opens under it.
    access_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// The pointer is over the Access value (its label is underlined).
    access_hover: bool,
}

/// The `connection_panel` hook.
pub fn build(_window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| SheetView {
        _subscription: cx.observe_global::<ConnectionsModel>(|_, cx| cx.notify()),
        scroll: ScrollHandle::new(),
        focus: cx.focus_handle(),
        access_focus: cx.focus_handle(),
        access_bounds: Rc::new(Cell::new(Bounds::default())),
        access_hover: false,
    })
    .into()
}

/// Where the password comes from (`secret-*`).
fn secret_label(cx: &App, p: &Profile) -> SharedString {
    let id = match p.auth {
        AuthProviderId::PgPass => ids::hv::CONN_SECRET_PGPASS,
        AuthProviderId::None | AuthProviderId::ClientCert => ids::hv::CONN_SECRET_NONE,
        AuthProviderId::Password => match p.retention {
            Retention::Forever => ids::hv::CONN_SECRET_FOREVER,
            Retention::ForSession => ids::hv::CONN_SECRET_SESSION,
            Retention::UntilRestart => ids::hv::CONN_SECRET_RESTART,
            Retention::Never => ids::hv::CONN_SECRET_NEVER,
        },
    };
    tr(cx, id, NoArgs)
}

impl SheetView {
    // ## m2a-t21-shots
    /// The scroll thumb (DESIGN.md: 3 px radius, on the scroll track's tone) while the rows are
    /// taller than the tile: 4 px wide at the right edge, its length and place in proportion.
    /// Read from the last layout, so it shows from the frame after the rows overflow.
    fn thumb(theme: &Theme, scroll: &ScrollHandle) -> Option<AnyElement> {
        let max = f32::from(scroll.max_offset().y);
        let view = f32::from(scroll.bounds().size.height);
        if max < 0.5 || view <= 0.0 {
            return None;
        }
        let t = theme.tokens;
        let length = (view * view / (view + max)).max(THUMB_MIN_PX);
        let at = (-f32::from(scroll.offset().y) / max).clamp(0.0, 1.0) * (view - length);
        Some(
            div()
                .debug_selector(|| "sheet-scroll-thumb".to_string())
                .absolute()
                .right(px(2.0))
                .top(px(at))
                .w(px(THUMB_PX))
                .h(px(length))
                .rounded(px(t.radii.xs))
                .bg(t.colors.seam_strong.to_hsla())
                .into_any_element(),
        )
    }

    /// Scrolls the rows from the keyboard (they are read-only: no row takes the focus).
    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        if stroke.modifiers.modified() {
            return;
        }
        let max = f32::from(self.scroll.max_offset().y);
        if max < 0.5 {
            return;
        }
        let at = -f32::from(self.scroll.offset().y);
        let page = (f32::from(self.scroll.bounds().size.height) - ROW_PX).max(ROW_PX);
        let to = match stroke.key.as_str() {
            "up" => at - ROW_PX,
            "down" => at + ROW_PX,
            "pageup" => at - page,
            "pagedown" => at + page,
            "home" => 0.0,
            "end" => max,
            _ => return,
        };
        let x = self.scroll.offset().x;
        self.scroll.set_offset(point(x, px(-to.clamp(0.0, max))));
        cx.stop_propagation();
        cx.notify();
    }

    fn kv(theme: &Theme, index: usize, key: SharedString, value: AnyElement) -> impl IntoElement {
        let t = theme.tokens;
        let c = &t.colors;
        div()
            .flex()
            .flex_row()
            .items_center()
            .min_h(px(ROW_PX))
            .gap(px(t.spacing.md))
            .child(
                div().w(px(KEY_PX)).flex_none().child(
                    // ## m2a-t21-shots: a long key ellipsizes in its 92 px column.
                    Label::new(
                        key,
                        TextSource::Fluent,
                        t.type_ramp.caption,
                        c.text_2.to_hsla(),
                    )
                    .truncate(true)
                    .id(SharedString::from(format!("sheet-key-{index}"))),
                ),
            )
            .child(div().flex_1().min_w(px(0.0)).child(value))
    }

    // ## m2b-t22-live
    /// The Access row's value (see the module docs).
    fn access_value(
        &self,
        theme: &Theme,
        p: &Profile,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = theme.tokens;
        let c = &t.colors;
        let live = cx.global::<ConnectionsModel>().live(p.id);
        let access = match &live {
            LiveState::Connected(info) => session_toasts::effective_access(cx, p.id, info.access),
            _ if p.read_only => AccessLabel::ReadOnlyGuarded,
            _ => AccessLabel::ReadWrite,
        };
        let label = match access {
            AccessLabel::ReadWrite => {
                return Label::new(
                    tr(cx, ids::hv::CAPSULE_ACCESS_READ_WRITE, NoArgs),
                    TextSource::Fluent,
                    t.type_ramp.body_sm,
                    c.text_2.to_hsla(),
                )
                .truncate(true)
                .id("sheet-access-label")
                .into_any_element();
            }
            AccessLabel::ReadOnlyEnforced => tr(cx, ids::hv::CONN_RO_SHEET_ENFORCED, NoArgs),
            AccessLabel::ReadOnlyGuarded => tr(cx, ids::hv::CONN_RO_SHEET_GUARDED, NoArgs),
        };
        // Hue Is Meaning: the lock is red only on production.
        let lock = if p.environment == Some(EnvTag::Production) {
            c.red
        } else {
            c.text_2
        };
        let explains = access_popover::facts_for(cx, p.id).is_some();
        let mut text = Label::new(
            label.clone(),
            TextSource::Fluent,
            t.type_ramp.body_sm,
            c.text.to_hsla(),
        )
        .truncate(true)
        .id("sheet-access-label");
        if explains && self.access_hover {
            let whole = std::iter::once(0..label.len()).collect();
            text = text.highlight(whole, c.text.to_hsla());
        }
        let value = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(t.spacing.sm))
            .min_w(px(0.0))
            .child(
                Icon::new(IconName::Lock)
                    .size(IconSize::Chrome)
                    .color(lock.to_hsla()),
            )
            .child(div().min_w(px(0.0)).child(text));
        if !explains {
            return value.into_any_element();
        }
        let bounds = self.access_bounds.clone();
        let focus = self.access_focus.clone().tab_stop(true);
        let focused = focus.is_focused(window) && window.last_input_was_keyboard();
        let id = p.id;
        let (on_click_focus, on_click_bounds) = (focus.clone(), bounds.clone());
        let key_bounds = bounds.clone();
        let hint = tr(cx, ids::hv::CMD_CONNECTION_EXPLAIN_ACCESS, NoArgs);
        let description = hint.clone();
        focus_outline(
            div()
                .id("sheet-access")
                .debug_selector(|| "sheet-access".to_string())
                .relative()
                .flex()
                .min_w(px(0.0))
                .rounded(px(t.radii.xs))
                .cursor_pointer()
                .track_focus(&focus)
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    if this.access_hover != *hovered {
                        this.access_hover = *hovered;
                        cx.notify();
                    }
                }))
                // Enter or Space on key-down, never held or with a modifier; the default is
                // prevented, so GPUI's key-up click does not open it a second time.
                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                    let stroke = &event.keystroke;
                    if event.is_held
                        || stroke.modifiers.modified()
                        || !matches!(stroke.key.as_str(), "enter" | "space")
                    {
                        return;
                    }
                    window.prevent_default();
                    cx.stop_propagation();
                    access_popover::open(id, key_bounds.get(), window, cx);
                })
                .on_click(move |_, window, cx| {
                    window.focus(&on_click_focus, cx);
                    access_popover::open(id, on_click_bounds.get(), window, cx);
                })
                .child(
                    canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full(),
                )
                .child(value)
                // Fix round 1 (M5): what it does, as its tooltip and its description.
                .tooltip(move |_, cx| Tip::view(hint.clone(), cx))
                .declare_full(A11yDecl::new(Role::Button, label).description(description)),
            theme,
            focused,
        )
        .into_any_element()
    }

    fn rows(
        theme: &Theme,
        p: &Profile,
        access: AnyElement,
        cx: &App,
    ) -> Vec<(SharedString, AnyElement)> {
        let t = theme.tokens;
        let c = &t.colors;
        let m = cx.global::<ConnectionsModel>();
        let live = m.live(p.id);
        let info = match &live {
            LiveState::Connected(info) => Some(info.clone()),
            _ => None,
        };
        let fluent = |s: SharedString| {
            Label::new(s, TextSource::Fluent, t.type_ramp.body_sm, c.text.to_hsla())
                .truncate(true)
                .into_any_element()
        };
        let data = |s: SharedString| {
            Label::new(s, TextSource::Data, t.type_ramp.body_sm, c.text.to_hsla())
                .truncate(true)
                .into_any_element()
        };
        let env = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(t.spacing.sm))
            .child(env_swatch(env_kind(p.environment), theme))
            .child(fluent(env_word(cx, p.environment)))
            .into_any_element();
        let file = matches!(p.endpoint, ProfileEndpoint::File { .. });
        let mut rows = vec![
            (tr(cx, ids::hv::CONN_KV_ENV, NoArgs), env),
            // ## m2b-t22-live: the Access row sits under Environment (NOTES M2b surface 6).
            (tr(cx, ids::hv::CONN_KV_ACCESS, NoArgs), access),
            (
                tr(cx, ids::hv::CONN_KV_ENGINE, NoArgs),
                data(engine_label(p, info.as_deref())),
            ),
            (
                if file {
                    tr(cx, ids::hv::CONN_KV_FILE, NoArgs)
                } else {
                    tr(cx, ids::hv::CONN_KV_HOST, NoArgs)
                },
                data(endpoint_text(p).into()),
            ),
        ];
        if let Some(user) = &p.user {
            rows.push((
                tr(cx, ids::hv::CONN_KV_USER, NoArgs),
                data(user.clone().into()),
            ));
        }
        if p.engine != DbmsKind::Sqlite {
            // ## m2b-t22-live: an SSH tunnel's path.
            rows.push((
                tr(cx, ids::hv::CONN_KV_PATH, NoArgs),
                fluent(path_label(cx, p, info.as_deref()).0),
            ));
        }
        if let Some(info) = &info {
            rows.push((
                tr(cx, ids::hv::CONN_KV_ENC, NoArgs),
                fluent(encryption_label(cx, p, info)),
            ));
        }
        if p.engine != DbmsKind::Sqlite {
            rows.push((
                tr(cx, ids::hv::CONN_KV_SECRET, NoArgs),
                fluent(secret_label(cx, p)),
            ));
        }
        rows
    }
}

impl Render for SheetView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let Some(m) = cx.try_global::<ConnectionsModel>() else {
            // P17: no model, the M1 panel.
            return tiles::connection_panel(&theme, cx).into_any_element();
        };
        let t = theme.tokens;
        let c = &t.colors;
        let header = panel_header(&theme).child(panel_title(
            &theme,
            tr(cx, ids::hv::PANEL_CONNECTION, NoArgs),
            "panel-connection-title",
        ));
        let body = match m.selected().and_then(|id| m.get(id)) {
            None => empty_state_wrapping(
                &theme,
                tr(cx, ids::hv::CONN_SHEET_NONE, NoArgs),
                "sheet-none",
            )
            .into_any_element(),
            Some(p) => {
                let mut body = div()
                    .flex()
                    .flex_col()
                    .px(px(t.spacing.xl))
                    .py(px(t.spacing.md))
                    .child(
                        div().pb(px(t.spacing.sm)).child(
                            Label::new(
                                p.name.clone(),
                                TextSource::Data,
                                t.type_ramp.body,
                                c.text.to_hsla(),
                            )
                            .truncate(true)
                            .id("sheet-name"),
                        ),
                    );
                let access = self.access_value(&theme, &p, window, cx);
                for (i, (key, value)) in Self::rows(&theme, &p, access, cx).into_iter().enumerate()
                {
                    body = body.child(Self::kv(&theme, i, key, value));
                }
                // ## m2a-t21-shots: a short tile scrolls its rows instead of cutting the last
                // ones off under the status bar, and a scroll thumb says there is more.
                // Keyboard: a Tab stop only while the rows overflow; ↑/↓ by a row, PgUp/PgDn
                // by a page, Home/End to the ends.
                let overflows = f32::from(self.scroll.max_offset().y) >= 0.5;
                let focus = self.focus.clone().tab_stop(overflows);
                let focused = overflows && focus.is_focused(window);
                let rows = focus_outline(
                    div()
                        .id("sheet-rows")
                        .debug_selector(|| "sheet-rows".to_string())
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll)
                        .track_focus(&focus)
                        .on_key_down(cx.listener(Self::on_key)),
                    &theme,
                    focused,
                )
                .child(scroll_viewport())
                .child(body);
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(rows)
                    .children(Self::thumb(&theme, &self.scroll))
                    .into_any_element()
            }
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(c.side.to_hsla())
            .child(header)
            .child(body)
            .into_any_element()
    }
}
