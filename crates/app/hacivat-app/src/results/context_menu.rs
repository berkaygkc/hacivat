//! The grid's context menu (HV-GRID-04 menu; M3c Task 39): cell, header and row-number variants
//! with disabled items and their reasons.
//!
//! ## m3b-t01b-seams: the seam (Task 1b). STUB: opens nothing
//! (`GridEvent::ContextMenuRequested` is not answered yet). Task 39 fills it.
//!
//! ## m3c-t39 (design/m3b-safe surface 23)
//! - **Where it opens.** A right press on a cell, a column header or a row number opens it at
//!   the pointer; ⇧F10, the context-menu key and a screen reader's Show menu open it under the
//!   focus cell with its first item active. What the press hits outside the selection is
//!   selected first (the grid does that); inside the selection the menu acts on the selection.
//! - **Rows** (NOTES surface 23):
//!   - cell: Copy · Copy with headers | Open value viewer · Raw display (this column) | Sort
//!     ascending · Sort descending · Clear sort | Go to row…;
//!   - header: Sort ascending · Sort descending (✓ when current) · Clear sort · Add as sort
//!     level N (while another column is sorted) | Freeze up to here or Unfreeze columns · Hide
//!     column · Show all columns · Fit width | Raw display (this column) · Copy column name;
//!   - row number: Copy rows · Copy with headers | Show as record · Go to row….
//!
//!   The Transposed and Text views offer no sorting and no go to row (Task 38): their cell menu
//!   ends after Raw display.
//! - **Disabled items keep their place** with the reason as the hint: "Ranges don't line up"
//!   (a copy the grid would refuse), "Select one cell", "Not sorted", "None hidden", "Last
//!   shown column".
//! - **Actions.** Copy, Open value viewer, Go to row, Unfreeze and Fit width are the grid's
//!   commands (`grid.*`): the workspace runs them on the grid once the menu has closed and the
//!   focus is back. The others are the menu's own and act on the grid that asked
//!   ([`choose`]): sorting by the menu's column (`GridView::request_sort`), freezing through it,
//!   hiding and showing columns, raw display, copying the column's name, the record inspector.
//! - **Raw display** is per column here (by column name, for every result of the console) and
//!   global in Settings (`grid.raw_display`); the column's choice wins. Copy is raw either way.
//! - **Keyboard-active row.** A menu opened from the keys starts on its first item with the
//!   selection tint (`PopupMenu::set_active_tint`, the chooser's look until Task 41 gives every
//!   menu its keyboard-active row).

use std::rc::Rc;

use hacivat_l10n::{Count, MsgRef, NoArgs, ids};
use hacivat_ui::menu::{MenuItemSpec, PopupMenu, PopupMenuEvent};
use hacivat_ui::text::TextFn;
use hacivat_ui_core::gpui_surface::{
    App, ClipboardItem, Entity, Global, Pixels, Point, WeakEntity, Window,
};
use hacivat_ui_grid::model::{CopyRefusal, SortKey, SortSpec};
use hacivat_ui_grid::{ColId, GridView, ModelCell};

use crate::console::{ConsoleId, Consoles};
use crate::l10n_global::{text_fn, tr};
use crate::shell::Workspace;
use crate::shell::keymap_state::shortcut_label;

// ## m3c-t39

/// Which menu: over a cell, a column header or a row number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuKind {
    Cell,
    Header,
    RowNumber,
}

/// What a menu is over: its kind, the column it names (a header's; a cell's; the focus column
/// of a row number) and the cell (none for a header).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuTarget {
    pub kind: MenuKind,
    pub col: ColId,
    pub cell: Option<ModelCell>,
}

/// A grid's request for its menu (`GridEvent::ContextMenuRequested`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MenuRequest {
    /// Where the menu's top left goes (window coordinates).
    pub at: Point<Pixels>,
    pub cell: Option<ModelCell>,
    pub header: Option<ColId>,
    pub row_number: bool,
    /// A right press asked; else the keys or a screen reader.
    pub pointer: bool,
}

impl MenuRequest {
    /// What the menu is over; `None` when the request names nothing.
    pub fn target(&self) -> Option<MenuTarget> {
        match (self.header, self.cell) {
            (Some(col), _) => Some(MenuTarget {
                kind: MenuKind::Header,
                col,
                cell: None,
            }),
            (None, Some(cell)) => Some(MenuTarget {
                kind: if self.row_number {
                    MenuKind::RowNumber
                } else {
                    MenuKind::Cell
                },
                col: cell.col,
                cell: Some(cell),
            }),
            (None, None) => None,
        }
    }
}

/// The grid's commands the menu runs (the workspace runs a chosen row's action as a command
/// once the menu has closed).
pub const COPY: &str = "grid.copy";
pub const COPY_HEADERS: &str = "grid.copy_with_headers";
pub const OPEN_VALUE: &str = "grid.open_value";
pub const GOTO: &str = "grid.goto_row";
pub const UNFREEZE: &str = "grid.unfreeze";
pub const AUTO_FIT: &str = "grid.auto_fit";

/// The menu's own actions ([`choose`] answers them; no command has them).
pub const SORT_ASC: &str = "results.grid_menu.sort_asc";
pub const SORT_DESC: &str = "results.grid_menu.sort_desc";
pub const CLEAR_SORT: &str = "results.grid_menu.clear_sort";
pub const ADD_SORT: &str = "results.grid_menu.add_sort";
pub const FREEZE: &str = "results.grid_menu.freeze";
pub const HIDE: &str = "results.grid_menu.hide";
pub const SHOW_ALL: &str = "results.grid_menu.show_all";
pub const RAW: &str = "results.grid_menu.raw";
pub const COPY_NAME: &str = "results.grid_menu.copy_name";
pub const RECORD: &str = "results.grid_menu.record";

/// The menu last opened, while it is open.
#[derive(Default)]
struct OpenMenu(Option<WeakEntity<PopupMenu>>);

impl Global for OpenMenu {}

/// The grid menu that is open, if one is.
pub fn open_menu(cx: &App) -> Option<Entity<PopupMenu>> {
    cx.try_global::<OpenMenu>()?.0.as_ref()?.upgrade()
}

/// Opens the menu `request` asks for over console `console`'s grid `grid` (the tile calls it
/// on `GridEvent::ContextMenuRequested`; the Transposed view's grid passes its table cell).
/// Deferred: the grid is still being updated when it asks.
pub fn open(console: ConsoleId, grid: &Entity<GridView>, request: MenuRequest, cx: &mut App) {
    let Some(target) = request.target() else {
        return;
    };
    let grid = grid.downgrade();
    cx.defer(move |cx| {
        let Some(grid) = grid.upgrade() else {
            return;
        };
        let window = cx
            .active_window()
            .into_iter()
            .chain(cx.windows())
            .find(|w| w.downcast::<Workspace>().is_some());
        if let Some(window) = window {
            let _ = window.update(cx, |_, window, cx| {
                show(console, &grid, target, request, window, cx);
            });
        }
    });
}

/// Opens the menu in `window`'s workspace (a `menu` layer, which gives the focus back to the
/// grid when it closes).
fn show(
    console: ConsoleId,
    grid: &Entity<GridView>,
    target: MenuTarget,
    request: MenuRequest,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    let items = menu_items(console, grid, target, cx);
    let first = items
        .iter()
        .position(|i| !i.separator && !i.disabled && !i.note);
    let menu = ws.update(cx, |ws, cx| {
        ws.open_popup_menu(request.at, items, window, cx)
    });
    menu.update(cx, |m, _| {
        m.set_access_keys(false);
        m.label(text_fn(ids::hv::GMENU_LABEL));
        if request.pointer {
            // The pointer chooses: no row is active until it (or an arrow key) moves.
            m.clear_selection();
        } else {
            // The keys: the first item is active, with the selection tint (NOTES surface 23).
            m.set_active_tint(true);
            match first {
                Some(ix) => m.select(ix),
                None => m.clear_selection(),
            }
        }
    });
    let weak = grid.downgrade();
    cx.subscribe(&menu, move |_, event: &PopupMenuEvent, cx| {
        if let PopupMenuEvent::Chosen(action) = event
            && let Some(grid) = weak.upgrade()
        {
            choose(console, &grid, target, action, cx);
        }
    })
    .detach();
    cx.set_global(OpenMenu(Some(menu.downgrade())));
}

/// The rows of the menu over `target` of console `console`'s grid `grid`, as they stand now.
pub fn menu_items(
    console: ConsoleId,
    grid: &Entity<GridView>,
    target: MenuTarget,
    cx: &App,
) -> Vec<MenuItemSpec> {
    let g = grid.read(cx);
    let table = super::views::table_shown(cx);
    let name = column_name(console, target.col, cx);
    let raw = name.as_deref().is_some_and(|n| is_raw(console, n, cx));
    let map = g.view_map();
    let visible = g.column_layout().visible();
    let copy_off = matches!(
        g.selection().normalize_for_copy(map, visible),
        Err(CopyRefusal::MultipleSelection)
    );
    let copy = |label, action| {
        let item = keyed(MenuItemSpec::new(text_fn(label), action), action, cx);
        reason(item, copy_off, ids::hv::GMENU_WHY_RANGES)
    };
    let raw_item = MenuItemSpec::new(text_fn(ids::hv::GMENU_RAW), RAW)
        .checked(raw)
        .hint(text_fn(ids::hv::GMENU_RAW_HINT));
    let sort = g.sort().clone();
    let sorting = super::sort::status(grid, cx).is_some_and(|s| s.running.is_some());
    let clear = reason(
        MenuItemSpec::new(text_fn(ids::hv::GRID_CLEAR_SORT), CLEAR_SORT),
        sort.is_empty() && !sorting,
        ids::hv::GMENU_WHY_NOT_SORTED,
    );
    let goto = || {
        keyed(
            MenuItemSpec::new(text_fn(ids::hv::GMENU_GOTO), GOTO),
            GOTO,
            cx,
        )
    };
    let mut items = Vec::new();
    match target.kind {
        MenuKind::Cell => {
            let many = g.selection().cell_count_in(map, visible) > 1;
            items.push(copy(ids::hv::GMENU_COPY, COPY));
            items.push(copy(ids::hv::GMENU_COPY_HEADERS, COPY_HEADERS));
            items.push(MenuItemSpec::separator());
            let open = keyed(
                MenuItemSpec::new(text_fn(ids::hv::GMENU_OPEN_VALUE), OPEN_VALUE),
                OPEN_VALUE,
                cx,
            );
            items.push(reason(open, many, ids::hv::GMENU_WHY_ONE_CELL));
            items.push(raw_item);
            if table {
                items.push(MenuItemSpec::separator());
                items.push(MenuItemSpec::new(
                    text_fn(ids::hv::GMENU_SORT_ASC),
                    SORT_ASC,
                ));
                items.push(MenuItemSpec::new(
                    text_fn(ids::hv::GMENU_SORT_DESC),
                    SORT_DESC,
                ));
                items.push(clear);
                items.push(MenuItemSpec::separator());
                items.push(goto());
            }
        }
        MenuKind::Header => {
            let level = sort.level_of(target.col).map(|(_, k)| k.desc);
            items.push(
                MenuItemSpec::new(text_fn(ids::hv::GMENU_SORT_ASC), SORT_ASC)
                    .checked(level == Some(false)),
            );
            items.push(
                MenuItemSpec::new(text_fn(ids::hv::GMENU_SORT_DESC), SORT_DESC)
                    .checked(level == Some(true)),
            );
            items.push(clear);
            if !sort.is_empty() && level.is_none() {
                let lvl = sort.keys.len() as u64 + 1;
                let label: TextFn = Rc::new(move |cx: &App| {
                    tr(
                        cx,
                        ids::hv::GMENU_ADD_SORT,
                        ids::hv::HvGmenuAddSort { lvl: Count(lvl) },
                    )
                });
                items.push(MenuItemSpec::new(label, ADD_SORT));
            }
            items.push(MenuItemSpec::separator());
            items.push(if g.frozen() > 0 {
                MenuItemSpec::new(text_fn(ids::hv::GMENU_UNFREEZE), UNFREEZE)
            } else {
                MenuItemSpec::new(text_fn(ids::hv::GMENU_FREEZE), FREEZE)
            });
            let layout = g.column_layout();
            items.push(reason(
                MenuItemSpec::new(text_fn(ids::hv::GMENU_HIDE), HIDE),
                layout.visible().len() <= 1,
                ids::hv::GMENU_WHY_LAST,
            ));
            items.push(reason(
                MenuItemSpec::new(text_fn(ids::hv::GMENU_SHOW_ALL), SHOW_ALL),
                layout.hidden().is_empty(),
                ids::hv::GMENU_WHY_NO_HIDDEN,
            ));
            items.push(MenuItemSpec::new(text_fn(ids::hv::GMENU_AUTOFIT), AUTO_FIT));
            items.push(MenuItemSpec::separator());
            items.push(raw_item);
            items.push(MenuItemSpec::new(
                text_fn(ids::hv::GMENU_COPY_NAME),
                COPY_NAME,
            ));
        }
        MenuKind::RowNumber => {
            items.push(copy(ids::hv::GMENU_COPY_ROWS, COPY));
            items.push(copy(ids::hv::GMENU_COPY_HEADERS, COPY_HEADERS));
            items.push(MenuItemSpec::separator());
            items.push(MenuItemSpec::new(text_fn(ids::hv::GMENU_RECORD), RECORD));
            if table {
                items.push(goto());
            }
        }
    }
    items
}

/// `item` with its command's shortcut as its keys, when the keymap binds one.
fn keyed(item: MenuItemSpec, command: &'static str, cx: &App) -> MenuItemSpec {
    if shortcut_label(cx, command).is_none() {
        return item;
    }
    let keys: TextFn =
        Rc::new(move |cx: &App| shortcut_label(cx, command).unwrap_or_default().into());
    item.keys(keys)
}

/// `item`, disabled with its reason as the hint when `off`.
fn reason(item: MenuItemSpec, off: bool, why: MsgRef<NoArgs>) -> MenuItemSpec {
    if off {
        item.disabled(true).hint(text_fn(why))
    } else {
        item
    }
}

/// Does what the row `action` of the menu over `target` says, on console `console`'s grid
/// `grid` (the grid's own commands are run by the workspace, not here).
pub fn choose(
    console: ConsoleId,
    grid: &Entity<GridView>,
    target: MenuTarget,
    action: &str,
    cx: &mut App,
) {
    let col = target.col;
    match action {
        SORT_ASC | SORT_DESC => {
            let desc = action == SORT_DESC;
            grid.update(cx, |g, cx| g.request_sort(SortSpec::by(col, desc), cx));
        }
        CLEAR_SORT => grid.update(cx, |g, cx| g.request_sort(SortSpec::default(), cx)),
        ADD_SORT => grid.update(cx, |g, cx| {
            let mut spec = g.sort().clone();
            if spec.level_of(col).is_none() {
                spec.keys.push(SortKey { col, desc: false });
                g.request_sort(spec, cx);
            }
        }),
        FREEZE => grid.update(cx, |g, cx| g.freeze_through(col, cx)),
        HIDE => grid.update(cx, |g, cx| {
            if g.column_layout().visible().len() > 1 {
                g.set_column_hidden(col, true, cx);
            }
        }),
        SHOW_ALL => grid.update(cx, |g, cx| {
            let hidden = g.column_layout().hidden().to_vec();
            for col in hidden {
                g.set_column_hidden(col, false, cx);
            }
        }),
        RAW => {
            if let Some(name) = column_name(console, col, cx) {
                let raw = is_raw(console, &name, cx);
                set_raw(console, &name, !raw, cx);
            }
        }
        COPY_NAME => {
            if let Some(name) = column_name(console, col, cx) {
                cx.write_to_clipboard(ClipboardItem::new_string(name.to_string()));
            }
        }
        RECORD => cx.defer(|cx| {
            if let Some(window) = cx.active_window() {
                let _ = window.update(cx, |_, window, cx| {
                    if !super::views::record_open(window, cx) {
                        super::views::toggle_record(window, cx);
                    }
                });
            }
        }),
        _ => {}
    }
}

/// The name of column `col` of console `console`'s active result (data).
fn column_name(console: ConsoleId, col: ColId, cx: &App) -> Option<std::sync::Arc<str>> {
    let (_, adapter) = super::find::active_target(console, cx)?;
    let snapshot = adapter.snapshot();
    snapshot
        .schema()
        .columns
        .get(col.0 as usize)
        .map(|c| c.name.clone())
}

/// Whether console `console` shows the column named `name` raw: its own choice, else the
/// `grid.raw_display` setting.
pub fn is_raw(console: ConsoleId, name: &str, cx: &App) -> bool {
    let all = super::adapter::raw_display_setting(cx);
    match Consoles::get(cx, console).and_then(|s| s.results.clone()) {
        Some(results) => {
            let mut raw = results.read(cx).display().raw().clone();
            raw.all = all;
            raw.is_raw(name)
        }
        None => all,
    }
}

/// Shows the column named `name` raw (or not) in console `console`'s results.
pub fn set_raw(console: ConsoleId, name: &str, raw: bool, cx: &mut App) {
    if let Some(results) = Consoles::get(cx, console).and_then(|s| s.results.clone()) {
        results.update(cx, |r, cx| r.set_raw_column(name, raw, cx));
    }
}
