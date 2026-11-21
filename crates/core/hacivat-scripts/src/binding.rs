// ## m4b-t01a-seams
//! Logical-database binding: which libraries appear under a database node (HV-SCRIPT-02;
//! R-M4b-3, -7, -32).
//!
//! ## m4b-t01a-seams
//! Task 1a: [`DbNode`] and a fail-closed [`resolve`] stub.
//!
//! ## m4b-t03-registry
//! Task 3: the matching.
//! 1. A local mapping for exactly this node (the profile and the server's exact database name)
//!    binds its library and nothing else; a mapping whose library is gone binds nothing (no
//!    automatic fallback for that node).
//! 2. Otherwise every library whose logical database or an alias matches the node's logical
//!    name ([`same_db`](crate::names::same_db): NFC, then ASCII case-insensitive).
//! 3. Order: the library's folder name (NFC, ASCII-lowercased; never locale casing), then its
//!    ID.
//!
//! Binding is about names only: whether a bound library is reachable (`LibraryState`) is the
//! projection's question (HV-SCRIPT-05), and the environment scope of each file is
//! HV-SCRIPT-04's. A database name is never joined to a path.

use std::path::Path;

use hacivat_proto::ProfileId;
use icu_normalizer::ComposingNormalizerBorrowed;

use crate::names::{DatabaseName, LogicalDb};
use crate::registry::{LibraryId, LocalMapping};
use crate::service::LibrarySnapshot;

/// A database node of the explorer: the profile, the server's exact database name, and the
/// name binding compares (the database name, or the SQLite file stem; R-M4b-32).
#[derive(Debug, Clone)]
pub struct DbNode {
    pub profile: ProfileId,
    pub database: DatabaseName,
    pub logical: LogicalDb,
}

/// The libraries bound to `node`, in stable order.
pub fn resolve(node: &DbNode, libs: &[LibrarySnapshot], maps: &[LocalMapping]) -> Vec<LibraryId> {
    if let Some(m) = maps
        .iter()
        .find(|m| m.profile == node.profile && m.database == node.database)
    {
        return libs
            .iter()
            .find(|l| l.id == m.library)
            .map(|l| l.id)
            .into_iter()
            .collect();
    }
    let mut bound: Vec<(String, LibraryId)> = libs
        .iter()
        .filter(|l| {
            l.db.same_db(&node.logical) || l.aliases.iter().any(|a| a.same_db(&node.logical))
        })
        .map(|l| (order_key(l), l.id))
        .collect();
    bound.sort();
    bound.dedup_by(|a, b| a.1 == b.1);
    bound.into_iter().map(|(_, id)| id).collect()
}

/// The library's folder name as it sorts: NFC, ASCII-lowercased.
fn order_key(lib: &LibrarySnapshot) -> String {
    let name = lib
        .root
        .path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    ComposingNormalizerBorrowed::new_nfc()
        .normalize(&name)
        .to_ascii_lowercase()
}

/// The logical name of an SQLite database: the main file's stem (`orders.db` → `orders`;
/// R-M4b-3, -32), or `None` when the stem is missing or not a valid name, so the node binds
/// only through a local mapping.
pub fn logical_for_sqlite(main_file: &Path) -> Option<LogicalDb> {
    let stem = main_file.file_stem()?.to_str()?;
    LogicalDb::parse(stem).ok()
}
// ## m4b-t01a-seams
