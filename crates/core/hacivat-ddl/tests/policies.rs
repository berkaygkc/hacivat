//! Quoting, constraint placement (HV-DDL-09), schema qualification (HV-DDL-10) and option
//! applicability (HV-DDL-11) as pure functions.

use hacivat_ddl::NoticeCode;
use hacivat_ddl::ir::{ConstraintBody, Position, QName};
use hacivat_ddl::options::{
    Category, OptionId, SPECS, applicable, if_not_exists_supported, or_replace_supported, spec,
};
use hacivat_ddl::produce::placement::{PlaceInput, Placed, place};
use hacivat_ddl::produce::qualify::QualifyPlan;
use hacivat_ddl::quote::{
    LiteralCtx, NulInText, QuoteCtx, needs_quoting, quote, quote_qualified, string_literal,
};
use hacivat_ddl::{DdlTarget, Placement, Qualify};
use hacivat_meta::props::{FkAction, FkMatch};
use hacivat_meta::{Name, ObjectKind};
use sql_dialect::{DialectId, ServerVersion};

fn n(s: &str) -> Name {
    Name::from(s)
}

fn q(d: DialectId) -> QuoteCtx {
    QuoteCtx::new(d, None)
}

// ---- quoting ------------------------------------------------------------------------------

#[test]
fn pg_quotes_like_the_servers_quote_identifier() {
    let pg = q(DialectId::Postgres);
    for bare in ["users", "_x", "a1", "t_2", "name", "status"] {
        assert!(!needs_quoting(pg, bare), "{bare}");
    }
    for quoted in [
        "Users",
        "MÜŞTERİ",
        "müşteri",
        "ılık",
        "1a",
        "a-b",
        "a b",
        "",
        "a$",
        "order",
        "select",
        "user",
        "between",
        "int",
    ] {
        assert!(needs_quoting(pg, quoted), "{quoted}");
    }
    assert_eq!(quote(pg, "MÜŞTERİ"), "\"MÜŞTERİ\"");
    assert_eq!(quote(pg, "say \"hi\""), "\"say \"\"hi\"\"\"");
}

#[test]
fn mysql_quotes_reserved_words_only_by_version() {
    let my = q(DialectId::MySql);
    for bare in ["Users", "CamelCase", "status", "name", "comment", "_x"] {
        assert!(!needs_quoting(my, bare), "{bare}");
    }
    for quoted in ["order", "select", "1st", "a-b", "Müşteri", "", "$x"] {
        assert!(needs_quoting(my, quoted), "{quoted}");
    }
    assert_eq!(quote(my, "we`ird"), "`we``ird`");
    // MariaDB reports no reserved flag: every keyword it lists is quoted.
    assert!(needs_quoting(q(DialectId::MariaDb), "status"));
}

#[test]
fn sqlite_quotes_keywords_and_non_identifiers() {
    let s = q(DialectId::Sqlite);
    assert!(!needs_quoting(s, "Notes"));
    assert!(needs_quoting(s, "order"));
    assert!(needs_quoting(s, "a.b"));
    assert_eq!(quote_qualified(s, &["aux", "order"]), "aux.\"order\"");
}

#[test]
fn quoting_never_depends_on_turkish_casing() {
    // `i` and `I` fold ASCII-only: `INDEX` is the keyword whatever the process locale, and the
    // dotless/dotted letters are never letters of a keyword.
    let my = q(DialectId::MySql);
    assert!(needs_quoting(my, "INDEX"));
    assert!(needs_quoting(my, "ındex"));
    assert!(needs_quoting(my, "İNDEX"));
}

#[test]
fn string_literals_per_dialect() {
    let lit = |d, scs, nbe| LiteralCtx {
        dialect: d,
        pg_standard_conforming_strings: scs,
        mysql_no_backslash_escapes: nbe,
    };
    assert_eq!(
        string_literal(lit(DialectId::Postgres, true, false), "a'b\\c").unwrap(),
        "'a''b\\c'"
    );
    assert_eq!(
        string_literal(lit(DialectId::Postgres, false, false), "a'b\\c").unwrap(),
        "E'a''b\\\\c'"
    );
    assert_eq!(
        string_literal(lit(DialectId::Postgres, false, false), "plain").unwrap(),
        "'plain'"
    );
    assert_eq!(
        string_literal(lit(DialectId::MySql, true, false), "a'b\\c").unwrap(),
        "'a''b\\\\c'"
    );
    assert_eq!(
        string_literal(lit(DialectId::MySql, true, true), "a'b\\c").unwrap(),
        "'a''b\\c'"
    );
    assert_eq!(
        string_literal(lit(DialectId::Sqlite, true, false), "x\n-- y").unwrap(),
        "'x\n-- y'"
    );
    assert_eq!(
        string_literal(lit(DialectId::Sqlite, true, false), "a\0b"),
        Err(NulInText)
    );
}

// ---- placement ----------------------------------------------------------------------------

fn pk_body(cols: &[&str]) -> ConstraintBody {
    ConstraintBody::PrimaryKey {
        columns: cols.iter().map(|c| n(c)).collect(),
    }
}

fn fk_body() -> ConstraintBody {
    ConstraintBody::ForeignKey {
        columns: vec![n("b_id")],
        target: QName::bare("b"),
        target_columns: vec![n("id")],
        on_update: FkAction::NoAction,
        on_delete: FkAction::NoAction,
        match_kind: FkMatch::Simple,
    }
}

fn input<'a>(body: &'a ConstraintBody) -> PlaceInput<'a> {
    PlaceInput {
        body,
        name: None,
        not_valid: false,
        target_later: false,
        auto_column: None,
    }
}

#[test]
fn inside_column_applies_to_single_column_keys_only() {
    let one = pk_body(&["id"]);
    let two = pk_body(&["a", "b"]);
    let pg = DialectId::Postgres;
    assert_eq!(
        place(&input(&one), Placement::InsideColumn, pg).0,
        Placed::Inside(Position::InColumn(n("id")))
    );
    assert_eq!(
        place(&input(&two), Placement::InsideColumn, pg).0,
        Placed::Inside(Position::InTable)
    );
    let ck = ConstraintBody::Check { expr: n("(a > 0)") };
    assert_eq!(
        place(&input(&ck), Placement::InsideColumn, pg).0,
        Placed::Inside(Position::InTable)
    );
}

#[test]
fn mysql_fk_is_never_inline() {
    let fk = fk_body();
    let (p, _) = place(&input(&fk), Placement::InsideColumn, DialectId::MySql);
    assert_eq!(p, Placed::Inside(Position::InTable));
}

#[test]
fn fk_cycle_is_postponed_after_tables() {
    let fk = fk_body();
    let later = PlaceInput {
        target_later: true,
        ..input(&fk)
    };
    for d in [DialectId::Postgres, DialectId::MySql, DialectId::MariaDb] {
        assert_eq!(
            place(&later, Placement::InsideTable, d).0,
            Placed::After,
            "{d:?}"
        );
    }
}

#[test]
fn sqlite_constraints_inside_table_forced_with_notice() {
    let fk = fk_body();
    let later = PlaceInput {
        target_later: true,
        ..input(&fk)
    };
    assert_eq!(
        place(&later, Placement::InsideTable, DialectId::Sqlite),
        (
            Placed::Inside(Position::InTable),
            Some(NoticeCode::SqlitePlacementForced)
        )
    );
    let pk = pk_body(&["id"]);
    assert_eq!(
        place(&input(&pk), Placement::AfterTable, DialectId::Sqlite),
        (
            Placed::Inside(Position::InTable),
            Some(NoticeCode::SqlitePlacementForced)
        )
    );
}

#[test]
fn auto_increment_keys_stay_with_their_table() {
    let pk = pk_body(&["id"]);
    let auto = n("id");
    let i = PlaceInput {
        auto_column: Some(&auto),
        ..input(&pk)
    };
    assert_eq!(
        place(&i, Placement::AfterTable, DialectId::Sqlite).0,
        Placed::Inside(Position::InColumn(n("id")))
    );
    assert_eq!(
        place(&i, Placement::AfterTable, DialectId::MySql).0,
        Placed::Inside(Position::InTable)
    );
}

#[test]
fn pg_not_valid_goes_after_the_table() {
    let ck = ConstraintBody::Check { expr: n("(a > 0)") };
    let i = PlaceInput {
        not_valid: true,
        ..input(&ck)
    };
    assert_eq!(
        place(&i, Placement::InsideTable, DialectId::Postgres).0,
        Placed::After
    );
}

#[test]
fn mysql_inline_unique_only_when_it_keeps_its_name() {
    let uq = ConstraintBody::Unique {
        columns: vec![n("code")],
    };
    let same = n("code");
    let other = n("code_uq");
    let with = |name| PlaceInput {
        name: Some(name),
        ..input(&uq)
    };
    assert_eq!(
        place(&with(&same), Placement::InsideColumn, DialectId::MySql).0,
        Placed::Inside(Position::InColumn(n("code")))
    );
    assert_eq!(
        place(&with(&other), Placement::InsideColumn, DialectId::MySql).0,
        Placed::Inside(Position::InTable)
    );
}

// ---- qualification ------------------------------------------------------------------------

#[test]
fn qualify_auto_one_schema_equal_to_current_is_unqualified() {
    let p = QualifyPlan::new(Qualify::Auto, DialectId::Postgres, Some(n("app")));
    assert!(!p.show_schema(&QName::new(Some("app"), "t1")));
    assert!(!p.show_schema(&QName::new(Some("app"), "t2")));
}

#[test]
fn qualify_auto_two_schemas() {
    let p = QualifyPlan::new(Qualify::Auto, DialectId::Postgres, Some(n("app")));
    assert!(!p.show_schema(&QName::new(Some("app"), "t1")));
    assert!(p.show_schema(&QName::new(Some("sales"), "t2")));
}

#[test]
fn qualify_ambiguous_name_is_qualified() {
    let p = QualifyPlan::new(Qualify::Auto, DialectId::Postgres, Some(n("app")))
        .with_ambiguous([n("customers")]);
    assert!(p.show_schema(&QName::new(Some("app"), "customers")));
    assert!(!p.show_schema(&QName::new(Some("app"), "orders")));
}

#[test]
fn qualify_modes_and_sqlite_main() {
    let t = QName::new(Some("app"), "t");
    assert!(QualifyPlan::always(DialectId::MySql).show_schema(&t));
    assert!(!QualifyPlan::never(DialectId::MySql).show_schema(&t));
    let no_current = QualifyPlan::new(Qualify::Auto, DialectId::MySql, None);
    assert!(no_current.show_schema(&t));
    let s = QualifyPlan::new(Qualify::Auto, DialectId::Sqlite, None);
    assert!(!s.show_schema(&QName::new(Some("main"), "t")));
    assert!(s.show_schema(&QName::new(Some("aux"), "t")));
    assert!(s.show_schema(&QName::new(Some("temp"), "t")));
    assert!(!s.show_schema(&QName::bare("t")));
}

// ---- options ------------------------------------------------------------------------------

#[test]
fn specs_follow_the_option_order() {
    for (i, s) in SPECS.iter().enumerate() {
        assert_eq!(s.id as usize, i);
        assert_eq!(spec(s.id).id, s.id);
    }
}

#[test]
fn option_applicability_per_dialect() {
    let pg = DdlTarget::new(DialectId::Postgres);
    let my = DdlTarget::new(DialectId::MySql);
    let lite = DdlTarget::new(DialectId::Sqlite);
    let c = Category::CreateScript;
    assert!(applicable(spec(OptionId::SkipDefiner), c, &my));
    assert!(!applicable(spec(OptionId::SkipDefiner), c, &pg));
    assert!(applicable(spec(OptionId::SkipOwner), c, &pg));
    assert!(!applicable(spec(OptionId::SkipGrants), c, &lite));
    assert!(!applicable(spec(OptionId::ServerDefinitions), c, &pg));
    assert!(applicable(spec(OptionId::ServerDefinitions), c, &my));
    assert!(!applicable(
        spec(OptionId::Placement),
        Category::DefinitionsByServer,
        &pg
    ));
}

#[test]
fn or_replace_matrix() {
    use ObjectKind::*;
    let pg = DdlTarget::new(DialectId::Postgres);
    assert!(or_replace_supported(&pg, View));
    assert!(!or_replace_supported(&pg, Table));
    assert!(!or_replace_supported(&pg, MaterializedView));
    assert!(or_replace_supported(&pg, Trigger));
    let pg13 = pg.clone().with_version(ServerVersion::new(13, 0, 0));
    assert!(!or_replace_supported(&pg13, Trigger));
    let my = DdlTarget::new(DialectId::MySql);
    assert!(or_replace_supported(&my, View));
    assert!(!or_replace_supported(&my, Procedure));
    let maria = DdlTarget::new(DialectId::MariaDb);
    assert!(or_replace_supported(&maria, Procedure));
    assert!(!or_replace_supported(
        &DdlTarget::new(DialectId::Sqlite),
        View
    ));
    assert!(if_not_exists_supported(&pg, Index));
    assert!(!if_not_exists_supported(&my, Index));
    assert!(if_not_exists_supported(&maria, Index));
}
