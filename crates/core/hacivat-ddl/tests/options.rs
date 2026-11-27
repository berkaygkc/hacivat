//! M4 Task 7: option applicability (alg/03 HV-DDL-11 v1 core subset, categories
//! `CreateScript` and `DefinitionsByServer` in M4).

use hacivat_ddl::DdlTarget;
use hacivat_ddl::options::{
    Category, GenOptions, OptionId, Placement, Qualify, SPECS, applicable, if_not_exists_supported,
    or_replace_supported, spec,
};
use hacivat_meta::kind::ObjectKind;
use sql_dialect::{DialectId, ServerVersion};

fn t(d: DialectId) -> DdlTarget {
    DdlTarget::new(d, ServerVersion::new(17, 0, 0))
}

#[test]
fn defaults_favour_safety() {
    // R-M4-23 and alg/03 HV-DDL-11 "Decision".
    let o = GenOptions::default();
    assert_eq!(o.qualify, Qualify::Auto);
    assert_eq!(o.placement, Placement::InsideTable);
    assert!(!o.or_replace && !o.if_not_exists);
    assert!(!o.skip_grants && !o.skip_owner && !o.skip_definer);
    assert!(!o.surrogate_names && !o.server_definitions);
}

#[test]
fn specs_follow_the_option_order() {
    assert_eq!(SPECS.len(), 9);
    for (i, s) in SPECS.iter().enumerate() {
        assert_eq!(s.id as usize, i);
        assert_eq!(spec(s.id).id, s.id);
    }
}

#[test]
fn option_applicability_per_dialect() {
    let (pg, my, maria, lite) = (
        t(DialectId::Postgres),
        t(DialectId::MySql),
        t(DialectId::MariaDb),
        t(DialectId::Sqlite),
    );
    let c = Category::CreateScript;
    for target in [&pg, &my, &maria, &lite] {
        for id in [
            OptionId::Qualify,
            OptionId::Placement,
            OptionId::SurrogateNames,
        ] {
            assert!(applicable(spec(id), c, target), "{id:?}");
        }
    }
    // Skip DEFINER: MySQL and MariaDB only.
    assert!(applicable(spec(OptionId::SkipDefiner), c, &my));
    assert!(applicable(spec(OptionId::SkipDefiner), c, &maria));
    assert!(!applicable(spec(OptionId::SkipDefiner), c, &pg));
    // Owners exist on PostgreSQL only; SQLite has neither owners nor grants.
    assert!(applicable(spec(OptionId::SkipOwner), c, &pg));
    assert!(!applicable(spec(OptionId::SkipOwner), c, &my));
    assert!(!applicable(spec(OptionId::SkipGrants), c, &lite));
    assert!(applicable(spec(OptionId::SkipGrants), c, &my));
    // "Definitions provided by server" is hidden on PostgreSQL, which has no table
    // definition function (R-M4-24).
    assert!(!applicable(spec(OptionId::ServerDefinitions), c, &pg));
    assert!(applicable(spec(OptionId::ServerDefinitions), c, &my));
    assert!(applicable(spec(OptionId::ServerDefinitions), c, &lite));
    // Server text is never re-placed or re-qualified.
    let s = Category::DefinitionsByServer;
    assert!(!applicable(spec(OptionId::Placement), s, &pg));
    assert!(!applicable(spec(OptionId::Qualify), s, &my));
    assert!(applicable(spec(OptionId::SkipDefiner), s, &my));
    // M7's categories have no M4 option yet.
    for spec in &SPECS {
        assert!(!applicable(spec, Category::Drop, &pg), "{:?}", spec.id);
    }
}

#[test]
fn or_replace_applies_to_pg_view_not_table() {
    use ObjectKind::*;
    let pg = t(DialectId::Postgres);
    assert!(or_replace_supported(&pg, View));
    assert!(or_replace_supported(&pg, Routine));
    assert!(!or_replace_supported(&pg, Table));
    assert!(!or_replace_supported(&pg, MaterializedView));
    assert!(or_replace_supported(&pg, Trigger));
    let mut pg13 = pg.clone();
    pg13.version = ServerVersion::new(13, 0, 0);
    assert!(!or_replace_supported(&pg13, Trigger));
    let my = t(DialectId::MySql);
    assert!(or_replace_supported(&my, View));
    assert!(!or_replace_supported(&my, Procedure));
    let maria = t(DialectId::MariaDb);
    assert!(or_replace_supported(&maria, Procedure));
    assert!(or_replace_supported(&maria, Trigger));
    assert!(!or_replace_supported(&maria, Table));
    assert!(!or_replace_supported(&t(DialectId::Sqlite), View));
}

#[test]
fn if_not_exists_matrix() {
    use ObjectKind::*;
    let pg = t(DialectId::Postgres);
    assert!(if_not_exists_supported(&pg, Table));
    assert!(if_not_exists_supported(&pg, Index));
    assert!(!if_not_exists_supported(&pg, View));
    assert!(!if_not_exists_supported(&t(DialectId::MySql), Index));
    assert!(if_not_exists_supported(&t(DialectId::MariaDb), Index));
    assert!(if_not_exists_supported(&t(DialectId::Sqlite), Table));
    assert!(!if_not_exists_supported(&t(DialectId::SqlServer), Table));
}
