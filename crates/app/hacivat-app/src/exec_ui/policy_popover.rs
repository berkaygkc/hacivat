//! The run-policy popover (CHOOSE-06; M3b Task 24, `design/m3b-safe` surface 9): how Run picks
//! statements in this console, opened by the chooser's "Customize…".
//!
//! ## m3b-t24-exec-ui
//!
//! - **What it sets.** "Caret inside a statement" (Ask / Smallest statement / Largest statement
//!   / Whole script) for Run, and "With a selection" (As separate statements / As one) for
//!   every Run: one [`RunPolicy`] per console, kept until the console closes (never saved).
//!   Run and its band read the same policy (`orchestrator::set_policy` tells the language
//!   service), so the preview still shows exactly what Run sends.
//! - **Anatomy.** The explanation popover's: a titled menu at the caret ("How Run picks
//!   statements in this console"), two groups of radio rows (the chosen one checked), the note
//!   "Kept for this console until it closes." A choice applies at once and the popover stays
//!   open on it; Escape (or a click outside) closes it and the focus goes back to the editor.
//!   No access keys (as the chooser).
//! - **No toolbar item** ([`render`] answers `None`, as the M3a toolbar stays): the policy is
//!   reached from the chooser (Task 1b's seam named a toolbar item; the plan's surface 9 puts it
//!   behind "Customize…").

use hacivat_sql_editor::{InsideChoice, RunPolicy};
use hacivat_ui::menu::{MenuItemSpec, PopupMenu, PopupMenuEvent};

use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, App, Entity, Global, Subscription, point, px,
};

use crate::console::ConsoleId;
use crate::exec_ui::orchestrator::{policy_of, set_policy};
use crate::exec_ui::{console_exec, workspace_window};
use crate::shell::layers::MENU_CONTEXT;
use crate::shell::{Layer, Workspace};

/// The popover's layer ID.
pub const POLICY_LAYER: &str = "policy-popover";
/// Its width (NOTES surface 9).
const WIDTH_PX: f32 = 300.0;
/// The widest it grows to, so its title is never cut (a longer locale).
const MAX_WIDTH_PX: f32 = 380.0;

/// The popover's toolbar item for console `console`: none (module docs).
pub fn render(console: ConsoleId, cx: &App) -> Option<AnyElement> {
    let _ = (console, cx);
    None
}

const INSIDE: [(InsideChoice, &str, &str); 4] = [
    (
        InsideChoice::Ask,
        "hv-policy-inside-ask",
        "policy.inside.ask",
    ),
    (
        InsideChoice::Smallest,
        "hv-policy-inside-smallest",
        "policy.inside.smallest",
    ),
    (
        InsideChoice::Largest,
        "hv-policy-inside-largest",
        "policy.inside.largest",
    ),
    (
        InsideChoice::Whole,
        "hv-policy-inside-whole",
        "policy.inside.whole",
    ),
];

const SELECTION: [(bool, &str, &str); 2] = [
    (false, "hv-policy-sel-separate", "policy.selection.separate"),
    (true, "hv-policy-sel-as-one", "policy.selection.as-one"),
];

/// The popover's rows for `policy`: two titled groups of radio rows, then the note.
pub(crate) fn rows(policy: RunPolicy) -> Vec<MenuItemSpec> {
    let text = crate::l10n_global::text_fn_id;
    let head = |id: &'static str| MenuItemSpec::new(text(id), "").disabled(true);
    let mut items = vec![head("hv-policy-inside")];
    for (choice, id, action) in INSIDE {
        items.push(MenuItemSpec::new(text(id), action).checked(policy.inside == choice));
    }
    items.push(MenuItemSpec::separator());
    items.push(head("hv-policy-selection"));
    for (as_one, id, action) in SELECTION {
        items.push(MenuItemSpec::new(text(id), action).checked(policy.selection_as_one == as_one));
    }
    items.push(MenuItemSpec::separator());
    items.push(head("hv-policy-note"));
    items
}

/// `policy` with the row of `action` chosen; `None` for an action of no row.
pub fn apply(policy: RunPolicy, action: &str) -> Option<RunPolicy> {
    if let Some((choice, _, _)) = INSIDE.iter().find(|(_, _, a)| *a == action) {
        return Some(RunPolicy {
            inside: *choice,
            ..policy
        });
    }
    SELECTION
        .iter()
        .find(|(_, _, a)| *a == action)
        .map(|(as_one, _, _)| RunPolicy {
            selection_as_one: *as_one,
            ..policy
        })
}

/// The open popover, if any: its console, menu, layer view and subscriptions.
#[derive(Default)]
struct OpenPolicy(Option<(ConsoleId, Entity<PopupMenu>, AnyView, Vec<Subscription>)>);

impl Global for OpenPolicy {}

/// Whether console `console`'s policy popover is open (tests).
pub fn is_open(console: ConsoleId, cx: &App) -> bool {
    cx.try_global::<OpenPolicy>()
        .and_then(|o| o.0.as_ref())
        .is_some_and(|(c, ..)| *c == console)
}

/// The open popover's menu (tests choose its rows).
pub fn menu(cx: &App) -> Option<Entity<PopupMenu>> {
    cx.try_global::<OpenPolicy>()
        .and_then(|o| o.0.as_ref())
        .map(|(_, m, ..)| m.clone())
}

/// Opens console `console`'s policy popover at its caret (module docs), highlighting `select`
/// (the row just chosen when it reopens on a choice).
pub fn open(console: ConsoleId, cx: &mut App) {
    open_at(console, None, cx);
}

fn open_at(console: ConsoleId, select: Option<usize>, cx: &mut App) {
    close(cx);
    let Some(editor) = console_exec(cx, console).and_then(|e| e.editor.upgrade()) else {
        return;
    };
    let anchor = editor
        .read(cx)
        .caret_bounds()
        .map_or(point(px(80.0), px(80.0)), |b| {
            point(b.left(), b.bottom() + px(2.0))
        });
    let items = rows(policy_of(console, cx));
    let Some(window) = workspace_window(cx) else {
        return;
    };
    let _ = window.update(cx, |_, window, cx| {
        let Some(ws) = window.root::<Workspace>().flatten() else {
            return;
        };
        let restore = window.focused(cx);
        let menu = PopupMenu::new(anchor, items, window, cx);
        menu.update(cx, |m, _| {
            m.set_access_keys(false);
            m.set_min_width(px(WIDTH_PX));
            m.set_max_width(px(MAX_WIDTH_PX));
            m.title(crate::l10n_global::text_fn_id("hv-policy-title"));
            m.set_key_hints(vec![
                ("up down", "hui-palette-hint-navigate"),
                ("escape", "hui-palette-hint-close"),
            ]);
            if let Some(i) = select {
                m.select(i);
            }
        });
        let chosen = cx.subscribe(&menu, move |menu, event: &PopupMenuEvent, cx| {
            let at = menu.read(cx).selected();
            match event {
                PopupMenuEvent::Chosen(action) => {
                    let current = policy_of(console, cx);
                    if let Some(next) = apply(current, action) {
                        set_policy(console, next, cx);
                    }
                    // It stays open on the choice (a radio list).
                    let reopen = at;
                    cx.defer(move |cx| open_at(console, reopen, cx));
                }
                PopupMenuEvent::Dismissed => close(cx),
            }
        });
        let view: AnyView = menu.clone().into();
        ws.update(cx, |ws, cx| {
            ws.push_layer(
                Layer {
                    id: POLICY_LAYER,
                    context: MENU_CONTEXT,
                    view: view.clone(),
                    restore_focus: restore,
                    modal: false,
                },
                window,
                cx,
            );
        });
        cx.default_global::<OpenPolicy>().0 = Some((console, menu, view, vec![chosen]));
    });
}

/// Closes the open popover, if any. Its layer gives the focus back.
pub fn close(cx: &mut App) {
    let Some((_, menu, view, subscriptions)) = cx.default_global::<OpenPolicy>().0.take() else {
        return;
    };
    let id = view.entity_id();
    cx.defer(move |cx| {
        if let Some(window) = workspace_window(cx) {
            let _ = window.update(cx, |_, window, cx| {
                if let Some(ws) = window.root::<Workspace>().flatten() {
                    ws.update(cx, |ws, cx| ws.remove_layer(id, window, cx));
                }
            });
        }
        drop((menu, view, subscriptions));
    });
}

// ---- shots fixtures (design/m3b-safe surface 9) -------------------------------------------------

#[cfg(feature = "shots")]
pub mod fixtures {
    //! The chooser additions and the policy popover for the shots harness (Task 24): drawn
    //! from the same rows the console opens, at a fixed place.

    use std::rc::Rc;

    use hacivat_sql_editor::{InsideChoice, RunPolicy};
    use hacivat_ui::menu::{MenuItemSpec, PopupMenu};
    use hacivat_ui::text::{TextFn, TextSource};
    use hacivat_ui_core::gpui_surface::{AnyView, App, SharedString, Window, point, px};

    use crate::exec_ui::chooser;
    use crate::shell::layers::MENU_CONTEXT;
    use crate::shell::{Layer, Workspace};

    /// One state (NOTES surface 9).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum State {
        /// The uncertain state with "Run as one" (not highlighted).
        Uncertain,
        /// "Run as one" highlighted.
        Highlighted,
        /// On a read-only session: disabled with its reason.
        ReadOnly,
        /// The chooser with "Customize…" highlighted.
        Customize,
        /// The policy popover, Largest statement and As one chosen.
        Popover,
    }

    pub const STATES: [State; 5] = [
        State::Uncertain,
        State::Highlighted,
        State::ReadOnly,
        State::Customize,
        State::Popover,
    ];

    impl State {
        pub fn name(self) -> &'static str {
            match self {
                State::Uncertain => "chooser-as-one",
                State::Highlighted => "chooser-as-one-highlighted",
                State::ReadOnly => "chooser-as-one-read-only",
                State::Customize => "chooser-customize",
                State::Popover => "policy-popover",
            }
        }
    }

    fn row(kind: &'static str, n: u64, sql: &'static str, action: &'static str) -> MenuItemSpec {
        let text: TextFn = Rc::new(move |cx: &App| {
            let kind = crate::exec_ui::marks::message(cx, kind, vec![]).to_string();
            crate::exec_ui::marks::message(
                cx,
                "hv-chooser-row",
                vec![
                    ("kind", hacivat_l10n::ArgValue::Text(kind)),
                    ("n", hacivat_l10n::ArgValue::Count(n)),
                ],
            )
        });
        MenuItemSpec::new(text, action)
            .hint(Rc::new(move |_: &App| SharedString::from(sql)))
            .hint_source(TextSource::Data)
    }

    /// Opens `state` in `window`'s workspace.
    pub fn open(state: State, window: &mut Window, cx: &mut App) {
        let Some(ws) = window.root::<Workspace>().flatten() else {
            return;
        };
        let reasons = ["exec-uncertain-unknown-block"];
        let (items, title, select, keys): (Vec<MenuItemSpec>, &'static str, Option<usize>, _) =
            match state {
                State::Uncertain | State::Highlighted | State::ReadOnly => {
                    let ro = state == State::ReadOnly;
                    let items = chooser::uncertain_rows(&reasons, Some((2, ro)));
                    let select = (state == State::Highlighted).then_some(reasons.len() + 3);
                    let keys = if ro {
                        vec![("escape", "hui-palette-hint-close")]
                    } else {
                        vec![
                            ("up down", "hui-palette-hint-navigate"),
                            ("enter", "hui-palette-hint-run"),
                            ("escape", "hui-palette-hint-close"),
                        ]
                    };
                    (items, "hv-chooser-uncertain-title", select, keys)
                }
                State::Customize => {
                    let mut items = vec![
                        row(
                            "hv-chooser-statement",
                            1,
                            "SELECT id FROM musteriler WHERE durum = 'aktif'",
                            "c.0",
                        ),
                        row(
                            "hv-chooser-whole",
                            3,
                            "SELECT id FROM musteriler WHERE durum = 'aktif'; UPDATE …",
                            "c.1",
                        ),
                    ];
                    items.extend(chooser::customize_rows());
                    let n = items.len() - 1;
                    (
                        items,
                        "hv-chooser-title",
                        Some(n),
                        vec![
                            ("up down", "hui-palette-hint-navigate"),
                            ("enter", "hui-palette-hint-run"),
                            ("escape", "hui-palette-hint-close"),
                        ],
                    )
                }
                State::Popover => {
                    let policy = RunPolicy {
                        inside: InsideChoice::Largest,
                        selection_as_one: true,
                    };
                    (
                        super::rows(policy),
                        "hv-policy-title",
                        None,
                        vec![
                            ("up down", "hui-palette-hint-navigate"),
                            ("escape", "hui-palette-hint-close"),
                        ],
                    )
                }
            };
        let (min, max) = if state == State::Popover {
            (super::WIDTH_PX, super::MAX_WIDTH_PX)
        } else {
            (460.0, 460.0)
        };
        let menu = PopupMenu::new(point(px(120.0), px(140.0)), items, window, cx);
        menu.update(cx, |m, _| {
            m.set_access_keys(false);
            m.set_min_width(px(min));
            m.set_max_width(px(max));
            m.title(crate::l10n_global::text_fn_id(title));
            m.set_key_hints(keys);
            if state != State::Popover {
                m.set_active_tint(true);
            }
            match select {
                Some(i) => m.select(i),
                None if state != State::Popover => m.clear_selection(),
                None => {}
            }
        });
        let view: AnyView = menu.clone().into();
        ws.update(cx, |ws, cx| {
            ws.push_layer(
                Layer {
                    id: super::POLICY_LAYER,
                    context: MENU_CONTEXT,
                    view,
                    restore_focus: None,
                    modal: false,
                },
                window,
                cx,
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_sets_its_choice_and_nothing_else() {
        let base = RunPolicy::default();
        let p = apply(base, "policy.inside.whole").unwrap();
        assert_eq!(p.inside, InsideChoice::Whole);
        assert!(!p.selection_as_one);
        let p = apply(p, "policy.selection.as-one").unwrap();
        assert_eq!(p.inside, InsideChoice::Whole);
        assert!(p.selection_as_one);
        assert_eq!(apply(p, ""), None, "a heading or the note sets nothing");
        assert_eq!(apply(p, "chooser.customize"), None);
    }
}
