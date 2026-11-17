//! The transaction mode and isolation chip in the console toolbar (D-014, D-036; M3b Task 22,
//! `design/m3b-safe` surface 3), its menu, and the "Shared session" chip (surface 12).
//!
//! ## m3b-t22-tx-ui
//!
//! - **Chip.** Right after the connection label, pinned (a trust signal: never hidden or cut):
//!   the commit glyph, the mode ("Auto-commit", "Manual", "Smart") and a chevron; a Tab stop
//!   that Enter, Space or a click opens. Production consoles start in Manual
//!   (`session::default_mode`, D-036); the chip shows what the session says.
//! - **Read-only session.** The chip reads Auto-commit, disabled (no chevron, `text-disabled`), still a
//!   Tab stop; its tooltip is the reason (Q14: a read-only session is always Auto).
//! - **Menu** (`PopupMenu`, title "Transaction mode"). With a user transaction open: "Commit
//!   transaction" and "Roll back transaction" first (Commit only on an open, not a failed one;
//!   both wait while an answer is pending, [`tx_badge::answer_waits`]), then the three modes
//!   (the check on the current one, a one-line hint), then "Isolation" with its current value.
//!   While a transaction is open the chip stays enabled (NOTES ruling 10) and the modes and
//!   Isolation are disabled with "Commit or roll back first" as their hint. On production a
//!   disabled note row says production consoles start in Manual.
//! - **Isolation** opens its own menu at the same place: "Server default" (the engine's level
//!   in mono as its hint) and the four SQL levels in mono, untranslated (they are SQL).
//! - **Requests.** A mode or a level is `Request::Tx(SetMode | SetIsolation)` on the console's
//!   session; a refusal (`TxError::InProgress`, Smart without its classifier) is a toast with the
//!   session's own sentence. Nothing here begins, commits or rolls back on its own.
//! - The status bar's transaction segment opens the same menu above itself ([`open_menu`]).

use std::rc::Rc;

use hacivat_exec::error_title_id;
use hacivat_l10n::{NoArgs, Text, ids};
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_session::session::{ExecError, IsolationLevel, Request, TxCommand, TxMode};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::menu::{MenuItemSpec, PopupMenu, PopupMenuEvent};
use hacivat_ui::popover::Tip;
use hacivat_ui::text::{Label, TextFn, TextSource};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Point,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, point, px,
};
use hacivat_ui_core::theme::Theme;

use crate::connections::ConnectionsModel;
use crate::console::tx_badge::{self, TxView};
use crate::console::{ConsoleId, Consoles};
use crate::l10n_global::{text_fn, tr};
use crate::shell::Workspace;

/// The menu's action IDs (never registered: the menu's own subscription answers them).
const COMMIT: &str = "tx-chip:commit";
const ROLLBACK: &str = "tx-chip:rollback";
const ISOLATION: &str = "tx-chip:isolation";
const MODE: &str = "tx-chip:mode:";
const LEVEL: &str = "tx-chip:level:";
const NOTE: &str = "tx-chip:note";

/// The mode's word.
pub fn mode_text(cx: &App, mode: TxMode) -> SharedString {
    match mode {
        TxMode::Auto => tr(cx, ids::hv::TX_CHIP_AUTO, NoArgs),
        TxMode::Manual => tr(cx, ids::hv::TX_CHIP_MANUAL, NoArgs),
        TxMode::Smart => tr(cx, ids::hv::TX_CHIP_SMART, NoArgs),
    }
}

fn mode_hint(mode: TxMode) -> TextFn {
    match mode {
        TxMode::Auto => text_fn(ids::hv::TX_MODE_AUTO_HINT),
        TxMode::Manual => text_fn(ids::hv::TX_MODE_MANUAL_HINT),
        TxMode::Smart => text_fn(ids::hv::TX_MODE_SMART_HINT),
    }
}

/// The SQL name of `level`; `None` for the server's default.
pub fn level_sql(level: IsolationLevel) -> Option<&'static str> {
    match level {
        IsolationLevel::Default => None,
        IsolationLevel::ReadUncommitted => Some("READ UNCOMMITTED"),
        IsolationLevel::ReadCommitted => Some("READ COMMITTED"),
        IsolationLevel::RepeatableRead => Some("REPEATABLE READ"),
        IsolationLevel::Serializable => Some("SERIALIZABLE"),
    }
}

/// The level an engine runs at when none is asked for (shown in mono beside "Server default").
/// Fix round 1 (m3): only the engines Hacivat knows (their built-in default; a server set up
/// otherwise is not read); `None` for any other, whose row then says "Server default" alone.
pub fn engine_default_level(engine: Option<DbmsKind>) -> Option<&'static str> {
    match engine? {
        DbmsKind::Postgres => Some("READ COMMITTED"),
        DbmsKind::MySql | DbmsKind::MariaDb => Some("REPEATABLE READ"),
        DbmsKind::Sqlite => Some("SERIALIZABLE"),
        _ => None,
    }
}

const LEVELS: [IsolationLevel; 5] = [
    IsolationLevel::Default,
    IsolationLevel::ReadUncommitted,
    IsolationLevel::ReadCommitted,
    IsolationLevel::RepeatableRead,
    IsolationLevel::Serializable,
];

fn level_id(level: IsolationLevel) -> &'static str {
    match level {
        IsolationLevel::Default => "default",
        IsolationLevel::ReadUncommitted => "read-uncommitted",
        IsolationLevel::ReadCommitted => "read-committed",
        IsolationLevel::RepeatableRead => "repeatable-read",
        IsolationLevel::Serializable => "serializable",
    }
}

/// Whether console `console`'s connection is production.
fn is_production(console: ConsoleId, cx: &App) -> bool {
    Consoles::get(cx, console)
        .and_then(|s| s.profile)
        .and_then(|p| cx.try_global::<ConnectionsModel>()?.get(p))
        .is_some_and(|p| p.environment == Some(EnvTag::Production))
}

/// The chip of console `console`; `None` until it has a session. It opens its menu under
/// itself.
pub fn render(console: ConsoleId, cx: &App) -> Option<AnyElement> {
    render_form(console, false, cx)
}

// ## m3c-t41 (named exception; design/m3b-safe surface 24, N4): the narrow form keeps the
// mode word and its chevron and drops the commit glyph; "Auto-commit" reads "Auto" there,
// except on a read-only session, whose disabled chip keeps "Auto-commit".
/// The chip's form in a narrow pane.
pub fn render_narrow(console: ConsoleId, cx: &App) -> Option<AnyElement> {
    render_form(console, true, cx)
}

// ## m3c-t41
/// The word the narrow chip shows (the toolbar measures its lines); `None` without a chip.
pub fn narrow_word(console: ConsoleId, cx: &App) -> Option<SharedString> {
    let view = tx_badge::view(console, cx)?;
    if view.ended {
        return None;
    }
    Some(if view.read_only {
        mode_text(cx, TxMode::Auto)
    } else if view.mode == TxMode::Auto {
        tr(cx, ids::hv::TX_CHIP_AUTO_SHORT, NoArgs)
    } else {
        mode_text(cx, view.mode)
    })
}

fn render_form(console: ConsoleId, narrow: bool, cx: &App) -> Option<AnyElement> {
    let view = tx_badge::view(console, cx)?;
    if view.ended {
        return None;
    }
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let c = &t.colors;
    let mode = if view.read_only {
        TxMode::Auto
    } else {
        view.mode
    };
    let word = mode_text(cx, mode);
    // ## m3c-t41 (N4)
    let shown = if narrow && !view.read_only && mode == TxMode::Auto {
        tr(cx, ids::hv::TX_CHIP_AUTO_SHORT, NoArgs)
    } else {
        word.clone()
    };
    let tip = if view.read_only {
        tr(cx, ids::hv::TX_CHIP_RO_REASON, NoArgs)
    } else {
        tr(
            cx,
            ids::hv::TX_CHIP_TIP,
            ids::hv::HvTxChipTip {
                mode: Text(word.to_string()),
            },
        )
    };
    // ## m3c-t41 (M3c-38): the read-only chip is disabled: the disabled ink.
    let ink = if view.read_only {
        c.text_disabled
    } else {
        c.text
    };
    let glyph = if view.read_only {
        c.text_disabled
    } else {
        c.text_2
    };
    let chip = div()
        .id("tx-chip")
        .debug_selector(|| "tx-chip".to_string())
        .relative()
        .flex()
        .flex_row()
        .gap(px(t.spacing.xs))
        .px(px(t.spacing.sm))
        .rounded(px(t.radii.sm))
        .border_1()
        .border_color(c.seam_strong.to_hsla());
    // ## m3c-t41 (N4): the narrow form drops the commit glyph and wraps its word inside the
    // chip should a long locale's word not fit the pane (chips never clip, NOTES surface 24).
    let mut chip = if narrow {
        let style = tx_badge::narrow_style(cx);
        chip.flex_1()
            .min_w(px(0.0))
            .items_center()
            .min_h(px(22.0))
            .py(px(hacivat_ui::metrics::NARROW_CHIP_PAD_Y_PX))
            .child(
                div().flex_1().min_w(px(0.0)).child(
                    hacivat_ui::text::paragraph(shown, TextSource::Fluent, style, ink.to_hsla())
                        .id("tx-chip-mode"),
                ),
            )
    } else {
        chip.flex_none()
            .items_center()
            .h(px(22.0))
            .child(
                Icon::new(IconName::GitCommit)
                    .size(IconSize::Chrome)
                    .color(glyph.to_hsla()),
            )
            .child(
                Label::new(
                    shown,
                    TextSource::Fluent,
                    t.type_ramp.caption,
                    ink.to_hsla(),
                )
                .id("tx-chip-mode"),
            )
    };
    if !view.read_only {
        chip = chip.child(
            // ## m3c-t41: never shrinks beside a wrapping word.
            div().flex_none().child(
                Icon::new(IconName::ChevronDown)
                    .size(IconSize::Chrome)
                    .color(glyph.to_hsla()),
            ),
        );
    }
    chip = chip
        .children(tx_badge::record_bounds(console, tx_badge::Focus::Chip, cx))
        .tooltip(move |_, cx| Tip::view(tip.clone(), cx));
    // A Tab stop in both states (the read-only chip's reason is its tooltip); Enter, Space and
    // a click open the menu of a read-write session.
    let read_only = view.read_only;
    let open = Rc::new(move |window: &mut Window, cx: &mut App| {
        if !read_only {
            open_chip_menu(console, window, cx);
        }
    });
    let on_click = open.clone();
    let chip = chip
        .on_click(move |_, window, cx| on_click(window, cx))
        .on_key_down(move |e, window, cx| {
            if matches!(e.keystroke.key.as_str(), "enter" | "space" | "down")
                && !e.keystroke.modifiers.modified()
            {
                cx.stop_propagation();
                open(window, cx);
            }
        });
    // The handle is the mirror's (one per console), so the focus survives repaints.
    let chip = match tx_badge::focus_of(console, tx_badge::Focus::Chip, cx) {
        Some(focus) => chip
            .track_focus(&focus)
            .focus_visible(|s| s.border_color(c.blue.to_hsla())),
        None => chip,
    };
    Some(chip.into_any_element())
}

/// The "Shared session" chip of a console that shares its session
/// ([`tx_badge::shares_session`]: the live relation, fix round 1 I1): the users glyph and the words; `icon_only` keeps the glyph
/// (the words stay its tooltip and accessible name). `None` otherwise.
pub fn render_shared(console: ConsoleId, icon_only: bool, cx: &App) -> Option<AnyElement> {
    let profile = Consoles::get(cx, console)?.profile?;
    let p = cx.try_global::<ConnectionsModel>()?.get(profile)?;
    // Fix round 1 (I1): what the consoles hold now, never the saved setting alone.
    if !tx_badge::shares_session(console, cx) {
        return None;
    }
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let c = &t.colors;
    let tip = tr(
        cx,
        ids::hv::CONSOLE_SHARED_TIP,
        ids::hv::HvConsoleSharedTip {
            name: Text(p.name.clone()),
        },
    );
    let mut chip = div()
        .id("tx-shared")
        .debug_selector(|| "tx-shared".to_string())
        .relative()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap(px(t.spacing.xs))
        .h(px(22.0))
        .px(px(t.spacing.sm))
        .rounded(px(t.radii.sm))
        .border_1()
        .border_color(c.seam_strong.to_hsla())
        .child(
            Icon::new(IconName::Users)
                .size(IconSize::Chrome)
                .color(c.text_2.to_hsla()),
        );
    if !icon_only {
        chip = chip.child(
            Label::new(
                tr(cx, ids::hv::CONSOLE_SHARED, NoArgs),
                TextSource::Fluent,
                t.type_ramp.caption,
                c.text.to_hsla(),
            )
            .id("tx-shared-label"),
        );
    }
    Some(
        chip.children(tx_badge::record_bounds(
            console,
            tx_badge::Focus::Shared,
            cx,
        ))
        .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
        .into_any_element(),
    )
}

/// The menu rows for console `console` as its session stands (see the module docs).
pub fn menu_items(console: ConsoleId, cx: &App) -> Vec<MenuItemSpec> {
    let Some(view) = tx_badge::view(console, cx) else {
        return Vec::new();
    };
    let waits = tx_badge::answer_waits(console, cx);
    let open = view.user.is_some();
    let mut items = Vec::new();
    if let Some(user) = view.user {
        items.push(
            MenuItemSpec::new(text_fn(ids::hv::CMD_TX_COMMIT), COMMIT)
                .disabled(user.failed || waits),
        );
        items.push(MenuItemSpec::new(text_fn(ids::hv::CMD_TX_ROLLBACK), ROLLBACK).disabled(waits));
        items.push(MenuItemSpec::separator());
    }
    let reason = text_fn(ids::hv::TX_MENU_REASON_OPEN);
    for mode in TxMode::ALL {
        let hint = if open {
            reason.clone()
        } else {
            mode_hint(mode)
        };
        items.push(
            MenuItemSpec::new(
                Rc::new(move |cx: &App| mode_text(cx, mode)),
                format!("{MODE}{}", mode.id()),
            )
            .checked(view.mode == mode)
            .disabled(open || view.read_only)
            .hint(hint),
        );
    }
    items.push(MenuItemSpec::separator());
    let engine = tx_badge::engine_of(console, cx);
    let level = view.isolation;
    let level_hint: TextFn = if open {
        reason
    } else {
        match level_sql(level) {
            Some(sql) => Rc::new(move |_: &App| SharedString::from(sql)),
            None => Rc::new(move |cx: &App| match engine_default_level(engine) {
                Some(level) => tr(
                    cx,
                    ids::hv::TX_ISO_DEFAULT_HINT,
                    ids::hv::HvTxIsoDefaultHint {
                        level: Text(level.to_string()),
                    },
                ),
                None => tr(cx, ids::hv::TX_ISO_DEFAULT, NoArgs),
            }),
        }
    };
    items.push(
        MenuItemSpec::new(text_fn(ids::hv::TX_MENU_ISOLATION), ISOLATION)
            // Fix round 1 (m1): the row opens a menu of its own ("Isolation ›").
            .keys(Rc::new(|_: &App| SharedString::from("›")))
            .disabled(open || view.read_only)
            .hint(level_hint)
            .hint_source(if open || level_sql(level).is_none() {
                TextSource::Fluent
            } else {
                TextSource::Data
            }),
    );
    if is_production(console, cx) {
        items.push(MenuItemSpec::separator());
        // ## m3b-t26-prod-unlock (named exception, the T22 review m12 carry): an informational
        // note (12 px `text-2`), not a disabled command in `text-3` on the raised menu.
        items.push(MenuItemSpec::new(text_fn(ids::hv::TX_MENU_PROD_NOTE), NOTE).note(true));
    }
    items
}

/// The isolation menu's rows: the server's default, then the four levels in mono.
pub fn isolation_items(console: ConsoleId, cx: &App) -> Vec<MenuItemSpec> {
    let Some(view) = tx_badge::view(console, cx) else {
        return Vec::new();
    };
    let engine = tx_badge::engine_of(console, cx);
    LEVELS
        .into_iter()
        .map(|level| {
            let action = format!("{LEVEL}{}", level_id(level));
            let item = match level_sql(level) {
                Some(sql) => {
                    MenuItemSpec::new(Rc::new(move |_: &App| SharedString::from(sql)), action)
                        .mono(true)
                }
                None => {
                    let item = MenuItemSpec::new(text_fn(ids::hv::TX_ISO_DEFAULT), action);
                    match engine_default_level(engine) {
                        Some(level) => item
                            .hint(Rc::new(move |_: &App| SharedString::from(level)))
                            .hint_source(TextSource::Data),
                        None => item,
                    }
                }
            };
            item.checked(view.isolation == level)
        })
        .collect()
}

/// Opens console `console`'s transaction menu under its chip (4 px below it).
pub fn open_chip_menu(console: ConsoleId, window: &mut Window, cx: &mut App) {
    let anchor = tx_badge::bounds_of(console, tx_badge::Focus::Chip, cx)
        .map_or_else(|| window.mouse_position(), |b| b.bottom_left());
    open_menu(console, point(anchor.x, anchor.y + px(4.0)), window, cx);
}

/// Opens console `console`'s transaction menu from its status segment: 4 px above it, left
/// edges aligned (the menu flips up to fit, NOTES surface 4).
pub fn open_segment_menu(console: ConsoleId, window: &mut Window, cx: &mut App) {
    let anchor = tx_badge::bounds_of(console, tx_badge::Focus::Segment, cx)
        .map_or_else(|| window.mouse_position(), |b| b.origin);
    open_menu(console, point(anchor.x, anchor.y - px(4.0)), window, cx);
}

/// Opens console `console`'s isolation menu under its chip.
pub fn open_isolation_menu(console: ConsoleId, window: &mut Window, cx: &mut App) {
    let anchor = tx_badge::bounds_of(console, tx_badge::Focus::Chip, cx)
        .map_or_else(|| window.mouse_position(), |b| b.bottom_left());
    let items = isolation_items(console, cx);
    if !items.is_empty() {
        open_with(
            console,
            point(anchor.x, anchor.y + px(4.0)),
            items,
            MenuKind::Isolation,
            window,
            cx,
        );
    }
}

/// Opens console `console`'s transaction menu with its top left at `anchor` (the status
/// segment passes a point above itself; the menu flips up to fit).
pub fn open_menu(console: ConsoleId, anchor: Point<Pixels>, window: &mut Window, cx: &mut App) {
    let items = menu_items(console, cx);
    if items.is_empty() {
        return;
    }
    open_with(console, anchor, items, MenuKind::Mode, window, cx);
}

#[derive(Clone, Copy)]
enum MenuKind {
    Mode,
    Isolation,
}

fn open_with(
    console: ConsoleId,
    anchor: Point<Pixels>,
    items: Vec<MenuItemSpec>,
    kind: MenuKind,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    let menu = ws.update(cx, |ws, cx| ws.open_popup_menu(anchor, items, window, cx));
    menu.update(cx, |m: &mut PopupMenu, _| {
        m.set_access_keys(false);
        match kind {
            MenuKind::Mode => m.title(text_fn(ids::hv::TX_MENU_TITLE)),
            MenuKind::Isolation => m.title(text_fn(ids::hv::TX_MENU_ISOLATION)),
        }
    });
    // The workspace closes the menu on any event; this answers the choice.
    cx.subscribe(&menu, move |_, event: &PopupMenuEvent, cx| {
        if let PopupMenuEvent::Chosen(action) = event {
            let action = action.to_string();
            // After the menu's layer is gone (the workspace removes it on this event).
            cx.defer(move |cx| {
                if let Some(handle) = crate::exec_ui::workspace_window(cx) {
                    let _ = handle.update(cx, |_, window, cx| {
                        choose(console, &action, anchor, window, cx);
                    });
                } else {
                    choose_headless(console, &action, cx);
                }
            });
        }
    })
    .detach();
}

/// Answers a menu row.
fn choose(
    console: ConsoleId,
    action: &str,
    anchor: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    if action == ISOLATION {
        let items = isolation_items(console, cx);
        open_with(console, anchor, items, MenuKind::Isolation, window, cx);
        return;
    }
    choose_headless(console, action, cx);
}

/// Answers a menu row that needs no window.
pub fn choose_headless(console: ConsoleId, action: &str, cx: &mut App) {
    match action {
        COMMIT | ROLLBACK => {
            let commit = action == COMMIT;
            let user = tx_badge::user_tx(console, cx);
            if user.is_some_and(|u| !(commit && u.failed)) && !tx_badge::answer_waits(console, cx) {
                crate::exec_ui::tx::end_tx(console, commit, cx).detach();
            }
        }
        _ => {
            if let Some(mode) = action.strip_prefix(MODE).and_then(TxMode::from_id) {
                send(console, TxCommand::SetMode(mode), cx);
            } else if let Some(level) = action
                .strip_prefix(LEVEL)
                .and_then(|id| LEVELS.into_iter().find(|l| level_id(*l) == id))
            {
                send(console, TxCommand::SetIsolation(level), cx);
            }
        }
    }
}

/// Sends `command` on console `console`'s session; a refusal is a toast with the session's
/// own sentence. Never while a transaction is open (the menu disables those rows): that
/// refusal is the same toast (`exec-error-tx-in-progress`), said before anything is sent.
pub fn send(console: ConsoleId, command: TxCommand, cx: &mut App) {
    let Some(view) = tx_badge::view(console, cx) else {
        return;
    };
    // A mode or level never changes under an open transaction (T19: the session refuses
    // `SetMode(Smart)` then too); the refusal is said, never swallowed. A read-only session
    // is always Auto (Q14).
    let refused = if view.user.is_some() {
        Some(hacivat_session::session::TxError::InProgress)
    } else if view.read_only {
        Some(hacivat_session::session::TxError::Unsupported)
    } else {
        None
    };
    if let Some(e) = refused {
        let id = error_title_id(&hacivat_session::session::ExecErrorKind::Tx(e));
        ToastQueue::push(
            cx,
            Toast::new(ToastKind::Error, crate::l10n_global::text_fn_id(id)),
        );
        return;
    }
    let Some(session) = Consoles::get(cx, console).and_then(|s| s.session.clone()) else {
        return;
    };
    let ticket = session.submit(Request::Tx(command));
    cx.spawn(async move |cx| {
        let reply = ticket.reply.await;
        drop(session);
        let error: Option<ExecError> = match reply {
            Ok(Ok(_)) => None,
            Ok(Err(e)) => Some(e),
            Err(_) => Some(ExecError::of(
                hacivat_session::session::ExecErrorKind::NotConnected,
            )),
        };
        if let Some(e) = error {
            cx.update(|cx| {
                let id = error_title_id(&e.kind);
                ToastQueue::push(
                    cx,
                    Toast::new(ToastKind::Error, crate::l10n_global::text_fn_id(id)),
                );
            });
        }
    })
    .detach();
}

/// The chip as a toolbar button would announce it: its word (for the overflow label).
pub fn chip_label(console: ConsoleId, cx: &App) -> SharedString {
    let view: Option<TxView> = tx_badge::view(console, cx);
    match view {
        Some(v) if !v.read_only => mode_text(cx, v.mode),
        _ => mode_text(cx, TxMode::Auto),
    }
}
