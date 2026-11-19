//! The live explorer (Task 24): a `CatalogHost` per profile over `CatalogHub` — the cached tree at
//! start, cached-then-validated on connect, the engine stopped on disconnect, and the expanded
//! state kept in `workspace.json` (mocks 1–3).
//!
//! ## m4-t01b-seams: stub; the explorer is still the M2a list. [`database_rows`] (M5, M4b) is
//! declared and answers nothing until Task 24 fills it.

use hacivat_proto::ProfileId;
use hacivat_ui_core::gpui_surface::App;

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
