//! The existence-diff merge (HV-INTRO-11): an introspection result plus the previous namespace
//! give the next namespace and its [`CatalogDelta`]. A failed or permission-denied existence
//! query never deletes; a mass deletion is guarded (D-039, R-M4-22).
//!
//! M4 Task 1a seam: [`merge`] answers `Err(MergeError::NotImplemented)` and nothing is
//! published, until Task 10.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::delta::CatalogDelta;
use crate::ids::{ObjectKey, ServerId, StateMark};
use crate::kind::ObjectKind;
use crate::name::Name;
use crate::namespace::{NamespaceSnapshot, Subtree, Watermark};
use crate::snapshot::IntroErrorCode;

/// One existing object as an existence query reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// The object's path inside the namespace (its own name last).
    pub path: Vec<Name>,
    pub server_id: Option<ServerId>,
    pub overload_sig: Option<Name>,
    pub state: StateMark,
}

/// The objects of one kind that exist, or why the existence query failed (then nothing of
/// that kind may be deleted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExistSet {
    Ok(Vec<Identity>),
    Failed(IntroErrorCode),
}

/// A changed (or new) object re-read with its subtree.
#[derive(Debug, Clone)]
pub struct ChangedObject {
    /// The key of the object's parent in the namespace.
    pub parent: ObjectKey,
    pub subtree: Subtree,
}

/// A category of parent-scoped rows a pass re-read for a whole kind (section cascade,
/// R-M4-48).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    Columns,
    Keys,
    ForeignKeys,
    Indexes,
    Checks,
    Triggers,
    Comments,
    Sources,
}

/// What one introspection pass found in one namespace.
#[derive(Debug, Clone)]
pub struct NamespaceResult {
    pub existent: BTreeMap<ObjectKind, ExistSet>,
    pub changed: Vec<ChangedObject>,
    pub categories_to_reload: BTreeSet<Category>,
    pub new_watermark: Watermark,
    /// A full pass (every object re-read), not an incremental one.
    pub full: bool,
}

impl NamespaceResult {
    /// A full pass that found nothing.
    pub fn empty_full() -> Self {
        NamespaceResult {
            existent: BTreeMap::new(),
            changed: Vec::new(),
            categories_to_reload: BTreeSet::new(),
            new_watermark: Watermark::None,
            full: true,
        }
    }
}

/// Who asked for the pass: a user refresh asks before a mass deletion; a background pass marks
/// the namespace stale and keeps the result pending.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Trigger {
    User,
    #[default]
    Background,
}

/// The mass-deletion guard (D-039): more than `guard_ratio_pct` % of a namespace with at least
/// `guard_min_objects` objects vanishing in one pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergePolicy {
    pub trigger: Trigger,
    pub guard_ratio_pct: u8,
    pub guard_min_objects: u32,
}

impl Default for MergePolicy {
    /// A background pass with D-039's thresholds (50 %, 20 objects).
    fn default() -> Self {
        MergePolicy {
            trigger: Trigger::Background,
            guard_ratio_pct: 50,
            guard_min_objects: 20,
        }
    }
}

/// The merge's answer.
#[derive(Debug, Clone)]
pub enum MergeOutcome {
    Applied {
        next: NamespaceSnapshot,
        delta: CatalogDelta,
    },
    /// The guard fired: nothing is applied until the user confirms.
    PendingConfirmation {
        next: NamespaceSnapshot,
        delta: CatalogDelta,
        vanished: u32,
    },
}

/// Why a merge failed (nothing is published).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeError {
    /// The result is for another namespace than `prev`.
    WrongNamespace,
    /// The merged arena is invalid.
    Model(crate::namespace::ModelError),
    /// Not written yet (M4 Task 1a stub).
    NotImplemented,
}

impl fmt::Display for MergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MergeError::WrongNamespace => write!(f, "result for another namespace"),
            MergeError::Model(e) => write!(f, "{e}"),
            MergeError::NotImplemented => write!(f, "not implemented"),
        }
    }
}

impl std::error::Error for MergeError {}

/// Merges `r` into `prev`.
///
/// Task 1a stub: `Err(MergeError::NotImplemented)`.
pub fn merge(
    prev: Option<&NamespaceSnapshot>,
    r: NamespaceResult,
    policy: &MergePolicy,
) -> Result<MergeOutcome, MergeError> {
    let _ = (prev, r, policy);
    Err(MergeError::NotImplemented)
}
