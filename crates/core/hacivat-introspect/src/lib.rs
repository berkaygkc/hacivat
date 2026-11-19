//! Introspection (HV-INTRO-01..16): catalog query packs, scope and filter grammars, the
//! scheduler and `CatalogHub`, the per-engine introspectors, freshness checks, refresh after
//! DDL and the `minicat` snapshot generator.
//!
//! M4 Task 1a creates the crate and its module lines; Task 1c (Part B) declares the seams with
//! fail-closed stubs.
//!
//! **Feature `session` (default; T1a fix round 2, ADR 0008 `m4-t01a`).** Everything that needs a
//! live connection — `hacivat-session` (`MetaConn`, `MetaQuery`), `hacivat-secrets` (the
//! `CatalogKey`) or `hacivat-settings` — sits behind it. Without it the crate is the pure
//! grammars only: `pack` (the parser, sections, macros and the lint; the renderer that builds a
//! `MetaQuery` is a `#[cfg(feature = "session")]` submodule of `pack`), `scope`, `filter` and
//! `glob`. The `fuzz/meta` package builds it that way, so fuzzing never compiles the session,
//! network and SSH stack; `xtask` tests that this graph never reaches `hacivat-session`.

pub mod filter;
pub mod glob;
pub mod pack;
pub mod scope;

#[cfg(feature = "session")]
pub mod assemble;
#[cfg(feature = "session")]
pub mod ddl_effects;
#[cfg(feature = "session")]
pub mod engine;
#[cfg(feature = "session")]
pub mod fragment;
#[cfg(feature = "session")]
pub mod freshness;
#[cfg(feature = "session")]
pub mod hub;
#[cfg(feature = "session")]
pub mod minicat;
#[cfg(feature = "session")]
pub mod mysql;
#[cfg(feature = "session")]
pub mod pg;
#[cfg(feature = "session")]
pub mod refresh_coord;
#[cfg(feature = "session")]
pub mod sqlite;
#[cfg(feature = "session")]
pub mod testing;
#[cfg(feature = "session")]
pub mod ticket;

// ## m4-t01c-core: the per-engine introspector seam (Part B), gated like every module that
// names a session type (ADR 0008 `m4-t01a`).
#[cfg(feature = "session")]
pub mod introspector;

#[cfg(feature = "session")]
pub use engine::{IntroHandle, Phase, Progress, TaskProgress};
#[cfg(feature = "session")]
pub use introspector::{
    FacetRows, IntroCaps, IntroError, Introspector, Staleness, introspector_for,
};
pub use pack::Mode;
pub use scope::{CurrentCtx, NsInfo, ScopePattern, parse_scope};
#[cfg(feature = "session")]
pub use ticket::RefreshTicket;
