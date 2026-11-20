// ## m4-t15b-facets (fix round 1: the T15b security review's hostile probes, committed)
//! T15b security review: hostile probes the implementer did not write.
//!
//! H1 and H1b failed at review (finding I-1) and now assert the fix (ruling R-W5-5, which
//! corrects R-W3-11): only the session's **own** temp schema (`pg_my_temp_schema()`) is
//! skipped before `pg_catalog`; another backend's `pg_temp_N` is searched for operators by
//! PostgreSQL and so refuses the path.
//!
//! The review's H2/H3 probes run `fragment::on_session` over a scripted PostgreSQL job session
//! (`hacivat_driver::fake::FakeDriver`), which this crate's dev-dependencies do not enable
//! (`hacivat-driver` without `fake`; a manifest change is the controller's). Their adapted text
//! (the own-temp column, `ServerFacts`) is kept beside the review for that amendment. Until
//! then: H2's decision is H1/H1b here plus `fragment`'s unit tests; H3's "nothing is sent on a
//! dbms mismatch" is `hacivat-session`'s `join_as_refuses_another_engine_before_anything_is_sent`
//! (the actor refuses before sending), which `on_session` uses (`MetaConn::join_as`); the
//! Docker test `joined_read_refuses_another_backends_temp_schema` runs the attack for real.

#![cfg(feature = "session")]

use hacivat_introspect::fragment::pg_path_is_safe;

/// The session's own temp schema (pg_temp_3), then a foreign one (pg_temp_7): refused.
#[test]
fn h1_a_foreign_temp_schema_before_pg_catalog_is_not_safe() {
    assert!(
        !pg_path_is_safe(&["pg_temp_3", "pg_temp_7", "pg_catalog"], Some("pg_temp_3")),
        "a second pg_temp_N is another backend's: operators there are searched"
    );
}

/// No own temp schema yet; the first entry is someone else's: refused.
#[test]
fn h1b_a_lone_foreign_temp_schema_first_is_not_safe() {
    assert!(!pg_path_is_safe(&["pg_temp_7", "pg_catalog"], None));
    // The same name is safe only when it is the session's own.
    assert!(pg_path_is_safe(
        &["pg_temp_7", "pg_catalog"],
        Some("pg_temp_7")
    ));
}
