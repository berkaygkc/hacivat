//! The connection dialog's "Schemas" section (Task 20; design/m4-explorer surface 7): the scope
//! as a checkbox tree, the pattern field with its error at a character position, the
//! all-databases question (PostgreSQL) and "Encrypt cached metadata".
//!
//! ## m4-t01b-seams: stub. The section draws nothing and the form does not show it, so the
//! dialog is unchanged until Task 20 adds it to the form (named lines in `form.rs`).

use hacivat_session::profile::ProfileDraft;
use hacivat_ui_core::gpui_surface::{AnyElement, App, IntoElement, Window, div};

/// The Schemas section. Stub (Task 20 fills it).
#[derive(Debug, Default)]
pub struct SchemasSection;

impl SchemasSection {
    /// The section for `draft`. Stub: an empty element.
    pub fn element(
        &self,
        _draft: &ProfileDraft,
        _window: &mut Window,
        _cx: &mut App,
    ) -> AnyElement {
        div().into_any_element()
    }
}
