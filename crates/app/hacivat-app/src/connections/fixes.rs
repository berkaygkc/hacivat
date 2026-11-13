//! Quick fixes the error surfaces offer, and what each does (HV-CONN-03, Task 20).
//!
//! The fixes are the error's own list (`ConnectError::fixes`: the registry's `fixes_for`, which
//! the pipeline narrows where a fix cannot help, Task 14), in its button order; an error that
//! carries none falls back to the registry's. `UseDatabase` becomes one button per suggested
//! database (at most 3); with no suggestion (SQLite, or no list) it offers nothing.
//!
//! M2b (Task 19): "Review certificate…" (only with a certificate to review) and "Import CA…"
//! replace M2a's interim "Encrypt without verifying" (B6). `ReviewHostKey` (Task 21) shows in
//! the error dialog only. "Connect through SSH…" (Task 24) opens the connection's SSH section;
//! a connection that already goes through SSH is not offered it.

use std::sync::Arc;

use hacivat_l10n::{NoArgs, Text, ids};
use hacivat_proto::{FixId, ProfileId};
use hacivat_session::pipeline::{ConnectError, TrailStep};
use hacivat_ui::button::ButtonKind;
use hacivat_ui_core::gpui_surface::{App, SharedString, Window};

use crate::connections::{cert_dialog, dialog, hostkey_prompt};
use crate::l10n_global::tr;

/// R11: where "Supported versions" leads (a placeholder URL, an open item for the PO).
pub const SUPPORTED_VERSIONS_URL: &str = "https://hacivat.dev/docs/engines";

/// The most "Use …" buttons an unknown database offers.
pub const MAX_DATABASE_FIXES: usize = 3;

/// The most server rejections of a password in one connect gesture (P8).
pub const MAX_REJECTIONS: u8 = 3;

/// A quick fix as the UI runs it (HV-CONN-03).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FixAction {
    EditConnection,
    TryAgain,
    ReenterPassword,
    UseDatabase(String),
    SupportedVersions,
    ConnectAnyway,
    // ## m2b-t01-interfaces: M2b flows, run by [`run_m2b_fix`]. Their owners add the labels
    // and map their `FixId`s in `fix_actions` (Tasks 19, 20, 21); until then no button shows.
    ReviewCertificate,
    ImportCa,
    ConnectThroughSsh,
    ReviewHostKey,
}

// ## m2b-t01-interfaces
/// Runs an M2b quick fix for `profile`'s failed connect `error` (Q9); `false` for any other
/// fix (the surfaces run those themselves).
pub fn run_m2b_fix(
    action: &FixAction,
    error: &Arc<ConnectError>,
    profile: ProfileId,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    match action {
        // ## m2b-t19-tls: the review builds its own change (Q26); Import CA picks the file.
        FixAction::ReviewCertificate => cert_dialog::review(error.clone(), profile, window, cx),
        FixAction::ImportCa => cert_dialog::import_ca(profile, cx),
        FixAction::ConnectThroughSsh => dialog::ssh::open_with_ssh(profile, window, cx),
        FixAction::ReviewHostKey => {
            if let Some(m) = error.host_key.clone() {
                hostkey_prompt::review_changed(m, profile, window, cx);
            }
        }
        _ => return false,
    }
    true
}

impl FixAction {
    /// Whether the fix changes the saved profile (and so needs a writable connections file).
    pub fn edits_profile(&self) -> bool {
        // ## m2b-t19-tls: a pin and a CA file are saved with the profile.
        matches!(
            self,
            FixAction::UseDatabase(_)
                | FixAction::ConnectAnyway
                | FixAction::ReviewCertificate
                | FixAction::ImportCa
        )
    }

    /// Whether the fix gives up a safety check (an unsupported server). Such a fix never has
    /// the initial focus: one reflexive Enter must not run it.
    pub fn weakens_safety(&self) -> bool {
        matches!(self, FixAction::ConnectAnyway)
    }

    /// Whether the fix starts a new connect gesture (controller ruling, fix round 1): every
    /// fix that connects except the password re-entry, which continues the gesture's count.
    pub fn starts_new_gesture(&self) -> bool {
        matches!(self, FixAction::TryAgain) || self.edits_profile()
    }
}

/// One fix button: its element ID (`fix-…`), action, label and kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixButton {
    pub id: SharedString,
    pub action: FixAction,
    pub label: SharedString,
    pub kind: ButtonKind,
    pub disabled: bool,
}

/// The fixes `error` offers, in its order (the registry's when it carries none). `UseDatabase`
/// expands into the first [`MAX_DATABASE_FIXES`] suggestions; `ReenterPassword` is left out
/// once the gesture's `rejections` reached [`MAX_REJECTIONS`]; `EditConnection` is left out
/// when `with_edit` is false (the inline strip sits on the form itself, as in the mock);
/// `ReviewCertificate` needs a certificate to review (NOTES M2b ruling 10).
pub fn fix_actions(error: &ConnectError, rejections: u8, with_edit: bool) -> Vec<FixAction> {
    let mut out = Vec::new();
    // ## m2b-t19-tls: the pipeline narrows the registry's list where a fix cannot help (an
    // expired certificate offers neither a pin nor a CA; Task 14 review M1).
    let fixes = if error.fixes.is_empty() {
        hacivat_driver::errors::fixes_for(error.class).to_vec()
    } else {
        error.fixes.clone()
    };
    // ## m2b-t24-shots (follow-up, R5): a connect whose trail starts with the tunnel already
    // goes through SSH.
    let through_ssh = error
        .trail
        .first()
        .is_some_and(|s| s.step == TrailStep::Ssh);
    for fix in fixes {
        match fix {
            FixId::EditConnection if with_edit => out.push(FixAction::EditConnection),
            FixId::TryAgain => out.push(FixAction::TryAgain),
            FixId::ReenterPassword if rejections < MAX_REJECTIONS => {
                out.push(FixAction::ReenterPassword);
            }
            FixId::UseDatabase => out.extend(
                error
                    .suggestions
                    .iter()
                    .take(MAX_DATABASE_FIXES)
                    .map(|db| FixAction::UseDatabase(db.clone())),
            ),
            FixId::SupportedVersions => out.push(FixAction::SupportedVersions),
            FixId::ConnectAnyway => out.push(FixAction::ConnectAnyway),
            // ## m2b-t19-tls
            FixId::ReviewCertificate if error.certificate.is_some() => {
                out.push(FixAction::ReviewCertificate);
            }
            FixId::ImportCa => out.push(FixAction::ImportCa),
            // ## m2b-t21-prompts: the review opens over the error dialog only (the inline test
            // strip has no saved connection to reconnect); it needs the changed key.
            FixId::ReviewHostKey if with_edit && error.host_key.is_some() => {
                out.push(FixAction::ReviewHostKey);
            }
            // ## m2b-t24-shots (fix round 1, I4): the connection's SSH section, from the error
            // dialog (`open_with_ssh`) and inline (the dialog's own section).
            FixId::ConnectThroughSsh if !through_ssh => out.push(FixAction::ConnectThroughSsh),
            // M2b flows of other tasks (SSH), M2a's retired `EncryptWithoutVerifying` (B6) and
            // the edit button inline.
            _ => {}
        }
    }
    out
}

/// The buttons for `actions`: IDs `fix-edit`, `fix-retry`, `fix-reenter`, `fix-use-<i>`,
/// `fix-versions`, `fix-anyway`, `fix-review-cert`, `fix-import-ca`. The first of re-enter, try
/// again, the first database or Review certificate is the primary (the mocks); "Connect
/// anyway" is a ghost (PO Q3); the rest are default buttons.
pub fn fix_buttons(actions: &[FixAction], cx: &App) -> Vec<FixButton> {
    let mut db_index = 0;
    let mut primary_taken = false;
    actions
        .iter()
        .map(|action| {
            let (id, label): (SharedString, SharedString) = match action {
                FixAction::EditConnection => {
                    ("fix-edit".into(), tr(cx, ids::hv::CONN_FIX_EDIT, NoArgs))
                }
                FixAction::TryAgain => {
                    ("fix-retry".into(), tr(cx, ids::hv::CONN_FIX_RETRY, NoArgs))
                }
                FixAction::ReenterPassword => (
                    "fix-reenter".into(),
                    tr(cx, ids::hv::CONN_FIX_REENTER, NoArgs),
                ),
                FixAction::UseDatabase(db) => {
                    let id = format!("fix-use-{db_index}").into();
                    db_index += 1;
                    let label = tr(
                        cx,
                        ids::hv::CONN_FIX_USE_DB,
                        ids::hv::HvConnFixUseDb {
                            db: Text(db.clone()),
                        },
                    );
                    (id, label)
                }
                FixAction::SupportedVersions => (
                    "fix-versions".into(),
                    tr(cx, ids::hv::CONN_FIX_VERSIONS, NoArgs),
                ),
                FixAction::ConnectAnyway => (
                    "fix-anyway".into(),
                    tr(cx, ids::hv::CONN_FIX_ANYWAY, NoArgs),
                ),
                // ## m2b-t19-tls
                FixAction::ReviewCertificate => (
                    "fix-review-cert".into(),
                    tr(cx, ids::hv::CONN_FIX_REVIEW_CERT, NoArgs),
                ),
                FixAction::ImportCa => (
                    "fix-import-ca".into(),
                    tr(cx, ids::hv::CONN_FIX_IMPORT_CA, NoArgs),
                ),
                // ## m2b-t21-prompts
                FixAction::ReviewHostKey => (
                    "fix-review-host-key".into(),
                    tr(cx, ids::hv::CONN_FIX_REVIEW_HOST_KEY, NoArgs),
                ),
                // ## m2b-t24-shots (fix round 1, I4): the M2a NOTES `fix-ssh`.
                FixAction::ConnectThroughSsh => {
                    ("fix-ssh".into(), tr(cx, ids::hv::CONN_FIX_SSH, NoArgs))
                }
            };
            let kind = match action {
                FixAction::ConnectAnyway => ButtonKind::Ghost,
                // ## m2b-t21-prompts: Review host key is primary too (the review's default is
                // Cancel, so reviewing is safe).
                FixAction::ReenterPassword
                | FixAction::TryAgain
                | FixAction::UseDatabase(_)
                | FixAction::ReviewCertificate
                | FixAction::ReviewHostKey
                    if !primary_taken =>
                {
                    primary_taken = true;
                    ButtonKind::Primary
                }
                _ => ButtonKind::Default,
            };
            FixButton {
                id,
                action: action.clone(),
                label,
                kind,
                disabled: false,
            }
        })
        .collect()
}
