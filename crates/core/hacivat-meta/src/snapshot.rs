//! Catalog snapshots (HV-MODEL-02): an immutable map of namespace arenas published through
//! `arc-swap` with a monotonically increasing generation. Readers call [`Catalog::current`]
//! (one atomic increment, no lock) and may keep the `Arc` as long as they like; a publish
//! shares every untouched namespace by `Arc` (`Arc::ptr_eq` holds across generations), so
//! its cost is O(namespaces) pointer copies plus the rebuilt namespaces.
//!
//! The tri-state `CatalogView` (`Found | Missing | Unknown`) and per-container `LoadState`
//! sit on top of this in M4 (they need the scope and the scheduler, HV-INTRO-01/02).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use hacivat_proto::DbmsKind;

use crate::ids::{NsKey, NsKind, ObjId, ObjectKey, Ref};
use crate::kind::ObjectKind;
use crate::name::Name;
use crate::namespace::{ModelError, NamespaceSnapshot, Obj, Subtree};

/// One immutable generation of a connection's catalog.
#[derive(Debug, Clone)]
pub struct CatalogSnapshot {
    generation: u64,
    dbms: DbmsKind,
    namespaces: BTreeMap<NsKey, Arc<NamespaceSnapshot>>,
}

impl CatalogSnapshot {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn dbms(&self) -> DbmsKind {
        self.dbms
    }

    pub fn namespace(&self, key: &NsKey) -> Option<&Arc<NamespaceSnapshot>> {
        self.namespaces.get(key)
    }

    /// Every namespace, in key order.
    pub fn namespaces(&self) -> impl Iterator<Item = &Arc<NamespaceSnapshot>> + '_ {
        self.namespaces.values()
    }

    pub fn namespace_count(&self) -> usize {
        self.namespaces.len()
    }

    pub fn object_count(&self) -> usize {
        self.namespaces.values().map(|n| n.len()).sum()
    }

    pub fn column_count(&self) -> usize {
        self.namespaces.values().map(|n| n.column_count()).sum()
    }

    pub fn obj(&self, id: &ObjId) -> Option<&Obj> {
        self.namespaces.get(&id.ns)?.obj(id.idx)
    }

    /// The namespace that holds objects under `path` (the longest namespace key that is a
    /// prefix of `path`, schemas first), with the prefix length.
    fn namespace_of(&self, path: &[Name]) -> Option<(&Arc<NamespaceSnapshot>, usize)> {
        for k in (0..=path.len()).rev() {
            for kind in [NsKind::Schema, NsKind::Database, NsKind::Server] {
                let key = NsKey {
                    kind,
                    path: path[..k].iter().cloned().collect(),
                };
                if let Some(ns) = self.namespaces.get(&key) {
                    return Some((ns, k));
                }
            }
        }
        None
    }

    /// The object `key` denotes in this generation ([`ObjectKey::same_object`]).
    pub fn find(&self, key: &ObjectKey) -> Option<ObjId> {
        for k in (0..=key.path.len()).rev() {
            for kind in [NsKind::Schema, NsKind::Database, NsKind::Server] {
                let ns_key = NsKey {
                    kind,
                    path: key.path[..k].iter().cloned().collect(),
                };
                if let Some(ns) = self.namespaces.get(&ns_key)
                    && let Some(idx) = ns.find(key)
                {
                    return Some(ObjId { ns: ns_key, idx });
                }
            }
        }
        None
    }

    /// Resolves a reference on demand: the target namespace by path, then the name index of
    /// the target kind's group; a server id match wins among several hits. `None` when the
    /// target namespace is not loaded or has no such object (the tri-state view maps that to
    /// `Unknown` or `Missing` from the load state).
    pub fn resolve(&self, r: &Ref) -> Option<ObjId> {
        let (name, parent) = r.path.split_last()?;
        let (ns, k) = self.namespace_of(parent)?;
        if k != parent.len() {
            return None;
        }
        let mut first = None;
        for idx in ns.lookup_kind(r.kind, name) {
            let o = ns.obj(idx)?;
            match (r.server_id, o.server_id) {
                (Some(a), Some(b)) if a == b => {
                    return Some(ObjId {
                        ns: ns.key().clone(),
                        idx,
                    });
                }
                _ => {
                    first.get_or_insert(idx);
                }
            }
        }
        first.map(|idx| ObjId {
            ns: ns.key().clone(),
            idx,
        })
    }
}

/// An incremental update to the catalog.
#[derive(Debug, Clone)]
pub enum CatalogChange {
    /// Inserts or replaces a whole namespace (a full or incremental pass; a cache load).
    Put(NamespaceSnapshot),
    /// Removes a namespace (a dropped schema).
    Remove(NsKey),
    /// Replaces the child of `parent` that is the same object as `subtree`'s root (a table
    /// and its columns, indexes, constraints after a fragment refresh), or inserts it.
    UpsertSubtree {
        ns: NsKey,
        parent: ObjectKey,
        subtree: Subtree,
    },
    /// Removes an object and everything under it (a dropped table).
    RemoveSubtree { ns: NsKey, target: ObjectKey },
}

/// What a publish produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    pub generation: u64,
    /// The namespaces that changed (replaced, inserted or removed), in change order.
    pub changed: Vec<NsKey>,
}

/// One connection profile's catalog: the current [`CatalogSnapshot`] behind an `ArcSwap`.
#[derive(Debug)]
pub struct Catalog {
    current: ArcSwap<CatalogSnapshot>,
    /// Serializes writers (the HV-INTRO-01 scheduler is the only one; this guards misuse).
    /// Readers never take it.
    writer: Mutex<()>,
}

impl Catalog {
    /// An empty catalog at generation 0.
    pub fn new(dbms: DbmsKind) -> Self {
        Catalog {
            current: ArcSwap::from_pointee(CatalogSnapshot {
                generation: 0,
                dbms,
                namespaces: BTreeMap::new(),
            }),
            writer: Mutex::new(()),
        }
    }

    /// The current generation. Lock-free; keep the `Arc` to pin a generation.
    pub fn current(&self) -> Arc<CatalogSnapshot> {
        self.current.load_full()
    }

    /// Applies `changes` atomically: on any error nothing is published and the current
    /// generation stays. Every replaced or inserted namespace carries the new generation.
    pub fn publish(
        &self,
        changes: impl IntoIterator<Item = CatalogChange>,
    ) -> Result<Published, ModelError> {
        let _w = self
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let old = self.current.load_full();
        let generation = old.generation + 1;
        let mut namespaces = old.namespaces.clone();
        let mut changed = Vec::new();
        for change in changes {
            match change {
                CatalogChange::Put(mut ns) => {
                    ns.set_generation(generation);
                    changed.push(ns.key().clone());
                    namespaces.insert(ns.key().clone(), Arc::new(ns));
                }
                CatalogChange::Remove(key) => {
                    if namespaces.remove(&key).is_none() {
                        return Err(ModelError::NoSuchNamespace(key));
                    }
                    changed.push(key);
                }
                CatalogChange::UpsertSubtree {
                    ns,
                    parent,
                    subtree,
                } => {
                    let cur = namespaces
                        .get(&ns)
                        .ok_or_else(|| ModelError::NoSuchNamespace(ns.clone()))?;
                    let at = cur.find(&parent).ok_or(ModelError::NoSuchObject)?;
                    let mut next = cur.upsert_child(at, subtree)?;
                    next.set_generation(generation);
                    namespaces.insert(ns.clone(), Arc::new(next));
                    changed.push(ns);
                }
                CatalogChange::RemoveSubtree { ns, target } => {
                    let cur = namespaces
                        .get(&ns)
                        .ok_or_else(|| ModelError::NoSuchNamespace(ns.clone()))?;
                    let at = cur.find(&target).ok_or(ModelError::NoSuchObject)?;
                    let mut next = cur.replace_subtree(at, None)?;
                    next.set_generation(generation);
                    namespaces.insert(ns.clone(), Arc::new(next));
                    changed.push(ns);
                }
            }
        }
        self.current.store(Arc::new(CatalogSnapshot {
            generation,
            dbms: old.dbms,
            namespaces,
        }));
        Ok(Published {
            generation,
            changed,
        })
    }
}

/// The key of `kind` named `name` under the object `parent` (a convenience for
/// [`CatalogChange`] targets).
pub fn child_key(parent: &ObjectKey, kind: ObjectKind, name: impl Into<Name>) -> ObjectKey {
    let mut path = parent.path.clone();
    path.push(name.into());
    ObjectKey {
        kind,
        path,
        server_id: None,
        overload_sig: None,
    }
}
