// ## m4-t01c-core
//! The per-engine introspector seam (HV-INTRO-01, -05, -07, -08, -15): what the engine
//! (Task 15) asks of a dialect, always on a read-only metadata connection (`MetaConn`, R-M4-7).
//! Feature `session` (it names `MetaConn`).
//!
//! **Fail-closed stubs (M4 Task 1c).** Every method of every introspector answers
//! [`IntroError::NotImplemented`] (Tasks 16–18 fill them; `stamps` and `facet` included), so no
//! namespace is ever published as loaded from a stub.

use std::sync::Arc;

use futures::future::BoxFuture;
use hacivat_meta::merge::NamespaceResult;
use hacivat_meta::persist::ServerFingerprint;
use hacivat_meta::props::{DepsFacet, FacetKind, GrantsFacet, StatsFacet};
use hacivat_meta::snapshot::IntroErrorCode;
use hacivat_meta::{Level, Name, NamespaceSnapshot, NsKey, ObjectKey, ObjectKind, StateMark};
use hacivat_proto::DbmsKind;
use hacivat_session::meta::{MetaConn, MetaError};
use tokio_util::sync::CancellationToken;

use crate::filter::CompiledFilter;
use crate::freshness::NsCheck;
use crate::pack::Mode;
use crate::scope::NsInfo;

/// Why an introspection step failed. Codes only: Fluent at the edge (`intro-*`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntroError {
    /// Not written yet (M4 Task 1c stubs).
    NotImplemented,
    /// The metadata connection failed.
    Meta(MetaError),
    /// The login cannot read objects of `kind` (permission loss is never a drop, HV-INTRO-11).
    Permission {
        kind: ObjectKind,
    },
    Timeout,
    Cancelled,
    /// PostgreSQL `XX000` "cache lookup failed": an object was dropped during the pass (PG
    /// spike finding 8); Task 15 retries once.
    CatalogRace,
    /// The server answered something the pack section does not declare.
    Protocol(Name),
}

impl IntroError {
    /// The model's error code for a failed namespace (`LoadState::Failed`).
    pub fn code(&self) -> IntroErrorCode {
        match self {
            IntroError::NotImplemented => IntroErrorCode::NotImplemented,
            IntroError::Meta(MetaError::Timeout) | IntroError::Timeout => IntroErrorCode::Timeout,
            IntroError::Meta(MetaError::Cancelled) | IntroError::Cancelled => {
                IntroErrorCode::Cancelled
            }
            IntroError::Meta(MetaError::Server { .. }) | IntroError::CatalogRace => {
                IntroErrorCode::ServerError
            }
            IntroError::Meta(_) => IntroErrorCode::Connection,
            IntroError::Permission { .. } => IntroErrorCode::PermissionDenied,
            IntroError::Protocol(_) => IntroErrorCode::Unknown,
        }
    }
}

impl From<MetaError> for IntroError {
    fn from(e: MetaError) -> Self {
        IntroError::Meta(e)
    }
}

/// What `check_state` found for one object (HV-INTRO-15). Editing flows treat `Unknown` as
/// `Changed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Staleness {
    Current,
    Changed,
    Dropped,
    /// Not decidable (no stamp, permission loss, not loaded): never `Dropped` by default.
    Unknown,
}

/// The rows of one facet read (R-M4-30), per object.
#[derive(Debug, Clone, PartialEq)]
pub enum FacetRows {
    Sources(Vec<(ObjectKey, Name)>),
    Grants(Vec<(ObjectKey, GrantsFacet)>),
    Dependencies(Vec<(ObjectKey, DepsFacet)>),
    Statistics(Vec<(ObjectKey, StatsFacet)>),
}

/// What an introspector can do on its engine.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IntroCaps {
    pub dbms: DbmsKind,
    /// DDL runs inside a transaction and is invisible until `COMMIT` (PostgreSQL, SQLite):
    /// refresh after DDL waits for the commit (HV-INTRO-12).
    pub transactional_ddl: bool,
    /// An `Incremental` pass is available (PostgreSQL watermark, MySQL markers; MariaDB is
    /// `Full` per namespace in v1, R-M4-10).
    pub incremental: bool,
    /// The facets [`Introspector::facet`] can read.
    pub facets: Vec<FacetKind>,
}

impl IntroCaps {
    /// What M4 plans for `dbms` (R-M4-9, R-M4-10, R-M4-30). The stubs read nothing, so their
    /// `incremental` and `facets` are off until the engine's introspector is written.
    pub fn planned(dbms: DbmsKind) -> Self {
        IntroCaps {
            dbms,
            transactional_ddl: matches!(dbms, DbmsKind::Postgres | DbmsKind::Sqlite),
            incremental: false,
            facets: Vec::new(),
        }
    }
}

/// One engine's introspector. Every method runs on a `MetaConn` and never on a user session
/// (R-M4-7); the `BoxFuture` style of `hacivat-driver`'s `RawConn`.
pub trait Introspector: Send + Sync {
    /// The namespaces the login can see (HV-INTRO-02 input).
    fn list_namespaces<'a>(
        &'a self,
        c: &'a mut MetaConn,
    ) -> BoxFuture<'a, Result<Vec<NsInfo>, IntroError>>;

    /// One pass over `ns` (HV-INTRO-05, -07, -08): `Full`, `Incremental` against `prev`'s
    /// watermark, `Brief` (names) or `System` (the snapshot sections, R-M4-44).
    ///
    /// ## m4-t15-engine (ruling R-W3-2): `filter`
    /// The profile's scope filter for this namespace (match-all for a system namespace, and
    /// whenever no scope filter is set; never a view filter). A section may push it to the
    /// server only through `engine::bounded_pushdown(filter, <every kind the section
    /// returns>, dialect, ident)`; `None` means no predicate. A pushed predicate cuts the
    /// section's existence rows and changed rows alike. The engine filters the result on the
    /// client anyway: pushdown is an optimisation that must only ever return a superset.
    #[allow(clippy::too_many_arguments, reason = "the planned seam (M4 Task 1c)")]
    fn introspect<'a>(
        &'a self,
        c: &'a mut MetaConn,
        ns: &'a NsKey,
        mode: Mode,
        level: Level,
        filter: &'a CompiledFilter,
        prev: Option<&'a NamespaceSnapshot>,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<NamespaceResult, IntroError>>;

    /// The objects `keys` of `ns` (and their children) at `level`; `filter` as for
    /// [`Introspector::introspect`] (R-W3-2).
    #[allow(
        clippy::too_many_arguments,
        reason = "the planned seam plus R-W3-2's filter"
    )]
    fn fragment<'a>(
        &'a self,
        c: &'a mut MetaConn,
        ns: &'a NsKey,
        keys: &'a [ObjectKey],
        level: Level,
        filter: &'a CompiledFilter,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<NamespaceResult, IntroError>>;

    /// The raw state marks of `keys` (`None`: the object does not exist), for `check_state`.
    fn stamps<'a>(
        &'a self,
        c: &'a mut MetaConn,
        keys: &'a [ObjectKey],
    ) -> BoxFuture<'a, Result<Vec<Option<StateMark>>, IntroError>>;

    /// One facet of `keys` (R-M4-30).
    fn facet<'a>(
        &'a self,
        c: &'a mut MetaConn,
        ns: &'a NsKey,
        keys: &'a [ObjectKey],
        kind: FacetKind,
    ) -> BoxFuture<'a, Result<FacetRows, IntroError>>;

    /// Whether `keys` changed on the server since they were loaded (HV-INTRO-15).
    fn check_state<'a>(
        &'a self,
        c: &'a mut MetaConn,
        keys: &'a [ObjectKey],
    ) -> BoxFuture<'a, Result<Vec<Staleness>, IntroError>>;

    /// The cheap validation of a cached namespace: object count, highest mark, existence
    /// digest (HV-INTRO-15).
    fn check_namespace<'a>(
        &'a self,
        c: &'a mut MetaConn,
        ns: &'a NsKey,
        snap: &'a NamespaceSnapshot,
    ) -> BoxFuture<'a, Result<NsCheck, IntroError>>;

    fn capabilities(&self) -> IntroCaps;

    // ## m4-t15-engine (ruling R-W3-3)
    /// The identity of the server behind `c` (PostgreSQL `system_identifier` and start time,
    /// MySQL `@@server_uuid`, SQLite file identity), hashed by the introspector, for
    /// `CacheStore::check_identity`. The default is [`ServerIdentity::Unknown`], which the
    /// engine treats fail-closed: no cached watermark is trusted (the next pass is full).
    /// Tasks 16–18 implement it.
    fn identity<'a>(
        &'a self,
        c: &'a mut MetaConn,
    ) -> BoxFuture<'a, Result<ServerIdentity, IntroError>> {
        let _ = c;
        Box::pin(async { Ok(ServerIdentity::Unknown) })
    }
}

// ## m4-t15-engine (ruling R-W3-3)
/// What an introspector knows of its server's identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServerIdentity {
    Known(ServerFingerprint),
    /// Not read (a stub, a login that cannot read it): no cached watermark is trusted.
    Unknown,
}

/// The introspector of `dbms`; `None` for an engine M4 does not introspect (R-M4-4).
pub fn introspector_for(dbms: DbmsKind) -> Option<Arc<dyn Introspector>> {
    match dbms {
        DbmsKind::Postgres => Some(Arc::new(crate::pg::PgIntrospector)),
        DbmsKind::MySql | DbmsKind::MariaDb => {
            Some(Arc::new(crate::mysql::MySqlIntrospector { flavor: dbms }))
        }
        // ## m4-t18-sqlite (named exception): the introspector holds options now.
        DbmsKind::Sqlite => Some(Arc::new(crate::sqlite::SqliteIntrospector::default())),
        _ => None,
    }
}

/// Implements every [`Introspector`] method of `$ty` as [`IntroError::NotImplemented`], with
/// `$caps` as its capabilities (the M4 Task 1c stubs; Tasks 15–18 replace each).
macro_rules! not_implemented_introspector {
    ($ty:ty, |$s:ident| $caps:expr) => {
        impl $crate::introspector::Introspector for $ty {
            fn list_namespaces<'a>(
                &'a self,
                _c: &'a mut ::hacivat_session::meta::MetaConn,
            ) -> ::futures::future::BoxFuture<
                'a,
                Result<Vec<$crate::scope::NsInfo>, $crate::introspector::IntroError>,
            > {
                Box::pin(async { Err($crate::introspector::IntroError::NotImplemented) })
            }
            fn introspect<'a>(
                &'a self,
                _c: &'a mut ::hacivat_session::meta::MetaConn,
                _ns: &'a ::hacivat_meta::NsKey,
                _mode: $crate::pack::Mode,
                _level: ::hacivat_meta::Level,
                _filter: &'a $crate::filter::CompiledFilter,
                _prev: Option<&'a ::hacivat_meta::NamespaceSnapshot>,
                _cancel: &'a ::tokio_util::sync::CancellationToken,
            ) -> ::futures::future::BoxFuture<
                'a,
                Result<::hacivat_meta::merge::NamespaceResult, $crate::introspector::IntroError>,
            > {
                Box::pin(async { Err($crate::introspector::IntroError::NotImplemented) })
            }
            fn fragment<'a>(
                &'a self,
                _c: &'a mut ::hacivat_session::meta::MetaConn,
                _ns: &'a ::hacivat_meta::NsKey,
                _keys: &'a [::hacivat_meta::ObjectKey],
                _level: ::hacivat_meta::Level,
                _filter: &'a $crate::filter::CompiledFilter,
                _cancel: &'a ::tokio_util::sync::CancellationToken,
            ) -> ::futures::future::BoxFuture<
                'a,
                Result<::hacivat_meta::merge::NamespaceResult, $crate::introspector::IntroError>,
            > {
                Box::pin(async { Err($crate::introspector::IntroError::NotImplemented) })
            }
            fn stamps<'a>(
                &'a self,
                _c: &'a mut ::hacivat_session::meta::MetaConn,
                _keys: &'a [::hacivat_meta::ObjectKey],
            ) -> ::futures::future::BoxFuture<
                'a,
                Result<Vec<Option<::hacivat_meta::StateMark>>, $crate::introspector::IntroError>,
            > {
                Box::pin(async { Err($crate::introspector::IntroError::NotImplemented) })
            }
            fn facet<'a>(
                &'a self,
                _c: &'a mut ::hacivat_session::meta::MetaConn,
                _ns: &'a ::hacivat_meta::NsKey,
                _keys: &'a [::hacivat_meta::ObjectKey],
                _kind: ::hacivat_meta::props::FacetKind,
            ) -> ::futures::future::BoxFuture<
                'a,
                Result<$crate::introspector::FacetRows, $crate::introspector::IntroError>,
            > {
                Box::pin(async { Err($crate::introspector::IntroError::NotImplemented) })
            }
            fn check_state<'a>(
                &'a self,
                _c: &'a mut ::hacivat_session::meta::MetaConn,
                _keys: &'a [::hacivat_meta::ObjectKey],
            ) -> ::futures::future::BoxFuture<
                'a,
                Result<Vec<$crate::introspector::Staleness>, $crate::introspector::IntroError>,
            > {
                Box::pin(async { Err($crate::introspector::IntroError::NotImplemented) })
            }
            fn check_namespace<'a>(
                &'a self,
                _c: &'a mut ::hacivat_session::meta::MetaConn,
                _ns: &'a ::hacivat_meta::NsKey,
                _snap: &'a ::hacivat_meta::NamespaceSnapshot,
            ) -> ::futures::future::BoxFuture<
                'a,
                Result<$crate::freshness::NsCheck, $crate::introspector::IntroError>,
            > {
                Box::pin(async { Err($crate::introspector::IntroError::NotImplemented) })
            }
            fn capabilities(&self) -> $crate::introspector::IntroCaps {
                let $s = self;
                $caps
            }
        }
    };
}
pub(crate) use not_implemented_introspector;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_m4_engine_has_an_introspector_and_no_other() {
        for (dbms, transactional) in [
            (DbmsKind::Postgres, true),
            (DbmsKind::MySql, false),
            (DbmsKind::MariaDb, false),
            (DbmsKind::Sqlite, true),
        ] {
            let caps = introspector_for(dbms).expect("an M4 engine").capabilities();
            assert_eq!(caps.dbms, dbms);
            assert_eq!(caps.transactional_ddl, transactional, "{dbms:?}");
            // ## m4-t16-pg (named exception): PostgreSQL is incremental since Task 16.
            // ## m4-t18-sqlite (named exception): SQLite's introspector is written (Task 18).
            match dbms {
                DbmsKind::Postgres => {
                    assert!(caps.incremental && caps.facets.is_empty(), "{dbms:?}");
                }
                DbmsKind::Sqlite => {}
                _ => assert!(!caps.incremental && caps.facets.is_empty(), "{dbms:?}"),
            }
        }
        assert!(introspector_for(DbmsKind::MsSql).is_none());
    }

    #[test]
    fn error_codes_map_to_the_model() {
        assert_eq!(
            IntroError::NotImplemented.code(),
            IntroErrorCode::NotImplemented
        );
        assert_eq!(
            IntroError::Meta(MetaError::Unavailable).code(),
            IntroErrorCode::Connection
        );
        assert_eq!(
            IntroError::Permission {
                kind: ObjectKind::Table
            }
            .code(),
            IntroErrorCode::PermissionDenied
        );
    }
}
