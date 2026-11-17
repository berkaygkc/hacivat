//! Applying a [`PgNamespaceDelta`] to the previous [`PgNamespace`] (HV-INTRO-11's merge, in
//! its simplest form). The spike uses it as the **reference** that proves an incremental pass
//! complete: `apply(prev, delta)` must equal a fresh full introspection. The M4 model owns
//! the real merge (identity, `CatalogDelta`, mass-deletion guard).

use std::collections::BTreeMap;

use super::model::*;

/// Sorts every list the way [`PgNamespace`] documents, so equal catalogs compare equal.
pub fn normalize(ns: &mut PgNamespace) {
    ns.relations
        .sort_by(|a, b| (&a.name, a.oid).cmp(&(&b.name, b.oid)));
    for r in &mut ns.relations {
        r.columns.sort_by_key(|c| c.num);
        r.indexes
            .sort_by(|a, b| (&a.name, a.oid).cmp(&(&b.name, b.oid)));
        r.constraints
            .sort_by(|a, b| (&a.name, a.oid).cmp(&(&b.name, b.oid)));
        r.triggers
            .sort_by(|a, b| (&a.name, a.oid).cmp(&(&b.name, b.oid)));
    }
    ns.sequences
        .sort_by(|a, b| (&a.name, a.oid).cmp(&(&b.name, b.oid)));
    ns.routines.sort_by(|a, b| {
        (&a.name, &a.identity_args, a.oid).cmp(&(&b.name, &b.identity_args, b.oid))
    });
    ns.types
        .sort_by(|a, b| (&a.name, a.oid).cmp(&(&b.name, b.oid)));
    for t in &mut ns.types {
        if let UserTypeKind::Domain { constraints, .. } = &mut t.kind {
            constraints.sort_by(|a, b| (&a.name, a.oid).cmp(&(&b.name, b.oid)));
        }
    }
}

/// `prev` with `delta` applied: objects missing from `delta.alive` are dropped, re-read
/// objects and facets replace their previous versions.
pub fn apply(prev: &PgNamespace, delta: &PgNamespaceDelta) -> PgNamespace {
    let alive = &delta.alive;
    let mut rels: BTreeMap<Oid, PgRelation> = prev
        .relations
        .iter()
        .filter(|r| alive.relations.contains(&r.oid))
        .map(|r| {
            let mut r = r.clone();
            r.indexes.retain(|i| alive.children.contains(&i.oid));
            r.constraints.retain(|k| alive.children.contains(&k.oid));
            r.triggers.retain(|t| alive.children.contains(&t.oid));
            (r.oid, r)
        })
        .collect();
    for p in &delta.relations {
        if let Some(h) = &p.header {
            match rels.get_mut(&p.oid) {
                Some(r) => {
                    let mut h = h.clone();
                    h.columns = std::mem::take(&mut r.columns);
                    h.indexes = std::mem::take(&mut r.indexes);
                    h.constraints = std::mem::take(&mut r.constraints);
                    h.triggers = std::mem::take(&mut r.triggers);
                    *r = h;
                }
                None => {
                    rels.insert(p.oid, h.clone());
                }
            }
        }
        let Some(r) = rels.get_mut(&p.oid) else {
            continue;
        };
        if let Some(c) = &p.columns {
            r.columns = c.clone();
        }
        if let Some(i) = &p.indexes {
            r.indexes = i.clone();
        }
        if let Some(k) = &p.constraints {
            r.constraints = k.clone();
        }
        if let Some(t) = &p.triggers {
            r.triggers = t.clone();
        }
        if let Some(c) = &p.comments {
            r.comment = c.relation.clone();
            let by_num: BTreeMap<i16, &String> = c.columns.iter().map(|(n, s)| (*n, s)).collect();
            for col in &mut r.columns {
                col.comment = by_num.get(&col.num).map(|s| (*s).clone());
            }
        }
    }
    let mut ns = PgNamespace {
        schema: delta.schema.clone().unwrap_or_else(|| prev.schema.clone()),
        relations: rels.into_values().collect(),
        sequences: upsert(&prev.sequences, &delta.sequences, &alive.sequences, |s| {
            s.oid
        }),
        routines: upsert(&prev.routines, &delta.routines, &alive.routines, |r| r.oid),
        types: upsert(&prev.types, &delta.types, &alive.types, |t| t.oid),
    };
    for t in &mut ns.types {
        if let UserTypeKind::Domain { constraints, .. } = &mut t.kind {
            constraints.retain(|k| alive.children.contains(&k.oid));
        }
    }
    normalize(&mut ns);
    ns
}

fn upsert<T: Clone>(
    prev: &[T],
    fresh: &[T],
    alive: &std::collections::BTreeSet<Oid>,
    oid: impl Fn(&T) -> Oid,
) -> Vec<T> {
    let mut m: BTreeMap<Oid, T> = prev
        .iter()
        .filter(|x| alive.contains(&oid(x)))
        .map(|x| (oid(x), x.clone()))
        .collect();
    for x in fresh {
        m.insert(oid(x), x.clone());
    }
    m.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(oid: Oid, name: &str) -> PgRelation {
        PgRelation {
            oid,
            name: name.into(),
            kind: RelKind::Table,
            owner: "o".into(),
            persistence: Persistence::Permanent,
            comment: None,
            parents: vec![],
            partition_bound: None,
            partition_key: None,
            view_definition: None,
            populated: true,
            options: vec![],
            columns: vec![],
            indexes: vec![],
            constraints: vec![],
            triggers: vec![],
        }
    }

    fn col(num: i16, name: &str) -> PgColumn {
        PgColumn {
            num,
            name: name.into(),
            ty: PgTypeRef {
                oid: 23,
                display: "integer".into(),
                schema: "pg_catalog".into(),
                name: "int4".into(),
                kind: TypeKind::Base,
            },
            array_dims: 0,
            not_null: false,
            default: None,
            identity: None,
            generated: None,
            collation: None,
            inherited: false,
            comment: None,
        }
    }

    fn idx(oid: Oid, name: &str) -> PgIndex {
        PgIndex {
            oid,
            name: name.into(),
            method: "btree".into(),
            unique: false,
            primary: false,
            exclusion: false,
            immediate: true,
            valid: true,
            nulls_not_distinct: false,
            keys: vec![],
            include: vec![],
            predicate: None,
            definition: None,
            comment: None,
        }
    }

    fn ns(relations: Vec<PgRelation>) -> PgNamespace {
        PgNamespace {
            schema: PgSchema {
                oid: 1,
                name: "s".into(),
                owner: "o".into(),
                comment: None,
                system: false,
            },
            relations,
            sequences: vec![],
            routines: vec![],
            types: vec![],
        }
    }

    #[test]
    fn drops_follow_the_alive_sets_and_patches_replace_facets() {
        let mut a = rel(10, "a");
        a.columns = vec![col(1, "x")];
        a.indexes = vec![idx(100, "a_i1"), idx(101, "a_i2")];
        let prev = ns(vec![a, rel(11, "b")]);
        let mut delta = PgNamespaceDelta::default();
        delta.alive.relations = [10, 12].into();
        delta.alive.children = [100].into();
        let mut c = rel(12, "c");
        c.comment = Some("new".into());
        delta.relations = vec![
            RelationPatch {
                oid: 10,
                columns: Some(vec![col(1, "x"), col(2, "y")]),
                comments: Some(RelComments {
                    relation: Some("ra".into()),
                    columns: vec![(2, "cy".into())],
                }),
                ..RelationPatch::default()
            },
            RelationPatch {
                oid: 12,
                header: Some(c),
                columns: Some(vec![col(1, "z")]),
                ..RelationPatch::default()
            },
        ];
        let next = apply(&prev, &delta);
        let names: Vec<&str> = next.relations.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["a", "c"]);
        let a = &next.relations[0];
        assert_eq!(a.columns.len(), 2);
        assert_eq!(a.comment.as_deref(), Some("ra"));
        assert_eq!(a.columns[0].comment, None);
        assert_eq!(a.columns[1].comment.as_deref(), Some("cy"));
        assert_eq!(a.indexes, vec![idx(100, "a_i1")]);
        assert_eq!(next.relations[1].columns, vec![col(1, "z")]);
    }

    #[test]
    fn a_header_patch_keeps_the_children() {
        let mut a = rel(10, "a");
        a.columns = vec![col(1, "x")];
        let prev = ns(vec![a]);
        let mut delta = PgNamespaceDelta::default();
        delta.alive.relations = [10].into();
        delta.relations = vec![RelationPatch {
            oid: 10,
            header: Some(rel(10, "renamed")),
            ..RelationPatch::default()
        }];
        let next = apply(&prev, &delta);
        assert_eq!(next.relations[0].name, "renamed");
        assert_eq!(next.relations[0].columns, vec![col(1, "x")]);
    }
}
