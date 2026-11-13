//! The connection commands: new, paste URL, connect, edit, duplicate, copy URL, delete, context menu (R20, Task 18).
//!
//! Every command is in the palette; the keymap binds the explorer ones in the `Explorer`
//! context (Task 1's data) and only `connection.new` is in the native File menu (P18). The
//! explorer commands act on the selected profile. The capsule's Retry and "unencrypted"
//! segments have commands too (their keyboard path), and `workspace.focus_notification` moves
//! the focus to the newest sticky toast.
//!
//! A write the store refuses shows why: a read-only store (P29) explains itself with
//! `hv-conn-store-read-only`, any other failure with a toast.

use std::rc::Rc;
use std::sync::Arc;

use hacivat_l10n::{Text, ids};
use hacivat_proto::ProfileId;
use hacivat_session::LiveState;
use hacivat_session::profile::{Profile, copy_url};
use hacivat_ui::actions::{ActionDef, ActionFn, ActionRegistry};
use hacivat_ui::toast::{self, Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, ClipboardItem, Window};

use crate::connections::capsule;
use crate::connections::delete;
use crate::connections::dialog::{DialogMode, open_connection_dialog};
use crate::connections::explorer;
use crate::connections::import;
use crate::connections::live::note_started;
use crate::connections::model::{ConnectionsModel, SaveError};
use crate::l10n_global::{text_fn, tr};

/// The key context of the explorer commands.
const EXPLORER: &str = "Explorer";
/// The key context of the workspace-wide commands.
const WORKSPACE: &str = "Workspace";

fn has_model(cx: &App) -> bool {
    cx.has_global::<ConnectionsModel>()
}

fn has_selection(cx: &App) -> bool {
    selected(cx).is_some()
}

/// The store can be written (P29): the mutating commands are disabled otherwise.
fn writable(cx: &App) -> bool {
    cx.try_global::<ConnectionsModel>()
        .is_some_and(|m| !m.store_read_only())
}

fn can_add(cx: &App) -> bool {
    has_model(cx) && writable(cx)
}

fn can_change_selection(cx: &App) -> bool {
    has_selection(cx) && writable(cx)
}

fn always(_: &App) -> bool {
    true
}

/// The selected profile, while its row is visible (a filtered-out row is never acted on).
fn selected(cx: &App) -> Option<Arc<Profile>> {
    let m = cx.try_global::<ConnectionsModel>()?;
    let id = m.selected().filter(|id| explorer::is_visible(cx, *id))?;
    m.get(id)
}

fn on_selected(f: fn(Arc<Profile>, &mut Window, &mut App)) -> ActionFn {
    Rc::new(move |window, cx| {
        if let Some(p) = selected(cx) {
            f(p, window, cx);
        }
    })
}

/// Registers the connection commands (one line in `connections::install`). An ID already
/// registered keeps its first definition.
pub fn register(cx: &mut App) {
    type Def = (
        &'static str,
        &'static str,
        &'static str,
        ActionFn,
        fn(&App) -> bool,
    );
    let defs: Vec<Def> = vec![
        (
            "connection.new",
            ids::hv::CMD_CONNECTION_NEW.id,
            WORKSPACE,
            Rc::new(|window, cx| open_connection_dialog(DialogMode::New, window, cx)),
            can_add,
        ),
        (
            "connection.paste_url",
            ids::hv::CMD_CONNECTION_PASTE_URL.id,
            WORKSPACE,
            Rc::new(paste_url),
            can_add,
        ),
        (
            "connection.toggle_connect",
            ids::hv::CMD_CONNECTION_TOGGLE_CONNECT.id,
            EXPLORER,
            on_selected(|p, _, cx| toggle_connect(p.id, cx)),
            has_selection,
        ),
        (
            "connection.edit",
            ids::hv::CMD_CONNECTION_EDIT.id,
            EXPLORER,
            on_selected(|p, window, cx| open_connection_dialog(DialogMode::Edit(p.id), window, cx)),
            can_change_selection,
        ),
        (
            "connection.duplicate",
            ids::hv::CMD_CONNECTION_DUPLICATE.id,
            EXPLORER,
            on_selected(|p, _, cx| duplicate(p.id, cx)),
            can_change_selection,
        ),
        (
            "connection.copy_url",
            ids::hv::CMD_CONNECTION_COPY_URL.id,
            EXPLORER,
            on_selected(|p, _, cx| copy_as_url(&p, cx)),
            has_selection,
        ),
        (
            "connection.delete",
            ids::hv::CMD_CONNECTION_DELETE.id,
            EXPLORER,
            on_selected(|p, window, cx| delete::confirm(p.id, window, cx)),
            can_change_selection,
        ),
        (
            "connection.context_menu",
            ids::hv::CMD_CONNECTION_CONTEXT_MENU.id,
            EXPLORER,
            Rc::new(|window, cx| explorer::open_context_menu(None, window, cx)),
            has_selection,
        ),
        (
            "connection.import_pg_service",
            ids::hv::CMD_CONNECTION_IMPORT_PG_SERVICE.id,
            WORKSPACE,
            Rc::new(import::open),
            can_add,
        ),
        (
            "connection.retry",
            ids::hv::CMD_CONNECTION_RETRY.id,
            WORKSPACE,
            Rc::new(|_, cx| capsule::retry(cx)),
            capsule::can_retry,
        ),
        (
            "connection.explain_unencrypted",
            ids::hv::CMD_CONNECTION_EXPLAIN_UNENCRYPTED.id,
            WORKSPACE,
            Rc::new(capsule::explain_unencrypted_from_keyboard),
            capsule::has_unencrypted,
        ),
        (
            "workspace.focus_notification",
            ids::hv::CMD_WORKSPACE_FOCUS_NOTIFICATION.id,
            WORKSPACE,
            Rc::new(|window, cx| {
                toast::focus_newest_sticky(window, cx);
            }),
            always,
        ),
    ];
    let reg = cx.default_global::<ActionRegistry>();
    for (id, title_id, context, run, enabled) in defs {
        let _ = reg.register(ActionDef {
            id,
            title_id,
            context,
            run,
            enabled,
        });
    }
}

/// A new connection prefilled from the clipboard's text (`connection.paste_url`).
fn paste_url(window: &mut Window, cx: &mut App) {
    let text = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default();
    open_connection_dialog(DialogMode::NewFromText(text), window, cx);
}

/// Connects `id`, or disconnects (cancels) it when it is connected (or connecting).
pub fn toggle_connect(id: ProfileId, cx: &mut App) {
    // The manager's state, not the mirror (which may lag by one change).
    let state = cx.global::<ConnectionsModel>().manager().state(id);
    match state {
        LiveState::Connected(_) => ConnectionsModel::disconnect(cx, id),
        LiveState::Connecting(_) => ConnectionsModel::cancel_connect(cx, id),
        // ## m2b-t01-interfaces: stopping a reconnect is a disconnect (Task 22 refines it).
        LiveState::Reconnecting { .. } => ConnectionsModel::disconnect(cx, id),
        LiveState::Disconnected | LiveState::Failed(_) | LiveState::Lost { .. } => {
            note_started(cx, id);
            ConnectionsModel::connect(cx, id, None);
        }
    }
}

/// Shows why a profile write failed (P29 and the T12 note): the read-only store explains
/// itself; a deleted profile needs no word.
pub fn show_save_error(error: &SaveError, cx: &mut App) {
    let id = match error {
        SaveError::StoreReadOnly => ids::hv::CONN_STORE_READ_ONLY,
        // ## m2a-t21-shots: the dialog's and the fixes' save-error texts.
        SaveError::Io(_) => ids::hv::CONN_FIX_SAVE_FAILED,
        SaveError::Secret(_) => ids::hv::CONN_DLG_SAVE_SECRET,
        SaveError::NotFound => return,
    };
    let kind = if *error == SaveError::StoreReadOnly {
        ToastKind::Info
    } else {
        ToastKind::Error
    };
    ToastQueue::push(cx, Toast::new(kind, text_fn(id)));
}

/// Whether the store is read-only; if so, says why (P29).
pub fn refuse_read_only(cx: &mut App) -> bool {
    let read_only = cx
        .try_global::<ConnectionsModel>()
        .is_some_and(ConnectionsModel::store_read_only);
    if read_only {
        show_save_error(&SaveError::StoreReadOnly, cx);
    }
    read_only
}

fn duplicate(id: ProfileId, cx: &mut App) {
    if refuse_read_only(cx) {
        return;
    }
    let task = ConnectionsModel::duplicate(cx, id);
    cx.spawn(async move |cx| {
        if let Err(e) = task.await {
            cx.update(|cx| show_save_error(&e, cx));
        }
    })
    .detach();
}

/// Copy as URL: the URL without the password on the clipboard, and a toast that says so.
fn copy_as_url(p: &Profile, cx: &mut App) {
    let Ok(copied) = copy_url(p) else {
        return;
    };
    cx.write_to_clipboard(ClipboardItem::new_string(copied.url));
    let mut toast = Toast::new(
        ToastKind::Info,
        if copied.omitted_password {
            text_fn(ids::hv::CONN_TOAST_NOPW)
        } else {
            Rc::new(|_: &App| Default::default())
        },
    );
    toast.title = Some(text_fn(ids::hv::CONN_TOAST_COPIED));
    ToastQueue::push(cx, toast);
}

/// The toast after a delete.
pub fn toast_deleted(name: String, cx: &mut App) {
    let text = Rc::new(move |cx: &App| {
        tr(
            cx,
            ids::hv::CONN_TOAST_DELETED,
            ids::hv::HvConnToastDeleted {
                name: Text(name.clone()),
            },
        )
    });
    ToastQueue::push(cx, Toast::new(ToastKind::Info, text));
}
