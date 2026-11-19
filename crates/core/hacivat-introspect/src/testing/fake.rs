// ## m4-t01c-core
//! `FakeIntrospector` (body Task 15): an in-memory introspector the engine's tests drive
//! (scripted namespaces, query counting, gated passes).
//!
//! ## m4-t15-engine (M4 Task 15)
//!
//! - **The server** is a map of namespaces, each a list of [`FakeObject`]s (tables or views,
//!   with column names, an optional OID and a version). A pass answers like a real
//!   introspector: complete existence sets per kind (scoped ones for fragments, resolved by
//!   server id first, as Task 10's RR-1 contract asks), the changed objects with their columns
//!   at `Structure` and above, a `PgXid` watermark when incremental passes are on.
//! - **Queries** are counted the way a real pass issues them: one per list, brief, fragment or
//!   state check; a full or incremental pass at `Structure` or above is two batches (relations,
//!   then columns), and the cancel token is checked between them.
//! - **Calls** are recorded ([`FakeCall`]); [`FakeIntrospector::gate`] holds the next matching
//!   call after its first batch until released; [`FakeIntrospector::fail`] scripts errors.
//!
//! [`FakeMetaSource`] hands out inert `MetaConn::offline` connections (the fake never reads
//! them) and can play a busy single-connection profile, a user transaction or a disconnect.
//! Product code never uses either.

#![allow(
    clippy::result_large_err,
    reason = "IntroError carries the server's message by value (MetaError::Server), as the seam does"
)]

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use futures::future::BoxFuture;
use hacivat_meta::merge::{self, ChangedObject, ExistSet, Identity, NamespaceResult};
use hacivat_meta::name::FoldRule;
use hacivat_meta::persist::ServerFingerprint;
use hacivat_meta::props::{ColumnProps, FacetKind, Props, TableProps, ViewProps};
use hacivat_meta::snapshot::IntroErrorCode;
use hacivat_meta::{
    Level, Name, NamespaceSnapshot, NsKey, Obj, ObjectKey, ObjectKind, ServerId, StateMark,
    Subtree, Watermark,
};
use hacivat_proto::DbmsKind;
use hacivat_session::meta::{MetaConn, MetaError};
use tokio::sync::{Semaphore, watch};
use tokio_util::sync::CancellationToken;

use crate::engine::MetaSource;
use crate::filter::CompiledFilter;
use crate::freshness::NsCheck;
use crate::introspector::{
    FacetRows, IntroCaps, IntroError, Introspector, ServerIdentity, Staleness,
};
use crate::pack::Mode;
use crate::scope::NsInfo;

/// One object of the fake server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeObject {
    /// `Table` or `View`.
    pub kind: ObjectKind,
    pub name: Name,
    pub oid: Option<u32>,
    pub columns: Vec<Name>,
    /// Bumped by [`FakeIntrospector::touch`]; an incremental pass re-reads objects whose
    /// version is above the namespace's watermark.
    pub version: u64,
}

impl FakeObject {
    /// A table `name` with `columns`.
    pub fn table(name: &str, columns: &[&str]) -> Self {
        FakeObject {
            kind: ObjectKind::Table,
            name: Name::from(name),
            oid: None,
            columns: columns.iter().map(|c| Name::from(*c)).collect(),
            version: 1,
        }
    }

    /// A view `name` with `columns`.
    pub fn view(name: &str, columns: &[&str]) -> Self {
        FakeObject {
            kind: ObjectKind::View,
            ..FakeObject::table(name, columns)
        }
    }

    /// The same object with OID `oid`.
    pub fn with_oid(mut self, oid: u32) -> Self {
        self.oid = Some(oid);
        self
    }
}

/// One namespace of the fake server.
#[derive(Debug, Clone, Default)]
pub struct FakeNs {
    pub objects: Vec<FakeObject>,
    /// Kinds whose existence query fails (permission loss).
    pub failed: BTreeMap<ObjectKind, IntroErrorCode>,
    /// A column whose table the namespace does not hold: `(table, column)` (HV-INTRO-01 edge
    /// 4, a child row naming a parent the pass did not return).
    pub orphan: Option<(Name, Name)>,
    pub system: bool,
}

/// A call the engine made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeCall {
    ListNamespaces,
    Introspect {
        ns: NsKey,
        mode: Mode,
        level: Level,
    },
    Fragment {
        ns: NsKey,
        keys: Vec<ObjectKey>,
        level: Level,
    },
    CheckState {
        keys: Vec<ObjectKey>,
    },
}

type Matcher = Box<dyn Fn(&FakeCall) -> bool + Send + Sync>;

struct Gate {
    matches: Matcher,
    entered: watch::Sender<bool>,
    open: Arc<Semaphore>,
    used: bool,
}

struct Failure {
    matches: Matcher,
    error: IntroError,
    times: u32,
}

/// A held call ([`FakeIntrospector::gate`]).
#[derive(Debug, Clone)]
pub struct FakeGate {
    entered: watch::Receiver<bool>,
    open: Arc<Semaphore>,
}

impl FakeGate {
    /// Resolves once a matching call reached the gate.
    pub async fn entered(&self) {
        let mut rx = self.entered.clone();
        let _ = rx.wait_for(|e| *e).await;
    }

    /// Lets the held call go on.
    pub fn release(&self) {
        self.open.add_permits(1);
    }
}

#[derive(Default)]
struct State {
    namespaces: BTreeMap<NsKey, FakeNs>,
    listing: Option<Vec<NsInfo>>,
    /// Listings per database the connection is to (`set_listing_for`).
    listing_for: BTreeMap<Option<Name>, Vec<NsInfo>>,
    /// The filter text of every introspect and fragment call, in order.
    filters: Vec<String>,
    identity: Option<u128>,
    /// The metadata source whose last connection names the database a listing is for.
    meta: Option<Arc<FakeMetaSource>>,
    calls: Vec<FakeCall>,
    gates: Vec<Gate>,
    failures: Vec<Failure>,
    /// The highest version handed out.
    clock: u64,
    /// The version each object had when last read, for `check_state`.
    read: HashMap<(NsKey, Name), u64>,
}

/// A scripted introspector for tests.
#[derive(Clone)]
pub struct FakeIntrospector {
    pub dbms: DbmsKind,
    incremental: Arc<AtomicBool>,
    queries: Arc<AtomicU64>,
    state: Arc<Mutex<State>>,
}

impl std::fmt::Debug for FakeIntrospector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeIntrospector")
            .field("dbms", &self.dbms)
            .field("queries", &self.queries())
            .finish_non_exhaustive()
    }
}

impl Default for FakeIntrospector {
    fn default() -> Self {
        FakeIntrospector::new(DbmsKind::Postgres)
    }
}

impl FakeIntrospector {
    pub fn new(dbms: DbmsKind) -> Self {
        FakeIntrospector {
            dbms,
            incremental: Arc::new(AtomicBool::new(false)),
            queries: Arc::new(AtomicU64::new(0)),
            state: Arc::new(Mutex::new(State {
                identity: Some(1),
                ..State::default()
            })),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Turns incremental passes on (`capabilities().incremental`, a `PgXid` watermark).
    pub fn with_incremental(self, on: bool) -> Self {
        self.incremental.store(on, Ordering::SeqCst);
        self
    }

    /// Sets namespace `ns` of the server.
    pub fn put_ns(&self, ns: NsKey, objects: Vec<FakeObject>) {
        let mut st = self.lock();
        st.clock = st
            .clock
            .max(objects.iter().map(|o| o.version).max().unwrap_or(0));
        st.namespaces.insert(
            ns,
            FakeNs {
                objects,
                ..FakeNs::default()
            },
        );
    }

    /// Changes namespace `ns` of the server in place (created empty when absent).
    pub fn edit_ns(&self, ns: &NsKey, f: impl FnOnce(&mut FakeNs)) {
        let mut st = self.lock();
        f(st.namespaces.entry(ns.clone()).or_default());
    }

    /// `n` tables `t0`… with one column each and OIDs from 20,000.
    pub fn tables(n: usize) -> Vec<FakeObject> {
        (0..n)
            .map(|i| FakeObject::table(&format!("t{i}"), &["id"]).with_oid(20_000 + i as u32))
            .collect()
    }

    /// Bumps the version of object `name` of `ns` (a DDL on the server).
    pub fn touch(&self, ns: &NsKey, name: &str) {
        let mut st = self.lock();
        st.clock += 1;
        let v = st.clock;
        if let Some(o) = st
            .namespaces
            .get_mut(ns)
            .and_then(|n| n.objects.iter_mut().find(|o| &*o.name == name))
        {
            o.version = v;
        }
    }

    /// The namespaces `list_namespaces` answers (default: every namespace set, as user ones).
    pub fn set_listing(&self, infos: Vec<NsInfo>) {
        self.lock().listing = Some(infos);
    }

    /// `list_namespaces` on a connection to `database` answers `infos` (PostgreSQL lists one
    /// database per connection). Needs [`FakeIntrospector::link_meta`].
    pub fn set_listing_for(&self, database: Option<&str>, infos: Vec<NsInfo>) {
        self.lock()
            .listing_for
            .insert(database.map(Name::from), infos);
    }

    /// Tells the fake which metadata source hands out its connections (for
    /// `set_listing_for`).
    pub fn link_meta(&self, meta: Arc<FakeMetaSource>) {
        self.lock().meta = Some(meta);
    }

    /// The server identity `identity` answers (`None`: `Unknown`). Default: a fixed one.
    pub fn set_identity(&self, fp: Option<u128>) {
        self.lock().identity = fp;
    }

    /// The filter text every introspect and fragment call got, in order.
    pub fn filters(&self) -> Vec<String> {
        self.lock().filters.clone()
    }

    /// The calls made so far, in order.
    pub fn calls(&self) -> Vec<FakeCall> {
        self.lock().calls.clone()
    }

    /// The catalog queries issued so far.
    pub fn queries(&self) -> u64 {
        self.queries.load(Ordering::SeqCst)
    }

    /// Holds the next call that `matches`, after its first batch, until released.
    pub fn gate(&self, matches: impl Fn(&FakeCall) -> bool + Send + Sync + 'static) -> FakeGate {
        let (tx, rx) = watch::channel(false);
        let open = Arc::new(Semaphore::new(0));
        self.lock().gates.push(Gate {
            matches: Box::new(matches),
            entered: tx,
            open: open.clone(),
            used: false,
        });
        FakeGate { entered: rx, open }
    }

    /// Fails the next `times` calls that `matches` with `error` (after their first query).
    pub fn fail(
        &self,
        matches: impl Fn(&FakeCall) -> bool + Send + Sync + 'static,
        error: IntroError,
        times: u32,
    ) {
        self.lock().failures.push(Failure {
            matches: Box::new(matches),
            error,
            times,
        });
    }

    /// Records `call`, counts its first query, applies a scripted failure, waits at a gate.
    async fn enter(
        &self,
        call: FakeCall,
        cancel: Option<&CancellationToken>,
    ) -> Result<(), IntroError> {
        self.queries.fetch_add(1, Ordering::SeqCst);
        let (fail, gate) = {
            let mut st = self.lock();
            st.calls.push(call.clone());
            let fail = st
                .failures
                .iter_mut()
                .find(|f| f.times > 0 && (f.matches)(&call))
                .map(|f| {
                    f.times -= 1;
                    f.error.clone()
                });
            let gate = st
                .gates
                .iter_mut()
                .find(|g| !g.used && (g.matches)(&call))
                .map(|g| {
                    g.used = true;
                    g.entered.send_replace(true);
                    g.open.clone()
                });
            (fail, gate)
        };
        if let Some(open) = gate {
            match cancel {
                Some(c) => tokio::select! {
                    () = c.cancelled() => return Err(IntroError::Cancelled),
                    p = open.acquire() => drop(p),
                },
                None => drop(open.acquire().await),
            }
        }
        if let Some(e) = fail {
            return Err(e);
        }
        Ok(())
    }

    fn root_key(&self, ns: &NsKey) -> Result<ObjectKey, IntroError> {
        merge::base(ns.clone(), self.dbms, FoldRule::Exact)
            .ok()
            .and_then(|b| b.object_key(0))
            .ok_or(IntroError::Protocol(Name::from("fake-root")))
    }

    fn subtree(o: &FakeObject, level: Level) -> Subtree {
        let props = match o.kind {
            ObjectKind::View => Props::View(Arc::new(ViewProps::default())),
            _ => Props::Table(Arc::new(TableProps::default())),
        };
        let mut root = Obj::new(o.kind, o.name.clone())
            .with_props(props)
            .with_state(StateMark::Hash(o.version));
        if let Some(oid) = o.oid {
            root = root.with_server_id(ServerId::Oid(oid));
        }
        let mut t = Subtree::new(root);
        if level >= Level::Structure {
            for (i, c) in o.columns.iter().enumerate() {
                let col = Obj::new(ObjectKind::Column, c.clone()).with_props(Props::Column(
                    Arc::new(ColumnProps {
                        ordinal: i as u32 + 1,
                        raw_type: Name::from("integer"),
                        nullable: true,
                        ..ColumnProps::default()
                    }),
                ));
                t.add(t.root(), col);
            }
        }
        t
    }

    fn ident(o: &FakeObject) -> Identity {
        Identity::of(vec![o.name.clone()], o.oid.map(ServerId::Oid))
    }

    fn column_ids(o: &FakeObject) -> impl Iterator<Item = Identity> + '_ {
        o.columns
            .iter()
            .map(|c| Identity::of(vec![o.name.clone(), c.clone()], None))
    }

    /// A pass over `ns`, as a real introspector answers it.
    fn pass(
        &self,
        ns: &NsKey,
        mode: Mode,
        level: Level,
        prev: Option<&NamespaceSnapshot>,
    ) -> Result<NamespaceResult, IntroError> {
        let root = self.root_key(ns)?;
        let mut st = self.lock();
        let clock = st.clock;
        let space = st.namespaces.get(ns).cloned().unwrap_or_default();
        let since = match (mode, prev.map(|p| &p.meta().watermark)) {
            (Mode::Incremental, Some(Watermark::PgXid { w, .. })) => Some(*w),
            _ => None,
        };
        let level = if mode == Mode::Brief {
            Level::Names
        } else {
            level
        };
        let mut existent = BTreeMap::new();
        let mut changed = Vec::new();
        let mut reread = Vec::new();
        for kind in [ObjectKind::Table, ObjectKind::View] {
            if let Some(code) = space.failed.get(&kind) {
                existent.insert(kind, ExistSet::Failed(*code));
                continue;
            }
            let of_kind: Vec<&FakeObject> =
                space.objects.iter().filter(|o| o.kind == kind).collect();
            existent.insert(
                kind,
                ExistSet::Ok(of_kind.iter().map(|o| Self::ident(o)).collect()),
            );
            for o in of_kind {
                if since.is_some_and(|w| o.version <= w) {
                    continue;
                }
                changed.push(ChangedObject {
                    parent: root.clone(),
                    subtree: Self::subtree(o, level),
                });
                reread.push(o.clone());
            }
        }
        if level >= Level::Structure {
            let cols: Vec<Identity> = reread.iter().flat_map(Self::column_ids).collect();
            let set = if since.is_some() {
                ExistSet::Scoped {
                    scope: reread.iter().map(|o| vec![o.name.clone()]).collect(),
                    ids: cols,
                }
            } else {
                ExistSet::Ok(cols)
            };
            existent.insert(ObjectKind::Column, set);
        }
        if let Some((table, column)) = &space.orphan {
            let mut parent = root.clone();
            parent.kind = ObjectKind::Table;
            parent.path.push(table.clone());
            changed.push(ChangedObject {
                parent,
                subtree: Subtree::new(Obj::new(ObjectKind::Column, column.clone())),
            });
        }
        for o in &reread {
            st.read.insert((ns.clone(), o.name.clone()), o.version);
        }
        Ok(NamespaceResult {
            existent,
            changed,
            categories_to_reload: Default::default(),
            new_watermark: if self.incremental.load(Ordering::SeqCst) {
                Watermark::PgXid {
                    w: clock,
                    identity: Default::default(),
                }
            } else {
                Watermark::None
            },
            full: mode != Mode::Incremental,
            level,
            read_at_ms: 0,
        })
    }

    /// The objects `keys` of `ns`: by server id first, else by name (RR-1).
    fn find<'a>(space: &'a FakeNs, key: &ObjectKey) -> Option<&'a FakeObject> {
        let by_id = key.server_id.and_then(|sid| {
            space
                .objects
                .iter()
                .find(|o| o.kind == key.kind && o.oid.map(ServerId::Oid) == Some(sid))
        });
        by_id.or_else(|| {
            space
                .objects
                .iter()
                .find(|o| o.kind == key.kind && &*o.name == key.name())
        })
    }

    fn fragment_result(
        &self,
        ns: &NsKey,
        keys: &[ObjectKey],
        level: Level,
    ) -> Result<NamespaceResult, IntroError> {
        let root = self.root_key(ns)?;
        let mut st = self.lock();
        let space = st.namespaces.get(ns).cloned().unwrap_or_default();
        let mut existent: BTreeMap<ObjectKind, ExistSet> = BTreeMap::new();
        let mut changed = Vec::new();
        let mut found = Vec::new();
        for key in keys {
            let rel = key.path[ns.path.len()..].to_vec();
            if rel.len() != 1 {
                continue;
            }
            let entry = existent.entry(key.kind).or_insert(ExistSet::Scoped {
                scope: Vec::new(),
                ids: Vec::new(),
            });
            let ExistSet::Scoped { scope, ids } = entry else {
                continue;
            };
            scope.push(rel);
            if let Some(o) = Self::find(&space, key) {
                ids.push(Self::ident(o));
                changed.push(ChangedObject {
                    parent: root.clone(),
                    subtree: Self::subtree(o, level),
                });
                found.push(o.clone());
            }
        }
        if level >= Level::Structure && !found.is_empty() {
            existent.insert(
                ObjectKind::Column,
                ExistSet::Scoped {
                    scope: found.iter().map(|o| vec![o.name.clone()]).collect(),
                    ids: found.iter().flat_map(Self::column_ids).collect(),
                },
            );
        }
        for o in &found {
            st.read.insert((ns.clone(), o.name.clone()), o.version);
        }
        Ok(NamespaceResult {
            existent,
            changed,
            categories_to_reload: Default::default(),
            new_watermark: Watermark::None,
            full: false,
            level,
            read_at_ms: 0,
        })
    }
}

impl Introspector for FakeIntrospector {
    fn list_namespaces<'a>(
        &'a self,
        _c: &'a mut MetaConn,
    ) -> BoxFuture<'a, Result<Vec<NsInfo>, IntroError>> {
        Box::pin(async move {
            self.enter(FakeCall::ListNamespaces, None).await?;
            let st = self.lock();
            if !st.listing_for.is_empty() {
                let db = st
                    .meta
                    .as_ref()
                    .and_then(|m| {
                        m.databases
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .last()
                            .cloned()
                    })
                    .flatten();
                return Ok(st.listing_for.get(&db).cloned().unwrap_or_default());
            }
            Ok(st.listing.clone().unwrap_or_else(|| {
                st.namespaces
                    .iter()
                    .map(|(k, n)| NsInfo {
                        key: k.clone(),
                        system: n.system,
                    })
                    .collect()
            }))
        })
    }

    fn introspect<'a>(
        &'a self,
        _c: &'a mut MetaConn,
        ns: &'a NsKey,
        mode: Mode,
        level: Level,
        filter: &'a CompiledFilter,
        prev: Option<&'a NamespaceSnapshot>,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<NamespaceResult, IntroError>> {
        Box::pin(async move {
            self.lock().filters.push(filter.text().to_owned());
            let call = FakeCall::Introspect {
                ns: ns.clone(),
                mode,
                level,
            };
            self.enter(call, Some(cancel)).await?;
            if mode != Mode::Brief && level >= Level::Structure {
                // The second batch (columns): a cancel between batches stops the pass.
                if cancel.is_cancelled() {
                    return Err(IntroError::Cancelled);
                }
                self.queries.fetch_add(1, Ordering::SeqCst);
            }
            self.pass(ns, mode, level, prev)
        })
    }

    fn fragment<'a>(
        &'a self,
        _c: &'a mut MetaConn,
        ns: &'a NsKey,
        keys: &'a [ObjectKey],
        level: Level,
        filter: &'a CompiledFilter,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<NamespaceResult, IntroError>> {
        Box::pin(async move {
            self.lock().filters.push(filter.text().to_owned());
            let call = FakeCall::Fragment {
                ns: ns.clone(),
                keys: keys.to_vec(),
                level,
            };
            self.enter(call, Some(cancel)).await?;
            self.fragment_result(ns, keys, level)
        })
    }

    fn stamps<'a>(
        &'a self,
        _c: &'a mut MetaConn,
        _keys: &'a [ObjectKey],
    ) -> BoxFuture<'a, Result<Vec<Option<StateMark>>, IntroError>> {
        Box::pin(async { Err(IntroError::NotImplemented) })
    }

    fn facet<'a>(
        &'a self,
        _c: &'a mut MetaConn,
        _ns: &'a NsKey,
        _keys: &'a [ObjectKey],
        _kind: FacetKind,
    ) -> BoxFuture<'a, Result<FacetRows, IntroError>> {
        Box::pin(async { Err(IntroError::NotImplemented) })
    }

    fn check_state<'a>(
        &'a self,
        _c: &'a mut MetaConn,
        keys: &'a [ObjectKey],
    ) -> BoxFuture<'a, Result<Vec<Staleness>, IntroError>> {
        Box::pin(async move {
            self.enter(
                FakeCall::CheckState {
                    keys: keys.to_vec(),
                },
                None,
            )
            .await?;
            let st = self.lock();
            Ok(keys
                .iter()
                .map(|k| {
                    let Some((ns, space)) = st.namespaces.iter().find(|(ns, _)| {
                        k.path.starts_with(&ns.path) && k.path.len() == ns.path.len() + 1
                    }) else {
                        return Staleness::Dropped;
                    };
                    match Self::find(space, k) {
                        None => Staleness::Dropped,
                        Some(o) => match st.read.get(&(ns.clone(), o.name.clone())) {
                            Some(v) if *v == o.version => Staleness::Current,
                            Some(_) => Staleness::Changed,
                            None => Staleness::Unknown,
                        },
                    }
                })
                .collect())
        })
    }

    fn check_namespace<'a>(
        &'a self,
        _c: &'a mut MetaConn,
        _ns: &'a NsKey,
        _snap: &'a NamespaceSnapshot,
    ) -> BoxFuture<'a, Result<NsCheck, IntroError>> {
        Box::pin(async { Err(IntroError::NotImplemented) })
    }

    fn identity<'a>(
        &'a self,
        _c: &'a mut MetaConn,
    ) -> BoxFuture<'a, Result<ServerIdentity, IntroError>> {
        Box::pin(async move {
            Ok(match self.lock().identity {
                Some(fp) => ServerIdentity::Known(ServerFingerprint(fp)),
                None => ServerIdentity::Unknown,
            })
        })
    }

    fn capabilities(&self) -> IntroCaps {
        IntroCaps {
            incremental: self.incremental.load(Ordering::SeqCst),
            ..IntroCaps::planned(self.dbms)
        }
    }
}

/// A metadata source for the engine's tests: inert offline connections, and switches for a
/// busy single-connection profile, a user transaction and a disconnected profile.
#[derive(Debug)]
pub struct FakeMetaSource {
    pub dbms: DbmsKind,
    /// `acquire` answers `Busy` (the user's session holds a transaction).
    pub busy: AtomicBool,
    /// `user_in_transaction` (periodic validation waits).
    pub in_tx: AtomicBool,
    /// `acquire` answers `Unavailable` (not connected).
    pub unavailable: AtomicBool,
    /// Connections handed out.
    pub acquired: AtomicU64,
    /// `acquire` calls, refused ones included.
    pub attempts: AtomicU64,
    /// `user_in_transaction` calls (each periodic validation asks once).
    pub tx_checks: AtomicU64,
    /// The profile's budget of metadata connections (0: unlimited): over it `acquire`
    /// answers `Busy` until one is `released`.
    pub budget: AtomicU64,
    /// Connections handed out and not yet released, and the most open at once.
    pub open: AtomicU64,
    pub peak: AtomicU64,
    /// The databases asked for, in order.
    pub databases: Mutex<Vec<Option<Name>>>,
}

impl FakeMetaSource {
    pub fn new(dbms: DbmsKind) -> Arc<Self> {
        Arc::new(FakeMetaSource {
            dbms,
            busy: AtomicBool::new(false),
            in_tx: AtomicBool::new(false),
            unavailable: AtomicBool::new(false),
            acquired: AtomicU64::new(0),
            attempts: AtomicU64::new(0),
            tx_checks: AtomicU64::new(0),
            budget: AtomicU64::new(0),
            open: AtomicU64::new(0),
            peak: AtomicU64::new(0),
            databases: Mutex::new(Vec::new()),
        })
    }
}

impl MetaSource for FakeMetaSource {
    fn acquire<'a>(
        &'a self,
        database: Option<Name>,
        _cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<MetaConn, MetaError>> {
        Box::pin(async move {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            if self.unavailable.load(Ordering::SeqCst) {
                return Err(MetaError::Unavailable);
            }
            if self.busy.load(Ordering::SeqCst) {
                return Err(MetaError::Busy);
            }
            let budget = self.budget.load(Ordering::SeqCst);
            if budget > 0 && self.open.load(Ordering::SeqCst) >= budget {
                return Err(MetaError::Busy);
            }
            let open = self.open.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(open, Ordering::SeqCst);
            self.acquired.fetch_add(1, Ordering::SeqCst);
            self.databases
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(database);
            Ok(MetaConn::offline(self.dbms))
        })
    }

    fn user_in_transaction(&self) -> bool {
        self.tx_checks.fetch_add(1, Ordering::SeqCst);
        self.in_tx.load(Ordering::SeqCst)
    }

    fn released(&self) {
        let _ = self
            .open
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1));
    }
}
