//! DDL generation (alg/03 HV-DDL-07, -09, -10, -11 v1 core subset; plan-m4 Tasks 7 and 19).
//!
//! - [`ir`]: the typed DDL IR ([`DdlOp`]), every alg/03 variant declared.
//! - [`render`]: one [`Renderer`] per dialect (PostgreSQL, MySQL/MariaDB, SQLite); M4 renders
//!   the view-only variants and refuses the rest with [`RenderError::Unsupported`].
//! - [`produce`]: [`create_script`], the view-only producers over a
//!   [`hacivat_meta::CatalogSnapshot`], with constraint placement
//!   ([`produce::placement`], HV-DDL-09) and schema qualification ([`produce::qualify`],
//!   HV-DDL-10).
//! - [`options`]: [`DdlTarget`], [`GenOptions`] and option applicability (HV-DDL-11).
//! - [`quote`]: identifier quoting and string literals per dialect.
//! - [`script`]: [`Script`] and its byte-stable text form.
//!
//! Pure (check-deps `DdlIsPure`): no async runtime, no I/O, no locale. Generated SQL is never
//! localized and is byte-stable; notices are codes that the UI turns into Fluent messages.

pub mod error;
pub mod ir;
pub mod options;
pub mod produce;
pub mod quote;
pub mod render;
pub mod script;

pub use error::{DdlError, Notice, NoticeCode, RenderError};
pub use ir::DdlOp;
pub use options::{DdlTarget, GenOptions, Placement, Qualify};
pub use produce::{Grant, GrantsFacet, NoSources, Sources, create_script};
pub use render::{Delim, RenderCtx, Renderer, Stmt, TxnClass, renderer};
pub use script::{DelimStyle, Script};
