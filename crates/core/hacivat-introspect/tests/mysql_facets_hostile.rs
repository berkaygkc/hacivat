// ## m4-t17b-mysql-facets (fix round 1)
//! The T17b security review's hostile tests (reviewer-written; committed in fix round 1 with
//! HR1 (I-1), HR5b (m-1) and HR2's NULL-user case (m-5) asserting the fixes), and its fold
//! probes HF1–HF4 (`task-m4-17b-review-fold.rs`).
#![cfg(feature = "session")]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use hacivat_driver::ServerMessage;
use hacivat_introspect::assemble::{ColumnIndex, TableId, mysql as asm};
use hacivat_introspect::mysql::facets::{
    self, Coverage, Opaque, coverage, decide_deps, decide_relation_grants, decide_schema_grants,
    name, read_on,
};
use hacivat_introspect::mysql::pass::SectionReader;
use hacivat_introspect::mysql::server::ServerCtx;
use hacivat_introspect::pack::{BindValue, DialectId, LogicalType, Pack, PackCtx, Params};
use hacivat_introspect::{FacetRows, IntroError};
use hacivat_meta::name::IdentCtx;
use hacivat_meta::namespace::ObjFlags;
use hacivat_meta::props::FacetKind;
use hacivat_meta::{Name, NsKey, ObjectKey, ObjectKind};
use hacivat_proto::DbmsKind;
use hacivat_session::meta::{MetaError, Rows, check_own_text, check_read_text_for};

type Row = Vec<Option<Arc<str>>>;

fn n(s: &str) -> Name {
    Name::from(s)
}

fn row(cells: &[Option<&str>]) -> Row {
    cells.iter().map(|c| c.map(Arc::from)).collect()
}

fn key(kind: ObjectKind, path: &[&str]) -> ObjectKey {
    ObjectKey {
        kind,
        path: path.iter().map(|p| n(p)).collect(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    }
}

fn mysql84() -> ServerCtx {
    let mut s = ServerCtx::assumed(DbmsKind::MySql, "8.4.3");
    s.login = Some(n("u@%"));
    s
}

fn refusal(code: i64) -> IntroError {
    IntroError::Meta(MetaError::Server {
        sqlstate: Some(Arc::from("42000")),
        message: ServerMessage {
            sqlstate: Some("42000".into()),
            vendor_code: Some(code),
            severity: None,
            text: "refused".into(),
            detail: None,
            hint: None,
            lang_hint: None,
            position: None,
        },
    })
}

/// Renders every call from the real pack (guard-checked), answers from a script.
#[derive(Default)]
struct Fake {
    answers: HashMap<&'static str, Result<Vec<Row>, IntroError>>,
    read: Arc<Mutex<Vec<&'static str>>>,
}

impl SectionReader for Fake {
    fn read<'a>(
        &'a mut self,
        section: &'static str,
        ctx: PackCtx,
    ) -> BoxFuture<'a, Result<Rows, IntroError>> {
        Box::pin(async move {
            let p = Pack::builtin(DialectId::MySql).unwrap();
            let qs = p
                .render(section, &ctx)
                .map_err(|_| IntroError::Protocol(n(section)))?;
            for q in &qs {
                check_own_text(q.sql(), DbmsKind::MySql).unwrap();
            }
            self.read.lock().unwrap().push(section);
            let columns: Vec<Arc<str>> = p
                .section(section)
                .unwrap()
                .returns()
                .iter()
                .map(|c| c.name.clone())
                .collect();
            let rows = match self.answers.get(section) {
                Some(Ok(r)) => r.clone(),
                Some(Err(e)) => return Err(e.clone()),
                None => Vec::new(),
            };
            Ok(Rows { columns, rows })
        })
    }
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(f)
}

fn db_row(user: &str, db: &str, yes: &[usize]) -> Row {
    let mut r = vec![
        Some(Arc::from(user)),
        Some(Arc::from("%")),
        Some(Arc::from(db)),
    ];
    for i in 0..19 {
        r.push(Some(Arc::from(if yes.contains(&i) { "Y" } else { "N" })));
    }
    r
}

/// HR1. `partial_revokes = ON`: `REVOKE SELECT ON db.* FROM u` (u holding global SELECT) is
/// stored in `mysql.user.User_attributes` (Restrictions), which the facet never reads. The
/// schema's grants must then be not loaded (R-W4-4 by analogy: a revocation the read cannot
/// see), never answered complete.
#[test]
fn hr1_schema_grants_are_not_loaded_under_partial_revokes() {
    let mut f = Fake::default();
    // u has INSERT on the schema (a mysql.db row); its SELECT is restricted (not visible).
    f.answers
        .insert(name::GRANTS_SCHEMA, Ok(vec![db_row("u", "hvm4_db", &[1])]));
    f.answers
        .insert(name::PARTIAL_REVOKES, Ok(vec![row(&[Some("1")])]));
    let ns = NsKey::schema(["hvm4_db"]);
    let schema = key(ObjectKind::Schema, &["hvm4_db"]);
    let got = block_on(read_on(
        &mut f,
        &mysql84(),
        &ns,
        std::slice::from_ref(&schema),
        FacetKind::Grants,
    ))
    .unwrap();
    let read = f.read.lock().unwrap().clone();
    let FacetRows::Grants(got) = got else {
        panic!("{got:?}")
    };
    assert!(
        got.iter().all(|(k, _)| *k != schema),
        "schema grants answered complete under partial_revokes=ON (sections read: {read:?}): {got:?}"
    );
}

/// HR2. Grant rows with roles, wildcards and odd text through the decoders.
#[test]
fn hr2_grant_rows_with_roles_wildcards_and_odd_text() {
    let t = key(ObjectKind::Table, &["hvm4_db", "t"]);
    // A MySQL 8 role is an account: its table grant is an entry.
    let got = decide_relation_grants(
        std::slice::from_ref(&t),
        &[row(&[Some("r"), Some("%"), Some("t"), Some("Select")])],
        &[],
        false,
    );
    assert!(got[0].complete());
    assert_eq!(&*got[0].facet.entries[0].grantee, "'r'@'%'");
    // A MariaDB role row (empty host): not loaded.
    let got = decide_relation_grants(
        std::slice::from_ref(&t),
        &[row(&[Some("r"), Some(""), Some("t"), Some("Select")])],
        &[],
        true,
    );
    assert!(!got[0].complete(), "{got:?}");
    // Elements outside the documented set, in any spelling: not loaded.
    for bad in [
        "select", "Execute", " Select", "Select ", "SELECT", "Usage", "Proxy",
    ] {
        let got = decide_relation_grants(
            std::slice::from_ref(&t),
            &[row(&[Some("a"), Some("%"), Some("t"), Some(bad)])],
            &[],
            false,
        );
        assert!(!got[0].complete(), "{bad}: {got:?}");
    }
    // A column grant whose table row is stored for another host spelling: not loaded.
    let got = decide_relation_grants(
        std::slice::from_ref(&t),
        &[row(&[Some("a"), Some("LOCALHOST"), Some("t"), Some("")])],
        &[row(&[
            Some("a"),
            Some("localhost"),
            Some("t"),
            Some("c"),
            Some("Select"),
        ])],
        false,
    );
    assert!(got[0].opaque.contains(&Opaque::NoTableRow), "{got:?}");
    // Fix round 1 (m-5): a NULL user (the column is NOT NULL: a broken or hostile server) is
    // opaque, never the anonymous user `''@'%'`.
    let got = decide_relation_grants(
        std::slice::from_ref(&t),
        &[row(&[None, Some("%"), Some("t"), Some("Select")])],
        &[],
        false,
    );
    assert!(!got[0].complete(), "{got:?}");
    assert!(got[0].facet.entries.is_empty(), "{got:?}");

    // Schema rows: patterns are never this schema's entries; the escaped spelling is opaque.
    let s = key(ObjectKind::Schema, &["hvm4_db"]);
    for pattern in ["hvm4\\_%", "hvm4_%", "%", "hvm4%", "HVM4_DB", "hvm4_db "] {
        let got = decide_schema_grants(&s, "hvm4_db", &[db_row("p", pattern, &[0])]);
        assert!(
            got.complete() && got.facet.entries.is_empty(),
            "{pattern}: {got:?}"
        );
    }
    let got = decide_schema_grants(&s, "hvm4_db", &[db_row("p", "hvm4\\_db", &[0])]);
    assert!(got.opaque.contains(&Opaque::EscapedSchemaRow));
    // Odd Y/N cells: not loaded.
    for cell in [Some("y"), Some(""), Some("1"), None] {
        let mut r = db_row("p", "hvm4_db", &[]);
        r[3] = cell.map(Arc::from);
        let got = decide_schema_grants(&s, "hvm4_db", &[r]);
        assert!(!got.complete(), "{cell:?}: {got:?}");
    }
    // Grant option alone on the schema: not loaded.
    let got = decide_schema_grants(&s, "hvm4_db", &[db_row("p", "hvm4_db", &[6])]);
    assert!(got.opaque.contains(&Opaque::GrantOptionOnly));
}

/// HR3. The dependency gate: read correctly and fail-closed.
#[test]
fn hr3_dependency_gate_probes() {
    let both = |g: &str| {
        vec![
            row(&[Some(g), Some("SELECT")]),
            row(&[Some(g), Some("EXECUTE")]),
        ]
    };
    assert_eq!(
        coverage(Some("u@%"), &both("'u'@'%'"), Some("0")),
        Coverage::Complete
    );
    assert_eq!(
        coverage(Some("u@%"), &both("'u'@'%'"), None),
        Coverage::Complete
    );
    // The server prints the grantee unescaped.
    assert_eq!(
        coverage(Some("o'b@%"), &both("'o'b'@'%'"), Some("0")),
        Coverage::Complete
    );
    for pr in ["1", "ON", "", "on", "?"] {
        assert!(
            matches!(
                coverage(Some("u@%"), &both("'u'@'%'"), Some(pr)),
                Coverage::Incomplete(_)
            ),
            "{pr}"
        );
    }
    // Another account's rows; split privileges; an ambiguous login; no login.
    for (login, rows) in [
        ("u@localhost", both("'u'@'%'")),
        ("u@%", both("'u'@'%' ")),
        (
            "u@%",
            vec![
                row(&[Some("'u'@'%'"), Some("SELECT")]),
                row(&[Some("'u2'@'%'"), Some("EXECUTE")]),
            ],
        ),
        ("a@b@c", both("'a'@'b@c'")),
        ("a@b@c", both("'a@b'@'c'")),
        ("u@%", vec![row(&[Some("'u'@'%'"), Some("SELECT")])]),
        (
            "u@%",
            vec![
                row(&[Some("'u'@'%'"), Some("select")]),
                row(&[Some("'u'@'%'"), Some("execute")]),
            ],
        ),
    ] {
        assert!(
            matches!(coverage(Some(login), &rows, None), Coverage::Incomplete(_)),
            "{login} {rows:?}"
        );
    }
    assert!(matches!(
        coverage(None, &both("'u'@'%'"), None),
        Coverage::Incomplete(_)
    ));

    // Through the read: `partial_revokes` read as no row is not "off"; a refusal is Permission.
    let ns = NsKey::schema(["hvm4_db"]);
    let v = key(ObjectKind::View, &["hvm4_db", "v"]);
    let mut f = Fake::default();
    f.answers.insert(name::DEPS_COVERAGE, Ok(both("'u'@'%'")));
    f.answers.insert(name::PARTIAL_REVOKES, Ok(vec![]));
    let (_, cov) = block_on(facets::dependencies(
        &mut f,
        &mysql84(),
        &ns,
        std::slice::from_ref(&v),
    ))
    .unwrap();
    assert!(matches!(cov, Coverage::Incomplete(_)), "{cov:?}");
    let mut f = Fake::default();
    f.answers.insert(name::DEPS_COVERAGE, Ok(both("'u'@'%'")));
    f.answers.insert(name::PARTIAL_REVOKES, Err(refusal(1227)));
    let got = block_on(read_on(
        &mut f,
        &mysql84(),
        &ns,
        std::slice::from_ref(&v),
        FacetKind::Dependencies,
    ));
    assert_eq!(
        got,
        Err(IntroError::Permission {
            kind: ObjectKind::View
        })
    );
    let mut f = Fake::default();
    f.answers.insert(name::DEPS_COVERAGE, Err(refusal(1142)));
    let got = block_on(read_on(
        &mut f,
        &mysql84(),
        &ns,
        std::slice::from_ref(&v),
        FacetKind::Dependencies,
    ));
    assert_eq!(
        got,
        Err(IntroError::Permission {
            kind: ObjectKind::View
        })
    );
    // Complete coverage: a used relation of an unknown kind (or no TABLES row) is not answered.
    let deps = decide_deps(
        std::slice::from_ref(&v),
        &[row(&[Some("v")])],
        &[row(&[Some("v"), Some("s"), Some("t"), None])],
        &[],
    );
    assert!(deps[0].unnamed);
    let deps = decide_deps(
        std::slice::from_ref(&v),
        &[row(&[Some("v")])],
        &[row(&[Some("v"), Some("s"), Some("t"), Some("SEQUENCE")])],
        &[],
    );
    assert!(deps[0].unnamed, "a MySQL row of an unmodelled kind");
    // A view the login cannot see is not answered even with usage rows.
    let deps = decide_deps(
        std::slice::from_ref(&v),
        &[],
        &[row(&[Some("v"), Some("s"), Some("t"), Some("BASE TABLE")])],
        &[],
    );
    assert!(deps.is_empty());
}

fn text(s: &str) -> BindValue {
    BindValue::Typed {
        hint: LogicalType::Text {
            max_chars: None,
            fixed: false,
        },
        text: Arc::from(s),
    }
}

/// HR4. Guard probe: binds shaped like placeholders, named params, executable comments, NUL
/// and a statement break never reach the text; the only `mysql.` tables named are the three
/// grant tables; no secret-bearing or role-graph table is named.
#[test]
fn hr4_facet_text_is_trusted_and_names_only_the_three_grant_tables() {
    let p = Pack::builtin(DialectId::MySql).unwrap();
    let hostile = [
        "?",
        ":schema",
        "/*!50000 DROP*/",
        "a\0b",
        "x; SELECT 1",
        "`",
        "\\'",
    ];
    let params = Params {
        scalars: vec![(n("schema"), text(hostile[1]))],
        lists: vec![(n("names"), hostile.iter().map(|h| text(h)).collect())],
    };
    let mut rendered = 0;
    for s in name::ALL {
        for mut ctx in p.reachable_variants(s) {
            ctx.params = params.clone();
            let Ok(qs) = p.render(s, &ctx) else { continue };
            for q in qs {
                rendered += 1;
                let sql = q.sql();
                for k in [DbmsKind::MySql, DbmsKind::MariaDb] {
                    check_own_text(sql, k).unwrap_or_else(|e| panic!("{s}: {e:?}"));
                    check_read_text_for(sql, k).unwrap_or_else(|e| panic!("{s}: {e:?}"));
                }
                assert!(
                    !sql.contains("/*!") && !sql.contains('\0') && !sql.contains(';'),
                    "{s}"
                );
                assert!(!sql.contains("DROP") && !sql.contains("SELECT 1"), "{s}");
                let low = sql.to_ascii_lowercase();
                let mut rest = low.as_str();
                while let Some(i) = rest.find("mysql.") {
                    let tail = &rest[i + 6..];
                    let table: String = tail
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect();
                    assert!(
                        matches!(table.as_str(), "db" | "tables_priv" | "columns_priv"),
                        "{s}: mysql.{table}"
                    );
                    rest = &tail[table.len()..];
                }
                for banned in [
                    "user_attributes",
                    "authentication",
                    "password",
                    "plugin",
                    "ssl_cipher",
                    "x509",
                    "proxies_priv",
                    "role_edges",
                    "default_roles",
                    "global_grants",
                    "procs_priv",
                    "for update",
                    "lock in share",
                    "into ",
                    "@@session",
                    "set ",
                ] {
                    assert!(!low.contains(banned), "{s}: {banned}");
                }
            }
        }
    }
    assert!(rendered >= name::ALL.len());
}

fn stat(index: &str, seq: u32, column: &str) -> asm::StatRow {
    asm::StatRow {
        table: n("t"),
        index: n(index),
        seq_in_index: seq,
        column: Some(n(column)),
        expression: None,
        collation: Some(n("A")),
        sub_part: None,
        non_unique: true,
        index_type: Some(n("BTREE")),
        visible: true,
        comment: None,
    }
}

fn fk_rows(index: asm::StatRow, fk_name: &str) -> asm::KeyRows {
    let mut r = asm::KeyRows::new(asm::Flavor::MySql);
    r.fk_index = asm::FkIndexRule::NamedAfterConstraint;
    r.indexes = vec![index];
    r.foreign_keys = vec![asm::FkRow {
        table: n("t"),
        name: n(fk_name),
        referenced_schema: n("shop"),
        referenced_table: n("p"),
        unique_constraint_name: Some(n("PRIMARY")),
        update_rule: n("NO ACTION"),
        delete_rule: n("NO ACTION"),
        match_option: n("NONE"),
    }];
    r.fk_columns = vec![asm::FkColumnRow {
        table: n("t"),
        constraint: n(fk_name),
        column: n("pid"),
        ordinal_position: 1,
        referenced_column: Some(n("id")),
    }];
    r
}

fn cols() -> ColumnIndex {
    let mut c = ColumnIndex::new(
        NsKey::schema(["shop"]),
        DialectId::MySql,
        IdentCtx::new(DialectId::MySql),
    );
    c.add_table(TableId::named("t"), [(1, n("pid"))]);
    c.add_table(TableId::named("p"), [(1, n("id"))]);
    c
}

fn implicit(rows: asm::KeyRows, index: &str) -> bool {
    asm::assemble_keys(rows, &cols())
        .iter()
        .find(|o| o.obj.kind() == ObjectKind::Index && &**o.obj.name() == index)
        .expect("the index")
        .obj
        .flags()
        .contains(ObjFlags::IMPLICIT)
}

/// HR5a. The pinned FK rule: the index InnoDB names after a named FK is implicit; an unnamed
/// FK's index (named after its column) stays shown.
#[test]
fn hr5a_fk_index_rule_named_and_unnamed() {
    assert!(implicit(
        fk_rows(stat("fk_named", 1, "pid"), "fk_named"),
        "fk_named"
    ));
    assert!(!implicit(fk_rows(stat("pid", 1, "pid"), "t_ibfk_1"), "pid"));
    // A user index named like an unnamed FK's generated constraint name: still the FK's name
    // rule; it only hides an index with exactly the FK's columns.
    assert!(!implicit(
        fk_rows(stat("t_ibfk_1", 1, "id_other"), "t_ibfk_1"),
        "t_ibfk_1"
    ));
}

/// HR5b. An index named like the FK, on exactly its columns, but with attributes InnoDB's own
/// index never has (a comment, a descending part, a prefix, invisible, another method): the
/// user's index, so it must stay shown (hiding it drops the attribute from the DDL).
#[test]
fn hr5b_fk_index_with_user_attributes_stays_shown() {
    let mut failures = Vec::new();
    let cases: Vec<(&str, asm::StatRow)> = vec![
        (
            "comment",
            asm::StatRow {
                comment: Some(n("mine")),
                ..stat("fk_x", 1, "pid")
            },
        ),
        (
            "desc",
            asm::StatRow {
                collation: Some(n("D")),
                ..stat("fk_x", 1, "pid")
            },
        ),
        (
            "prefix",
            asm::StatRow {
                sub_part: Some(10),
                ..stat("fk_x", 1, "pid")
            },
        ),
        (
            "invisible",
            asm::StatRow {
                visible: false,
                ..stat("fk_x", 1, "pid")
            },
        ),
        (
            "hash",
            asm::StatRow {
                index_type: Some(n("HASH")),
                ..stat("fk_x", 1, "pid")
            },
        ),
    ];
    for (what, ix) in cases {
        if implicit(fk_rows(ix, "fk_x"), "fk_x") {
            failures.push(what);
        }
    }
    assert!(
        failures.is_empty(),
        "hidden as the FK's implicit index: {failures:?}"
    );
}

// ---- the review's fold probes (task-m4-17b-review-fold.rs) --------------------------------------

mod fold {
    use hacivat_meta::fixture::ns_mysql;
    use hacivat_meta::kind::ObjectKind;
    use hacivat_meta::name::{DialectId, IdentCtx, has_non_ascii_cased};
    use hacivat_meta::view::{Lookup, UnknownReason};

    /// The equalities the T17 facts prove (plus ASCII case), per flavor: `Some(class)` for a
    /// letter the server folds into a recorded class, `None` for a letter only equal to itself.
    fn class(d: DialectId, c: char) -> char {
        match (d, c) {
            (_, 'A'..='Z') => c.to_ascii_lowercase(),
            (_, 'Ä') => 'ä',
            (DialectId::MySql, 'İ') => 'i',
            _ => c,
        }
    }

    fn proven_equal(d: DialectId, a: &str, b: &str) -> bool {
        a.chars().count() == b.chars().count()
            && a.chars()
                .zip(b.chars())
                .all(|(x, y)| class(d, x) == class(d, y))
    }

    /// Every pair over a hostile alphabet, both flavors, quoted and not: `Found` only where the
    /// facts prove equality; `Missing` only for an ASCII probe against a stored name with no
    /// non-ASCII cased letter (the pre-T17b rule; T17b must not widen it).
    #[test]
    fn hf1_found_only_when_proven_and_missing_never_widened() {
        let alphabet = [
            "a", "A", "i", "I", "İ", "ı", "ä", "Ä", "\u{212A}", "k", "é", "É", "ß", "ǅ", "ǆ", "Ⓐ",
            "ª", "µ", "a\u{308}", "·",
        ];
        let mut words: Vec<String> = Vec::new();
        for x in alphabet {
            words.push(format!("{x}d"));
            for y in alphabet {
                words.push(format!("{x}{y}"));
            }
        }
        let mut checked = 0usize;
        for d in [DialectId::MySql, DialectId::MariaDb] {
            let ctx = IdentCtx::new(d);
            for stored in &words {
                let ns = ns_mysql("shop", 0).table("t", &[stored.as_str()]).finish();
                let t = ns.lookup_kind(ObjectKind::Table, "t").next().unwrap();
                for probe in &words {
                    for quoted in [false, true] {
                        let got = ns.match_child(t, ObjectKind::Column, probe, quoted, &ctx);
                        checked += 1;
                        match got {
                            Lookup::Found(_) => assert!(
                                stored == probe || proven_equal(d, stored, probe),
                                "{d:?}: Found {stored:?} for {probe:?}"
                            ),
                            Lookup::Missing(_) => assert!(
                                !has_non_ascii_cased(probe)
                                    && !has_non_ascii_cased(stored)
                                    && !proven_equal(d, stored, probe),
                                "{d:?}: Missing {stored:?} for {probe:?}"
                            ),
                            Lookup::Unknown(r) => assert_eq!(r, UnknownReason::ServerDependent),
                        }
                    }
                }
            }
        }
        assert!(checked > 100_000, "{checked}");
    }

    /// The facts' own positives are found, both directions, both flavors (MySQL `İ`=`I`=`i`;
    /// `Ä`=`ä`), and MariaDB's negatives for `İ` stay unknown, never missing.
    #[test]
    fn hf2_recorded_positives_and_mariadb_dotted_i() {
        for (d, stored, probe, want_found) in [
            (DialectId::MySql, "İD", "id", true),
            (DialectId::MySql, "ID", "İd", true),
            (DialectId::MySql, "id", "İD", true),
            (DialectId::MySql, "xÄ", "XÄ", true),
            (DialectId::MariaDb, "İd", "id", false),
            (DialectId::MariaDb, "id", "İd", false),
            (DialectId::MariaDb, "İd", "İD", true),
            (DialectId::MySql, "ıd", "ID", false),
        ] {
            let ns = ns_mysql("shop", 0).table("t", &[stored]).finish();
            let t = ns.lookup_kind(ObjectKind::Table, "t").next().unwrap();
            let got = ns.match_child(t, ObjectKind::Column, probe, false, &IdentCtx::new(d));
            if want_found {
                assert!(
                    matches!(got, Lookup::Found(_)),
                    "{d:?} {stored} {probe}: {got:?}"
                );
            } else {
                assert!(
                    matches!(got, Lookup::Unknown(UnknownReason::ServerDependent)),
                    "{d:?} {stored} {probe}: {got:?}"
                );
            }
        }
    }

    /// An unknown `lower_case_table_names` (the hub placeholder before the server answers) and
    /// any lctn ≠ 0 pin no fold: never a fold-made `Found`, never `Missing`.
    #[test]
    fn hf3_no_fold_without_pinned_lctn() {
        for d in [DialectId::MySql, DialectId::MariaDb] {
            for lctn in [1u8, 2, u8::MAX] {
                let ctx = IdentCtx {
                    lower_case_table_names: lctn,
                    ..IdentCtx::new(d)
                };
                let ns = ns_mysql("shop", if lctn == u8::MAX { 0 } else { lctn })
                    .table("t", &["İd", "Äb"])
                    .finish();
                let t = ns.lookup_kind(ObjectKind::Table, "t").next().unwrap();
                for probe in ["id", "ID", "äb", "äB"] {
                    let got = ns.match_child(t, ObjectKind::Column, probe, false, &ctx);
                    assert!(
                        matches!(got, Lookup::Unknown(UnknownReason::ServerDependent)),
                        "{d:?} lctn {lctn} {probe}: {got:?}"
                    );
                }
            }
        }
    }

    /// Only columns are pinned: indexes, routines and triggers under the same probe stay unpinned.
    #[test]
    fn hf4_only_columns_are_pinned() {
        let ns = ns_mysql("shop", 0).table("t", &["İd"]).finish();
        let t = ns.lookup_kind(ObjectKind::Table, "t").next().unwrap();
        let got = ns.match_child(
            t,
            ObjectKind::Index,
            "id",
            false,
            &IdentCtx::new(DialectId::MySql),
        );
        assert!(!matches!(got, Lookup::Found(_)), "{got:?}");
    }
}
