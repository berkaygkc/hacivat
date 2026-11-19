//! DDL generation (HV-DDL-07 view-only producers, -09, -10, -11 subset; alg/03): the typed IR
//! ([`ir::DdlOp`]), one renderer per dialect ([`render`]), and producers that build the IR from
//! catalog snapshots ([`produce::create_script`]). Generated SQL is never localized and is
//! byte-stable; everything around it (notices) is a Fluent id.
//!
//! Pure on its default features (`DdlIsPure`): no async runtime, no session, no driver, no
//! introspection, no l10n. The non-default feature `apply` is M7's (R-M4-35).
//!
//! M4 Task 1a seams: the IR is declared in full; every renderer answers
//! `RenderError::Unsupported` until Task 7 and the producer `DdlError::NotImplemented` until
//! Task 19.

pub mod ir;
pub mod options;
pub mod produce;
pub mod render;
pub mod script;

use std::fmt;

use hacivat_meta::name::Name;
use hacivat_meta::snapshot::Flavor;
use hacivat_proto::DbmsKind;
use sql_dialect::{DialectId, ServerVersion};
use sql_ident::quote::QuoteCtx;

pub use options::GenOptions;
pub use produce::create_script;
pub use render::{RenderCtx, RenderError, Renderer, renderer};
pub use script::{Notice, Script};

/// The server a script is generated for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DdlTarget {
    pub dialect: DialectId,
    /// The precise engine (MySQL vs MariaDB texts differ, F9).
    pub flavor: Flavor,
    pub version: ServerVersion,
    /// The session's current schema (HV-SESS-07), for `Qualify::Auto`.
    pub current_schema: Option<Name>,
    pub search_path: Vec<Name>,
    pub quote: QuoteCtx,
}

impl DdlTarget {
    /// A target of `dialect` at `version` with the dialect's own engine as flavor, no current
    /// schema and the servers' default quoting settings.
    pub fn new(dialect: DialectId, version: ServerVersion) -> Self {
        let flavor = match dialect {
            DialectId::Postgres => DbmsKind::Postgres,
            DialectId::MySql => DbmsKind::MySql,
            DialectId::MariaDb => DbmsKind::MariaDb,
            DialectId::Sqlite => DbmsKind::Sqlite,
            DialectId::SqlServer => DbmsKind::MsSql,
            DialectId::ClickHouse => DbmsKind::ClickHouse,
            DialectId::Oracle => DbmsKind::Oracle,
            _ => DbmsKind::Unknown,
        };
        DdlTarget {
            dialect,
            flavor,
            version,
            current_schema: None,
            search_path: Vec::new(),
            quote: QuoteCtx::default(),
        }
    }
}

/// Why a script cannot be produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DdlError {
    /// An object the view does not hold (or not at a level that has its definition).
    NotLoaded,
    /// A kind the target dialect cannot generate (`ddl-unsupported-kind`).
    UnsupportedKind,
    /// A name that cannot be quoted (it holds a NUL).
    Quote(sql_ident::quote::QuoteError),
    Render(RenderError),
    /// Not written yet (M4 Task 1a stub).
    NotImplemented,
}

impl fmt::Display for DdlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DdlError::NotLoaded => write!(f, "object not loaded"),
            DdlError::UnsupportedKind => write!(f, "kind not supported by the target"),
            DdlError::Quote(e) => write!(f, "cannot quote a name: {e:?}"),
            DdlError::Render(e) => write!(f, "{e}"),
            DdlError::NotImplemented => write!(f, "not implemented"),
        }
    }
}

impl std::error::Error for DdlError {}
