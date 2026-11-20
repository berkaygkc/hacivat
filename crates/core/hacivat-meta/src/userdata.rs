//! Per-profile user data about the catalog (A-19): the introspection scope, view and scope
//! filters, level overrides, virtual foreign keys, annotations and the DDL view's options.
//!
//! Types and (Task 26) merge logic only: the file I/O is `hacivat_settings::profile_data`.
//! Serde, versioned ([`PROFILE_USER_DATA_VERSION`]); maps keyed by [`ObjectKey`] travel as
//! lists of pairs, so TOML and JSON (string-keyed maps only) can hold them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ids::ObjectKey;
use crate::namespace::Level;
use crate::overlay::VirtualFk;

/// The current version of [`ProfileUserData`]'s serialized form.
pub const PROFILE_USER_DATA_VERSION: u32 = 1;

/// Schema qualification in generated DDL (HV-DDL-10; R-M4-23 default `Auto`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Qualify {
    #[default]
    Auto,
    Never,
    Always,
}

/// Constraint placement in generated DDL (HV-DDL-09; R-M4-23 default `InsideTable`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Placement {
    InsideColumn,
    #[default]
    InsideTable,
    AfterTable,
}

/// The DDL view's options strip, remembered per profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DdlViewOptions {
    pub qualify: Qualify,
    pub placement: Placement,
    /// "Definitions provided by server".
    pub server_definitions: bool,
    pub grants: bool,
    pub owner: bool,
}

impl Default for DdlViewOptions {
    fn default() -> Self {
        DdlViewOptions {
            qualify: Qualify::Auto,
            placement: Placement::InsideTable,
            server_definitions: false,
            grants: true,
            owner: true,
        }
    }
}

/// One profile's catalog user data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileUserData {
    #[serde(default = "current_version")]
    pub version: u32,
    /// The scope pattern (HV-INTRO-02); `None` = the engine's default (R-M4-14).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, with = "pairs")]
    pub view_filters: BTreeMap<ObjectKey, String>,
    #[serde(default, with = "pairs")]
    pub scope_filters: BTreeMap<ObjectKey, String>,
    #[serde(default, with = "pairs")]
    pub level_overrides: BTreeMap<ObjectKey, Level>,
    #[serde(default)]
    pub virtual_fks: Vec<VirtualFk>,
    #[serde(default, with = "pairs")]
    pub annotations: BTreeMap<ObjectKey, String>,
    #[serde(default)]
    pub ddl_view: DdlViewOptions,
    // ## m4-t27-refresh-ui (named exception, T26 carry: persist "Introspect live")
    /// The system namespaces the user chose to introspect live rather than read from the
    /// bundled snapshot (`overlay::Overlays::live_system`, Task 26), sorted, no duplicates.
    /// Absent from older files (empty).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub live_system: Vec<crate::ids::NsKey>,
}

fn current_version() -> u32 {
    PROFILE_USER_DATA_VERSION
}

impl Default for ProfileUserData {
    fn default() -> Self {
        ProfileUserData {
            version: PROFILE_USER_DATA_VERSION,
            scope: None,
            view_filters: BTreeMap::new(),
            scope_filters: BTreeMap::new(),
            level_overrides: BTreeMap::new(),
            virtual_fks: Vec::new(),
            annotations: BTreeMap::new(),
            ddl_view: DdlViewOptions::default(),
            // ## m4-t27-refresh-ui (named exception)
            live_system: Vec::new(),
        }
    }
}

/// A `BTreeMap<ObjectKey, V>` as a list of `(key, value)` pairs.
mod pairs {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use crate::ids::ObjectKey;

    pub fn serialize<V: Serialize, S: Serializer>(
        map: &BTreeMap<ObjectKey, V>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        s.collect_seq(map.iter())
    }

    pub fn deserialize<'de, V: Deserialize<'de>, D: Deserializer<'de>>(
        d: D,
    ) -> Result<BTreeMap<ObjectKey, V>, D::Error> {
        let list: Vec<(ObjectKey, V)> = Vec::deserialize(d)?;
        Ok(list.into_iter().collect())
    }
}
