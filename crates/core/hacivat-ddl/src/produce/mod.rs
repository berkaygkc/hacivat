//! Producers (HV-DDL-07): model objects to IR to a script, for view-only use in M4 (the DDL
//! view, copy, open in console). Synthesized DDL for tables, indexes and sequences; server text
//! for views, routines and triggers (R-M4-24). A producer whose grants facet is not loaded
//! emits the notice `ddl-grants-not-loaded`, never a script that silently lacks grants.
//!
//! M4 Task 1a seam: [`create_script`] answers `DdlError::NotImplemented` until Task 19.

pub mod mysql;
pub mod pg;
pub mod placement;
pub mod qualify;
pub mod sqlite;

use hacivat_meta::ids::ObjectKey;
use hacivat_meta::snapshot::ObjHandle;
use hacivat_meta::view::ModelView;

use crate::options::GenOptions;
use crate::script::Script;
use crate::{DdlError, DdlTarget};

/// The CREATE script of `objects` as `view` holds them.
///
/// Task 1a stub: `Err(DdlError::NotImplemented)`.
pub fn create_script(
    view: &dyn ModelView<Def = ObjHandle>,
    objects: &[ObjectKey],
    target: &DdlTarget,
    opts: &GenOptions,
) -> Result<Script, DdlError> {
    let _ = (view, objects, target, opts);
    Err(DdlError::NotImplemented)
}
