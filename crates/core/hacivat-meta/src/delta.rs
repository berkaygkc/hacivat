//! The catalog delta of one publish (HV-INTRO-11; R-M4-36 keeps the name `CatalogDelta`, M5's
//! statement-effect delta is `DdlDelta`): what was created, altered, dropped, renamed or moved.
//! The explorer maps it to tree updates keyed by `ObjectKey`. M4 Task 1a seam (types only;
//! Task 10 computes it).

use serde::{Deserialize, Serialize};

use crate::ids::{NsKey, ObjectKey};

/// Which parts of an altered object changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChangedFacets {
    pub definition: bool,
    pub children: bool,
    pub comment: bool,
    pub source: bool,
    pub grants: bool,
}

/// How sure a rename is: by server id (certain) or by a structural heuristic (suggested).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Confidence {
    ServerId,
    Heuristic,
}

/// The changes of one publish.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CatalogDelta {
    pub created: Vec<ObjectKey>,
    pub altered: Vec<(ObjectKey, ChangedFacets)>,
    pub dropped: Vec<ObjectKey>,
    /// `(before, after, confidence)`.
    pub renamed: Vec<(ObjectKey, ObjectKey, Confidence)>,
    /// `(from namespace, to namespace, object)`.
    pub moved: Vec<(NsKey, NsKey, ObjectKey)>,
}

impl CatalogDelta {
    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.created.is_empty()
            && self.altered.is_empty()
            && self.dropped.is_empty()
            && self.renamed.is_empty()
            && self.moved.is_empty()
    }
}
