//! M4 Task 7: constraint placement (alg/03 HV-DDL-09) as a pure function.

use hacivat_ddl::ir::{ConstraintBody, ConstraintDef, Deferrable, QualifiedName};
use hacivat_ddl::options::Placement;
use hacivat_ddl::produce::placement::{
    PlacePlan, Placed, Position, SQLITE_PLACEMENT_FORCED, place,
};
use hacivat_meta::name::Name;
use hacivat_meta::props::{FkAction, FkMatch};
use smallvec::SmallVec;
use sql_dialect::DialectId;
use sql_ident::ident::Ident;

fn n(s: &str) -> Name {
    Name::from(s)
}

fn qn(parts: &[&str]) -> QualifiedName {
    QualifiedName {
        parts: parts
            .iter()
            .map(|p| Ident {
                text: n(p),
                quoted: false,
            })
            .collect::<SmallVec<[Ident; 3]>>(),
    }
}

fn c(name: Option<&str>, body: ConstraintBody) -> ConstraintDef {
    ConstraintDef {
        name: name.map(n),
        body,
        deferrable: Deferrable::No,
        not_valid: false,
    }
}

fn pk(cols: &[&str]) -> ConstraintDef {
    c(
        None,
        ConstraintBody::PrimaryKey {
            columns: cols.iter().map(|s| n(s)).collect(),
        },
    )
}

fn unique(name: &str, col: &str) -> ConstraintDef {
    c(
        Some(name),
        ConstraintBody::Unique {
            columns: vec![n(col)],
            nulls_not_distinct: false,
        },
    )
}

/// `b_id REFERENCES <target> (id)`.
fn fk(target: &[&str]) -> ConstraintDef {
    c(
        Some("fk_b"),
        ConstraintBody::ForeignKey {
            columns: vec![n("b_id")],
            target: qn(target),
            target_columns: vec![n("id")],
            on_update: FkAction::NoAction,
            on_delete: FkAction::NoAction,
            match_kind: FkMatch::Simple,
        },
    )
}

fn check() -> ConstraintDef {
    c(Some("ck"), ConstraintBody::Check { expr: n("a > 0") })
}

fn one(k: ConstraintDef, mode: Placement, d: DialectId, plan: &PlacePlan<'_>) -> Placed {
    let mut v = place(std::slice::from_ref(&k), mode, d, plan);
    assert_eq!(v.len(), 1);
    v.remove(0)
}

fn at(k: ConstraintDef, mode: Placement, d: DialectId) -> Position {
    one(k, mode, d, &PlacePlan::default()).position
}

#[test]
fn modes_place_as_asked_where_the_dialect_allows() {
    let pg = DialectId::Postgres;
    assert_eq!(
        at(pk(&["id"]), Placement::InsideColumn, pg),
        Position::InColumn(n("id"))
    );
    assert_eq!(
        at(pk(&["id"]), Placement::InsideTable, pg),
        Position::InTable
    );
    assert_eq!(at(pk(&["id"]), Placement::AfterTable, pg), Position::After);
    // One placement per constraint, in order.
    let all = place(
        &[pk(&["id"]), check(), fk(&["app", "b"])],
        Placement::InsideColumn,
        pg,
        &PlacePlan::default(),
    );
    let positions: Vec<Position> = all.into_iter().map(|p| p.position).collect();
    assert_eq!(
        positions,
        [
            Position::InColumn(n("id")),
            Position::InTable,
            Position::InColumn(n("b_id"))
        ]
    );
}

#[test]
fn inside_column_applies_to_single_column_keys_only() {
    let pg = DialectId::Postgres;
    // A multi-column key and a check (its columns are unknown) stay table elements.
    assert_eq!(
        at(pk(&["a", "b"]), Placement::InsideColumn, pg),
        Position::InTable
    );
    assert_eq!(at(check(), Placement::InsideColumn, pg), Position::InTable);
    let excl = c(
        None,
        ConstraintBody::Exclusion {
            raw: n("USING gist (room WITH &&)"),
        },
    );
    assert_eq!(at(excl, Placement::InsideColumn, pg), Position::InTable);
}

#[test]
fn mysql_fk_is_never_inline() {
    for d in [DialectId::MySql, DialectId::MariaDb] {
        assert_eq!(
            at(fk(&["shop", "b"]), Placement::InsideColumn, d),
            Position::InTable
        );
        // A unique key goes inline only when its name is the column's (MySQL names an inline
        // key after its column).
        assert_eq!(
            at(unique("code", "code"), Placement::InsideColumn, d),
            Position::InColumn(n("code"))
        );
        assert_eq!(
            at(unique("code_uq", "code"), Placement::InsideColumn, d),
            Position::InTable
        );
    }
}

#[test]
fn fk_cycle_is_postponed_after_tables() {
    // `a.b_id → b` where `b` is created later in the script (or `a ↔ b` is a cycle).
    let later = [qn(&["app", "b"])];
    let plan = PlacePlan {
        later: &later,
        auto_column: None,
    };
    for d in [DialectId::Postgres, DialectId::MySql, DialectId::MariaDb] {
        for mode in [
            Placement::InsideColumn,
            Placement::InsideTable,
            Placement::AfterTable,
        ] {
            let p = one(fk(&["app", "b"]), mode, d, &plan);
            assert_eq!(p.position, Position::After, "{d:?} {mode:?}");
            assert_eq!(p.notice, None);
        }
        // A reference to a table already created stays where the mode puts it.
        assert_eq!(
            one(fk(&["app", "c"]), Placement::InsideTable, d, &plan).position,
            Position::InTable
        );
    }
}

#[test]
fn sqlite_constraints_inside_table_forced_with_notice() {
    let later = [qn(&["b"])];
    let plan = PlacePlan {
        later: &later,
        auto_column: None,
    };
    // A cycle (or a later table) and the AfterTable mode both stay inside, with the notice.
    let p = one(fk(&["b"]), Placement::InsideTable, DialectId::Sqlite, &plan);
    assert_eq!(p.position, Position::InTable);
    assert_eq!(p.notice.map(|n| n.id), Some(SQLITE_PLACEMENT_FORCED));
    assert_eq!(SQLITE_PLACEMENT_FORCED, "ddl-sqlite-placement-forced");
    let p = one(
        pk(&["id"]),
        Placement::AfterTable,
        DialectId::Sqlite,
        &PlacePlan::default(),
    );
    assert_eq!(p.position, Position::InTable);
    assert_eq!(p.notice.map(|n| n.id), Some(SQLITE_PLACEMENT_FORCED));
    // Nothing forced, no notice.
    let p = one(
        pk(&["id"]),
        Placement::InsideTable,
        DialectId::Sqlite,
        &PlacePlan::default(),
    );
    assert_eq!((p.position, p.notice), (Position::InTable, None));
}

#[test]
fn auto_increment_keys_stay_with_their_table() {
    let id = n("id");
    let plan = PlacePlan {
        later: &[],
        auto_column: Some(&id),
    };
    // SQLite: `INTEGER PRIMARY KEY AUTOINCREMENT` lives in its column, without a notice.
    let p = one(pk(&["id"]), Placement::AfterTable, DialectId::Sqlite, &plan);
    assert_eq!((p.position, p.notice), (Position::InColumn(n("id")), None));
    // MySQL refuses a table whose AUTO_INCREMENT column has no key at CREATE TABLE.
    assert_eq!(
        one(pk(&["id"]), Placement::AfterTable, DialectId::MySql, &plan).position,
        Position::InTable
    );
}

#[test]
fn pg_not_valid_goes_after_the_table() {
    let mut k = check();
    k.not_valid = true;
    assert_eq!(
        at(k, Placement::InsideTable, DialectId::Postgres),
        Position::After
    );
}
