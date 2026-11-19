//! Overlays above the introspected catalog (HV-INTRO-16): the system-catalog snapshot of the
//! server's version, stored virtual foreign keys (local only, D-057) and annotations.
//! M4 Task 1a seam (types only; Task 26 layers them).

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::ids::ObjectKey;
use crate::name::Name;
use crate::system::SystemSnapshot;

/// A virtual foreign key: columns in FK order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualFk {
    pub from: ObjectKey,
    pub from_cols: Vec<String>,
    pub to: ObjectKey,
    pub to_cols: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// The overlays of one generation.
#[derive(Debug, Clone, Default)]
pub struct Overlays {
    pub system: Option<Arc<SystemSnapshot>>,
    pub virtual_fks: Vec<VirtualFk>,
    pub annotations: BTreeMap<ObjectKey, Name>,
}
