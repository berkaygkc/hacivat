// ## m4-t16-pg (spike origin 5a5ab08, `src/pg/introspect.rs` `Facets`, the cascade of
// commit 14b7c1b)
//! What an incremental pass re-reads (R-M4-48): the `(facet, oid)` rows of `ChangedParents`
//! and `ChangedDependents`, removed comment keys, and the same-relation cascade.
//!
//! A relation has the facets `rel` (its own row), `columns`, `indexes`, `constraints`,
//! `triggers` and `comments`. A `columns` change also sets `rel` (a column rename re-deparses
//! the partition key, index expressions and checks, which store attnums); a `rel` change also
//! sets `indexes`, `constraints` and `triggers` (finding 5: an index backing a constraint is
//! two `pg_depend` hops from its table, so a table rename reaches its text only this way).
//! Indexes and constraints are re-read together ([`Facets::keys`]): Task 13 assembles a key
//! with its backing index.

use std::collections::{BTreeMap, BTreeSet};

use super::model::Oid;
use crate::introspector::IntroError;

/// The facets a relation needs re-read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Facets {
    pub rel: bool,
    pub columns: bool,
    pub indexes: bool,
    pub constraints: bool,
    pub triggers: bool,
    pub comments: bool,
}

impl Facets {
    /// The same-relation cascade (module docs).
    pub fn cascade(&mut self) {
        if self.columns {
            self.rel = true;
        }
        if self.rel {
            self.indexes = true;
            self.constraints = true;
            self.triggers = true;
        }
    }

    /// Indexes and constraints, read together.
    pub fn keys(&self) -> bool {
        self.indexes || self.constraints
    }

    /// Only the comments changed: the `Comments` section patches them.
    pub fn comments_only(&self) -> bool {
        self.comments && !self.rel && !self.columns
    }

    pub fn any(&self) -> bool {
        self.rel || self.columns || self.keys() || self.triggers || self.comments
    }
}

/// Everything an incremental pass found changed, by owner.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Changes {
    pub relations: BTreeMap<Oid, Facets>,
    pub sequences: BTreeSet<Oid>,
    pub routines: BTreeSet<Oid>,
    pub types: BTreeSet<Oid>,
    pub schema: bool,
    /// A schema the namespace references changed (`schema-rename`): the pass runs full.
    pub schema_rename: bool,
}

impl Changes {
    /// Adds one `(facet, oid)` row; an unknown facet is a protocol error (the pack and this
    /// code disagree).
    pub fn add(&mut self, facet: &str, oid: Oid) -> Result<(), IntroError> {
        fn rel(c: &mut Changes, oid: Oid) -> &mut Facets {
            c.relations.entry(oid).or_default()
        }
        match facet {
            "rel" => rel(self, oid).rel = true,
            "columns" => rel(self, oid).columns = true,
            "indexes" => rel(self, oid).indexes = true,
            "constraints" => rel(self, oid).constraints = true,
            "triggers" => rel(self, oid).triggers = true,
            "comments" => rel(self, oid).comments = true,
            "sequence" => {
                self.sequences.insert(oid);
            }
            "routine" => {
                self.routines.insert(oid);
            }
            "type" => {
                self.types.insert(oid);
            }
            "schema" => self.schema = true,
            "schema-rename" => self.schema_rename = true,
            other => {
                return Err(IntroError::Protocol(
                    format!("pg/ChangedParents: unknown facet {other:?}").into(),
                ));
            }
        }
        Ok(())
    }

    /// Applies the cascade to every relation and keeps only the relations that still exist.
    pub fn settle(&mut self, alive_relations: &BTreeSet<Oid>) {
        self.relations
            .retain(|oid, _| alive_relations.contains(oid));
        for f in self.relations.values_mut() {
            f.cascade();
        }
    }

    /// The relations whose facet `pick` is set, in oid order.
    pub fn relations_with(&self, pick: impl Fn(&Facets) -> bool) -> Vec<Oid> {
        self.relations
            .iter()
            .filter(|(_, f)| pick(f))
            .map(|(o, _)| *o)
            .collect()
    }

    /// Whether nothing needs a re-read.
    pub fn is_empty(&self) -> bool {
        !self.relations.values().any(Facets::any)
            && self.sequences.is_empty()
            && self.routines.is_empty()
            && self.types.is_empty()
            && !self.schema
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_facets_are_refused() {
        let mut c = Changes::default();
        assert!(c.add("rel", 1).is_ok());
        assert!(matches!(c.add("relx", 1), Err(IntroError::Protocol(_))));
    }

    #[test]
    fn settle_drops_dropped_relations() {
        let mut c = Changes::default();
        c.add("columns", 1).unwrap();
        c.add("rel", 2).unwrap();
        c.settle(&BTreeSet::from([1]));
        assert_eq!(c.relations.len(), 1);
        assert!(c.relations[&1].rel && c.relations[&1].triggers);
    }
}
