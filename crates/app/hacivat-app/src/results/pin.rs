//! Pinned result tabs (HV-EXEC-07 UI; M3b Task 25): a pinned tab is never reused by the next
//! run; pin and unpin from the tab's context menu.
//!
//! ## m3b-t01b-seams: the seam (Task 1b). STUB: no tab is pinned, so every run reuses tabs as
//! in M3a. Task 25 fills it.
//!
//! ## m3b-t25-results (design/m3b-safe surface 10)
//! The tab menu opens on a right press on a result tab: Pin tab / Unpin tab, a separator,
//! Close tab, Close other tabs. A pinned tab shows the 12 px pin glyph (`text-2`, tooltip
//! "Pinned: the next run opens a new tab"); the next run of its statement opens a new tab
//! (`ConsoleResults::apply`), and eviction treats it as on screen.

use hacivat_l10n::ids;
use hacivat_ui::menu::{MenuItemSpec, PopupMenu, PopupMenuEvent};
use hacivat_ui_core::gpui_surface::{App, Pixels, Point, Window};

use crate::console::{ConsoleId, Consoles};
use crate::l10n_global::text_fn;
use crate::shell::Workspace;

/// Whether console `console`'s result tab `ix` is pinned.
pub fn is_pinned(console: ConsoleId, ix: usize, cx: &App) -> bool {
    // ## m3b-t25-results
    Consoles::get(cx, console)
        .and_then(|s| s.results.clone())
        .is_some_and(|r| r.read(cx).tab(ix).is_some_and(|t| t.pinned()))
}

// ## m3b-t25-results

/// The tab menu's action IDs (answered by its subscription; no command has them).
const PIN: &str = "results.tab_menu.pin";
const UNPIN: &str = "results.tab_menu.unpin";
const CLOSE: &str = "results.tab_menu.close";
const CLOSE_OTHERS: &str = "results.tab_menu.close_others";

/// Pins (or unpins) console `console`'s result tab `ix`.
pub fn set_pinned(console: ConsoleId, ix: usize, pinned: bool, cx: &mut App) {
    if let Some(results) = Consoles::get(cx, console).and_then(|s| s.results.clone()) {
        results.update(cx, |r, cx| r.set_pinned(ix, pinned, cx));
    }
}

/// The tab menu of console `console`'s result tab `ix`.
pub fn menu_items(console: ConsoleId, ix: usize, cx: &App) -> Vec<MenuItemSpec> {
    let pinned = is_pinned(console, ix, cx);
    let others = Consoles::get(cx, console)
        .and_then(|s| s.results.clone())
        .is_some_and(|r| r.read(cx).tabs().len() > 1);
    vec![
        if pinned {
            MenuItemSpec::new(text_fn(ids::hv::RESULTS_TAB_UNPIN), UNPIN)
        } else {
            MenuItemSpec::new(text_fn(ids::hv::RESULTS_TAB_PIN), PIN)
        },
        MenuItemSpec::separator(),
        MenuItemSpec::new(text_fn(ids::hui::TAB_CLOSE), CLOSE),
        MenuItemSpec::new(text_fn(ids::hv::RESULTS_TAB_CLOSE_OTHERS), CLOSE_OTHERS)
            .disabled(!others),
    ]
}

/// Opens the tab menu of console `console`'s result tab `ix` with its top left at `anchor`.
pub fn open_tab_menu(
    console: ConsoleId,
    ix: usize,
    anchor: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    let items = menu_items(console, ix, cx);
    let menu = ws.update(cx, |ws, cx| ws.open_popup_menu(anchor, items, window, cx));
    menu.update(cx, |m: &mut PopupMenu, _| m.set_access_keys(false));
    cx.subscribe(&menu, move |_, event: &PopupMenuEvent, cx| {
        if let PopupMenuEvent::Chosen(action) = event {
            choose(console, ix, action, cx);
        }
    })
    .detach();
}

/// Does what the tab menu's row `action` says.
pub fn choose(console: ConsoleId, ix: usize, action: &str, cx: &mut App) {
    let Some(results) = Consoles::get(cx, console).and_then(|s| s.results.clone()) else {
        return;
    };
    match action {
        PIN => results.update(cx, |r, cx| r.set_pinned(ix, true, cx)),
        UNPIN => results.update(cx, |r, cx| r.set_pinned(ix, false, cx)),
        CLOSE => results.update(cx, |r, cx| r.close_tab(ix, cx)),
        CLOSE_OTHERS => results.update(cx, |r, cx| r.close_others(ix, cx)),
        _ => {}
    }
}
