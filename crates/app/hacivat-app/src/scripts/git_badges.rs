//! Git status badges on script rows (HV-SCRIPT-11, R-M4b-11; NOTES "New primitives"): the
//! tree's `status` slot with a glyph and its required text alternative (Modified and New amber,
//! Conflicted ink, Unknown text-3, the folder "contains changes" dot), never red.
//!
//! ## m4b-t01b-seams: nothing yet (Task 13).
//!
//! ## m4b-t13-explorer
//! The badge of a shown file comes from `LibraryView::git_status` (the env-filtered view,
//! R-T10-2), so a path a connection does not show never has a badge there, and a folder shows
//! the "contains changes" dot only over files that connection shows. The colours are the tree
//! row's (`hacivat-ui` `tree/view/row.rs`, ruling R-T2-1 a); the letters are the same in every
//! language (NOTES ruling 13), the text alternatives are Fluent.

use hacivat_git::{FileStatus, UnknownWhy};
use hacivat_l10n::ArgValue;
use hacivat_ui::tree::{RowStatus, StatusKind};
use hacivat_ui_core::gpui_surface::{App, SharedString};

use crate::l10n_global::tr_id;

/// Whether `status` marks an uncommitted change (its folders show the dot); an unknown status
/// says nothing about the folder.
pub(crate) fn is_change(status: FileStatus) -> bool {
    matches!(
        status,
        FileStatus::Modified | FileStatus::New | FileStatus::Conflicted
    )
}

fn unknown_text(why: UnknownWhy, cx: &App) -> SharedString {
    let (id, reason) = match why {
        UnknownWhy::Dataless => ("hv-scripts-git-unknown-tip", "dataless"),
        UnknownWhy::Filtered => ("hv-scripts-git-unknown-tip", "filter"),
        UnknownWhy::Submodule => ("hv-scripts-git-unknown-other-tip", "submodule"),
        UnknownWhy::CaseMismatch => ("hv-scripts-git-unknown-other-tip", "case"),
        UnknownWhy::NotAFile => ("hv-scripts-git-unknown-other-tip", "notafile"),
        // ## m4b-cifix-2-merge (controller): R-CI2-2's reason, an index path never followed.
        UnknownWhy::UnsafePath => ("hv-scripts-git-unknown-other-tip", "unsafepath"),
    };
    crate::explorer::live::text(cx, id, vec![("reason", ArgValue::Text(reason.into()))])
}

/// A file's badge: its glyph and text alternative.
pub(crate) fn file_status(status: FileStatus, cx: &App) -> RowStatus {
    let (kind, glyph, text) = match status {
        FileStatus::Modified => (
            StatusKind::Modified,
            tr_id(cx, "hv-scripts-git-modified"),
            tr_id(cx, "hv-scripts-git-modified-tip"),
        ),
        FileStatus::New => (
            StatusKind::New,
            tr_id(cx, "hv-scripts-git-new"),
            tr_id(cx, "hv-scripts-git-new-tip"),
        ),
        FileStatus::Conflicted => (
            StatusKind::Conflicted,
            tr_id(cx, "hv-scripts-git-conflicted"),
            tr_id(cx, "hv-scripts-git-conflicted-tip"),
        ),
        FileStatus::Unknown(why) => (
            StatusKind::Unknown,
            tr_id(cx, "hv-scripts-git-unknown"),
            unknown_text(why, cx),
        ),
    };
    RowStatus { kind, glyph, text }
}

/// A folder's "contains changes" dot.
pub(crate) fn contains_changes(cx: &App) -> RowStatus {
    RowStatus {
        kind: StatusKind::ContainsChanges,
        glyph: tr_id(cx, "hv-scripts-git-contains"),
        text: tr_id(cx, "hv-scripts-git-contains-tip"),
    }
}

/// The badge's line in the row tooltip: the glyph, then its text alternative.
pub(crate) fn tip_line(status: &RowStatus) -> SharedString {
    format!("{} {}", status.glyph, status.text).into()
}

/// The word assistive technology reads for a file's badge.
pub(crate) fn a11y_word(status: FileStatus, cx: &App) -> SharedString {
    tr_id(
        cx,
        match status {
            FileStatus::Modified => "hv-scripts-a11y-modified",
            FileStatus::New => "hv-scripts-a11y-new",
            FileStatus::Conflicted => "hv-scripts-a11y-conflicted",
            FileStatus::Unknown(_) => "hv-scripts-a11y-unknown",
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_real_changes_mark_their_folders() {
        assert!(is_change(FileStatus::Modified));
        assert!(is_change(FileStatus::New));
        assert!(is_change(FileStatus::Conflicted));
        assert!(!is_change(FileStatus::Unknown(UnknownWhy::Filtered)));
        assert!(!is_change(FileStatus::Unknown(UnknownWhy::Dataless)));
    }
}
