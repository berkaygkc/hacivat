//! The range chooser (Task 33): a popover listing the candidate ranges when the execution range
//! is ambiguous.
//!
//! - **D-011 (R-M3-36).** `Ask` opens a `PopupMenu` (460 px) under `EditorView::caret_bounds()`,
//!   the candidates in resolve order: the kind, its count of statements when it has several,
//!   and the whitespace-collapsed SQL as data. The highlight starts on the candidate Run
//!   preselects (the one the preview band shows); the highlighted row is previewed in the band
//!   (`set_preview_override`), so hover and ↑/↓ show exactly what Enter would run. Enter (or a
//!   click) runs the **highlighted** row: `units_for_candidate`, then one run. Escape (or a
//!   click outside, or an edit) closes it and sends nothing. The chosen kind is remembered per
//!   console for the session and preselected next time (the language service is told, so the
//!   preview follows).
//! - **No access keys (P15).** The labels are SQL text; typing a letter never runs anything.
//! - **Boundary uncertain (R-M3-24).** An uncertain resolution opens the chooser with no run
//!   item at all: the title, why nothing was sent, the reason, and what to do. Nothing in it
//!   can send anything; Escape closes it.

use std::rc::Rc;

use hacivat_l10n::ArgValue;
use hacivat_ui::menu::{MenuItemSpec, PopupMenu, PopupMenuEvent};
use hacivat_ui::text::{TextFn, TextSource};
use hacivat_ui_core::gpui_surface::{
    App, AppContext as _, Bounds, Context, Entity, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, Pixels, Point, Render, SharedString, Styled as _, Window,
    div, point, px,
};
use smallvec::SmallVec;
use sql_split::DocSnapshot;
use sql_split::exec_range::{Candidate, CandidateKind};

use crate::console::{ConsoleId, Consoles};
use crate::exec_ui::marks::message;
use crate::exec_ui::{console_exec, orchestrator, update_console_exec, workspace_window};
use crate::shell::layers::MENU_CONTEXT;
use crate::shell::{Layer, Workspace};

/// The chooser's layer ID.
pub const CHOOSER_LAYER: &str = "chooser";
/// The chooser's width (NOTES surface 4).
const CHOOSER_WIDTH_PX: f32 = 460.0;
/// The action ID prefix of a candidate row (`chooser.<index>`).
const ACTION_PREFIX: &str = "chooser.";
// ## m3a-f2-visual: the chooser's key-hint footer (the palette's words; NOTES surface 4).
const CHOOSER_KEYS: [(&str, &str); 3] = [
    ("up down", "hui-palette-hint-navigate"),
    ("enter", "hui-palette-hint-run"),
    ("escape", "hui-palette-hint-close"),
];

fn kind_id(kind: CandidateKind) -> &'static str {
    match kind {
        CandidateKind::Subquery => "hv-chooser-subquery",
        CandidateKind::Statement => "hv-chooser-statement",
        CandidateKind::LargestOrBatch => "hv-chooser-largest",
        CandidateKind::FromCaret => "hv-chooser-from-caret",
        CandidateKind::WholeScript => "hv-chooser-whole",
    }
}

/// Where the chooser opens: under the caret, else near the editor's top left.
fn anchor_of(caret: Option<Bounds<Pixels>>) -> Point<Pixels> {
    match caret {
        Some(b) => point(b.left(), b.bottom() + px(2.0)),
        None => point(px(80.0), px(80.0)),
    }
}

/// The ranges of `c`, for the band's override.
fn ranges_of(c: &Candidate) -> SmallVec<[std::ops::Range<usize>; 2]> {
    c.units
        .iter()
        .map(|u| usize::from(u.range.start())..usize::from(u.range.end()))
        .collect()
}

/// Opens the chooser over `candidates` (resolved at editor `version` on `doc`).
pub(crate) fn open(
    console: ConsoleId,
    version: u64,
    doc: DocSnapshot,
    candidates: Vec<Candidate>,
    preselect: usize,
    cx: &mut App,
) {
    let Some(exec) = console_exec(cx, console) else {
        return;
    };
    let Some(editor) = exec.editor.upgrade() else {
        return;
    };
    let remembered = exec.remembered;
    let preselect = preselect.min(candidates.len().saturating_sub(1));
    // The check marks the remembered kind only (mock); the highlight is set to Run's
    // preselected row after the menu is built (Fix round 1, Minor 13).
    let items: Vec<MenuItemSpec> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let kind = kind_id(c.kind);
            let count = c.units.len();
            // One Fluent message per row (Fix round 1, Minor 13): the kind, and its count of
            // statements when it has several.
            let label: TextFn = Rc::new(move |cx: &App| {
                let kind = message(cx, kind, vec![]).to_string();
                message(
                    cx,
                    "hv-chooser-row",
                    vec![
                        ("kind", ArgValue::Text(kind)),
                        ("n", ArgValue::Count(count as u64)),
                    ],
                )
            });
            let sql = SharedString::from(c.label.clone());
            MenuItemSpec::new(label, format!("{ACTION_PREFIX}{i}"))
                .hint(Rc::new(move |_: &App| sql.clone()))
                .hint_source(TextSource::Data)
                .checked(remembered.is_some() && Some(c.kind) == remembered)
        })
        .collect();
    let caret = editor.read(cx).caret_bounds();
    let candidates = Rc::new(candidates);
    let Some(window) = workspace_window(cx) else {
        return;
    };
    let _ = window.update(cx, |_, window, cx| {
        let Some(ws) = window.root::<Workspace>().flatten() else {
            return;
        };
        // Taken before the menu takes the focus, so closing it gives the focus back to the
        // editor (Fix round 1, I4).
        let restore = window.focused(cx);
        let menu = PopupMenu::new(anchor_of(caret), items, window, cx);
        menu.update(cx, |m, _| {
            m.set_access_keys(false);
            m.set_min_width(px(CHOOSER_WIDTH_PX));
            // ## m3a-t35-shots (named exception): 460 px (NOTES surface 4); the SQL label
            // ellipsizes inside it, never past the window.
            m.set_max_width(px(CHOOSER_WIDTH_PX));
            m.title(crate::l10n_global::text_fn_id("hv-chooser-title"));
            // ## m3a-f2-visual (named exception): the active row reads as a selection and the
            // footer lists the keys (NOTES surface 4).
            m.set_active_tint(true);
            m.set_key_hints(CHOOSER_KEYS.to_vec());
            m.select(preselect);
        });
        // The band follows the highlighted row.
        let follow = {
            let (editor, candidates) = (editor.downgrade(), candidates.clone());
            cx.observe(&menu, move |menu, cx| {
                // Only while this menu is the console's open chooser: the notify that follows
                // a choice must not draw over the band once the chooser closed.
                let open = console_exec(cx, console)
                    .and_then(|e| e.chooser.as_ref())
                    .is_some_and(|m| m.entity_id() == menu.entity_id());
                if !open {
                    return;
                }
                let Some(c) = menu.read(cx).selected().and_then(|i| candidates.get(i)) else {
                    return;
                };
                let ranges = ranges_of(c);
                if let Some(editor) = editor.upgrade() {
                    editor.update(cx, |e, cx| e.set_preview_override(Some(ranges), cx));
                }
            })
        };
        let chosen = {
            let (candidates, doc) = (candidates.clone(), doc.clone());
            cx.subscribe(&menu, move |_menu, event: &PopupMenuEvent, cx| {
                let index = match event {
                    PopupMenuEvent::Chosen(action) => action
                        .strip_prefix(ACTION_PREFIX)
                        .and_then(|i| i.parse::<usize>().ok()),
                    PopupMenuEvent::Dismissed => None,
                };
                close(console, cx);
                let Some(c) = index.and_then(|i| candidates.get(i)) else {
                    return;
                };
                remember(console, c.kind, cx);
                let units = hacivat_exec::units_for_candidate(&doc, c);
                orchestrator::start_run(console, version, units, cx);
            })
        };
        let first = menu.read(cx).selected().and_then(|i| candidates.get(i));
        if let Some(c) = first {
            let ranges = ranges_of(c);
            editor.update(cx, |e, cx| e.set_preview_override(Some(ranges), cx));
        }
        push(
            console,
            &ws,
            menu,
            vec![follow, chosen],
            restore,
            window,
            cx,
        );
    });
}

/// Opens the chooser's "boundary uncertain" state: no run item, the reasons (`exec-uncertain-*`
/// IDs) and what to do. Nothing is sent.
pub(crate) fn open_uncertain(console: ConsoleId, reasons: Vec<&'static str>, cx: &mut App) {
    let Some(editor) = console_exec(cx, console).and_then(|e| e.editor.upgrade()) else {
        return;
    };
    let caret = editor.read(cx).caret_bounds();
    let mut items: Vec<MenuItemSpec> = Vec::new();
    let row = |label: TextFn| MenuItemSpec::new(label, "").disabled(true);
    items.push(row(crate::l10n_global::text_fn_id(
        "hv-chooser-uncertain-body",
    )));
    for reason in reasons {
        items.push(row(crate::l10n_global::text_fn_id(reason)));
    }
    items.push(row(Rc::new(|cx: &App| {
        // Without a Run shortcut the sentence names none (Fix round 1, Minor 13).
        match crate::shell::keymap_state::shortcut_label(cx, "editor.run") {
            Some(key) => message(
                cx,
                "hv-chooser-uncertain-fix",
                vec![("key", ArgValue::Text(key))],
            ),
            None => message(cx, "hv-chooser-uncertain-fix-unbound", vec![]),
        }
    })));
    let Some(window) = workspace_window(cx) else {
        return;
    };
    let _ = window.update(cx, |_, window, cx| {
        let Some(ws) = window.root::<Workspace>().flatten() else {
            return;
        };
        let restore = window.focused(cx);
        let menu = PopupMenu::new(anchor_of(caret), items, window, cx);
        menu.update(cx, |m, _| {
            m.set_access_keys(false);
            m.set_min_width(px(CHOOSER_WIDTH_PX));
            m.title(crate::l10n_global::text_fn_id("hv-chooser-uncertain-title"));
            // ## m3a-f2-visual (named exception): only Escape does anything here.
            m.set_key_hints(vec![("escape", "hui-palette-hint-close")]);
        });
        let closed = cx.subscribe(&menu, move |_menu, _: &PopupMenuEvent, cx| {
            close(console, cx)
        });
        push(console, &ws, menu, vec![closed], restore, window, cx);
    });
}

/// Pushes `menu` as the chooser layer; the console keeps it (and its subscriptions) until it
/// closes.
fn push(
    console: ConsoleId,
    ws: &Entity<Workspace>,
    menu: Entity<PopupMenu>,
    subscriptions: Vec<hacivat_ui_core::gpui_surface::Subscription>,
    restore: Option<hacivat_ui_core::gpui_surface::FocusHandle>,
    window: &mut Window,
    cx: &mut App,
) {
    let host = cx.new(|_| ChooserHost { menu: menu.clone() });
    let layer = Layer {
        id: CHOOSER_LAYER,
        context: MENU_CONTEXT,
        view: host.clone().into(),
        restore_focus: restore,
        modal: false,
    };
    ws.update(cx, |ws, cx| ws.push_layer(layer, window, cx));
    update_console_exec(cx, console, |e| {
        e.chooser = Some(menu);
        e.chooser_host = Some(host.into());
        e.chooser_subscriptions = subscriptions;
    });
}

/// The chooser's layer: the menu, under a key filter. Enter runs the highlighted row only on a
/// plain key-down: a held Enter (auto-repeat) or a modified one (the ⌘↵ that asked for Run,
/// still held) never chooses (NOTES surface 4: "↵ (key-down, no repeat)").
struct ChooserHost {
    menu: Entity<PopupMenu>,
}

impl Render for ChooserHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .capture_key_down(|event: &KeyDownEvent, _window, cx| {
                let stroke = &event.keystroke;
                if stroke.key == "enter" && (event.is_held || stroke.modifiers.modified()) {
                    cx.stop_propagation();
                }
            })
            .child(self.menu.clone())
    }
}

/// Closes console `console`'s chooser, if it is open, and clears the band's override. Sends
/// nothing.
pub(crate) fn close(console: ConsoleId, cx: &mut App) {
    let Some((menu, subscriptions)) = update_console_exec(cx, console, |e| {
        (
            e.chooser.take(),
            std::mem::take(&mut e.chooser_subscriptions),
        )
    }) else {
        return;
    };
    let Some(menu) = menu else {
        return;
    };
    let host = update_console_exec(cx, console, |e| e.chooser_host.take()).flatten();
    if let Some(editor) = console_exec(cx, console).and_then(|e| e.editor.upgrade()) {
        editor.update(cx, |e, cx| e.set_preview_override(None, cx));
    }
    let view = host
        .as_ref()
        .map_or_else(|| menu.entity_id(), |h| h.entity_id());
    // The menu and its subscriptions live until this event has reached every subscriber.
    cx.defer(move |cx| {
        if let Some(window) = workspace_window(cx) {
            let _ = window.update(cx, |_, window, cx| {
                if let Some(ws) = window.root::<Workspace>().flatten() {
                    ws.update(cx, |ws, cx| ws.remove_layer(view, window, cx));
                }
            });
        }
        drop((menu, host, subscriptions));
    });
}

/// Remembers `kind` for console `console` (R-M3-36): Run's chooser opens on it next time, and
/// the preview shows that candidate.
fn remember(console: ConsoleId, kind: CandidateKind, cx: &mut App) {
    update_console_exec(cx, console, |e| e.remembered = Some(kind));
    let Some(editor) = console_exec(cx, console).and_then(|e| e.editor.upgrade()) else {
        return;
    };
    if let Some(service) = Consoles::get(cx, console).and_then(|s| s.service.clone()) {
        let doc = editor.read(cx).doc();
        service.set_remembered_kind(doc, Some(kind), cx);
    }
}

/// Whether console `console`'s chooser is open (tests).
pub fn is_open(console: ConsoleId, cx: &App) -> bool {
    console_exec(cx, console).is_some_and(|e| e.chooser.is_some())
}

/// The open chooser of console `console` (tests read its highlighted row).
pub fn menu_of(console: ConsoleId, cx: &App) -> Option<Entity<PopupMenu>> {
    console_exec(cx, console).and_then(|e| e.chooser.clone())
}
