//! Option sanitizing: which driver options a profile of a given origin may carry (HV-DRV-05).
//! Body in Task 2.

use std::path::PathBuf;

use hacivat_proto::{DbmsKind, Origin};

use crate::catalog::EngineCatalog;

/// Why an option was rejected (HV-DRV-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViolationReason {
    UnknownOption,
    DoubleEncoding,
    PathOutsideRoots,
    AlwaysDenied,
    QuarantinedScript,
}

/// One rejected option (HV-DRV-05).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Violation {
    pub key: String,
    pub reason: ViolationReason,
}

/// What [`sanitize`] checks (HV-DRV-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SanitizeInput<'a> {
    pub kind: DbmsKind,
    pub origin: &'a Origin,
    pub options: &'a [(String, String)],
    /// Options that name files, with their paths.
    pub paths: &'a [(String, PathBuf)],
    /// The user allowed options outside the allow-list.
    pub unsafe_toggle: bool,
    pub approved_roots: &'a [PathBuf],
}

/// The options to pass to the driver, or every violation (Task 2). The stub passes nothing.
pub fn sanitize(
    _input: &SanitizeInput<'_>,
    _catalog: &EngineCatalog,
) -> Result<Vec<(String, String)>, Vec<Violation>> {
    Ok(Vec::new())
}
