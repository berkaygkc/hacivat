//! The live explorer (Task 24): a `CatalogHost` per profile over `CatalogHub` — the cached tree at
//! start, cached-then-validated on connect, the engine stopped on disconnect, and the expanded
//! state kept in `workspace.json` (mocks 1–3).
//!
//! ## m4-t01b-seams: stub; the explorer is still the M2a list. [`database_rows`] and [`reveal`]
//! (M5, M4b) are declared; until Task 24 fills them they list nothing and reveal nothing
//! (`RevealOutcome::Unavailable`, never a claim that the object is missing).

use hacivat_meta::ObjectKey;
use hacivat_proto::ProfileId;
use hacivat_ui_core::gpui_surface::{App, Window};

/// One database of a profile as the explorer shows it (R-M4-19).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseRow {
    /// The name, verbatim.
    pub name: String,
    /// In the profile's scope (read), or listed greyed with "Add to scope" (R-M4-14).
    pub in_scope: bool,
    /// The database the profile's connection is on.
    pub connected: bool,
}

/// The databases `profile`'s explorer lists, in display order. Stub (Task 24): none.
pub fn database_rows(_profile: ProfileId, _cx: &App) -> Vec<DatabaseRow> {
    Vec::new()
}

/// What [`reveal`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevealOutcome {
    /// The object's row is expanded to, selected and scrolled into view.
    Revealed,
    /// An ancestor is loading; the row is selected when it arrives.
    Pending,
    /// The loaded catalog has no such object.
    Missing,
    /// The explorer cannot reveal objects (no catalog for the profile, or not built yet).
    Unavailable,
}

/// Expands the ancestors of `key` of `profile` (loading them lazily), selects its row and
/// scrolls it into view (M5 "reveal in explorer", M4b). Stub (Task 24): `Unavailable`.
pub fn reveal(
    _profile: ProfileId,
    _key: &ObjectKey,
    _window: &mut Window,
    _cx: &mut App,
) -> RevealOutcome {
    RevealOutcome::Unavailable
}
