// ## m4-t01c-core
//! Facet loading (R-M4-30; Task 15b): single-flight per `(object, facet)`, `Introspector::facet`
//! on a pooled metadata connection, published as a side-table change that never touches the
//! object's structure.
//!
//! ## m4-t15b-facets (M4 Task 15b; ADR 0008 `## m4-t15b — facets and joined fragments`)
//!
//! [`IntroHandle::load_facet`] sends one request; the scheduler (the catalog's only writer)
//! runs it as a task of the object's namespace (`Work::Facet`), so a facet read never overlaps
//! a pass of the same namespace and is published in order with it.
//!
//! - **Keys.** Each key is recorded under the object's own key in the current generation (the
//!   arena's key, with its server id: [`CatalogSnapshot::facet_key`]). A key no arena holds is
//!   not read (its facet stays `NotLoaded`).
//! - **Single flight.** A key whose facet is loaded and not stale answers at once with no
//!   I/O; a request whose keys a running read of the same namespace and facet covers joins it;
//!   queued requests of one namespace and facet coalesce (the union of their keys).
//! - **Publish** ([`answered`]): one `CatalogChange::SetFacet` per key, no `Put`: no arena and
//!   no generation of any namespace changes. A key the read answered is `Loaded`; a key it left
//!   out is `NotLoaded` (R-W4-3: the read could not vouch for it completely, so nothing is
//!   shown as its facet). A refused read ([`failed`]): `Permission` and the definite errors are
//!   published `Failed` for every key (R-W4-3); a facet the introspector does not read
//!   (`IntroCaps::facets`, `NotImplemented`) is `Failed { Unsupported }`; a transient error
//!   (connection, timeout, cancel) publishes nothing.
//! - **Sources where the engine has no `Sources` facet** (PostgreSQL reads definitions as a
//!   level): the request becomes a fragment of the keys at `Level::Sources` (not a load: it
//!   reads even when the namespace is held at that level), whose texts the pass captures.
//! - **Source texts.** A pass or fragment captures every definition text it hashes into a
//!   `SourceRef` ([`source_ref`], [`capturing`]); the scheduler publishes them with the pass
//!   (`CatalogChange::PutSources`), and the catalog keeps a namespace's texts only while its
//!   arena references them. `CatalogSnapshot::source_text` serves `DdlSources::text`.
//! - **Invalidation** ([`invalidations`]): a pass or fragment that re-read or dropped an
//!   object drops its loaded `Grants`, `Dependencies` and `Sources` facets (and every facet of
//!   a dropped object); a `Dependencies` facet anywhere whose references name a re-read or
//!   dropped object (by server id, or by path for a column) is dropped too (T16b m-5). The next
//!   `load_facet` reads them again. T22 marks grants stale on a role change or a MySQL
//!   privilege-hash change; a stale facet is read again on the next `load_facet` and is never
//!   handed to a DDL producer (`CatalogSnapshot::grants_facet`).

#![allow(
    clippy::result_large_err,
    reason = "IntroError carries the server's message by value (MetaError::Server), as the seam does"
)]

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use hacivat_meta::delta::CatalogDelta;
use hacivat_meta::props::FacetKind;
use hacivat_meta::snapshot::{CatalogChange, ContainerKey, FacetValue, IntroErrorCode, LoadState};
use hacivat_meta::{
    CatalogSnapshot, Level, Name, NsKey, ObjectKey, ServerId, SourceRef, Watermark,
};

use super::{IntroHandle, Priority, scheduler};
use crate::introspector::{FacetRows, IntroError};
use crate::ticket::RefreshTicket;

/// Loads facet `kind` of `keys` through `handle`'s engine.
pub(crate) fn load(handle: &IntroHandle, keys: Vec<ObjectKey>, kind: FacetKind) -> RefreshTicket {
    handle.request(scheduler::Request::Facet(keys, kind), Priority::User)
}

// ---- source texts ------------------------------------------------------------------------------

type Captured = Arc<Mutex<Vec<(SourceRef, Name)>>>;

tokio::task_local! {
    static SOURCES: Captured;
}

/// The reference of definition text `text` ([`SourceRef::of`]); inside [`capturing`] the text
/// is also kept, for the catalog's content store. The introspectors call it where they hash a
/// view's, routine's or trigger's text.
pub fn source_ref(text: &str) -> SourceRef {
    let r = SourceRef::of(text);
    let _ = SOURCES.try_with(|s| {
        s.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((r, Name::from(text)));
    });
    r
}

/// Runs `fut`, answering with the texts it passed to [`source_ref`] (each once).
pub async fn capturing<F: std::future::Future>(fut: F) -> (F::Output, Vec<(SourceRef, Name)>) {
    let sink: Captured = Arc::default();
    let out = SOURCES.scope(sink.clone(), fut).await;
    let mut texts = std::mem::take(&mut *sink.lock().unwrap_or_else(PoisonError::into_inner));
    let mut seen = HashSet::new();
    texts.retain(|(r, _)| seen.insert(*r));
    (out, texts)
}

// ---- requests ----------------------------------------------------------------------------------

/// The keys of one request as the scheduler plans them: per namespace, the objects' own keys
/// whose facet is not loaded (or is stale); keys no arena holds are left out.
pub(crate) fn plan(
    snap: &CatalogSnapshot,
    keys: &[ObjectKey],
    kind: FacetKind,
) -> BTreeMap<NsKey, Vec<ObjectKey>> {
    let mut out: BTreeMap<NsKey, Vec<ObjectKey>> = BTreeMap::new();
    for k in keys {
        let Some(ContainerKey::Facet { ns, object, .. }) = snap.facet_key(k, kind) else {
            continue;
        };
        let key = ContainerKey::Facet {
            ns: ns.clone(),
            object: object.clone(),
            facet: kind,
        };
        if matches!(snap.state(&key), LoadState::Loaded { stale: false, .. }) {
            continue;
        }
        let v = out.entry(ns).or_default();
        if !v.contains(&object) {
            v.push(object);
        }
    }
    out
}

/// The container of facet `kind` of `object` in `ns`.
fn container(ns: &NsKey, object: &ObjectKey, kind: FacetKind) -> ContainerKey {
    ContainerKey::Facet {
        ns: ns.clone(),
        object: object.clone(),
        facet: kind,
    }
}

/// Whether `key` is still the object it was when the read started: the current generation
/// records it under the same key.
fn still(snap: &CatalogSnapshot, ns: &NsKey, key: &ObjectKey, kind: FacetKind) -> bool {
    snap.facet_key(key, kind) == Some(container(ns, key, kind))
}

/// The value of `key` in `rows` (rows are keyed by the keys the read was given).
fn value_of(rows: &FacetRows, key: &ObjectKey) -> Option<FacetValue> {
    fn find<'a, T>(v: &'a [(ObjectKey, T)], key: &ObjectKey) -> Option<&'a T> {
        v.iter()
            .find(|(k, _)| k == key)
            .or_else(|| v.iter().find(|(k, _)| k.same_object(key)))
            .map(|(_, t)| t)
    }
    match rows {
        FacetRows::Sources(v) => find(v, key).map(|t| FacetValue::Sources(t.clone())),
        FacetRows::Grants(v) => find(v, key).map(|g| FacetValue::Grants(Arc::new(g.clone()))),
        FacetRows::Dependencies(v) => {
            find(v, key).map(|d| FacetValue::Dependencies(Arc::new(d.clone())))
        }
        FacetRows::Statistics(v) => {
            find(v, key).map(|s| FacetValue::Statistics(Arc::new(s.clone())))
        }
    }
}

/// The facet kind `rows` hold.
fn kind_of(rows: &FacetRows) -> FacetKind {
    match rows {
        FacetRows::Sources(_) => FacetKind::Sources,
        FacetRows::Grants(_) => FacetKind::Grants,
        FacetRows::Dependencies(_) => FacetKind::Dependencies,
        FacetRows::Statistics(_) => FacetKind::Statistics,
    }
}

/// What publishes a read of facet `kind` of `keys` of `ns` that answered `rows` (module docs):
/// `Loaded` with its value for each key answered, `NotLoaded` for each key left out. Keys that
/// are no longer the same object in `snap` are skipped (the pass that changed them decides).
/// Rows of another facet than asked are a protocol error.
pub(crate) fn answered(
    snap: &CatalogSnapshot,
    ns: &NsKey,
    keys: &[ObjectKey],
    kind: FacetKind,
    rows: &FacetRows,
    level: Level,
) -> Result<Vec<CatalogChange>, IntroError> {
    if kind_of(rows) != kind {
        return Err(IntroError::Protocol(Name::from("facet kind")));
    }
    let at = SystemTime::now();
    Ok(keys
        .iter()
        .filter(|k| still(snap, ns, k, kind))
        .map(|k| match value_of(rows, k) {
            Some(value) => CatalogChange::SetFacet {
                key: container(ns, k, kind),
                state: LoadState::Loaded {
                    level,
                    at,
                    watermark: Watermark::None,
                    stale: false,
                },
                value: Some(value),
            },
            None => CatalogChange::SetFacet {
                key: container(ns, k, kind),
                state: LoadState::NotLoaded,
                value: None,
            },
        })
        .collect())
}

/// Whether a failed facet read decided something about its keys (module docs): a transient
/// error decides nothing.
pub(crate) fn decides(e: &IntroError) -> bool {
    use hacivat_session::meta::MetaError as M;
    match e {
        IntroError::Cancelled | IntroError::Timeout => false,
        IntroError::Meta(m) => matches!(
            m,
            M::NotJoinable | M::RefusedText | M::ReadOnlyNotEnforced | M::Server { .. }
        ),
        IntroError::NotImplemented
        | IntroError::Permission { .. }
        | IntroError::CatalogRace
        | IntroError::Protocol(_) => true,
    }
}

/// The code a decided failure is published with: `Unsupported` for a facet the introspector
/// does not read.
pub(crate) fn failure_code(e: &IntroError) -> IntroErrorCode {
    match e {
        IntroError::NotImplemented => IntroErrorCode::Unsupported,
        e => e.code(),
    }
}

/// What publishes a decided failure of facet `kind` of `keys` of `ns`: `Failed` for each key
/// that is still the same object (R-W4-3).
pub(crate) fn failed(
    snap: &CatalogSnapshot,
    ns: &NsKey,
    keys: &[ObjectKey],
    kind: FacetKind,
    error: IntroErrorCode,
    retry_after: std::time::Duration,
) -> Vec<CatalogChange> {
    let at = SystemTime::now();
    keys.iter()
        .filter(|k| still(snap, ns, k, kind))
        .map(|k| CatalogChange::SetFacet {
            key: container(ns, k, kind),
            state: LoadState::Failed {
                at,
                error,
                retry_after,
            },
            value: None,
        })
        .collect()
}

// ---- invalidation ------------------------------------------------------------------------------

/// The facets a re-read of their object makes stale (the other, `Statistics`, is an estimate
/// for display, dropped only with its object).
pub const REREAD_FACETS: [FacetKind; 3] = [
    FacetKind::Grants,
    FacetKind::Dependencies,
    FacetKind::Sources,
];

/// The facets a pass of `ns` that re-read `reread` and published `delta` drops (module docs):
/// `SetFacet … NotLoaded` changes, to publish with the pass.
pub(crate) fn invalidations(
    snap: &CatalogSnapshot,
    ns: &NsKey,
    reread: &[ObjectKey],
    delta: &CatalogDelta,
) -> Vec<CatalogChange> {
    let dropped: Vec<&ObjectKey> = delta
        .dropped
        .iter()
        .chain(delta.renamed.iter().map(|(before, _, _)| before))
        .chain(delta.out_of_scope.iter())
        .collect();
    if reread.is_empty() && dropped.is_empty() {
        return Vec::new();
    }
    let touched: Vec<&ObjectKey> = reread.iter().chain(dropped.iter().copied()).collect();
    let ids: HashSet<ServerId> = touched.iter().filter_map(|k| k.server_id).collect();
    let names = |r: &hacivat_meta::Ref| {
        (r.server_id.is_some_and(|s| ids.contains(&s)))
            || touched
                .iter()
                .any(|k| r.path.starts_with(k.path.as_slice()) && !k.path.is_empty())
    };
    let mut out = Vec::new();
    for key in snap.states.keys() {
        let ContainerKey::Facet {
            ns: n,
            object,
            facet,
        } = key
        else {
            continue;
        };
        let own = n == ns
            && ((REREAD_FACETS.contains(facet) && reread.iter().any(|k| k.same_object(object)))
                || dropped.iter().any(|k| k.same_object(object)));
        let deps = *facet == FacetKind::Dependencies
            && matches!(snap.facet_value(key), Some(FacetValue::Dependencies(d))
                if d.depends_on.iter().any(names));
        if own || deps {
            out.push(CatalogChange::SetFacet {
                key: key.clone(),
                state: LoadState::NotLoaded,
                value: None,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn capturing_keeps_each_hashed_text_once_and_nothing_outside() {
        // Outside a capture the reference is the plain hash.
        assert_eq!(source_ref("SELECT 1"), SourceRef::of("SELECT 1"));
        let ((), texts) = capturing(async {
            let a = source_ref("SELECT 1");
            let b = source_ref("SELECT 2");
            let again = source_ref("SELECT 1");
            assert_eq!(a, again);
            assert_ne!(a, b);
        })
        .await;
        assert_eq!(
            texts,
            vec![
                (SourceRef::of("SELECT 1"), Name::from("SELECT 1")),
                (SourceRef::of("SELECT 2"), Name::from("SELECT 2")),
            ]
        );
    }

    #[test]
    fn only_definite_errors_decide() {
        use hacivat_meta::ObjectKind;
        use hacivat_session::meta::MetaError;
        assert!(decides(&IntroError::Permission {
            kind: ObjectKind::Table
        }));
        assert!(decides(&IntroError::NotImplemented));
        assert!(decides(&IntroError::Meta(MetaError::NotJoinable)));
        for e in [
            IntroError::Cancelled,
            IntroError::Timeout,
            IntroError::Meta(MetaError::ConnectionLost),
            IntroError::Meta(MetaError::Busy),
            IntroError::Meta(MetaError::Unavailable),
            IntroError::Meta(MetaError::Timeout),
        ] {
            assert!(!decides(&e), "{e:?}");
        }
        assert_eq!(
            failure_code(&IntroError::NotImplemented),
            IntroErrorCode::Unsupported
        );
    }
}
