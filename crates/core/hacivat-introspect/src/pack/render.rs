// ## m4-t01c-core
//! The pack renderer (feature `session`): a section rendered for a [`PackCtx`] becomes
//! [`RenderedQuery`]s, and [`RenderedQuery::as_meta_query`] is the **only** caller of
//! `MetaQuery::new` (R-M4-7; check-deps `MetaQueryOnlyFromPacks`). The text is the pack's,
//! never a user's; every name and id is a bind (R-M4-3).
//!
//! **Fail-closed stub (M4 Task 1c; body Task 4).** [`Pack::render`] answers
//! [`RenderError::NotImplemented`], so no `MetaQuery` is ever built.

use hacivat_session::meta::MetaQuery;
use hacivat_values::BindValue;

use super::{Pack, PackCtx};

/// One statement of a rendered section: pack text with its binds. Built only by
/// [`Pack::render`] (its fields are private).
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedQuery {
    sql: String,
    binds: Vec<BindValue>,
    joinable: bool,
}

impl RenderedQuery {
    /// The statement as a [`MetaQuery`], the only input a `MetaConn` accepts.
    pub fn as_meta_query(&self) -> MetaQuery<'_> {
        MetaQuery::new(&self.sql, &self.binds, self.joinable)
    }

    pub fn sql(&self) -> &str {
        &self.sql
    }

    pub fn binds(&self) -> &[BindValue] {
        &self.binds
    }

    /// The section is marked `--- joinable ---` (R-M4-29).
    pub fn joinable(&self) -> bool {
        self.joinable
    }
}

/// Why a section does not render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    /// The renderer is not written yet (M4 Task 1c stub).
    NotImplemented,
    /// The pack has no section of that name.
    UnknownSection,
    /// A `:name` or `:[name]` bind the section uses is missing from `PackCtx::params`.
    MissingBind,
    /// A joined connection asked for a section not marked `joinable` (R-M4-29).
    NotJoinable,
}

impl Pack {
    /// Renders `section` for `ctx`: one query, or several when a list bind is chunked
    /// (R-M4-3); none when a list bind is empty (the section is skipped).
    ///
    /// Stub: [`RenderError::NotImplemented`].
    pub fn render(&self, section: &str, ctx: &PackCtx) -> Result<Vec<RenderedQuery>, RenderError> {
        let _ = (section, ctx);
        Err(RenderError::NotImplemented)
    }
}
