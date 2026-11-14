//! The editor's context menu (Task 25; `design/m3a-query` surface 8): a 250 px `PopupMenu`
//! with Cut, Copy, Paste, Select all · Run, Describe execution range.
//!
//! `editor.context_menu` opens it at the right click's pointer
//! (`EditorView::context_menu_point`), or under the caret when the keyboard asked (Shift+F10,
//! the menu key), and tells the host (`EditorEvent::ContextMenuOpened { at }`). The menu is an
//! embedded `PopupMenu` the view draws through `commands::overlay` in a `deferred(anchored())`
//! (the `Select` pattern), so it paints over the text and stays inside the window. It takes
//! the focus; choosing an entry, Escape, a press outside or the focus leaving closes it, and
//! the focus goes back to the editor. An entry runs its editor command through
//! `EditorView::run_command`, the same path as a key.
//!
//! Cut and Copy stay enabled without a selection (they take the line); a read-only editor
//! disables Cut and Paste. Key hints come from the host ([`ShortcutHints`]): only the app's
//! keymap knows the chord of a command on the active layout.

use std::rc::Rc;

use hacivat_ui::l10n::ui_text;
use hacivat_ui::menu::{MenuItemSpec, PopupMenu, PopupMenuEvent};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, Context, Entity, EntityId, Focusable as _, Global, IntoElement as _,
    ParentElement as _, Pixels, Point, SharedString, Subscription, Window, anchored, deferred,
    point, px,
};

use hacivat_ui_core::theme::Theme;

use super::command_by_id;
use crate::view::{EditorEvent, EditorView};

/// The menu's width (surface 8).
const MENU_WIDTH_PX: f32 = 250.0;

/// The menu's entries: editor command ids, `None` for a separator.
const ENTRIES: [Option<&str>; 7] = [
    Some("editor.cut"),
    Some("editor.copy"),
    Some("editor.paste"),
    Some("editor.select_all"),
    None,
    Some("editor.run"),
    Some("editor.describe_exec_range"),
];

/// What [`ShortcutHints`] holds: the chord of the command with the given id, if it has one.
pub type HintFn = dyn Fn(&App, &str) -> Option<SharedString>;

/// The chord of a command as the user's keymap and layout show it (`None`: unbound). The app
/// installs it as a global (Task 32); without it the menu shows no key hints.
#[derive(Clone)]
pub struct ShortcutHints(pub Rc<HintFn>);

impl Global for ShortcutHints {}

impl std::fmt::Debug for ShortcutHints {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShortcutHints").finish_non_exhaustive()
    }
}

/// The open menu: its entity, where it opened, and what keeps it listening.
pub(crate) struct OpenMenu {
    menu: Entity<PopupMenu>,
    at: Point<Pixels>,
    _subscriptions: [Subscription; 2],
}

impl std::fmt::Debug for OpenMenu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenMenu")
            .field("at", &self.at)
            .finish_non_exhaustive()
    }
}

/// The context menu's entries for `v`, in order (labels are the commands' titles).
pub fn context_menu_entries(v: &EditorView, cx: &App) -> Vec<MenuItemSpec> {
    let hints = cx.try_global::<ShortcutHints>().cloned();
    ENTRIES
        .iter()
        .map(|entry| {
            let Some(command) = entry.and_then(command_by_id) else {
                return MenuItemSpec::separator();
            };
            let (id, title) = (command.id, command.title_id);
            let edits = matches!(id, "editor.cut" | "editor.paste");
            let mut item = MenuItemSpec::new(Rc::new(move |cx: &App| ui_text(cx, title)), id)
                .disabled(edits && v.config.read_only);
            if let Some(hints) = hints.clone().filter(|h| (h.0)(cx, id).is_some()) {
                item = item.keys(Rc::new(move |cx: &App| {
                    (hints.0)(cx, id).unwrap_or_default()
                }));
            }
            item
        })
        .collect()
}

pub(super) fn context_menu(v: &mut EditorView, window: &mut Window, cx: &mut Context<EditorView>) {
    super::acted(v);
    close(v, false, window, cx);
    // The menu's surface is drawn with the installed theme's tokens. Without one (a bare test
    // window; the app always installs it) there is nothing to draw it with, so nothing opens.
    if !cx.has_global::<Theme>() {
        return;
    }
    // The pointer of a right click; else under the caret; else the text's corner (the caret is
    // not painted: scrolled out of view, or no frame yet).
    let at = v
        .context_menu_point()
        .or_else(|| v.caret_bounds().map(|b| point(b.left(), b.bottom())))
        .or_else(|| v.last_layout().map(|l| l.rows().bounds.origin))
        .unwrap_or_default();
    let items = context_menu_entries(v, cx);
    let menu = PopupMenu::embedded(items, window, cx);
    menu.update(cx, |menu, _| {
        menu.set_min_width(px(MENU_WIDTH_PX));
        menu.label(Rc::new(|cx: &App| {
            ui_text(cx, "hv-cmd-editor-context-menu")
        }));
    });
    let events = cx.subscribe_in(&menu, window, on_menu_event);
    // The focus leaving the menu (a press elsewhere, another window's popover) closes it.
    let focus = menu.focus_handle(cx);
    let id = menu.entity_id();
    let blur = cx.on_focus_out(&focus, window, move |v, _, window, cx| {
        if is_open(v, id) {
            close(v, false, window, cx);
        }
    });
    v.commands.menu = Some(OpenMenu {
        menu,
        at,
        _subscriptions: [events, blur],
    });
    cx.emit(EditorEvent::ContextMenuOpened { at });
    cx.notify();
}

/// Closes the menu; `restore` gives the focus back to the editor.
fn close(v: &mut EditorView, restore: bool, window: &mut Window, cx: &mut Context<EditorView>) {
    let Some(open) = v.commands.menu.take() else {
        return;
    };
    // Dropped once this update is done: the event in flight still reaches its subscribers (the
    // `open_dialog` pattern).
    cx.defer(move |_| drop(open));
    if restore {
        window.focus(&v.focus, cx);
    }
    cx.notify();
}

/// The open menu is the one with entity `id` (an event of a menu already replaced by a newer
/// one, as a second right click does, must not close the newer one).
fn is_open(v: &EditorView, id: EntityId) -> bool {
    v.commands
        .menu
        .as_ref()
        .is_some_and(|open| open.menu.entity_id() == id)
}

fn on_menu_event(
    v: &mut EditorView,
    menu: &Entity<PopupMenu>,
    event: &PopupMenuEvent,
    window: &mut Window,
    cx: &mut Context<EditorView>,
) {
    if !is_open(v, menu.entity_id()) {
        return;
    }
    close(v, true, window, cx);
    if let PopupMenuEvent::Chosen(id) = event {
        v.run_command(id, window, cx);
    }
}

pub(super) fn overlay(v: &EditorView, _: &mut Window, _: &mut App) -> Option<AnyElement> {
    let open = v.commands.menu.as_ref()?;
    let placed = anchored()
        .position(open.at)
        .snap_to_window()
        .child(open.menu.clone());
    Some(deferred(placed).with_priority(1).into_any_element())
}
