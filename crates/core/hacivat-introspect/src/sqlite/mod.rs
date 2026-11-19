// ## m4-t01c-core (seam), filled by m4-t18-sqlite
//! The SQLite introspector (HV-INTRO-08; Task 18): `sqlite_schema` and the pragma passes on a
//! separate `mode=ro` connection (R-M4-38), `schema_version` plus a schema digest as the change
//! marker, the `temp` schema read on the user's idle connection with the authorizer's pragmas.
//!
//! ## m4-t18-sqlite (spike origin `f690f45`, `sqlite.rs`; ADR 0008 `## m4-t18 — SQLite
//! introspection`)
//!
//! **Where it reads.** `main` and attached schemas on the session manager's read-only reader
//! (a deferred `BEGIN` makes a pass one consistent snapshot), `temp` on the user's own idle
//! connection (the engine asks `MetaConnSpec { database: Some("temp") }`), a joined read on
//! the writer that holds a Hacivat-owned transaction (R-M4-29). Catalog text comes only from
//! `packs/sqlite/*.hvq.sql`, names and schemas only as binds. An attached file is read under
//! the fixed alias [`ATTACH_ALIAS`]: `sqlite_schema` needs its schema written as a qualifier,
//! which a bind cannot be, so the introspector attaches the file the connection already lists
//! (read-only, through `MetaConn::attach_read_only`) under that alias. A schema the connection
//! does not have answers `Meta(Unavailable)`, never an empty namespace.
//!
//! **Change detection** (alg/02 HV-INTRO-08 steps 1–3). A pass reads `pragma_database_list`
//! (the file of each schema), stats the file for `(dev, inode)` (Unix; `None` elsewhere), and
//! `main`'s `schema_version` (the table-valued `pragma_schema_version` reads `main` only;
//! `temp` and attached schemas have none, version 0). A different version re-reads. Otherwise
//! the XXH3-128 digest of the **sorted** `(type, name, tbl_name, sql)` rows of the schema's
//! `sqlite_schema` decides, computed when the connection is new to the namespace
//! ([`PassConn::Fresh`] — the trait's passes, because the engine opens one reader per task),
//! when the identity changed or is unknown, when there is no version, or on every pass with
//! [`SqliteIntrospector::digest_every_pass`] (network shares, edge 5). A changed schema is read
//! whole (SQLite has no per-object marker; a full read is local). `PRAGMA data_version` is not
//! used (F8). An unchanged pass returns no change and the fresh watermark.
//!
//! **Levels.** `Names`: tables, views and virtual tables. `Structure`: with columns, keys,
//! indexes, foreign keys and triggers (`build`). `Sources`: with the view and trigger texts'
//! `SourceRef`s; the texts themselves, and table and index texts, are the `Sources` facet
//! (`sqlite_schema.sql`, which T19 shows as the server's text, R-M4-24).
//!
//! **State** (HV-INTRO-15) is schema-level: every object carries [`schema_mark`] of its
//! schema; `stamps` answers that mark for each key that exists (`None` when gone);
//! `check_state` answers `Dropped` for a key that is gone and `Unknown` otherwise (it has no
//! stored mark to compare; Task 22 compares `stamps`); `check_namespace` the relation count,
//! the mark and the existence digest, which [`ns_check_of`] computes the same way from a
//! cached namespace.
//!
//! **Locks.** A file another connection holds locked answers `SQLITE_BUSY` after the driver's
//! busy timeout; the pass is tried [`BUSY_ATTEMPTS`] times, then fails softly (the namespace
//! is `Failed`, nothing is decided). **Built-ins** (`function_list`, `module_list`,
//! `collation_list`) are read live per connection ([`SqliteIntrospector::builtins`]; no
//! snapshot, R-M4-6); none enters a namespace. **Facets:** `Sources`; `Grants` is not
//! applicable (SQLite has no privileges: an empty list); `Statistics` has no row estimate;
//! `Dependencies` is unsupported (`NotImplemented`).

#![allow(
    clippy::result_large_err,
    reason = "the seam's `IntroError` carries the server's message (T1c's type)"
)]

mod build;
mod read;
mod text;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use futures::future::BoxFuture;
use hacivat_meta::merge::{ChangedObject, NamespaceResult};
use hacivat_meta::props::{FacetKind, GrantsFacet, StatsFacet};
use hacivat_meta::{
    Level, Name, NamespaceSnapshot, NsKey, NsKind, Obj, ObjFlags, ObjectKey, ObjectKind, StateMark,
    Subtree, Watermark,
};
use hacivat_proto::DbmsKind;
use hacivat_session::meta::{MetaConn, MetaError, SnapshotTxn};
use tokio_util::sync::CancellationToken;

use hacivat_meta::persist::ServerFingerprint;

use crate::engine::Phase;
use crate::filter::CompiledFilter;
use crate::freshness::NsCheck;
use crate::introspector::{
    FacetRows, IntroCaps, IntroError, Introspector, ServerIdentity, Staleness,
};
use crate::pack::{DialectId, Mode, Pack};
use crate::scope::NsInfo;

use build::{RELATION_KINDS, relation_kind};
use read::{Cx, DbEntry, ObjRow, Variant};

/// The schema name the introspector attaches an attached file under on the read-only reader
/// (module docs). It is never listed as a namespace of its own.
pub const ATTACH_ALIAS: &str = "hacivat_attached";

/// How many times a pass is tried when the file is locked (alg/02 HV-INTRO-08 edge 11); each
/// try waits the driver's busy timeout first.
pub const BUSY_ATTEMPTS: u32 = 2;

/// The pack sections a pass ran, in order (a diagnostics hook; tests read it).
pub type SectionTrace = Arc<Mutex<Vec<Name>>>;

/// Where a pass reports its counts: the namespace, the phase, how many are done and of how
/// many (`None` until known). The trait has no progress parameter (T15's scheduler counts
/// tasks; a count inside one pass was carried to Tasks 16–18), so the engine hands the sink to
/// the introspector it builds ([`SqliteIntrospector::with_progress`]).
pub type ProgressSink = Arc<dyn Fn(&NsKey, Phase, u64, Option<u64>) + Send + Sync>;

/// The SQLite introspector.
#[derive(Clone, Default)]
pub struct SqliteIntrospector {
    /// Edge 5 (a file on a network share, whose inode is not stable): check the digest on
    /// every pass, also on a reused connection whose version and identity match.
    pub digest_every_pass: bool,
    trace: Option<SectionTrace>,
    progress: Option<ProgressSink>,
}

impl std::fmt::Debug for SqliteIntrospector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteIntrospector")
            .field("digest_every_pass", &self.digest_every_pass)
            .field("trace", &self.trace.is_some())
            .field("progress", &self.progress.is_some())
            .finish()
    }
}

/// Whether the connection is new to the namespace (module docs, "Change detection").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassConn {
    /// First pass on this connection: the digest is checked.
    Fresh,
    /// The connection read this namespace before: a matching version and identity skip the
    /// digest (unless `digest_every_pass`).
    Reused,
}

/// A function of `pragma_function_list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteFunction {
    pub name: Name,
    pub builtin: bool,
    /// `s` scalar, `a` aggregate, `w` window, as the engine reports it.
    pub kind: Name,
    /// The argument count (`-1`: any).
    pub narg: i64,
    /// `SQLITE_DETERMINISTIC` is set.
    pub deterministic: bool,
}

/// What one connection offers besides its schemas (alg/02 HV-INTRO-08 step 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteBuiltins {
    pub functions: Vec<SqliteFunction>,
    /// `None` where the connection refuses `pragma_module_list` (the user's read-only
    /// connection, R-M4-38).
    pub modules: Option<Vec<Name>>,
    /// In `seq` order.
    pub collations: Vec<Name>,
}

/// `SQLITE_DETERMINISTIC` in `pragma_function_list.flags`.
const SQLITE_DETERMINISTIC: i64 = 0x800;

/// The schema-level state mark of a schema (HV-INTRO-15): XXH3-64 of its `schema_version` and
/// its digest, little-endian.
pub fn schema_mark(schema_version: u32, digest: u128) -> StateMark {
    let mut b = [0u8; 20];
    b[..4].copy_from_slice(&schema_version.to_le_bytes());
    b[4..].copy_from_slice(&digest.to_le_bytes());
    StateMark::Hash(twox_hash::XxHash3_64::oneshot(&b))
}

/// The XXH3-128 digest of `rows`, sorted first (byte order of type, name, tbl_name, sql), each
/// field length-prefixed and NULL distinct from empty: independent of the order the server
/// returned them in.
fn schema_digest(rows: &[ObjRow]) -> u128 {
    let mut sorted: Vec<&ObjRow> = rows.iter().collect();
    sorted.sort();
    let mut buf = Vec::new();
    for r in sorted {
        for f in [Some(&r.ty), Some(&r.name), Some(&r.tbl), r.sql.as_ref()] {
            match f {
                Some(s) => {
                    buf.push(1);
                    buf.extend_from_slice(&(s.len() as u64).to_le_bytes());
                    buf.extend_from_slice(s.as_bytes());
                }
                None => buf.push(0),
            }
        }
    }
    twox_hash::XxHash3_128::oneshot(&buf)
}

/// XXH3-64 of the sorted `(kind code, name)` of `rels`.
fn existence_digest(mut rels: Vec<(ObjectKind, Name)>) -> u64 {
    rels.sort();
    let mut buf = Vec::new();
    for (k, n) in rels {
        buf.extend_from_slice(&k.code().to_le_bytes());
        buf.extend_from_slice(&(n.len() as u64).to_le_bytes());
        buf.extend_from_slice(n.as_bytes());
    }
    twox_hash::XxHash3_64::oneshot(&buf)
}

/// What [`Introspector::check_namespace`] answers, computed from a cached namespace: the
/// relation count, the mark of its watermark and its existence digest. `None` when the
/// namespace holds no SQLite watermark (never loaded by a full pass).
pub fn ns_check_of(snap: &NamespaceSnapshot) -> Option<NsCheck> {
    let Watermark::Sqlite {
        schema_version,
        digest,
        ..
    } = &snap.meta().watermark
    else {
        return None;
    };
    let rels: Vec<(ObjectKind, Name)> = snap
        .children(0)
        .filter(|(_, o)| RELATION_KINDS.contains(&o.kind()))
        .map(|(_, o)| (o.kind(), o.name().clone()))
        .collect();
    Some(NsCheck {
        objects: rels.len() as u64,
        max_mark: schema_mark(*schema_version, *digest),
        existence_digest: existence_digest(rels),
    })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// `(dev, inode)` of `path` on the blocking pool; `None` off Unix or when it cannot be read.
async fn file_identity(path: PathBuf) -> Option<(u64, u64)> {
    tokio::task::spawn_blocking(move || identity_of(&path))
        .await
        .ok()
        .flatten()
}

#[cfg(unix)]
fn identity_of(path: &std::path::Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(path).ok()?;
    Some((m.dev(), m.ino()))
}

#[cfg(not(unix))]
fn identity_of(_path: &std::path::Path) -> Option<(u64, u64)> {
    None
}

/// The schema a SQLite namespace key names (`[name]`).
fn ns_name(ns: &NsKey) -> Result<&str, IntroError> {
    match (ns.kind, ns.path.as_slice()) {
        (NsKind::Schema, [n]) => Ok(n),
        _ => Err(IntroError::Protocol(Name::from("namespace"))),
    }
}

fn root_key(ns: &NsKey) -> ObjectKey {
    ObjectKey {
        kind: ObjectKind::Schema,
        path: ns.path.iter().cloned().collect(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    }
}

fn pack() -> Result<Pack, IntroError> {
    Pack::builtin(DialectId::Sqlite).map_err(|_| IntroError::Protocol(Name::from("pack")))
}

/// The entry of schema `name` (SQLite compares schema names ASCII-caselessly).
fn entry<'a>(list: &'a [DbEntry], name: &str) -> Option<&'a DbEntry> {
    list.iter().find(|e| e.name.eq_ignore_ascii_case(name))
}

fn nonempty(file: &Name) -> Option<PathBuf> {
    (!file.is_empty()).then(|| PathBuf::from(&**file))
}

/// The schemas worth listing: [`ATTACH_ALIAS`] is left out when it is this introspector's
/// second attachment of another listed file.
fn listed(list: &[DbEntry]) -> Vec<&DbEntry> {
    list.iter()
        .filter(|e| {
            !(e.name.eq_ignore_ascii_case(ATTACH_ALIAS)
                && list.iter().any(|o| {
                    !o.name.eq_ignore_ascii_case(ATTACH_ALIAS)
                        && !o.file.is_empty()
                        && o.file == e.file
                }))
        })
        .collect()
}

/// Where a schema is read, and its file.
struct Place {
    variant: Variant,
    file: Option<PathBuf>,
}

/// What a pass decided before reading the schema's objects.
enum Decide {
    Reread,
    /// Compare the digest with the stored one.
    Digest(u128),
    /// Version and identity match on a reused connection.
    Skip(u128),
}

impl SqliteIntrospector {
    /// Records every pack section this introspector runs into `trace`.
    pub fn with_trace(mut self, trace: SectionTrace) -> Self {
        self.trace = Some(trace);
        self
    }

    /// Reports each pass's counts to `sink`: relations read (`Tables`), relations whose
    /// columns are read (`Columns`), indexes (`Indexes`) and foreign keys (`Constraints`).
    pub fn with_progress(mut self, sink: ProgressSink) -> Self {
        self.progress = Some(sink);
        self
    }

    fn cx<'a>(
        &'a self,
        pack: &'a Pack,
        c: &MetaConn,
        level: Level,
        cancel: &'a CancellationToken,
    ) -> Cx<'a> {
        // Only `MetaConn::join` makes a connection that does not know its engine.
        Cx::new(
            pack,
            c.dbms() == DbmsKind::Unknown,
            level,
            self.trace.as_ref(),
            cancel,
        )
    }

    /// The connection's schemas (`pragma_database_list`), on a snapshot of their own.
    async fn databases(cx: &Cx<'_>, c: &mut MetaConn) -> Result<Vec<DbEntry>, IntroError> {
        let mut snap = c.begin_snapshot().await?;
        let list = read::database_list(cx, &mut snap).await?;
        snap.commit().await?;
        Ok(list)
    }

    /// Where schema `name` is read on `c`, attaching an attached file under [`ATTACH_ALIAS`]
    /// first when the reader has not yet (module docs).
    async fn place(cx: &Cx<'_>, c: &mut MetaConn, name: &str) -> Result<Place, IntroError> {
        let list = Self::databases(cx, c).await?;
        if name.eq_ignore_ascii_case("main") {
            return Ok(Place {
                variant: Variant::Main,
                file: entry(&list, "main").and_then(|e| nonempty(&e.file)),
            });
        }
        if name.eq_ignore_ascii_case("temp") {
            return Ok(Place {
                variant: Variant::Temp,
                file: None,
            });
        }
        let unavailable = || IntroError::Meta(MetaError::Unavailable);
        let e = entry(&list, name).ok_or_else(unavailable)?;
        let file = nonempty(&e.file).ok_or_else(unavailable)?;
        if !name.eq_ignore_ascii_case(ATTACH_ALIAS) {
            match entry(&list, ATTACH_ALIAS) {
                Some(a) if a.file == e.file => {}
                Some(_) => return Err(unavailable()),
                None => c.attach_read_only(ATTACH_ALIAS, &file).await?,
            }
        }
        Ok(Place {
            variant: Variant::Attached,
            file: Some(file),
        })
    }

    /// One pass over `ns` (see [`Introspector::introspect`]); `on` says whether the connection
    /// read this namespace before (module docs).
    #[allow(clippy::too_many_arguments, reason = "the trait's arguments plus `on`")]
    pub fn pass<'a>(
        &'a self,
        c: &'a mut MetaConn,
        ns: &'a NsKey,
        mode: Mode,
        level: Level,
        filter: &'a CompiledFilter,
        prev: Option<&'a NamespaceSnapshot>,
        on: PassConn,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<NamespaceResult, IntroError>> {
        Box::pin(async move {
            if mode == Mode::System {
                // SQLite has no system snapshot (R-M4-6): its built-ins are read live.
                return Err(IntroError::NotImplemented);
            }
            let level = if mode == Mode::Brief {
                Level::Names
            } else {
                level
            };
            let mut attempt = 0;
            loop {
                attempt += 1;
                match self
                    .pass_once(c, ns, mode, level, filter, prev, on, cancel)
                    .await
                {
                    Err(e) if read::is_busy(&e) && attempt < BUSY_ATTEMPTS => {}
                    other => return other,
                }
            }
        })
    }

    #[allow(clippy::too_many_arguments, reason = "the pass's arguments")]
    async fn pass_once(
        &self,
        c: &mut MetaConn,
        ns: &NsKey,
        mode: Mode,
        level: Level,
        filter: &CompiledFilter,
        prev: Option<&NamespaceSnapshot>,
        on: PassConn,
        cancel: &CancellationToken,
    ) -> Result<NamespaceResult, IntroError> {
        let name = ns_name(ns)?;
        let pack = pack()?;
        let mut cx = self.cx(&pack, c, level, cancel);
        cx.progress = self.progress.as_ref().map(|p| (ns, p));
        let place = Self::place(&cx, c, name).await?;
        let identity = match place.file.clone() {
            Some(f) => file_identity(f).await,
            None => None,
        };
        let mut snap = c.begin_snapshot().await?;
        let version = match place.variant {
            Variant::Main => Some(read::schema_version(&cx, &mut snap).await?),
            _ => None,
        };
        let decide = self.decide(prev, mode, level, version, identity, on);
        let wm = |digest| Watermark::Sqlite {
            schema_version: version.unwrap_or(0),
            file_identity: identity,
            digest,
        };
        let unchanged = |digest| NamespaceResult {
            new_watermark: wm(digest),
            full: false,
            level,
            read_at_ms: now_ms(),
            ..NamespaceResult::empty_full()
        };
        if let Decide::Skip(d) = decide {
            snap.commit().await?;
            return Ok(unchanged(d));
        }
        let objects = read::objects(&cx, &mut snap, place.variant).await?;
        let relations = objects
            .iter()
            .filter(|o| relation_kind(o).is_some())
            .count() as u64;
        cx.report(Phase::Tables, relations, Some(relations));
        let digest = schema_digest(&objects);
        if let Decide::Digest(d) = decide
            && d == digest
        {
            snap.commit().await?;
            return Ok(unchanged(d));
        }
        let rows = read::read_schema(
            &cx,
            &mut snap,
            place.variant,
            level,
            objects,
            place.variant != Variant::Temp,
        )
        .await?;
        snap.commit().await?;
        let temp = place.variant == Variant::Temp;
        let mark = schema_mark(version.unwrap_or(0), digest);
        let built = build::build(ns, &rows, level, temp, mark, None, Some(filter));
        let parent = root_key(ns);
        let mut changed = Vec::with_capacity(built.relations.len() + 1);
        if temp {
            // The temp root carries `TEMP`: the namespace is never persisted (alg/02 edge 8).
            changed.push(ChangedObject {
                parent: parent.clone(),
                subtree: Subtree::new(Obj::new(ObjectKind::Schema, name).add_flags(ObjFlags::TEMP)),
            });
        }
        changed.extend(built.relations.into_iter().map(|subtree| ChangedObject {
            parent: parent.clone(),
            subtree,
        }));
        Ok(NamespaceResult {
            existent: built.existent,
            changed,
            categories_to_reload: BTreeSet::new(),
            new_watermark: wm(digest),
            full: true,
            level,
            read_at_ms: now_ms(),
        })
    }

    fn decide(
        &self,
        prev: Option<&NamespaceSnapshot>,
        mode: Mode,
        level: Level,
        version: Option<u32>,
        identity: Option<(u64, u64)>,
        on: PassConn,
    ) -> Decide {
        let Some(p) = prev.filter(|_| mode != Mode::Full) else {
            return Decide::Reread;
        };
        if p.meta().level < level {
            return Decide::Reread;
        }
        let Watermark::Sqlite {
            schema_version,
            file_identity,
            digest,
        } = &p.meta().watermark
        else {
            return Decide::Reread;
        };
        if version.is_some_and(|v| v != *schema_version) {
            return Decide::Reread;
        }
        let digest_needed = on == PassConn::Fresh
            || self.digest_every_pass
            || version.is_none()
            || identity.is_none()
            || *file_identity != identity;
        if digest_needed {
            Decide::Digest(*digest)
        } else {
            Decide::Skip(*digest)
        }
    }

    /// The connection's functions, modules and collations, live (no snapshot, R-M4-6).
    pub fn builtins<'a>(
        &'a self,
        c: &'a mut MetaConn,
    ) -> BoxFuture<'a, Result<SqliteBuiltins, IntroError>> {
        Box::pin(async move {
            let pack = pack()?;
            let cancel = CancellationToken::new();
            let cx = self.cx(&pack, c, Level::Names, &cancel);
            let mut snap = c.begin_snapshot().await?;
            let functions = read::rows(&cx, &mut snap, "FunctionList", &[])
                .await?
                .iter()
                .map(|r| SqliteFunction {
                    name: read::name(r, 0),
                    builtin: read::int(r, 1).is_some_and(|v| v != 0),
                    kind: read::name(r, 2),
                    narg: read::int(r, 3).unwrap_or(-1),
                    deterministic: read::int(r, 4).is_some_and(|f| f & SQLITE_DETERMINISTIC != 0),
                })
                .collect();
            let mut collations: Vec<(i64, Name)> = read::rows(&cx, &mut snap, "CollationList", &[])
                .await?
                .iter()
                .map(|r| (read::int(r, 0).unwrap_or(i64::MAX), read::name(r, 1)))
                .collect();
            collations.sort();
            let modules = match read::rows(&cx, &mut snap, "ModuleList", &[]).await {
                Ok(rs) => Some(rs.iter().map(|r| read::name(r, 0)).collect()),
                Err(IntroError::Meta(
                    MetaError::Server { .. }
                    | MetaError::RefusedText
                    | MetaError::ReadOnlyNotEnforced,
                )) => None,
                Err(e) => return Err(e),
            };
            snap.commit().await?;
            Ok(SqliteBuiltins {
                functions,
                modules,
                collations: collations.into_iter().map(|(_, n)| n).collect(),
            })
        })
    }

    /// The identity of the profile's database file, for the cache's server fingerprint (the
    /// coming `Introspector::identity`, R-W3-3): XXH3-128 of the main file's canonical path and
    /// its `application_id`. Both survive DDL and `VACUUM`; a file moved elsewhere or replaced
    /// by another format is another database. `None` (unknown) for an in-memory database or a
    /// path that does not resolve. The inode is left out: a tool that saves by writing a new
    /// file and renaming it over the old one keeps the same database.
    pub fn file_fingerprint<'a>(
        &'a self,
        c: &'a mut MetaConn,
    ) -> BoxFuture<'a, Result<Option<u128>, IntroError>> {
        Box::pin(async move {
            let pack = pack()?;
            let cancel = CancellationToken::new();
            let cx = self.cx(&pack, c, Level::Names, &cancel);
            let list = Self::databases(&cx, c).await?;
            let Some(path) = entry(&list, "main").and_then(|e| nonempty(&e.file)) else {
                return Ok(None);
            };
            let mut snap = c.begin_snapshot().await?;
            let app = read::rows(&cx, &mut snap, "ApplicationIdMain", &[])
                .await?
                .first()
                .and_then(|r| read::int(r, 0))
                .ok_or_else(|| IntroError::Protocol(Name::from("ApplicationIdMain")))?;
            snap.commit().await?;
            let canonical = tokio::task::spawn_blocking(move || std::fs::canonicalize(path).ok())
                .await
                .ok()
                .flatten();
            Ok(canonical.and_then(|p| {
                let p = p.to_str()?.to_string();
                let mut b = Vec::with_capacity(p.len() + 8);
                b.extend_from_slice(&app.to_le_bytes());
                b.extend_from_slice(p.as_bytes());
                Some(twox_hash::XxHash3_128::oneshot(&b))
            }))
        })
    }

    /// The attached files the connection lists (name, path): what a reader must attach to read
    /// the schemas the user attached on their own connection (`MetaConn::attach_read_only`).
    pub fn attached_files<'a>(
        &'a self,
        c: &'a mut MetaConn,
    ) -> BoxFuture<'a, Result<Vec<(Name, PathBuf)>, IntroError>> {
        Box::pin(async move {
            let pack = pack()?;
            let cancel = CancellationToken::new();
            let cx = self.cx(&pack, c, Level::Names, &cancel);
            let list = Self::databases(&cx, c).await?;
            Ok(listed(&list)
                .into_iter()
                .filter(|e| {
                    !e.name.eq_ignore_ascii_case("main") && !e.name.eq_ignore_ascii_case("temp")
                })
                .filter_map(|e| Some((e.name.clone(), nonempty(&e.file)?)))
                .collect())
        })
    }

    /// Reads `ns` for `stamps`/`check_namespace`/`facet`: its objects and mark, and (at
    /// `Structure`) the built objects' kinds and paths.
    async fn state_of(
        &self,
        c: &mut MetaConn,
        ns: &NsKey,
        level: Level,
    ) -> Result<(Vec<ObjRow>, StateMark, BTreeSet<(ObjectKind, Vec<Name>)>), IntroError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            match self.state_once(c, ns, level).await {
                Err(e) if read::is_busy(&e) && attempt < BUSY_ATTEMPTS => {}
                other => return other,
            }
        }
    }

    async fn state_once(
        &self,
        c: &mut MetaConn,
        ns: &NsKey,
        level: Level,
    ) -> Result<(Vec<ObjRow>, StateMark, BTreeSet<(ObjectKind, Vec<Name>)>), IntroError> {
        let name = ns_name(ns)?;
        let pack = pack()?;
        let cancel = CancellationToken::new();
        let cx = self.cx(&pack, c, level, &cancel);
        let place = Self::place(&cx, c, name).await?;
        let mut snap = c.begin_snapshot().await?;
        let (objects, mark, rows) = read_all(&cx, &mut snap, place.variant, level).await?;
        snap.commit().await?;
        let mut present = BTreeSet::new();
        if level >= Level::Structure {
            present = build::build(ns, &rows, level, false, mark, None, None).paths;
        } else {
            for o in &objects {
                if let Some(k) = relation_kind(o) {
                    present.insert((k, vec![o.name.clone()]));
                }
            }
        }
        Ok((objects, mark, present))
    }

    async fn fragment_once(
        &self,
        c: &mut MetaConn,
        ns: &NsKey,
        names: &BTreeSet<Name>,
        level: Level,
        filter: &CompiledFilter,
        cancel: &CancellationToken,
    ) -> Result<NamespaceResult, IntroError> {
        let name = ns_name(ns)?;
        let pack = pack()?;
        let mut cx = self.cx(&pack, c, level, cancel);
        cx.progress = self.progress.as_ref().map(|p| (ns, p));
        let place = Self::place(&cx, c, name).await?;
        let mut snap = c.begin_snapshot().await?;
        let (_, mark, rows) = read_all(&cx, &mut snap, place.variant, level).await?;
        snap.commit().await?;
        let temp = place.variant == Variant::Temp;
        let built = build::build(ns, &rows, level, temp, mark, Some(names), Some(filter));
        let parent = root_key(ns);
        Ok(NamespaceResult {
            existent: built.existent,
            changed: built
                .relations
                .into_iter()
                .map(|subtree| ChangedObject {
                    parent: parent.clone(),
                    subtree,
                })
                .collect(),
            categories_to_reload: BTreeSet::new(),
            // A fragment never vouches for the rest of the namespace: the next incremental
            // pass reads it whole.
            new_watermark: Watermark::None,
            full: false,
            level,
            read_at_ms: now_ms(),
        })
    }
}

/// Every schema read of one snapshot: objects, mark and (at `Structure`) the pragma rows.
async fn read_all(
    cx: &Cx<'_>,
    snap: &mut SnapshotTxn<'_>,
    v: Variant,
    level: Level,
) -> Result<(Vec<ObjRow>, StateMark, read::SchemaRows), IntroError> {
    let version = match v {
        Variant::Main => read::schema_version(cx, snap).await?,
        _ => 0,
    };
    let objects = read::objects(cx, snap, v).await?;
    let mark = schema_mark(version, schema_digest(&objects));
    let rows = read::read_schema(cx, snap, v, level, objects.clone(), v != Variant::Temp).await?;
    Ok((objects, mark, rows))
}

/// The namespace-relative path of `key` in `ns`, when it lies there.
fn rel_path(ns_name: &str, key: &ObjectKey) -> Option<Vec<Name>> {
    let (first, rest) = key.path.split_first()?;
    (first.eq_ignore_ascii_case(ns_name)).then(|| rest.to_vec())
}

/// Groups `keys` by their schema (the first path element), keeping their positions.
fn by_schema(keys: &[ObjectKey]) -> BTreeMap<Name, Vec<usize>> {
    let mut out: BTreeMap<Name, Vec<usize>> = BTreeMap::new();
    for (i, k) in keys.iter().enumerate() {
        if let Some(s) = k.path.first() {
            out.entry(s.clone()).or_default().push(i);
        }
    }
    out
}

impl Introspector for SqliteIntrospector {
    fn list_namespaces<'a>(
        &'a self,
        c: &'a mut MetaConn,
    ) -> BoxFuture<'a, Result<Vec<NsInfo>, IntroError>> {
        Box::pin(async move {
            let pack = pack()?;
            let cancel = CancellationToken::new();
            let cx = self.cx(&pack, c, Level::Names, &cancel);
            let list = Self::databases(&cx, c).await?;
            // `main` and `temp` always (the user's `temp` exists on their connection even when
            // this one has not opened its own), then the attached schemas in attach order.
            let mut names: Vec<Name> = vec![Name::from("main"), Name::from("temp")];
            for e in listed(&list) {
                if !e.name.eq_ignore_ascii_case("main") && !e.name.eq_ignore_ascii_case("temp") {
                    names.push(e.name.clone());
                }
            }
            Ok(names
                .into_iter()
                .map(|n| NsInfo {
                    key: NsKey::schema([n]),
                    system: false,
                })
                .collect())
        })
    }

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
        // The engine opens one reader per task: every pass is a connection's first.
        self.pass(c, ns, mode, level, filter, prev, PassConn::Fresh, cancel)
    }

    fn fragment<'a>(
        &'a self,
        c: &'a mut MetaConn,
        ns: &'a NsKey,
        keys: &'a [ObjectKey],
        level: Level,
        filter: &'a CompiledFilter,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<NamespaceResult, IntroError>> {
        Box::pin(async move {
            let name = ns_name(ns)?;
            // Every asked object stands for its relation (SQLite reads a relation whole).
            let names: BTreeSet<Name> = keys
                .iter()
                .filter_map(|k| rel_path(name, k)?.first().cloned())
                .collect();
            let mut attempt = 0;
            loop {
                attempt += 1;
                match self
                    .fragment_once(c, ns, &names, level, filter, cancel)
                    .await
                {
                    Err(e) if read::is_busy(&e) && attempt < BUSY_ATTEMPTS => {}
                    other => return other,
                }
            }
        })
    }

    fn stamps<'a>(
        &'a self,
        c: &'a mut MetaConn,
        keys: &'a [ObjectKey],
    ) -> BoxFuture<'a, Result<Vec<Option<StateMark>>, IntroError>> {
        Box::pin(async move {
            let mut out = vec![None; keys.len()];
            for (schema, at) in by_schema(keys) {
                let ns = NsKey::schema([schema.clone()]);
                // Relation keys are answered by `sqlite_schema`; anything else needs the pragmas.
                let level = if at.iter().all(|&i| {
                    keys[i].path.len() <= 2
                        && (keys[i].kind == ObjectKind::Schema
                            || RELATION_KINDS.contains(&keys[i].kind))
                }) {
                    Level::Names
                } else {
                    Level::Structure
                };
                let (_, mark, present) = self.state_of(c, &ns, level).await?;
                for i in at {
                    let k = &keys[i];
                    let Some(path) = rel_path(&schema, k) else {
                        continue;
                    };
                    let exists = if path.is_empty() {
                        k.kind == ObjectKind::Schema
                    } else {
                        present.contains(&(k.kind, path))
                    };
                    out[i] = exists.then_some(mark);
                }
            }
            Ok(out)
        })
    }

    fn facet<'a>(
        &'a self,
        c: &'a mut MetaConn,
        ns: &'a NsKey,
        keys: &'a [ObjectKey],
        kind: FacetKind,
    ) -> BoxFuture<'a, Result<FacetRows, IntroError>> {
        Box::pin(async move {
            match kind {
                // SQLite has no privileges: the facet is not applicable, and empty.
                FacetKind::Grants => Ok(FacetRows::Grants(
                    keys.iter()
                        .map(|k| (k.clone(), GrantsFacet::default()))
                        .collect(),
                )),
                // No statistics without `sqlite_stat1` and `ANALYZE`: no row estimate.
                FacetKind::Statistics => Ok(FacetRows::Statistics(
                    keys.iter()
                        .map(|k| {
                            (
                                k.clone(),
                                StatsFacet {
                                    row_estimate: None,
                                    at: SystemTime::now(),
                                },
                            )
                        })
                        .collect(),
                )),
                FacetKind::Dependencies => Err(IntroError::NotImplemented),
                FacetKind::Sources => {
                    let name = ns_name(ns)?;
                    let (objects, _, _) = self.state_of(c, ns, Level::Names).await?;
                    let mut out = Vec::new();
                    for k in keys {
                        let Some(path) = rel_path(name, k) else {
                            continue;
                        };
                        let row = objects.iter().find(|o| match (k.kind, path.as_slice()) {
                            (kind, [n]) if RELATION_KINDS.contains(&kind) => {
                                o.name == *n && relation_kind(o) == Some(kind)
                            }
                            (ObjectKind::Index, [t, n]) => {
                                &*o.ty == "index" && o.name == *n && o.tbl.eq_ignore_ascii_case(t)
                            }
                            (ObjectKind::Trigger, [t, n]) => {
                                &*o.ty == "trigger" && o.name == *n && o.tbl.eq_ignore_ascii_case(t)
                            }
                            _ => false,
                        });
                        if let Some(sql) = row.and_then(|o| o.sql.clone()) {
                            out.push((k.clone(), sql));
                        }
                    }
                    Ok(FacetRows::Sources(out))
                }
            }
        })
    }

    fn check_state<'a>(
        &'a self,
        c: &'a mut MetaConn,
        keys: &'a [ObjectKey],
    ) -> BoxFuture<'a, Result<Vec<Staleness>, IntroError>> {
        Box::pin(async move {
            Ok(self
                .stamps(c, keys)
                .await?
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
        _snap: &'a NamespaceSnapshot,
    ) -> BoxFuture<'a, Result<NsCheck, IntroError>> {
        Box::pin(async move {
            let (objects, mark, _) = self.state_of(c, ns, Level::Names).await?;
            let rels: Vec<(ObjectKind, Name)> = objects
                .iter()
                .filter_map(|o| relation_kind(o).map(|k| (k, o.name.clone())))
                .collect();
            Ok(NsCheck {
                objects: rels.len() as u64,
                max_mark: mark,
                existence_digest: existence_digest(rels),
            })
        })
    }

    /// R-W3-3: the file's identity ([`SqliteIntrospector::file_fingerprint`]); `Unknown` for an
    /// in-memory database or a path that does not resolve.
    fn identity<'a>(
        &'a self,
        c: &'a mut MetaConn,
    ) -> BoxFuture<'a, Result<ServerIdentity, IntroError>> {
        Box::pin(async move {
            Ok(match self.file_fingerprint(c).await? {
                Some(f) => ServerIdentity::Known(ServerFingerprint(f)),
                None => ServerIdentity::Unknown,
            })
        })
    }

    fn capabilities(&self) -> IntroCaps {
        IntroCaps {
            dbms: DbmsKind::Sqlite,
            transactional_ddl: true,
            incremental: true,
            facets: vec![FacetKind::Sources, FacetKind::Grants, FacetKind::Statistics],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(ty: &str, name: &str, sql: Option<&str>) -> ObjRow {
        ObjRow {
            ty: Name::from(ty),
            name: Name::from(name),
            tbl: Name::from(name),
            sql: sql.map(Name::from),
        }
    }

    #[test]
    fn digest_ignores_row_order_and_tells_null_from_empty() {
        let a = vec![
            row("table", "t", Some("CREATE TABLE t(a)")),
            row("index", "sqlite_autoindex_t_1", None),
            row("view", "v", Some("CREATE VIEW v AS SELECT 1")),
        ];
        let mut b = a.clone();
        b.reverse();
        assert_eq!(schema_digest(&a), schema_digest(&b));
        let mut c = a.clone();
        c[1].sql = Some(Name::from(""));
        assert_ne!(schema_digest(&a), schema_digest(&c));
        // Length prefixes: moving a byte between fields changes the digest.
        let d = vec![row("table", "ta", Some("b"))];
        let e = vec![row("table", "t", Some("ab"))];
        assert_ne!(schema_digest(&d), schema_digest(&e));
    }

    #[test]
    fn the_mark_depends_on_version_and_digest() {
        assert_eq!(schema_mark(1, 7), schema_mark(1, 7));
        assert_ne!(schema_mark(1, 7), schema_mark(2, 7));
        assert_ne!(schema_mark(1, 7), schema_mark(1, 8));
    }

    #[test]
    fn the_alias_is_listed_only_when_it_is_not_a_second_attachment() {
        let e = |seq, name: &str, file: &str| DbEntry {
            seq,
            name: Name::from(name),
            file: Name::from(file),
        };
        let list = vec![
            e(0, "main", "/m.db"),
            e(2, "arsiv", "/a.db"),
            e(3, ATTACH_ALIAS, "/a.db"),
        ];
        let names: Vec<&str> = listed(&list).iter().map(|e| &*e.name).collect();
        assert_eq!(names, vec!["main", "arsiv"]);
        let list = vec![e(0, "main", "/m.db"), e(2, ATTACH_ALIAS, "/own.db")];
        assert_eq!(listed(&list).len(), 2, "a user schema of that name is kept");
    }

    #[test]
    fn ns_key_must_be_one_schema() {
        assert!(ns_name(&NsKey::schema(["main"])).is_ok());
        assert!(ns_name(&NsKey::schema(["db", "main"])).is_err());
        assert!(ns_name(&NsKey::database("main")).is_err());
    }
}
