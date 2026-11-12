//! Quick fixes the error surfaces offer, and what each does (HV-CONN-03, Task 20).
//!
//! The fixes come from the error registry (`fixes_for`, Task 10), in its button order. M2a runs
//! seven of them; the others (`ReviewCertificate`, `ConnectThroughSsh`, …) belong to M2b flows
//! and are not shown. `UseDatabase` becomes one button per suggested database (at most 3); with
//! no suggestion (SQLite, or no list) it offers nothing.

use hacivat_l10n::{NoArgs, Text, ids};
use hacivat_proto::FixId;
use hacivat_session::pipeline::ConnectError;
use hacivat_ui::button::ButtonKind;
use hacivat_ui_core::gpui_surface::{App, SharedString};

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
    EncryptWithoutVerifying,
}

impl FixAction {
    /// Whether the fix changes the saved profile (and so needs a writable connections file).
    pub fn edits_profile(&self) -> bool {
        matches!(
            self,
            FixAction::UseDatabase(_)
                | FixAction::ConnectAnyway
                | FixAction::EncryptWithoutVerifying
        )
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

/// The fixes `error` offers, in the registry's order. `UseDatabase` expands into the first
/// [`MAX_DATABASE_FIXES`] suggestions; `ReenterPassword` is left out once the gesture's
/// `rejections` reached [`MAX_REJECTIONS`]; `EditConnection` is left out when `with_edit` is
/// false (the inline strip sits on the form itself, as in the mock).
pub fn fix_actions(error: &ConnectError, rejections: u8, with_edit: bool) -> Vec<FixAction> {
    let mut out = Vec::new();
    for fix in hacivat_driver::errors::fixes_for(error.class) {
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
            FixId::EncryptWithoutVerifying => out.push(FixAction::EncryptWithoutVerifying),
            // M2b flows (certificate review, SSH, …) and the edit button inline.
            _ => {}
        }
    }
    out
}

/// The buttons for `actions`: IDs `fix-edit`, `fix-retry`, `fix-reenter`, `fix-use-<i>`,
/// `fix-versions`, `fix-anyway`, `fix-require`. The first of re-enter, try again or the first
/// database is the primary (the mock); "Connect anyway" is a ghost (PO Q3); the rest are
/// default buttons. "Encrypt without verifying" is never primary: it weakens the connection.
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
                FixAction::EncryptWithoutVerifying => (
                    "fix-require".into(),
                    tr(cx, ids::hv::CONN_FIX_REQUIRE, NoArgs),
                ),
            };
            let kind = match action {
                FixAction::ConnectAnyway => ButtonKind::Ghost,
                FixAction::ReenterPassword | FixAction::TryAgain | FixAction::UseDatabase(_)
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
