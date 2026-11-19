//! M4 Task 7: schema qualification (alg/03 HV-DDL-10) as a pure function.

use hacivat_ddl::DdlTarget;
use hacivat_ddl::ir::QualifiedName;
use hacivat_ddl::options::Qualify;
use hacivat_ddl::produce::qualify::{QualifiedRef, QualifyCtx, qualify};
use hacivat_meta::name::Name;
use smallvec::SmallVec;
use sql_dialect::{DialectId, ServerVersion};
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

/// What `qualify` keeps of `schema.name`, as text parts.
fn shown(ctx: &QualifyCtx, schema: &str, name: &str) -> Vec<String> {
    let r: QualifiedRef = qualify(&qn(&[schema, name]), ctx);
    r.to_qualified_name()
        .parts
        .iter()
        .map(|p| p.text.to_string())
        .collect()
}

#[test]
fn qualify_auto_one_schema_equal_to_current_is_unqualified() {
    let ctx = QualifyCtx::new(Qualify::Auto, DialectId::Postgres, Some(n("app")));
    assert_eq!(shown(&ctx, "app", "t1"), ["t1"]);
    assert_eq!(shown(&ctx, "app", "t2"), ["t2"]);
    let r = qualify(&qn(&["app", "t1"]), &ctx);
    assert!(!r.is_qualified());
    assert_eq!(r.name.as_ref(), "t1");
    // alg/03 HV-DDL-10 test 1: the same selection while `public` is current is qualified.
    let public = QualifyCtx::new(Qualify::Auto, DialectId::Postgres, Some(n("public")));
    assert_eq!(shown(&public, "app", "t1"), ["app", "t1"]);
}

#[test]
fn qualify_auto_two_schemas() {
    // `app.t1` + `sales.t2`, current `app` → `t1`, `sales.t2` (alg/03 HV-DDL-10 test 2).
    let ctx = QualifyCtx::new(Qualify::Auto, DialectId::Postgres, Some(n("app")));
    assert_eq!(shown(&ctx, "app", "t1"), ["t1"]);
    assert_eq!(shown(&ctx, "sales", "t2"), ["sales", "t2"]);
}

#[test]
fn qualify_ambiguous_name_is_qualified() {
    // A name that exists in more than one schema of the scope is qualified even in the current
    // schema.
    let ctx = QualifyCtx::new(Qualify::Auto, DialectId::Postgres, Some(n("app")))
        .with_ambiguous([n("customers")]);
    assert_eq!(shown(&ctx, "app", "customers"), ["app", "customers"]);
    assert_eq!(shown(&ctx, "app", "orders"), ["orders"]);
}

#[test]
fn qualify_modes_and_dialects() {
    for d in [
        DialectId::Postgres,
        DialectId::MySql,
        DialectId::MariaDb,
        DialectId::Sqlite,
    ] {
        let always = QualifyCtx::new(Qualify::Always, d, Some(n("app")));
        assert_eq!(shown(&always, "app", "t"), ["app", "t"], "{d:?}");
        let never = QualifyCtx::new(Qualify::Never, d, None);
        assert_eq!(shown(&never, "sales", "t"), ["t"], "{d:?}");
    }
    // No current schema: every name is qualified.
    let none = QualifyCtx::new(Qualify::Auto, DialectId::MySql, None);
    assert_eq!(shown(&none, "shop", "t"), ["shop", "t"]);
    // SQLite: `main` stays bare, `temp` and attached schemas are qualified.
    let lite = QualifyCtx::new(Qualify::Auto, DialectId::Sqlite, None);
    assert_eq!(shown(&lite, "main", "t"), ["t"]);
    assert_eq!(shown(&lite, "aux", "t"), ["aux", "t"]);
    assert_eq!(shown(&lite, "temp", "t"), ["temp", "t"]);
    // A bare name stays bare; a database part before the schema is never written.
    assert_eq!(
        qualify(
            &qn(&["t"]),
            &QualifyCtx::new(Qualify::Always, DialectId::Postgres, None)
        )
        .to_qualified_name(),
        qn(&["t"])
    );
    assert_eq!(
        qualify(
            &qn(&["db", "app", "t"]),
            &QualifyCtx::new(Qualify::Always, DialectId::Postgres, None)
        )
        .to_qualified_name(),
        qn(&["app", "t"])
    );
}

#[test]
fn current_schema_comes_from_the_target() {
    let mut t = DdlTarget::new(DialectId::Postgres, ServerVersion::new(17, 0, 0));
    // Only the first `search_path` entry counts (HV-DDL-10: anything not in the first schema
    // is qualified).
    t.search_path = vec![n("app"), n("public")];
    let ctx = QualifyCtx::for_target(&t, Qualify::Auto);
    assert_eq!(shown(&ctx, "app", "t"), ["t"]);
    assert_eq!(shown(&ctx, "public", "t"), ["public", "t"]);
    // The session's current schema wins over the search path.
    t.current_schema = Some(n("public"));
    let ctx = QualifyCtx::for_target(&t, Qualify::Auto);
    assert_eq!(shown(&ctx, "public", "t"), ["t"]);
}
