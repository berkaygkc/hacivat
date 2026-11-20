// ## m4-t16-pg (spike origin 5a5ab08, `tests/docker_pg_intro.rs`)
//! The PostgreSQL introspector on real servers (M4 Task 16; HV-INTRO-05, the PG parts of
//! HV-INTRO-13 and -15). Every test is `#[ignore = "docker"]`, returns early unless
//! `HACIVAT_DOCKER=1`, takes its targets only from `require_target`, and creates its own
//! `hvm4_t16_*` schema, dropped by a guard (also on panic). The one shared fixture is the
//! `hv_m4_big` schema on `pg17` (full set; dropped at the gate).
//!
//! Run: `<main>/target/docker/run-docker-tests.sh --set full -p hacivat-introspect --test
//! docker_pg_intro` (PR set: pg17 only).
//!
//! "Incremental equals full" compares the cached namespace merged with an incremental pass
//! (Task 10's `merge`) against a fresh full pass merged into an empty base, object by object
//! (kind, path, server id, flags, properties).
// ## m4-t33a-hygiene (named exception): the modules this file uses exist only with the
// `session` feature, so `--no-default-features --tests` builds (T16b/T17 carry).
#![cfg(feature = "session")]

mod common;

use std::sync::Arc;
use std::time::Instant;

use common::pg::{DropSchemas, PG, Pg, admin, unique};
use hacivat_introspect::pack::Mode;
use hacivat_introspect::pg::pass::{self, PassOpts, PgPass};
use hacivat_introspect::pg::sections::{Binds, ctx, name, pack};
use hacivat_introspect::pg::watermark::FullReason;
use hacivat_introspect::pg::{state, system};
use hacivat_introspect::testing::big_catalog;
use hacivat_introspect::{Introspector, Staleness};
use hacivat_meta::delta::CatalogDelta;
use hacivat_meta::merge::{MergePolicy, NamespaceResult, Trigger, base, merge};
use hacivat_meta::name::FoldRule;
use hacivat_meta::props::{FkAction, Props, TypeBody};
use hacivat_meta::{
    Level, NamespaceSnapshot, NsKey, Obj, ObjFlags, ObjectKey, ObjectKind, ServerId, StateMark,
    Watermark,
};
use hacivat_proto::DbmsKind;
use hacivat_session::meta::MetaConn;
use hacivat_testkit::{DockerTarget, docker_enabled, require_target};
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------------------------
// Helpers

fn targets() -> Vec<DockerTarget> {
    PG.into_iter().filter_map(require_target).collect()
}

fn major(t: &DockerTarget) -> u32 {
    t.major.parse().unwrap_or(0)
}

fn ns_of(t: &DockerTarget, schema: &str) -> NsKey {
    NsKey::schema([t.database.as_str(), schema])
}

async fn pass_at(
    c: &mut MetaConn,
    ns: &NsKey,
    mode: Mode,
    level: Level,
    prev: Option<&NamespaceSnapshot>,
) -> PgPass {
    let cancel = CancellationToken::new();
    pass::introspect(
        c,
        ns,
        PassOpts {
            mode,
            level,
            prev,
            ..PassOpts::default()
        },
        &cancel,
    )
    .await
    .unwrap_or_else(|e| panic!("{ns:?} {mode:?}: {e:?}"))
}

/// `r` merged into `prev` (a user refresh; the guard's pending result is taken as is).
fn apply(prev: &NamespaceSnapshot, r: NamespaceResult) -> (NamespaceSnapshot, CatalogDelta) {
    let policy = MergePolicy {
        trigger: Trigger::User,
        ..MergePolicy::default()
    };
    match merge(Some(prev), r, &policy).expect("merge") {
        hacivat_meta::merge::MergeOutcome::Applied { next, delta, .. }
        | hacivat_meta::merge::MergeOutcome::PendingConfirmation { next, delta, .. } => {
            (next, delta)
        }
    }
}

/// A fresh full pass at `level`, as a namespace.
async fn load(c: &mut MetaConn, ns: &NsKey, level: Level) -> NamespaceSnapshot {
    let p = pass_at(c, ns, Mode::Full, level, None).await;
    let b = base(ns.clone(), DbmsKind::Postgres, FoldRule::Exact).unwrap();
    apply(&b, p.result).0
}

/// One incremental pass over `prev`, applied.
async fn refresh(
    c: &mut MetaConn,
    ns: &NsKey,
    prev: &NamespaceSnapshot,
) -> (NamespaceSnapshot, CatalogDelta, PgPass) {
    let level = prev.meta().level;
    let p = pass_at(c, ns, Mode::Incremental, level, Some(prev)).await;
    let (next, delta) = apply(prev, p.result.clone());
    (next, delta, p)
}

/// Every object as a comparable line (kind, path, server id, flags, properties).
fn canon(ns: &NamespaceSnapshot) -> Vec<String> {
    let mut v: Vec<String> = (0..ns.len() as u32)
        .filter_map(|i| {
            let o = ns.obj(i)?;
            let k = ns.object_key(i)?;
            Some(format!(
                "{:?} {:?} {:?} {:?} {:?}",
                k.kind,
                k.path,
                k.server_id,
                o.flags(),
                o.props()
            ))
        })
        .collect();
    v.sort();
    v
}

fn assert_same(label: &str, got: &NamespaceSnapshot, want: &NamespaceSnapshot) {
    let (g, w) = (canon(got), canon(want));
    if g != w {
        let only_got: Vec<_> = g.iter().filter(|x| !w.contains(x)).collect();
        let only_want: Vec<_> = w.iter().filter(|x| !g.contains(x)).collect();
        panic!(
            "{label}: incremental differs from full\n only incremental: {only_got:#?}\n only full: {only_want:#?}"
        );
    }
}

/// The object of `kind` whose namespace-relative path is `path`.
fn find<'a>(ns: &'a NamespaceSnapshot, kind: ObjectKind, path: &[&str]) -> Option<&'a Obj> {
    let base = ns.key().path.len();
    (0..ns.len() as u32).find_map(|i| {
        let k = ns.object_key(i)?;
        (k.kind == kind
            && k.path.len() == base + path.len()
            && k.path[base..].iter().zip(path).all(|(a, b)| &**a == *b))
        .then(|| ns.obj(i))
        .flatten()
    })
}

fn get<'a>(ns: &'a NamespaceSnapshot, kind: ObjectKind, path: &[&str]) -> &'a Obj {
    find(ns, kind, path).unwrap_or_else(|| panic!("{kind:?} {path:?} not found"))
}

fn key_of(ns: &NamespaceSnapshot, kind: ObjectKind, path: &[&str]) -> ObjectKey {
    let base = ns.key().path.len();
    (0..ns.len() as u32)
        .find_map(|i| {
            let k = ns.object_key(i)?;
            (k.kind == kind
                && k.path[base..].iter().zip(path).all(|(a, b)| &**a == *b)
                && k.path.len() == base + path.len())
            .then_some(k)
        })
        .unwrap_or_else(|| panic!("{kind:?} {path:?} not found"))
}

fn children_of(
    ns: &NamespaceSnapshot,
    kind: ObjectKind,
    path: &[&str],
    child: ObjectKind,
) -> Vec<String> {
    let k = key_of(ns, kind, path);
    let i = ns.find(&k).unwrap();
    ns.children_of_kind(i, child)
        .map(|(_, o)| o.name().to_string())
        .collect()
}

/// A fresh schema `hvm4_t16_<slug>` of `t` with `ddl` (`{s}` is the schema), and its guard.
async fn schema_with(t: &DockerTarget, slug: &str, ddl: &str) -> (String, DropSchemas) {
    let s = unique(&format!("hvm4_t16_{slug}"));
    let guard = DropSchemas {
        t: t.clone(),
        names: vec![s.clone()],
    };
    let mut a = admin(t).await;
    a.batch(&format!("CREATE SCHEMA {s}")).await;
    if !ddl.is_empty() {
        a.batch(&ddl.replace("{s}", &s)).await;
    }
    a.close().await;
    (s, guard)
}

async fn run(t: &DockerTarget, sql: &str) {
    let mut a = admin(t).await;
    a.batch(sql).await;
    a.close().await;
}

fn ran(p: &PgPass, section: &str) -> bool {
    p.report.log.ran(section)
}

// ---------------------------------------------------------------------------------------------
// The kinds (spike test 1)

fn fixture(schema: &str) -> String {
    include_str!("fixtures/pg_kinds.sql").replace("hvm4_fx", schema)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn full_pass_reads_every_kind() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let s = unique("hvm4_t16_kinds");
        let _g = DropSchemas {
            t: t.clone(),
            names: vec![s.clone()],
        };
        run(&t, &fixture(&s)).await;
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let ns = ns_of(&t, &s);
        let n = load(&mut c, &ns, Level::Sources).await;
        let id = &t.id;
        // The schema.
        assert_eq!(
            n.root().props().comment().map(|c| &**c),
            Some("fixture schema"),
            "{id}"
        );
        // Columns.
        let parent = |c: &str| {
            get(&n, ObjectKind::Column, &["parent", c])
                .props()
                .as_column()
                .unwrap()
                .clone()
        };
        assert!(matches!(
            parent("id").identity(),
            Some(hacivat_meta::props::Identity::Always)
        ));
        assert_eq!(&*parent("tags").raw_type, "text[]", "{id}");
        assert_eq!(&*parent("grid").raw_type, "integer[]", "{id}");
        assert_eq!(parent("m").type_ref.unwrap().name.as_ref(), "mood", "{id}");
        assert_eq!(&*parent("total").raw_type, "numeric(10,2)", "{id}");
        assert_eq!(parent("total").default_raw.as_deref(), Some("0"), "{id}");
        let doubled = parent("doubled");
        assert!(doubled.default_raw.is_none(), "{id}");
        assert!(doubled.generated().unwrap().expr.contains("total"), "{id}");
        assert_eq!(parent("name").collation().map(|c| &**c), Some("C"), "{id}");
        assert_eq!(
            &*parent("ts").raw_type,
            "timestamp(3) with time zone",
            "{id}"
        );
        assert_eq!(parent("code").comment.as_deref(), Some("Code"), "{id}");
        assert!(!parent("code").nullable, "{id}");
        // Ordinal = attnum.
        assert_eq!(parent("id").ordinal, 1, "{id}");
        assert_eq!(
            get(&n, ObjectKind::Table, &["parent"])
                .props()
                .comment()
                .map(|c| &**c),
            Some("Parents")
        );
        // Keys, a check, an FK, an exclusion; backing indexes implicit.
        let pk = get(&n, ObjectKind::PrimaryKey, &["parent", "parent_pkey"]);
        let Props::Key(k) = pk.props() else {
            panic!("{id}: pk props")
        };
        assert_eq!(k.backing_index.as_deref(), Some("parent_pkey"), "{id}");
        let ix = get(&n, ObjectKind::Index, &["parent", "parent_pkey"]);
        assert!(ix.flags().contains(ObjFlags::IMPLICIT), "{id}");
        assert!(
            find(&n, ObjectKind::UniqueKey, &["parent", "parent_code_key"]).is_some(),
            "{id}"
        );
        let ck = get(&n, ObjectKind::Check, &["parent", "parent_total_ck"]);
        let Props::Check(ck) = ck.props() else {
            panic!()
        };
        assert!(ck.expr.contains("total >="), "{id}: {}", ck.expr);
        let fk = children_of(&n, ObjectKind::Table, &["child"], ObjectKind::ForeignKey);
        assert_eq!(fk.len(), 1, "{id}");
        let fk = get(&n, ObjectKind::ForeignKey, &["child", &fk[0]]);
        let Props::ForeignKey(fk) = fk.props() else {
            panic!()
        };
        assert!(fk.deferrable && fk.initially_deferred, "{id}");
        assert_eq!(fk.on_delete, FkAction::Cascade, "{id}");
        assert_eq!(fk.target.path.last().map(|p| &**p), Some("parent"), "{id}");
        let uq = get(&n, ObjectKind::UniqueKey, &["child", "child_ab_uq"]);
        let Props::Key(uq) = uq.props() else { panic!() };
        let order: Vec<_> = uq
            .columns
            .iter()
            .map(|c| c.name.as_deref().unwrap_or(""))
            .collect();
        assert_eq!(order, ["b", "a"], "{id}: key order");
        assert!(
            find(&n, ObjectKind::Exclusion, &["child", "child_r_excl"]).is_some(),
            "{id}"
        );
        // Indexes: expression + INCLUDE + predicate; DESC NULLS LAST.
        let ex = get(&n, ObjectKind::Index, &["child", "child_expr_idx"]);
        let Props::Index(ex) = ex.props() else {
            panic!()
        };
        assert_eq!(ex.key.expressions.len(), 1, "{id}");
        assert!(ex.key.expressions[0].contains("a + b"), "{id}");
        assert_eq!(
            ex.key.include.iter().map(|c| &**c).collect::<Vec<_>>(),
            ["parent_id"]
        );
        assert_eq!(ex.key.predicate.as_deref(), Some("(a > 0)"), "{id}");
        let de = get(&n, ObjectKind::Index, &["child", "child_desc_idx"]);
        let Props::Index(de) = de.props() else {
            panic!()
        };
        assert_eq!(de.key.desc, [true, false], "{id}");
        assert_eq!(de.comment.as_deref(), Some("desc"), "{id}");
        // Triggers: the internal FK triggers are left out.
        assert_eq!(
            children_of(&n, ObjectKind::Table, &["child"], ObjectKind::Trigger),
            ["child_bi"]
        );
        let tr = get(&n, ObjectKind::Trigger, &["child", "child_bi"]);
        let Props::Trigger(tr) = tr.props() else {
            panic!()
        };
        assert!(tr.enabled && tr.for_each_row && tr.source.is_some(), "{id}");
        // Partitions.
        let ev = get(&n, ObjectKind::Table, &["events"])
            .props()
            .as_table()
            .unwrap()
            .clone();
        assert!(
            ev.partitioned && ev.partition_key.as_deref() == Some("RANGE (at)"),
            "{id}"
        );
        let p26 = get(&n, ObjectKind::Table, &["events_2026"])
            .props()
            .as_table()
            .unwrap()
            .clone();
        assert_eq!(
            p26.partition_of.unwrap().path.last().map(|p| &**p),
            Some("events")
        );
        assert!(
            p26.partition_bound.unwrap().starts_with("FOR VALUES FROM"),
            "{id}"
        );
        // Views, sources at Sources level.
        let Props::View(v) = get(&n, ObjectKind::View, &["v_parent"]).props() else {
            panic!()
        };
        assert!(v.source.is_some() && v.comment.as_deref() == Some("positive parents"));
        assert!(
            find(&n, ObjectKind::MaterializedView, &["mv_child"]).is_some(),
            "{id}"
        );
        // Sequences: explicit, serial-owned, identity (implicit).
        let Props::Sequence(sq) = get(&n, ObjectKind::Sequence, &["seq_x"]).props() else {
            panic!()
        };
        assert_eq!(
            (sq.increment, sq.min, sq.max, sq.start, sq.cache, sq.cycle),
            (Some(5), Some(10), Some(1000), Some(10), Some(2), true),
            "{id}"
        );
        let serial = get(&n, ObjectKind::Sequence, &["child_id_seq"]);
        let Props::Sequence(sp) = serial.props() else {
            panic!()
        };
        assert_eq!(sp.owned_by.as_ref().map(|(_, c)| &**c), Some("id"), "{id}");
        assert!(!serial.flags().contains(ObjFlags::IMPLICIT), "{id}");
        assert!(
            get(&n, ObjectKind::Sequence, &["parent_id_seq"])
                .flags()
                .contains(ObjFlags::IMPLICIT)
        );
        // Routines: overloads, SETOF, a procedure, an aggregate.
        let adds: Vec<_> = n
            .objs()
            .iter()
            .filter(|o| o.kind() == ObjectKind::Routine && &**o.name() == "add")
            .map(|o| {
                o.props()
                    .overload_sig()
                    .map(|s| s.to_string())
                    .unwrap_or_default()
            })
            .collect();
        assert_eq!(adds.len(), 2, "{id}");
        assert!(
            adds.contains(&"a integer, b integer".to_string()),
            "{id}: {adds:?}"
        );
        let Props::Routine(series) = get(&n, ObjectKind::Routine, &["series"]).props() else {
            panic!()
        };
        assert!(series.returns_set, "{id}");
        assert!(find(&n, ObjectKind::Procedure, &["noop"]).is_some(), "{id}");
        assert!(
            find(&n, ObjectKind::Aggregate, &["mysum"]).is_some(),
            "{id}"
        );
        // Types.
        let body = |k, name| match get(&n, k, &[name]).props() {
            Props::Type(t) => t.body.clone(),
            other => panic!("{other:?}"),
        };
        assert!(
            matches!(body(ObjectKind::EnumType, "mood"), TypeBody::Enum { labels } if labels.iter().map(|l| &**l).collect::<Vec<_>>() == ["sad", "ok", "happy"])
        );
        let TypeBody::Domain {
            base,
            not_null,
            not_null_constraint,
            checks,
            default_raw,
        } = body(ObjectKind::Domain, "posint")
        else {
            panic!("{id}: domain")
        };
        assert_eq!((&*base, not_null), ("integer", true), "{id}");
        assert_eq!(
            checks.iter().map(|c| &**c).collect::<Vec<_>>(),
            ["VALUE > 0"],
            "{id}"
        );
        assert_eq!(default_raw.as_deref(), Some("1"), "{id}");
        assert_eq!(
            not_null_constraint.is_some(),
            major(&t) >= 17,
            "{id}: domain NOT NULL row"
        );
        assert!(
            matches!(body(ObjectKind::CompositeType, "pair"), TypeBody::Composite { attrs } if attrs.len() == 2)
        );
        assert!(
            matches!(body(ObjectKind::RangeType, "floatrange"), TypeBody::Range { subtype } if &*subtype == "double precision")
        );
        // Quoted and non-ASCII names verbatim; unlogged.
        assert!(
            find(&n, ObjectKind::Column, &["Mixed Case", "Ünïcode Col"]).is_some(),
            "{id}"
        );
        assert!(
            find(&n, ObjectKind::Column, &["Mixed Case", "quote\"d"]).is_some(),
            "{id}"
        );
        assert_eq!(
            get(&n, ObjectKind::Table, &["u"])
                .props()
                .as_table()
                .unwrap()
                .persistence,
            hacivat_meta::props::Persistence::Unlogged
        );
        // PostgreSQL 18 catalogues column NOT NULL constraints.
        let nn = parent("code")
            .rare
            .and_then(|r| r.not_null_constraint.clone());
        assert_eq!(nn.is_some(), major(&t) >= 18, "{id}");
        drop(c);
        pg.shutdown().await;
    }
}

// ---------------------------------------------------------------------------------------------
// Incremental (alg/02 HV-INTRO-05 tests 1–8, 12)

/// Loads `ddl` at `level`, runs `step`, refreshes, and checks incremental == full. Returns
/// the refreshed namespace, the delta and the pass.
async fn step_case(
    t: &DockerTarget,
    slug: &str,
    ddl: &str,
    step: &str,
    level: Level,
) -> (NamespaceSnapshot, CatalogDelta, PgPass, String) {
    let (s, g) = schema_with(t, slug, ddl).await;
    let pg = Pg::connect(t).await;
    let mut c = pg.meta().await;
    let ns = ns_of(t, &s);
    let prev = load(&mut c, &ns, level).await;
    run(t, &step.replace("{s}", &s)).await;
    let (next, delta, p) = refresh(&mut c, &ns, &prev).await;
    assert!(
        p.report.full_reason.is_none(),
        "{} {slug}: incremental",
        t.id
    );
    let fresh = load(&mut c, &ns, level).await;
    assert_same(&format!("{} {slug}", t.id), &next, &fresh);
    drop(c);
    pg.shutdown().await;
    drop(g);
    (next, delta, p, s)
}

const X: &str = "CREATE TABLE {s}.x (id int PRIMARY KEY, a int, b int)";

fn altered(delta: &CatalogDelta, name: &str) -> Option<hacivat_meta::delta::ChangedFacets> {
    delta
        .altered
        .iter()
        .find(|(k, _)| k.name() == name)
        .map(|(_, f)| *f)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn create_index_reports_indexes_changed_not_table_props() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (n, delta, p, _) = step_case(
            &t,
            "ix",
            X,
            "CREATE INDEX x_a ON {s}.x (a)",
            Level::Structure,
        )
        .await;
        assert!(find(&n, ObjectKind::Index, &["x", "x_a"]).is_some());
        let f = altered(&delta, "x").expect("x altered");
        assert!(f.children && !f.definition && !f.comment, "{}: {f:?}", t.id);
        assert!(ran(&p, name::RETRIEVE_INDEXES), "{}", t.id);
        assert!(
            !ran(&p, name::RETRIEVE_RELATIONS) && !ran(&p, name::RETRIEVE_COLUMNS),
            "{}",
            t.id
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn add_column_rereads_the_table() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (n, _, p, _) = step_case(
            &t,
            "addcol",
            X,
            "ALTER TABLE {s}.x ADD COLUMN c text",
            Level::Structure,
        )
        .await;
        assert!(
            find(&n, ObjectKind::Column, &["x", "c"]).is_some(),
            "{}",
            t.id
        );
        assert!(ran(&p, name::RETRIEVE_COLUMNS) && ran(&p, name::RETRIEVE_RELATIONS));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn comment_only_updates_the_comment() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (n, delta, p, _) = step_case(
            &t,
            "cmt",
            X,
            "COMMENT ON TABLE {s}.x IS 'hello'",
            Level::Structure,
        )
        .await;
        assert_eq!(
            get(&n, ObjectKind::Table, &["x"])
                .props()
                .comment()
                .map(|c| &**c),
            Some("hello")
        );
        assert!(altered(&delta, "x").is_some_and(|f| f.comment && !f.definition));
        assert!(ran(&p, name::COMMENTS), "{}", t.id);
        for s in [
            name::RETRIEVE_COLUMNS,
            name::RETRIEVE_RELATIONS,
            name::RETRIEVE_INDEXES,
        ] {
            assert!(!ran(&p, s), "{}: {s}", t.id);
        }
    }
}

const TRG: &str = "CREATE TABLE {s}.x (id int PRIMARY KEY, a int); \
    CREATE FUNCTION {s}.f() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$; \
    CREATE TRIGGER t1 BEFORE INSERT ON {s}.x FOR EACH ROW EXECUTE FUNCTION {s}.f()";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn second_trigger_is_created() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (n, _, p, _) = step_case(
            &t,
            "trg",
            TRG,
            "CREATE TRIGGER t2 AFTER UPDATE ON {s}.x FOR EACH ROW EXECUTE FUNCTION {s}.f()",
            Level::Structure,
        )
        .await;
        let mut trgs = children_of(&n, ObjectKind::Table, &["x"], ObjectKind::Trigger);
        trgs.sort();
        assert_eq!(trgs, ["t1", "t2"], "{}", t.id);
        assert!(ran(&p, name::RETRIEVE_TRIGGERS) && !ran(&p, name::RETRIEVE_COLUMNS));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn drop_index_removed_by_existence() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (n, delta, _, _) = step_case(
            &t,
            "dropix",
            &format!("{X}; CREATE INDEX x_a ON {{s}}.x (a)"),
            "DROP INDEX {s}.x_a",
            Level::Structure,
        )
        .await;
        assert!(
            find(&n, ObjectKind::Index, &["x", "x_a"]).is_none(),
            "{}",
            t.id
        );
        assert!(delta.dropped.iter().any(|k| k.name() == "x_a"), "{}", t.id);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn drop_column_removed() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (n, _, _, _) = step_case(
            &t,
            "dropcol",
            X,
            "ALTER TABLE {s}.x DROP COLUMN b",
            Level::Structure,
        )
        .await;
        assert!(
            find(&n, ObjectKind::Column, &["x", "b"]).is_none(),
            "{}",
            t.id
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn alter_function_volatility_updated() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (n, _, p, _) = step_case(
            &t,
            "vol",
            "CREATE FUNCTION {s}.f() RETURNS int LANGUAGE sql VOLATILE AS 'SELECT 1'",
            "ALTER FUNCTION {s}.f() STABLE",
            Level::Structure,
        )
        .await;
        let Props::Routine(r) = get(&n, ObjectKind::Routine, &["f"]).props() else {
            panic!()
        };
        assert_eq!(
            r.volatility,
            hacivat_meta::props::Volatility::Stable,
            "{}",
            t.id
        );
        assert!(ran(&p, name::RETRIEVE_ROUTINES) && !ran(&p, name::RETRIEVE_COLUMNS));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn uncommitted_ddl_appears_on_next_pass() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (s, _g) = schema_with(&t, "late", "CREATE TABLE {s}.a (x int)").await;
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let ns = ns_of(&t, &s);
        let prev = load(&mut c, &ns, Level::Structure).await;
        // Another session opens a transaction that creates a table and stays open.
        let mut open = admin(&t).await;
        open.batch(&format!("BEGIN; CREATE TABLE {s}.late (x int)"))
            .await;
        let (n1, _, _) = refresh(&mut c, &ns, &prev).await;
        assert!(
            find(&n1, ObjectKind::Table, &["late"]).is_none(),
            "{}",
            t.id
        );
        open.batch("COMMIT").await;
        open.close().await;
        let (n2, _, p) = refresh(&mut c, &ns, &n1).await;
        assert!(p.report.full_reason.is_none(), "{}", t.id);
        assert!(
            find(&n2, ObjectKind::Table, &["late"]).is_some(),
            "{}",
            t.id
        );
        assert_same(&t.id, &n2, &load(&mut c, &ns, Level::Structure).await);
        drop(c);
        pg.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn set_schema_is_one_move_when_both_in_scope() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (a, _ga) = schema_with(&t, "mva", "CREATE TABLE {s}.m (x int)").await;
        let (b, _gb) = schema_with(&t, "mvb", "").await;
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let (na, nb) = (ns_of(&t, &a), ns_of(&t, &b));
        let pa = load(&mut c, &na, Level::Structure).await;
        let pb = load(&mut c, &nb, Level::Structure).await;
        run(&t, &format!("ALTER TABLE {a}.m SET SCHEMA {b}")).await;
        let (_, da, _) = refresh(&mut c, &na, &pa).await;
        let (_, db, _) = refresh(&mut c, &nb, &pb).await;
        let all = CatalogDelta::combine(vec![(na.clone(), da), (nb.clone(), db)]);
        assert_eq!(all.moved.len(), 1, "{}: {all:?}", t.id);
        assert!(all.dropped.is_empty() && all.created.is_empty(), "{}", t.id);
        drop(c);
        pg.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn case_twins_users_and_quoted_users_are_two_tables() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (s, _g) = schema_with(
            &t,
            "twins",
            "CREATE TABLE {s}.users (a int); CREATE TABLE {s}.\"Users\" (b int)",
        )
        .await;
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let n = load(&mut c, &ns_of(&t, &s), Level::Structure).await;
        let a = get(&n, ObjectKind::Table, &["users"]);
        let b = get(&n, ObjectKind::Table, &["Users"]);
        assert_ne!(a.server_id(), b.server_id(), "{}", t.id);
        drop(c);
        pg.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn key_order_and_expression_include_partial_index() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (s, _g) = schema_with(
            &t,
            "keys",
            "CREATE TABLE {s}.k (a int, b int, c int, CONSTRAINT k_ba UNIQUE (b, a)); \
             CREATE INDEX k_expr ON {s}.k ((a + b) DESC) INCLUDE (c) WHERE a > 0",
        )
        .await;
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let n = load(&mut c, &ns_of(&t, &s), Level::Structure).await;
        let Props::Key(u) = get(&n, ObjectKind::UniqueKey, &["k", "k_ba"]).props() else {
            panic!()
        };
        let cols: Vec<_> = u
            .columns
            .iter()
            .map(|c| (c.ordinal, c.name.as_deref().unwrap_or("")))
            .collect();
        assert_eq!(cols, [(2, "b"), (1, "a")], "{}", t.id);
        let Props::Index(i) = get(&n, ObjectKind::Index, &["k", "k_expr"]).props() else {
            panic!()
        };
        assert_eq!(i.key.columns[0].ordinal, 0, "{}: an expression slot", t.id);
        assert!(i.key.expressions[0].contains("a + b"), "{}", t.id);
        assert_eq!(i.key.desc, [true], "{}", t.id);
        assert_eq!(
            i.key.include.iter().map(|c| &**c).collect::<Vec<_>>(),
            ["c"]
        );
        assert_eq!(i.key.predicate.as_deref(), Some("(a > 0)"), "{}", t.id);
        drop(c);
        pg.shutdown().await;
    }
}

// ---------------------------------------------------------------------------------------------
// State (HV-INTRO-15)

async fn stamp_of(c: &mut MetaConn, key: &ObjectKey) -> Option<StateMark> {
    let cancel = CancellationToken::new();
    state::stamps(c, std::slice::from_ref(key), &cancel)
        .await
        .expect("stamps")
        .0
        .remove(0)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn check_state_changed_after_comment_and_dropped_after_recreate() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (s, _g) = schema_with(&t, "state", "CREATE TABLE {s}.x (a int)").await;
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let n = load(&mut c, &ns_of(&t, &s), Level::Structure).await;
        let key = key_of(&n, ObjectKind::Table, &["x"]);
        let s1 = stamp_of(&mut c, &key).await.expect("exists");
        run(&t, &format!("COMMENT ON TABLE {s}.x IS 'c'")).await;
        let s2 = stamp_of(&mut c, &key).await.expect("exists");
        assert_ne!(s1, s2, "{}: the comment moves the stamp", t.id);
        run(&t, &format!("DROP TABLE {s}.x; CREATE TABLE {s}.x (a int)")).await;
        assert_eq!(
            stamp_of(&mut c, &key).await,
            None,
            "{}: the old oid is gone",
            t.id
        );
        let st = hacivat_introspect::pg::PgIntrospector
            .check_state(&mut c, std::slice::from_ref(&key))
            .await
            .unwrap();
        assert_eq!(st, [Staleness::Dropped], "{}", t.id);
        drop(c);
        pg.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn validation_with_unchanged_server_runs_state_queries_only() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (s, _g) = schema_with(
            &t,
            "valid",
            &format!(
                "{X}; COMMENT ON TABLE {{s}}.x IS 'c'; \
                 CREATE DOMAIN {{s}}.d AS int CONSTRAINT d_pos CHECK (VALUE > 0)"
            ),
        )
        .await;
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let ns = ns_of(&t, &s);
        let n = load(&mut c, &ns, Level::Structure).await;
        let cancel = CancellationToken::new();
        let (check, report) = pass::check_namespace(&mut c, &ns, &n, &cancel)
            .await
            .unwrap();
        // Changes that leave no young row still move the digest (review I-3).
        let mut last = check.existence_digest;
        for step in [
            format!("COMMENT ON TABLE {s}.x IS NULL"),
            format!("ALTER DOMAIN {s}.d DROP CONSTRAINT d_pos"),
        ] {
            run(&t, &step).await;
            let (again, _) = pass::check_namespace(&mut c, &ns, &n, &cancel)
                .await
                .unwrap();
            assert_ne!(again.existence_digest, last, "{}: {step}", t.id);
            last = again.existence_digest;
        }
        let Watermark::PgXid { w, .. } = n.meta().watermark else {
            panic!()
        };
        assert_eq!(
            check.max_mark,
            StateMark::Xmin((w & 0xFFFF_FFFF) as u32),
            "{}",
            t.id
        );
        for s in report.log.names() {
            assert!(
                [
                    name::IDENTITY,
                    name::LIST_EXISTENT,
                    name::LIST_EXISTENT_COMMENTS,
                    name::CHANGED_PARENTS,
                    name::CHANGED_DEPENDENTS,
                    name::CANARY
                ]
                .contains(&s),
                "{}: {s} ran",
                t.id
            );
        }
        let key = key_of(&n, ObjectKind::Table, &["x"]);
        let (_, r) = state::stamps(&mut c, &[key], &cancel).await.unwrap();
        assert_eq!(
            r.log.names(),
            [name::IDENTITY, name::STATE_OF_RELATION, name::CANARY]
        );
        drop(c);
        pg.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn stamp_moves_on_comment_removal_second_trigger_and_dependency_rename() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (s, _g) = schema_with(
            &t,
            "stamp",
            &format!("CREATE TYPE {{s}}.e AS ENUM ('a'); {TRG}; ALTER TABLE {{s}}.x ADD COLUMN v {{s}}.e; COMMENT ON TABLE {{s}}.x IS 'c'"),
        )
        .await;
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let n = load(&mut c, &ns_of(&t, &s), Level::Structure).await;
        let key = key_of(&n, ObjectKind::Table, &["x"]);
        let mut last = stamp_of(&mut c, &key).await.unwrap();
        for step in [
            format!("COMMENT ON TABLE {s}.x IS NULL"),
            format!(
                "CREATE TRIGGER t2 AFTER UPDATE ON {s}.x FOR EACH ROW EXECUTE FUNCTION {s}.f()"
            ),
            format!("ALTER TYPE {s}.e RENAME TO e2"),
        ] {
            run(&t, &step).await;
            let now = stamp_of(&mut c, &key).await.unwrap();
            assert_ne!(now, last, "{}: {step}", t.id);
            last = now;
        }
        // A column rename touches pg_attribute only (review I-2): the stamp of a view that
        // selects the column, and of a table whose FK names the referenced column, moves.
        run(
            &t,
            &format!(
                "CREATE TABLE {s}.p (id int PRIMARY KEY); \
                 ALTER TABLE {s}.x ADD COLUMN pid int REFERENCES {s}.p (id); \
                 CREATE VIEW {s}.vw AS SELECT a FROM {s}.x"
            ),
        )
        .await;
        let n = load(&mut c, &ns_of(&t, &s), Level::Structure).await;
        let view = key_of(&n, ObjectKind::View, &["vw"]);
        let table = key_of(&n, ObjectKind::Table, &["x"]);
        let (v0, t0) = (
            stamp_of(&mut c, &view).await,
            stamp_of(&mut c, &table).await,
        );
        run(&t, &format!("ALTER TABLE {s}.x RENAME COLUMN a TO a2")).await;
        let v1 = stamp_of(&mut c, &view).await;
        assert_ne!(v1, v0, "{}: a view's renamed column", t.id);
        let t1 = stamp_of(&mut c, &table).await;
        run(&t, &format!("ALTER TABLE {s}.p RENAME COLUMN id TO id2")).await;
        assert_ne!(
            stamp_of(&mut c, &table).await,
            t1,
            "{}: an FK's renamed column",
            t.id
        );
        assert!(t0.is_some() && v1.is_some());
        drop(c);
        pg.shutdown().await;
    }
}

// ---------------------------------------------------------------------------------------------
// Every section (HV-INTRO-04 test 7)

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn every_pg_section_prepares_and_returns_its_declared_columns() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (s, _g) = schema_with(&t, "every", X).await;
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let n = load(&mut c, &ns_of(&t, &s), Level::Structure).await;
        let Some(ServerId::Oid(ns_oid)) = n.root().server_id() else {
            panic!()
        };
        let Some(ServerId::Oid(x)) = get(&n, ObjectKind::Table, &["x"]).server_id() else {
            panic!()
        };
        let p = pack().unwrap();
        // Every variant is rendered at the server's own `server_version_num` (review I-6): a
        // variant gated above it (an 18+ column on pg17) is the other branch there.
        let cancel = CancellationToken::new();
        let version = pass::server_identity(&mut c, &cancel)
            .await
            .unwrap()
            .version;
        let mut snap = c.begin_snapshot().await.unwrap();
        for section in p.sections() {
            let decl: Vec<String> = p
                .section(section)
                .unwrap()
                .returns()
                .iter()
                .map(|c| c.name.to_string())
                .collect();
            let mut ran = 0;
            let mut seen = std::collections::BTreeSet::new();
            for v in p.reachable_variants(section) {
                let binds = Binds {
                    ns: Some(if v.mode == Mode::System { 11 } else { ns_oid }),
                    w: Some(3),
                    schema: Some(s.clone().into()),
                    oids: Some(vec![x]),
                };
                let cx = ctx(version, v.mode, v.level, &binds);
                for q in p.render(section, &cx).unwrap() {
                    if !seen.insert(q.sql().to_string()) {
                        continue;
                    }
                    let rows = snap
                        .rows(q.as_meta_query())
                        .await
                        .unwrap_or_else(|e| panic!("{} {section} {:?}: {e:?}", t.id, v.mode));
                    let got: Vec<String> = rows.columns.iter().map(|c| c.to_string()).collect();
                    assert_eq!(got, decl, "{} {section}", t.id);
                    ran += 1;
                }
            }
            assert!(ran >= 1, "{} {section}: never ran", t.id);
        }
        snap.commit().await.unwrap();
        drop(c);
        pg.shutdown().await;
    }
}

// ---------------------------------------------------------------------------------------------
// Findings 3–6

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn comment_removal_is_seen_by_existence() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (n, _, p, _) = step_case(
            &t,
            "uncmt",
            &format!("{X}; COMMENT ON TABLE {{s}}.x IS 'gone soon'"),
            "COMMENT ON TABLE {s}.x IS NULL",
            Level::Structure,
        )
        .await;
        assert_eq!(
            get(&n, ObjectKind::Table, &["x"]).props().comment(),
            None,
            "{}",
            t.id
        );
        assert!(ran(&p, name::COMMENTS), "{}", t.id);
        assert!(!ran(&p, name::RETRIEVE_COLUMNS) && !ran(&p, name::RETRIEVE_RELATIONS));
    }
}

const RENAMES: &str = "CREATE TYPE {s}.ty AS ENUM ('a'); \
    CREATE TABLE {s}.parent (id int PRIMARY KEY); \
    CREATE TABLE {s}.r (total int CHECK (total >= 0), g int GENERATED ALWAYS AS (total * 2) STORED, \
      t {s}.ty, pid int REFERENCES {s}.parent (id)); \
    CREATE VIEW {s}.v AS SELECT total FROM {s}.r; \
    CREATE FUNCTION {s}.tf() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$; \
    CREATE TRIGGER rt BEFORE INSERT ON {s}.r FOR EACH ROW EXECUTE FUNCTION {s}.tf()";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn rename_of_a_referenced_object_rereads_dependent_text() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (s, _g) = schema_with(&t, "ren", RENAMES).await;
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let ns = ns_of(&t, &s);
        let mut prev = load(&mut c, &ns, Level::Sources).await;
        for step in [
            "ALTER TABLE {s}.r RENAME COLUMN total TO amount",
            "ALTER TYPE {s}.ty RENAME TO ty2",
            "ALTER TABLE {s}.parent RENAME TO parent2",
            "ALTER FUNCTION {s}.tf() RENAME TO tf2",
        ] {
            run(&t, &step.replace("{s}", &s)).await;
            let (next, _, p) = refresh(&mut c, &ns, &prev).await;
            assert!(p.report.full_reason.is_none(), "{}: {step}", t.id);
            assert_same(
                &format!("{} {step}", t.id),
                &next,
                &load(&mut c, &ns, Level::Sources).await,
            );
            prev = next;
        }
        // A renamed *schema* is two hops away (review I-1): an FK onto its table, a trigger
        // calling its function, a default calling its sequence and a view selecting its table
        // are re-read through a full pass (reason `schema-rename`).
        let o = format!("{s}_o");
        let _go = DropSchemas {
            t: t.clone(),
            names: vec![o.clone(), format!("{o}2")],
        };
        run(
            &t,
            &format!(
                "CREATE SCHEMA {o}; CREATE TABLE {o}.ot (id int PRIMARY KEY); \
                 CREATE SEQUENCE {o}.os; \
                 CREATE FUNCTION {o}.ofn() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$; \
                 CREATE TABLE {s}.dep (oid_ref int REFERENCES {o}.ot (id), n bigint DEFAULT nextval('{o}.os')); \
                 CREATE TRIGGER dt BEFORE INSERT ON {s}.dep FOR EACH ROW EXECUTE FUNCTION {o}.ofn(); \
                 CREATE VIEW {s}.dv AS SELECT id FROM {o}.ot"
            ),
        )
        .await;
        let (next, _, _) = refresh(&mut c, &ns, &prev).await;
        prev = next;
        run(&t, &format!("ALTER SCHEMA {o} RENAME TO {o}2")).await;
        let p = pass_at(&mut c, &ns, Mode::Incremental, Level::Sources, Some(&prev)).await;
        assert_eq!(
            p.report.full_reason,
            Some(FullReason::SchemaRename),
            "{}",
            t.id
        );
        let (next, _) = apply(&prev, p.result);
        assert_same(
            &format!("{} schema rename", t.id),
            &next,
            &load(&mut c, &ns, Level::Sources).await,
        );
        let Props::ForeignKey(fk) = get(
            &next,
            ObjectKind::ForeignKey,
            &[
                "dep",
                &children_of(&next, ObjectKind::Table, &["dep"], ObjectKind::ForeignKey)[0],
            ],
        )
        .props() else {
            panic!()
        };
        assert_eq!(fk.target.path[1].as_ref(), format!("{o}2"), "{}", t.id);
        drop(c);
        pg.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn rename_table_rereads_backing_index_text() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (n, _, _, _) = step_case(
            &t,
            "renix",
            "CREATE TABLE {s}.parent (id int PRIMARY KEY, code text UNIQUE)",
            "ALTER TABLE {s}.parent RENAME TO parent2",
            Level::Sources,
        )
        .await;
        // step_case compared every property (the index's definition SourceRef included) with
        // a fresh full pass; the key now lives under the new name.
        assert!(
            find(&n, ObjectKind::Index, &["parent2", "parent_pkey"]).is_some(),
            "{}",
            t.id
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn xid_burst_between_probe_and_change_query_misses_nothing() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (s, _g) = schema_with(&t, "burst", "CREATE TABLE {s}.a (x int)").await;
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let ns = ns_of(&t, &s);
        let prev = load(&mut c, &ns, Level::Structure).await;
        // After this pass's Identity, another session commits a CREATE TABLE and then assigns
        // 200 XIDs: the next pass's watermark is this pass's snapshot xmin, and a client-side
        // `txage` margin of 64 would miss the table (finding 6).
        let (tt, ss) = (t.clone(), s.clone());
        let hook: pass::Hook = Arc::new(move || {
            let (tt, ss) = (tt.clone(), ss.clone());
            Box::pin(async move {
                let mut a = admin(&tt).await;
                a.batch(&format!("CREATE TABLE {ss}.burst (x int)")).await;
                // One autocommit statement each: a multi-statement text would be one
                // implicit transaction, and assign one XID.
                for _ in 0..200 {
                    a.batch("SELECT pg_catalog.txid_current()").await;
                }
                a.close().await;
            })
        });
        let cancel = CancellationToken::new();
        let p1 = pass::introspect(
            &mut c,
            &ns,
            PassOpts {
                mode: Mode::Incremental,
                level: Level::Structure,
                prev: Some(&prev),
                after_identity: Some(hook),
                ..PassOpts::default()
            },
            &cancel,
        )
        .await
        .unwrap();
        let (n1, _) = apply(&prev, p1.result);
        let (n2, _, p2) = refresh(&mut c, &ns, &n1).await;
        assert!(p2.report.full_reason.is_none(), "{}", t.id);
        assert!(
            find(&n2, ObjectKind::Table, &["burst"]).is_some(),
            "{}: the burst table",
            t.id
        );
        assert_same(&t.id, &n2, &load(&mut c, &ns, Level::Structure).await);
        drop(c);
        pg.shutdown().await;
    }
}

// ---------------------------------------------------------------------------------------------
// Version facts (R-M4-47)

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn version_specific_not_null_constraints() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (s, _g) = schema_with(
            &t,
            "nn",
            "CREATE DOMAIN {s}.d AS int NOT NULL; CREATE TABLE {s}.x (a int NOT NULL, b int NOT NULL, c int)",
        )
        .await;
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let n = load(&mut c, &ns_of(&t, &s), Level::Structure).await;
        let Props::Type(d) = get(&n, ObjectKind::Domain, &["d"]).props() else {
            panic!()
        };
        let TypeBody::Domain {
            not_null_constraint,
            ..
        } = &d.body
        else {
            panic!()
        };
        assert_eq!(
            usize::from(not_null_constraint.is_some()),
            usize::from(major(&t) >= 17),
            "{}",
            t.id
        );
        let named = ["a", "b", "c"]
            .into_iter()
            .filter(|col| {
                get(&n, ObjectKind::Column, &["x", col])
                    .props()
                    .as_column()
                    .and_then(|p| p.rare.as_ref())
                    .is_some_and(|r| r.not_null_constraint.is_some())
            })
            .count();
        if major(&t) >= 18 {
            assert!(named >= 2, "{}", t.id);
        } else {
            assert_eq!(named, 0, "{}", t.id);
        }
        drop(c);
        pg.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_ordinal_is_attnum_with_gaps_after_drop_column() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (s, _g) = schema_with(
            &t,
            "ord",
            "CREATE TABLE {s}.x (a int, b int, c int); ALTER TABLE {s}.x DROP COLUMN b",
        )
        .await;
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let n = load(&mut c, &ns_of(&t, &s), Level::Structure).await;
        let ord = |col| {
            get(&n, ObjectKind::Column, &["x", col])
                .props()
                .as_column()
                .unwrap()
                .ordinal
        };
        assert_eq!((ord("a"), ord("c")), (1, 3), "{}", t.id);
        drop(c);
        pg.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn nulls_not_distinct_read_on_15_plus_false_before() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let ddl = if major(&t) >= 15 {
            "CREATE TABLE {s}.x (a int); CREATE UNIQUE INDEX x_a ON {s}.x (a) NULLS NOT DISTINCT"
        } else {
            "CREATE TABLE {s}.x (a int); CREATE UNIQUE INDEX x_a ON {s}.x (a)"
        };
        let (s, _g) = schema_with(&t, "nnd", ddl).await;
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let n = load(&mut c, &ns_of(&t, &s), Level::Structure).await;
        let Props::Index(i) = get(&n, ObjectKind::Index, &["x", "x_a"]).props() else {
            panic!()
        };
        assert_eq!(i.key.nulls_not_distinct, major(&t) >= 15, "{}", t.id);
        drop(c);
        pg.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_pk_backing_index_is_implicit_for_the_pk() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (s, _g) = schema_with(&t, "pk", X).await;
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let n = load(&mut c, &ns_of(&t, &s), Level::Structure).await;
        let ix = get(&n, ObjectKind::Index, &["x", "x_pkey"]);
        assert!(ix.flags().contains(ObjFlags::IMPLICIT), "{}", t.id);
        let Props::Index(ip) = ix.props() else {
            panic!()
        };
        let r = ip.key.implicit_for.as_ref().expect("implicit_for");
        assert_eq!(
            (r.kind, r.path.last().map(|p| &**p)),
            (ObjectKind::PrimaryKey, Some("x_pkey"))
        );
        drop(c);
        pg.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_row_estimate_minus_one_is_none() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (s, _g) = schema_with(&t, "est", "CREATE TABLE {s}.x (a int)").await;
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let ns = ns_of(&t, &s);
        let est = |p: &PgPass| {
            p.stats
                .iter()
                .find(|(k, _)| k.name() == "x")
                .map(|(_, f)| f.row_estimate)
        };
        let p = pass_at(&mut c, &ns, Mode::Full, Level::Structure, None).await;
        // `reltuples` is -1 before the first VACUUM or ANALYZE from PostgreSQL 14 (0 on 13).
        let want = if major(&t) >= 14 { None } else { Some(0) };
        assert_eq!(est(&p), Some(want), "{}", t.id);
        run(
            &t,
            &format!("INSERT INTO {s}.x VALUES (1), (2), (3); ANALYZE {s}.x"),
        )
        .await;
        let p = pass_at(&mut c, &ns, Mode::Full, Level::Structure, None).await;
        assert_eq!(est(&p), Some(Some(3)), "{}", t.id);
        drop(c);
        pg.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn server_facts_filled_per_engine() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (s, _g) = schema_with(&t, "facts", "").await;
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let p = pass_at(&mut c, &ns_of(&t, &s), Mode::Full, Level::Names, None).await;
        let f = p.report.facts.expect("facts");
        assert_eq!(f.dbms, DbmsKind::Postgres);
        assert_eq!(f.version_num / 10_000, major(&t), "{}", t.id);
        assert!(f.version.starts_with(&t.major), "{}: {}", t.id, f.version);
        assert_eq!(f.standard_conforming_strings, Some(true), "{}", t.id);
        assert_eq!(f.in_recovery, Some(false), "{}", t.id);
        assert_eq!(f.ident.encoding.as_deref(), Some("UTF8"), "{}", t.id);
        assert!(
            f.default_collation.contains_key(t.database.as_str()),
            "{}",
            t.id
        );
        drop(c);
        pg.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn system_mode_reads_only_oids_below_16384_with_pinned_path() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let ns = ns_of(&t, "pg_catalog");
        let p = pass_at(&mut c, &ns, Mode::System, Level::Structure, None).await;
        assert!(p.result.full, "{}", t.id);
        let b = base(ns.clone(), DbmsKind::Postgres, FoldRule::Exact).unwrap();
        let (n, _) = apply(&b, p.result);
        for o in n.objs().iter().skip(1) {
            if let Some(ServerId::Oid(oid)) = o.server_id() {
                assert!(oid < 16_384, "{}: {} {oid}", t.id, o.name());
            }
        }
        assert!(
            find(&n, ObjectKind::Table, &["pg_class"]).is_some(),
            "{}",
            t.id
        );
        assert!(
            n.objs().iter().any(|o| o.kind() == ObjectKind::Operator),
            "{}",
            t.id
        );
        // The pinned search_path: built-in types print unqualified.
        let ty = get(&n, ObjectKind::Column, &["pg_class", "relname"])
            .props()
            .as_column()
            .unwrap()
            .raw_type
            .clone();
        assert_eq!(&*ty, "name", "{}", t.id);
        let cancel = CancellationToken::new();
        let rows = system::read_rows(&mut c, "information_schema", &cancel)
            .await
            .unwrap();
        assert!(
            rows.relations.iter().any(|r| r.name == "tables"),
            "{}",
            t.id
        );
        assert!(rows.relations.iter().all(|r| r.oid < 16_384), "{}", t.id);
        let gucs = system::list_settings(&mut c, &cancel).await.unwrap();
        assert!(gucs.iter().any(|g| g.name == "search_path"), "{}", t.id);
        assert!(gucs.iter().all(|g| !g.name.contains('.')), "{}", t.id);
        drop(c);
        pg.shutdown().await;
    }
}

// ---------------------------------------------------------------------------------------------
// pg17 only (full set): foreign watermarks and the big catalog

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn foreign_watermarks_force_a_full_pass() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("pg17") else {
        return;
    };
    let (s, _g) = schema_with(&t, "foreign", "CREATE TABLE {s}.a (x int)").await;
    let pg = Pg::connect(&t).await;
    let mut c = pg.meta().await;
    let ns = ns_of(&t, &s);
    let first = load(&mut c, &ns, Level::Structure).await;
    let Watermark::PgXid { w, identity } = first.meta().watermark.clone() else {
        panic!()
    };
    let mut cases = Vec::new();
    cases.push((
        forged(&mut c, &ns, Watermark::None).await,
        FullReason::NoWatermark,
    ));
    let mut other = identity;
    other.start_time_micros += 1;
    cases.push((
        forged(&mut c, &ns, Watermark::PgXid { w, identity: other }).await,
        FullReason::Restart,
    ));
    let mut other = identity;
    other.system_identifier ^= 1 << 40;
    cases.push((
        forged(&mut c, &ns, Watermark::PgXid { w, identity: other }).await,
        FullReason::Identity,
    ));
    // A watermark ahead of the server's next XID (a restore, `pg_resetwal`).
    cases.push((
        forged(
            &mut c,
            &ns,
            Watermark::PgXid {
                w: w + (1 << 30),
                identity,
            },
        )
        .await,
        FullReason::Identity,
    ));
    for (prev, reason) in cases {
        let p = pass_at(
            &mut c,
            &ns,
            Mode::Incremental,
            Level::Structure,
            Some(&prev),
        )
        .await;
        assert_eq!(p.report.full_reason, Some(reason), "{reason:?}");
        let (n, _) = apply(&prev, p.result);
        assert_same(reason.code(), &n, &first);
    }
    drop(c);
    pg.shutdown().await;
}

/// A cached namespace with a forged watermark: a full pass merged with `wm`.
async fn forged(c: &mut MetaConn, ns: &NsKey, wm: Watermark) -> NamespaceSnapshot {
    let mut p = pass_at(c, ns, Mode::Full, Level::Structure, None).await;
    p.result.new_watermark = wm;
    let b = base(ns.clone(), DbmsKind::Postgres, FoldRule::Exact).unwrap();
    apply(&b, p.result).0
}

/// R-W3-2: the scope filter reaches the server as `relname LIKE $n ESCAPE $m` (`C`
/// collation; one run per pattern), the client filter still decides, and the progress sink
/// sees each section's phase and rows.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn scope_filter_pushdown_reads_only_matching_relations() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (s, _g) = schema_with(
            &t,
            "push",
            "CREATE TABLE {s}.orders_a (a int); CREATE TABLE {s}.\"Orders_b\" (a int); \
             CREATE TABLE {s}.ordersx (a int); CREATE TABLE {s}.other (a int)",
        )
        .await;
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let ns = ns_of(&t, &s);
        let f = hacivat_introspect::filter::parse_filter(
            "orders_*",
            sql_dialect::DialectId::Postgres,
            hacivat_introspect::filter::FilterRole::Introspect,
        )
        .unwrap();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink_seen = seen.clone();
        let sink: hacivat_introspect::pg::sections::ProgressSink =
            Arc::new(move |phase, rows| sink_seen.lock().unwrap().push((phase, rows)));
        let cancel = CancellationToken::new();
        let p = pass::introspect(
            &mut c,
            &ns,
            PassOpts {
                mode: Mode::Full,
                level: Level::Structure,
                filter: Some(&f),
                progress: Some(sink),
                ..PassOpts::default()
            },
            &cancel,
        )
        .await
        .unwrap();
        let rels = p
            .report
            .log
            .sections
            .iter()
            .find(|x| x.name == name::RETRIEVE_RELATIONS)
            .unwrap();
        // Exact on PostgreSQL: `Orders_b` and `ordersx` are not `orders_*` (the `_` is
        // literal), and `other` never left the server.
        assert_eq!(rels.rows, 1, "{}", t.id);
        let b = base(ns.clone(), DbmsKind::Postgres, FoldRule::Exact).unwrap();
        let (n, _) = apply(&b, p.result);
        let tables: Vec<String> = n
            .objs()
            .iter()
            .filter(|o| o.kind() == ObjectKind::Table)
            .map(|o| o.name().to_string())
            .collect();
        assert_eq!(tables, ["orders_a"], "{}", t.id);
        let seen = seen.lock().unwrap().clone();
        assert!(
            seen.contains(&(hacivat_introspect::Phase::Tables, 1)),
            "{}: {seen:?}",
            t.id
        );
        drop(c);
        pg.shutdown().await;
    }
}

const BIG_TABLES: u32 = 15_000;
const BIG_COLUMNS: u32 = 160_000;

/// `hv_m4_big` on pg17, created once per test process (idempotent). Tests run in parallel:
/// two concurrent `CREATE TABLE IF NOT EXISTS` of one table race on `pg_type`'s unique
/// index (23505, CI 44640824635), so the build runs once behind a process-wide cell.
async fn big(t: &DockerTarget) {
    static BUILT: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    BUILT
        .get_or_init(|| async {
            let mut a = admin(t).await;
            big_catalog::ensure_pg(a.live(), BIG_TABLES, BIG_COLUMNS)
                .await
                .unwrap_or_else(|e| panic!("hv_m4_big: {e:?}"));
            a.close().await;
        })
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn no_catalog_query_seq_scans_user_sized_structures_without_a_namespace_predicate() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("pg17") else {
        return;
    };
    big(&t).await;
    let mut a = admin(&t).await;
    let ns_oid = a
        .scalar("SELECT oid FROM pg_catalog.pg_namespace WHERE nspname = 'hv_m4_big'")
        .await;
    let t1 = a
        .scalar("SELECT oid FROM pg_catalog.pg_class WHERE relname = 't1' AND relnamespace = 'hv_m4_big'::regnamespace")
        .await;
    let p = pack().unwrap();
    // User-sized catalogs: a sequential scan of one needs a namespace or xmin filter (the
    // whole-database young-row scans of R-M4-48 filter on xmin).
    const BIG: [&str; 9] = [
        "pg_class",
        "pg_attribute",
        "pg_constraint",
        "pg_index",
        "pg_depend",
        "pg_description",
        "pg_trigger",
        "pg_type",
        "pg_proc",
    ];
    let mut violations = Vec::new();
    for section in p.sections() {
        if section.starts_with("System") || &**section == name::LIST_SETTINGS {
            continue;
        }
        let binds = Binds {
            ns: Some(ns_oid.parse().unwrap()),
            w: Some(3),
            schema: Some("hv_m4_big".into()),
            oids: Some(vec![t1.parse().unwrap()]),
        };
        for v in p.reachable_variants(section) {
            for q in p
                .render(section, &ctx(170_000, v.mode, v.level, &binds))
                .unwrap()
            {
                // Binds are inlined here only: test text on an admin connection.
                let mut sql = q.sql().to_string();
                for (k, b) in q.binds().iter().enumerate().rev() {
                    let value = match b {
                        hacivat_values::BindValue::Typed { text, .. } => format!("'{text}'"),
                        hacivat_values::BindValue::Null => "NULL".to_string(),
                    };
                    sql = sql.replace(&format!("${}", k + 1), &value);
                }
                a.batch("BEGIN").await;
                // ## m4-t16b-pg-facets (named exception, CI 44658780330): with sequential
                // scans disabled for this transaction only, the planner keeps a Seq Scan only
                // where no index path exists, which is the violation this test looks for. A
                // Seq Scan it picked on cost (a small `pg_description` after other tests
                // added comments, `StateOfRelation`) is not one.
                a.batch("SET LOCAL enable_seqscan = off").await;
                let plan = a.scalar(&format!("EXPLAIN (FORMAT JSON) {sql}")).await;
                a.batch("ROLLBACK").await;
                // Per scan node (review m-3): a sequential scan of a user-sized catalog must
                // filter on the namespace, on xmin (the young-row scans; the whole-database
                // ones are R-M4-48's design), or on the oid list. The one exception is a
                // whole-namespace read (a full-mode section whose plan filters the namespace
                // on its driving relation), whose hash-joined child scans are the cheapest
                // plan when the namespace is most of the database, as `hv_m4_big` is; and the
                // dependency CTE of `ChangedDependents`, which reads `pg_depend` whole by
                // design.
                let whole_ns = v.mode != Mode::Incremental
                    && !section.starts_with("StateOf")
                    && &**section != name::COMMENTS
                    && plan.contains("namespace");
                for catalog in BIG {
                    let needle = format!("\"Relation Name\": \"{catalog}\"");
                    for (i, _) in plan.match_indices(&needle) {
                        let node = &plan[plan[..i].rfind('{').unwrap_or(0)..];
                        let node = &node[..node.find('}').unwrap_or(node.len())];
                        let filtered = node.contains("namespace")
                            || node.contains("xmin")
                            || node.contains(&t1);
                        let by_design =
                            &**section == name::CHANGED_DEPENDENTS && catalog == "pg_depend";
                        if node.contains("\"Seq Scan\"") && !filtered && !whole_ns && !by_design {
                            violations.push(format!("{section} {:?}: {catalog}", v.mode));
                        }
                    }
                }
            }
        }
    }
    a.close().await;
    assert!(violations.is_empty(), "{violations:#?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn big_catalog_cold_and_incremental_timings() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("pg17") else {
        return;
    };
    big(&t).await;
    let pg = Pg::connect(&t).await;
    let mut c = pg.meta().await;
    let ns = ns_of(&t, big_catalog::SCHEMA);
    let t0 = Instant::now();
    let names = pass_at(&mut c, &ns, Mode::Full, Level::Names, None).await;
    let names_ms = t0.elapsed().as_millis();
    let t0 = Instant::now();
    let n = load(&mut c, &ns, Level::Structure).await;
    let structure_ms = t0.elapsed().as_millis();
    assert_eq!(n.column_count(), BIG_COLUMNS as usize);
    let t0 = Instant::now();
    let (_, delta, p) = refresh(&mut c, &ns, &n).await;
    let inc_ms = t0.elapsed().as_millis();
    assert!(delta.is_empty() && p.report.full_reason.is_none());
    assert!(
        !p.report
            .log
            .names()
            .iter()
            .any(|s| s.starts_with("Retrieve"))
    );
    assert!(!ran(&names, name::RETRIEVE_COLUMNS));
    eprintln!(
        "hv_m4_big (debug build): Names {names_ms} ms, Structure {structure_ms} ms, no-change incremental {inc_ms} ms"
    );
    drop(c);
    pg.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn twenty_thousand_and_one_tables_auto_level_names_and_source_fragment_only() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("pg17") else {
        return;
    };
    // A 20,001-table scratch schema (one column each), built in 250-table DO blocks and
    // dropped the same way by its guard (review I-5).
    let s = unique("hvm4_t16_k20");
    let _g = common::pg::DropBatchedSchema {
        t: t.clone(),
        schema: s.clone(),
        tables: 20_001,
    };
    let mut a = admin(&t).await;
    a.batch(&format!("CREATE SCHEMA {s}")).await;
    let mut first = 1;
    while first <= 20_001 {
        let last = (first + 249).min(20_001);
        let body: String = (first..=last)
            .map(|i| format!("CREATE TABLE {s}.t{i} (a int);\n"))
            .collect();
        a.batch(&format!("DO $b$ BEGIN\n{body}END $b$")).await;
        first = last + 1;
    }
    a.close().await;
    let pg = Pg::connect(&t).await;
    let mut c = pg.meta().await;
    let ns = ns_of(&t, &s);
    // The Names level (what the auto level picks above 20,000 tables, D-038; LevelPolicy is
    // Task 15's) reads names only.
    let brief = pass_at(&mut c, &ns, Mode::Brief, Level::Structure, None).await;
    assert_eq!(brief.result.level, Level::Names);
    assert!(!ran(&brief, name::RETRIEVE_COLUMNS) && !ran(&brief, name::RETRIEVE_INDEXES));
    let b = base(ns.clone(), DbmsKind::Postgres, FoldRule::Exact).unwrap();
    let (n, _) = apply(&b, brief.result);
    assert_eq!(
        n.objs()
            .iter()
            .filter(|o| o.kind() == ObjectKind::Table)
            .count(),
        20_001
    );
    // A source fragment of one table reads that table only.
    let key = key_of(&n, ObjectKind::Table, &["t7"]);
    let cancel = CancellationToken::new();
    let f = pass::fragment(&mut c, &ns, &[key], Level::Sources, None, &cancel)
        .await
        .unwrap();
    assert_eq!(f.result.changed.len(), 1);
    let cols = f
        .report
        .log
        .sections
        .iter()
        .find(|x| x.name == name::RETRIEVE_COLUMNS)
        .unwrap();
    assert_eq!((cols.queries, cols.rows), (1, 1));
    assert!(!f.result.full && f.result.new_watermark == Watermark::None);
    drop(c);
    pg.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn drop_big_fixture_when_asked() {
    if !docker_enabled() || std::env::var("HACIVAT_M4_CLEANUP").as_deref() != Ok("1") {
        return;
    }
    let Some(t) = require_target("pg17") else {
        return;
    };
    let mut a = admin(&t).await;
    big_catalog::drop_pg(a.live(), BIG_TABLES).await.unwrap();
    let left = a
        .scalar("SELECT count(*) FROM pg_catalog.pg_namespace WHERE nspname = 'hv_m4_big'")
        .await;
    assert_eq!(left, "0");
    a.close().await;
}

// ## m4-t28-routine-lookup (named exception, ruling R-W5-9)
/// An unchanged routine is not re-read (G4 finding, CI 44692679308). The incremental pass
/// used to look cached objects up by a key without the overload signature, which never matched
/// a routine, so every routine counted as uncached on every pass. Overloads, a zero-argument
/// function, a procedure and an aggregate; then one changed routine is re-read alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn insert_only_pass_with_routines_rereads_no_routine() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let (s, _g) = schema_with(
            &t,
            "rtn",
            "CREATE TABLE {s}.x (a int); \
             CREATE FUNCTION {s}.f0() RETURNS int LANGUAGE sql AS 'SELECT 1'; \
             CREATE FUNCTION {s}.f2(a int, b text) RETURNS int LANGUAGE sql AS 'SELECT a'; \
             CREATE FUNCTION {s}.f2(a text) RETURNS int LANGUAGE sql AS 'SELECT 1'; \
             CREATE PROCEDURE {s}.p(x int) LANGUAGE plpgsql AS $$ BEGIN END $$; \
             CREATE AGGREGATE {s}.agg(int) (SFUNC = int4pl, STYPE = int)",
        )
        .await;
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let ns = ns_of(&t, &s);
        let prev = load(&mut c, &ns, Level::Sources).await;
        run(&t, &format!("INSERT INTO {s}.x VALUES (1)")).await;
        let (next, _, p) = refresh(&mut c, &ns, &prev).await;
        assert!(p.report.full_reason.is_none(), "{}", t.id);
        assert!(
            !ran(&p, name::RETRIEVE_ROUTINES),
            "{}: an unchanged routine was re-read: {:?}",
            t.id,
            p.report.log.names()
        );
        assert_same(
            &format!("{} insert", t.id),
            &next,
            &load(&mut c, &ns, Level::Sources).await,
        );
        // One changed overload is re-read, and only it.
        run(&t, &format!("ALTER FUNCTION {s}.f2(a int, b text) STABLE")).await;
        let (after, _, p) = refresh(&mut c, &ns, &next).await;
        let rows: usize = p
            .report
            .log
            .sections
            .iter()
            .filter(|x| x.name == name::RETRIEVE_ROUTINES)
            .map(|x| x.rows)
            .sum();
        assert_eq!(rows, 1, "{}: {:?}", t.id, p.report.log.sections);
        assert_same(
            &format!("{} alter f2", t.id),
            &after,
            &load(&mut c, &ns, Level::Sources).await,
        );
        drop(c);
        pg.shutdown().await;
    }
}
// ## m4-t28-routine-lookup
