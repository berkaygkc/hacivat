//! Decoding the text rows of [`super::sql`]'s sections into [`super::model`] structs. The
//! column order of each decoder is the select-list order of its section.

use std::str::FromStr;

use super::IntroError;
use super::model::*;
use super::text::{parse_array, parse_bool, parse_num_array, parse_vector};

/// One text row of a named section.
pub struct Row<'a> {
    pub section: &'static str,
    pub cells: &'a [Option<String>],
}

impl<'a> Row<'a> {
    fn err(&self, i: usize, what: &str) -> IntroError {
        IntroError::Decode {
            section: self.section,
            detail: format!(
                "column {i}: expected {what}, got {:?}",
                self.cells.get(i).cloned().flatten()
            ),
        }
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
        parse_bool(self.text(i)?).ok_or_else(|| self.err(i, "t or f"))
    }

    pub fn char(&self, i: usize) -> Result<char, IntroError> {
        let s = self.text(i)?;
        let mut it = s.chars();
        match (it.next(), it.next()) {
            (Some(c), None) => Ok(c),
            _ => Err(self.err(i, "one character")),
        }
    }

    /// A `"char"` column that is empty (`attidentity = ''`) as `None`.
    pub fn opt_char(&self, i: usize) -> Result<Option<char>, IntroError> {
        match self.text(i)? {
            "" => Ok(None),
            _ => self.char(i).map(Some),
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

    /// A text array with NULL elements.
    pub fn opt_strings(&self, i: usize) -> Result<Vec<Option<String>>, IntroError> {
        parse_array(self.text(i)?).ok_or_else(|| self.err(i, "a text array"))
    }

    /// A number array (`int2[]`, `oid[]`); a NULL array is empty.
    pub fn nums<T: FromStr>(&self, i: usize) -> Result<Vec<T>, IntroError> {
        let Some(s) = self.opt(i) else {
            return Ok(Vec::new());
        };
        parse_num_array(s).ok_or_else(|| self.err(i, "a number array"))
    }

    /// An `int2vector`.
    pub fn vector<T: FromStr>(&self, i: usize) -> Result<Vec<T>, IntroError> {
        parse_vector(self.text(i)?).ok_or_else(|| self.err(i, "an int2vector"))
    }
}

/// `pg/ListNamespaces` / `pg/RetrieveNamespace`.
pub fn schema(r: &Row<'_>) -> Result<PgSchema, IntroError> {
    let name = r.string(1)?;
    Ok(PgSchema {
        oid: r.num(0)?,
        system: name.starts_with("pg_") || name == "information_schema",
        name,
        owner: r.string(2)?,
        comment: r.opt_string(3),
    })
}

/// `pg/RetrieveRelations` (children empty).
pub fn relation(r: &Row<'_>) -> Result<PgRelation, IntroError> {
    let kind = match r.char(2)? {
        'r' => RelKind::Table,
        'p' => RelKind::PartitionedTable,
        'v' => RelKind::View,
        'm' => RelKind::MaterializedView,
        'f' => RelKind::ForeignTable,
        'c' => RelKind::CompositeType,
        _ => return Err(r.err(2, "a relkind")),
    };
    let persistence = match r.char(4)? {
        'p' => Persistence::Permanent,
        'u' => Persistence::Unlogged,
        't' => Persistence::Temporary,
        _ => return Err(r.err(4, "a relpersistence")),
    };
    Ok(PgRelation {
        oid: r.num(0)?,
        name: r.string(1)?,
        kind,
        owner: r.string(3)?,
        persistence,
        comment: r.opt_string(5),
        parents: r.nums(6)?,
        partition_bound: r.opt_string(7),
        partition_key: r.opt_string(8),
        view_definition: r.opt_string(9),
        populated: r.bool(10)?,
        options: r.strings(11)?,
        columns: Vec::new(),
        indexes: Vec::new(),
        constraints: Vec::new(),
        triggers: Vec::new(),
    })
}

/// `pg/RetrieveColumns`: `(relation oid, column)`.
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

/// `pg/RetrieveIndexes`: `(relation oid, index)`.
pub fn index(r: &Row<'_>) -> Result<(Oid, PgIndex), IntroError> {
    let nkey: usize = r.num(10)?;
    let attnums: Vec<i16> = r.vector(11)?;
    let options: Vec<i16> = r.vector(12)?;
    let exprs = r.opt_strings(13)?;
    if exprs.len() != attnums.len() || nkey > attnums.len() {
        return Err(r.err(13, "one expression slot per index column"));
    }
    let elem = |k: usize| -> Result<IndexElem, IntroError> {
        match (attnums[k], &exprs[k]) {
            (0, Some(e)) => Ok(IndexElem::Expression(e.clone())),
            (0, None) => Err(r.err(13, "an expression for attnum 0")),
            (n, _) => Ok(IndexElem::Column(n)),
        }
    };
    let mut keys = Vec::with_capacity(nkey);
    for k in 0..nkey {
        let opt = options.get(k).copied().unwrap_or(0);
        keys.push(IndexKey {
            elem: elem(k)?,
            descending: opt & 1 != 0,
            nulls_first: opt & 2 != 0,
        });
    }
    let include = (nkey..attnums.len())
        .map(elem)
        .collect::<Result<Vec<_>, _>>()?;
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
            predicate: r.opt_string(14),
            definition: r.opt_string(15),
            comment: r.opt_string(16),
        },
    ))
}

fn fk_action(r: &Row<'_>, i: usize) -> Result<FkAction, IntroError> {
    Ok(match r.char(i)? {
        'a' => FkAction::NoAction,
        'r' => FkAction::Restrict,
        'c' => FkAction::Cascade,
        'n' => FkAction::SetNull,
        'd' => FkAction::SetDefault,
        _ => return Err(r.err(i, "a foreign key action")),
    })
}

/// `pg/RetrieveConstraints`: `(relation oid, constraint)`.
pub fn constraint(r: &Row<'_>) -> Result<(Oid, PgConstraint), IntroError> {
    let kind = match r.char(3)? {
        'p' => ConstraintKind::PrimaryKey,
        'u' => ConstraintKind::Unique,
        'f' => ConstraintKind::ForeignKey,
        'c' => ConstraintKind::Check,
        'x' => ConstraintKind::Exclusion,
        't' => ConstraintKind::Trigger,
        'n' => ConstraintKind::NotNull,
        _ => return Err(r.err(3, "a contype")),
    };
    let foreign = match r.opt_num::<Oid>(12)? {
        Some(table) if kind == ConstraintKind::ForeignKey => Some(ForeignKeyRef {
            table,
            schema: r.string(13)?,
            name: r.string(14)?,
            columns: r.nums(15)?,
            on_update: fk_action(r, 16)?,
            on_delete: fk_action(r, 17)?,
            match_type: r.char(18)?,
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
            columns: r.nums(10)?,
            index: r.opt_num(11)?,
            foreign,
            definition: r.string(19)?,
            comment: r.opt_string(20),
        },
    ))
}

/// `pg/RetrieveTriggers`: `(relation oid, trigger)`.
pub fn trigger(r: &Row<'_>) -> Result<(Oid, PgTrigger), IntroError> {
    let tgtype: i32 = r.num(3)?;
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
            function_schema: r.string(5)?,
            function_name: r.string(6)?,
            definition: r.string(7)?,
            constraint_trigger: r.bool(8)?,
            comment: r.opt_string(9),
        },
    ))
}

/// `pg/RetrieveSequences`.
pub fn sequence(r: &Row<'_>) -> Result<PgSequence, IntroError> {
    let owned_by = match (r.opt_num::<Oid>(10)?, r.opt_num::<i16>(11)?) {
        (Some(t), Some(c)) => Some((t, c)),
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
        comment: r.opt_string(13),
    })
}

/// `pg/RetrieveRoutines`.
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
        definition: r.opt_string(13),
        comment: r.opt_string(14),
    })
}

/// `pg/RetrieveTypes` (domain constraints empty).
pub fn user_type(r: &Row<'_>) -> Result<PgType, IntroError> {
    let kind = match r.char(2)? {
        'e' => UserTypeKind::Enum {
            labels: r.strings(10)?,
        },
        'd' => UserTypeKind::Domain {
            base: r.num(5)?,
            base_display: r.string(6)?,
            not_null: r.bool(7)?,
            default: r.opt_string(8),
            constraints: Vec::new(),
        },
        'c' => UserTypeKind::Composite { relid: r.num(9)? },
        'r' => UserTypeKind::Range {
            subtype: r.string(11)?,
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

/// `pg/RetrieveDomainConstraints`: `(type oid, constraint)`.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cells: &[Option<&str>]) -> Vec<Option<String>> {
        cells.iter().map(|c| c.map(str::to_string)).collect()
    }

    #[test]
    fn a_column_row_decodes_types_identity_and_generation() {
        let cells = row(&[
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
            Some("f"),
            None,
            Some(""),
            Some(""),
            None,
            Some("0"),
            Some("Tags"),
        ]);
        let (rel, c) = column(&Row {
            section: "pg/RetrieveColumns",
            cells: &cells,
        })
        .unwrap();
        assert_eq!(rel, 16400);
        assert_eq!(c.num, 3);
        assert_eq!(c.ty.kind, TypeKind::Array { element: 25 });
        assert_eq!(c.ty.display, "text[]");
        assert_eq!(c.identity, None);
        assert_eq!(c.generated, None);
        assert_eq!(c.comment.as_deref(), Some("Tags"));

        let mut cells = cells;
        cells[7] = Some("d".into());
        cells[8] = None;
        cells[9] = Some("23".into());
        cells[13] = Some("a".into());
        cells[14] = Some("v".into());
        let (_, c) = column(&Row {
            section: "pg/RetrieveColumns",
            cells: &cells,
        })
        .unwrap();
        assert_eq!(c.ty.kind, TypeKind::Domain { base: 23 });
        assert_eq!(c.identity, Some(Identity::Always));
        assert_eq!(c.generated, Some(Generated::Virtual));
    }

    #[test]
    fn an_index_row_splits_keys_includes_and_expressions() {
        // CREATE INDEX ON t ((a + b) DESC, c) INCLUDE (d) WHERE a > 0
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
            Some(r#"{"(a + b)",NULL,NULL}"#),
            Some("(a > 0)"),
            None,
            None,
        ]);
        let (_, i) = index(&Row {
            section: "pg/RetrieveIndexes",
            cells: &cells,
        })
        .unwrap();
        assert_eq!(
            i.keys,
            vec![
                IndexKey {
                    elem: IndexElem::Expression("(a + b)".into()),
                    descending: true,
                    nulls_first: true
                },
                IndexKey {
                    elem: IndexElem::Column(3),
                    descending: false,
                    nulls_first: false
                },
            ]
        );
        assert_eq!(i.include, vec![IndexElem::Column(4)]);
        assert_eq!(i.predicate.as_deref(), Some("(a > 0)"));
    }

    #[test]
    fn trigger_type_bits() {
        // ROW | BEFORE | INSERT | UPDATE = 1 + 2 + 4 + 16
        let cells = row(&[
            Some("1"),
            Some("2"),
            Some("tr"),
            Some("23"),
            Some("O"),
            Some("s"),
            Some("f"),
            Some("CREATE TRIGGER …"),
            Some("f"),
            None,
        ]);
        let (_, t) = trigger(&Row {
            section: "pg/RetrieveTriggers",
            cells: &cells,
        })
        .unwrap();
        assert_eq!(t.timing, TriggerTiming::Before);
        assert_eq!(t.events, vec![TriggerEvent::Insert, TriggerEvent::Update]);
        assert!(t.row_level);
    }

    #[test]
    fn a_bad_cell_names_its_section_and_column() {
        let cells = row(&[Some("x")]);
        let e = schema(&Row {
            section: "pg/ListNamespaces",
            cells: &cells,
        })
        .unwrap_err();
        match e {
            IntroError::Decode { section, detail } => {
                assert_eq!(section, "pg/ListNamespaces");
                assert!(detail.contains("column 1"), "{detail}");
            }
            other => panic!("{other:?}"),
        }
    }
}
