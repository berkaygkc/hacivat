//! The explorer list of profiles, by environment, with filter and keyboard (NOTES surface 8, Task 18).
//!
//! - **Filter.** A `TextInput` matched through `PreparedQuery` (Turkish-tolerant for Turkish
//!   users, D-074) against each profile's name and host. Its `InputEvent::Cancelled` (Escape
//!   in the field) runs the shell's Escape hook (P16).
//! - **Groups** follow `EnvTag::severity_rank`, then "No environment" last (R31), each with a
//!   small-caps header and its count.
//! - **Rows** (23 px): the engine glyph, the name, the environment mark, flags (a read-only lock,
//!   red only on production; lock-open while the live connection is plaintext), then the live
//!   state (✓ and latency, the run marker, or ⚠) and the engine on the right.
//! - **Selection** is `blue-tint` with a 1 px blue edge. The list registers the key context
//!   `Explorer` (R20, P4): its bindings (Enter, ⌘E, ⌘D, ⌘⇧C, ⌫, Shift-F10) act on the
//!   selection; ↑/↓ move it here. Focusing the list with nothing selected selects the first
//!   row. Right-click opens the connection menu, double-click connects.
//! - **Empty state** (NOTES §8): the title, one sentence, New connection, Paste URL and a ghost
//!   Import pg_service.conf…
//!
//! Debug selectors: `explorer-list`, `explorer-row-<i>` (the visible rows, top down); label IDs
//! `explorer-group-<i>`, `explorer-group-count-<i>` and `explorer-row-name-<i>`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::panic::Location;
use std::rc::Rc;
use std::sync::Arc;

use hacivat_l10n::{MatchMode, NoArgs, PreparedQuery, SearchOpts, build_key, ids};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_session::LiveState;
use hacivat_session::profile::{Profile, ProfileEndpoint};
use hacivat_ui::actions::ActionRegistry;
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::capsule::env_swatch;
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::input::{InputEvent, TextInput};
use hacivat_ui::menu::MenuItemSpec;
use hacivat_ui::step_trail::run_marker;
use hacivat_ui::text::{Label, RenderedText, TextFn, TextRegistry, TextSource};
use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, App, AppContext as _, Bounds, Context, Entity, FocusHandle, Global, Hsla,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, ParentElement,
    Pixels, Point, Render, SharedString, StatefulInteractiveElement, Styled, Subscription,
    WeakEntity, Window, canvas, div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode, TypeStyle};

use crate::commands::palette::search_opts;
use crate::connections::capsule::{engine_label, env_kind, is_unencrypted, latency_label};
use crate::connections::model::ConnectionsModel;
use crate::l10n_global::{text_fn, tr};
use crate::shell::hooks::ShellHooks;
use crate::shell::keymap_state::shortcut_label;
use crate::shell::tiles::{self, panel_header, panel_title};
use crate::shell::{Workspace, register_focus_context};

/// The key context of the focused list (R20).
pub const EXPLORER_CONTEXT: &str = "Explorer";
/// A row's height (NOTES §8).
const ROW_PX: f32 = 23.0;
/// A group header's height.
const GROUP_PX: f32 = 26.0;

/// The explorer the shell built, for the focus commands.
struct ExplorerHandle(WeakEntity<ExplorerView>);

impl Global for ExplorerHandle {}

/// The explorer view (the `explorer` hook).
pub struct ExplorerView {
    filter: Entity<TextInput>,
    list_focus: FocusHandle,
    /// Each visible row's bounds, from its last frame (the keyboard menu's anchor).
    row_bounds: Rc<RefCell<HashMap<ProfileId, Bounds<Pixels>>>>,
    _subscriptions: Vec<Subscription>,
}

/// The `explorer` hook: builds the view, registers the list's `Explorer` key context (P4).
pub fn build(window: &mut Window, cx: &mut App) -> AnyView {
    let view = cx.new(|cx| ExplorerView::new(window, cx));
    let list = view.read(cx).list_focus.clone();
    register_focus_context(cx, list, EXPLORER_CONTEXT);
    cx.set_global(ExplorerHandle(view.downgrade()));
    view.into()
}

fn explorer(cx: &App) -> Option<Entity<ExplorerView>> {
    cx.try_global::<ExplorerHandle>()?.0.upgrade()
}

/// Focuses the filter field (`explorer.focus_filter`).
pub fn focus_filter(window: &mut Window, cx: &mut App) {
    if let Some(view) = explorer(cx) {
        let handle = view.read(cx).filter_focus(cx);
        window.focus(&handle, cx);
    }
}

/// Whether `id`'s row is visible under the current filter (true without an explorer view).
pub fn is_visible(cx: &App, id: ProfileId) -> bool {
    match explorer(cx) {
        Some(view) => view.read(cx).visible_ids(cx).contains(&id),
        None => true,
    }
}

/// Focuses the list (its `Explorer` bindings then apply).
pub fn focus_list(window: &mut Window, cx: &mut App) {
    if let Some(view) = explorer(cx) {
        let handle = view.read(cx).list_focus.clone();
        window.focus(&handle, cx);
    }
}

/// The search options of the filter: contains, Turkish-tolerant for Turkish users.
fn filter_opts(cx: &App) -> SearchOpts {
    SearchOpts {
        mode: MatchMode::Contains,
        ..search_opts(cx)
    }
}

/// A profile's host or file, as the filter and the sheet see it.
pub fn endpoint_text(p: &Profile) -> String {
    match &p.endpoint {
        ProfileEndpoint::Server { host, .. } => host.clone(),
        ProfileEndpoint::Socket { path, .. } | ProfileEndpoint::File { path, .. } => {
            path.display().to_string()
        }
    }
}

/// The visible groups for `query`: by severity, then "No environment" (R31); file order inside
/// a group.
pub fn visible_groups(cx: &App, query: &str) -> Vec<(Option<EnvTag>, Vec<Arc<Profile>>)> {
    let Some(m) = cx.try_global::<ConnectionsModel>() else {
        return Vec::new();
    };
    let opts = filter_opts(cx);
    let prepared = PreparedQuery::new(query, &opts);
    let shown = |p: &Profile| {
        prepared.matches(&build_key(&p.name, &opts)).is_some()
            || prepared
                .matches(&build_key(&endpoint_text(p), &opts))
                .is_some()
    };
    let mut order: Vec<Option<EnvTag>> = EnvTag::ALL.iter().copied().map(Some).collect();
    order.sort_by_key(|t| t.map(EnvTag::severity_rank));
    order.push(None);
    order
        .into_iter()
        .filter_map(|tag| {
            let rows: Vec<Arc<Profile>> = m
                .profiles()
                .iter()
                .filter(|p| p.environment == tag && shown(p))
                .cloned()
                .collect();
            (!rows.is_empty()).then_some((tag, rows))
        })
        .collect()
}

fn group_title(cx: &App, tag: Option<EnvTag>) -> SharedString {
    let id = match tag {
        Some(EnvTag::Production) => ids::hv::EXPLORER_GROUP_PRODUCTION,
        Some(EnvTag::Staging) => ids::hv::EXPLORER_GROUP_STAGING,
        Some(EnvTag::Test) => ids::hv::EXPLORER_GROUP_TEST,
        Some(EnvTag::Dev) => ids::hv::EXPLORER_GROUP_DEV,
        Some(EnvTag::Local) => ids::hv::EXPLORER_GROUP_LOCAL,
        None => ids::hv::EXPLORER_GROUP_UNSET,
    };
    tr(cx, id, NoArgs)
}

// ## m2a-t21-shots
/// How readily a row's name shrinks against its meta (flex weights): the meta gives way first.
const NAME_SHRINK: f32 = 0.01;
/// How much of a name a row keeps at the least: about six characters (and the ellipsis).
const NAME_MIN_CHARS: usize = 6;

/// The narrowest a row's name gets: its first [`NAME_MIN_CHARS`] characters and the ellipsis,
/// or the whole name when it is that short.
fn name_min_width(name: &str, style: TypeStyle, window: &mut Window) -> Pixels {
    let mut chars = name.chars();
    let head: String = chars.by_ref().take(NAME_MIN_CHARS).collect();
    let text: SharedString = if chars.next().is_some() {
        format!("{head}…").into()
    } else {
        head.into()
    };
    let run = hacivat_ui_core::gpui_surface::TextRun {
        len: text.len(),
        font: font_for(&style),
        color: hacivat_ui_core::gpui_surface::hsla(0.0, 0.0, 0.0, 1.0),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(text, px(style.size_px), &[run], None)
        .width
}

/// A wrapping paragraph recorded in the text registry like a label (`Label` draws one line),
/// so the pseudo-locale and test checks see it. `id` is appended to its path as `#id`.
#[track_caller]
pub(crate) fn paragraph(
    text: SharedString,
    style: TypeStyle,
    color: Hsla,
    id: &'static str,
) -> AnyElement {
    let at = Location::caller();
    let recorded = text.clone();
    // Recorded while painted (as `Label`), so the registry keeps paint order.
    let record = canvas(
        move |_, _, cx| {
            TextRegistry::record(cx, || RenderedText {
                text: recorded.clone(),
                source: TextSource::Fluent,
                size_px: style.size_px,
                painted_px: 0.0,
                clipped: false,
                truncated: false,
                path: format!(
                    "{}:{}:{}#{id}",
                    at.file().replace('\\', "/"),
                    at.line(),
                    at.column()
                )
                .into(),
            });
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_0();
    div()
        .relative()
        .child(record)
        .w_full()
        .font(font_for(&style))
        .text_size(px(style.size_px))
        .line_height(px(style.line_height))
        .text_color(color)
        .child(text)
        .into_any_element()
}

/// Records its parent's bounds while it is prepainted (the keyboard menu's anchor).
fn bounds_probe(
    slot: Rc<RefCell<HashMap<ProfileId, Bounds<Pixels>>>>,
    id: ProfileId,
) -> impl IntoElement {
    canvas(
        move |bounds, _, _| {
            slot.borrow_mut().insert(id, bounds);
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// The connection menu's items for `id` (NOTES §8): Connect or Disconnect, Edit…,
/// Duplicate, Copy as URL, a separator, Delete…. They run on the selection.
pub fn menu_items(cx: &App, id: ProfileId) -> Vec<MenuItemSpec> {
    let live = cx
        .try_global::<ConnectionsModel>()
        .map(|m| m.live(id))
        .unwrap_or_default();
    let connected = matches!(live, LiveState::Connected(_) | LiveState::Connecting(_));
    let enabled = |action: &str| {
        cx.try_global::<ActionRegistry>()
            .and_then(|r| r.get(action))
            .is_some_and(|d| (d.enabled)(cx))
    };
    let item = |label: TextFn, action: &'static str| {
        // A command that cannot run now (a read-only store) shows disabled.
        let mut item = MenuItemSpec::new(label, action).disabled(!enabled(action));
        if shortcut_label(cx, action).is_some() {
            let keys: TextFn =
                Rc::new(move |cx: &App| shortcut_label(cx, action).unwrap_or_default().into());
            item = item.keys(keys);
        }
        item
    };
    vec![
        item(
            text_fn(if connected {
                ids::hv::CONN_MENU_DISCONNECT
            } else {
                ids::hv::CONN_MENU_CONNECT
            }),
            "connection.toggle_connect",
        ),
        item(text_fn(ids::hv::CONN_MENU_EDIT), "connection.edit"),
        item(text_fn(ids::hv::CONN_MENU_DUP), "connection.duplicate"),
        item(text_fn(ids::hv::CONN_MENU_COPY_URL), "connection.copy_url"),
        MenuItemSpec::separator(),
        // ## m2a-t21-shots: "Delete… in red" (NOTES §8).
        item(text_fn(ids::hv::CONN_MENU_DELETE), "connection.delete").danger(true),
    ]
}

/// Opens the connection menu of the selection at `anchor` (or under the selected row).
pub fn open_context_menu(anchor: Option<Point<Pixels>>, window: &mut Window, cx: &mut App) {
    let Some(id) = cx
        .try_global::<ConnectionsModel>()
        .and_then(|m| m.selected())
    else {
        return;
    };
    let anchor = anchor
        .or_else(|| {
            let view = explorer(cx)?;
            let bounds = view.read(cx).row_bounds.borrow().get(&id).copied()?;
            Some(bounds.bottom_left())
        })
        .unwrap_or_default();
    let items = menu_items(cx, id);
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    ws.update(cx, |ws, cx| {
        ws.open_popup_menu(anchor, items, window, cx);
    });
}

impl ExplorerView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| TextInput::new(cx, text_fn(ids::hv::EXPLORER_FILTER)));
        let list_focus = cx.focus_handle().tab_stop(true);
        let subscriptions = vec![
            cx.subscribe_in(
                &filter,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Changed => {
                        this.keep_selection_visible(cx);
                        cx.notify();
                    }
                    // P16: Escape in the field is the shell's Escape (cancel a connect).
                    InputEvent::Cancelled => {
                        let escape = cx.global::<ShellHooks>().escape.clone();
                        if let Some(escape) = escape {
                            escape(window, cx);
                        }
                    }
                    InputEvent::Submitted => {
                        let handle = this.list_focus.clone();
                        window.focus(&handle, cx);
                    }
                    _ => {}
                },
            ),
            cx.observe_global::<ConnectionsModel>(|_, cx| cx.notify()),
        ];
        ExplorerView {
            filter,
            list_focus,
            row_bounds: Rc::default(),
            _subscriptions: subscriptions,
        }
    }

    /// A selection the filter hides moves to the first visible row, or is cleared when no
    /// row is visible: the `Explorer` commands never act on a hidden row.
    fn keep_selection_visible(&self, cx: &mut Context<Self>) {
        let ids = self.visible_ids(cx);
        let selected = cx.global::<ConnectionsModel>().selected();
        if selected.is_some_and(|s| !ids.contains(&s)) {
            ConnectionsModel::select(cx, ids.first().copied());
        }
    }

    fn filter_focus(&self, cx: &App) -> FocusHandle {
        use hacivat_ui_core::gpui_surface::Focusable as _;
        self.filter.focus_handle(cx)
    }

    fn query(&self, cx: &App) -> String {
        self.filter.read(cx).value().to_string()
    }

    /// The visible profiles, top down.
    fn visible_ids(&self, cx: &App) -> Vec<ProfileId> {
        visible_groups(cx, &self.query(cx))
            .into_iter()
            .flat_map(|(_, rows)| rows.into_iter().map(|p| p.id))
            .collect()
    }

    /// ↑/↓ move the selection among the visible rows.
    fn on_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        if stroke.modifiers.modified() {
            return;
        }
        let forward = match stroke.key.as_str() {
            "down" => true,
            "up" => false,
            _ => return,
        };
        let ids = self.visible_ids(cx);
        if ids.is_empty() {
            return;
        }
        let selected = cx.global::<ConnectionsModel>().selected();
        let at = selected.and_then(|s| ids.iter().position(|id| *id == s));
        let next = match (at, forward) {
            (None, true) => 0,
            (None, false) => ids.len() - 1,
            (Some(i), true) => (i + 1).min(ids.len() - 1),
            (Some(i), false) => i.saturating_sub(1),
        };
        ConnectionsModel::select(cx, Some(ids[next]));
        cx.stop_propagation();
    }

    fn filter_row(&self, theme: &Theme, cx: &App) -> AnyElement {
        let t = theme.tokens;
        let c = &t.colors;
        let handle = self.filter_focus(cx).tab_stop(true);
        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .flex_none()
            .mx(px(t.spacing.md))
            .mt(px(t.spacing.md))
            .mb(px(t.spacing.xs))
            .gap(px(t.spacing.sm))
            .child(
                Icon::new(IconName::Search)
                    .size(IconSize::Chrome)
                    .color(c.text_3.to_hsla()),
            )
            .child(
                // A tab stop for the field (the input's own handle is not one).
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .track_focus(&handle)
                    .child(self.filter.clone()),
            );
        if theme.mode == ThemeMode::Light {
            row = row.bg(c.panel.to_hsla());
        }
        row.into_any_element()
    }

    fn empty(&self, theme: &Theme, cx: &mut App) -> AnyElement {
        let t = theme.tokens;
        let c = &t.colors;
        let button =
            |id: &'static str, label: SharedString, kind, action: &'static str, cx: &App| {
                let mut b = Button::new(id, label)
                    .kind(kind)
                    .on_click(move |_, window, cx| {
                        ActionRegistry::run(action, window, cx);
                    });
                if let Some(keys) = shortcut_label(cx, action) {
                    b = b.key_hint(keys.into());
                }
                b
            };
        let body = paragraph(
            tr(cx, ids::hv::EXPLORER_EMPTY_BODY, NoArgs),
            t.type_ramp.body_sm,
            c.text_2.to_hsla(),
            "explorer-empty-body",
        );
        div()
            .flex()
            .flex_col()
            .flex_1()
            .justify_center()
            .px(px(t.spacing.s_3xl))
            .gap(px(t.spacing.lg))
            .child(
                Label::new(
                    tr(cx, ids::hv::EXPLORER_EMPTY, NoArgs),
                    TextSource::Fluent,
                    TypeStyle {
                        weight: 600,
                        ..t.type_ramp.body
                    },
                    c.text.to_hsla(),
                )
                .id("explorer-empty"),
            )
            .child(body)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(t.spacing.sm))
                    .child(button(
                        "explorer-new",
                        tr(cx, ids::hv::EXPLORER_NEW, NoArgs),
                        ButtonKind::Primary,
                        "connection.new",
                        cx,
                    ))
                    .child(button(
                        "explorer-paste",
                        tr(cx, ids::hv::EXPLORER_PASTE, NoArgs),
                        ButtonKind::Default,
                        "connection.paste_url",
                        cx,
                    ))
                    .child(button(
                        "explorer-import",
                        tr(cx, ids::hv::EXPLORER_IMPORT, NoArgs),
                        ButtonKind::Ghost,
                        "connection.import_pg_service",
                        cx,
                    )),
            )
            .into_any_element()
    }

    fn row(
        &self,
        index: usize,
        p: &Profile,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = theme.tokens;
        let c = &t.colors;
        let m = cx.global::<ConnectionsModel>();
        let selected = m.selected() == Some(p.id);
        let live = m.live(p.id);
        let id = p.id;
        let production = p.environment == Some(EnvTag::Production);
        let glyph = if p.engine == DbmsKind::Sqlite {
            IconName::FileCode
        } else {
            IconName::Database
        };
        let info = match &live {
            LiveState::Connected(info) => Some(info.clone()),
            _ => None,
        };
        let mut row = div()
            .id(("explorer-row", index))
            .debug_selector(move || format!("explorer-row-{index}"))
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .flex_none()
            .h(px(ROW_PX))
            .pl(px(t.spacing.lg))
            .pr(px(t.spacing.md))
            .gap(px(t.spacing.sm))
            .border_l_1()
            .border_color(if selected { c.blue } else { c.side }.to_hsla())
            .child(bounds_probe(self.row_bounds.clone(), id))
            .child(
                Icon::new(glyph)
                    .size(IconSize::Chrome)
                    .color(c.text_2.to_hsla()),
            )
            .child(
                // ## m2a-t21-shots: the name gives way last (the meta ellipsizes first)
                // and never below about six characters.
                div()
                    .min_w(name_min_width(&p.name, t.type_ramp.body, window))
                    .flex_shrink(NAME_SHRINK)
                    .child(
                        Label::new(
                            p.name.clone(),
                            TextSource::Data,
                            t.type_ramp.body,
                            c.text.to_hsla(),
                        )
                        .truncate(true)
                        .id(SharedString::from(format!("explorer-row-name-{index}"))),
                    ),
            )
            .child(env_swatch(env_kind(p.environment), theme));
        if selected {
            row = row.bg(c.blue_tint.to_hsla());
        } else {
            row = row.hover(|s| s.bg(c.hover.to_hsla()));
        }
        // Flags: read-only (red only on production, DESIGN.md), plaintext while live.
        if p.read_only {
            let ink = if production { c.red } else { c.text_2 };
            row = row.child(
                Icon::new(IconName::Lock)
                    .size(IconSize::Chrome)
                    .color(ink.to_hsla()),
            );
        }
        if let Some(info) = &info
            && is_unencrypted(p, info)
        {
            row = row.child(
                Icon::new(IconName::LockOpen)
                    .size(IconSize::Chrome)
                    .color(c.text_2.to_hsla()),
            );
        }
        row = row.child(div().flex_1());
        // ## m2a-t21-shots: the meta (live mark, latency, engine) shrinks first: its labels
        // ellipsize (tooltip: the whole text) before the name does.
        let mut meta = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(t.spacing.sm))
            .min_w(px(0.0))
            .flex_shrink(1.0);
        match &live {
            LiveState::Connected(info) => {
                meta = meta
                    .child(
                        div().flex_none().child(
                            Icon::new(IconName::Check)
                                .size(IconSize::Chrome)
                                .color(c.green.to_hsla()),
                        ),
                    )
                    .child(
                        Label::new(
                            latency_label(cx, info),
                            TextSource::Fluent,
                            t.type_ramp.mono_data,
                            c.text_2.to_hsla(),
                        )
                        .truncate(true)
                        .id(SharedString::from(format!("explorer-row-latency-{index}"))),
                    );
            }
            // ## m2b-t01-interfaces: a reconnect runs like a connect (Task 22 refines it).
            LiveState::Connecting(_) | LiveState::Reconnecting { .. } => {
                meta = meta.child(div().flex_none().child(run_marker(theme)));
            }
            LiveState::Failed(_) | LiveState::Lost { .. } => {
                meta = meta.child(
                    div().flex_none().child(
                        Icon::new(IconName::TriangleAlert)
                            .size(IconSize::Chrome)
                            .color(c.text.to_hsla()),
                    ),
                );
            }
            LiveState::Disconnected => {}
        }
        meta = meta.child(
            Label::new(
                engine_label(p, info.as_deref()),
                TextSource::Data,
                t.type_ramp.caption,
                c.text_2.to_hsla(),
            )
            .truncate(true)
            .id(SharedString::from(format!("explorer-row-engine-{index}"))),
        );
        row = row.child(meta);
        let list = self.list_focus.clone();
        let list_right = list.clone();
        row.on_mouse_down(
            MouseButton::Left,
            cx.listener(move |_, event: &MouseDownEvent, window, cx| {
                ConnectionsModel::select(cx, Some(id));
                window.focus(&list, cx);
                if event.click_count == 2 {
                    ActionRegistry::run("connection.toggle_connect", window, cx);
                }
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |_, event: &MouseDownEvent, window, cx| {
                ConnectionsModel::select(cx, Some(id));
                window.focus(&list_right, cx);
                let position = event.position;
                window.defer(cx, move |window, cx| {
                    open_context_menu(Some(position), window, cx);
                });
            }),
        )
        .into_any_element()
    }

    fn list(&self, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = theme.tokens;
        let c = &t.colors;
        let mut list = div()
            .id("explorer-list")
            .debug_selector(|| "explorer-list".to_string())
            .track_focus(&self.list_focus)
            .on_key_down(cx.listener(Self::on_key))
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .pb(px(t.spacing.md))
            // ## m2a-t21-shots: the frame checks treat this clip as a scroll viewport.
            .child(hacivat_ui::text::scroll_viewport());
        let mut index = 0;
        for (g, (tag, rows)) in visible_groups(cx, &self.query(cx)).into_iter().enumerate() {
            let header = div()
                .flex()
                .flex_row()
                .items_center()
                .flex_none()
                .h(px(GROUP_PX))
                .px(px(t.spacing.lg))
                .gap(px(t.spacing.sm))
                .child(
                    Label::new(
                        group_title(cx, tag),
                        TextSource::Fluent,
                        t.type_ramp.panel_title,
                        c.text_2.to_hsla(),
                    )
                    .id(SharedString::from(format!("explorer-group-{g}"))),
                )
                .child(div().flex_1().h(px(1.0)).bg(c.seam.to_hsla()))
                .child(
                    Label::new(
                        rows.len().to_string(),
                        TextSource::Data,
                        t.type_ramp.caption,
                        c.text_2.to_hsla(),
                    )
                    .id(SharedString::from(format!("explorer-group-count-{g}"))),
                );
            list = list.child(header);
            for p in rows {
                let row = self.row(index, &p, theme, window, cx);
                list = list.child(row);
                index += 1;
            }
        }
        list.into_any_element()
    }
}

impl Render for ExplorerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let Some(m) = cx.try_global::<ConnectionsModel>() else {
            // P17: no model, the M1 explorer.
            return tiles::explorer(&theme, cx).into_any_element();
        };
        // The focused list with nothing selected selects its first row (after this frame), so
        // Enter and the other `Explorer` bindings have a target.
        if self.list_focus.is_focused(window)
            && m.selected().is_none()
            && let Some(first) = self.visible_ids(cx).first().copied()
        {
            window.defer(cx, move |_, cx| ConnectionsModel::select(cx, Some(first)));
        }
        let m = cx.global::<ConnectionsModel>();
        let empty = m.profiles().is_empty();
        let c = &theme.tokens.colors;
        let body = if empty {
            self.empty(&theme, cx)
        } else {
            self.list(&theme, window, cx)
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(c.side.to_hsla())
            .child(panel_header(&theme).child(panel_title(
                &theme,
                tr(cx, ids::hv::PANEL_EXPLORER, NoArgs),
                "panel-explorer-title",
            )))
            .child(self.filter_row(&theme, cx))
            .child(body)
            .into_any_element()
    }
}

// ## m2a-t18-explorer: shots fixtures (R28, P19)

#[cfg(feature = "shots")]
pub use fixtures::*;

#[cfg(feature = "shots")]
mod fixtures {
    use std::sync::Arc;

    use hacivat_driver::Support;
    use hacivat_proto::{DbmsKind, EnvTag, ErrClass};
    use hacivat_session::LiveState;
    use hacivat_session::pipeline::{Progress, TrailStep};
    use hacivat_session::profile::{Profile, ProfileDraft, ProfileEndpoint};
    use hacivat_ui_core::gpui_surface::{App, Window};

    use crate::connections::capsule::sample_live_info;
    use crate::connections::error_view::sample_error;
    use crate::connections::model::ConnectionsModel;

    fn add(cx: &mut App, name: &str, engine: DbmsKind, env: Option<EnvTag>, read_only: bool) {
        let endpoint = if engine == DbmsKind::Sqlite {
            ProfileEndpoint::File {
                path: "/Users/demo/app.db".into(),
                create_if_missing: false,
            }
        } else {
            ProfileEndpoint::Server {
                host: format!("{name}.db.internal"),
                port: None,
            }
        };
        let profile = Profile {
            name: name.to_string(),
            engine,
            environment: env,
            read_only,
            endpoint,
            ..Profile::default()
        };
        ConnectionsModel::save(
            cx,
            ProfileDraft {
                is_new: true,
                ..ProfileDraft::edit(profile)
            },
        )
        .detach();
    }

    /// Shows `name` in `state` (the mirror's value; nothing connects).
    fn live(cx: &mut App, name: &str, state: LiveState) {
        let id = cx
            .global::<ConnectionsModel>()
            .profiles()
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.id);
        if let Some(id) = id {
            cx.global_mut::<ConnectionsModel>().set_live(id, state);
        }
    }

    /// The explorer with no connection (NOTES §8 empty state).
    pub fn empty_fixture(_window: &mut Window, _cx: &mut App) {}

    /// A list over every group, one row selected.
    pub fn list_fixture(window: &mut Window, cx: &mut App) {
        add(
            cx,
            "orders-prod",
            DbmsKind::Postgres,
            Some(EnvTag::Production),
            true,
        );
        add(cx, "billing", DbmsKind::MySql, Some(EnvTag::Staging), false);
        add(
            cx,
            "orders-test",
            DbmsKind::MariaDb,
            Some(EnvTag::Test),
            false,
        );
        add(cx, "scratch", DbmsKind::Postgres, Some(EnvTag::Dev), false);
        add(cx, "app.db", DbmsKind::Sqlite, Some(EnvTag::Local), false);
        add(cx, "legacy", DbmsKind::Postgres, None, false);
        // ## m2a-t21-shots: every live mark (NOTES §8): connected with latency (and the
        // plaintext flag), connecting (the run marker), failed (the alert).
        live(
            cx,
            "billing",
            LiveState::Connected(Arc::new(sample_live_info(false, false, Support::Supported))),
        );
        live(
            cx,
            "orders-test",
            LiveState::Connecting(Progress {
                trail: Vec::new(),
                current: Some(TrailStep::SignIn),
                started: std::time::Instant::now(),
            }),
        );
        live(
            cx,
            "scratch",
            LiveState::Failed(Arc::new(sample_error(
                ErrClass::NetworkRefused,
                TrailStep::Connect,
                None,
                "",
            ))),
        );
        let first = cx
            .global::<ConnectionsModel>()
            .profiles()
            .first()
            .map(|p| p.id);
        ConnectionsModel::select(cx, first);
        super::focus_list(window, cx);
    }

    /// The list with the connection menu open on the selection.
    pub fn menu_fixture(window: &mut Window, cx: &mut App) {
        list_fixture(window, cx);
        super::open_context_menu(None, window, cx);
    }
}
