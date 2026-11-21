//! Logical-database binding: which libraries appear under a database node (HV-SCRIPT-02;
//! R-M4b-3, -7, -32).
//!
//! ## m4b-t01a-seams
//! Task 1a: [`DbNode`] and a fail-closed [`resolve`] stub that binds nothing. Task 3 writes the
//! matching (NFC, then ASCII case-insensitive; mappings beat automatic binding per node).

use hacivat_proto::ProfileId;

use crate::names::{DatabaseName, LogicalDb};
use crate::registry::{LibraryId, LocalMapping};
use crate::service::LibrarySnapshot;

/// A database node of the explorer: the profile, the server's exact database name, and the
/// name binding compares (the database name, or the SQLite file stem; R-M4b-32).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbNode {
    pub profile: ProfileId,
    pub database: DatabaseName,
    pub logical: LogicalDb,
}

/// The libraries bound to `node`, in stable order.
///
/// Task 1a stub (fail closed): binds nothing, so no Scripts node is shown.
pub fn resolve(node: &DbNode, libs: &[LibrarySnapshot], maps: &[LocalMapping]) -> Vec<LibraryId> {
    let _ = (node, libs, maps);
    Vec::new()
}
// ## m4b-t01a-seams
