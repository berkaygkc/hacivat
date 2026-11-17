//! The PostgreSQL introspector (HV-INTRO-01..05, -11..15 as they apply to PostgreSQL 13–18).
//!
//! - [`full`] reads one schema whole: relations (tables, partitioned tables, views,
//!   materialized views, foreign tables, composite types) with columns, indexes,
//!   constraints and triggers; sequences; routines; user types (enums, domains, composite,
//!   range, base); comments. One aggregate query per kind, never per object (HV-INTRO-13).
//! - [`refresh`] re-reads only what changed since a [`PgWatermark`] (HV-INTRO-05): the
//!   existence query finds drops, the `age(xmin) BETWEEN 0 AND age(w)` change query finds the
//!   young catalog rows, a dependency query finds objects whose deparsed text names a renamed
//!   object, and only the changed facets are re-read. It falls back to full on restart,
//!   identity or version change, standby, or a watermark older than the 32-bit age range.
//! - [`apply`] merges a delta into the previous snapshot (the reference merge the tests use).
//!
//! Every pass is one `REPEATABLE READ READ ONLY` transaction on the caller's connection and
//! never assigns an XID (checked by the canary into [`PassStats::xid_assigned`]).

mod conn;
mod decode;
mod introspect;
mod merge;
pub mod model;
mod sql;
mod text;
mod watermark;

use std::fmt;

use hacivat_driver::RawError;

pub use conn::{CatalogConn, Rows};
pub use introspect::{
    IntrospectOpts, Pass, PassStats, Refresh, SectionStat, full, list_schemas, refresh,
};
pub use merge::{apply, normalize};
pub use model::*;
pub use sql::PgVersion;
pub use watermark::{FullReason, PgIdentity, PgWatermark, Plan, Probe, SPAN_LIMIT, plan};

/// Why a pass failed.
#[derive(Debug)]
pub enum IntroError {
    /// The server refused a query (permission, timeout, a concurrent drop surfacing as
    /// `cache lookup failed`, …); the pass rolled back and published nothing.
    Db(RawError),
    /// A row did not have the shape its section declares.
    Decode {
        section: &'static str,
        detail: String,
    },
    /// The namespace no longer exists.
    NamespaceGone(Oid),
}

impl fmt::Display for IntroError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntroError::Db(e) => write!(f, "catalog query failed: {:?}", e.kind),
            IntroError::Decode { section, detail } => write!(f, "{section}: {detail}"),
            IntroError::NamespaceGone(oid) => write!(f, "namespace {oid} no longer exists"),
        }
    }
}

impl std::error::Error for IntroError {}
