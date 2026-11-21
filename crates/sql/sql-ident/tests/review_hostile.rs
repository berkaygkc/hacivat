//! The M4 Task 3 reviewer's hostile test (committed in fix round 1; it failed on 8d3f30c, I1).
use sql_ident::key::{DialectId, IdentCtx, NameClass, match_key};
use std::sync::Arc;

/// Redshift folds ASCII letters of *quoted* identifiers too (AWS "Names and identifiers"), so a
/// PG-rule key for quoted `"Users"` answers a wrong `Missing` for stored `users`.
#[test]
fn finding_redshift_quoted_mixed_case_is_not_pg_exact() {
    let d = DialectId::PgWire(sql_dialect::PgWireFlavor::Redshift);
    let ctx = IdentCtx {
        encoding: Some(Arc::from("UTF8")),
        ..IdentCtx::new(d)
    };
    let k = match_key(d, &ctx, "Users", true, NameClass::Table);
    assert!(
        k.is_err() || k.as_ref().unwrap().as_str() == "users",
        "Redshift quoted \"Users\" keyed as {:?}",
        k.map(|k| k.as_str().to_string())
    );
}
