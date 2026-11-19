//! Introspection levels per namespace (HV-INTRO-01 "Auto level"; R-M4-21, D-038): the auto
//! level from a cheap object count, the user's per-container overrides (the pills, inherited by
//! children), and the engine's back-pressure lowering (a namespace whose pass timed out twice
//! is read one level lower, `intro-hint-level-lowered`).
//!
//! The merge never lowers a namespace's level (I-2); lowering is this policy's job: the engine
//! reads the namespace at the lowered level and publishes that level in its `LoadState`.

use std::collections::{BTreeMap, HashMap};

use hacivat_meta::snapshot::ContainerKey;
use hacivat_meta::{Level, NsKey, NsKind};

/// Where a namespace stands for the auto level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NsRole {
    /// The session's current namespace.
    Current,
    /// Any other user namespace in scope.
    Other,
    /// A system namespace named explicitly by the scope (`pg_catalog`, `mysql`, …).
    System,
}

/// The D-038 thresholds of one role, in major objects (top-level objects of the namespace).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Thresholds {
    /// At most this many: `Sources` (`None`: never `Sources` automatically).
    pub sources_max: Option<u32>,
    /// At most this many: `Structure`; above it: `Names`.
    pub structure_max: u32,
}

/// D-038 for the current namespace: `Sources` up to 2,000, `Structure` up to 20,000.
pub const CURRENT: Thresholds = Thresholds {
    sources_max: Some(2_000),
    structure_max: 20_000,
};

/// D-038 for other namespaces: `Structure` up to 5,000.
pub const OTHER: Thresholds = Thresholds {
    sources_max: None,
    structure_max: 5_000,
};

/// The auto level of a namespace of `role` holding `count` major objects (D-038, R-M4-21).
pub fn auto_level(role: NsRole, count: u32) -> Level {
    LevelPolicy::default().auto(role, count)
}

impl Thresholds {
    /// The level of a namespace of `count` major objects under these thresholds.
    pub fn level(self, count: u32) -> Level {
        if self.sources_max.is_some_and(|max| count <= max) {
            Level::Sources
        } else if count <= self.structure_max {
            Level::Structure
        } else {
            Level::Names
        }
    }
}

/// One level lower (`Names` stays `Names`).
pub fn lower(level: Level) -> Level {
    match level {
        Level::Sources => Level::Structure,
        Level::Structure | Level::Names => Level::Names,
    }
}

/// Where an effective level came from (the pill shows "auto" or the user's choice).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LevelSource {
    /// The D-038 auto level (possibly lowered by back-pressure).
    Auto,
    /// A user override on the container itself or inherited from an ancestor.
    Override,
}

/// A namespace's effective level and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LevelChoice {
    pub level: Level,
    pub source: LevelSource,
    /// Back-pressure lowered the auto level this many steps.
    pub lowered: u8,
}

/// The level policy of one profile: D-038's thresholds, the overrides and the lowering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelPolicy {
    pub current: Thresholds,
    pub other: Thresholds,
    pub system: Level,
    overrides: BTreeMap<ContainerKey, Level>,
    lowered: HashMap<NsKey, u8>,
}

impl Default for LevelPolicy {
    fn default() -> Self {
        LevelPolicy {
            current: CURRENT,
            other: OTHER,
            system: Level::Names,
            overrides: BTreeMap::new(),
            lowered: HashMap::new(),
        }
    }
}

impl LevelPolicy {
    /// The auto level of a namespace of `role` with `count` major objects under this policy's
    /// thresholds.
    pub fn auto(&self, role: NsRole, count: u32) -> Level {
        match role {
            NsRole::Current => self.current.level(count),
            NsRole::Other => self.other.level(count),
            NsRole::System => self.system,
        }
    }

    /// Sets (`Some`) or clears (`None`) the override of `container` (a pill on a database or
    /// schema row). It applies to the container and every namespace below it that has no
    /// override of its own.
    pub fn set_override(&mut self, container: ContainerKey, level: Option<Level>) {
        match level {
            Some(l) => {
                self.overrides.insert(container, l);
            }
            None => {
                self.overrides.remove(&container);
            }
        }
    }

    /// The override that applies to `ns`: its own, else its database's (PostgreSQL schemas),
    /// else the server's.
    pub fn override_for(&self, ns: &NsKey) -> Option<Level> {
        ancestors(ns).find_map(|k| self.overrides.get(&ContainerKey::Namespace(k)).copied())
    }

    /// The overrides set, by container.
    pub fn overrides(&self) -> &BTreeMap<ContainerKey, Level> {
        &self.overrides
    }

    /// The level `ns` is read at: the override when one applies (never lowered: the user
    /// chose it), else the auto level of `role` and `count` lowered by back-pressure.
    pub fn level_for(&self, ns: &NsKey, role: NsRole, count: u32) -> LevelChoice {
        if let Some(level) = self.override_for(ns) {
            return LevelChoice {
                level,
                source: LevelSource::Override,
                lowered: 0,
            };
        }
        let steps = self.lowered.get(ns).copied().unwrap_or(0);
        let mut level = self.auto(role, count);
        for _ in 0..steps {
            level = lower(level);
        }
        LevelChoice {
            level,
            source: LevelSource::Auto,
            lowered: steps,
        }
    }

    /// Back-pressure: reads of `ns` go one level lower from now on. Answers the new auto level
    /// for `role` and `count`, or `None` when nothing changed (already `Names`, or an override
    /// applies).
    pub fn lower(&mut self, ns: &NsKey, role: NsRole, count: u32) -> Option<Level> {
        let before = self.level_for(ns, role, count);
        if before.source == LevelSource::Override || before.level == Level::Names {
            return None;
        }
        *self.lowered.entry(ns.clone()).or_insert(0) += 1;
        Some(self.level_for(ns, role, count).level)
    }

    /// Forgets the lowering of `ns` (the user refreshed it at a chosen level).
    pub fn reset_lowering(&mut self, ns: &NsKey) {
        self.lowered.remove(ns);
    }
}

/// `ns` and the namespaces above it, nearest first: a PostgreSQL schema `[db, s]`, then the
/// database `[db]`, then the server.
fn ancestors(ns: &NsKey) -> impl Iterator<Item = NsKey> {
    let mut out = vec![ns.clone()];
    if ns.kind == NsKind::Schema && ns.path.len() == 2 {
        out.push(NsKey::database(ns.path[0].clone()));
    }
    if ns.kind != NsKind::Server {
        out.push(NsKey::server());
    }
    out.into_iter()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowering_stops_at_names_and_never_touches_an_override() {
        let ns = NsKey::schema(["db", "s"]);
        let mut p = LevelPolicy::default();
        assert_eq!(p.lower(&ns, NsRole::Current, 10), Some(Level::Structure));
        assert_eq!(p.lower(&ns, NsRole::Current, 10), Some(Level::Names));
        assert_eq!(p.lower(&ns, NsRole::Current, 10), None);
        p.set_override(ContainerKey::Namespace(ns.clone()), Some(Level::Sources));
        assert_eq!(p.lower(&ns, NsRole::Current, 10), None);
        assert_eq!(
            p.level_for(&ns, NsRole::Current, 10).source,
            LevelSource::Override
        );
    }
}
