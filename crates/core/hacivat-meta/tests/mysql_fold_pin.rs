// ## m4-t17b-mysql-facets
//! R-M4-41 pinned (M4 Task 17b): a MySQL/MariaDB column lookup over every recorded probe pair
//! of Task 17's probe (CI 44649745390, `t17-docker-facts-44649745390.txt`). Where the server
//! found the column, the lookup is `Found`; where it answered 1054, the lookup is never
//! `Found` (and never `Missing` for a text the server's rule is not pinned for).

use hacivat_meta::fixture::ns_mysql;
use hacivat_meta::kind::ObjectKind;
use hacivat_meta::name::{DialectId, IdentCtx};
use hacivat_meta::view::{Lookup, UnknownReason};

/// `(stored, probe, found on MySQL 8.0/8.4, found on MariaDB 11.4)`, as recorded.
const PAIRS: [(&str, &str, bool, bool); 18] = [
    ("Äb", "äb", true, true),
    ("Äb", "ıd", false, false),
    ("Äb", "Id", false, false),
    ("Äb", "ID", false, false),
    ("Äb", "id", false, false),
    ("Äb", "İd", false, false),
    ("ıd", "äb", false, false),
    ("ıd", "Id", false, false),
    ("ıd", "ID", false, false),
    ("ıd", "ıd", true, true),
    ("ıd", "id", false, false),
    ("ıd", "İd", false, false),
    ("İd", "äb", false, false),
    ("İd", "ıd", false, false),
    ("İd", "Id", true, false),
    ("İd", "ID", true, false),
    ("İd", "id", true, false),
    ("İd", "İd", true, true),
];

fn ctx(d: DialectId) -> IdentCtx {
    IdentCtx::new(d)
}

#[test]
fn every_recorded_column_pair_is_found_exactly_where_the_server_found_it() {
    for (d, col) in [(DialectId::MySql, 2), (DialectId::MariaDb, 3)] {
        for (stored, probe, my, maria) in PAIRS {
            let found = if col == 2 { my } else { maria };
            let n = ns_mysql("shop", 0).table("t", &[stored]).finish();
            let t = n
                .lookup_kind(ObjectKind::Table, "t")
                .next()
                .expect("the table");
            for quoted in [false, true] {
                let got = n.match_child(t, ObjectKind::Column, probe, quoted, &ctx(d));
                if found {
                    assert!(
                        matches!(got, Lookup::Found(i) if i == t + 1),
                        "{d:?}: {stored} vs {probe}: {got:?}"
                    );
                } else {
                    assert!(
                        matches!(got, Lookup::Unknown(UnknownReason::ServerDependent)),
                        "{d:?}: {stored} vs {probe}: {got:?}"
                    );
                }
            }
        }
    }
}

/// Letters the probe did not cover decide nothing: never `Found` by a fold, never `Missing`.
#[test]
fn unprobed_letters_stay_server_dependent() {
    for d in [DialectId::MySql, DialectId::MariaDb] {
        for (stored, probe) in [
            ("Éa", "éa"),
            ("Öz", "öz"),
            ("ab", "äb"),
            ("ß", "SS"),
            ("ǅ", "ǆ"),
        ] {
            let n = ns_mysql("shop", 0).table("t", &[stored]).finish();
            let t = n.lookup_kind(ObjectKind::Table, "t").next().unwrap();
            let got = n.match_child(t, ObjectKind::Column, probe, false, &ctx(d));
            assert!(
                matches!(got, Lookup::Unknown(UnknownReason::ServerDependent)),
                "{d:?}: {stored} vs {probe}: {got:?}"
            );
        }
    }
    // Not a column (index names were not probed): only an exact match.
    let n = ns_mysql("shop", 0).table("t", &["İd"]).finish();
    let t = n.lookup_kind(ObjectKind::Table, "t").next().unwrap();
    assert!(matches!(
        n.match_child(t, ObjectKind::Index, "id", false, &ctx(DialectId::MySql)),
        Lookup::Missing(_) | Lookup::Unknown(_)
    ));
}
