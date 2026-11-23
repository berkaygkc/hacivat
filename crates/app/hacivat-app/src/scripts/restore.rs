//! Script tabs across a restart (R-M4b-22, R-M4b-29; NOTES surface 12): `workspace.json`
//! version 2 saves each script tab as its own tab kind; on restore a missing library or file
//! opens the tab with a notice (and its recovery copy), a deleted profile leaves an unbound tab,
//! and the capsule always reads the profile's current environment.
//!
//! ## m4b-t01b-seams: the tab kind and the version bump are in the shell
//! (`shell::workspace`, `SavedKind::Script`).
//!
//! ## m4b-t14-tabs
//! - **A bad entry drops only its tab** (T1b m1): every field of a saved script tab is read
//!   leniently ([`lenient`]: a value of the wrong type reads as missing), so a hand-edited or
//!   damaged entry closes that tab and the rest of the layout restores.
//! - **One tab per (file, connection)** (T1b m2): a second saved tab with the same binding is
//!   dropped ([`Dedup`]).
//! - **The binding is restored as saved, never re-resolved**: the profile by its ID only (a
//!   profile with the same name is another connection), the database as stored (R-T1b-1). A
//!   profile that no longer exists leaves the tab showing "The connection this tab was opened
//!   under was deleted" and running nothing (T1b m6); the capsule reads the profile's
//!   environment as it is now.
//! - **Recovery at launch** ([`recover_at_launch`], NOTES surface 12): the copies a crashed or
//!   quit instance left (`recovery::take_all`) open as dirty tabs bound as they were, with the
//!   "Recovered unsaved changes" notice, and a launch toast lists them. A copy whose writer may
//!   still run (`liveness_unknown`, R-T5-5) says so and is kept until the user saves or
//!   discards it explicitly.

use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use hacivat_l10n::ArgValue;
use hacivat_scripts::recovery::{self, RecoveryEntry};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::App;
use serde::Deserialize as _;

use crate::scripts::open;
use crate::scripts::tabs::{self, PendingRecovered, ScriptTab, ScriptTabId, display_name, msg};

/// Reads any JSON value as an optional string: a value of another type is `None` (a bad
/// field closes its tab, never the layout; T1b m1).
pub(crate) fn lenient<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(s) => Some(s),
        _ => None,
    })
}

/// The handles a restore already gave a tab (one tab per binding).
#[derive(Debug, Default)]
pub(crate) struct Dedup(HashSet<ScriptTabId>);

impl Dedup {
    /// `true` the first time `id` is seen.
    pub(crate) fn first(&mut self, id: ScriptTabId) -> bool {
        self.0.insert(id)
    }
}

/// How long the launch recovery waits for the library service to read its registry.
const REGISTRY_WAIT: Duration = Duration::from_secs(10);
const REGISTRY_POLL: Duration = Duration::from_millis(100);

/// Offers the recovery copies left by an instance that is gone (NOTES surface 12): each opens
/// as a dirty tab bound as it was. Runs once, after the workspace opened; nothing happens
/// unless the app has script libraries.
pub fn recover_at_launch(cx: &mut App) {
    let Some(m) = cx.try_global::<crate::connections::ConnectionsModel>() else {
        return;
    };
    let Some(paths) = cx
        .try_global::<crate::shell::AppState>()
        .map(|s| s.paths.clone())
    else {
        return;
    };
    let handle = m.runtime();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let registry = paths.libraries_toml();
    handle.spawn_blocking(move || {
        let _ = tx.send(registry.exists());
    });
    cx.spawn(async move |cx| {
        if rx.await != Ok(true) {
            return;
        }
        let Some(rt) = cx.update(open::runtime) else {
            return;
        };
        let found = {
            let rt = rt.clone();
            let (tx, rx) = tokio::sync::oneshot::channel();
            rt.handle.clone().spawn_blocking(move || {
                // The service reads its registry on its own actor: wait for it, a bounded
                // while, on the blocking pool (never the UI thread).
                let mut waited = Duration::ZERO;
                while rt.service.snapshot().state == hacivat_scripts::LibraryState::Scanning
                    && waited < REGISTRY_WAIT
                {
                    std::thread::sleep(REGISTRY_POLL);
                    waited += REGISTRY_POLL;
                }
                let ids: Vec<hacivat_scripts::LibraryId> = rt
                    .service
                    .snapshot()
                    .libraries
                    .iter()
                    .map(|l| l.id)
                    .collect();
                let Some(instance) = rt.instance.get_or_init(|| {
                    recovery::Instance::start(&rt.paths)
                        .inspect_err(|e| tracing::warn!(?e, "scripts: no recovery instance"))
                        .ok()
                }) else {
                    let _ = tx.send(Vec::new());
                    return;
                };
                let all: Vec<RecoveryEntry> = ids
                    .into_iter()
                    .flat_map(|id| recovery::take_all(&rt.paths, instance, id))
                    .collect();
                let _ = tx.send(all);
            });
            rx.await.unwrap_or_default()
        };
        if found.is_empty() {
            return;
        }
        cx.update(|cx| offer(found, cx));
    })
    .detach();
}

/// Opens a dirty tab for each recovered copy and says so once.
fn offer(found: Vec<RecoveryEntry>, cx: &mut App) {
    let mut items = Vec::new();
    for entry in found {
        let tab = ScriptTab {
            library: entry.key.library,
            rel: entry.key.rel.clone(),
            profile: entry.key.profile,
            database: entry.key.database.clone(),
        };
        let file = display_name(crate::scripts::file_name(&tab.rel));
        let conn = tab
            .profile
            .and_then(|p| open::profile_of(p, cx))
            .map(|p| display_name(&p.name));
        items.push((file, conn));
        let id = tabs::intern(tab, cx);
        // ## m4b-t14-tabs fix round 2 (m-fr1-7): its first read waits for the listing.
        tabs::mark_restored(id, cx);
        // ## m4b-t14-tabs fix round 3 (m-fr2-1): a SQLite tab keeps the file its copy was
        // written on (Run then refuses, with its notice, if the profile was repointed since);
        // a tab of the restored layout already knows it.
        if let Some(file) = entry.copy.db_file.clone()
            && tabs::sqlite_file(id, cx).is_none()
        {
            tabs::set_sqlite_file(
                id,
                tabs::SqliteFile {
                    path: file.path,
                    id: file.id,
                },
                cx,
            );
        }
        // ## m4b-t14-tabs fix round 1 (I2): a tab already in the restored layout (the usual
        // crash: the dirty tab was open) takes its copy, read or not.
        if let Some(doc) = tabs::doc_of_script(id, cx) {
            doc.update(cx, |d, cx| d.apply_recovered(entry, cx));
            crate::exec_ui::in_workspace_window(cx, move |ws, _, cx| {
                ws.update(cx, |ws, cx| ws.open_script_tab(id, cx));
            });
            continue;
        }
        // Two copies of one binding (two instances): the newest wins, the other stays offered
        // for a later launch.
        let pending = &mut cx.default_global::<PendingRecovered>().0;
        let newer = pending
            .get(&id)
            .is_none_or(|old| old.saved_at < entry.saved_at);
        if newer {
            pending.insert(id, entry);
        }
        crate::exec_ui::in_workspace_window(cx, move |ws, _, cx| {
            ws.update(cx, |ws, cx| ws.open_script_tab(id, cx));
        });
    }
    let n = items.len() as u64;
    let bullets: Vec<hacivat_ui::text::TextFn> = items
        .into_iter()
        .map(|(file, conn)| -> hacivat_ui::text::TextFn {
            Rc::new(move |cx: &App| match &conn {
                Some(conn) => msg(
                    cx,
                    "hv-scripts-recover-item",
                    vec![
                        ("file", ArgValue::Text(file.clone())),
                        ("conn", ArgValue::Text(conn.clone())),
                    ],
                ),
                None => msg(
                    cx,
                    "hv-scripts-recover-item-unbound",
                    vec![("file", ArgValue::Text(file.clone()))],
                ),
            })
        })
        .collect();
    let mut toast = Toast::new(
        ToastKind::Info,
        crate::l10n_global::text_fn_id("hv-scripts-recover-body"),
    );
    toast.title = Some(Rc::new(move |cx: &App| {
        msg(
            cx,
            "hv-scripts-recover-title",
            vec![("n", ArgValue::Count(n))],
        )
    }));
    toast.bullets = bullets;
    toast.sticky = true;
    toast.wide = true;
    ToastQueue::push(cx, toast);
}
