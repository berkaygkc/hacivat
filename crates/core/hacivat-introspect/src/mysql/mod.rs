// ## m4-t01c-core
//! The MySQL and MariaDB introspector (HV-INTRO-07; Task 17, facets Task 17b): passes over
//! `INFORMATION_SCHEMA` on a read-only session with a consistent-snapshot read-only transaction
//! (R-M4-7), the R-M4-39 markers on MySQL, `Full` per namespace on MariaDB (R-M4-10), and
//! `SHOW CREATE` text through the quoted-identifier macro (R-M4-37).
//!
//! ## m4-t17-mysql (bodies, M4 Task 17; spike origin `f690f45`; ADR 0008 `m4-t17`)
//!
//! Every method opens one snapshot of the `MetaConn` (MySQL: `START TRANSACTION WITH
//! CONSISTENT SNAPSHOT, READ ONLY` on a read-only UTF-8 UTC session, Task 5), reads the
//! server context (`Identity`; the flavor is `VERSION()`'s) and runs pack sections only
//! ([`TxnReader`]). The pass logic is in [`pass`], written against [`pass::SectionReader`];
//! the model objects in [`build`]; the stamps and the namespace check in [`state`]; server
//! text in [`show_create`]. `facet` and the `Server` namespace are Task 17b's ([`facets`]).

#![allow(
    clippy::result_large_err,
    reason = "IntroError is the seam's error type (Task 1c); it carries the server's message"
)]

pub mod build;
pub mod facets;
pub mod pass;
pub mod rows;
pub mod server;
pub mod show_create;
pub mod state;

use std::collections::{BTreeMap, BTreeSet};

use futures::future::BoxFuture;
use hacivat_meta::merge::NamespaceResult;
use hacivat_meta::props::FacetKind;
use hacivat_meta::{Level, Name, NamespaceSnapshot, NsKey, NsKind, ObjectKey, ObjectKind};
use hacivat_meta::{ServerFacts, StateMark};
use hacivat_proto::DbmsKind;
use hacivat_session::meta::{MetaConn, Rows, SnapshotTxn};
use sql_dialect::DialectId;
use tokio_util::sync::CancellationToken;

use crate::filter::CompiledFilter;
use crate::freshness::NsCheck;
use crate::introspector::{
    FacetRows, IntroCaps, IntroError, Introspector, ServerIdentity, Staleness,
};
use crate::pack::{Mode, Pack, PackCtx};
use crate::scope::NsInfo;

pub use pass::{PassStats, SYSTEM_SCHEMAS, SectionReader, Setting, relation_kinds};
pub use server::ServerCtx;

/// The MySQL and MariaDB introspector.
#[derive(Debug, Clone, Copy)]
pub struct MySqlIntrospector {
    /// `DbmsKind::MySql` or `DbmsKind::MariaDb` (one pack; flavor gates inside, F9). Only a
    /// hint: every pass takes the flavor from the server's `VERSION()`.
    pub flavor: DbmsKind,
}

/// The production [`SectionReader`]: the built-in MySQL pack rendered for each call and run
/// on one snapshot. Checks `cancel` before every statement; logs `(section, statements)`.
pub struct TxnReader<'t, 'c> {
    txn: &'t mut SnapshotTxn<'c>,
    pack: Pack,
    cancel: Option<&'t CancellationToken>,
    /// Every section read, with its statement count (an empty list renders none).
    pub log: Vec<(&'static str, usize)>,
}

impl<'t, 'c> TxnReader<'t, 'c> {
    pub fn new(
        txn: &'t mut SnapshotTxn<'c>,
        cancel: Option<&'t CancellationToken>,
    ) -> Result<Self, IntroError> {
        // MySQL and MariaDB share one pack; the dialect only picks the parsed copy.
        let pack = Pack::builtin(DialectId::MySql)
            .map_err(|_| IntroError::Protocol(Name::from("pack")))?;
        Ok(TxnReader {
            txn,
            pack,
            cancel,
            log: Vec::new(),
        })
    }
}

impl SectionReader for TxnReader<'_, '_> {
    fn read<'a>(
        &'a mut self,
        section: &'static str,
        ctx: PackCtx,
    ) -> BoxFuture<'a, Result<Rows, IntroError>> {
        Box::pin(async move {
            let qs = self
                .pack
                .render(section, &ctx)
                .map_err(|_| IntroError::Protocol(Name::from(section)))?;
            let mut out = Rows::default();
            for q in &qs {
                if self.cancel.is_some_and(CancellationToken::is_cancelled) {
                    return Err(IntroError::Cancelled);
                }
                let rows = self.txn.rows(q.as_meta_query()).await?;
                if out.columns.is_empty() {
                    out.columns = rows.columns;
                }
                out.rows.extend(rows.rows);
            }
            self.log.push((section, qs.len()));
            Ok(out)
        })
    }
}

/// Runs `body` (with `r`, the snapshot's [`TxnReader`], and `srv`, its [`ServerCtx`]) on one
/// snapshot of the connection, then commits; evaluates to `Result<_, IntroError>`.
macro_rules! with_snapshot {
    ($conn:expr, $hint:expr, $cancel:expr, |$r:ident, $srv:ident| $body:expr) => {{
        let mut txn = $conn.begin_snapshot().await?;
        let out = {
            let mut reader = TxnReader::new(&mut txn, $cancel)?;
            let $r: &mut TxnReader<'_, '_> = &mut reader;
            let $srv: ServerCtx = pass::read_server($r, $hint).await?;
            $body
        };
        txn.commit().await?;
        Ok::<_, IntroError>(out)
    }};
}

impl MySqlIntrospector {
    /// [`Introspector::introspect`] with what the pass did (the Docker tests' query log).
    #[allow(clippy::too_many_arguments, reason = "the seam's arguments")]
    pub async fn introspect_with_stats(
        &self,
        c: &mut MetaConn,
        ns: &NsKey,
        mode: Mode,
        level: Level,
        filter: &CompiledFilter,
        prev: Option<&NamespaceSnapshot>,
        cancel: &CancellationToken,
    ) -> Result<(NamespaceResult, PassStats), IntroError> {
        if ns.kind == NsKind::Server {
            return facets::server_namespace(c)
                .await
                .map(|r| (r, PassStats::default()));
        }
        with_snapshot!(c, self.flavor, Some(cancel), |r, srv| {
            pass::run_pass(r, &srv, ns, mode, level, filter, prev).await?
        })
    }

    /// [`Introspector::fragment`] with what the read did. The keys name the objects, so no
    /// filter is pushed (the engine filters the result on the client, R-W3-2).
    pub async fn fragment_with_stats(
        &self,
        c: &mut MetaConn,
        ns: &NsKey,
        keys: &[ObjectKey],
        level: Level,
        cancel: &CancellationToken,
    ) -> Result<(NamespaceResult, PassStats), IntroError> {
        with_snapshot!(c, self.flavor, Some(cancel), |r, srv| {
            pass::run_fragment(r, &srv, ns, keys, level).await?
        })
    }

    /// The server context and the model's facts (`Identity`, and the default collation of
    /// every database from `ListNamespaces`).
    pub async fn server_facts(
        &self,
        c: &mut MetaConn,
    ) -> Result<(ServerCtx, ServerFacts), IntroError> {
        with_snapshot!(c, self.flavor, None, |r, srv| {
            let collations: BTreeMap<Name, Name> = pass::list_namespaces(r, &srv)
                .await?
                .into_iter()
                .filter_map(|(n, c)| c.map(|c| (n, c)))
                .collect();
            let facts = srv.facts(collations);
            (srv, facts)
        })
    }

    /// The `SHOW CREATE` statement of one object (R-M4-24, R-M4-37): the source of the
    /// `Sources` facet (Task 17b) and the "Definitions provided by server" toggle.
    pub async fn show_create(
        &self,
        c: &mut MetaConn,
        kind: ObjectKind,
        schema: &str,
        name: &str,
    ) -> Result<Option<Name>, IntroError> {
        with_snapshot!(c, self.flavor, None, |r, srv| {
            show_create::show_create(r, &srv, kind, schema, name).await?
        })
    }

    /// The server's identity for the cache (ruling R-W3-3; [`server::fingerprint`]): `None`
    /// when a fact it needs could not be read (the seam's `Unknown`); `Introspector::identity`
    /// wraps it.
    pub async fn server_fingerprint(
        &self,
        c: &mut MetaConn,
    ) -> Result<Option<hacivat_meta::persist::ServerFingerprint>, IntroError> {
        with_snapshot!(c, self.flavor, None, |r, srv| {
            let host = if srv.is_mariadb() {
                pass::read_host_facts(r, &srv).await?
            } else {
                server::HostFacts::default()
            };
            server::fingerprint(&srv, &host).map(hacivat_meta::persist::ServerFingerprint)
        })
    }

    /// The server's variable names (R-M4-44; `ListSettings`).
    pub async fn list_settings(&self, c: &mut MetaConn) -> Result<Vec<Setting>, IntroError> {
        with_snapshot!(c, self.flavor, None, |r, srv| {
            pass::list_settings(r, &srv).await?
        })
    }

    /// The relation names of `schema` the filter's patterns select on the server (R-W3-2;
    /// see [`pass::pushed_relations`]).
    pub async fn pushed_relations(
        &self,
        c: &mut MetaConn,
        schema: &str,
        filter: &CompiledFilter,
    ) -> Result<Option<BTreeSet<Name>>, IntroError> {
        with_snapshot!(c, self.flavor, None, |r, srv| {
            pass::pushed_relations(r, &srv, schema, filter).await?
        })
    }

    async fn list(&self, c: &mut MetaConn) -> Result<Vec<NsInfo>, IntroError> {
        with_snapshot!(c, self.flavor, None, |r, srv| {
            pass::list_namespaces(r, &srv)
                .await?
                .into_iter()
                .map(|(n, _)| NsInfo {
                    // The four system schemas are always stored lower case.
                    system: SYSTEM_SCHEMAS.contains(&&*n),
                    key: NsKey::schema([n]),
                })
                .collect::<Vec<_>>()
        })
    }

    async fn stamps_of(
        &self,
        c: &mut MetaConn,
        keys: &[ObjectKey],
    ) -> Result<Vec<Option<StateMark>>, IntroError> {
        with_snapshot!(c, self.flavor, None, |r, srv| {
            state::stamps(r, &srv, keys).await?
        })
    }

    async fn ns_check(&self, c: &mut MetaConn, ns: &NsKey) -> Result<NsCheck, IntroError> {
        let schema = pass::schema_of(ns)?;
        with_snapshot!(c, self.flavor, None, |r, srv| {
            state::ns_check(&state::server_top_marks(r, &srv, &schema).await?)
        })
    }
}

impl Introspector for MySqlIntrospector {
    fn list_namespaces<'a>(
        &'a self,
        c: &'a mut MetaConn,
    ) -> BoxFuture<'a, Result<Vec<NsInfo>, IntroError>> {
        Box::pin(self.list(c))
    }

    #[allow(clippy::too_many_arguments, reason = "the seam's arguments")]
    fn introspect<'a>(
        &'a self,
        c: &'a mut MetaConn,
        ns: &'a NsKey,
        mode: Mode,
        level: Level,
        filter: &'a CompiledFilter,
        prev: Option<&'a NamespaceSnapshot>,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<NamespaceResult, IntroError>> {
        Box::pin(async move {
            self.introspect_with_stats(c, ns, mode, level, filter, prev, cancel)
                .await
                .map(|(r, _)| r)
        })
    }

    #[allow(clippy::too_many_arguments, reason = "the seam's arguments")]
    fn fragment<'a>(
        &'a self,
        c: &'a mut MetaConn,
        ns: &'a NsKey,
        keys: &'a [ObjectKey],
        level: Level,
        filter: &'a CompiledFilter,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<NamespaceResult, IntroError>> {
        let _ = filter;
        Box::pin(async move {
            self.fragment_with_stats(c, ns, keys, level, cancel)
                .await
                .map(|(r, _)| r)
        })
    }

    fn stamps<'a>(
        &'a self,
        c: &'a mut MetaConn,
        keys: &'a [ObjectKey],
    ) -> BoxFuture<'a, Result<Vec<Option<StateMark>>, IntroError>> {
        Box::pin(self.stamps_of(c, keys))
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

    /// The introspector holds no stored marks, so every key is `Unknown` (Task 22's
    /// `freshness::check_state` compares [`Introspector::stamps`] with the snapshot's marks).
    /// A key `INFORMATION_SCHEMA` no longer lists is `Unknown` too, never `Dropped`: the login
    /// may merely have lost the privilege that showed it (review m7). Drops are decided by a
    /// pass's existence sets, behind the privilege hash and the mass-deletion guard.
    fn check_state<'a>(
        &'a self,
        c: &'a mut MetaConn,
        keys: &'a [ObjectKey],
    ) -> BoxFuture<'a, Result<Vec<Staleness>, IntroError>> {
        Box::pin(async move {
            Ok(self
                .stamps_of(c, keys)
                .await?
                .into_iter()
                .map(|_| Staleness::Unknown)
                .collect())
        })
    }

    /// The server side of the cheap check; [`state::top_marks_of`] gives the cached side.
    fn check_namespace<'a>(
        &'a self,
        c: &'a mut MetaConn,
        ns: &'a NsKey,
        _snap: &'a NamespaceSnapshot,
    ) -> BoxFuture<'a, Result<NsCheck, IntroError>> {
        Box::pin(self.ns_check(c, ns))
    }

    /// R-W3-3: [`MySqlIntrospector::server_fingerprint`]; a fact that cannot be read (or a
    /// failed read) is `Unknown`, which the engine treats fail-closed.
    fn identity<'a>(
        &'a self,
        c: &'a mut MetaConn,
    ) -> BoxFuture<'a, Result<ServerIdentity, IntroError>> {
        Box::pin(async move {
            Ok(match self.server_fingerprint(c).await {
                Ok(Some(fp)) => ServerIdentity::Known(fp),
                Ok(None) | Err(_) => ServerIdentity::Unknown,
            })
        })
    }

    /// T15 carry m4: the facts the catalog was made with are a placeholder (lctn 0, no
    /// version); these replace them on connect.
    fn server_facts<'a>(
        &'a self,
        c: &'a mut MetaConn,
    ) -> BoxFuture<'a, Result<Option<ServerFacts>, IntroError>> {
        Box::pin(async move { Ok(Some(MySqlIntrospector::server_facts(self, c).await?.1)) })
    }

    fn capabilities(&self) -> IntroCaps {
        IntroCaps {
            dbms: self.flavor,
            transactional_ddl: false,
            // R-M4-10: MariaDB refreshes `Full` per namespace in v1.
            incremental: self.flavor == DbmsKind::MySql,
            // ## m4-t17b-mysql-facets (named exception): server text, grants and (MySQL)
            // view dependencies on demand (`facets::read`).
            facets: facets::facets_for(self.flavor),
        }
    }
}
