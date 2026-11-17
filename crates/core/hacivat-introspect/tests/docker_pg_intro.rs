//! The PostgreSQL introspector against PostgreSQL 13–18 (`docker/m2a`): a full pass reads
//! every object kind correctly, and an incremental pass after each DDL of the ver/01
//! #pg-xmin matrix (extended) equals a fresh full pass. Every test is `#[ignore = "docker"]`;
//! run with `cargo xtask docker test --set full -p hacivat-introspect` (pg17 alone in `pr`).

#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

mod common;

use common::*;
use hacivat_introspect::pg::*;
use hacivat_testkit::require_target;

const SOURCES: IntrospectOpts = IntrospectOpts { sources: true };

fn fixture(schema: &str) -> String {
    include_str!("fixtures/pg_kinds.sql").replace("hvpgi_fx", schema)
}

async fn drop_schemas(t: &hacivat_testkit::DockerTarget, names: &[String]) {
    let mut c = connect(t).await;
    for n in names {
        run(&mut c, &format!("DROP SCHEMA IF EXISTS {n} CASCADE")).await;
    }
}

fn rel<'a>(ns: &'a PgNamespace, name: &str) -> &'a PgRelation {
    ns.relations
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no relation {name}"))
}

fn col<'a>(r: &'a PgRelation, name: &str) -> &'a PgColumn {
    r.columns
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no column {name} in {}", r.name))
}

fn num(r: &PgRelation, name: &str) -> i16 {
    col(r, name).num
}

/// Every object kind, PostgreSQL 13–18.
#[tokio::test]
#[ignore = "docker"]
async fn full_pass_reads_every_kind() {
    for id in PG_IDS {
        let Some(t) = require_target(id) else {
            continue;
        };
        let s = unique_name("hvpgi_k");
        let names = vec![s.clone()];
        guarded(
            async {
                let mut c = connect(&t).await;
                run(&mut c, &fixture(&s)).await;
                let oid = schema_oid(&mut c, &s).await;
                let pass = full(&mut c, oid, SOURCES).await.unwrap();
                assert!(!pass.stats.xid_assigned, "{id}: the pass assigned an XID");
                check_kinds(id, &pass.result);
                let listed = list_schemas(&mut c).await.unwrap();
                assert!(listed.iter().any(|x| x.oid == oid && !x.system), "{id}");
                assert!(
                    listed.iter().any(|x| x.name == "pg_catalog" && x.system),
                    "{id}"
                );
                assert!(
                    !listed.iter().any(|x| x.name.starts_with("pg_toast")),
                    "{id}"
                );
            },
            drop_schemas(&t, &names),
        )
        .await;
    }
}

fn check_kinds(id: &str, ns: &PgNamespace) {
    let major: u32 = id[2..].parse().unwrap();
    assert_eq!(ns.schema.comment.as_deref(), Some("fixture schema"), "{id}");
    let mut names: Vec<(&str, RelKind)> = ns
        .relations
        .iter()
        .map(|r| (r.name.as_str(), r.kind))
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            ("Mixed Case", RelKind::Table),
            ("child", RelKind::Table),
            ("events", RelKind::PartitionedTable),
            ("events_2026", RelKind::Table),
            ("mv_child", RelKind::MaterializedView),
            ("pair", RelKind::CompositeType),
            ("parent", RelKind::Table),
            ("u", RelKind::Table),
            ("v_parent", RelKind::View),
        ],
        "{id}"
    );

    // Columns: identity, arrays, enum, domain, composite, typmods, generated, collation.
    let p = rel(ns, "parent");
    assert_eq!(p.comment.as_deref(), Some("Parents"), "{id}");
    let id_col = col(p, "id");
    assert_eq!(id_col.identity, Some(Identity::Always), "{id}");
    assert_eq!(id_col.ty.display, "bigint", "{id}");
    assert!(id_col.not_null, "{id}");
    assert_eq!(col(p, "code").comment.as_deref(), Some("Code"), "{id}");
    let tags = col(p, "tags");
    assert_eq!(tags.ty.kind, TypeKind::Array { element: 25 }, "{id}");
    assert_eq!(tags.ty.display, "text[]", "{id}");
    let grid = col(p, "grid");
    assert_eq!(
        grid.ty.display, "integer[]",
        "{id}: dims are not part of the type"
    );
    assert_eq!(grid.array_dims, 2, "{id}");
    let m = col(p, "m");
    assert_eq!(m.ty.kind, TypeKind::Enum, "{id}");
    assert_eq!(m.ty.name, "mood", "{id}");
    assert!(
        m.default.as_deref().unwrap().starts_with("'ok'"),
        "{id}: {:?}",
        m.default
    );
    let mood = ns.types.iter().find(|t| t.name == "mood").unwrap();
    assert_eq!(
        col(p, "moods").ty.kind,
        TypeKind::Array { element: mood.oid },
        "{id}"
    );
    assert!(
        matches!(col(p, "p").ty.kind, TypeKind::Domain { base: 23 }),
        "{id}"
    );
    assert_eq!(col(p, "pr").ty.kind, TypeKind::Composite, "{id}");
    assert_eq!(col(p, "total").ty.display, "numeric(10,2)", "{id}");
    assert_eq!(col(p, "total").default.as_deref(), Some("0"), "{id}");
    let doubled = col(p, "doubled");
    assert_eq!(doubled.generated, Some(Generated::Stored), "{id}");
    assert!(
        doubled.default.as_deref().unwrap().contains("total"),
        "{id}"
    );
    assert_eq!(col(p, "name").collation.as_deref(), Some("C"), "{id}");
    assert_eq!(
        col(p, "ts").ty.display,
        "timestamp(3) with time zone",
        "{id}"
    );
    assert_eq!(col(p, "ts").default.as_deref(), Some("now()"), "{id}");

    // Constraints: PK, UK (key order), FK, CHECK, EXCLUDE; PG 18 catalogues NOT NULL.
    let ch = rel(ns, "child");
    let uq = ch
        .constraints
        .iter()
        .find(|k| k.name == "child_ab_uq")
        .unwrap();
    assert_eq!(uq.kind, ConstraintKind::Unique, "{id}");
    assert_eq!(uq.columns, [num(ch, "b"), num(ch, "a")], "{id}: key order");
    assert_eq!(uq.comment.as_deref(), Some("uq"), "{id}");
    let uq_index = uq.index.expect("a unique constraint has a backing index");
    assert!(
        ch.indexes.iter().any(|i| i.oid == uq_index && i.unique),
        "{id}"
    );
    let fk = ch
        .constraints
        .iter()
        .find(|k| k.kind == ConstraintKind::ForeignKey)
        .unwrap();
    let fref = fk.foreign.as_ref().unwrap();
    assert_eq!((fref.table, fref.name.as_str()), (p.oid, "parent"), "{id}");
    assert_eq!(fref.columns, [num(p, "id")], "{id}");
    assert_eq!(fref.on_delete, FkAction::Cascade, "{id}");
    assert!(fk.deferrable && fk.deferred, "{id}");
    assert!(
        ch.constraints
            .iter()
            .any(|k| k.kind == ConstraintKind::Exclusion && k.name == "child_r_excl"),
        "{id}"
    );
    let ck = p
        .constraints
        .iter()
        .find(|k| k.name == "parent_total_ck")
        .unwrap();
    assert_eq!(ck.definition, "CHECK (total >= 0::numeric)", "{id}");
    let not_nulls = p
        .constraints
        .iter()
        .filter(|k| k.kind == ConstraintKind::NotNull)
        .count();
    if major >= 18 {
        assert!(
            not_nulls >= 2,
            "{id}: PG 18 catalogues NOT NULL constraints"
        );
    } else {
        assert_eq!(not_nulls, 0, "{id}");
    }
    assert!(p.constraints.iter().all(|k| k.enforced), "{id}");

    // Indexes: expression + INCLUDE + partial; DESC NULLS LAST; comment.
    let ei = ch
        .indexes
        .iter()
        .find(|i| i.name == "child_expr_idx")
        .unwrap();
    assert_eq!(
        ei.keys,
        [IndexKey {
            elem: IndexElem::Expression("(a + b)".into()),
            descending: false,
            nulls_first: false
        }],
        "{id}"
    );
    assert_eq!(
        ei.include,
        [IndexElem::Column(num(ch, "parent_id"))],
        "{id}"
    );
    assert_eq!(ei.predicate.as_deref(), Some("(a > 0)"), "{id}");
    assert!(
        ei.definition
            .as_deref()
            .unwrap()
            .starts_with("CREATE INDEX child_expr_idx"),
        "{id}"
    );
    let di = ch
        .indexes
        .iter()
        .find(|i| i.name == "child_desc_idx")
        .unwrap();
    assert_eq!(
        (di.keys[0].descending, di.keys[0].nulls_first),
        (true, false),
        "{id}"
    );
    assert_eq!(di.comment.as_deref(), Some("desc"), "{id}");
    assert!(ch.indexes.iter().any(|i| i.primary), "{id}");
    let ex = ch
        .indexes
        .iter()
        .find(|i| i.name == "child_r_excl")
        .unwrap();
    assert!(ex.exclusion && ex.method == "gist", "{id}");

    // Triggers.
    let tr = &ch.triggers;
    assert_eq!(tr.len(), 1, "{id}: internal FK triggers are left out");
    assert_eq!(tr[0].name, "child_bi", "{id}");
    assert_eq!(tr[0].timing, TriggerTiming::Before, "{id}");
    assert_eq!(
        tr[0].events,
        [TriggerEvent::Insert, TriggerEvent::Update],
        "{id}"
    );
    assert!(tr[0].row_level && tr[0].enabled == 'O', "{id}");
    assert_eq!(tr[0].function_name, "trg", "{id}");
    assert_eq!(tr[0].comment.as_deref(), Some("trg"), "{id}");

    // Partitions, views, materialized views, persistence, quoting.
    let ev = rel(ns, "events");
    assert_eq!(ev.partition_key.as_deref(), Some("RANGE (at)"), "{id}");
    let ev26 = rel(ns, "events_2026");
    assert_eq!(ev26.parents, [ev.oid], "{id}");
    assert_eq!(
        ev26.partition_bound.as_deref(),
        Some("FOR VALUES FROM ('2026-01-01') TO ('2027-01-01')"),
        "{id}"
    );
    let v = rel(ns, "v_parent");
    assert!(
        v.view_definition.as_deref().unwrap().contains("total > "),
        "{id}"
    );
    assert_eq!(v.comment.as_deref(), Some("positive parents"), "{id}");
    assert_eq!(v.columns.len(), 2, "{id}");
    assert!(!rel(ns, "mv_child").populated, "{id}");
    assert_eq!(rel(ns, "u").persistence, Persistence::Unlogged, "{id}");
    let mixed = rel(ns, "Mixed Case");
    assert_eq!(mixed.columns[0].name, "Ünïcode Col", "{id}");
    assert_eq!(mixed.columns[1].name, "quote\"d", "{id}");

    // Sequences: explicit, serial-owned, identity.
    let sx = ns.sequences.iter().find(|s| s.name == "seq_x").unwrap();
    assert_eq!(
        (
            sx.data_type.as_str(),
            sx.start,
            sx.increment,
            sx.min,
            sx.max,
            sx.cache,
            sx.cycle
        ),
        ("integer", 10, 5, 10, 1000, 2, true),
        "{id}"
    );
    assert_eq!(sx.comment.as_deref(), Some("seq"), "{id}");
    let serial = ns
        .sequences
        .iter()
        .find(|s| s.name == "child_id_seq")
        .unwrap();
    assert_eq!(serial.owned_by, Some((ch.oid, num(ch, "id"))), "{id}");
    assert!(!serial.identity, "{id}");
    let ident = ns
        .sequences
        .iter()
        .find(|s| s.name == "parent_id_seq")
        .unwrap();
    assert!(ident.identity, "{id}");

    // Routines: overloads, set-returning, procedure, trigger function, aggregate.
    let adds: Vec<&str> = ns
        .routines
        .iter()
        .filter(|r| r.name == "add")
        .map(|r| r.identity_args.as_str())
        .collect();
    assert_eq!(adds, ["a integer, b integer", "a text, b text"], "{id}");
    let add_int = ns
        .routines
        .iter()
        .find(|r| r.name == "add" && r.identity_args.contains("integer"))
        .unwrap();
    assert_eq!(add_int.volatility, Volatility::Immutable, "{id}");
    assert!(add_int.strict, "{id}");
    assert_eq!(add_int.comment.as_deref(), Some("adds"), "{id}");
    assert!(
        add_int
            .definition
            .as_deref()
            .unwrap()
            .contains("SELECT a + b"),
        "{id}"
    );
    let series = ns.routines.iter().find(|r| r.name == "series").unwrap();
    assert!(series.returns_set, "{id}");
    assert_eq!(series.result.as_deref(), Some("SETOF integer"), "{id}");
    let noop = ns.routines.iter().find(|r| r.name == "noop").unwrap();
    assert_eq!(
        (noop.kind, noop.result.as_deref()),
        (RoutineKind::Procedure, None),
        "{id}"
    );
    let agg = ns.routines.iter().find(|r| r.name == "mysum").unwrap();
    assert_eq!(agg.kind, RoutineKind::Aggregate, "{id}");
    assert!(agg.definition.is_none(), "{id}");

    // Types.
    assert_eq!(
        mood.kind,
        UserTypeKind::Enum {
            labels: vec!["sad".into(), "ok".into(), "happy".into()]
        },
        "{id}"
    );
    assert_eq!(mood.comment.as_deref(), Some("feelings"), "{id}");
    let posint = ns.types.iter().find(|t| t.name == "posint").unwrap();
    let UserTypeKind::Domain {
        base,
        base_display,
        not_null,
        default,
        constraints,
    } = &posint.kind
    else {
        panic!("{id}: posint is a domain");
    };
    assert_eq!(
        (*base, base_display.as_str(), *not_null),
        (23, "integer", true),
        "{id}"
    );
    assert_eq!(default.as_deref(), Some("1"), "{id}");
    let checks: Vec<&str> = constraints
        .iter()
        .filter(|k| k.kind == 'c')
        .map(|k| k.name.as_str())
        .collect();
    assert_eq!(checks, ["posint_ck"], "{id}");
    let domain_nn = constraints.iter().filter(|k| k.kind == 'n').count();
    assert_eq!(
        domain_nn,
        usize::from(major >= 17),
        "{id}: PG 17 catalogues domain NOT NULL"
    );
    let pair = ns.types.iter().find(|t| t.name == "pair").unwrap();
    assert_eq!(
        pair.kind,
        UserTypeKind::Composite {
            relid: rel(ns, "pair").oid
        },
        "{id}"
    );
    let fr = ns.types.iter().find(|t| t.name == "floatrange").unwrap();
    assert_eq!(
        fr.kind,
        UserTypeKind::Range {
            subtype: "double precision".into()
        },
        "{id}"
    );
    assert_eq!(ns.types.len(), 4, "{id}: no array, row or multirange types");
}

/// One DDL step of the matrix and what its incremental pass may and must read.
struct Step {
    label: &'static str,
    sql: &'static str,
    /// Sections the pass must run.
    ran: &'static [&'static str],
    /// Sections the pass must not run.
    not_ran: &'static [&'static str],
}

const fn step(label: &'static str, sql: &'static str) -> Step {
    Step {
        label,
        sql,
        ran: &[],
        not_ran: &[],
    }
}

/// ver/01 #pg-xmin's DDL loop, extended with renames, comment removal, types, partitions,
/// sequences and moves. `{s}` is the schema.
const STEPS: &[Step] = &[
    Step {
        ran: &["pg/RetrieveIndexes"],
        not_ran: &["pg/RetrieveColumns", "pg/RetrieveRelations"],
        ..step(
            "create index (first on u)",
            "CREATE INDEX u_a_idx ON {s}.u (a)",
        )
    },
    step(
        "create index (second on child)",
        "CREATE INDEX child_a_idx ON {s}.child (a)",
    ),
    Step {
        ran: &["pg/RetrieveColumns"],
        ..step("add column", "ALTER TABLE {s}.parent ADD COLUMN extra int")
    },
    Step {
        ran: &["pg/Comments"],
        not_ran: &["pg/RetrieveColumns", "pg/RetrieveRelations"],
        ..step("comment on table", "COMMENT ON TABLE {s}.u IS 'u table'")
    },
    Step {
        not_ran: &["pg/RetrieveColumns"],
        ..step("comment on column", "COMMENT ON COLUMN {s}.u.a IS 'col a'")
    },
    Step {
        ran: &["pg/Comments"],
        not_ran: &["pg/RetrieveColumns"],
        ..step(
            "remove a table comment",
            "COMMENT ON TABLE {s}.parent IS NULL",
        )
    },
    step(
        "remove a column comment",
        "COMMENT ON COLUMN {s}.parent.code IS NULL",
    ),
    step(
        "remove an index comment",
        "COMMENT ON INDEX {s}.child_desc_idx IS NULL",
    ),
    step("grant on table", "GRANT SELECT ON {s}.u TO PUBLIC"),
    step("grant on column", "GRANT SELECT (a) ON {s}.u TO PUBLIC"),
    Step {
        ran: &["pg/RetrieveTriggers"],
        ..step(
            "first trigger on u",
            "CREATE TRIGGER u_t1 BEFORE INSERT ON {s}.u FOR EACH ROW EXECUTE FUNCTION {s}.trg()",
        )
    },
    step(
        "second trigger on u",
        "CREATE TRIGGER u_t2 AFTER DELETE ON {s}.u FOR EACH STATEMENT EXECUTE FUNCTION {s}.trg()",
    ),
    Step {
        ran: &["pg/RetrieveRoutines"],
        not_ran: &["pg/RetrieveColumns"],
        ..step("alter function", "ALTER FUNCTION {s}.add(int, int) STABLE")
    },
    step(
        "create or replace function",
        "CREATE OR REPLACE FUNCTION {s}.add(a text, b text) RETURNS text LANGUAGE sql AS 'SELECT b || a'",
    ),
    step(
        "set default",
        "ALTER TABLE {s}.child ALTER COLUMN a SET DEFAULT 5",
    ),
    step(
        "drop default",
        "ALTER TABLE {s}.child ALTER COLUMN a DROP DEFAULT",
    ),
    step(
        "add check",
        "ALTER TABLE {s}.u ADD CONSTRAINT u_a_ck CHECK (a > 0)",
    ),
    step("drop index", "DROP INDEX {s}.child_a_idx"),
    step("drop trigger", "DROP TRIGGER u_t2 ON {s}.u"),
    step("drop column", "ALTER TABLE {s}.parent DROP COLUMN extra"),
    step(
        "rename a column named by a check, a generated column and a view",
        "ALTER TABLE {s}.parent RENAME COLUMN total TO amount",
    ),
    step(
        "rename a table referenced by a foreign key and a view",
        "ALTER TABLE {s}.parent RENAME TO parent2",
    ),
    step(
        "rename a type used by columns",
        "ALTER TYPE {s}.mood RENAME TO feeling",
    ),
    step(
        "enum add value",
        "ALTER TYPE {s}.feeling ADD VALUE 'meh' AFTER 'ok'",
    ),
    step(
        "enum rename value",
        "ALTER TYPE {s}.feeling RENAME VALUE 'sad' TO 'blue'",
    ),
    step(
        "domain add constraint",
        "ALTER DOMAIN {s}.posint ADD CONSTRAINT posint_max CHECK (VALUE < 1000)",
    ),
    step(
        "domain drop constraint",
        "ALTER DOMAIN {s}.posint DROP CONSTRAINT posint_max",
    ),
    step(
        "domain set default",
        "ALTER DOMAIN {s}.posint SET DEFAULT 2",
    ),
    Step {
        ran: &["pg/RetrieveSequences"],
        not_ran: &["pg/RetrieveColumns"],
        ..step("alter sequence", "ALTER SEQUENCE {s}.seq_x INCREMENT 7")
    },
    step(
        "create or replace view",
        "CREATE OR REPLACE VIEW {s}.v_parent AS SELECT id, code, name FROM {s}.parent2 WHERE amount > 0",
    ),
    step(
        "rename a trigger function",
        "ALTER FUNCTION {s}.trg() RENAME TO trg2",
    ),
    step(
        "disable a trigger",
        "ALTER TABLE {s}.child DISABLE TRIGGER child_bi",
    ),
    step(
        "attach a partition",
        "CREATE TABLE {s}.events_2027 (id int, at date NOT NULL); \
         ALTER TABLE {s}.events ATTACH PARTITION {s}.events_2027 FOR VALUES FROM ('2027-01-01') TO ('2028-01-01')",
    ),
    step(
        "detach a partition",
        "ALTER TABLE {s}.events DETACH PARTITION {s}.events_2026",
    ),
    step(
        "set not null",
        "ALTER TABLE {s}.child ALTER COLUMN b SET NOT NULL",
    ),
    step(
        "drop not null",
        "ALTER TABLE {s}.child ALTER COLUMN b DROP NOT NULL",
    ),
    step(
        "rename an index",
        "ALTER INDEX {s}.child_desc_idx RENAME TO child_desc_idx2",
    ),
    step(
        "drop a constraint",
        "ALTER TABLE {s}.u DROP CONSTRAINT u_a_ck",
    ),
    step(
        "create a table",
        "CREATE TABLE {s}.newt (x int PRIMARY KEY, y text DEFAULT 'q'); COMMENT ON TABLE {s}.newt IS 'n'",
    ),
    step("drop a table", "DROP TABLE {s}.newt"),
    step(
        "move a table to another schema",
        "CREATE SCHEMA {s}_other; ALTER TABLE {s}.\"Mixed Case\" SET SCHEMA {s}_other",
    ),
    step("drop an aggregate", "DROP AGGREGATE {s}.mysum(int)"),
    step(
        "comment on the schema",
        "COMMENT ON SCHEMA {s} IS 'changed'",
    ),
    step("change an owner", "ALTER TABLE {s}.u OWNER TO hv_ro"),
    step(
        "composite type add attribute",
        "ALTER TYPE {s}.pair ADD ATTRIBUTE c int",
    ),
    step(
        "refresh a materialized view",
        "REFRESH MATERIALIZED VIEW {s}.mv_child",
    ),
    step("truncate", "TRUNCATE {s}.u"),
    step(
        "alter a column type",
        "ALTER TABLE {s}.child ALTER COLUMN b TYPE bigint",
    ),
    step(
        "rename a composite type used by a column",
        "ALTER TYPE {s}.pair RENAME TO pair2",
    ),
];

/// After every DDL of the matrix, `apply(prev, incremental) == full`, PostgreSQL 13–18; the
/// incremental pass never assigns an XID and reads only the facets the step changed.
#[tokio::test]
#[ignore = "docker"]
async fn incremental_equals_full_after_each_ddl() {
    for id in PG_IDS {
        let Some(t) = require_target(id) else {
            continue;
        };
        let s = unique_name("hvpgi_x");
        let names = vec![s.clone(), format!("{s}_other")];
        guarded(
            async {
                let mut c = connect(&t).await;
                run(&mut c, &fixture(&s)).await;
                let oid = schema_oid(&mut c, &s).await;
                let first = full(&mut c, oid, SOURCES).await.unwrap();
                let mut prev = first.result;
                let mut wm = first.watermark;
                for st in STEPS {
                    run(&mut c, &st.sql.replace("{s}", &s)).await;
                    let pass = refresh(&mut c, &prev, Some(&wm), SOURCES).await.unwrap();
                    let label = format!("{id} / {}", st.label);
                    assert!(
                        !pass.stats.xid_assigned,
                        "{label}: the pass assigned an XID"
                    );
                    let Refresh::Delta(delta) = &pass.result else {
                        panic!("{label}: expected a delta, got {:?}", pass.result);
                    };
                    for name in st.ran {
                        assert!(pass.stats.ran(name), "{label}: {name} did not run");
                    }
                    for name in st.not_ran {
                        assert!(!pass.stats.ran(name), "{label}: {name} ran");
                    }
                    let merged = apply(&prev, delta);
                    let now = full(&mut c, oid, SOURCES).await.unwrap();
                    assert_namespaces_eq(&label, &merged, &now.result);
                    prev = merged;
                    wm = pass.watermark;
                }
                // A data change writes no catalog row: the next pass reads nothing.
                run(&mut c, &format!("INSERT INTO {s}.u VALUES (1)")).await;
                let pass = refresh(&mut c, &prev, Some(&wm), SOURCES).await.unwrap();
                let Refresh::Delta(delta) = &pass.result else {
                    panic!("{id}: expected a delta");
                };
                assert!(delta.reads_nothing(), "{id}: {delta:?}");
                // The schema is dropped: the pass says so.
                run(&mut c, &format!("DROP SCHEMA {s} CASCADE")).await;
                let pass = refresh(&mut c, &prev, Some(&wm), SOURCES).await.unwrap();
                assert_eq!(pass.result, Refresh::Gone, "{id}");
            },
            drop_schemas(&t, &names),
        )
        .await;
    }
}

/// Compares two namespaces object by object, so a failure names the object that differs.
fn assert_namespaces_eq(label: &str, got: &PgNamespace, want: &PgNamespace) {
    assert_eq!(got.schema, want.schema, "{label}: schema");
    let names =
        |ns: &PgNamespace| -> Vec<String> { ns.relations.iter().map(|r| r.name.clone()).collect() };
    assert_eq!(names(got), names(want), "{label}: relations");
    for (g, w) in got.relations.iter().zip(&want.relations) {
        assert_eq!(g, w, "{label}: relation {}", w.name);
    }
    assert_eq!(got.sequences, want.sequences, "{label}: sequences");
    assert_eq!(got.routines, want.routines, "{label}: routines");
    assert_eq!(got.types, want.types, "{label}: types");
}

/// HV-INTRO-05 test 8: DDL in a transaction still open at the pass's snapshot is not seen
/// then, and is seen by the next pass after it commits (its XID is at or after the watermark).
#[tokio::test]
#[ignore = "docker"]
async fn ddl_open_during_a_pass_is_seen_by_the_next_pass() {
    for id in PG_IDS {
        let Some(t) = require_target(id) else {
            continue;
        };
        let s = unique_name("hvpgi_l");
        let names = vec![s.clone()];
        guarded(
            async {
                let mut c = connect(&t).await;
                let mut other = connect(&t).await;
                run(
                    &mut c,
                    &format!("CREATE SCHEMA {s}; CREATE TABLE {s}.a (x int)"),
                )
                .await;
                let oid = schema_oid(&mut c, &s).await;
                let first = full(&mut c, oid, SOURCES).await.unwrap();
                run(&mut other, &format!("BEGIN; CREATE TABLE {s}.late (a int)")).await;
                let p1 = refresh(&mut c, &first.result, Some(&first.watermark), SOURCES)
                    .await
                    .unwrap();
                let Refresh::Delta(d1) = &p1.result else {
                    panic!("{id}: expected a delta");
                };
                let mid = apply(&first.result, d1);
                assert!(!mid.relations.iter().any(|r| r.name == "late"), "{id}");
                run(&mut other, "COMMIT").await;
                let p2 = refresh(&mut c, &mid, Some(&p1.watermark), SOURCES)
                    .await
                    .unwrap();
                let Refresh::Delta(d2) = &p2.result else {
                    panic!("{id}: expected a delta");
                };
                let end = apply(&mid, d2);
                assert!(end.relations.iter().any(|r| r.name == "late"), "{id}");
                let now = full(&mut c, oid, SOURCES).await.unwrap();
                assert_namespaces_eq(id, &end, &now.result);
            },
            drop_schemas(&t, &names),
        )
        .await;
    }
}

/// A watermark from another database, a restart or a horizon that went backwards forces a full
/// pass (the plan is unit-tested; this checks the probe feeds it real values).
#[tokio::test]
#[ignore = "docker"]
async fn foreign_watermarks_force_a_full_pass() {
    let Some(t) = require_target("pg17") else {
        return;
    };
    let s = unique_name("hvpgi_w");
    let names = vec![s.clone()];
    guarded(
        async {
            let mut c = connect(&t).await;
            run(
                &mut c,
                &format!("CREATE SCHEMA {s}; CREATE TABLE {s}.a (x int)"),
            )
            .await;
            let oid = schema_oid(&mut c, &s).await;
            let first = full(&mut c, oid, IntrospectOpts::default()).await.unwrap();
            let wm = &first.watermark;
            assert!(wm.identity.system_identifier.is_some());
            assert_eq!(wm.version.major(), 17);
            let mut restarted = wm.clone();
            restarted.identity.postmaster_start.push('1');
            let mut other_db = wm.clone();
            other_db.identity.database += 1;
            let mut ahead = wm.clone();
            ahead.w += 1 << 20; // a horizon that went backwards (restore, pg_resetwal)
            for (w, reason) in [
                (None, FullReason::NoWatermark),
                (Some(&restarted), FullReason::Restart),
                (Some(&other_db), FullReason::Identity),
                (Some(&ahead), FullReason::Identity),
            ] {
                let pass = refresh(&mut c, &first.result, w, IntrospectOpts::default())
                    .await
                    .unwrap();
                match pass.result {
                    Refresh::Full { reason: r, ns } => {
                        assert_eq!(r, reason);
                        assert_eq!(ns, first.result);
                    }
                    other => panic!("{reason:?}: {other:?}"),
                }
            }
        },
        drop_schemas(&t, &names),
    )
    .await;
}
