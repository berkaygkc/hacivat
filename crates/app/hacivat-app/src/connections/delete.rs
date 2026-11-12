//! Deleting a profile: the confirmation and the secrets it removes (NOTES surface 8, Task 18).
//!
//! An alertdialog (`DialogSpec`): "Delete {name}?", the body says the Keychain entry goes too,
//! scripts and history stay and it can't be undone. Cancel has the initial focus; Delete is
//! the Danger button (outline until hover or focus). Confirming runs `ConnectionsModel::delete`,
//! which cancels a connect in progress, forgets the session secrets and deletes every stored
//! secret slot, then toasts "Deleted {name}".

use std::rc::Rc;

use hacivat_l10n::{Text, ids};
use hacivat_proto::ProfileId;
use hacivat_ui::button::ButtonKind;
use hacivat_ui::dialog::{Dialog, DialogButton, DialogEvent, DialogSize, DialogSpec};
use hacivat_ui_core::gpui_surface::{App, Entity, Global, WeakEntity, Window};

use crate::connections::commands::{refuse_read_only, show_save_error, toast_deleted};
use crate::connections::model::ConnectionsModel;
use crate::l10n_global::{text_fn, tr};
use crate::shell::Workspace;

/// The open confirmation (tests read its focused button through it).
struct OpenConfirm(WeakEntity<Dialog>);

impl Global for OpenConfirm {}

/// The open delete confirmation, if any.
pub fn open_dialog(cx: &App) -> Option<Entity<Dialog>> {
    cx.try_global::<OpenConfirm>()?.0.upgrade()
}

/// The confirmation's buttons, in order.
const CANCEL: usize = 0;
const DELETE: usize = 1;

/// The confirmation for `id`'s name.
fn spec(name: String) -> DialogSpec {
    let button = |label, kind| DialogButton {
        label,
        kind,
        accesskey: None,
        english_accesskey: None,
    };
    DialogSpec {
        title: Rc::new(move |cx: &App| {
            tr(
                cx,
                ids::hv::CONN_DEL_TITLE,
                ids::hv::HvConnDelTitle {
                    name: Text(name.clone()),
                },
            )
        }),
        body: text_fn(ids::hv::CONN_DEL_BODY),
        buttons: vec![
            button(text_fn(ids::hv::CONN_DEL_CANCEL), ButtonKind::Default),
            button(text_fn(ids::hv::CONN_DEL_BTN), ButtonKind::Danger),
        ],
        cancel: CANCEL,
        // The initial focus is on Cancel (NOTES §8).
        primary: CANCEL,
        size: DialogSize::Standard,
    }
}

/// Asks before deleting `id` (`connection.delete`); a read-only store says why instead.
pub fn confirm(id: ProfileId, window: &mut Window, cx: &mut App) {
    if refuse_read_only(cx) {
        return;
    }
    let Some(name) = cx
        .try_global::<ConnectionsModel>()
        .and_then(|m| m.get(id))
        .map(|p| p.name.clone())
    else {
        return;
    };
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    let dialog = ws.update(cx, |ws, cx| ws.open_dialog(spec(name.clone()), window, cx));
    cx.set_global(OpenConfirm(dialog.downgrade()));
    cx.subscribe(&dialog, move |_, event: &DialogEvent, cx| {
        if *event == DialogEvent::Pressed(DELETE) {
            delete_now(id, name.clone(), cx);
        }
    })
    .detach();
}

/// Deletes `id` and toasts; a failure says why.
pub fn delete_now(id: ProfileId, name: String, cx: &mut App) {
    let task = ConnectionsModel::delete(cx, id);
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| match result {
            Ok(()) => toast_deleted(name, cx),
            Err(e) => show_save_error(&e, cx),
        });
    })
    .detach();
}

// ## m2a-t18-explorer: shots fixture (R28, P19)

/// The delete confirmation over a saved profile.
#[cfg(feature = "shots")]
pub fn confirm_fixture(window: &mut Window, cx: &mut App) {
    use hacivat_proto::EnvTag;
    use hacivat_session::profile::{Profile, ProfileDraft, ProfileEndpoint};
    let profile = Profile {
        name: "orders-prod".to_string(),
        environment: Some(EnvTag::Production),
        endpoint: ProfileEndpoint::Server {
            host: "db.acme.io".to_string(),
            port: None,
        },
        ..Profile::default()
    };
    let id = profile.id;
    ConnectionsModel::save(
        cx,
        ProfileDraft {
            is_new: true,
            ..ProfileDraft::edit(profile)
        },
    )
    .detach();
    confirm(id, window, cx);
}
