// ## m4-t17-mysql
//! The MySQL and MariaDB introspector without a server (M4 Task 17): the pack sections render
//! and pass the metadata connection's token guard in every variant; the text rules (ENUM
//! labels, pre-quoted defaults, time markers); and the passes driven over an in-memory server
//! ([`Fake`], a `SectionReader` that answers each section from a small catalog and logs every
//! read), so the R-M4-39 marker rule, the existence sets and "incremental equals full" are
//! checked here as well as on Docker (`docker_mysql_intro.rs`).

#![cfg(feature = "session")]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use futures::future::BoxFuture;
use hacivat_introspect::mysql::build::table_stamp;
use hacivat_introspect::mysql::pass::{
    self, PassStats, SectionReader, in_window, read_server, reread, run_fragment, run_pass,
};
use hacivat_introspect::mysql::rows::{default_sql, enum_labels, parse_extra, parse_time_micros};
use hacivat_introspect::mysql::server::ServerCtx;
use hacivat_introspect::mysql::show_create::section_of;
use hacivat_introspect::mysql::state::{self, ns_check, top_marks_of};
use hacivat_introspect::pack::{
    BindValue, DialectId, LogicalType, Mode, Pack, PackCtx, PackFlags, Params, lint,
};
use hacivat_introspect::{IntroError, introspector_for};
use hacivat_meta::merge::{ExistSet, MergePolicy, NamespaceResult, base, merge};
use hacivat_meta::name::{FoldRule, Name};
use hacivat_meta::props::{ColumnExt, Identity as ColIdentity, Props};
use hacivat_meta::{
    Level, NamespaceSnapshot, NsKey, ObjFlags, ObjectKey, ObjectKind, StateMark, Watermark,
};
use hacivat_proto::DbmsKind;
use hacivat_session::meta::{Rows, check_own_text, check_read_text_for};
use smallvec::smallvec;
use sql_ident::quote::QuoteCtx;

// ---- helpers ---------------------------------------------------------------------------------

fn n(s: &str) -> Name {
    Arc::from(s)
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

fn bind_text(v: &BindValue) -> String {
    match v {
        BindValue::Typed { text, .. } => text.to_string(),
        other => panic!("not a text bind: {other:?}"),
    }
}

fn pack() -> Pack {
    Pack::builtin(DialectId::MySql).expect("the MySQL pack parses")
}

/// The binds every section may name, with plain values.
fn params_for_any() -> Params {
    Params {
        scalars: ["schema", "name", "name_like", "like_escape"]
            .into_iter()
            .map(|b| (n(b), text("hvm4_x")))
            .collect(),
        lists: vec![(n("names"), vec![text("a"), text("b")])],
    }
}

// ---- the pack ----------------------------------------------------------------------------------

#[test]
fn every_section_renders_and_passes_the_metadata_guard_in_every_variant() {
    let p = pack();
    assert!(lint(&p, DialectId::MySql).is_empty());
    let mut rendered = 0;
    for s in p.sections() {
        let sec = p.section(s).expect("declared");
        for mut ctx in p.reachable_variants(s) {
            ctx.params = params_for_any();
            match p.render(s, &ctx) {
                Ok(qs) => {
                    for q in qs {
                        rendered += 1;
                        let kind = if ctx.flavor == DbmsKind::MariaDb {
                            DbmsKind::MariaDb
                        } else {
                            DbmsKind::MySql
                        };
                        check_own_text(q.sql(), kind)
                            .unwrap_or_else(|e| panic!("{s} {ctx:?}: {e:?}\n{}", q.sql()));
                        if sec.joinable() {
                            check_read_text_for(q.sql(), kind)
                                .unwrap_or_else(|e| panic!("joinable {s}: {e:?}"));
                        }
                        // No `GROUP_CONCAT`, no `UPDATE_TIME`, no case folding (HV-INTRO-07,
                        // -14), and the marker is never filtered in SQL (R-M4-39).
                        let up = q.sql().to_ascii_uppercase();
                        for banned in [
                            "GROUP_CONCAT",
                            "UPDATE_TIME",
                            "LOWER(",
                            "UPPER(",
                            "COLLATE",
                            "CREATE_TIME >",
                            "CREATED >",
                            "LAST_ALTERED >",
                        ] {
                            assert!(!up.contains(banned), "{s}: {banned}");
                        }
                    }
                }
                Err(e) => {
                    // Only the deliberately gated sections have a variant with no statement.
                    assert!(
                        matches!(s.as_ref(), "ServerUuid" | "ViewUsage" | "CheckConstraints"),
                        "{s} {ctx:?}: {e:?}"
                    );
                }
            }
        }
    }
    assert!(rendered > 60, "{rendered}");
}

#[test]
fn the_sections_t17_owns_are_declared_with_their_widths() {
    let p = pack();
    for (s, w) in [
        ("Identity", 10),
        ("Tables", 8),
        ("Views", 7),
        ("Columns", 12),
        ("Statistics", 11),
        ("TableConstraints", 4),
        ("KeyColumnUsage", 7),
        ("ReferentialConstraints", 8),
        ("CheckConstraints", 3),
        ("Routines", 10),
        ("Parameters", 6),
        ("Triggers", 7),
        ("Events", 10),
        ("StateOfRelation", 6),
        ("SystemColumns", 5),
        ("ListSettings", 3),
    ] {
        assert_eq!(p.section(s).expect(s).returns().len(), w, "{s}");
    }
    for k in [
        ObjectKind::Table,
        ObjectKind::View,
        ObjectKind::Procedure,
        ObjectKind::Routine,
        ObjectKind::Trigger,
        ObjectKind::Event,
    ] {
        let (s, w, at) = section_of(k).expect("a show-create section");
        let sec = p.section(s).expect(s);
        assert!(sec.show_create(), "{s}");
        assert_eq!(sec.returns().len(), w, "{s}");
        assert_eq!(&*sec.returns()[at].name, "create_statement", "{s}");
    }
    for s in [
        "StateOfRelation",
        "StateOfRoutine",
        "StateOfTrigger",
        "StateOfEvent",
    ] {
        assert!(p.section(s).expect(s).joinable(), "{s}");
    }
}

/// R-M4-37: hostile names render as one backtick-quoted identifier each, and the text passes
/// the metadata connection's guard (a non-ASCII quoted name in a `SHOW CREATE` is a name, not
/// a lookalike function: T17 named exception in Task 5's guard).
#[test]
fn show_create_sections_quote_hostile_names_and_pass_the_guard() {
    let p = pack();
    for name in ["a`b", "İ", "t ", "a:x?b", "ıd", "sleep", "x`--"] {
        for k in [ObjectKind::Table, ObjectKind::View, ObjectKind::Trigger] {
            let (s, _, _) = section_of(k).unwrap();
            let ctx = PackCtx {
                dialect: DialectId::MySql,
                version: 80_046,
                flavor: DbmsKind::MySql,
                mode: Mode::Full,
                level: Level::Sources,
                flags: PackFlags::empty(),
                params: Params {
                    scalars: vec![(n("schema"), text("hvm4_db")), (n("name"), text(name))],
                    lists: Vec::new(),
                },
            };
            let q = p.render(s, &ctx).expect("renders").remove(0);
            let quoted = format!("`{}`", name.replace('`', "``"));
            assert!(
                q.sql().ends_with(&format!("`hvm4_db`.{quoted}")),
                "{}",
                q.sql()
            );
            assert!(q.binds().is_empty());
            check_own_text(q.sql(), DbmsKind::MySql)
                .unwrap_or_else(|e| panic!("{name:?}: {e:?} {}", q.sql()));
        }
    }
    let ctx = PackCtx {
        dialect: DialectId::MySql,
        version: 80_046,
        flavor: DbmsKind::MySql,
        mode: Mode::Full,
        level: Level::Sources,
        flags: PackFlags::empty(),
        params: Params {
            scalars: vec![(n("schema"), text("db")), (n("name"), text("a\0b"))],
            lists: Vec::new(),
        },
    };
    assert!(p.render("ShowCreateTable", &ctx).is_err(), "NUL is refused");
    // A `;` inside a name renders quoted, but the connection's guard refuses any text holding
    // one: such an object has no server text, and nothing is sent (fail closed).
    let mut ctx = ctx;
    ctx.params.scalars[1].1 = text("x`; DROP TABLE t; --");
    let q = p.render("ShowCreateTable", &ctx).unwrap().remove(0);
    assert_eq!(q.sql(), "SHOW CREATE TABLE `db`.`x``; DROP TABLE t; --`");
    assert!(check_own_text(q.sql(), DbmsKind::MySql).is_err());
}

/// The guard exception is narrow: outside `SHOW CREATE`, a non-ASCII or deny-listed quoted
/// name is still refused, and inside it a quoted name called as a function is too.
#[test]
fn the_show_create_guard_exception_is_narrow() {
    for refused in [
        "SELECT `İ` FROM t",
        "SELECT `sleep`(1)",
        "SHOW CREATE TABLE `db`.`sleep`(1)",
        "SHOW TABLES WHERE `nеxtval` = 1",
        "SHOW CREATE TABLE `db`.`t` INTO x",
        "SHOW CREATE TABLE `db`.`t`; SELECT 1",
    ] {
        assert!(
            check_own_text(refused, DbmsKind::MySql).is_err(),
            "{refused}"
        );
    }
    for allowed in [
        "SHOW CREATE TABLE `db`.`İ`",
        "SHOW CREATE TRIGGER `db`.`update`",
        "SHOW CREATE VIEW `ş`.`ıd`",
    ] {
        assert_eq!(
            check_own_text(allowed, DbmsKind::MySql),
            Ok(()),
            "{allowed}"
        );
        assert_eq!(
            check_own_text(allowed, DbmsKind::MariaDb),
            Ok(()),
            "{allowed}"
        );
    }
}

// ---- the text rules --------------------------------------------------------------------------

#[test]
fn mysql_enum_labels_parse_doubled_quotes_and_backslashes_per_sql_mode() {
    assert_eq!(
        enum_labels("enum('a','b''c','')", true),
        Some(vec![n("a"), n("b'c"), n("")])
    );
    assert_eq!(
        enum_labels("set('x,y','z')", true),
        Some(vec![n("x,y"), n("z")])
    );
    // A backslash escapes under the default sql_mode, and is a character under
    // NO_BACKSLASH_ESCAPES.
    assert_eq!(
        enum_labels(r"enum('a\\b','c\'d')", true),
        Some(vec![n(r"a\b"), n("c'd")])
    );
    assert_eq!(enum_labels(r"enum('a\\b')", false), Some(vec![n(r"a\\b")]));
    assert_eq!(
        enum_labels("ENUM('İ','ı')", true),
        Some(vec![n("İ"), n("ı")])
    );
    for bad in [
        "int(11)",
        "enum(a)",
        "enum('a'",
        "enum('a') x",
        "varchar(10)",
    ] {
        assert_eq!(enum_labels(bad, true), None, "{bad}");
    }
}

#[test]
fn defaults_arrive_pre_quoted_and_keep_each_flavors_spelling() {
    let q = QuoteCtx {
        no_backslash_escapes: Some(false),
        ..QuoteCtx::default()
    };
    let none = parse_extra("");
    let gen_ = parse_extra("DEFAULT_GENERATED");
    // F9: MySQL `eu` vs MariaDB `'eu'`; both arrive as SQL text `'eu'`.
    assert_eq!(
        default_sql(false, Some("eu"), "varchar", &none, &q),
        Some(n("'eu'"))
    );
    assert_eq!(
        default_sql(true, Some("'eu'"), "varchar", &none, &q),
        Some(n("'eu'"))
    );
    // `CURRENT_TIMESTAMP` (MySQL) vs `current_timestamp()` (MariaDB), verbatim.
    assert_eq!(
        default_sql(false, Some("CURRENT_TIMESTAMP"), "timestamp", &gen_, &q),
        Some(n("CURRENT_TIMESTAMP"))
    );
    assert_eq!(
        default_sql(true, Some("current_timestamp()"), "timestamp", &none, &q),
        Some(n("current_timestamp()"))
    );
    // An expression default is parenthesized; a number is quoted as SHOW CREATE does.
    assert_eq!(
        default_sql(false, Some("rand()"), "double", &gen_, &q),
        Some(n("(rand())"))
    );
    assert_eq!(
        default_sql(false, Some("0"), "int", &none, &q),
        Some(n("'0'"))
    );
    assert_eq!(
        default_sql(false, Some("b'101'"), "bit", &none, &q),
        Some(n("b'101'"))
    );
    assert_eq!(
        default_sql(false, Some(""), "char", &none, &q),
        Some(n("''"))
    );
    assert_eq!(default_sql(false, None, "int", &none, &q), None);
    // A quote is doubled and a backslash doubled unless NO_BACKSLASH_ESCAPES is on.
    assert_eq!(
        default_sql(false, Some(r"it's\x"), "varchar", &none, &q),
        Some(n(r"'it''s\\x'"))
    );
    let nbe = QuoteCtx {
        no_backslash_escapes: Some(true),
        ..QuoteCtx::default()
    };
    assert_eq!(
        default_sql(false, Some(r"a\b"), "varchar", &none, &nbe),
        Some(n(r"'a\b'"))
    );
}

#[test]
fn the_marker_rule_is_inequality_or_the_one_second_window() {
    let w = parse_time_micros("2026-11-20 10:00:10").unwrap();
    let old = StateMark::MyTable {
        created: parse_time_micros("2026-11-20 09:00:00").unwrap(),
        trigger_max: 0,
        trigger_count: 0,
    };
    // Unchanged and old: not re-read.
    assert!(!reread(Some(&old), &old, true, w));
    // A long ALTER stamped with its start (F1): earlier than W, but different.
    let started = StateMark::MyTable {
        created: parse_time_micros("2026-11-20 09:59:00").unwrap(),
        trigger_max: 0,
        trigger_count: 0,
    };
    assert!(reread(Some(&old), &started, true, w));
    // The same mark inside `W − 1 s` (a second DDL in the same second): re-read.
    let edge = StateMark::MyTable {
        created: parse_time_micros("2026-11-20 10:00:09").unwrap(),
        trigger_max: 0,
        trigger_count: 0,
    };
    assert!(in_window(&edge, w) && reread(Some(&edge), &edge, true, w));
    let before = StateMark::Modified(parse_time_micros("2026-11-20 10:00:08.999999").unwrap());
    assert!(!in_window(&before, w));
    // New, or no time to compare: re-read.
    assert!(reread(None, &old, true, w));
    assert!(reread(Some(&old), &old, false, w));
    // A trigger created in the window moves nothing on CREATE_TIME, but is caught.
    let trig = table_stamp(
        Some(&n("2026-11-20 09:00:00")),
        Some(&(parse_time_micros("2026-11-20 10:00:09.5").unwrap(), 1)),
    );
    assert!(in_window(&trig, w));
}

// ---- the in-memory server ------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Col {
    name: String,
    ty: String,
    default: Option<String>,
    extra: String,
}

#[derive(Clone, Debug)]
struct Fk {
    name: String,
    col: String,
    target: String,
    target_col: String,
}

#[derive(Clone, Debug)]
struct Rel {
    ty: &'static str,
    created: Option<String>,
    comment: String,
    cols: Vec<Col>,
    /// `(index, columns, unique)`; `PRIMARY` is the primary key.
    idx: Vec<(String, Vec<String>, bool)>,
    fks: Vec<Fk>,
    checks: Vec<(String, String)>,
    /// A view: its hash and the relations it reads.
    view: Option<(u64, Vec<String>)>,
}

/// `(specific name, type, last altered, params)`.
type FakeRoutine = (String, &'static str, String, Vec<(String, String)>);

#[derive(Clone, Debug)]
struct Fake {
    flavor: DbmsKind,
    version: &'static str,
    uuid: &'static str,
    now: String,
    schema: String,
    privs: Vec<[String; 6]>,
    rels: BTreeMap<String, Rel>,
    /// `(trigger, table, created)`.
    triggers: Vec<(String, String, String)>,
    /// `(specific name, type, last altered, params)`.
    routines: Vec<FakeRoutine>,
    events: Vec<(String, String)>,
    /// `(section, list length)` of every read.
    log: Vec<(String, Option<usize>)>,
}

fn col(name: &str, ty: &str) -> Col {
    Col {
        name: name.into(),
        ty: ty.into(),
        default: None,
        extra: String::new(),
    }
}

fn table(created: &str, cols: Vec<Col>) -> Rel {
    let pk = cols[0].name.clone();
    Rel {
        ty: "BASE TABLE",
        created: Some(created.into()),
        comment: String::new(),
        cols,
        idx: vec![("PRIMARY".into(), vec![pk], true)],
        fks: Vec::new(),
        checks: Vec::new(),
        view: None,
    }
}

fn view(hash: u64, uses: &[&str], cols: Vec<Col>) -> Rel {
    Rel {
        ty: "VIEW",
        created: None,
        comment: "VIEW".into(),
        cols,
        idx: Vec::new(),
        fks: Vec::new(),
        checks: Vec::new(),
        view: Some((hash, uses.iter().map(|s| s.to_string()).collect())),
    }
}

type Row = Vec<Option<Arc<str>>>;

fn c(s: &str) -> Option<Arc<str>> {
    Some(Arc::from(s))
}

impl Fake {
    fn mysql() -> Fake {
        let mut rels = BTreeMap::new();
        rels.insert(
            "customers".to_string(),
            table(
                "2026-11-20 09:00:00",
                vec![col("id", "int"), col("name", "varchar(64)")],
            ),
        );
        let mut orders = table(
            "2026-11-20 09:00:01",
            vec![
                col("id", "int"),
                col("customer_id", "int"),
                Col {
                    name: "status".into(),
                    ty: "enum('new','done')".into(),
                    default: Some("new".into()),
                    extra: String::new(),
                },
                Col {
                    name: "at".into(),
                    ty: "timestamp".into(),
                    default: Some("CURRENT_TIMESTAMP".into()),
                    extra: "DEFAULT_GENERATED on update CURRENT_TIMESTAMP".into(),
                },
            ],
        );
        orders
            .idx
            .push(("fk_cust".into(), vec!["customer_id".into()], false));
        orders.fks.push(Fk {
            name: "fk_cust".into(),
            col: "customer_id".into(),
            target: "customers".into(),
            target_col: "id".into(),
        });
        orders.checks.push(("chk_id".into(), "(`id` > 0)".into()));
        rels.insert("orders".to_string(), orders);
        rels.insert(
            "v_orders".to_string(),
            view(7, &["orders"], vec![col("id", "int")]),
        );
        Fake {
            flavor: DbmsKind::MySql,
            version: "8.4.3",
            uuid: "uuid-1",
            now: "2026-11-20 10:00:00.000000".into(),
            schema: "shop".into(),
            privs: vec![[
                "user".into(),
                "'hv'@'%'".into(),
                String::new(),
                String::new(),
                "USAGE".into(),
                "NO".into(),
            ]],
            rels,
            triggers: vec![(
                "trg_orders".into(),
                "orders".into(),
                "2026-11-20 09:00:02.00".into(),
            )],
            routines: vec![
                (
                    "add_one".into(),
                    "FUNCTION",
                    "2026-11-20 09:00:03".into(),
                    vec![("x".into(), "int".into())],
                ),
                (
                    "add_one".into(),
                    "PROCEDURE",
                    "2026-11-20 09:00:03".into(),
                    Vec::new(),
                ),
            ],
            events: vec![("nightly".into(), "2026-11-20 09:00:04".into())],
            log: Vec::new(),
        }
    }

    fn mariadb() -> Fake {
        let mut f = Fake::mysql();
        f.flavor = DbmsKind::MariaDb;
        f.version = "11.4.5-MariaDB-ubu2404";
        f.rels.insert(
            "seq".into(),
            Rel {
                ty: "SEQUENCE",
                created: Some("2026-11-20 09:00:05".into()),
                comment: String::new(),
                cols: vec![col("next_not_cached_value", "bigint(21)")],
                idx: Vec::new(),
                fks: Vec::new(),
                checks: Vec::new(),
                view: None,
            },
        );
        f
    }

    /// Advances the server clock (every DDL below happens "now").
    fn at(&mut self, t: &str) {
        self.now = t.into();
    }

    fn sections_read(&self) -> Vec<String> {
        self.log.iter().map(|(s, _)| s.clone()).collect()
    }

    fn hash_hex(h: u64) -> String {
        format!("{h:016x}{}", "0".repeat(48))
    }

    fn answer(&self, section: &str, ctx: &PackCtx) -> Vec<Row> {
        let list: Option<BTreeSet<String>> = ctx
            .params
            .lists
            .iter()
            .find(|(n, _)| &**n == "names")
            .map(|(_, v)| v.iter().map(bind_text).collect());
        let schema = ctx
            .params
            .scalars
            .iter()
            .find(|(n, _)| &**n == "schema")
            .map(|(_, v)| bind_text(v));
        if schema.as_deref().is_some_and(|s| s != self.schema) {
            return Vec::new();
        }
        let keep = |name: &str| list.as_ref().is_none_or(|l| l.contains(name));
        let mariadb = self.flavor == DbmsKind::MariaDb;
        let mut out: Vec<Row> = Vec::new();
        match section {
            "Identity" => out.push(vec![
                c(self.version),
                c("0"),
                c("STRICT_TRANS_TABLES"),
                c("utf8mb3"),
                c("utf8mb4"),
                c("utf8mb4"),
                c("utf8mb4"),
                c("+00:00"),
                c("hv@%"),
                c("NONE"),
            ]),
            "ServerUuid" => out.push(vec![c(self.uuid)]),
            "ServerIdentity" => out.push(vec![c("1"), c("db"), c("3306")]),
            "Watermark" => out.push(vec![c(&self.now)]),
            "Privileges" => {
                for p in &self.privs {
                    out.push(p.iter().map(|s| c(s)).collect());
                }
            }
            "ListNamespaces" => {
                out.push(vec![c(&self.schema), c("utf8mb4"), c("utf8mb4_0900_ai_ci")])
            }
            "TableMarks" | "Tables" => {
                for (name, r) in &self.rels {
                    if !keep(name) {
                        continue;
                    }
                    let created = r.created.as_deref().and_then(c);
                    if section == "TableMarks" {
                        out.push(vec![c(name), c(r.ty), created]);
                    } else {
                        let is_view = r.ty == "VIEW";
                        out.push(vec![
                            c(name),
                            c(r.ty),
                            if is_view { None } else { c("InnoDB") },
                            if is_view {
                                None
                            } else {
                                c("utf8mb4_0900_ai_ci")
                            },
                            c(""),
                            c(&r.comment),
                            created,
                            if is_view { None } else { c("0") },
                        ]);
                    }
                }
            }
            "ViewMarks" | "Views" => {
                for (name, r) in &self.rels {
                    let Some((h, _)) = &r.view else { continue };
                    if !keep(name) {
                        continue;
                    }
                    if section == "ViewMarks" {
                        out.push(vec![c(name), c(&Fake::hash_hex(*h))]);
                    } else {
                        out.push(vec![
                            c(name),
                            c("NONE"),
                            c("YES"),
                            c("DEFINER"),
                            c("root@%"),
                            if mariadb { c("UNDEFINED") } else { None },
                            c(&Fake::hash_hex(*h)),
                        ]);
                    }
                }
            }
            "ViewUsage" => {
                for (name, r) in &self.rels {
                    if let Some((_, uses)) = &r.view {
                        for u in uses {
                            out.push(vec![c(name), c(&self.schema), c(u)]);
                        }
                    }
                }
            }
            "Columns" => {
                for (name, r) in &self.rels {
                    if !keep(name) {
                        continue;
                    }
                    for (i, k) in r.cols.iter().enumerate() {
                        let ord = (i + 1).to_string();
                        let base = k.ty.split('(').next().unwrap_or("").to_string();
                        out.push(vec![
                            c(name),
                            c(&k.name),
                            c(&ord),
                            k.default.as_deref().and_then(c),
                            c("YES"),
                            c(&base),
                            c(&k.ty),
                            None,
                            None,
                            c(&k.extra),
                            c(""),
                            c(""),
                        ]);
                    }
                }
            }
            "Statistics" => {
                for (name, r) in &self.rels {
                    if !keep(name) {
                        continue;
                    }
                    for (ix, cols, unique) in &r.idx {
                        for (i, k) in cols.iter().enumerate() {
                            let seq = (i + 1).to_string();
                            out.push(vec![
                                c(name),
                                c(ix),
                                c(if *unique { "0" } else { "1" }),
                                c(&seq),
                                c(k),
                                c("A"),
                                None,
                                c("BTREE"),
                                c(""),
                                c("YES"),
                                None,
                            ]);
                        }
                    }
                }
            }
            "TableConstraints" => {
                for (name, r) in &self.rels {
                    if !keep(name) {
                        continue;
                    }
                    for (ix, _, unique) in &r.idx {
                        if *unique {
                            let ty = if ix == "PRIMARY" {
                                "PRIMARY KEY"
                            } else {
                                "UNIQUE"
                            };
                            out.push(vec![c(name), c(ix), c(ty), c("YES")]);
                        }
                    }
                    for f in &r.fks {
                        out.push(vec![c(name), c(&f.name), c("FOREIGN KEY"), c("YES")]);
                    }
                    for (ch, _) in &r.checks {
                        out.push(vec![c(name), c(ch), c("CHECK"), c("YES")]);
                    }
                }
            }
            "KeyColumnUsage" => {
                for (name, r) in &self.rels {
                    if !keep(name) {
                        continue;
                    }
                    for f in &r.fks {
                        out.push(vec![
                            c(name),
                            c(&f.name),
                            c(&f.col),
                            c("1"),
                            c(&self.schema),
                            c(&f.target),
                            c(&f.target_col),
                        ]);
                    }
                }
            }
            "ReferentialConstraints" => {
                for (name, r) in &self.rels {
                    if !keep(name) {
                        continue;
                    }
                    for f in &r.fks {
                        out.push(vec![
                            c(name),
                            c(&f.name),
                            c(&self.schema),
                            c(&f.target),
                            c("PRIMARY"),
                            c("NONE"),
                            c(if mariadb { "RESTRICT" } else { "NO ACTION" }),
                            c("CASCADE"),
                        ]);
                    }
                }
            }
            "CheckConstraints" => {
                for (name, r) in &self.rels {
                    if !keep(name) {
                        continue;
                    }
                    for (ch, clause) in &r.checks {
                        out.push(vec![c(name), c(ch), c(clause)]);
                    }
                }
            }
            "TriggerMarks" | "Triggers" => {
                for (t, tab, created) in &self.triggers {
                    if !keep(tab) {
                        continue;
                    }
                    if section == "TriggerMarks" {
                        out.push(vec![c(t), c(tab), c(created)]);
                    } else {
                        out.push(vec![
                            c(t),
                            c(tab),
                            c("BEFORE"),
                            c("INSERT"),
                            c("1"),
                            c(created),
                            c("root@%"),
                        ]);
                    }
                }
            }
            "RoutineMarks" | "Routines" => {
                for (spec, ty, last, _) in &self.routines {
                    if !keep(spec) {
                        continue;
                    }
                    if section == "RoutineMarks" {
                        out.push(vec![c(spec), c(ty), c(last)]);
                    } else {
                        out.push(vec![
                            c(spec),
                            c(spec),
                            c(ty),
                            if *ty == "FUNCTION" { c("int") } else { None },
                            c("YES"),
                            c("CONTAINS SQL"),
                            c("DEFINER"),
                            c("root@%"),
                            c(""),
                            c(last),
                        ]);
                    }
                }
            }
            "Parameters" => {
                for (spec, ty, _, ps) in &self.routines {
                    if !keep(spec) {
                        continue;
                    }
                    for (i, (pn, pt)) in ps.iter().enumerate() {
                        let ord = (i + 1).to_string();
                        out.push(vec![c(spec), c(ty), c(&ord), c("IN"), c(pn), c(pt)]);
                    }
                }
            }
            "EventMarks" | "Events" => {
                for (e, last) in &self.events {
                    if !keep(e) {
                        continue;
                    }
                    if section == "EventMarks" {
                        out.push(vec![c(e), c(last)]);
                    } else {
                        out.push(vec![
                            c(e),
                            c("RECURRING"),
                            None,
                            c("1"),
                            c("DAY"),
                            c("ENABLED"),
                            c("PRESERVE"),
                            c(""),
                            c("root@%"),
                            c(last),
                        ]);
                    }
                }
            }
            "StateOfRelation" => {
                for (name, r) in &self.rels {
                    if !keep(name) {
                        continue;
                    }
                    let hash = r.view.as_ref().map(|(h, _)| Fake::hash_hex(*h));
                    let base = |t: Option<&(String, String, String)>| -> Row {
                        vec![
                            c(name),
                            c(r.ty),
                            r.created.as_deref().and_then(c),
                            hash.as_deref().and_then(c),
                            t.and_then(|t| c(&t.0)),
                            t.and_then(|t| c(&t.2)),
                        ]
                    };
                    let trigs: Vec<_> = self.triggers.iter().filter(|t| &t.1 == name).collect();
                    if trigs.is_empty() {
                        out.push(base(None));
                    }
                    for t in trigs {
                        out.push(base(Some(t)));
                    }
                }
            }
            "TableNamesLike" => {
                let pat = ctx
                    .params
                    .scalars
                    .iter()
                    .find(|(n, _)| &**n == "name_like")
                    .map(|(_, v)| bind_text(v))
                    .expect("a pattern");
                for (name, r) in &self.rels {
                    if like(name, &pat) {
                        out.push(vec![c(name), c(r.ty)]);
                    }
                }
            }
            "StateOfRoutine" => {
                for (spec, ty, last, _) in &self.routines {
                    if keep(spec) {
                        out.push(vec![c(spec), c(ty), c(last)]);
                    }
                }
            }
            "StateOfTrigger" => {
                for (t, tab, created) in &self.triggers {
                    if keep(t) {
                        out.push(vec![c(t), c(tab), c(created)]);
                    }
                }
            }
            "StateOfEvent" => {
                for (e, last) in &self.events {
                    if keep(e) {
                        out.push(vec![c(e), c(last)]);
                    }
                }
            }
            other => panic!("the fake does not answer {other}"),
        }
        out
    }
}

/// SQL `LIKE` with `ESCAPE '\\'`, exact (binary) comparison.
fn like(s: &str, p: &str) -> bool {
    let (s, p): (Vec<char>, Vec<char>) = (s.chars().collect(), p.chars().collect());
    fn go(s: &[char], p: &[char]) -> bool {
        match p.first() {
            None => s.is_empty(),
            Some('%') => (0..=s.len()).any(|i| go(&s[i..], &p[1..])),
            Some('_') => !s.is_empty() && go(&s[1..], &p[1..]),
            Some('\\') if p.len() > 1 => s.first() == Some(&p[1]) && go(&s[1..], &p[2..]),
            Some(c) => s.first() == Some(c) && go(&s[1..], &p[1..]),
        }
    }
    go(&s, &p)
}

impl SectionReader for Fake {
    fn read<'a>(
        &'a mut self,
        section: &'static str,
        ctx: PackCtx,
    ) -> BoxFuture<'a, Result<Rows, IntroError>> {
        Box::pin(async move {
            // Every read the pass asks for must render from the real pack.
            let qs = pack()
                .render(section, &ctx)
                .unwrap_or_else(|e| panic!("{section}: {e:?}"));
            let list = ctx
                .params
                .lists
                .iter()
                .find(|(n, _)| &**n == "names")
                .map(|(_, v)| v.len());
            self.log.push((section.to_string(), list));
            if qs.is_empty() {
                // An empty list renders no statement: nothing reaches the server.
                return Ok(Rows::default());
            }
            let width = pack().section(section).unwrap().returns().len();
            let rows = self.answer(section, &ctx);
            assert!(rows.iter().all(|r| r.len() == width), "{section} width");
            Ok(Rows {
                columns: Vec::new(),
                rows,
            })
        })
    }
}

fn ns() -> NsKey {
    NsKey::schema(["shop"])
}

/// The canonical content of a namespace: every object's key with its kind data, sorted.
fn canon(s: &NamespaceSnapshot) -> Vec<(ObjectKey, StateMark, ObjFlags, Props)> {
    let mut v: Vec<_> = (0..s.len() as u32)
        .map(|i| {
            let o = s.obj(i).unwrap();
            (
                s.object_key(i).unwrap(),
                o.state(),
                o.flags(),
                o.props().clone(),
            )
        })
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

async fn pass_on(
    f: &mut Fake,
    mode: Mode,
    prev: Option<&NamespaceSnapshot>,
) -> (NamespaceSnapshot, NamespaceResult, PassStats) {
    f.log.clear();
    let srv = read_server(f, DbmsKind::MySql).await.expect("identity");
    let (res, stats) = run_pass(f, &srv, &ns(), mode, Level::Structure, prev)
        .await
        .expect("the pass");
    let b = match prev {
        Some(p) => p.clone(),
        None => base(ns(), srv.flavor, FoldRule::Exact).unwrap(),
    };
    let next = merge(Some(&b), res.clone(), &MergePolicy::default())
        .expect("merges")
        .next()
        .clone();
    (next, res, stats)
}

/// An incremental pass on `f` over `prev` equals a fresh full pass (the G4 property).
async fn incremental_equals_full(
    f: &mut Fake,
    prev: &NamespaceSnapshot,
) -> (NamespaceSnapshot, PassStats) {
    let (inc, _, stats) = pass_on(f, Mode::Incremental, Some(prev)).await;
    let inc_log = std::mem::take(&mut f.log);
    let (full, _, _) = pass_on(f, Mode::Full, None).await;
    assert_eq!(canon(&inc), canon(&full), "incremental == full");
    // The log is the incremental pass's.
    f.log = inc_log;
    (inc, stats)
}

fn names(v: &[Name]) -> Vec<&str> {
    v.iter().map(|n| &**n).collect()
}

// ---- passes ----------------------------------------------------------------------------------

#[tokio::test]
async fn a_full_pass_builds_every_kind_with_its_marks() {
    let mut f = Fake::mysql();
    let (snap, res, stats) = pass_on(&mut f, Mode::Full, None).await;
    assert_eq!(stats.full_reason, Some("requested"));
    assert!(res.full);
    // Identity, ServerUuid are read before the pass; the pass is the watermark, the privilege
    // hash and 12 bulk sections (13 with the watermark), never one query per object.
    assert_eq!(
        f.sections_read(),
        [
            "Identity",
            "ServerUuid",
            "Watermark",
            "Privileges",
            "Tables",
            "Views",
            "Triggers",
            "Routines",
            "Events",
            "Columns",
            "Parameters",
            "Statistics",
            "TableConstraints",
            "KeyColumnUsage",
            "ReferentialConstraints",
            "CheckConstraints",
        ]
    );
    let Watermark::MyTime {
        at_utc_micros,
        server_uuid,
        version,
        ..
    } = &snap.meta().watermark
    else {
        panic!("a MySQL watermark");
    };
    assert_eq!(
        Some(*at_utc_micros),
        parse_time_micros("2026-11-20 10:00:00")
    );
    assert_eq!((&**server_uuid, &**version), ("uuid-1", "8.4.3"));
    let key = |kind, path: &[&str]| ObjectKey {
        kind,
        path: path.iter().map(|s| n(s)).collect(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    };
    let orders = snap
        .find(&key(ObjectKind::Table, &["shop", "orders"]))
        .unwrap();
    assert_eq!(
        snap.obj(orders).unwrap().state(),
        StateMark::MyTable {
            created: parse_time_micros("2026-11-20 09:00:01").unwrap(),
            trigger_max: parse_time_micros("2026-11-20 09:00:02").unwrap(),
            trigger_count: 1,
        }
    );
    let kinds: BTreeSet<ObjectKind> = snap.children(orders).map(|(_, o)| o.kind()).collect();
    assert_eq!(
        kinds,
        [
            ObjectKind::Column,
            ObjectKind::Index,
            ObjectKind::PrimaryKey,
            ObjectKind::ForeignKey,
            ObjectKind::Check,
            ObjectKind::Trigger
        ]
        .into_iter()
        .collect()
    );
    // ENUM labels, pre-quoted default, verbatim EXTRA.
    let status = snap
        .find(&key(ObjectKind::Column, &["shop", "orders", "status"]))
        .unwrap();
    let Props::Column(cp) = snap.obj(status).unwrap().props() else {
        panic!()
    };
    assert_eq!(cp.default_raw.as_deref(), Some("'new'"));
    assert_eq!(
        cp.rare.as_ref().and_then(|r| r.enum_labels.as_deref()),
        Some(&[n("new"), n("done")][..])
    );
    let at = snap
        .find(&key(ObjectKind::Column, &["shop", "orders", "at"]))
        .unwrap();
    let Props::Column(cp) = snap.obj(at).unwrap().props() else {
        panic!()
    };
    assert_eq!(cp.default_raw.as_deref(), Some("CURRENT_TIMESTAMP"));
    assert!(matches!(
        cp.ext.as_deref(),
        Some(ColumnExt::MySql { extra: Some(e), .. }) if &**e == "DEFAULT_GENERATED on update CURRENT_TIMESTAMP"
    ));
    // A function and a procedure share a name (identity `(ROUTINE_TYPE, SPECIFIC_NAME)`).
    assert!(
        snap.find(&key(ObjectKind::Routine, &["shop", "add_one"]))
            .is_some()
    );
    assert!(
        snap.find(&key(ObjectKind::Procedure, &["shop", "add_one"]))
            .is_some()
    );
    assert!(
        snap.find(&key(ObjectKind::Argument, &["shop", "add_one", "x"]))
            .is_some()
    );
    let v = snap
        .find(&key(ObjectKind::View, &["shop", "v_orders"]))
        .unwrap();
    assert_eq!(snap.obj(v).unwrap().state(), StateMark::Hash(7));
    assert!(
        snap.find(&key(ObjectKind::Event, &["shop", "nightly"]))
            .is_some()
    );
    // The FK points at the stored table.
    let fk = snap
        .find(&key(ObjectKind::ForeignKey, &["shop", "orders", "fk_cust"]))
        .unwrap();
    let Props::ForeignKey(fp) = snap.obj(fk).unwrap().props() else {
        panic!()
    };
    assert_eq!(names(&fp.target.path), ["shop", "customers"]);
    assert!(!snap.obj(fk).unwrap().flags().contains(ObjFlags::BROKEN));
}

#[tokio::test]
async fn insert_only_rereads_no_table() {
    let mut f = Fake::mysql();
    let (s0, _, _) = pass_on(&mut f, Mode::Full, None).await;
    // An INSERT moves no marker; the next pass is 10 s later.
    f.at("2026-11-20 10:00:10.000000");
    let (s1, stats) = incremental_equals_full(&mut f, &s0).await;
    assert_eq!(stats.full_reason, None);
    assert!(stats.relations.is_empty() && stats.routines.is_empty() && stats.events.is_empty());
    assert_eq!(canon(&s1), canon(&s0));
}

#[tokio::test]
async fn instant_add_column_and_add_index_reread_only_that_table() {
    let mut f = Fake::mysql();
    let (s0, _, _) = pass_on(&mut f, Mode::Full, None).await;
    f.at("2026-11-20 10:00:20.000000");
    let c = f.rels.get_mut("customers").unwrap();
    c.cols.push(col("email", "varchar(128)"));
    c.idx.push(("ix_email".into(), vec!["email".into()], false));
    c.created = Some("2026-11-20 10:00:15".into());
    let (s1, stats) = incremental_equals_full(&mut f, &s0).await;
    // The FK of `orders` references `customers`: re-read with it.
    assert_eq!(names(&stats.relations), ["customers", "orders"]);
    assert!(s1.len() > s0.len());
    // Each re-read section carried the list, never the whole schema.
    for (s, list) in &f.log {
        if ["Tables", "Columns", "Statistics"].contains(&s.as_str()) {
            assert_eq!(*list, Some(2), "{s}");
        }
    }
}

#[tokio::test]
async fn create_trigger_found_although_create_time_unchanged() {
    let mut f = Fake::mysql();
    let (s0, _, _) = pass_on(&mut f, Mode::Full, None).await;
    f.at("2026-11-20 10:01:00.000000");
    f.triggers.push((
        "trg_c".into(),
        "customers".into(),
        "2026-11-20 10:00:30.00".into(),
    ));
    let (s1, stats) = incremental_equals_full(&mut f, &s0).await;
    assert!(stats.relations.contains(&n("customers")));
    // A dropped trigger moves the count: the table is re-read and the trigger is gone.
    f.at("2026-11-20 10:02:00.000000");
    f.triggers.retain(|t| t.0 != "trg_c");
    let (_, stats) = incremental_equals_full(&mut f, &s1).await;
    assert!(stats.relations.contains(&n("customers")));
}

#[tokio::test]
async fn ddl_in_the_same_second_as_the_watermark_is_caught() {
    let mut f = Fake::mysql();
    // A DDL at 10:00:00.2, the pass at 10:00:00.5 reads its stamp `10:00:00`.
    f.rels.get_mut("customers").unwrap().created = Some("2026-11-20 10:00:00".into());
    f.at("2026-11-20 10:00:00.500000");
    let (s0, _, _) = pass_on(&mut f, Mode::Full, None).await;
    // A second DDL at 10:00:00.8 keeps the stamp `10:00:00`: the marker is equal, but in
    // `W − 1 s`, so the next pass re-reads it.
    f.rels
        .get_mut("customers")
        .unwrap()
        .cols
        .push(col("late", "int"));
    f.at("2026-11-20 10:00:30.000000");
    let (s1, stats) = incremental_equals_full(&mut f, &s0).await;
    assert!(stats.relations.contains(&n("customers")));
    // Outside the window, nothing is re-read again.
    f.at("2026-11-20 10:01:00.000000");
    let (_, stats) = incremental_equals_full(&mut f, &s1).await;
    assert!(stats.relations.is_empty());
}

#[tokio::test]
async fn long_alter_stamped_with_its_start_is_seen() {
    let mut f = Fake::mysql();
    f.at("2026-11-20 10:00:05.000000");
    let (s0, _, _) = pass_on(&mut f, Mode::Full, None).await;
    // F1: MySQL stamps an ALTER that started at 10:00:03 (before W) and ended after the pass.
    let c = f.rels.get_mut("customers").unwrap();
    c.created = Some("2026-11-20 10:00:03".into());
    c.cols.push(col("copied", "int"));
    f.at("2026-11-20 10:05:00.000000");
    let (_, stats) = incremental_equals_full(&mut f, &s0).await;
    assert!(stats.relations.contains(&n("customers")));
}

#[tokio::test]
async fn create_or_replace_view_rehashes_and_rereads_columns() {
    let mut f = Fake::mysql();
    let (s0, _, _) = pass_on(&mut f, Mode::Full, None).await;
    f.at("2026-11-20 10:10:00.000000");
    let v = f.rels.get_mut("v_orders").unwrap();
    v.view = Some((8, vec!["orders".into()]));
    v.cols.push(col("customer_id", "int"));
    let (s1, stats) = incremental_equals_full(&mut f, &s0).await;
    assert_eq!(names(&stats.relations), ["v_orders"]);
    // A base table change re-reads the view over it (VIEW_TABLE_USAGE).
    f.at("2026-11-20 10:20:00.000000");
    let o = f.rels.get_mut("orders").unwrap();
    o.created = Some("2026-11-20 10:15:00".into());
    o.cols[0].ty = "bigint".into();
    let (_, stats) = incremental_equals_full(&mut f, &s1).await;
    assert_eq!(names(&stats.relations), ["orders", "v_orders"]);
}

#[tokio::test]
async fn drops_are_existence_diffs_and_routines_and_events_follow_last_altered() {
    let mut f = Fake::mysql();
    let (s0, _, _) = pass_on(&mut f, Mode::Full, None).await;
    f.at("2026-11-20 11:00:00.000000");
    f.events.clear();
    f.routines.retain(|r| r.1 != "PROCEDURE");
    f.routines[0].2 = "2026-11-20 10:30:00".into();
    f.routines[0].3.push(("y".into(), "int".into()));
    let (_, stats) = incremental_equals_full(&mut f, &s0).await;
    assert_eq!(names(&stats.routines), ["add_one"]);
    assert!(stats.relations.is_empty() && stats.events.is_empty());
}

#[tokio::test]
async fn server_identity_privileges_or_level_changes_make_the_pass_full() {
    let mut f = Fake::mysql();
    let (s0, _, _) = pass_on(&mut f, Mode::Full, None).await;
    f.at("2026-11-20 10:00:10.000000");
    f.uuid = "uuid-2";
    let (_, _, stats) = pass_on(&mut f, Mode::Incremental, Some(&s0)).await;
    assert_eq!(stats.full_reason, Some("server identity"));
    f.uuid = "uuid-1";
    f.privs[0][4] = "SELECT".into();
    let (_, _, stats) = pass_on(&mut f, Mode::Incremental, Some(&s0)).await;
    assert_eq!(stats.full_reason, Some("privileges"));
    let (_, _, stats) = pass_on(&mut f, Mode::Incremental, None).await;
    assert_eq!(stats.full_reason, Some("no previous snapshot"));
}

#[tokio::test]
async fn mariadb_full_per_namespace_after_each_alter() {
    let mut f = Fake::mariadb();
    let (s0, res, _) = pass_on(&mut f, Mode::Full, None).await;
    assert_eq!(res.new_watermark, Watermark::None);
    // R-M4-10: an incremental request is a full pass on MariaDB.
    f.at("2026-11-20 10:00:10.000000");
    f.rels
        .get_mut("customers")
        .unwrap()
        .cols
        .push(col("z", "int(11)"));
    let (s1, res, stats) = pass_on(&mut f, Mode::Incremental, Some(&s0)).await;
    assert!(res.full);
    assert_eq!(stats.full_reason, Some("flavor"));
    // No ServerUuid, no privilege hash on MariaDB.
    assert!(
        !f.sections_read()
            .iter()
            .any(|s| s == "ServerUuid" || s == "Privileges")
    );
    // The sequence is a Sequence with no children; the FK rule is kept as MariaDB reports it.
    let seq = s1
        .find(&ObjectKey {
            kind: ObjectKind::Sequence,
            path: smallvec![n("shop"), n("seq")],
            server_id: None,
            overload_sig: None,
            raw_kind: None,
        })
        .expect("a MariaDB sequence");
    assert_eq!(s1.children(seq).count(), 0);
    let (full, _, _) = pass_on(&mut f, Mode::Full, None).await;
    assert_eq!(canon(&s1), canon(&full));
}

#[tokio::test]
async fn mariadb_rows_without_a_listed_parent_are_skipped() {
    let mut f = Fake::mariadb();
    let srv = read_server(&mut f, DbmsKind::MariaDb).await.unwrap();
    // A reader that answers COLUMNS for a table TABLES did not list (no consistent snapshot).
    struct Racing(Fake);
    impl SectionReader for Racing {
        fn read<'a>(
            &'a mut self,
            section: &'static str,
            ctx: PackCtx,
        ) -> BoxFuture<'a, Result<Rows, IntroError>> {
            Box::pin(async move {
                let mut rows = self.0.read(section, ctx).await?;
                if section == "Columns" {
                    let mut ghost = rows.rows[0].clone();
                    ghost[0] = c("created_after_tables");
                    rows.rows.push(ghost);
                }
                if section == "Statistics" {
                    let mut ghost = rows.rows[0].clone();
                    ghost[0] = c("created_after_tables");
                    rows.rows.push(ghost);
                }
                Ok(rows)
            })
        }
    }
    let mut r = Racing(f.clone());
    let (res, _) = run_pass(&mut r, &srv, &ns(), Mode::Full, Level::Structure, None)
        .await
        .unwrap();
    let b = base(ns(), DbmsKind::MariaDb, FoldRule::Exact).unwrap();
    let next = merge(Some(&b), res, &MergePolicy::default()).unwrap();
    assert!(
        next.next()
            .iter_named(&[ObjectKind::Table])
            .all(|(_, o)| &**o.name() != "created_after_tables")
    );
    let (full, _, _) = pass_on(&mut f, Mode::Full, None).await;
    assert_eq!(canon(next.next()), canon(&full));
}

#[tokio::test]
async fn a_fragment_rereads_the_named_objects_with_scoped_existence() {
    let mut f = Fake::mysql();
    let (s0, _, _) = pass_on(&mut f, Mode::Full, None).await;
    let srv = read_server(&mut f, DbmsKind::MySql).await.unwrap();
    f.rels.get_mut("customers").unwrap().cols.pop();
    let keys = [ObjectKey {
        kind: ObjectKind::Column,
        path: smallvec![n("shop"), n("customers"), n("name")],
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    }];
    f.log.clear();
    let (res, stats) = run_fragment(&mut f, &srv, &ns(), &keys, Level::Structure)
        .await
        .unwrap();
    assert_eq!(names(&stats.relations), ["customers"]);
    assert_eq!(res.new_watermark, Watermark::None);
    assert!(!res.full);
    match &res.existent[&ObjectKind::Column] {
        ExistSet::Scoped { scope, ids } => {
            assert_eq!(scope, &vec![vec![n("customers")]]);
            assert_eq!(ids.len(), 1);
        }
        other => panic!("{other:?}"),
    }
    let next = merge(Some(&s0), res, &MergePolicy::default()).unwrap();
    assert_eq!(
        next.next().len() + 1,
        s0.len(),
        "the dropped column is gone"
    );
}

#[tokio::test]
async fn stamps_answer_raw_marks_and_none_for_gone_objects() {
    let mut f = Fake::mysql();
    let srv = read_server(&mut f, DbmsKind::MySql).await.unwrap();
    let k = |kind, path: &[&str]| ObjectKey {
        kind,
        path: path.iter().map(|s| n(s)).collect(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    };
    let keys = [
        k(ObjectKind::Table, &["shop", "orders"]),
        k(ObjectKind::Column, &["shop", "orders", "id"]),
        k(ObjectKind::View, &["shop", "v_orders"]),
        k(ObjectKind::Table, &["shop", "v_orders"]),
        k(ObjectKind::Procedure, &["shop", "add_one"]),
        k(ObjectKind::Trigger, &["shop", "orders", "trg_orders"]),
        k(ObjectKind::Event, &["shop", "nightly"]),
        k(ObjectKind::Table, &["shop", "missing"]),
        k(ObjectKind::Table, &["other", "orders"]),
        k(ObjectKind::Domain, &["shop", "d"]),
    ];
    let got = state::stamps(&mut f, &srv, &keys).await.unwrap();
    let t = |s| parse_time_micros(s).unwrap();
    let orders = StateMark::MyTable {
        created: t("2026-11-20 09:00:01"),
        trigger_max: t("2026-11-20 09:00:02"),
        trigger_count: 1,
    };
    assert_eq!(
        got,
        vec![
            Some(orders),
            Some(orders),
            Some(StateMark::Hash(7)),
            None,
            Some(StateMark::Modified(t("2026-11-20 09:00:03"))),
            Some(StateMark::Created(t("2026-11-20 09:00:02"))),
            Some(StateMark::Modified(t("2026-11-20 09:00:04"))),
            None,
            None,
            None,
        ]
    );
    // One list query per kind and schema, never one per key: four kinds in `shop`, the
    // relation of `other`.
    assert_eq!(
        f.log
            .iter()
            .filter(|(s, _)| s.starts_with("StateOf"))
            .count(),
        5
    );
}

#[tokio::test]
async fn the_namespace_check_agrees_with_the_cached_namespace() {
    let mut f = Fake::mysql();
    let (s0, _, _) = pass_on(&mut f, Mode::Full, None).await;
    let srv = read_server(&mut f, DbmsKind::MySql).await.unwrap();
    let server = ns_check(
        &state::server_top_marks(&mut f, &srv, &n("shop"))
            .await
            .unwrap(),
    );
    assert_eq!(server, ns_check(&top_marks_of(&s0)));
    assert_eq!(server.objects, 6);
    f.triggers.clear();
    let after = ns_check(
        &state::server_top_marks(&mut f, &srv, &n("shop"))
            .await
            .unwrap(),
    );
    assert_ne!(after.existence_digest, server.existence_digest);
}

#[tokio::test]
async fn a_routine_argument_keeps_its_mode_and_type() {
    let mut f = Fake::mysql();
    let (snap, _, _) = pass_on(&mut f, Mode::Full, None).await;
    let (_, arg) = snap
        .iter_named(&[ObjectKind::Argument])
        .next()
        .expect("an argument");
    let Props::Column(cp) = arg.props() else {
        panic!()
    };
    assert_eq!((&*cp.raw_type, cp.ordinal), ("int", 1));
    assert!(matches!(
        cp.ext.as_deref(),
        Some(ColumnExt::MySql { extra: Some(m), .. }) if &**m == "IN"
    ));
    assert_eq!(cp.identity(), None::<ColIdentity>);
}

#[tokio::test]
async fn the_server_context_takes_the_flavor_from_version() {
    let mut f = Fake::mariadb();
    // The profile says MySQL; the server is MariaDB.
    let srv = read_server(&mut f, DbmsKind::MySql).await.unwrap();
    assert_eq!(srv.flavor, DbmsKind::MariaDb);
    assert_eq!(srv.version_num, 110_405);
    assert!(srv.utf8mb4 && srv.utc_session);
    assert_eq!(srv.server_uuid, None);
    let my = read_server(&mut Fake::mysql(), DbmsKind::MariaDb)
        .await
        .unwrap();
    assert_eq!(my.flavor, DbmsKind::MySql);
    assert_eq!(my.server_uuid.as_deref(), Some("uuid-1"));
    assert_eq!(
        ServerCtx::assumed(DbmsKind::MySql, "8.4.3").version_num,
        80_403
    );
}

#[test]
fn capabilities_follow_r_m4_10() {
    let my = introspector_for(DbmsKind::MySql).unwrap().capabilities();
    let maria = introspector_for(DbmsKind::MariaDb).unwrap().capabilities();
    assert!(my.incremental && !my.transactional_ddl);
    assert!(!maria.incremental && !maria.transactional_ddl);
    let _ = pass::INCREMENTAL_LIMIT;
}

#[tokio::test]
async fn filter_pushdown_reads_matching_relation_names_through_like_escape() {
    use hacivat_introspect::filter::{FilterRole, parse_filter};
    let mut f = Fake::mysql();
    f.rels.insert(
        "ord_x".into(),
        table("2026-11-20 09:00:00", vec![col("id", "int")]),
    );
    let srv = read_server(&mut f, DbmsKind::MySql).await.unwrap();
    let filter = parse_filter("ord*||v_*", DialectId::MySql, FilterRole::Introspect)
        .unwrap()
        .with_ident(srv.ident());
    f.log.clear();
    let got = pass::pushdown_relations(&mut f, &srv, "shop", &filter)
        .await
        .unwrap()
        .expect("a kindless pattern pushes for every relation kind");
    assert_eq!(
        got.iter().map(|n| &**n).collect::<Vec<_>>(),
        ["ord_x", "orders", "v_orders"]
    );
    // One query per pattern (R-W2-10); the `_` of `v_` travels escaped.
    assert_eq!(f.sections_read(), ["TableNamesLike", "TableNamesLike"]);
    // A filter that names only tables cannot be pushed into `TABLES` (views would be cut).
    let tables_only = parse_filter("table:ord*", DialectId::MySql, FilterRole::Introspect)
        .unwrap()
        .with_ident(srv.ident());
    assert_eq!(
        pass::pushdown_relations(&mut f, &srv, "shop", &tables_only)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn the_mariadb_fingerprint_reads_its_host_facts() {
    use hacivat_introspect::mysql::server::fingerprint;
    let mut f = Fake::mariadb();
    let srv = read_server(&mut f, DbmsKind::MariaDb).await.unwrap();
    let host = pass::read_host_facts(&mut f, &srv).await.unwrap();
    assert_eq!(host.port.as_deref(), Some("3306"));
    let a = fingerprint(&srv, &host).expect("MariaDB host facts");
    let mut my = Fake::mysql();
    let srv = read_server(&mut my, DbmsKind::MySql).await.unwrap();
    let b = fingerprint(&srv, &Default::default()).expect("MySQL server_uuid");
    assert_ne!(a, b);
}
