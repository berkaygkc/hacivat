// ## m4-t01c-core (seam), filled by m4-t16-pg (spike origin 5a5ab08)
//! The PostgreSQL introspector (HV-INTRO-05; Task 16, facets Task 16b): passes over
//! `pg_catalog` by oid inside a `REPEATABLE READ READ ONLY` snapshot with the pinned
//! `search_path` (R-M4-46), the `pg_current_snapshot()` watermark (R-M4-8) and one-hop change
//! detection (R-M4-48). PostgreSQL 13–18 share one code path; the packs gate
//! `indnullsnotdistinct` (15+), `conenforced` (18+) and read the catalogued `NOT NULL`
//! constraints wherever they exist (domains 17+, columns 18+), R-M4-47.
//!
//! - [`pass`]: the passes ([`pass::introspect`], [`pass::fragment`], [`pass::stamps`],
//!   [`pass::list_namespaces`], [`pass::check_namespace`]) with their reports (the Docker
//!   tests' query log, the full reason, the row estimates).
//! - [`sections`]: rendering and running the `packs/pg` sections; [`decode`], [`text`]: the
//!   rows; [`model`]: the rows as structs; [`build`]: the model objects (keys through Task
//!   13's assembly); [`watermark`], [`changes`]: the full-or-incremental decision and the
//!   facet cascade; [`state`]: the `StateOf<Kind>` stamps; [`system`]: `Mode::System`.
//! - `facet` reads go to [`facets::read`] and the server namespace to
//!   [`facets::server_namespace`] (Task 16b; `NotImplemented` until then).

// `IntroError` is the seam's error type (Task 1c); the decoders return it per cell.
#![allow(
    clippy::result_large_err,
    reason = "IntroError is the fixed introspector seam type"
)]

pub mod build;
pub mod changes;
pub mod decode;
pub mod facets;
pub mod model;
pub mod pass;
pub mod sections;
pub mod state;
pub mod system;
pub mod text;
pub mod watermark;

use futures::future::BoxFuture;
use hacivat_meta::merge::NamespaceResult;
use hacivat_meta::props::FacetKind;
use hacivat_meta::{Level, NamespaceSnapshot, NsKey, NsKind, ObjectKey, StateMark};
use hacivat_proto::DbmsKind;
use hacivat_session::meta::MetaConn;
use tokio_util::sync::CancellationToken;

use crate::freshness::NsCheck;
use crate::introspector::{FacetRows, IntroCaps, IntroError, Introspector, Staleness};
use crate::pack::Mode;
use crate::scope::NsInfo;

/// The PostgreSQL introspector.
#[derive(Debug, Clone, Copy, Default)]
pub struct PgIntrospector;

impl Introspector for PgIntrospector {
    fn list_namespaces<'a>(
        &'a self,
        c: &'a mut MetaConn,
    ) -> BoxFuture<'a, Result<Vec<NsInfo>, IntroError>> {
        Box::pin(async move {
            let cancel = CancellationToken::new();
            pass::list_namespaces(c, &cancel).await
        })
    }

    fn introspect<'a>(
        &'a self,
        c: &'a mut MetaConn,
        ns: &'a NsKey,
        mode: Mode,
        level: Level,
        prev: Option<&'a NamespaceSnapshot>,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<NamespaceResult, IntroError>> {
        Box::pin(async move {
            if ns.kind == NsKind::Server {
                return facets::server_namespace(c).await;
            }
            let opts = pass::PassOpts {
                mode,
                level,
                prev,
                filter: None,
                after_identity: None,
            };
            pass::introspect(c, ns, opts, cancel)
                .await
                .map(|p| p.result)
        })
    }

    fn fragment<'a>(
        &'a self,
        c: &'a mut MetaConn,
        ns: &'a NsKey,
        keys: &'a [ObjectKey],
        level: Level,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<NamespaceResult, IntroError>> {
        Box::pin(async move {
            pass::fragment(c, ns, keys, level, None, cancel)
                .await
                .map(|p| p.result)
        })
    }

    fn stamps<'a>(
        &'a self,
        c: &'a mut MetaConn,
        keys: &'a [ObjectKey],
    ) -> BoxFuture<'a, Result<Vec<Option<StateMark>>, IntroError>> {
        Box::pin(async move {
            let cancel = CancellationToken::new();
            state::stamps(c, keys, &cancel).await.map(|(m, _)| m)
        })
    }

    fn facet<'a>(
        &'a self,
        c: &'a mut MetaConn,
        ns: &'a NsKey,
        keys: &'a [ObjectKey],
        kind: FacetKind,
    ) -> BoxFuture<'a, Result<FacetRows, IntroError>> {
        facets::read(c, ns, keys, kind)
    }

    /// `Dropped` where the object's own row is gone; `Unknown` otherwise: comparing a stamp
    /// with the snapshot's mark is Task 22's (`freshness::check_state`), and editing flows treat
    /// `Unknown` as `Changed`.
    fn check_state<'a>(
        &'a self,
        c: &'a mut MetaConn,
        keys: &'a [ObjectKey],
    ) -> BoxFuture<'a, Result<Vec<Staleness>, IntroError>> {
        Box::pin(async move {
            let cancel = CancellationToken::new();
            let (marks, _) = state::stamps(c, keys, &cancel).await?;
            Ok(marks
                .into_iter()
                .map(|m| match m {
                    None => Staleness::Dropped,
                    Some(_) => Staleness::Unknown,
                })
                .collect())
        })
    }

    fn check_namespace<'a>(
        &'a self,
        c: &'a mut MetaConn,
        ns: &'a NsKey,
        snap: &'a NamespaceSnapshot,
    ) -> BoxFuture<'a, Result<NsCheck, IntroError>> {
        Box::pin(async move {
            let cancel = CancellationToken::new();
            pass::check_namespace(c, ns, snap, &cancel)
                .await
                .map(|(n, _)| n)
        })
    }

    fn capabilities(&self) -> IntroCaps {
        IntroCaps {
            dbms: DbmsKind::Postgres,
            transactional_ddl: true,
            incremental: true,
            // Grants and dependencies are Task 16b's; row estimates ride with the relations.
            facets: Vec::new(),
        }
    }
}
