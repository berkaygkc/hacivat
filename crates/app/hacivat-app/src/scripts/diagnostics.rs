//! The library diagnostics row and popover (HV-SCRIPT-03, -04, -08, -11; NOTES surface 11):
//! unknown `@` folders, contradictory scopes, limits hit, the polling fallback, an invalid
//! `workspace.toml`, an untrusted repository, a missing or unmounted folder.
//!
//! ## m4b-t01b-seams: nothing yet (Task 13).
//!
//! ## m4b-t13-explorer
//! Every problem comes from the library as the row's connection sees it (`LibraryView`,
//! R-T10-2): a diagnostic that names a path that connection does not show is only counted
//! ("N problems concern files this connection doesn't show"), never named. Names are in their
//! display form already (escaped, NFC); a `workspace.toml` problem names keys, never values.
//! The popover is the shipped explanation popover (`hacivat_ui::explain`), anchored under the
//! problems row; Escape returns the focus to the row. Its "Script library settings…" footer is
//! Task 16's (Settings › Script libraries).

use hacivat_l10n::ArgValue;
use hacivat_scripts::manifest::Diag as ManifestDiag;
use hacivat_scripts::scan::{Diag as ScanDiag, LimitHit};
use hacivat_scripts::{GitSummary, LibraryDiag, LibraryId, LibraryState, LibraryView, PollReason};
use hacivat_ui::explain::ExplainPopover;
use hacivat_ui::icon::IconName;
use hacivat_ui::tree::TreeKey;
use hacivat_ui_core::gpui_surface::{App, SharedString, Window};

use super::projection::{self, Place};
use crate::l10n_global::tr_id;
use crate::shell::Workspace;

/// The layer of the diagnostics popover (`Workspace::open_anchored`).
pub const DIAGNOSTICS_LAYER: &str = "scripts-diagnostics";

fn text(cx: &App, id: &str, args: Vec<(&'static str, ArgValue)>) -> SharedString {
    crate::explorer::live::text(cx, id, args)
}

fn arg(s: &str) -> ArgValue {
    ArgValue::Text(s.to_string())
}

fn num(n: usize) -> ArgValue {
    ArgValue::Count(u64::try_from(n).unwrap_or(u64::MAX))
}

/// The problems `lib` shows its connection: its diagnostics, a polled library and an untrusted
/// repository.
#[derive(Debug, Clone, PartialEq)]
enum Problem {
    Diag(LibraryDiag),
    Polling(PollReason),
    Untrusted,
}

fn problems(lib: &LibraryView<'_>) -> Vec<Problem> {
    let mut out: Vec<Problem> = lib.diagnostics().into_iter().map(Problem::Diag).collect();
    if let LibraryState::Polling(r) = lib.library().state {
        out.push(Problem::Polling(r));
    }
    if matches!(lib.git(), GitSummary::Untrusted(_)) {
        out.push(Problem::Untrusted);
    }
    out
}

/// How many problems the library's row counts ("2 problems").
pub(crate) fn count(lib: &LibraryView<'_>) -> usize {
    problems(lib).len()
}

fn manifest_line(d: &ManifestDiag, folder: &str, cx: &App) -> SharedString {
    let file = ".hacivat/workspace.toml";
    let toml = |reason: &str| {
        text(
            cx,
            "hv-scripts-diag-toml",
            vec![
                ("reason", arg(reason)),
                ("file", arg(file)),
                ("folder", arg(folder)),
            ],
        )
    };
    match d {
        ManifestDiag::NewerFormat(_) => toml("newer"),
        ManifestDiag::TooLarge => toml("large"),
        ManifestDiag::UnknownKey(_) | ManifestDiag::MoreUnknownKeys(_) => {
            text(cx, "hv-scripts-diag-toml-keys", vec![("file", arg(file))])
        }
        ManifestDiag::InvalidToml
        | ManifestDiag::NotUtf8
        | ManifestDiag::TooDeep
        | ManifestDiag::MissingFormat
        | ManifestDiag::BadFormat => toml("invalid"),
        _ => text(
            cx,
            "hv-scripts-diag-other",
            vec![
                ("what", arg("manifest")),
                ("n", num(0)),
                ("folder", arg(folder)),
            ],
        ),
    }
}

fn other(what: &str, n: usize, folder: &str, cx: &App) -> SharedString {
    text(
        cx,
        "hv-scripts-diag-other",
        vec![("what", arg(what)), ("n", num(n)), ("folder", arg(folder))],
    )
}

/// The lines of one problem.
fn lines(p: &Problem, folder: &str, cx: &App) -> Vec<SharedString> {
    match p {
        Problem::Diag(LibraryDiag::Scan(ScanDiag::UnknownEnvFolder { name, suggestion })) => {
            let first = match suggestion {
                Some(tag) => text(
                    cx,
                    "hv-scripts-diag-unknown-env",
                    vec![
                        ("folder", arg(name)),
                        ("suggestion", arg(&format!("@{}", tag.id()))),
                    ],
                ),
                None => text(
                    cx,
                    "hv-scripts-diag-unknown-env-nosug",
                    vec![("folder", arg(name))],
                ),
            };
            vec![first, tr_id(cx, "hv-scripts-diag-env-help")]
        }
        Problem::Diag(LibraryDiag::Scan(ScanDiag::EnvContradiction { folder: f })) => vec![text(
            cx,
            "hv-scripts-diag-contradict",
            vec![("path", arg(&f.display_text()))],
        )],
        Problem::Diag(LibraryDiag::Scan(ScanDiag::BadName { .. })) => {
            vec![other("badname", 0, folder, cx)]
        }
        Problem::Diag(LibraryDiag::Scan(ScanDiag::Unreadable { .. })) => {
            vec![other("unreadable", 0, folder, cx)]
        }
        Problem::Diag(LibraryDiag::Scan(ScanDiag::LinkScope { .. })) => {
            vec![other("linkscope", 0, folder, cx)]
        }
        Problem::Diag(LibraryDiag::ScanStopped(hit)) => {
            let (limit, n) = match hit {
                LimitHit::Depth(n) => ("depth", *n),
                LimitHit::Files(n) | LimitHit::Visited(n) => ("files", *n),
            };
            vec![text(
                cx,
                "hv-scripts-diag-limit",
                vec![("limit", arg(limit)), ("n", num(n))],
            )]
        }
        Problem::Diag(LibraryDiag::Manifest(d)) => vec![manifest_line(d, folder, cx)],
        Problem::Diag(LibraryDiag::Suppressed(n)) => vec![other("suppressed", *n, folder, cx)],
        Problem::Diag(LibraryDiag::HiddenProblems(n)) => vec![other("hidden", *n, folder, cx)],
        Problem::Diag(LibraryDiag::NoLogicalName) => vec![other("noname", 0, folder, cx)],
        Problem::Diag(LibraryDiag::RootInEnvFolder(_)) => vec![other("rootenv", 0, folder, cx)],
        Problem::Diag(LibraryDiag::RootRefused(_)) => vec![other("moved", 0, folder, cx)],
        Problem::Polling(r) => {
            let reason = match r {
                PollReason::WatchBudget => "watches",
                PollReason::NetworkFs | PollReason::BackendLost => "network",
            };
            vec![text(
                cx,
                "hv-scripts-diag-polling",
                vec![("reason", arg(reason))],
            )]
        }
        Problem::Untrusted => vec![
            tr_id(cx, "hv-scripts-diag-untrusted"),
            tr_id(cx, "hv-scripts-diag-untrusted-help"),
        ],
    }
}

/// The popover's title and body for library `lib`.
pub(crate) fn content(lib: &LibraryView<'_>, cx: &App) -> (SharedString, Vec<SharedString>) {
    let name = projection::library_name(lib);
    let all = problems(lib);
    let title = text(
        cx,
        "hv-scripts-diag-title",
        vec![("n", num(all.len())), ("library", arg(&name))],
    );
    let mut body: Vec<SharedString> = Vec::new();
    for p in &all {
        for line in lines(p, &name, cx) {
            // The environment help is said once.
            if !body.contains(&line) {
                body.push(line);
            }
        }
    }
    (title, body)
}

/// Opens the diagnostics popover of `library` under the problems row `key`.
pub(crate) fn open_popover(
    place: &Place,
    library: LibraryId,
    key: TreeKey,
    window: &mut Window,
    cx: &mut App,
) {
    let Some((title, body)) = projection::with_view(place, library, cx, |lib| content(lib, cx))
    else {
        return;
    };
    let anchor = crate::connections::explorer::tree(cx)
        .and_then(|t| t.read(cx).row_bounds(key))
        .map(|b| b.bottom_left())
        .unwrap_or_default();
    let popover = ExplainPopover::view(title, IconName::CircleAlert, body, cx);
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    ws.update(cx, |ws, cx| {
        ws.open_anchored(DIAGNOSTICS_LAYER, anchor, popover, window, cx);
    });
}
