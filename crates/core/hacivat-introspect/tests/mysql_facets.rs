// ## m4-t17b-mysql-facets
//! MySQL and MariaDB facets without a server (M4 Task 17b, R-M4-30): the `20-facets` sections
//! render and pass the metadata connection's guard in every variant (R-W3-9); the pure
//! deciders (grants from the grant tables, the usage-view coverage, view dependencies, roles);
//! and the reads over a scripted [`SectionReader`] that renders every call from the real pack:
//! a refused read is an error, never an empty answer, and an incomplete object is not
//! answered (rulings R-W4-3, R-W4-4).

#![cfg(feature = "session")]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use hacivat_driver::ServerMessage;
use hacivat_introspect::mysql::facets::{
    self, Coverage, GrantTarget, Opaque, account, coverage, decide_deps, decide_relation_grants,
    decide_schema_grants, escaped_schema, grant_target, login_parts, name, read_on, renders,
    roles_result,
};
use hacivat_introspect::mysql::pass::SectionReader;
use hacivat_introspect::mysql::server::ServerCtx;
use hacivat_introspect::pack::{
    BindValue, DialectId, LogicalType, Mode, Pack, PackCtx, Params, lint,
};
use hacivat_introspect::{FacetRows, IntroError};
use hacivat_meta::KindGroup;
use hacivat_meta::merge::{ExistSet, MergeOutcome, MergePolicy, Trigger, base, merge};
use hacivat_meta::name::{FoldRule, IdentCtx, NamePart};
use hacivat_meta::props::{DepsKind, FacetKind, Grant};
use hacivat_meta::snapshot::IntroErrorCode;
use hacivat_meta::snapshot::{Catalog, CatalogChange, ContainerKey, LoadState, ServerFacts};
use hacivat_meta::view::{Lookup, ModelView, SnapshotView};
use hacivat_meta::{Level, Name, NsKey, ObjectKey, ObjectKind, Ref};
use hacivat_proto::DbmsKind;
use hacivat_session::meta::{MetaError, Rows, check_own_text, check_read_text_for};
use smallvec::SmallVec;
use std::time::SystemTime;

type Row = Vec<Option<Arc<str>>>;

fn n(s: &str) -> Name {
    Name::from(s)
}

fn c(s: &str) -> Option<Arc<str>> {
    Some(Arc::from(s))
}

fn row(cells: &[Option<&str>]) -> Row {
    cells.iter().map(|c| c.map(Arc::from)).collect()
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

fn pack() -> Pack {
    Pack::builtin(DialectId::MySql).expect("the MySQL pack parses")
}

fn ns() -> NsKey {
    NsKey::schema(["hvm4_db"])
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

fn table(t: &str) -> ObjectKey {
    key(ObjectKind::Table, &["hvm4_db", t])
}

fn g(grantee: &str, privilege: &str, columns: &[&str], grantable: bool) -> Grant {
    Grant {
        grantee: n(grantee),
        privilege: n(privilege),
        columns: columns.iter().map(|c| n(c)).collect(),
        grantable,
    }
}

fn mysql() -> ServerCtx {
    let mut s = ServerCtx::assumed(DbmsKind::MySql, "8.4.3");
    s.login = Some(n("root@localhost"));
    s
}

fn mariadb() -> ServerCtx {
    let mut s = ServerCtx::assumed(DbmsKind::MariaDb, "11.4.5-MariaDB");
    s.login = Some(n("root@localhost"));
    s
}

fn server_error(code: i64) -> IntroError {
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

// ---- a scripted server -------------------------------------------------------------------------

/// Answers each section from a script (default: no rows), after rendering it from the real
/// pack exactly as `mysql::TxnReader` does (a variant that renders no statement is
/// `Protocol`), checking the width and the metadata guard, and logging the text and binds.
/// One logged read: section, rendered text, bind values.
type Logged = (String, String, Vec<String>);

#[derive(Default)]
struct Script {
    answers: HashMap<&'static str, Result<Vec<Row>, IntroError>>,
    log: Arc<Mutex<Vec<Logged>>>,
}

impl Script {
    fn answer(mut self, section: &'static str, rows: Vec<Row>) -> Self {
        self.answers.insert(section, Ok(rows));
        self
    }

    fn refuse(mut self, section: &'static str, code: i64) -> Self {
        self.answers.insert(section, Err(server_error(code)));
        self
    }

    fn sections(&self) -> Vec<String> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .map(|l| l.0.clone())
            .collect()
    }
}

fn bind_text(v: &BindValue) -> String {
    match v {
        BindValue::Typed { text, .. } => text.to_string(),
        other => format!("{other:?}"),
    }
}

impl SectionReader for Script {
    fn read<'a>(
        &'a mut self,
        section: &'static str,
        ctx: PackCtx,
    ) -> BoxFuture<'a, Result<Rows, IntroError>> {
        Box::pin(async move {
            let p = pack();
            let qs = p
                .render(section, &ctx)
                .map_err(|_| IntroError::Protocol(n(section)))?;
            let kind = if ctx.flavor == DbmsKind::MariaDb {
                DbmsKind::MariaDb
            } else {
                DbmsKind::MySql
            };
            for q in &qs {
                check_own_text(q.sql(), kind).unwrap_or_else(|e| panic!("{section}: {e:?}"));
                self.log.lock().unwrap().push((
                    section.to_string(),
                    q.sql().to_string(),
                    q.binds().iter().map(bind_text).collect(),
                ));
            }
            let columns: Vec<Arc<str>> = p
                .section(section)
                .unwrap()
                .returns()
                .iter()
                .map(|c| c.name.clone())
                .collect();
            let rows = match self.answers.get(section) {
                Some(Ok(rows)) => rows.clone(),
                Some(Err(e)) => return Err(e.clone()),
                None => Vec::new(),
            };
            for r in &rows {
                assert_eq!(r.len(), columns.len(), "{section}: a scripted row's width");
            }
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

// ---- the pack ----------------------------------------------------------------------------------

fn servers() -> Vec<ServerCtx> {
    [
        "8.0.12",
        "8.0.13",
        "8.0.15",
        "8.0.16",
        "8.0.18",
        "8.0.19",
        "8.0.46",
        "8.4.3",
        "9.1.0",
        "10.6.21-MariaDB",
        "11.4.5-MariaDB",
    ]
    .into_iter()
    .map(|v| ServerCtx::assumed(DbmsKind::MySql, v))
    .collect()
}

fn hostile_params() -> Params {
    Params {
        scalars: vec![(n("schema"), text("a'b`c\\d"))],
        lists: vec![(n("names"), vec![text("x' OR '1'='1"), text("İ`; --")])],
    }
}

/// R-W3-9: every facet section, in every variant, renders with hostile binds that never reach
/// the text, passes the guard of both MySQL-family engines, is not joinable, and reads no
/// authentication column.
#[test]
fn every_facet_section_renders_and_passes_the_metadata_guard() {
    let p = pack();
    assert!(lint(&p, DialectId::MySql).is_empty());
    let mut rendered = std::collections::BTreeSet::new();
    for s in name::ALL {
        let sec = p.section(s).expect(s);
        assert!(!sec.joinable(), "{s}");
        for mut ctx in p.reachable_variants(s) {
            ctx.params = hostile_params();
            let Ok(qs) = p.render(s, &ctx) else {
                continue;
            };
            for q in qs {
                rendered.insert(s);
                for other in [DbmsKind::MySql, DbmsKind::MariaDb] {
                    check_own_text(q.sql(), other).unwrap_or_else(|e| panic!("{s}: {e:?}"));
                    check_read_text_for(q.sql(), other).unwrap_or_else(|e| panic!("{s}: {e:?}"));
                }
                for hostile in ["a'b", "x'", "İ", "OR '1'"] {
                    assert!(!q.sql().contains(hostile), "{s}: a bind spliced");
                }
                let low = q.sql().to_ascii_lowercase();
                for banned in [
                    "mysql.user",
                    "global_priv",
                    "authentication_string",
                    "password",
                    "group_concat",
                    "lower(",
                    "upper(",
                    "collate",
                ] {
                    assert!(!low.contains(banned), "{s}: {banned}");
                }
            }
        }
    }
    assert_eq!(
        rendered.len(),
        name::ALL.len(),
        "every section renders somewhere"
    );
}

/// The reads' gate mirror (`renders`) agrees with the pack on every server: a gated section is
/// never read (the reader would answer `Protocol`), and an ungated one is.
#[test]
fn the_gate_mirror_agrees_with_the_pack() {
    let p = pack();
    for srv in servers() {
        for s in name::ALL {
            let ctx = srv.pack_ctx(Mode::Full, Level::Structure, hostile_params());
            let renders_text = p.render(s, &ctx).is_ok_and(|q| !q.is_empty());
            assert_eq!(
                renders(&srv, s),
                renders_text,
                "{s} on {}",
                srv.version_text
            );
        }
    }
}

#[test]
fn the_facet_sections_are_declared_with_their_widths() {
    let p = pack();
    for (s, w) in [
        (name::GRANTS_TABLE, 4),
        (name::GRANTS_COLUMN, 5),
        (name::GRANTS_SCHEMA, 22),
        (name::DEPS_VIEWS, 1),
        (name::DEPS_VIEW_TABLES, 4),
        (name::DEPS_VIEW_ROUTINES, 3),
        (name::DEPS_COVERAGE, 2),
        (name::PARTIAL_REVOKES, 1),
        (name::LIST_ROLES, 2),
    ] {
        assert_eq!(p.section(s).expect(s).returns().len(), w, "{s}");
    }
    assert_eq!(facets::SCHEMA_PRIVILEGES.len() + 3, 22);
}

// ---- accounts ----------------------------------------------------------------------------------

#[test]
fn accounts_are_literals_with_doubled_quotes_and_refuse_what_has_no_portable_form() {
    assert_eq!(account("app", "%", false).as_deref(), Some("'app'@'%'"));
    assert_eq!(
        account("o'brien", "h'x", false).as_deref(),
        Some("'o''brien'@'h''x'")
    );
    assert_eq!(
        account("", "localhost", false).as_deref(),
        Some("''@'localhost'")
    );
    assert_eq!(account("a@b", "%", false).as_deref(), Some("'a@b'@'%'"));
    assert_eq!(account("İ", "%", true).as_deref(), Some("'İ'@'%'"));
    assert_eq!(account("a\\", "%", false), None);
    assert_eq!(account("a", "h\0", false), None);
    // MariaDB keeps roles' rows with an empty host: a role or a user, unknown.
    assert_eq!(account("r", "", true), None);
    assert_eq!(account("r", "", false).as_deref(), Some("'r'@''"));
}

#[test]
fn the_login_splits_only_on_exactly_one_at() {
    assert_eq!(login_parts("hv_test@%"), Some(("hv_test", "%")));
    assert_eq!(login_parts("a@b@c"), None);
    assert_eq!(login_parts("nohost"), None);
    assert_eq!(login_parts("@"), Some(("", "")));
}

// ---- grants -----------------------------------------------------------------------------------

#[test]
fn table_grants_decode_the_set_columns_and_keep_the_grant_option() {
    let keys = [table("t"), table("u")];
    let tables = vec![
        row(&[
            Some("app"),
            Some("%"),
            Some("t"),
            Some("Select,Insert,Grant"),
        ]),
        row(&[
            Some("o'b"),
            Some("h"),
            Some("t"),
            Some("Show view,Create View,Trigger"),
        ]),
        // Another table, and one differing only in case: never `t`'s.
        row(&[Some("x"), Some("%"), Some("u"), Some("Delete")]),
        row(&[Some("x"), Some("%"), Some("T"), Some("Drop")]),
        // Column privileges only: an empty table set with the column flag.
        row(&[Some("col"), Some("%"), Some("t"), Some("")]),
    ];
    let columns = vec![
        row(&[
            Some("col"),
            Some("%"),
            Some("t"),
            Some("b"),
            Some("Select,Update"),
        ]),
        row(&[Some("col"), Some("%"), Some("t"), Some("a"), Some("Select")]),
        row(&[
            Some("app"),
            Some("%"),
            Some("t"),
            Some("c"),
            Some("References"),
        ]),
    ];
    let got = decide_relation_grants(&keys, &tables, &columns, false);
    assert_eq!(got.len(), 2);
    assert!(got.iter().all(|o| o.complete()), "{got:?}");
    assert_eq!(
        got[0].facet.entries,
        vec![
            g("'app'@'%'", "INSERT", &[], true),
            g("'app'@'%'", "REFERENCES", &["c"], true),
            g("'app'@'%'", "SELECT", &[], true),
            g("'col'@'%'", "SELECT", &["a", "b"], false),
            g("'col'@'%'", "UPDATE", &["b"], false),
            g("'o''b'@'h'", "CREATE VIEW", &[], false),
            g("'o''b'@'h'", "SHOW VIEW", &[], false),
            g("'o''b'@'h'", "TRIGGER", &[], false),
        ]
    );
    assert_eq!(
        got[1].facet.entries,
        vec![g("'x'@'%'", "DELETE", &[], false)]
    );
}

#[test]
fn an_incomplete_object_is_marked_and_never_complete() {
    let k = [table("t")];
    let one = |t: Vec<Row>, c: Vec<Row>, maria: bool| {
        decide_relation_grants(&k, &t, &c, maria).remove(0).opaque
    };
    // An element the documentation does not list.
    assert_eq!(
        one(
            vec![row(&[
                Some("a"),
                Some("%"),
                Some("t"),
                Some("Select,Frobnicate")
            ])],
            vec![],
            false
        ),
        vec![Opaque::Privilege("Frobnicate".into())]
    );
    // A grant option with nothing to grant.
    assert_eq!(
        one(
            vec![row(&[Some("a"), Some("%"), Some("t"), Some("Grant")])],
            vec![],
            false
        ),
        vec![Opaque::GrantOptionOnly]
    );
    // A column grant without its table row: its grant option is unknown.
    assert_eq!(
        one(
            vec![],
            vec![row(&[
                Some("a"),
                Some("%"),
                Some("t"),
                Some("c"),
                Some("Select")
            ])],
            false
        ),
        vec![Opaque::NoTableRow]
    );
    // A backslash in an account; a MariaDB role-or-user row.
    assert!(matches!(
        one(
            vec![row(&[Some("a\\"), Some("%"), Some("t"), Some("Select")])],
            vec![],
            false
        )[..],
        [Opaque::Account { .. }]
    ));
    assert!(matches!(
        one(
            vec![row(&[Some("r"), Some(""), Some("t"), Some("Select")])],
            vec![],
            true
        )[..],
        [Opaque::Account { .. }]
    ));
    // MariaDB's history privilege is a documented element.
    let k2 = decide_relation_grants(
        &k,
        &[row(&[
            Some("a"),
            Some("%"),
            Some("t"),
            Some("Delete versioning rows"),
        ])],
        &[],
        true,
    );
    assert!(k2[0].complete());
    assert_eq!(
        k2[0].facet.entries,
        vec![g("'a'@'%'", "DELETE HISTORY", &[], false)]
    );
}

fn db_row(user: &str, db: &str, privs: &[&str]) -> Row {
    let mut r = vec![c(user), c("%"), c(db)];
    for i in 0..19 {
        r.push(c(privs.get(i).copied().unwrap_or("N")));
    }
    r
}

#[test]
fn schema_grants_decode_the_y_n_columns_and_refuse_an_escaped_row() {
    let k = key(ObjectKind::Schema, &["hvm4_db"]);
    let mut y = vec!["N"; 19];
    y[0] = "Y"; // SELECT
    y[6] = "Y"; // Grant_priv
    y[16] = "Y"; // EXECUTE
    let got = decide_schema_grants(&k, "hvm4_db", &[db_row("app", "hvm4_db", &y)]);
    assert!(got.complete());
    assert_eq!(
        got.facet.entries,
        vec![
            g("'app'@'%'", "EXECUTE", &[], true),
            g("'app'@'%'", "SELECT", &[], true)
        ]
    );
    // A row for the escaped spelling grants exactly this schema in a form re-granting
    // `ON hvm4_db.*` would widen: not loaded. A row of another pattern is ignored.
    let got = decide_schema_grants(
        &k,
        "hvm4_db",
        &[
            db_row("app", "hvm4\\_db", &["Y"]),
            db_row("app", "hvm4%", &["Y"]),
        ],
    );
    assert_eq!(got.opaque, vec![Opaque::EscapedSchemaRow]);
    assert!(got.facet.entries.is_empty());
    let mut weird = vec!["N"; 19];
    weird[3] = "maybe";
    let got = decide_schema_grants(&k, "hvm4_db", &[db_row("app", "hvm4_db", &weird)]);
    assert_eq!(got.opaque, vec![Opaque::Privilege("maybe".into())]);
    assert_eq!(escaped_schema("a_b%c\\d"), "a\\_b\\%c\\\\d");
}

#[test]
fn grant_targets_are_the_namespace_relations_and_its_own_schema() {
    let ns = ns();
    assert_eq!(grant_target(&ns, &table("t")), Some(GrantTarget::Relation));
    assert_eq!(
        grant_target(&ns, &key(ObjectKind::View, &["hvm4_db", "v"])),
        Some(GrantTarget::Relation)
    );
    assert_eq!(
        grant_target(&ns, &key(ObjectKind::Schema, &["hvm4_db"])),
        Some(GrantTarget::Schema)
    );
    for k in [
        key(ObjectKind::Table, &["other", "t"]),
        key(ObjectKind::Schema, &["other"]),
        key(ObjectKind::Routine, &["hvm4_db", "f"]),
        key(ObjectKind::Column, &["hvm4_db", "t", "c"]),
    ] {
        assert_eq!(grant_target(&ns, &k), None, "{k:?}");
    }
}

#[test]
fn a_refused_grant_table_is_a_permission_error_never_an_empty_answer() {
    for code in facets::PERMISSION_CODES {
        let mut r = Script::default().refuse(name::GRANTS_TABLE, code);
        let got = block_on(read_on(
            &mut r,
            &mysql(),
            &ns(),
            &[table("t")],
            FacetKind::Grants,
        ));
        assert_eq!(
            got,
            Err(IntroError::Permission {
                kind: ObjectKind::Table
            }),
            "{code}"
        );
        let mut r = Script::default().refuse(name::GRANTS_COLUMN, code);
        let got = block_on(read_on(
            &mut r,
            &mysql(),
            &ns(),
            &[table("t")],
            FacetKind::Grants,
        ));
        assert_eq!(
            got,
            Err(IntroError::Permission {
                kind: ObjectKind::Table
            }),
            "{code}"
        );
        let mut r = Script::default().refuse(name::GRANTS_SCHEMA, code);
        let got = block_on(read_on(
            &mut r,
            &mysql(),
            &ns(),
            &[key(ObjectKind::Schema, &["hvm4_db"])],
            FacetKind::Grants,
        ));
        assert_eq!(
            got,
            Err(IntroError::Permission {
                kind: ObjectKind::Table
            }),
            "{code}"
        );
    }
    // Another server error stays itself.
    let mut r = Script::default().refuse(name::GRANTS_TABLE, 1064);
    let got = block_on(read_on(
        &mut r,
        &mysql(),
        &ns(),
        &[table("t")],
        FacetKind::Grants,
    ));
    assert!(matches!(got, Err(IntroError::Meta(_))), "{got:?}");
}

#[test]
fn a_grants_read_answers_only_complete_objects_with_names_as_binds() {
    let mut r = Script::default()
        .answer(
            name::GRANTS_TABLE,
            vec![
                row(&[Some("a"), Some("%"), Some("t"), Some("Select")]),
                row(&[Some("a"), Some("%"), Some("x'y"), Some("Select,Bogus")]),
            ],
        )
        .answer(name::GRANTS_SCHEMA, vec![db_row("a", "hvm4_db", &["Y"])]);
    let keys = [
        table("t"),
        table("x'y"),
        table("none"),
        key(ObjectKind::Schema, &["hvm4_db"]),
        key(ObjectKind::Routine, &["hvm4_db", "f"]),
    ];
    let got = block_on(read_on(&mut r, &mysql(), &ns(), &keys, FacetKind::Grants)).unwrap();
    let FacetRows::Grants(got) = got else {
        panic!("{got:?}")
    };
    let answered: Vec<&str> = got.iter().map(|(k, _)| k.name()).collect();
    // `x'y` is incomplete (an unknown element), the routine is not covered; `none` has no
    // grants, which is a fact the complete grant tables state.
    assert_eq!(answered, ["t", "none", "hvm4_db"]);
    assert!(got[1].1.entries.is_empty());
    let log = r.log.lock().unwrap().clone();
    let sections: Vec<&str> = log.iter().map(|l| l.0.as_str()).collect();
    assert_eq!(
        sections,
        [name::GRANTS_TABLE, name::GRANTS_COLUMN, name::GRANTS_SCHEMA]
    );
    for (s, sql, binds) in &log {
        assert!(!sql.contains("x'y"), "{s}");
        if s == name::GRANTS_SCHEMA {
            assert_eq!(binds, &["hvm4_db".to_string(), "hvm4\\_db".to_string()]);
        } else {
            assert!(binds.contains(&"x'y".to_string()), "{s}: {binds:?}");
        }
    }
}

#[test]
fn grants_on_mariadb_skip_the_schema_and_an_lctn_server_answers_nothing() {
    let schema = [key(ObjectKind::Schema, &["hvm4_db"])];
    let mut r = Script::default();
    let got = block_on(read_on(
        &mut r,
        &mariadb(),
        &ns(),
        &schema,
        FacetKind::Grants,
    ));
    assert_eq!(got, Ok(FacetRows::Grants(Vec::new())));
    assert!(r.sections().is_empty());
    let mut lctn = mysql();
    lctn.lower_case_table_names = 1;
    let mut r = Script::default();
    let got = block_on(read_on(
        &mut r,
        &lctn,
        &ns(),
        &[table("t")],
        FacetKind::Grants,
    ));
    assert_eq!(got, Ok(FacetRows::Grants(Vec::new())));
    assert!(r.sections().is_empty(), "nothing is read");
}

// ---- dependencies -----------------------------------------------------------------------------

fn privs(grantee: &str, list: &[&str]) -> Vec<Row> {
    list.iter()
        .map(|p| row(&[Some(grantee), Some(p)]))
        .collect()
}

#[test]
fn coverage_needs_global_select_and_execute_for_the_login_and_no_partial_revokes() {
    let full = privs("'root'@'localhost'", &["SELECT", "EXECUTE", "INSERT"]);
    assert_eq!(
        coverage(Some("root@localhost"), &full, None),
        Coverage::Complete
    );
    assert_eq!(
        coverage(Some("root@localhost"), &full, Some("0")),
        Coverage::Complete
    );
    assert!(matches!(
        coverage(Some("root@localhost"), &full, Some("1")),
        Coverage::Incomplete(_)
    ));
    assert!(matches!(
        coverage(Some("root@localhost"), &full, Some("?")),
        Coverage::Incomplete(_)
    ));
    // Only SELECT: a function the login cannot execute would be hidden.
    let select = privs("'root'@'localhost'", &["SELECT"]);
    assert!(matches!(
        coverage(Some("root@localhost"), &select, None),
        Coverage::Incomplete(_)
    ));
    // Another account's rows (a login reading the mysql schema sees every account's).
    let other = privs("'admin'@'%'", &["SELECT", "EXECUTE"]);
    assert!(matches!(
        coverage(Some("root@localhost"), &other, None),
        Coverage::Incomplete(_)
    ));
    // USAGE is no privilege; an ambiguous or unknown login decides nothing.
    let usage = privs("'u'@'%'", &["USAGE"]);
    assert!(matches!(
        coverage(Some("u@%"), &usage, None),
        Coverage::Incomplete(_)
    ));
    assert!(matches!(
        coverage(Some("a@b@c"), &full, None),
        Coverage::Incomplete(_)
    ));
    assert!(matches!(
        coverage(None, &full, None),
        Coverage::Incomplete(_)
    ));
}

fn rref(kind: ObjectKind, s: &str, n_: &str) -> Ref {
    Ref {
        kind,
        path: [n(s), n(n_)].into_iter().collect::<SmallVec<_>>(),
        server_id: None,
    }
}

#[test]
fn view_dependencies_name_tables_views_and_functions_once() {
    let v = key(ObjectKind::View, &["hvm4_db", "v"]);
    let w = key(ObjectKind::View, &["hvm4_db", "w"]);
    let gone = key(ObjectKind::View, &["hvm4_db", "gone"]);
    let views = vec![row(&[Some("v")]), row(&[Some("w")])];
    let tables = vec![
        row(&[Some("v"), Some("hvm4_db"), Some("t"), Some("BASE TABLE")]),
        row(&[Some("v"), Some("hvm4_db"), Some("t"), Some("BASE TABLE")]),
        row(&[Some("v"), Some("other"), Some("v2"), Some("VIEW")]),
        row(&[Some("w"), Some("hvm4_db"), Some("dropped"), None]),
    ];
    let routines = vec![row(&[Some("v"), Some("hvm4_db"), Some("f")])];
    let got = decide_deps(&[v.clone(), w.clone(), gone], &views, &tables, &routines);
    assert_eq!(got.len(), 2, "a view the login cannot see is not answered");
    assert_eq!(got[0].key, v);
    assert!(!got[0].unnamed);
    assert_eq!(got[0].facet.kind, DepsKind::Usage);
    assert_eq!(
        got[0].facet.depends_on,
        vec![
            rref(ObjectKind::Table, "hvm4_db", "t"),
            rref(ObjectKind::View, "other", "v2"),
            rref(ObjectKind::Routine, "hvm4_db", "f"),
        ]
    );
    assert_eq!(got[1].key, w);
    assert!(got[1].unnamed, "a used relation without a TABLES row");
}

#[test]
fn a_dependencies_read_is_complete_or_a_permission_error() {
    let v = key(ObjectKind::View, &["hvm4_db", "v"]);
    let full = privs("'root'@'localhost'", &["SELECT", "EXECUTE"]);
    let script = || {
        Script::default()
            .answer(name::DEPS_COVERAGE, full.clone())
            .answer(name::PARTIAL_REVOKES, vec![row(&[Some("0")])])
            .answer(name::DEPS_VIEWS, vec![row(&[Some("v")])])
            .answer(
                name::DEPS_VIEW_TABLES,
                vec![row(&[
                    Some("v"),
                    Some("hvm4_db"),
                    Some("t"),
                    Some("BASE TABLE"),
                ])],
            )
    };
    let mut r = script();
    let got = block_on(read_on(
        &mut r,
        &mysql(),
        &ns(),
        &[v.clone(), table("t")],
        FacetKind::Dependencies,
    ));
    let Ok(FacetRows::Dependencies(got)) = got else {
        panic!("{got:?}")
    };
    assert_eq!(got.len(), 1, "only views have usage rows");
    assert_eq!(
        got[0].1.depends_on,
        vec![rref(ObjectKind::Table, "hvm4_db", "t")]
    );
    // The login sees only what it has a privilege on: a permission error, never a list that
    // may lack a hidden table.
    let mut r = script().answer(
        name::DEPS_COVERAGE,
        privs("'root'@'localhost'", &["SELECT"]),
    );
    let got = block_on(read_on(
        &mut r,
        &mysql(),
        &ns(),
        std::slice::from_ref(&v),
        FacetKind::Dependencies,
    ));
    assert_eq!(
        got,
        Err(IntroError::Permission {
            kind: ObjectKind::View
        })
    );
    let mut r = script().answer(name::PARTIAL_REVOKES, vec![row(&[Some("1")])]);
    let got = block_on(read_on(
        &mut r,
        &mysql(),
        &ns(),
        std::slice::from_ref(&v),
        FacetKind::Dependencies,
    ));
    assert_eq!(
        got,
        Err(IntroError::Permission {
            kind: ObjectKind::View
        })
    );
    // A refused usage view.
    let mut r = script().refuse(name::DEPS_VIEW_TABLES, 1142);
    let got = block_on(read_on(
        &mut r,
        &mysql(),
        &ns(),
        std::slice::from_ref(&v),
        FacetKind::Dependencies,
    ));
    assert_eq!(
        got,
        Err(IntroError::Permission {
            kind: ObjectKind::View
        })
    );
    // MySQL before 8.0.16 has no partial revokes (nothing read for it).
    let mut old = mysql();
    old.version_num = 80_014;
    let mut r = script();
    assert!(matches!(
        block_on(read_on(&mut r, &old, &ns(), std::slice::from_ref(&v), FacetKind::Dependencies)),
        Ok(FacetRows::Dependencies(d)) if d.len() == 1
    ));
    assert!(!r.sections().contains(&name::PARTIAL_REVOKES.to_string()));
}

#[test]
fn mariadb_and_old_mysql_have_no_dependency_read() {
    let v = [key(ObjectKind::View, &["hvm4_db", "v"])];
    for srv in [mariadb(), {
        let mut s = mysql();
        s.version_num = 80_012;
        s
    }] {
        let mut r = Script::default();
        assert_eq!(
            block_on(read_on(&mut r, &srv, &ns(), &v, FacetKind::Dependencies)),
            Err(IntroError::NotImplemented)
        );
        assert!(r.sections().is_empty(), "nothing is read");
    }
    assert!(!facets::facets_for(DbmsKind::MariaDb).contains(&FacetKind::Dependencies));
    assert!(facets::facets_for(DbmsKind::MySql).contains(&FacetKind::Dependencies));
    let mut r = Script::default();
    assert_eq!(
        block_on(read_on(&mut r, &mysql(), &ns(), &v, FacetKind::Statistics)),
        Err(IntroError::NotImplemented)
    );
}

// ---- sources ----------------------------------------------------------------------------------

#[test]
fn sources_answer_server_text_skip_gone_objects_and_fail_on_a_refusal() {
    let t = table("t");
    let trg = key(ObjectKind::Trigger, &["hvm4_db", "t", "trg"]);
    let col = key(ObjectKind::Column, &["hvm4_db", "t", "c"]);
    let mut r = Script::default().answer(
        "ShowCreateTable",
        vec![row(&[Some("t"), Some("CREATE TABLE `t` (a int)")])],
    );
    let got = block_on(read_on(
        &mut r,
        &mysql(),
        &ns(),
        &[t.clone(), col],
        FacetKind::Sources,
    ));
    assert_eq!(
        got,
        Ok(FacetRows::Sources(vec![(
            t.clone(),
            n("CREATE TABLE `t` (a int)")
        )]))
    );
    // A trigger is named in its schema; a dropped one is skipped.
    let mut r = Script::default().refuse("ShowCreateTrigger", 1360);
    let got = block_on(read_on(
        &mut r,
        &mysql(),
        &ns(),
        std::slice::from_ref(&trg),
        FacetKind::Sources,
    ));
    assert_eq!(got, Ok(FacetRows::Sources(Vec::new())));
    let log = r.log.lock().unwrap().clone();
    assert!(log[0].1.ends_with("`hvm4_db`.`trg`"), "{}", log[0].1);
    let mut r = Script::default().refuse("ShowCreateTable", 1142);
    let got = block_on(read_on(&mut r, &mysql(), &ns(), &[t], FacetKind::Sources));
    assert_eq!(
        got,
        Err(IntroError::Permission {
            kind: ObjectKind::Table
        })
    );
}

// ---- roles ------------------------------------------------------------------------------------

/// A catalog holding only `server` (the server namespace), loaded at its level, as the
/// engine publishes a pass.
fn server_view(server: hacivat_meta::NamespaceSnapshot) -> SnapshotView {
    let catalog = Catalog::new(Arc::new(ServerFacts::new(
        DbmsKind::MySql,
        IdentCtx::new(DialectId::MySql),
    )));
    let level = server.meta().level;
    catalog
        .publisher()
        .unwrap()
        .publish(vec![
            CatalogChange::SetState(
                ContainerKey::Namespace(NsKey::server()),
                LoadState::Loaded {
                    level,
                    at: SystemTime::UNIX_EPOCH,
                    watermark: hacivat_meta::Watermark::None,
                    stale: false,
                },
            ),
            CatalogChange::Put(Arc::new(server)),
        ])
        .unwrap();
    SnapshotView::new(catalog.current())
}

fn account_lookup(v: &SnapshotView, name: &str) -> Lookup<()> {
    match v.lookup(
        &[NamePart {
            text: n(name),
            quoted: true,
        }],
        KindGroup::Accounts,
    ) {
        Lookup::Found(_) => Lookup::Found(()),
        Lookup::Missing(r) => Lookup::Missing(r),
        Lookup::Unknown(r) => Lookup::Unknown(r),
    }
}

#[test]
fn roles_are_the_applicable_roles_and_the_login_at_names_level() {
    let rows = vec![
        row(&[Some("app_read"), Some("%")]),
        row(&[Some("app_read"), Some("%")]),
        row(&[Some("o'r"), Some("h")]),
        row(&[Some("bad\\"), Some("%")]),
    ];
    let r = roles_result(Some(&rows), Some("hv_test@%"), false, 7);
    assert_eq!(r.level, Level::Names);
    assert!(r.full);
    let names: Vec<(ObjectKind, String)> = r
        .changed
        .iter()
        .map(|c| {
            (
                c.subtree.root_obj().kind(),
                c.subtree.root_obj().name().to_string(),
            )
        })
        .collect();
    assert_eq!(
        names,
        vec![
            (ObjectKind::Role, "'app_read'@'%'".to_string()),
            (ObjectKind::Role, "'o''r'@'h'".to_string()),
            (ObjectKind::User, "'hv_test'@'%'".to_string()),
        ]
    );
    assert!(matches!(&r.existent[&ObjectKind::Role], ExistSet::Ok(ids) if ids.len() == 2));
    // MariaDB roles have no host: the bare name.
    let maria = roles_result(Some(&[row(&[Some("editor"), None])]), Some("u@%"), true, 0);
    assert_eq!(
        maria.changed[0].subtree.root_obj().name().as_ref(),
        "editor"
    );
    // No APPLICABLE_ROLES (MySQL before 8.0.19): the role set failed, never empty.
    let none = roles_result(None, Some("u@%"), false, 0);
    assert_eq!(
        none.existent[&ObjectKind::Role],
        ExistSet::Failed(IntroErrorCode::NotImplemented)
    );
    // The roles merge into the server namespace.
    let b = base(NsKey::server(), DbmsKind::MySql, FoldRule::Exact).unwrap();
    let policy = MergePolicy {
        trigger: Trigger::User,
        ..MergePolicy::default()
    };
    let next = match merge(Some(&b), r, &policy).expect("merge") {
        MergeOutcome::Applied { next, .. } => next,
        other => panic!("{other:?}"),
    };
    let listed: Vec<String> = next
        .children(0)
        .map(|(_, o)| o.name().to_string())
        .collect();
    assert_eq!(listed.len(), 3, "{listed:?}");
    assert_eq!(next.meta().level, Level::Names);
    // Published as the engine does (loaded at `Names`): a listed account is found, and one
    // the result does not list is unknown, never missing (it is not every account).
    let v = server_view(next);
    assert!(matches!(
        account_lookup(&v, "'hv_test'@'%'"),
        Lookup::Found(_)
    ));
    assert!(matches!(
        account_lookup(&v, "'app_read'@'%'"),
        Lookup::Found(_)
    ));
    assert!(matches!(
        account_lookup(&v, "'someone_else'@'%'"),
        Lookup::Unknown(_)
    ));
}

#[test]
fn a_refused_role_list_is_a_permission_error_and_old_mysql_reads_nothing() {
    let mut r = Script::default().refuse(name::LIST_ROLES, 1142);
    let got = block_on(facets::roles(&mut r, &mysql()));
    assert!(matches!(
        got,
        Err(IntroError::Permission {
            kind: ObjectKind::Role
        })
    ));
    let mut old = mysql();
    old.version_num = 80_018;
    let mut r = Script::default();
    let got = block_on(facets::roles(&mut r, &old)).unwrap();
    assert!(r.sections().is_empty());
    assert!(matches!(
        got.existent[&ObjectKind::Role],
        ExistSet::Failed(_)
    ));
}
