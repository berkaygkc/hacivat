//! What the library service announces (HV-SCRIPT-08 step 8; R-M4b-16).
//!
//! ## m4b-t01a-seams
//! Task 1a: the event type. Task 10 sends them (coalesced; the app redraws once per frame).

use crate::fileio::DiskStamp;
use crate::pathsafe::RelPath;
use crate::registry::LibraryId;

/// One change the UI may react to; the snapshot is the truth, events are prompts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryEvent {
    /// A new snapshot of the library was published.
    Changed {
        id: LibraryId,
        generation: u64,
    },
    /// A file changed on disk (not Hacivat's own write), with its new stamp.
    FileChanged {
        id: LibraryId,
        rel: RelPath,
        stamp: DiskStamp,
    },
    FileDeleted {
        id: LibraryId,
        rel: RelPath,
    },
    /// Paired by content hash after reconciliation (alg/28 C4).
    FileRenamed {
        id: LibraryId,
        from: RelPath,
        to: RelPath,
    },
    GitChanged {
        id: LibraryId,
    },
    // ## m4b-t10-service (fix round 1, I1, R-T10-2)
    /// The file moved to a place this subscriber's connection does not show: "No longer
    /// listed under this connection". Only a filtered `Subscription` sends it; its new path is
    /// never given.
    FileLeftScope {
        id: LibraryId,
        rel: RelPath,
    },
    // ## m4b-t10-service
}
// ## m4b-t01a-seams
