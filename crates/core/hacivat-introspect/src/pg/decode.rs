// ## m4-t16-pg (spike origin 5a5ab08, `src/pg/decode.rs`)
//! Decoding the rows of the `packs/pg` sections into [`super::model`] structs. Each decoder
//! reads its section's declared columns by position (the `--- returns: … ---` order, which
//! [`super::sections`] checks against the server's columns before decoding). A cell that is
//! not what the section declares is [`IntroError::Protocol`] naming the section and the
//! column, never a panic and never a silent default.

use std::str::FromStr;
use std::sync::Arc;

use super::model::*;
use super::text::{parse_array, parse_bool, parse_num_array, parse_vector};
use crate::introspector::IntroError;

/// One row of a named section.
#[derive(Debug, Clone, Copy)]
pub struct Row<'a> {
    pub section: &'static str,
    pub cells: &'a [Option<Arc<str>>],
}

impl<'a> Row<'a> {
    /// The protocol error for column `i` (the section and the column, never the value: a
    /// cell may hold user text).
    pub fn err(&self, i: usize, what: &str) -> IntroError {
        IntroError::Protocol(format!("pg/{}: column {i}: expected {what}", self.section).into())
    }

    pub fn opt(&self, i: usize) -> Option<&'a str> {
        self.cells.get(i).and_then(|c| c.as_deref())
    }

    pub fn text(&self, i: usize) -> Result<&'a str, IntroError> {
        self.opt(i).ok_or_else(|| self.err(i, "a value"))
    }

    pub fn num<T: FromStr>(&self, i: usize) -> Result<T, IntroError> {
        self.text(i)?.parse().map_err(|_| self.err(i, "a number"))
    }

    pub fn opt_num<T: FromStr>(&self, i: usize) -> Result<Option<T>, IntroError> {
        self.opt(i)
            .map(|v| v.parse().map_err(|_| self.err(i, "a number")))
            .transpose()
    }

    pub fn bool(&self, i: usize) -> Result<bool, IntroError> {
        parse_bool(self.text(i)?).ok_or_else(|| self.err(i, "a boolean"))
    }

    pub fn char(&self, i: usize) -> Result<char, IntroError> {
        let s = self.text(i)?;
        let mut it = s.chars();
        match (it.next(), it.next()) {
            (Some(c), None) => Ok(c),
            _ => Err(self.err(i, "one character")),
        }
    }

    /// A `"char"` column that is empty (`attidentity = ''`) or NULL as `None`.
    pub fn opt_char(&self, i: usize) -> Result<Option<char>, IntroError> {
        match self.opt(i) {
            None | Some("") => Ok(None),
            Some(_) => self.char(i).map(Some),
        }
    }

    pub fn string(&self, i: usize) -> Result<String, IntroError> {
        self.text(i).map(str::to_string)
    }

    pub fn opt_string(&self, i: usize) -> Option<String> {
        self.opt(i).map(str::to_string)
    }

    /// A text array whose elements are never NULL; a NULL array is empty.
    pub fn strings(&self, i: usize) -> Result<Vec<String>, IntroError> {
        let Some(s) = self.opt(i) else {
            return Ok(Vec::new());
        };
        parse_array(s)
            .and_then(|v| v.into_iter().collect::<Option<Vec<_>>>())
            .ok_or_else(|| self.err(i, "a text array"))
    }

    /// A text array with NULL elements; a NULL array is empty.
    pub fn opt_strings(&self, i: usize) -> Result<Vec<Option<String>>, IntroError> {
        let Some(s) = self.opt(i) else {
            return Ok(Vec::new());
        };
        parse_array(s).ok_or_else(|| self.err(i, "a text array"))
    }

    /// A number array (`int2[]`, `oid[]`); a NULL array is empty.
    pub fn nums<T: FromStr>(&self, i: usize) -> Result<Vec<T>, IntroError> {
        let Some(s) = self.opt(i) else {
            return Ok(Vec::new());
        };
        parse_num_array(s).ok_or_else(|| self.err(i, "a number array"))
    }

    /// An `int2vector` / `oidvector`.
    pub fn vector<T: FromStr>(&self, i: usize) -> Result<Vec<T>, IntroError> {
        parse_vector(self.text(i)?).ok_or_else(|| self.err(i, "an int2vector"))
    }
}

/// What the `Identity` probe read (the first statement of every pass).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityRow {
    pub version: u32,
    pub in_recovery: bool,
    /// `pg_postmaster_start_time()` as epoch seconds (exact text).
    pub start_epoch: String,
    pub db_oid: Oid,
    pub db_name: String,
    pub system_identifier: Option<String>,
    pub timeline: Option<u32>,
    pub snap_xmin: u64,
    pub snap_xmax: u64,
    /// The schema's oid; `None` when no schema of that name exists.
    pub ns_oid: Option<Oid>,
    pub version_text: String,
    pub encoding: String,
    pub standard_conforming_strings: Option<bool>,
    pub db_collation: Option<String>,
}

impl IdentityRow {
    /// The server facts this probe read (Task 16: encoding, `standard_conforming_strings`,
    /// recovery, the database's default collation).
    pub fn server_facts(&self) -> hacivat_meta::snapshot::ServerFacts {
        let mut ident = hacivat_meta::name::IdentCtx::new(hacivat_meta::name::DialectId::Postgres);
        ident.encoding = Some(Arc::from(self.encoding.as_str()));
        let mut f =
            hacivat_meta::snapshot::ServerFacts::new(hacivat_proto::DbmsKind::Postgres, ident);
        f.version = Arc::from(self.version_text.as_str());
        f.version_num = self.version;
        f.standard_conforming_strings = self.standard_conforming_strings;
        f.in_recovery = Some(self.in_recovery);
        if let Some(c) = &self.db_collation {
            f.default_collation
                .insert(Arc::from(self.db_name.as_str()), Arc::from(c.as_str()));
        }
        f
    }
}

/// `Identity`.
pub fn identity(r: &Row<'_>) -> Result<IdentityRow, IntroError> {
    Ok(IdentityRow {
        version: r.num(0)?,
        in_recovery: r.bool(1)?,
        start_epoch: r.string(2)?,
        db_oid: r.num(3)?,
        db_name: r.string(4)?,
        system_identifier: r.opt_string(5),
        timeline: r.opt_num(6)?,
        snap_xmin: r.num(7)?,
        snap_xmax: r.num(8)?,
        ns_oid: r.opt_num(9)?,
        version_text: r.string(10)?,
        encoding: r.string(11)?,
        standard_conforming_strings: r.opt(12).map(|v| v == "on"),
        db_collation: r.opt_string(13),
    })
}

/// `ListNamespaces`: the schema and the database name.
pub fn namespace_listed(r: &Row<'_>) -> Result<(PgSchema, String), IntroError> {
    Ok((schema(r)?, r.string(4)?))
}

/// `RetrieveNamespace` (and the first four columns of `ListNamespaces`).
pub fn schema(r: &Row<'_>) -> Result<PgSchema, IntroError> {
    Ok(PgSchema {
        oid: r.num(0)?,
        name: r.string(1)?,
        owner: r.string(2)?,
        comment: r.opt_string(3),
    })
}

/// One `ListExistent` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Existent {
    pub tag: String,
    pub oid: Oid,
    pub parent: Option<String>,
    pub name: String,
    pub sub: Option<String>,
}

/// `ListExistent`.
pub fn existent(r: &Row<'_>) -> Result<Existent, IntroError> {
    Ok(Existent {
        tag: r.string(0)?,
        oid: r.num(1)?,
        parent: r.opt_string(2),
        name: r.string(3)?,
        sub: r.opt_string(4),
    })
}

/// `ListExistentComments`: `(class tag, objoid, objsubid)`.
pub fn comment_key(r: &Row<'_>) -> Result<(char, Oid, i32), IntroError> {
    Ok((r.char(0)?, r.num(1)?, r.num(2)?))
}

/// `ChangedParents` / `ChangedDependents`: `(facet, oid)`.
pub fn facet(r: &Row<'_>) -> Result<(String, Oid), IntroError> {
    Ok((r.string(0)?, r.num(1)?))
}

/// `RetrieveRelations`.
pub fn relation(r: &Row<'_>) -> Result<PgRelation, IntroError> {
    let kind = RelKind::from_relkind(r.char(2)?).ok_or_else(|| r.err(2, "a relkind"))?;
    let persistence = match r.char(4)? {
        'p' => Persistence::Permanent,
        'u' => Persistence::Unlogged,
        't' => Persistence::Temporary,
        _ => return Err(r.err(4, "a relpersistence")),
    };
    let partition_of = match r.opt_num::<Oid>(6)? {
        Some(oid) => Some((oid, r.string(7)?, r.string(8)?)),
        None => None,
    };
    let reltuples: f64 = r.num(15)?;
    Ok(PgRelation {
        oid: r.num(0)?,
        name: r.string(1)?,
        kind,
        owner: r.string(3)?,
        persistence,
        comment: r.opt_string(5),
        partition_of,
        partition_bound: r.opt_string(9),
        partition_key: r.opt_string(10),
        view_definition: r.opt_string(11),
        populated: r.bool(12)?,
        options: r.strings(13)?,
        row_security: r.bool(14)?,
        row_estimate: row_estimate(reltuples),
        access_method: r.opt_string(16),
        tablespace: r.opt_string(17),
    })
}

/// `reltuples` as an estimate: negative (`-1`, never analyzed or vacuumed, PostgreSQL 14+)
/// is unknown.
pub fn row_estimate(reltuples: f64) -> Option<u64> {
    if reltuples.is_finite() && reltuples >= 0.0 {
        // A float row count; the estimate is whole rows.
        Some(reltuples.round() as u64)
    } else {
        None
    }
}

/// `RetrieveColumns`: `(relation oid, column)`.
pub fn column(r: &Row<'_>) -> Result<(Oid, PgColumn), IntroError> {
    let typtype = r.char(7)?;
    let element: Option<Oid> = r.opt_num(8)?;
    let base: Option<Oid> = r.opt_num(9)?;
    let kind = match (typtype, element, base) {
        ('d', _, Some(base)) => TypeKind::Domain { base },
        (_, Some(element), _) => TypeKind::Array { element },
        ('b', _, _) => TypeKind::Base,
        ('e', _, _) => TypeKind::Enum,
        ('c', _, _) => TypeKind::Composite,
        ('r', _, _) => TypeKind::Range,
        ('m', _, _) => TypeKind::Multirange,
        ('p', _, _) => TypeKind::Pseudo,
        _ => return Err(r.err(7, "a typtype")),
    };
    let identity = match r.opt_char(13)? {
        None => None,
        Some('a') => Some(Identity::Always),
        Some('d') => Some(Identity::ByDefault),
        Some(_) => return Err(r.err(13, "an attidentity")),
    };
    let generated = match r.opt_char(14)? {
        None => None,
        Some('s') => Some(Generated::Stored),
        Some('v') => Some(Generated::Virtual),
        Some(_) => return Err(r.err(14, "an attgenerated")),
    };
    Ok((
        r.num(0)?,
        PgColumn {
            num: r.num(1)?,
            name: r.string(2)?,
            ty: PgTypeRef {
                oid: r.num(3)?,
                display: r.string(4)?,
                schema: r.string(5)?,
                name: r.string(6)?,
                kind,
            },
            array_dims: r.num(10)?,
            not_null: r.bool(11)?,
            default: r.opt_string(12),
            identity,
            generated,
            collation: r.opt_string(15),
            inherited: r.num::<i32>(16)? > 0,
            comment: r.opt_string(17),
        },
    ))
}

/// `RetrieveIndexes`: `(relation oid, index)`.
pub fn index(r: &Row<'_>) -> Result<(Oid, PgIndex), IntroError> {
    let nkey: usize = r.num(10)?;
    let attnums: Vec<i16> = r.vector(11)?;
    let options: Vec<i16> = r.vector(12)?;
    let names = r.opt_strings(13)?;
    let exprs = r.opt_strings(14)?;
    let opclasses = r.opt_strings(15)?;
    let collations = r.opt_strings(16)?;
    if exprs.len() != attnums.len() || names.len() != attnums.len() || nkey > attnums.len() {
        return Err(r.err(14, "one slot per index column"));
    }
    let mut keys = Vec::with_capacity(nkey);
    for k in 0..nkey {
        let attnum = attnums[k];
        let expression = exprs[k].clone();
        if attnum == 0 && expression.is_none() {
            return Err(r.err(14, "an expression for attnum 0"));
        }
        let opt = options.get(k).copied().unwrap_or(0);
        keys.push(PgIndexPart {
            attnum,
            name: if attnum == 0 { None } else { names[k].clone() },
            expression: if attnum == 0 { expression } else { None },
            descending: opt & 1 != 0,
            nulls_first: opt & 2 != 0,
            opclass: opclasses.get(k).cloned().flatten(),
            collation: collations.get(k).cloned().flatten(),
        });
    }
    let include = (nkey..attnums.len())
        .map(|k| (attnums[k], names[k].clone()))
        .collect();
    Ok((
        r.num(0)?,
        PgIndex {
            oid: r.num(1)?,
            name: r.string(2)?,
            method: r.string(3)?,
            unique: r.bool(4)?,
            primary: r.bool(5)?,
            exclusion: r.bool(6)?,
            immediate: r.bool(7)?,
            valid: r.bool(8)?,
            nulls_not_distinct: r.bool(9)?,
            keys,
            include,
            predicate: r.opt_string(17),
            definition: r.opt_string(18),
            comment: r.opt_string(19),
        },
    ))
}

/// `(attnum, name)` pairs from an `int2[]` and its parallel name array.
fn key_pairs(
    r: &Row<'_>,
    nums: usize,
    names: usize,
) -> Result<Vec<(i16, Option<String>)>, IntroError> {
    let n: Vec<i16> = r.nums(nums)?;
    let mut m = r.opt_strings(names)?;
    if m.is_empty() {
        m = vec![None; n.len()];
    }
    if m.len() != n.len() {
        return Err(r.err(names, "one name per key column"));
    }
    Ok(n.into_iter().zip(m).collect())
}

/// `RetrieveConstraints`: `(relation oid, constraint)`.
pub fn constraint(r: &Row<'_>) -> Result<(Oid, PgConstraint), IntroError> {
    let kind = ConstraintKind::from_contype(r.char(3)?).ok_or_else(|| r.err(3, "a contype"))?;
    let index = r
        .opt_num::<Oid>(13)?
        .map(|oid| (oid, r.opt_string(14).unwrap_or_default()));
    let foreign = match r.opt_num::<Oid>(15)? {
        Some(table) if kind == ConstraintKind::ForeignKey => Some(ForeignKeyRef {
            table,
            schema: r.string(16)?,
            name: r.string(17)?,
            columns: key_pairs(r, 18, 19)?,
            on_update: r.char(20)?,
            on_delete: r.char(21)?,
            match_type: r.char(22)?,
        }),
        _ => None,
    };
    Ok((
        r.num(0)?,
        PgConstraint {
            oid: r.num(1)?,
            name: r.string(2)?,
            kind,
            deferrable: r.bool(4)?,
            deferred: r.bool(5)?,
            validated: r.bool(6)?,
            enforced: r.bool(7)?,
            local: r.bool(8)?,
            no_inherit: r.bool(9)?,
            parent: r.opt_num(10)?,
            columns: key_pairs(r, 11, 12)?,
            index,
            foreign,
            definition: r.string(23)?,
            check_expr: r.opt_string(24),
            exclusion_ops: r.strings(25)?,
            comment: r.opt_string(26),
        },
    ))
}

/// `RetrieveTriggers`: `(relation oid, trigger)`. `tgtype` bits (`pg_trigger.h`): 1 row,
/// 2 before, 4 insert, 8 delete, 16 update, 32 truncate, 64 instead of.
pub fn trigger(r: &Row<'_>) -> Result<(Oid, PgTrigger), IntroError> {
    let tgtype: i16 = r.num(3)?;
    let timing = if tgtype & 64 != 0 {
        TriggerTiming::InsteadOf
    } else if tgtype & 2 != 0 {
        TriggerTiming::Before
    } else {
        TriggerTiming::After
    };
    let mut events = Vec::new();
    for (bit, e) in [
        (4, TriggerEvent::Insert),
        (16, TriggerEvent::Update),
        (8, TriggerEvent::Delete),
        (32, TriggerEvent::Truncate),
    ] {
        if tgtype & bit != 0 {
            events.push(e);
        }
    }
    events.sort();
    Ok((
        r.num(0)?,
        PgTrigger {
            oid: r.num(1)?,
            name: r.string(2)?,
            timing,
            events,
            row_level: tgtype & 1 != 0,
            enabled: r.char(4)?,
            function: (r.num(5)?, r.string(6)?, r.string(7)?),
            definition: r.opt_string(8),
            constraint_trigger: r.bool(9)?,
            comment: r.opt_string(10),
        },
    ))
}

/// `RetrieveSequences`.
pub fn sequence(r: &Row<'_>) -> Result<PgSequence, IntroError> {
    let owned_by = match (r.opt_num::<Oid>(10)?, r.opt_num::<i32>(11)?) {
        (Some(t), Some(c)) if c > 0 => Some((
            t,
            r.opt_string(13).unwrap_or_default(),
            r.opt_string(14).unwrap_or_default(),
            r.opt_string(15).unwrap_or_default(),
        )),
        _ => None,
    };
    Ok(PgSequence {
        oid: r.num(0)?,
        name: r.string(1)?,
        owner: r.string(2)?,
        data_type: r.string(3)?,
        start: r.num(4)?,
        increment: r.num(5)?,
        min: r.num(6)?,
        max: r.num(7)?,
        cache: r.num(8)?,
        cycle: r.bool(9)?,
        owned_by,
        identity: r.opt(12) == Some("i"),
        comment: r.opt_string(16),
    })
}

/// `RetrieveRoutines`.
pub fn routine(r: &Row<'_>) -> Result<PgRoutine, IntroError> {
    let kind = match r.char(2)? {
        'f' => RoutineKind::Function,
        'p' => RoutineKind::Procedure,
        'a' => RoutineKind::Aggregate,
        'w' => RoutineKind::Window,
        _ => return Err(r.err(2, "a prokind")),
    };
    let volatility = match r.char(6)? {
        'i' => Volatility::Immutable,
        's' => Volatility::Stable,
        'v' => Volatility::Volatile,
        _ => return Err(r.err(6, "a provolatile")),
    };
    let types = r.strings(15)?;
    let names = r.opt_strings(13)?;
    let modes: Vec<char> = match r.opt(14) {
        None => vec!['i'; types.len()],
        Some(_) => r
            .strings(14)?
            .iter()
            .map(|m| m.chars().next().unwrap_or('i'))
            .collect(),
    };
    if modes.len() != types.len() || (!names.is_empty() && names.len() != types.len()) {
        return Err(r.err(15, "one type per argument"));
    }
    let defaults: usize = r.num::<i16>(17)?.max(0).unsigned_abs().into();
    // The last `pronargdefaults` input arguments have defaults.
    let inputs: Vec<usize> = modes
        .iter()
        .enumerate()
        .filter(|(_, m)| matches!(m, 'i' | 'b' | 'v'))
        .map(|(k, _)| k)
        .collect();
    let first_default = inputs.len().saturating_sub(defaults);
    let args = types
        .into_iter()
        .enumerate()
        .map(|(k, type_text)| PgArg {
            name: names.get(k).cloned().flatten().filter(|n| !n.is_empty()),
            type_text,
            mode: modes[k],
            has_default: inputs
                .iter()
                .position(|&i| i == k)
                .is_some_and(|p| p >= first_default),
        })
        .collect();
    Ok(PgRoutine {
        oid: r.num(0)?,
        name: r.string(1)?,
        kind,
        identity_args: r.string(3)?,
        result: r.opt_string(4),
        language: r.string(5)?,
        volatility,
        strict: r.bool(7)?,
        security_definer: r.bool(8)?,
        returns_set: r.bool(9)?,
        parallel: r.char(10)?,
        owner: r.string(11)?,
        from_extension: r.bool(12)?,
        args,
        definition: r.opt_string(18),
        comment: r.opt_string(19),
    })
}

/// `RetrieveTypes` (domain constraints and composite attributes empty: their own sections).
pub fn user_type(r: &Row<'_>) -> Result<PgType, IntroError> {
    let kind = match r.char(2)? {
        'e' => UserTypeKind::Enum {
            labels: r.strings(9)?,
        },
        'd' => UserTypeKind::Domain {
            base_display: r.string(5)?,
            not_null: r.bool(6)?,
            default: r.opt_string(7),
            constraints: Vec::new(),
        },
        'c' => UserTypeKind::Composite {
            relid: r.num(8)?,
            attrs: Vec::new(),
        },
        'r' => UserTypeKind::Range {
            subtype: r.string(10)?,
        },
        'b' => UserTypeKind::Base,
        _ => return Err(r.err(2, "a typtype")),
    };
    Ok(PgType {
        oid: r.num(0)?,
        name: r.string(1)?,
        kind,
        owner: r.string(3)?,
        comment: r.opt_string(4),
    })
}

/// `RetrieveTypeAttributes`: `(type oid, (attnum, name, type text))`.
pub fn type_attribute(r: &Row<'_>) -> Result<(Oid, (i16, String, String)), IntroError> {
    Ok((r.num(0)?, (r.num(1)?, r.string(2)?, r.string(3)?)))
}

/// `RetrieveDomainConstraints`: `(type oid, constraint)`.
pub fn domain_constraint(r: &Row<'_>) -> Result<(Oid, PgDomainConstraint), IntroError> {
    Ok((
        r.num(0)?,
        PgDomainConstraint {
            oid: r.num(1)?,
            name: r.string(2)?,
            kind: r.char(3)?,
            definition: r.string(4)?,
            validated: r.bool(5)?,
        },
    ))
}

/// `Comments`: `(objoid, objsubid, description)`.
pub fn comment(r: &Row<'_>) -> Result<(Oid, i32, String), IntroError> {
    Ok((r.num(0)?, r.num(1)?, r.string(2)?))
}

/// The bare expression of a `CHECK (expr)` definition (`pg_get_constraintdef`), without a
/// trailing `NOT VALID`; `None` when the text is not that shape.
pub fn check_body(def: &str) -> Option<&str> {
    let def = def.strip_suffix(" NOT VALID").unwrap_or(def);
    let inner = def.strip_prefix("CHECK (")?.strip_suffix(')')?;
    Some(inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cells: &[Option<&str>]) -> Vec<Option<Arc<str>>> {
        cells.iter().map(|c| c.map(Arc::from)).collect()
    }

    #[test]
    fn a_column_row_decodes_types_identity_and_generation() {
        let mut cells = row(&[
            Some("16400"),
            Some("3"),
            Some("tags"),
            Some("1009"),
            Some("text[]"),
            Some("pg_catalog"),
            Some("_text"),
            Some("b"),
            Some("25"),
            None,
            Some("1"),
            Some("false"),
            None,
            Some(""),
            Some(""),
            None,
            Some("0"),
            Some("Tags"),
        ]);
        let (rel, c) = column(&Row {
            section: "RetrieveColumns",
            cells: &cells,
        })
        .unwrap();
        assert_eq!(rel, 16400);
        assert_eq!(c.num, 3);
        assert_eq!(c.ty.kind, TypeKind::Array { element: 25 });
        assert_eq!(c.ty.display, "text[]");
        assert_eq!((c.identity, c.generated), (None, None));
        assert_eq!(c.comment.as_deref(), Some("Tags"));
        cells[7] = Some(Arc::from("d"));
        cells[8] = None;
        cells[9] = Some(Arc::from("23"));
        cells[13] = Some(Arc::from("a"));
        cells[14] = Some(Arc::from("v"));
        let (_, c) = column(&Row {
            section: "RetrieveColumns",
            cells: &cells,
        })
        .unwrap();
        assert_eq!(c.ty.kind, TypeKind::Domain { base: 23 });
        assert_eq!(c.identity, Some(Identity::Always));
        assert_eq!(c.generated, Some(Generated::Virtual));
    }

    #[test]
    fn an_index_row_splits_keys_includes_expressions_and_options() {
        // CREATE INDEX ON t ((a + b) DESC, c text_pattern_ops) INCLUDE (d) WHERE a > 0
        let cells = row(&[
            Some("16400"),
            Some("16500"),
            Some("t_expr_idx"),
            Some("btree"),
            Some("f"),
            Some("f"),
            Some("f"),
            Some("t"),
            Some("t"),
            Some("f"),
            Some("2"),
            Some("0 3 4"),
            Some("3 0"),
            Some("{NULL,c,d}"),
            Some(r#"{"(a + b)",NULL,NULL}"#),
            Some("{NULL,text_pattern_ops}"),
            Some("{NULL,NULL}"),
            Some("(a > 0)"),
            None,
            None,
        ]);
        let (_, i) = index(&Row {
            section: "RetrieveIndexes",
            cells: &cells,
        })
        .unwrap();
        assert_eq!(i.keys.len(), 2);
        assert_eq!(i.keys[0].expression.as_deref(), Some("(a + b)"));
        assert!(i.keys[0].descending && i.keys[0].nulls_first);
        assert_eq!(i.keys[1].attnum, 3);
        assert_eq!(i.keys[1].name.as_deref(), Some("c"));
        assert_eq!(i.keys[1].opclass.as_deref(), Some("text_pattern_ops"));
        assert_eq!(i.include, vec![(4, Some("d".to_string()))]);
        assert_eq!(i.predicate.as_deref(), Some("(a > 0)"));
    }

    #[test]
    fn trigger_type_bits_and_enabled() {
        // ROW | BEFORE | INSERT | UPDATE = 1 + 2 + 4 + 16
        let cells = row(&[
            Some("1"),
            Some("2"),
            Some("tr"),
            Some("23"),
            Some("D"),
            Some("9"),
            Some("s"),
            Some("f"),
            None,
            Some("f"),
            None,
        ]);
        let (_, t) = trigger(&Row {
            section: "RetrieveTriggers",
            cells: &cells,
        })
        .unwrap();
        assert_eq!(t.timing, TriggerTiming::Before);
        assert_eq!(t.events, vec![TriggerEvent::Insert, TriggerEvent::Update]);
        assert!(t.row_level);
        assert_eq!(t.enabled, 'D');
    }

    #[test]
    fn routine_arguments_and_defaults() {
        // f(a int, b text DEFAULT 'x', OUT c int)
        let cells = row(&[
            Some("7"),
            Some("f"),
            Some("f"),
            Some("a integer, b text"),
            Some("integer"),
            Some("sql"),
            Some("v"),
            Some("f"),
            Some("f"),
            Some("f"),
            Some("u"),
            Some("hv_test"),
            Some("f"),
            Some("{a,b,c}"),
            Some("{i,i,o}"),
            Some("{integer,text,integer}"),
            Some("2"),
            Some("1"),
            None,
            None,
        ]);
        let p = routine(&Row {
            section: "RetrieveRoutines",
            cells: &cells,
        })
        .unwrap();
        assert_eq!(p.args.len(), 3);
        assert!(!p.args[0].has_default && p.args[1].has_default && !p.args[2].has_default);
        assert_eq!(p.args[2].mode, 'o');
    }

    #[test]
    fn a_bad_cell_names_its_section_and_column_not_its_value() {
        let cells = row(&[Some("secret-ish")]);
        let e = schema(&Row {
            section: "ListNamespaces",
            cells: &cells,
        })
        .unwrap_err();
        let IntroError::Protocol(m) = e else {
            panic!("{e:?}")
        };
        assert!(
            m.contains("ListNamespaces") && m.contains("column 0"),
            "{m}"
        );
        assert!(!m.contains("secret-ish"));
    }

    #[test]
    fn check_bodies_and_estimates() {
        assert_eq!(check_body("CHECK (VALUE > 0)"), Some("VALUE > 0"));
        assert_eq!(check_body("CHECK ((a > 0)) NOT VALID"), Some("(a > 0)"));
        assert_eq!(check_body("NOT NULL VALUE"), None);
        assert_eq!(row_estimate(-1.0), None);
        assert_eq!(row_estimate(0.0), Some(0));
        assert_eq!(row_estimate(1234.0), Some(1234));
    }
}
