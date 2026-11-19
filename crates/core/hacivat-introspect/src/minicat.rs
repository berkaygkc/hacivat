// ## m4-t01c-core
//! The system-catalog snapshot read (HV-INTRO-16; Task 23): the introspectors' own system-mode
//! pack sections (`packs/<dialect>/40-system.hvq.sql`, R-M4-44) run on a `MetaConn`, one table
//! of rows per section in server row order, for `cargo xtask minicat`.
//!
//! **Fail-closed stub (M4 Task 1c).** [`run_system_sections`] answers
//! `IntroError::NotImplemented`, so no snapshot is ever produced from a stub.

use futures::future::BoxFuture;
use hacivat_meta::Name;
use hacivat_session::meta::{MetaConn, Rows};

use crate::introspector::IntroError;
use crate::pack::{Pack, PackCtx};

/// Runs every system-mode section of `pack` on `c` and answers each section's rows.
///
/// Stub: [`IntroError::NotImplemented`].
pub fn run_system_sections<'a>(
    c: &'a mut MetaConn,
    pack: &'a Pack,
    ctx: &'a PackCtx,
) -> BoxFuture<'a, Result<Vec<(Name, Rows)>, IntroError>> {
    let _ = (c, pack, ctx);
    Box::pin(async { Err(IntroError::NotImplemented) })
}
