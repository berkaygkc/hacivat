// ## m4-t17-mysql
//! The in-memory MySQL server of the T17 tests (`mysql_rows.rs`, `mysql_intro_hostile.rs`): a
//! [`SectionReader`] that answers each pack section from a small catalog, renders every read
//! from the real pack (asserting the declared width) and logs `(section, list length)`; and the
//! pass helpers (`pass_on`, `incremental_equals_full`, `canon`).

#![allow(dead_code, reason = "each test file uses a subset of the fake")]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use futures::future::BoxFuture;
use hacivat_introspect::IntroError;
use hacivat_introspect::mysql::pass::{PassStats, SectionReader, read_server, run_pass};
use hacivat_introspect::pack::{BindValue, DialectId, LogicalType, Mode, Pack, PackCtx, Params};
use hacivat_meta::merge::{MergePolicy, NamespaceResult, base, merge};
use hacivat_meta::name::{FoldRule, Name};
use hacivat_meta::props::Props;
use hacivat_meta::{Level, NamespaceSnapshot, NsKey, ObjFlags, ObjectKey, StateMark};
use hacivat_proto::DbmsKind;
use hacivat_session::meta::Rows;

pub fn n(s: &str) -> Name {
    Arc::from(s)
}

pub fn text(s: &str) -> BindValue {
    BindValue::Typed {
        hint: LogicalType::Text {
            max_chars: None,
            fixed: false,
        },
        text: Arc::from(s),
    }
}

pub fn bind_text(v: &BindValue) -> String {
    match v {
        BindValue::Typed { text, .. } => text.to_string(),
        other => panic!("not a text bind: {other:?}"),
    }
}

pub fn pack() -> Pack {
    Pack::builtin(DialectId::MySql).expect("the MySQL pack parses")
}

/// The binds every section may name, with plain values.
pub fn params_for_any() -> Params {
    Params {
        scalars: ["schema", "name", "name_like", "like_escape"]
            .into_iter()
            .map(|b| (n(b), text("hvm4_x")))
            .collect(),
        lists: vec![(n("names"), vec![text("a"), text("b")])],
    }
}

#[derive(Clone, Debug)]
pub struct Col {
    pub name: String,
    pub ty: String,
    pub default: Option<String>,
    pub extra: String,
}

#[derive(Clone, Debug)]
pub struct Fk {
    pub name: String,
    pub col: String,
    pub target: String,
    pub target_col: String,
}

#[derive(Clone, Debug)]
pub struct Rel {
    pub ty: &'static str,
    pub created: Option<String>,
    pub comment: String,
    pub cols: Vec<Col>,
    /// `(index, columns, unique)`; `PRIMARY` is the primary key.
    pub idx: Vec<(String, Vec<String>, bool)>,
    pub fks: Vec<Fk>,
    pub checks: Vec<(String, String)>,
    /// A view: its hash and the relations it reads.
    pub view: Option<(u64, Vec<String>)>,
}

/// `(specific name, type, last altered, params)`.
pub type FakeRoutine = (String, &'static str, String, Vec<(String, String)>);

#[derive(Clone, Debug)]
pub struct Fake {
    pub flavor: DbmsKind,
    pub version: &'static str,
    pub uuid: &'static str,
    pub now: String,
    pub schema: String,
    pub privs: Vec<[String; 6]>,
    pub rels: BTreeMap<String, Rel>,
    /// `(trigger, table, created)`.
    pub triggers: Vec<(String, String, String)>,
    /// `(specific name, type, last altered, params)`.
    pub routines: Vec<FakeRoutine>,
    pub events: Vec<(String, String)>,
    /// Views whose definition this login cannot read (no SHOW VIEW; review I-1).
    pub hidden: BTreeSet<String>,
    /// `(section, list length)` of every read.
    pub log: Vec<(String, Option<usize>)>,
}

pub fn col(name: &str, ty: &str) -> Col {
    Col {
        name: name.into(),
        ty: ty.into(),
        default: None,
        extra: String::new(),
    }
}

pub fn table(created: &str, cols: Vec<Col>) -> Rel {
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

pub fn view(hash: u64, uses: &[&str], cols: Vec<Col>) -> Rel {
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

pub type Row = Vec<Option<Arc<str>>>;

pub fn c(s: &str) -> Option<Arc<str>> {
    Some(Arc::from(s))
}

impl Fake {
    pub fn mysql() -> Fake {
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
            hidden: BTreeSet::new(),
            log: Vec::new(),
        }
    }

    pub fn mariadb() -> Fake {
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
    pub fn at(&mut self, t: &str) {
        self.now = t.into();
    }

    pub fn sections_read(&self) -> Vec<String> {
        self.log.iter().map(|(s, _)| s.clone()).collect()
    }

    /// A view's definition hash and `def_visible`: a hidden definition (no SHOW VIEW) hashes
    /// as empty text, whatever the view is.
    pub fn view_def(&self, name: &str, h: u64) -> (String, &'static str) {
        if self.hidden.contains(name) {
            (Fake::hash_hex(0), "NO")
        } else {
            (Fake::hash_hex(h), "YES")
        }
    }

    pub fn hash_hex(h: u64) -> String {
        format!("{h:016x}{}", "0".repeat(48))
    }

    pub fn answer(&self, section: &str, ctx: &PackCtx) -> Vec<Row> {
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
                    let (hash, visible) = self.view_def(name, *h);
                    if section == "ViewMarks" {
                        out.push(vec![c(name), c(&hash), c(visible)]);
                    } else {
                        out.push(vec![
                            c(name),
                            c("NONE"),
                            c("YES"),
                            c("DEFINER"),
                            c("root@%"),
                            if mariadb { c("UNDEFINED") } else { None },
                            c(&hash),
                            c(visible),
                        ]);
                    }
                }
            }
            "ViewColumnMarks" => {
                for (name, r) in &self.rels {
                    if r.view.is_none() || !keep(name) {
                        continue;
                    }
                    for (i, k) in r.cols.iter().enumerate() {
                        let ord = (i + 1).to_string();
                        out.push(vec![
                            c(name),
                            c(&k.name),
                            c(&ord),
                            c(&k.ty),
                            c("YES"),
                            k.default.as_deref().and_then(c),
                            None,
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
                    let def = r.view.as_ref().map(|(h, _)| self.view_def(name, *h));
                    let base = |t: Option<&(String, String, String)>| -> Row {
                        vec![
                            c(name),
                            c(r.ty),
                            r.created.as_deref().and_then(c),
                            def.as_ref().and_then(|d| c(&d.0)),
                            c(def.as_ref().map_or("YES", |d| d.1)),
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
pub fn like(s: &str, p: &str) -> bool {
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

/// The filter the engine passes when the profile has none (an empty introspect filter).
pub fn match_all() -> hacivat_introspect::filter::CompiledFilter {
    hacivat_introspect::filter::parse_filter(
        "",
        DialectId::MySql,
        hacivat_introspect::filter::FilterRole::Introspect,
    )
    .expect("the empty filter")
}

pub fn ns() -> NsKey {
    NsKey::schema(["shop"])
}

/// The canonical content of a namespace: every object's key with its kind data, sorted.
pub fn canon(s: &NamespaceSnapshot) -> Vec<(ObjectKey, StateMark, ObjFlags, Props)> {
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

pub async fn pass_on(
    f: &mut Fake,
    mode: Mode,
    prev: Option<&NamespaceSnapshot>,
) -> (NamespaceSnapshot, NamespaceResult, PassStats) {
    f.log.clear();
    let srv = read_server(f, DbmsKind::MySql).await.expect("identity");
    let (res, stats) = run_pass(f, &srv, &ns(), mode, Level::Structure, &match_all(), prev)
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
pub async fn incremental_equals_full(
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

pub fn names(v: &[Name]) -> Vec<&str> {
    v.iter().map(|n| &**n).collect()
}
