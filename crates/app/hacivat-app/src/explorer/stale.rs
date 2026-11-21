//! Stale objects (Task 27; mock 11): "{name} changed on the server since it was loaded" and
//! "{name} no longer exists on the server", as explorer tooltips.
//!
//! ## m4-t27-refresh-ui
//!
//! A state check (Task 22's verdicts; the DDL view's check, `ddl_view::view`) that found an
//! object changed and could not read it again, or dropped while the catalog still holds it,
//! is [`record`]ed for the object's namespace generation of that moment. The object's explorer
//! row then carries the DDL view's sentence as its tooltip (and accessible description) until
//! its namespace is published again (a refresh or a pass reads it: the verdict no longer
//! describes what the explorer shows) or a later check finds it current.

use hacivat_introspect::introspector::Staleness;
use hacivat_meta::{CatalogSnapshot, ObjectKey};
use hacivat_proto::ProfileId;
use hacivat_ui_core::gpui_surface::App;

use super::live;

/// What a check found out about one object, for the namespace generation it was shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleObject {
    /// Changed on the server; not read again.
    Changed { generation: u64 },
    /// No longer on the server; the catalog still holds it.
    Gone { generation: u64 },
}

impl StaleObject {
    /// The generation of the object's namespace the verdict was about.
    pub fn generation(self) -> u64 {
        match self {
            StaleObject::Changed { generation } | StaleObject::Gone { generation } => generation,
        }
    }

    /// The tooltip's message (the DDL view's sentence, surface 11).
    pub fn message_id(self) -> &'static str {
        match self {
            StaleObject::Changed { .. } => "hv-ddl-view-stale",
            StaleObject::Gone { .. } => "hv-ddl-view-gone",
        }
    }
}

/// Records what a state check of `key` (in `profile`'s catalog `snap`) decided: `verdict`, and
/// whether the object was read again after a `Changed` verdict (`read`).
pub fn record(
    profile: ProfileId,
    key: &ObjectKey,
    verdict: Staleness,
    read: bool,
    snap: &CatalogSnapshot,
    cx: &mut App,
) {
    let Some(found) = snap.find(key) else {
        // Not in the catalog: nothing in the explorer to mark.
        live::set_stale_object(profile, key.clone(), None, cx);
        return;
    };
    let generation = found.ns.generation();
    let key = found.key().unwrap_or_else(|| key.clone());
    let mark = match verdict {
        Staleness::Changed if !read => Some(StaleObject::Changed { generation }),
        Staleness::Dropped => Some(StaleObject::Gone { generation }),
        // Current, a re-read that succeeded, or a check that decided nothing: no mark (an
        // undecided check never claims a change).
        _ => None,
    };
    live::set_stale_object(profile, key, mark, cx);
}

/// The mark of `key` while it still describes what the explorer shows (the namespace was not
/// published again since the check).
pub fn current_mark(
    marks: &std::collections::BTreeMap<ObjectKey, StaleObject>,
    key: &ObjectKey,
    ns_generation: u64,
) -> Option<StaleObject> {
    marks
        .get(key)
        .copied()
        .filter(|m| m.generation() == ns_generation)
}
