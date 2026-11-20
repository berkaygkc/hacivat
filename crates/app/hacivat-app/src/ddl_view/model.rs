//! The DDL view's pure parts: which options a dialect shows, the generator options they make,
//! the target, and the script of one object (Task 19's `create_script_with`, nothing else: the
//! text shown is exactly its bytes).
//!
//! ## m4-t25-ddl-view

use std::sync::Arc;

use hacivat_ddl::produce::{DdlSources, UNSUPPORTED_KIND, create_script_with};
use hacivat_ddl::script::{DelimStyle, Script};
use hacivat_ddl::{DdlError, DdlTarget, GenOptions};
use hacivat_meta::props::FacetKind;
use hacivat_meta::snapshot::{ContainerKey, LoadState};
use hacivat_meta::userdata::DdlViewOptions;
use hacivat_meta::view::SnapshotView;
use hacivat_meta::{CatalogSnapshot, Name, NsKey, NsKind, ObjectKey, ObjectKind};
use hacivat_proto::DbmsKind;
use sql_dialect::{DialectId, ServerVersion};

use crate::explorer::live::dialect_of;

/// Which options of the strip a dialect shows (mock 10; HV-DDL-11 applicability).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shown {
    /// "Definitions provided by server": MySQL/MariaDB and SQLite; PostgreSQL has no table
    /// definition function (R-M4-24).
    pub server: bool,
    /// The server toggle can be changed: SQLite tables always show the server's text (their
    /// CHECK and generated expressions exist only there, F10), so on SQLite it is checked and
    /// fixed.
    pub server_fixed: bool,
    /// Grants: PostgreSQL and MySQL/MariaDB (SQLite has none).
    pub grants: bool,
    /// Owner (PostgreSQL) or definer (MySQL/MariaDB).
    pub owner: bool,
}

/// What the strip shows on `dbms`.
pub fn shown(dbms: DbmsKind) -> Shown {
    match dialect_of(dbms) {
        DialectId::Sqlite => Shown {
            server: true,
            server_fixed: true,
            grants: false,
            owner: false,
        },
        DialectId::MySql | DialectId::MariaDb => Shown {
            server: true,
            server_fixed: false,
            grants: true,
            owner: true,
        },
        _ => Shown {
            server: false,
            server_fixed: false,
            grants: true,
            owner: true,
        },
    }
}

/// The options in effect on `dbms`: an option the dialect does not show is the safe default
/// (no server text on PostgreSQL; SQLite's server text is always on).
pub fn effective(dbms: DbmsKind, opts: DdlViewOptions) -> DdlViewOptions {
    let s = shown(dbms);
    DdlViewOptions {
        server_definitions: if s.server_fixed {
            true
        } else {
            s.server && opts.server_definitions
        },
        ..opts
    }
}

/// The generator options of the strip (R-M4-23 defaults through `DdlViewOptions::default`).
pub fn gen_options(dbms: DbmsKind, opts: DdlViewOptions) -> GenOptions {
    let o = effective(dbms, opts);
    GenOptions {
        qualify: o.qualify,
        placement: o.placement,
        skip_grants: !o.grants,
        skip_owner: !o.owner,
        skip_definer: !o.owner,
        server_definitions: o.server_definitions,
        ..GenOptions::default()
    }
}

/// `ServerFacts::version_num` as a version: PostgreSQL `170002` is 17.0.2 (10 and later),
/// `90624` 9.6.24; MySQL/MariaDB `80400` is 8.4.0; 0 (unknown) is 0.0.0.
pub fn server_version(dbms: DbmsKind, num: u32) -> ServerVersion {
    let part = |n: u32| u16::try_from(n).unwrap_or(u16::MAX);
    match dbms {
        DbmsKind::Postgres if num >= 100_000 => {
            ServerVersion::new(part(num / 10_000), 0, part(num % 10_000))
        }
        DbmsKind::Postgres | DbmsKind::MySql | DbmsKind::MariaDb => {
            ServerVersion::new(part(num / 10_000), part((num / 100) % 100), part(num % 100))
        }
        _ => ServerVersion::new(0, 0, 0),
    }
}

/// The target of `snap`'s server for display (`current_schema`: the user's session's).
pub fn target(dbms: DbmsKind, snap: &CatalogSnapshot, current_schema: Option<Name>) -> DdlTarget {
    let mut t = DdlTarget::new(
        dialect_of(dbms),
        server_version(dbms, snap.server.version_num),
    );
    if snap.server.dbms == dbms {
        t.flavor = snap.server.flavor;
    }
    t.current_schema = current_schema;
    t
}

/// The kinds whose definition is the server's text (R-M4-24): an object of one of them is read
/// at `Sources` before its DDL is shown (elevation).
pub fn needs_sources(kind: ObjectKind) -> bool {
    matches!(
        kind,
        ObjectKind::View
            | ObjectKind::MaterializedView
            | ObjectKind::Routine
            | ObjectKind::Procedure
            | ObjectKind::Trigger
            | ObjectKind::Event
    )
}

/// Whether the shown text is the server's (the source caption; Qualification and Constraints
/// hide, NOTES ruling 16).
pub fn is_server_text(dbms: DbmsKind, kind: ObjectKind, opts: DdlViewOptions) -> bool {
    let o = effective(dbms, opts);
    needs_sources(kind)
        || (dialect_of(dbms) == DialectId::Sqlite
            && matches!(kind, ObjectKind::Table | ObjectKind::Index))
        || (o.server_definitions && kind == ObjectKind::Table)
}

/// The `$kind` selector of `hv-ddl-view-unsupported` (one whole sentence per kind).
pub fn unsupported_selector(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Domain => "domain",
        ObjectKind::Extension => "extension",
        ObjectKind::Operator => "operator",
        ObjectKind::Policy | ObjectKind::RowPolicy => "policy",
        ObjectKind::Rule => "rule",
        ObjectKind::EnumType
        | ObjectKind::RangeType
        | ObjectKind::CompositeType
        | ObjectKind::AliasType
        | ObjectKind::TableType => "type",
        _ => "other",
    }
}

/// What generating one object gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Generated {
    /// The script; its text is `Script::text(DelimStyle::Semicolon)`.
    Script(Script),
    /// No DDL for this kind on this engine.
    Unsupported,
    /// The object, or a part of its definition, is not loaded.
    NotLoaded,
    /// A name that cannot be written (a NUL), or a renderer refusal.
    Failed,
}

/// The CREATE script of `key` as `snap` holds it (`create_script_with`, no other text).
pub fn generate(
    dbms: DbmsKind,
    snap: &Arc<CatalogSnapshot>,
    sources: &dyn DdlSources,
    key: &ObjectKey,
    opts: DdlViewOptions,
    current_schema: Option<Name>,
) -> Generated {
    let view = SnapshotView::new(snap.clone());
    let t = target(dbms, snap, current_schema);
    match create_script_with(
        &view,
        sources,
        std::slice::from_ref(key),
        &t,
        &gen_options(dbms, opts),
    ) {
        Ok(s) if s.stmts.is_empty() && s.has_notice(UNSUPPORTED_KIND) => Generated::Unsupported,
        Ok(s) => Generated::Script(s),
        Err(DdlError::UnsupportedKind) => Generated::Unsupported,
        Err(DdlError::NotLoaded) => Generated::NotLoaded,
        Err(_) => Generated::Failed,
    }
}

/// The text a script shows (and Copy and Open in console use).
pub fn text_of(s: &Script) -> String {
    s.text(DelimStyle::Semicolon)
}

/// The namespace `key` lives in, as `snap` knows it: the longest prefix of its path that is a
/// namespace of the snapshot.
pub fn ns_of(snap: &CatalogSnapshot, key: &ObjectKey) -> Option<NsKey> {
    if let Some(h) = snap.find(key) {
        return Some(h.ns.key().clone());
    }
    for k in (0..key.path.len()).rev() {
        for kind in [NsKind::Schema, NsKind::Database, NsKind::Server] {
            let ns = NsKey {
                kind,
                path: key.path[..k].iter().cloned().collect(),
            };
            let known = snap.namespaces.contains_key(&ns)
                || snap
                    .states
                    .contains_key(&ContainerKey::Namespace(ns.clone()));
            if known {
                return Some(ns);
            }
        }
    }
    None
}

/// The load state of the namespace `key` lives in.
pub fn ns_state(snap: &CatalogSnapshot, key: &ObjectKey) -> LoadState {
    ns_of(snap, key)
        .map(|ns| snap.state(&ContainerKey::Namespace(ns)).clone())
        .unwrap_or_default()
}

/// The level `key`'s namespace is loaded at (`None`: not loaded).
pub fn loaded_level(snap: &CatalogSnapshot, key: &ObjectKey) -> Option<hacivat_meta::Level> {
    match ns_state(snap, key) {
        LoadState::Loaded { level, .. } => Some(level),
        _ => None,
    }
}

/// Whether `snap` says `key` no longer exists: its namespace is loaded (a fact the engine only
/// publishes after a pass that read it) and holds no such object.
pub fn is_gone(snap: &CatalogSnapshot, key: &ObjectKey) -> bool {
    snap.find(key).is_none()
        && ns_of(snap, key).is_some_and(|ns| {
            snap.namespaces.contains_key(&ns)
                && matches!(
                    snap.state(&ContainerKey::Namespace(ns)),
                    LoadState::Loaded { .. }
                )
        })
}

/// The generation `key`'s namespace arena was published in (0: none).
pub fn ns_generation(snap: &CatalogSnapshot, key: &ObjectKey) -> u64 {
    snap.find(key).map_or(0, |h| h.ns.generation())
}

/// Whether `key`'s namespace is marked stale (a cached namespace not validated yet, a held
/// deletion, refresh after DDL with "Ask" or "Never").
pub fn is_stale(snap: &CatalogSnapshot, key: &ObjectKey) -> bool {
    !matches!(ns_state(snap, key), LoadState::Loaded { stale: false, .. })
}

/// Whether facet `facet` of `key` is loaded and not stale in `snap` (the load state the
/// facet's text has; `DdlSources` carries none, T19 review m6).
pub fn facet_vouches(snap: &CatalogSnapshot, key: &ObjectKey, facet: FacetKind) -> bool {
    let Some(ns) = ns_of(snap, key) else {
        return false;
    };
    // The facet is recorded under the object's own key (with its server id when it has one).
    let object = snap
        .find(key)
        .and_then(|h| h.key())
        .unwrap_or_else(|| key.clone());
    matches!(
        snap.state(&ContainerKey::Facet { ns, object, facet }),
        LoadState::Loaded { stale: false, .. }
    )
}

/// The key the DDL view of `key` shows: a column's is its table's (or view's).
pub fn ddl_object(snap: Option<&CatalogSnapshot>, key: &ObjectKey) -> ObjectKey {
    if key.kind != ObjectKind::Column {
        return key.clone();
    }
    snap.and_then(|s| {
        let h = s.find(key)?;
        let parent = h.obj().parent()?;
        h.ns.object_key(parent)
    })
    .unwrap_or_else(|| key.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_from_their_numbers() {
        assert_eq!(
            server_version(DbmsKind::Postgres, 170_002),
            ServerVersion::new(17, 0, 2)
        );
        assert_eq!(
            server_version(DbmsKind::Postgres, 90_624),
            ServerVersion::new(9, 6, 24)
        );
        assert_eq!(
            server_version(DbmsKind::MySql, 80_400),
            ServerVersion::new(8, 4, 0)
        );
        assert_eq!(
            server_version(DbmsKind::MariaDb, 101_106),
            ServerVersion::new(10, 11, 6)
        );
        assert_eq!(
            server_version(DbmsKind::Sqlite, 3_045_000),
            ServerVersion::new(0, 0, 0)
        );
    }

    #[test]
    fn options_follow_the_dialect() {
        let all_on = DdlViewOptions {
            server_definitions: true,
            ..DdlViewOptions::default()
        };
        // PostgreSQL never takes server text for a table; SQLite always does.
        assert!(!effective(DbmsKind::Postgres, all_on).server_definitions);
        assert!(effective(DbmsKind::Sqlite, DdlViewOptions::default()).server_definitions);
        assert!(effective(DbmsKind::MySql, all_on).server_definitions);
        let off = DdlViewOptions {
            grants: false,
            owner: false,
            ..DdlViewOptions::default()
        };
        let g = gen_options(DbmsKind::Postgres, off);
        assert!(g.skip_grants && g.skip_owner && g.skip_definer);
        let g = gen_options(DbmsKind::Postgres, DdlViewOptions::default());
        assert!(!g.skip_grants && !g.skip_owner && !g.server_definitions);
    }
}
