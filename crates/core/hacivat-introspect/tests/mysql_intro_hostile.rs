// ## m4-t17-mysql
//! The T17 security review's hostile tests (`task-m4-17-review-hostile.rs`, reviewer-written),
//! committed with fix round 1: H1, H2 and H4 reproduced findings I-1, I-4 and I-2 and now
//! assert the fixes; H3, H5 and H6 are kept as written (a control and two guards). The fix
//! round's own tests follow: a hidden view definition, a fragment of an `Unknown` relation
//! (m6), and the engine's facts path (I-2: a failed read, and a fold rule that changes).

#![cfg(feature = "session")]

mod common;
mod mysql_fake;

use common::{RigOpts, bounded, idle, rig_with};
use futures::future::BoxFuture;
use hacivat_introspect::IntroError;
use hacivat_introspect::ddl_effects::Target;
use hacivat_introspect::mysql::pass::{SectionReader, read_server, run_fragment, run_pass};
use hacivat_introspect::mysql::server::ServerCtx;
use hacivat_introspect::mysql::show_create::section_of;
use hacivat_introspect::pack::{DialectId, Mode, PackCtx, PackFlags, Params};
use hacivat_introspect::scope::CurrentCtx;
use hacivat_introspect::testing::FakeIntrospector;
use hacivat_meta::merge::{ExistSet, MergePolicy, Trigger, base, merge};
use hacivat_meta::name::{FoldRule, IdentCtx};
use hacivat_meta::props::Props;
use hacivat_meta::{
    KindGroup, Level, Lookup, NamespaceSnapshot, NsKey, ObjectKey, ObjectKind, ServerFacts,
};
use hacivat_proto::DbmsKind;
use hacivat_session::meta::{Rows, check_own_text};
use mysql_fake::*;
use smallvec::smallvec;

fn key(path: &[&str], kind: ObjectKind) -> ObjectKey {
    let mut p = smallvec![n("shop")];
    p.extend(path.iter().map(|s| n(s)));
    ObjectKey {
        kind,
        path: p,
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    }
}

fn col_type(s: &NamespaceSnapshot, path: &[&str]) -> Option<String> {
    let i = s.find(&key(path, ObjectKind::Column))?;
    match s.obj(i).unwrap().props() {
        Props::Column(c) => Some(c.raw_type.to_string()),
        _ => None,
    }
}

/// H1 (I-1, flipped). A login that is not the view's definer and lacks SHOW VIEW reads an
/// empty `VIEWS.VIEW_DEFINITION`, so the definition hash does not move when the view is
/// replaced. The view's mark folds in its `COLUMNS` rows (visible without SHOW VIEW), so the
/// replaced view is re-read.
#[tokio::test]
async fn h1_view_replaced_under_a_hidden_definition_is_caught_by_incremental() {
    let mut f = Fake::mysql();
    let (s0, _, _) = pass_on(&mut f, Mode::Full, None).await;
    assert!(
        s0.find(&key(&["v_orders", "id"], ObjectKind::Column))
            .is_some()
    );
    // CREATE OR REPLACE VIEW v_orders AS SELECT id, status FROM orders (same hash: hidden).
    f.at("2026-11-20 10:10:00.000000");
    f.rels.get_mut("v_orders").unwrap().cols = vec![col("id", "int"), col("status", "varchar(8)")];
    let (inc, stats) = incremental_equals_full(&mut f, &s0).await;
    assert!(
        names(&stats.relations).contains(&"v_orders"),
        "{:?}",
        stats.relations
    );
    assert!(
        inc.find(&key(&["v_orders", "status"], ObjectKind::Column))
            .is_some()
    );
}

/// I-1, the rest: a view whose definition this login cannot read may change without a column
/// moving (a new `WHERE`), so it is re-read by every incremental pass, never left fresh.
#[tokio::test]
async fn a_view_with_a_hidden_definition_is_reread_by_every_pass() {
    let mut f = Fake::mysql();
    f.hidden.insert("v_orders".into());
    let (s0, _, _) = pass_on(&mut f, Mode::Full, None).await;
    for at in ["2026-11-20 10:10:00.000000", "2026-11-20 10:20:00.000000"] {
        f.at(at);
        let (_, stats) = incremental_equals_full(&mut f, &s0).await;
        assert_eq!(names(&stats.relations), ["v_orders"], "{at}");
    }
    // A readable definition is not re-read without a change.
    f.hidden.clear();
    let (s1, _, _) = pass_on(&mut f, Mode::Full, None).await;
    f.at("2026-11-20 10:30:00.000000");
    let (_, stats) = incremental_equals_full(&mut f, &s1).await;
    assert!(stats.relations.is_empty(), "{:?}", stats.relations);
}

async fn run_filtered(
    f: &mut Fake,
    srv: &ServerCtx,
    filter: &hacivat_introspect::filter::CompiledFilter,
    mode: Mode,
    prev: Option<NamespaceSnapshot>,
) -> (NamespaceSnapshot, hacivat_introspect::mysql::PassStats) {
    let (res, stats) = run_pass(f, srv, &ns(), mode, Level::Structure, filter, prev.as_ref())
        .await
        .unwrap();
    let b = prev.unwrap_or_else(|| base(ns(), DbmsKind::MySql, FoldRule::Exact).unwrap());
    (
        merge(Some(&b), res, &MergePolicy::default())
            .unwrap()
            .next()
            .clone(),
        stats,
    )
}

/// H2 (I-4, flipped). A pushed scope filter that keeps a view but not its base table: the
/// base's marker is cut from `TableMarks`, but the view's own mark folds in its `COLUMNS`
/// rows, so a type change of the base re-reads the view.
#[tokio::test]
async fn h2_pushed_view_over_an_unpushed_base_follows_the_base() {
    use hacivat_introspect::filter::{FilterRole, parse_filter};
    let mut f = Fake::mysql();
    let srv = read_server(&mut f, DbmsKind::MySql).await.unwrap();
    let filter = parse_filter("v_*", DialectId::MySql, FilterRole::Introspect)
        .unwrap()
        .with_ident(srv.ident());
    let (s0, st0) = run_filtered(&mut f, &srv, &filter, Mode::Full, None).await;
    assert_eq!(names(&st0.relations), ["v_orders"]);
    // ALTER TABLE orders MODIFY id BIGINT: the view's `COLUMNS` row now says bigint.
    f.at("2026-11-20 10:10:00.000000");
    {
        let o = f.rels.get_mut("orders").unwrap();
        o.created = Some("2026-11-20 10:10:00".into());
        o.cols[0].ty = "bigint".into();
    }
    f.rels.get_mut("v_orders").unwrap().cols[0].ty = "bigint".into();
    let (inc, st1) = run_filtered(&mut f, &srv, &filter, Mode::Incremental, Some(s0)).await;
    let (full, _) = run_filtered(&mut f, &srv, &filter, Mode::Full, None).await;
    assert_eq!(names(&st1.relations), ["v_orders"]);
    assert_eq!(
        col_type(&inc, &["v_orders", "id"]).as_deref(),
        Some("bigint")
    );
    assert_eq!(canon(&inc), canon(&full));
}

/// H3. The control: without a filter the same base change re-reads the view (the
/// VIEW_TABLE_USAGE path works when the base is listed).
#[tokio::test]
async fn h3_control_unfiltered_base_change_rereads_the_view() {
    let mut f = Fake::mysql();
    let (s0, _, _) = pass_on(&mut f, Mode::Full, None).await;
    f.at("2026-11-20 10:10:00.000000");
    {
        let o = f.rels.get_mut("orders").unwrap();
        o.created = Some("2026-11-20 10:10:00".into());
        o.cols[0].ty = "bigint".into();
    }
    f.rels.get_mut("v_orders").unwrap().cols[0].ty = "bigint".into();
    let (_, stats) = incremental_equals_full(&mut f, &s0).await;
    assert!(
        names(&stats.relations).contains(&"v_orders"),
        "{:?}",
        stats.relations
    );
}

/// H4 (I-2, flipped). The MySQL-family placeholder facts (the hub's, and what the engine
/// publishes when the facts read fails) carry an unknown `lower_case_table_names`: no table
/// rule is pinned, so an arena built under them answers `ServerDependent` for `CUSTOMERS`
/// (stored `customers`), never a definite `Missing`. lctn 0 (a real, read fact) is exact.
#[tokio::test]
async fn h4_unknown_lctn_answers_server_dependent_not_missing() {
    let mut f = Fake::mysql();
    let (s0, _, _) = pass_on(&mut f, Mode::Full, None).await;
    let unknown = IdentCtx {
        lower_case_table_names: u8::MAX,
        ..IdentCtx::new(DialectId::MySql)
    };
    let got = s0.match_named(KindGroup::Relations, "CUSTOMERS", false, &unknown);
    assert!(matches!(got, Lookup::Unknown(_)), "{got:?}");
    let lctn0 = IdentCtx::new(DialectId::MySql);
    let got = s0.match_named(KindGroup::Relations, "CUSTOMERS", false, &lctn0);
    assert!(matches!(got, Lookup::Missing(_)), "{got:?}");
}

/// H5. Show-create quoting and the guard over a wider hostile set, in every MySQL-family
/// flavor: a rendered text is either refused by the guard, or exactly one doubled-backtick
/// identifier pair; mysql_common's named-parameter scanner would see no `:name` outside
/// backticks.
#[test]
fn h5_show_create_hostile_names_never_escape_the_identifier() {
    let p = pack();
    let names = [
        "a`b",
        "`",
        "``",
        "a`",
        "`a",
        "x` FOR UPDATE `",
        "x`(",
        "x` (1) `",
        "a:x",
        "`:x`",
        "a?b",
        "a'b",
        "a\"b",
        "a\\",
        "a\\`",
        "*/",
        "/*",
        "x`/*!50000 SELECT 1*/`",
        "--",
        "# c",
        "a\nb",
        "a\tb",
        "a\u{2028}b",
        "a\u{0085}b",
        "a\u{200B}b",
        "a\u{00A0}b",
        "a;b",
        "x`; SELECT SLEEP(1); `",
        "İ",
        "update",
        "sleep",
        "一",
        "\u{FF07}",
    ];
    // mysql_common 0.37.3 ParsedNamedParams::parse: backticks enter/leave an identifier.
    fn params_outside_backticks(sql: &str) -> bool {
        let mut in_ident = false;
        let b = sql.as_bytes();
        for (i, &c) in b.iter().enumerate() {
            if in_ident {
                if c == b'`' {
                    in_ident = false;
                }
                continue;
            }
            match c {
                b'`' => in_ident = true,
                b'?' => return true,
                b':' if b
                    .get(i + 1)
                    .is_some_and(|d| d.is_ascii_lowercase() || *d == b'_') =>
                {
                    return true;
                }
                _ => {}
            }
        }
        false
    }
    let mut accepted = 0;
    for flavor in [DbmsKind::MySql, DbmsKind::MariaDb] {
        for (k, kw) in [
            (ObjectKind::Table, "TABLE"),
            (ObjectKind::View, "VIEW"),
            (ObjectKind::Procedure, "PROCEDURE"),
            (ObjectKind::Routine, "FUNCTION"),
            (ObjectKind::Trigger, "TRIGGER"),
            (ObjectKind::Event, "EVENT"),
        ] {
            let (s, _, _) = section_of(k).unwrap();
            for name in names {
                let maria = flavor == DbmsKind::MariaDb;
                let ctx = PackCtx {
                    dialect: if maria {
                        DialectId::MariaDb
                    } else {
                        DialectId::MySql
                    },
                    version: if maria { 110_405 } else { 80_046 },
                    flavor,
                    mode: Mode::Full,
                    level: Level::Sources,
                    flags: if maria {
                        PackFlags::MARIADB
                    } else {
                        PackFlags::empty()
                    },
                    params: Params {
                        scalars: vec![(n("schema"), text("s`c")), (n("name"), text(name))],
                        lists: Vec::new(),
                    },
                };
                let q = p.render(s, &ctx).expect("renders").remove(0);
                let want = format!("SHOW CREATE {kw} `s``c`.`{}`", name.replace('`', "``"));
                assert_eq!(q.sql(), want, "{name:?}");
                assert!(!params_outside_backticks(q.sql()), "{name:?}: {}", q.sql());
                if check_own_text(q.sql(), flavor).is_ok() {
                    accepted += 1;
                    // An accepted text never holds a `;`, an executable comment or a
                    // control/invisible character.
                    assert!(
                        !q.sql().contains(';') && !q.sql().contains("/*!"),
                        "{name:?}"
                    );
                    assert!(
                        !q.sql()
                            .chars()
                            .any(|c| (c.is_control() && c != '\n' && c != '\t')
                                || (c.is_whitespace() && !c.is_ascii())
                                || c == '\u{200B}'),
                        "{name:?}"
                    );
                }
                // Never exempt outside the MySQL family.
                for other in [DbmsKind::Sqlite, DbmsKind::Postgres] {
                    if !name.is_ascii() || ["update", "sleep"].contains(&name) {
                        assert!(
                            check_own_text(q.sql(), other).is_err(),
                            "{name:?} {other:?}"
                        );
                    }
                }
            }
        }
    }
    assert!(accepted > 0);
}

/// H6. A non-ASCII name that the guard would accept in a `SHOW CREATE` must still be refused
/// when the connection is not utf8mb4 (the GBK breakout precondition): the introspector checks
/// `Identity`'s three character sets before rendering.
#[tokio::test]
async fn h6_show_create_refused_without_utf8mb4() {
    use hacivat_introspect::mysql::show_create::show_create;
    struct Gbk(Fake);
    impl SectionReader for Gbk {
        fn read<'a>(
            &'a mut self,
            section: &'static str,
            ctx: PackCtx,
        ) -> BoxFuture<'a, Result<Rows, IntroError>> {
            Box::pin(async move {
                if section == "Identity" {
                    let mut r = self.0.answer(section, &ctx);
                    r[0][5] = c("gbk");
                    return Ok(Rows {
                        columns: Vec::new(),
                        rows: r,
                    });
                }
                assert!(
                    !section.starts_with("ShowCreate"),
                    "{section} sent under GBK"
                );
                self.0.read(section, ctx).await
            })
        }
    }
    let mut g = Gbk(Fake::mysql());
    let srv = read_server(&mut g, DbmsKind::MySql).await.unwrap();
    assert!(!srv.utf8mb4);
    let got = show_create(&mut g, &srv, ObjectKind::Table, "shop", "一").await;
    assert!(got.is_err(), "{got:?}");
}

/// m6: a fragment asked for by an `ObjectKind::Unknown` key decides `Unknown` objects too, so
/// a dropped relation of a type this build does not model is removed by the fragment.
#[tokio::test]
async fn a_fragment_of_an_unknown_relation_decides_its_existence() {
    let mut f = Fake::mysql();
    let mut tmp = table("2026-11-20 09:00:00", vec![col("id", "int")]);
    tmp.ty = "TEMPORARY";
    f.rels.insert("tmpx".into(), tmp);
    let (s0, _, _) = pass_on(&mut f, Mode::Full, None).await;
    let k = ObjectKey::unknown(smallvec![n("shop"), n("tmpx")], None, Some(n("TEMPORARY")));
    assert!(s0.find(&k).is_some());
    f.rels.remove("tmpx");
    let srv = read_server(&mut f, DbmsKind::MySql).await.unwrap();
    let (res, _) = run_fragment(
        &mut f,
        &srv,
        &ns(),
        std::slice::from_ref(&k),
        Level::Structure,
    )
    .await
    .unwrap();
    match &res.existent[&ObjectKind::Unknown] {
        ExistSet::Scoped { scope, ids } => {
            assert_eq!(scope, &vec![vec![n("tmpx")]]);
            assert!(ids.is_empty());
        }
        other => panic!("{other:?}"),
    }
    let next = merge(Some(&s0), res, &MergePolicy::default()).unwrap();
    assert!(next.next().find(&k).is_none());
}

// ---- I-2: the engine's facts -------------------------------------------------------------

fn mysql_facts(lctn: u8) -> ServerFacts {
    let mut f = ServerFacts::new(
        DbmsKind::MySql,
        IdentCtx {
            lower_case_table_names: lctn,
            ..IdentCtx::new(DialectId::MySql)
        },
    );
    f.version = n("8.4.3");
    f.version_num = 80_403;
    f
}

fn mysql_rig(fake: FakeIntrospector) -> common::Rig {
    rig_with(
        fake,
        RigOpts {
            dbms: DbmsKind::MySql,
            scope: "*",
            current: CurrentCtx {
                database: Some(n("shop")),
                schema: Some(n("shop")),
                ident: None,
            },
            ..RigOpts::default()
        },
    )
}

/// A failed facts read publishes an unknown `lower_case_table_names` (never the lctn-0
/// placeholder), so a lookup of `T1` (stored `t1`) answers `ServerDependent`, not `Missing`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_facts_read_publishes_an_unknown_identifier_rule() {
    let shop = NsKey::schema(["shop"]);
    let fake = FakeIntrospector::new(DbmsKind::MySql);
    fake.put_ns(shop.clone(), FakeIntrospector::tables(3));
    fake.set_facts(Some(Err(IntroError::Timeout)));
    let r = mysql_rig(fake);
    bounded(r.handle.on_connect()).await.unwrap();
    idle(&r.handle).await;
    let snap = r.catalog.current();
    assert_eq!(snap.server.ident.lower_case_table_names, u8::MAX);
    let arena = snap.namespaces.get(&shop).expect("loaded");
    let got = arena.match_named(KindGroup::Relations, "T1", false, &snap.server.ident);
    assert!(matches!(got, Lookup::Unknown(_)), "{got:?}");
}

/// The real facts replace the placeholder, and a later fact that changes the table fold rule
/// (lctn 0 → 1) re-folds the cached arenas: `T1` then finds the stored `t1`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_changed_fold_rule_refolds_the_cached_arenas() {
    let shop = NsKey::schema(["shop"]);
    let fake = FakeIntrospector::new(DbmsKind::MySql);
    fake.put_ns(shop.clone(), FakeIntrospector::tables(3));
    fake.set_facts(Some(Ok(mysql_facts(0))));
    let r = mysql_rig(fake);
    bounded(r.handle.on_connect()).await.unwrap();
    idle(&r.handle).await;
    let snap = r.catalog.current();
    assert_eq!(*snap.server, mysql_facts(0));
    let arena = snap.namespaces.get(&shop).expect("loaded").clone();
    assert_eq!(arena.fold(), FoldRule::Exact);
    let got = arena.match_named(KindGroup::Relations, "T1", false, &snap.server.ident);
    assert!(matches!(got, Lookup::Missing(_)), "{got:?}");
    r.fake.set_facts(Some(Ok(mysql_facts(1))));
    bounded(r.handle.refresh(Target::ScopeRefresh, Trigger::User))
        .await
        .unwrap();
    idle(&r.handle).await;
    let snap = r.catalog.current();
    assert_eq!(snap.server.ident.lower_case_table_names, 1);
    let arena = snap.namespaces.get(&shop).expect("still loaded");
    assert_eq!(arena.fold(), FoldRule::AsciiCaseless);
    assert_eq!(arena.len(), r_len(&r, &shop));
    let got = arena.match_named(KindGroup::Relations, "T1", false, &snap.server.ident);
    assert!(matches!(got, Lookup::Found(_)), "{got:?}");
}

/// The object count of `ns` in the current catalog.
fn r_len(r: &common::Rig, ns: &NsKey) -> usize {
    r.catalog
        .current()
        .namespaces
        .get(ns)
        .map_or(0, |a| a.len())
}

#[test]
fn the_hub_placeholder_pins_no_mysql_table_rule() {
    let placeholder = IdentCtx {
        lower_case_table_names: u8::MAX,
        ..IdentCtx::new(DialectId::MySql)
    };
    assert_eq!(
        FoldRule::for_names(
            DialectId::MySql,
            &placeholder,
            hacivat_meta::name::NameClass::Table
        ),
        None
    );
}
