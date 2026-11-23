//! Saving a script tab (HV-SCRIPT-07, R-M4b-5, R-M4b-38): `file.save` and `file.save_all`,
//! the secret check first, then an atomic save that expects the file as it was read; a change
//! on disk asks "Overwrite?"; a failure keeps the tab dirty. Dirty is the tab's: the editor's
//! version against the version loaded, reloaded or saved.
//!
//! ## m4b-t01b-seams: `file.save` and `file.save_all` were registered disabled.
//!
//! ## m4b-t14-tabs
//! - **Only on ⌘S** (Q1): `file.save` (the script tab's context) and `file.save_all`; nothing
//!   saves a script on its own.
//! - **A preview is never saved** (T5 I4): Save is disabled on a T2 preview and says why.
//! - **The secret check** (R-M4b-17) runs first: `scan_secrets` over the text with the history
//!   redactor's spans (R-M4b-28). ## m4b-t17-secrets-ui: the scan runs off the UI thread (the
//!   tab's own scan of the same version when there is one) and a finding asks in Task 17's
//!   warning (`secret_dialog`); "Save anyway" is remembered for the tab's findings until the
//!   app quits.
//! - **Never over a change on disk** (R-M4b-16): the save expects the stamp the tab last read
//!   or saved; `ChangedOnDisk` asks "Overwrite?" (Overwrite / Cancel, Cancel focused) and only
//!   then saves expecting what is on disk now. After "Deleted on disk" a save writes the file
//!   again (no expectation).
//! - **The tab's environment goes to the save** (R-T5-6), read now.
//! - `SavedButPathChanged` is a save, with a notice; every other error keeps the tab dirty and
//!   says why; `Damaged` also keeps a recovery copy.

use std::rc::Rc;

use hacivat_l10n::ArgValue;
use hacivat_scripts::fileio::{SaveError, SaveText, SizeTier};
use hacivat_ui::actions::{ActionDef, ActionRegistry};
use hacivat_ui::button::ButtonKind;
use hacivat_ui::dialog::{DialogButton, DialogEvent, DialogSize, DialogSpec};
use hacivat_ui::text::TextFn;
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, Entity, Role, Window};

use crate::scripts::tabs::{self, ScriptDoc, display_name, msg};

/// What a save ended with (the unsaved prompt and Save all continue on `Saved`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOutcome {
    Saved,
    /// Cancelled at a question, or it failed: the tab stays dirty.
    NotSaved,
}

/// What runs after a save.
pub type Then = Box<dyn FnOnce(SaveOutcome, &mut App)>;

/// Registers `file.save` and `file.save_all` (before the stubs, so these definitions win).
pub(crate) fn register_commands(cx: &mut App) {
    let reg = cx.default_global::<ActionRegistry>();
    let _ = reg.register(ActionDef {
        id: "file.save",
        title_id: "hv-cmd-file-save",
        context: crate::scripts::SCRIPT_TAB_CONTEXT,
        run: Rc::new(|window, cx| {
            if let Some(doc) = active_doc(cx) {
                save(&doc, window, cx, None);
            }
        }),
        enabled: save_enabled,
    });
    let _ = reg.register(ActionDef {
        id: "file.save_all",
        title_id: "hv-cmd-file-save-all",
        context: "Workspace",
        run: Rc::new(|_, cx| save_all(cx)),
        enabled: any_dirty,
    });
}

/// The active tab's script doc.
fn active_doc(cx: &App) -> Option<Entity<ScriptDoc>> {
    let tab = cx
        .try_global::<crate::shell::workspace::ActiveTab>()
        .and_then(|a| a.0)?;
    tabs::doc_of_tab(tab, cx)
}

/// `file.save` is on for a script tab that was read and is not a preview.
fn save_enabled(cx: &App) -> bool {
    active_doc(cx).is_some_and(|d| {
        let d = d.read(cx);
        // ## m4b-t14-tabs fix round 1 (I6): off once the file left the connection.
        d.load == tabs::Load::Ready && d.tier != SizeTier::Preview && !d.unlisted()
    })
}

/// `file.save_all` is on while a script tab has unsaved edits.
fn any_dirty(cx: &App) -> bool {
    tabs::all_docs(cx).iter().any(|d| d.read(cx).is_dirty(cx))
}

/// Saves every dirty script tab, one after the other (a question pauses the rest).
pub fn save_all(cx: &mut App) {
    let dirty: Vec<Entity<ScriptDoc>> = tabs::all_docs(cx)
        .into_iter()
        .filter(|d| d.read(cx).is_dirty(cx))
        .collect();
    save_chain(dirty, cx);
}

fn save_chain(mut docs: Vec<Entity<ScriptDoc>>, cx: &mut App) {
    if docs.is_empty() {
        return;
    }
    let doc = docs.remove(0);
    crate::exec_ui::in_workspace_window(cx, move |_, window, cx| {
        save(
            &doc,
            window,
            cx,
            Some(Box::new(move |outcome, cx| {
                if outcome == SaveOutcome::Saved {
                    save_chain(docs, cx);
                }
            })),
        );
    });
}

/// Saves script tab `doc` (⌘S), then runs `then`.
pub fn save(doc: &Entity<ScriptDoc>, window: &mut Window, cx: &mut App, then: Option<Then>) {
    let finish = |then: Option<Then>, outcome, cx: &mut App| {
        if let Some(then) = then {
            then(outcome, cx);
        }
    };
    let (ready, preview, editor) = {
        let d = doc.read(cx);
        (
            d.load == tabs::Load::Ready,
            d.tier == SizeTier::Preview,
            d.editor().cloned(),
        )
    };
    let Some(editor) = editor.filter(|_| ready) else {
        return finish(then, SaveOutcome::NotSaved, cx);
    };
    if preview {
        let text = msg(cx, "hv-scripts-preview-no-save", Vec::new());
        info_toast(text, cx);
        return finish(then, SaveOutcome::NotSaved, cx);
    }
    // ## m4b-t14-tabs fix round 1 (I6, R-T14-4): a file no longer listed under the tab's
    // connection is never written back where it no longer belongs.
    if doc.read(cx).unlisted() {
        info_toast(msg(cx, "hv-scripts-unlisted-off", Vec::new()), cx);
        return finish(then, SaveOutcome::NotSaved, cx);
    }
    let snap = editor.read(cx).snapshot();
    let (text, version) = (snap.buffer.to_string(), snap.version);
    let expect = {
        let d = doc.read(cx);
        if d.deleted { None } else { Some(d.stamp) }
    };
    // ## m4b-t17-secrets-ui: the secret check (R-M4b-17) reads the tab's scan of exactly this
    // text, or scans it now off the UI thread (`secret_marks::scan_off_ui`), then asks
    // (`secret_dialog`) unless nothing new was found.
    let known = doc.read(cx).secrets.at(version).cloned();
    match known {
        Some(scan) => checked(doc, scan, text, version, expect, window, cx, then),
        None => {
            let scan = crate::scripts::secret_marks::scan_off_ui(version, text.clone(), cx);
            let doc = doc.clone();
            cx.spawn(async move |cx| {
                let scan = scan.await;
                cx.update(|cx| {
                    crate::exec_ui::in_workspace_window(cx, move |_, window, cx| {
                        checked(&doc, scan, text, version, expect, window, cx, then);
                    });
                });
            })
            .detach();
        }
    }
}

// ## m4b-t17-secrets-ui
/// The secret check's answer for `text` (the editor at `version`): written at once when
/// nothing asks, else after "Save anyway" (which remembers the findings for the tab).
#[allow(clippy::too_many_arguments)]
fn checked(
    doc: &Entity<ScriptDoc>,
    scan: crate::scripts::secret_marks::Scan,
    text: String,
    version: u64,
    expect: Option<Option<hacivat_scripts::DiskStamp>>,
    window: &mut Window,
    cx: &mut App,
    then: Option<Then>,
) {
    use crate::scripts::secret_dialog::{self, SaveDecision};
    if secret_dialog::decide(&scan, doc.read(cx)) == Some(SaveDecision::Save) {
        return write(doc, text, version, expect, window, cx, then);
    }
    let target = doc.clone();
    let asked = scan.clone();
    secret_dialog::confirm(
        doc,
        &asked,
        window,
        cx,
        Box::new(move |answer, window, cx| match answer {
            SaveDecision::Cancel => {
                if let Some(then) = then {
                    then(SaveOutcome::NotSaved, cx);
                }
            }
            SaveDecision::Save => {
                target.update(cx, |d, _| d.secret_ok.extend(scan.prints()));
                write(&target, text, version, expect, window, cx, then);
            }
        }),
    );
}

/// Writes `text` (the editor at `version`) expecting the file at `expect` (`None`: whatever
/// is there; `Some(None)`: no file; `Some(Some(s))`: the file at `s`).
fn write(
    doc: &Entity<ScriptDoc>,
    text: String,
    version: u64,
    expect: Option<Option<hacivat_scripts::DiskStamp>>,
    window: &mut Window,
    cx: &mut App,
    then: Option<Then>,
) {
    let _ = window;
    let (Some(tab), Some(service)) = (doc.read(cx).spec(cx), crate::scripts::open::service(cx))
    else {
        save_failed(doc, "missing", cx);
        if let Some(then) = then {
            then(SaveOutcome::NotSaved, cx);
        }
        return;
    };
    let env = doc.read(cx).env(cx);
    let meta = doc.read(cx).meta;
    let expect_stamp = expect.flatten();
    let (lib, rel) = (tab.library, tab.rel.clone());
    let body = SaveText {
        text: text.clone(),
        meta,
    };
    let saved = tabs::on_core(cx, async move {
        service.save(lib, &rel, env, body, expect_stamp).await
    });
    let doc = doc.clone();
    cx.spawn(async move |cx| {
        let saved = saved
            .await
            .unwrap_or(Err(SaveError::Io(std::io::ErrorKind::Interrupted)));
        cx.update(|cx| saved_done(&doc, saved, text, version, cx, then));
    })
    .detach();
}

fn saved_done(
    doc: &Entity<ScriptDoc>,
    saved: Result<hacivat_scripts::DiskStamp, SaveError>,
    text: String,
    version: u64,
    cx: &mut App,
    then: Option<Then>,
) {
    let file = tabs::file_label(doc.read(cx), cx);
    let outcome = match saved {
        Ok(stamp) | Err(SaveError::SavedButPathChanged(stamp)) => {
            let changed = matches!(saved, Err(SaveError::SavedButPathChanged(_)));
            doc.update(cx, |d, cx| {
                d.deleted = false;
                d.clean_at(version, Some(stamp), cx);
                d.discard_recovered(cx);
                d.hide(|n| matches!(n, tabs::Notice::Changed), cx);
                let text = msg(
                    cx,
                    "hv-scripts-tab-saved",
                    vec![("file", ArgValue::Text(file.clone()))],
                );
                d.say(text, cx);
            });
            // The file's tabs under other connections see the change (R-M4b-4).
            tabs::saved_elsewhere(doc, stamp, cx);
            if changed {
                let text = msg(
                    cx,
                    "hv-scripts-saved-path-changed",
                    vec![("file", ArgValue::Text(file))],
                );
                info_toast(text, cx);
            }
            SaveOutcome::Saved
        }
        Err(SaveError::ChangedOnDisk(on_disk)) => {
            // Never over a change on disk without an answer (R-M4b-16).
            let doc = doc.clone();
            crate::exec_ui::in_workspace_window(cx, move |_, window, cx| {
                confirm(
                    window,
                    cx,
                    ConfirmSpec {
                        title: crate::l10n_global::text_fn_id("hv-scripts-disk-overwrite-title"),
                        body: crate::l10n_global::text_fn_id("hv-scripts-disk-overwrite-body"),
                        yes: crate::l10n_global::text_fn_id("hv-scripts-disk-overwrite"),
                        yes_kind: ButtonKind::Danger,
                    },
                    Box::new(move |yes, window, cx| {
                        if yes {
                            write(&doc, text, version, Some(Some(on_disk)), window, cx, then);
                        } else if let Some(then) = then {
                            then(SaveOutcome::NotSaved, cx);
                        }
                    }),
                );
            });
            return;
        }
        Err(SaveError::DeletedOnDisk) => {
            doc.update(cx, |d, cx| {
                d.deleted = true;
                d.show(tabs::Notice::Deleted, cx);
            });
            SaveOutcome::NotSaved
        }
        Err(SaveError::Unencodable { first_offset }) => {
            let encoding = doc.read(cx).meta.encoding.name().to_string();
            let ch = text[first_offset..]
                .chars()
                .next()
                .map(|c| display_name(&c.to_string()))
                .unwrap_or_default();
            let line = text[..first_offset].matches('\n').count() as u64 + 1;
            let title = msg(
                cx,
                "hv-scripts-enc-unencodable-title",
                vec![("file", ArgValue::Text(file.clone()))],
            );
            let body = msg(
                cx,
                "hv-scripts-enc-unencodable-body",
                vec![
                    ("char", ArgValue::Text(ch)),
                    ("line", ArgValue::Count(line)),
                    ("encoding", ArgValue::Text(encoding)),
                ],
            );
            error_toast(title, body, cx);
            SaveOutcome::NotSaved
        }
        Err(SaveError::Damaged) => {
            save_failed(doc, "damaged", cx);
            SaveOutcome::NotSaved
        }
        Err(SaveError::PathChanged | SaveError::PathRefused) => {
            save_failed(doc, "path", cx);
            SaveOutcome::NotSaved
        }
        Err(SaveError::OutOfScope) => {
            save_failed(doc, "scope", cx);
            SaveOutcome::NotSaved
        }
        Err(SaveError::Io(std::io::ErrorKind::NotFound)) => {
            save_failed(doc, "missing", cx);
            SaveOutcome::NotSaved
        }
        Err(SaveError::PreviewOnly) => {
            info_toast(msg(cx, "hv-scripts-preview-no-save", Vec::new()), cx);
            SaveOutcome::NotSaved
        }
        Err(e) => {
            tracing::warn!(?e, "scripts: a save failed");
            save_failed(doc, "other", cx);
            SaveOutcome::NotSaved
        }
    };
    if let Some(then) = then {
        then(outcome, cx);
    }
}

/// The error toast of a failed save (the tab stays dirty).
fn save_failed(doc: &Entity<ScriptDoc>, reason: &'static str, cx: &mut App) {
    let file = tabs::file_label(doc.read(cx), cx);
    let conn = tabs::conn_label(doc.read(cx), cx);
    let title = msg(
        cx,
        "hv-scripts-save-failed",
        vec![("file", ArgValue::Text(file))],
    );
    let body = msg(
        cx,
        "hv-scripts-save-failed-body",
        vec![
            ("reason", ArgValue::Text(reason.to_string())),
            ("conn", ArgValue::Text(conn)),
        ],
    );
    error_toast(title, body, cx);
}

fn info_toast(text: hacivat_ui_core::gpui_surface::SharedString, cx: &mut App) {
    ToastQueue::push(
        cx,
        Toast::new(ToastKind::Info, Rc::new(move |_: &App| text.clone())),
    );
}

fn error_toast(
    title: hacivat_ui_core::gpui_surface::SharedString,
    body: hacivat_ui_core::gpui_surface::SharedString,
    cx: &mut App,
) {
    let mut toast = Toast::new(ToastKind::Error, Rc::new(move |_: &App| body.clone()));
    toast.title = Some(Rc::new(move |_: &App| title.clone()));
    toast.wide = true;
    ToastQueue::push(cx, toast);
}

// ---- the confirmations -----------------------------------------------------------------------------

/// A two-button alertdialog: `yes` and Cancel (Primary, focused; Escape answers Cancel).
pub(crate) struct ConfirmSpec {
    pub title: TextFn,
    pub body: TextFn,
    pub yes: TextFn,
    pub yes_kind: ButtonKind,
}

/// What a confirmation's answer runs.
pub(crate) type Answer = Box<dyn FnOnce(bool, &mut Window, &mut App)>;

/// Opens `spec` in the workspace of `window`; `answer(true)` on its first button.
pub(crate) fn confirm(window: &mut Window, cx: &mut App, spec: ConfirmSpec, answer: Answer) {
    let Some(ws) = window.root::<crate::shell::Workspace>().flatten() else {
        answer(false, window, cx);
        return;
    };
    let button = |label: TextFn, kind: ButtonKind| DialogButton {
        label,
        kind,
        accesskey: None,
        english_accesskey: None,
    };
    let dialog_spec = DialogSpec {
        title: spec.title,
        body: spec.body,
        buttons: vec![
            button(spec.yes, spec.yes_kind),
            button(
                crate::l10n_global::text_fn_id("hv-tx-cancel"),
                ButtonKind::Primary,
            ),
        ],
        cancel: 1,
        primary: 1,
        size: DialogSize::Standard,
    };
    let dialog = ws.update(cx, |ws, cx| ws.open_dialog(dialog_spec, window, cx));
    dialog.update(cx, |d, _| d.set_role(Role::AlertDialog));
    let answer = std::cell::RefCell::new(Some(answer));
    let ws_weak = ws.downgrade();
    cx.subscribe(&dialog, move |_, event: &DialogEvent, cx| {
        let yes = matches!(event, DialogEvent::Pressed(0));
        if let Some(answer) = answer.borrow_mut().take() {
            let _ = &ws_weak;
            crate::exec_ui::in_workspace_window(cx, move |_, window, cx| answer(yes, window, cx));
        }
    })
    .detach();
}
