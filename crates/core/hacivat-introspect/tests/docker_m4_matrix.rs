// ## m4-t28-matrix
//! Gate G4 (M4 Task 28; spec §6 M4 exit "incremental refresh is correct on the ver/01 matrix":
//! #pg-xmin, #pg-xmin-freeze with `pg_resetwal`, #mysql-is-times; #sqlite-schema-version is in
//! `sqlite_intro.rs`, block `m4-t28-oracle`).
//!
//! **Oracle.** After every step the incremental snapshot (the cached namespace merged with an
//! incremental pass, Task 10's `merge`) must equal a fresh `Full` pass of the same namespace
//! at the same moment, compared by `common::g4::canon`: the namespace's key, engine, fold rule,
//! level and object count, and every object (root included) with its key and server id,
//! `StateMark`, flags and properties. Only `loaded_at`, the generation and the namespace
//! watermark are left out. Each step also asserts its narrow-read fact (the incremental pass
//! is not `Full`, and its query log shows the expected reads), so a test cannot pass by always
//! running `Full`.
//!
//! **Record** (CI uploads no artifacts, R-W3-14, so the log is the record). Every line is
//! `HV-FACT g4 <target> <topic> <k=v …>`:
//! - a cell that holds: `HV-FACT g4 <target> <case> incremental==full`;
//! - a per-step row: `HV-FACT g4 <target> row step=… changed_parents=… queries=…` (also in
//!   `target/docker/t28-g4-<test>.tsv`, whose `incremental_eq_full` column is always `true`: a
//!   row is written only after its oracle held);
//! - the wall time of each test on each target, with that target's running total in this
//!   process: `HV-FACT g4 <target> wall test=… ms=… target_total_ms=…`. The tests run one at a
//!   time, so the last `wall` line of a target is its total: the plan's PR subset (step 4,
//!   ≤ 5 min) is the `pg17` total plus the `mysql84` total; the full set (≤ 15 min) is the sum
//!   of every target's last total.
//!
//! Every Docker test is `#[ignore = "docker"]`, returns early unless `HACIVAT_DOCKER=1`,
//! takes its targets only from `require_target`, and drops what it creates through guards
//! (also on panic). Run: `cargo test -p hacivat-introspect --no-run`, then
//! `<main>/target/docker/run-docker-tests.sh --set full -p hacivat-introspect --test
//! docker_m4_matrix` (the PR set runs pg17 and mysql84; `pg17wrap`, `pg17-tr` and the other
//! servers are full-set only).

#![cfg(feature = "session")]

mod common;

use std::time::Instant;

use common::g4::{Tsv, assert_equal, canon, cell, note};
use common::my::{self, Login};
use common::pg::{DropSchemas, PG, Pg, admin, admin_on, unique};
use hacivat_introspect::Introspector;
use hacivat_introspect::mysql::{MySqlIntrospector, PassStats};
use hacivat_introspect::pack::Mode;
use hacivat_introspect::pg::pass::{self, PassOpts, PgPass};
use hacivat_introspect::pg::sections::{Binds, ctx, name, pack};
use hacivat_introspect::pg::watermark::{FullReason, SPAN_LIMIT};
use hacivat_introspect::pg::{PgIntrospector, state};
use hacivat_meta::merge::{
    ExistSet, MergeOutcome, MergePolicy, NamespaceResult, Trigger, base, merge,
};
use hacivat_meta::name::{DialectId, FoldRule, IdentCtx};
use hacivat_meta::{
    KindGroup, Level, Lookup, NamespaceSnapshot, NsKey, ObjectKey, ObjectKind, ServerId, Watermark,
};
use hacivat_proto::DbmsKind;
use hacivat_session::meta::MetaConn;
use hacivat_testkit::{DockerTarget, docker_enabled, require_target};
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------------------------
// The oracle itself (no Docker): it sees every compared field and ignores only the three.

mod oracle {
    use std::sync::Arc;

    use hacivat_meta::namespace::{NsMeta, PgIdentity};
    use hacivat_meta::props::{Props, TableProps};
    use hacivat_meta::{
        Level, Name, NamespaceSnapshot, NsKey, Obj, ObjectKind, ServerId, StateMark, Subtree,
        Watermark,
    };
    use hacivat_proto::DbmsKind;

    use super::common::g4::{canon, diff};
    use hacivat_meta::name::FoldRule;

    /// One table `t` under schema `s`, with what a test varies.
    pub struct Shape {
        pub oid: u32,
        pub state: StateMark,
        pub comment: Option<&'static str>,
        pub level: Level,
        pub watermark: Watermark,
        pub loaded_at_ms: i64,
        pub generation: u64,
    }

    impl Default for Shape {
        fn default() -> Self {
            Shape {
                oid: 16_400,
                state: StateMark::Xmin(700),
                comment: Some("c"),
                level: Level::Structure,
                watermark: Watermark::None,
                loaded_at_ms: 1,
                generation: 1,
            }
        }
    }

    pub fn build(s: &Shape) -> NamespaceSnapshot {
        let mut tree = Subtree::new(Obj::new(ObjectKind::Schema, "s"));
        let table = Obj::new(ObjectKind::Table, "t")
            .with_server_id(ServerId::Oid(s.oid))
            .with_state(s.state)
            .with_props(Props::Table(Arc::new(TableProps {
                comment: s.comment.map(Name::from),
                ..TableProps::default()
            })));
        let root = tree.root();
        tree.graft_under(root, Subtree::new(table));
        NamespaceSnapshot::build(
            NsKey::schema(["db", "s"]),
            DbmsKind::Postgres,
            FoldRule::Exact,
            NsMeta {
                level: s.level,
                watermark: s.watermark.clone(),
                loaded_at_ms: s.loaded_at_ms,
            },
            tree,
        )
        .expect("a valid namespace")
        .with_generation(s.generation)
    }

    #[test]
    fn the_oracle_ignores_only_loaded_at_generation_and_watermark() {
        let a = build(&Shape::default());
        let same = build(&Shape {
            watermark: Watermark::PgXid {
                w: 9,
                identity: PgIdentity {
                    system_identifier: 1,
                    start_time_micros: 2,
                },
            },
            loaded_at_ms: 99,
            generation: 7,
            ..Shape::default()
        });
        assert_eq!(canon(&a), canon(&same));
        // Each compared field, changed alone, is a difference on both sides.
        for (what, other) in [
            (
                "server id",
                build(&Shape {
                    oid: 16_401,
                    ..Shape::default()
                }),
            ),
            (
                "state mark",
                build(&Shape {
                    state: StateMark::Xmin(701),
                    ..Shape::default()
                }),
            ),
            (
                "property",
                build(&Shape {
                    comment: None,
                    ..Shape::default()
                }),
            ),
            (
                "level",
                build(&Shape {
                    level: Level::Sources,
                    ..Shape::default()
                }),
            ),
        ] {
            let (only_a, only_other) = diff(&a, &other);
            assert!(
                !only_a.is_empty() && !only_other.is_empty(),
                "{what}: {only_a:?} / {only_other:?}"
            );
        }
    }

    #[test]
    #[should_panic(expected = "incremental differs from full")]
    fn assert_equal_panics_on_a_difference() {
        let a = build(&Shape::default());
        let b = build(&Shape {
            comment: Some("other"),
            ..Shape::default()
        });
        super::common::g4::assert_equal("control", &a, &b);
    }

    // ## m4-t28 fix round 1: the review's oracle-negative probes (HOSTILE1 and HOSTILE2) as
    // pure tests. A PostgreSQL-shaped step (`SET DEFAULT 5; COMMENT 'new'`, the table's xmin
    // 700 → 800) is merged incrementally onto the cached namespace (the merge's general walk)
    // and compared with a fresh full read merged into an empty base. The clean merge is equal;
    // a merge that loses what the re-read brought (an injected miss) must fail the oracle.

    use std::collections::BTreeMap;

    use hacivat_meta::merge::{
        ChangedObject, ExistSet, Identity, MergePolicy, NamespaceResult, Trigger, base, merge,
    };
    use hacivat_meta::props::ColumnProps;

    type Col = (&'static str, u32, Option<&'static str>);

    fn table(xmin: u32, comment: &str, cols: &[Col]) -> Subtree {
        let mut t = Subtree::new(
            Obj::new(ObjectKind::Table, "t")
                .with_server_id(ServerId::Oid(16_400))
                .with_state(StateMark::Xmin(xmin))
                .with_props(Props::Table(Arc::new(TableProps {
                    comment: Some(Name::from(comment)),
                    ..TableProps::default()
                }))),
        );
        let r = t.root();
        for (name, ord, default) in cols {
            t.add(
                r,
                Obj::new(ObjectKind::Column, *name).with_props(Props::Column(Arc::new(
                    ColumnProps {
                        ordinal: *ord,
                        raw_type: Name::from("int4"),
                        nullable: true,
                        default_raw: default.map(Name::from),
                        ..ColumnProps::default()
                    },
                ))),
            );
        }
        t
    }

    fn ns_key() -> NsKey {
        NsKey::schema(["db", "s"])
    }

    /// A pass that read `t` (full, or incremental with `t`'s columns scoped).
    fn read(full: bool, xmin: u32, comment: &str, cols: &[Col]) -> NamespaceResult {
        let b = base(ns_key(), DbmsKind::Postgres, FoldRule::Exact).unwrap();
        let parent = b.object_key(0).unwrap();
        let mut existent = BTreeMap::new();
        existent.insert(
            ObjectKind::Table,
            ExistSet::Ok(vec![Identity {
                state: StateMark::Xmin(xmin),
                ..Identity::of(vec![Name::from("t")], Some(ServerId::Oid(16_400)))
            }]),
        );
        existent.insert(
            ObjectKind::Column,
            ExistSet::Scoped {
                scope: vec![vec![Name::from("t")]],
                ids: cols
                    .iter()
                    .map(|(n, _, _)| Identity::of(vec![Name::from("t"), Name::from(*n)], None))
                    .collect(),
            },
        );
        NamespaceResult {
            existent,
            changed: vec![ChangedObject {
                parent,
                subtree: table(xmin, comment, cols),
            }],
            categories_to_reload: Default::default(),
            new_watermark: Watermark::None,
            full,
            level: Level::Structure,
            read_at_ms: 1,
        }
    }

    fn apply(prev: &NamespaceSnapshot, r: NamespaceResult) -> NamespaceSnapshot {
        let policy = MergePolicy {
            trigger: Trigger::User,
            ..MergePolicy::default()
        };
        merge(Some(prev), r, &policy).unwrap().next().clone()
    }

    const BEFORE: [Col; 2] = [("id", 1, None), ("a", 2, None)];
    const AFTER: [Col; 2] = [("id", 1, None), ("a", 2, Some("5"))];

    /// The cached namespace, and the fresh full read after the step.
    fn cached_and_fresh() -> (NamespaceSnapshot, NamespaceSnapshot) {
        let b = base(ns_key(), DbmsKind::Postgres, FoldRule::Exact).unwrap();
        (
            apply(&b, read(true, 700, "old", &BEFORE)),
            apply(&b, read(true, 800, "new", &AFTER)),
        )
    }

    fn oracle_fails(inc: &NamespaceSnapshot, fresh: &NamespaceSnapshot) -> bool {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            super::common::g4::assert_equal("probe", inc, fresh);
        }))
        .is_err()
    }

    #[test]
    fn the_clean_incremental_merge_passes_the_oracle() {
        let (prev, fresh) = cached_and_fresh();
        let inc = apply(&prev, read(false, 800, "new", &AFTER));
        super::common::g4::assert_equal("clean", &inc, &fresh);
    }

    /// HOSTILE1: the merge keeps the cached properties of a re-read pair (the new default and
    /// comment are lost; the state mark moved).
    #[test]
    fn the_oracle_fails_on_an_incremental_merge_that_keeps_cached_props() {
        let (prev, fresh) = cached_and_fresh();
        let missed = apply(&prev, read(false, 800, "old", &BEFORE));
        assert!(oracle_fails(&missed, &fresh), "a lost re-read passed G4");
        let (only_inc, only_full) = diff(&missed, &fresh);
        assert!(
            only_inc.iter().any(|l| l.contains("\"old\""))
                && only_full.iter().any(|l| l.contains("\"new\""))
                && only_full.iter().any(|l| l.contains("\"5\"")),
            "{only_inc:#?} / {only_full:#?}"
        );
    }

    /// HOSTILE2: the merge keeps the cached `StateMark` of a re-read pair (the properties
    /// moved, the stamp did not).
    #[test]
    fn the_oracle_fails_on_an_incremental_merge_that_keeps_the_cached_state_mark() {
        let (prev, fresh) = cached_and_fresh();
        let missed = apply(&prev, read(false, 700, "new", &AFTER));
        assert!(
            oracle_fails(&missed, &fresh),
            "a stale state mark passed G4"
        );
        let (only_inc, only_full) = diff(&missed, &fresh);
        assert!(
            only_inc.iter().any(|l| l.contains("Xmin(700)"))
                && only_full.iter().any(|l| l.contains("Xmin(800)")),
            "{only_inc:#?} / {only_full:#?}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// One Docker test at a time

/// Held by every Docker test of this binary for its whole run. A concurrent test's open DDL
/// transaction would hold a pass's snapshot xmin (the PostgreSQL watermark) below an earlier
/// step's commit, so the next pass would re-read that step again: correct, but not the narrow
/// read the step asserts. The MySQL time-zone test also changes a server-wide setting.
async fn serial() -> tokio::sync::MutexGuard<'static, ()> {
    static SERIAL: std::sync::LazyLock<tokio::sync::Mutex<()>> =
        std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));
    SERIAL.lock().await
}

// ---------------------------------------------------------------------------------------------
// PostgreSQL helpers

fn pg_targets() -> Vec<DockerTarget> {
    PG.into_iter().filter_map(require_target).collect()
}

/// `pg17-tr` (full set) and `pg17` (PR set): HV-INTRO-14 and HV-INTRO-11 on the matrix.
fn tr_targets() -> Vec<DockerTarget> {
    ["pg17-tr", "pg17"]
        .into_iter()
        .filter_map(require_target)
        .collect()
}

fn pg_ns(t: &DockerTarget, schema: &str) -> NsKey {
    NsKey::schema([t.database.as_str(), schema])
}

async fn pg_pass(
    c: &mut MetaConn,
    ns: &NsKey,
    mode: Mode,
    level: Level,
    prev: Option<&NamespaceSnapshot>,
) -> PgPass {
    pass::introspect(
        c,
        ns,
        PassOpts {
            mode,
            level,
            prev,
            ..PassOpts::default()
        },
        &CancellationToken::new(),
    )
    .await
    .unwrap_or_else(|e| panic!("{ns:?} {mode:?}: {e:?}"))
}

/// `r` merged into `prev` as a user refresh (the guard's pending result taken as is).
fn apply(prev: &NamespaceSnapshot, r: NamespaceResult) -> NamespaceSnapshot {
    let policy = MergePolicy {
        trigger: Trigger::User,
        ..MergePolicy::default()
    };
    merge(Some(prev), r, &policy)
        .expect("the merge runs")
        .next()
        .clone()
}

/// A fresh full pass at `level`, merged into an empty namespace.
async fn pg_full(c: &mut MetaConn, ns: &NsKey, level: Level) -> NamespaceSnapshot {
    let p = pg_pass(c, ns, Mode::Full, level, None).await;
    let b = base(ns.clone(), DbmsKind::Postgres, FoldRule::Exact).expect("a base");
    apply(&b, p.result)
}

/// A cached namespace whose watermark is `wm`: a full pass merged with `wm`.
async fn forged(c: &mut MetaConn, ns: &NsKey, level: Level, wm: Watermark) -> NamespaceSnapshot {
    let mut p = pg_pass(c, ns, Mode::Full, level, None).await;
    p.result.new_watermark = wm;
    let b = base(ns.clone(), DbmsKind::Postgres, FoldRule::Exact).expect("a base");
    apply(&b, p.result)
}

fn ran(p: &PgPass, section: &str) -> bool {
    p.report.log.ran(section)
}

/// The retrieve and comment sections that sent a statement (what an incremental pass re-read).
fn reads(p: &PgPass) -> Vec<&'static str> {
    p.report
        .log
        .names()
        .into_iter()
        .filter(|s| s.starts_with("Retrieve") || *s == name::COMMENTS)
        .collect()
}

/// Rows of the two change-detection sections (the changed parents and dependents).
fn changed_rows(p: &PgPass) -> usize {
    p.report
        .log
        .sections
        .iter()
        .filter(|s| s.name == name::CHANGED_PARENTS || s.name == name::CHANGED_DEPENDENTS)
        .map(|s| s.rows)
        .sum()
}

fn queries(p: &PgPass) -> usize {
    p.report.log.sections.iter().map(|s| s.queries).sum()
}

/// Whether `ns` holds an object of `kind` at the namespace-relative `path`.
fn has(ns: &NamespaceSnapshot, kind: ObjectKind, path: &[&str]) -> bool {
    let base = ns.key().path.len();
    (0..ns.len() as u32).any(|i| {
        ns.object_key(i).is_some_and(|k| {
            k.kind == kind
                && k.path.len() == base + path.len()
                && k.path[base..].iter().zip(path).all(|(a, b)| &**a == *b)
        })
    })
}

fn key_of(ns: &NamespaceSnapshot, kind: ObjectKind, path: &[&str]) -> ObjectKey {
    let base = ns.key().path.len();
    (0..ns.len() as u32)
        .find_map(|i| {
            let k = ns.object_key(i)?;
            (k.kind == kind
                && k.path.len() == base + path.len()
                && k.path[base..].iter().zip(path).all(|(a, b)| &**a == *b))
            .then_some(k)
        })
        .unwrap_or_else(|| panic!("{kind:?} {path:?} not found"))
}

/// One test's wall time on `t` (`common::g4::wall`: per target, with the running total).
fn wall(t: &DockerTarget, test: &str, since: Instant) {
    common::g4::wall(&t.id, test, since);
}

/// Runs `sql` on `t`'s `hv_test` database when it goes (also on panic), on its own thread.
struct SqlOnDrop {
    t: DockerTarget,
    sql: String,
}

impl Drop for SqlOnDrop {
    fn drop(&mut self) {
        let (t, sql) = (self.t.clone(), self.sql.clone());
        let rt = tokio::runtime::Handle::current();
        std::thread::scope(|s| {
            s.spawn(|| {
                rt.block_on(async {
                    let mut a = admin(&t).await;
                    a.batch(&sql).await;
                    a.close().await;
                });
            });
        });
    }
}

// ---------------------------------------------------------------------------------------------
// #pg-xmin: the ver/01 script and the PG spike's extensions, one incremental pass after each

/// What an incremental pass after a step must and must not read.
#[derive(Clone, Copy)]
enum Narrow {
    /// These sections send a statement.
    Ran(&'static [&'static str]),
    /// These do not.
    NotRan(&'static [&'static str]),
    /// No retrieve or comment section sends a statement: the existence and change queries
    /// decide alone (a drop, a data-only change).
    ExistenceOnly,
    /// Only the comment section re-reads (COMMENT ON a table or column, and a comment
    /// removal: `Comments` and existence).
    CommentsOnly,
}

/// One DDL step; `gone` is an object the step removes from the namespace.
struct Step {
    id: &'static str,
    sql: &'static str,
    narrow: &'static [Narrow],
    gone: Option<(ObjectKind, &'static [&'static str])>,
}

const fn step(id: &'static str, sql: &'static str) -> Step {
    Step {
        id,
        sql,
        narrow: &[],
        gone: None,
    }
}

const NO_REL_NO_COL: Narrow = Narrow::NotRan(&[name::RETRIEVE_RELATIONS, name::RETRIEVE_COLUMNS]);
const NO_COL: Narrow = Narrow::NotRan(&[name::RETRIEVE_COLUMNS]);

/// The ver/01 #pg-xmin table `x` and its routine and sequence (beside the kinds fixture).
const XS: &str = "CREATE TABLE {s}.x (id int PRIMARY KEY, a int, b int); \
    CREATE FUNCTION {s}.xf(i int) RETURNS int LANGUAGE sql VOLATILE AS 'SELECT i'; \
    CREATE SEQUENCE {s}.xseq; \
    CREATE SCHEMA {s}_other";

/// ver/01 #pg-xmin (own wording), then the PG spike's extensions on the kinds fixture
/// (`tests/fixtures/pg_kinds.sql`), then a data-only change. `{s}` is the schema.
const STEPS: &[Step] = &[
    // ver/01 #pg-xmin
    Step {
        narrow: &[Narrow::Ran(&[name::RETRIEVE_INDEXES]), NO_REL_NO_COL],
        ..step("create-index-xi1", "CREATE INDEX xi1 ON {s}.x (a)")
    },
    Step {
        narrow: &[Narrow::Ran(&[name::RETRIEVE_INDEXES]), NO_REL_NO_COL],
        ..step("create-index-xi2", "CREATE INDEX xi2 ON {s}.x (b)")
    },
    Step {
        narrow: &[Narrow::Ran(&[name::RETRIEVE_COLUMNS])],
        ..step("add-column-c", "ALTER TABLE {s}.x ADD COLUMN c text")
    },
    Step {
        narrow: &[Narrow::CommentsOnly],
        ..step("comment-on-table", "COMMENT ON TABLE {s}.x IS 'x table'")
    },
    Step {
        narrow: &[Narrow::CommentsOnly],
        ..step("comment-on-column", "COMMENT ON COLUMN {s}.x.a IS 'col a'")
    },
    step("grant-table", "GRANT SELECT ON {s}.x TO PUBLIC"),
    step("grant-column", "GRANT SELECT (a) ON {s}.x TO PUBLIC"),
    Step {
        narrow: &[Narrow::Ran(&[name::RETRIEVE_TRIGGERS]), NO_COL],
        ..step(
            "first-trigger",
            "CREATE TRIGGER tr1 BEFORE INSERT ON {s}.x FOR EACH ROW EXECUTE FUNCTION {s}.trg()",
        )
    },
    Step {
        narrow: &[Narrow::Ran(&[name::RETRIEVE_TRIGGERS]), NO_COL],
        ..step(
            "second-trigger",
            "CREATE TRIGGER tr2 AFTER UPDATE ON {s}.x FOR EACH ROW EXECUTE FUNCTION {s}.trg()",
        )
    },
    Step {
        narrow: &[Narrow::Ran(&[name::RETRIEVE_ROUTINES]), NO_COL],
        ..step("alter-function-stable", "ALTER FUNCTION {s}.xf(int) STABLE")
    },
    Step {
        narrow: &[Narrow::Ran(&[name::RETRIEVE_ROUTINES]), NO_COL],
        ..step(
            "create-or-replace-function",
            "CREATE OR REPLACE FUNCTION {s}.xf(i int) RETURNS int LANGUAGE sql STABLE AS 'SELECT i + 1'",
        )
    },
    Step {
        narrow: &[Narrow::Ran(&[name::RETRIEVE_COLUMNS])],
        ..step(
            "set-default",
            "ALTER TABLE {s}.x ALTER COLUMN a SET DEFAULT 5",
        )
    },
    Step {
        narrow: &[Narrow::Ran(&[name::RETRIEVE_CONSTRAINTS])],
        ..step(
            "add-check",
            "ALTER TABLE {s}.x ADD CONSTRAINT x_a_ck CHECK (a > 0)",
        )
    },
    Step {
        narrow: &[Narrow::ExistenceOnly],
        gone: Some((ObjectKind::Index, &["x", "xi2"])),
        ..step("drop-index-xi2", "DROP INDEX {s}.xi2")
    },
    Step {
        narrow: &[Narrow::ExistenceOnly],
        gone: Some((ObjectKind::Trigger, &["x", "tr2"])),
        ..step("drop-trigger-tr2", "DROP TRIGGER tr2 ON {s}.x")
    },
    Step {
        narrow: &[Narrow::Ran(&[name::RETRIEVE_COLUMNS])],
        gone: Some((ObjectKind::Column, &["x", "c"])),
        ..step("drop-column-c", "ALTER TABLE {s}.x DROP COLUMN c")
    },
    Step {
        narrow: &[Narrow::ExistenceOnly],
        ..step("insert", "INSERT INTO {s}.x (id, a, b) VALUES (1, 1, 1)")
    },
    Step {
        narrow: &[Narrow::ExistenceOnly],
        ..step("vacuum-analyze", "VACUUM ANALYZE {s}.x")
    },
    Step {
        narrow: &[Narrow::Ran(&[name::RETRIEVE_SEQUENCES]), NO_COL],
        ..step("alter-sequence", "ALTER SEQUENCE {s}.xseq INCREMENT 7")
    },
    // PG spike extensions
    Step {
        narrow: &[Narrow::CommentsOnly],
        ..step(
            "remove-table-comment",
            "COMMENT ON TABLE {s}.parent IS NULL",
        )
    },
    Step {
        narrow: &[Narrow::CommentsOnly],
        ..step(
            "remove-column-comment",
            "COMMENT ON COLUMN {s}.parent.code IS NULL",
        )
    },
    // An index comment is part of its relation's keys facet (`CommentOwner::RelKeys`): the
    // keys are re-read, never the relation row or its columns.
    Step {
        narrow: &[Narrow::Ran(&[name::RETRIEVE_INDEXES]), NO_REL_NO_COL],
        ..step(
            "remove-index-comment",
            "COMMENT ON INDEX {s}.child_desc_idx IS NULL",
        )
    },
    step(
        "enum-add-value",
        "ALTER TYPE {s}.mood ADD VALUE 'meh' AFTER 'ok'",
    ),
    step(
        "enum-rename-value",
        "ALTER TYPE {s}.mood RENAME VALUE 'sad' TO 'blue'",
    ),
    step(
        "domain-add-constraint",
        "ALTER DOMAIN {s}.posint ADD CONSTRAINT posint_max CHECK (VALUE < 1000)",
    ),
    step(
        "domain-drop-constraint",
        "ALTER DOMAIN {s}.posint DROP CONSTRAINT posint_max",
    ),
    step(
        "domain-set-default",
        "ALTER DOMAIN {s}.posint SET DEFAULT 2",
    ),
    step(
        "composite-add-attribute",
        "ALTER TYPE {s}.pair ADD ATTRIBUTE c int",
    ),
    step(
        "create-or-replace-view",
        "CREATE OR REPLACE VIEW {s}.v_parent AS SELECT id, code, name FROM {s}.parent WHERE total > 0",
    ),
    step(
        "refresh-materialized-view",
        "REFRESH MATERIALIZED VIEW {s}.mv_child",
    ),
    step(
        "rename-trigger-function",
        "ALTER FUNCTION {s}.trg() RENAME TO trg2",
    ),
    step(
        "disable-trigger",
        "ALTER TABLE {s}.child DISABLE TRIGGER child_bi",
    ),
    step(
        "attach-partition",
        "CREATE TABLE {s}.events_2027 (id int, at date NOT NULL); \
         ALTER TABLE {s}.events ATTACH PARTITION {s}.events_2027 FOR VALUES FROM ('2027-01-01') TO ('2028-01-01')",
    ),
    step(
        "detach-partition",
        "ALTER TABLE {s}.events DETACH PARTITION {s}.events_2026",
    ),
    step(
        "set-not-null",
        "ALTER TABLE {s}.child ALTER COLUMN b SET NOT NULL",
    ),
    step(
        "drop-not-null",
        "ALTER TABLE {s}.child ALTER COLUMN b DROP NOT NULL",
    ),
    step(
        "alter-column-type",
        "ALTER TABLE {s}.child ALTER COLUMN b TYPE bigint",
    ),
    step(
        "rename-index",
        "ALTER INDEX {s}.child_desc_idx RENAME TO child_desc_idx2",
    ),
    step(
        "create-table",
        "CREATE TABLE {s}.newt (x int PRIMARY KEY, y text DEFAULT 'q'); COMMENT ON TABLE {s}.newt IS 'n'",
    ),
    Step {
        narrow: &[Narrow::ExistenceOnly],
        gone: Some((ObjectKind::Table, &["newt"])),
        ..step("drop-table", "DROP TABLE {s}.newt")
    },
    Step {
        gone: Some((ObjectKind::Table, &["Mixed Case"])),
        ..step(
            "set-schema-out",
            "ALTER TABLE {s}.\"Mixed Case\" SET SCHEMA {s}_other",
        )
    },
    step("owner-change", "ALTER TABLE {s}.u OWNER TO hv_ro"),
    step("truncate", "TRUNCATE {s}.u"),
    Step {
        narrow: &[Narrow::ExistenceOnly],
        gone: Some((ObjectKind::Aggregate, &["mysum"])),
        ..step("drop-aggregate", "DROP AGGREGATE {s}.mysum(int)")
    },
    step("comment-on-schema", "COMMENT ON SCHEMA {s} IS 'changed'"),
    step(
        "rename-column-named-by-check-generated-and-view",
        "ALTER TABLE {s}.parent RENAME COLUMN total TO amount",
    ),
    step(
        "rename-table-referenced-by-fk-and-view",
        "ALTER TABLE {s}.parent RENAME TO parent2",
    ),
    step("rename-enum-type", "ALTER TYPE {s}.mood RENAME TO feeling"),
    step(
        "rename-composite-type",
        "ALTER TYPE {s}.pair RENAME TO pair2",
    ),
    // After every DDL: a data-only change reads nothing.
    Step {
        narrow: &[Narrow::ExistenceOnly],
        ..step("insert-after-all-ddl", "INSERT INTO {s}.u VALUES (1)")
    },
];

/// Asserts `step`'s narrow-read facts on pass `p`.
fn check_narrow(label: &str, st: &Step, p: &PgPass) {
    let log = p.report.log.names();
    for n in st.narrow {
        match *n {
            Narrow::Ran(names) => {
                for s in names {
                    assert!(ran(p, s), "{label}: {s} did not run; log {log:?}");
                }
            }
            Narrow::NotRan(names) => {
                for s in names {
                    assert!(!ran(p, s), "{label}: {s} ran; log {log:?}");
                }
            }
            Narrow::ExistenceOnly => {
                assert!(reads(p).is_empty(), "{label}: re-read {:?}", reads(p));
            }
            Narrow::CommentsOnly => {
                assert_eq!(reads(p), [name::COMMENTS], "{label}: log {log:?}");
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_xmin_matrix_incremental_equals_full() {
    if !docker_enabled() {
        return;
    }
    let _serial = serial().await;
    let mut tsv = Tsv::new("pg-xmin");
    for t in pg_targets() {
        let started = Instant::now();
        let s = unique("hvm4_t28_xs");
        let _g = DropSchemas {
            t: t.clone(),
            names: vec![s.clone(), format!("{s}_other")],
        };
        let mut a = admin(&t).await;
        a.batch(&include_str!("fixtures/pg_kinds.sql").replace("hvm4_fx", &s))
            .await;
        a.batch(&XS.replace("{s}", &s)).await;
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let ns = pg_ns(&t, &s);
        let mut prev = pg_full(&mut c, &ns, Level::Sources).await;
        for st in STEPS {
            let label = format!("{} {}", t.id, st.id);
            a.batch(&st.sql.replace("{s}", &s)).await;
            let p = pg_pass(&mut c, &ns, Mode::Incremental, Level::Sources, Some(&prev)).await;
            assert_eq!(p.report.full_reason, None, "{label}: incremental");
            assert!(ran(&p, name::CANARY), "{label}: the canary");
            check_narrow(&label, st, &p);
            let next = apply(&prev, p.result.clone());
            let fresh = pg_full(&mut c, &ns, Level::Sources).await;
            assert_equal(&label, &next, &fresh);
            if let Some((kind, path)) = st.gone {
                assert!(!has(&next, kind, path), "{label}: {kind:?} {path:?} stays");
            }
            tsv.row(
                &t.id,
                st.id,
                &changed_rows(&p).to_string(),
                &queries(&p).to_string(),
            );
            cell(&t.id, st.id);
            prev = next;
        }
        // DROP SCHEMA: the namespace is gone (the pass says so, and it is no longer listed).
        a.batch(&format!("DROP SCHEMA {s} CASCADE")).await;
        let gone = pass::introspect(
            &mut c,
            &ns,
            PassOpts {
                mode: Mode::Incremental,
                level: Level::Sources,
                prev: Some(&prev),
                ..PassOpts::default()
            },
            &CancellationToken::new(),
        )
        .await;
        assert!(
            matches!(&gone, Err(hacivat_introspect::IntroError::Protocol(m)) if &**m == "pg/namespace-gone"),
            "{}: drop schema: {:?}",
            t.id,
            gone.map(|p| p.report.full_reason)
        );
        let listed = PgIntrospector
            .list_namespaces(&mut c)
            .await
            .unwrap_or_else(|e| panic!("{}: list: {e:?}", t.id));
        assert!(listed.iter().all(|n| n.key != ns), "{}: still listed", t.id);
        note(&t.id, "drop-schema", "namespace_gone=true");
        a.close().await;
        drop(c);
        pg.shutdown().await;
        wall(&t, "pg_xmin_matrix", started);
    }
}

// ---------------------------------------------------------------------------------------------
// #pg-canary on every pass kind

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_canary_no_xid_assigned_by_any_pass() {
    if !docker_enabled() {
        return;
    }
    let _serial = serial().await;
    // Every PostgreSQL target (fix round 1, m-9): the version matrix, the Turkish-locale server
    // and the jumped cluster.
    let every: Vec<DockerTarget> = pg_targets()
        .into_iter()
        .chain(
            ["pg17-tr", "pg17wrap"]
                .into_iter()
                .filter_map(require_target),
        )
        .collect();
    for t in every {
        let started = Instant::now();
        let s = unique("hvm4_t28_canary");
        let _g = DropSchemas {
            t: t.clone(),
            names: vec![s.clone()],
        };
        let mut a = admin(&t).await;
        a.batch(&format!(
            "CREATE SCHEMA {s}; CREATE TABLE {s}.x (id int PRIMARY KEY, a int); \
             COMMENT ON TABLE {s}.x IS 'c'"
        ))
        .await;
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let ns = pg_ns(&t, &s);
        let cancel = CancellationToken::new();
        // Every pass runs `Canary` last and fails when it reads an XID; each must have run it.
        let mut passes = Vec::new();
        let full = pg_pass(&mut c, &ns, Mode::Full, Level::Sources, None).await;
        passes.push(("full", full.report.log.clone()));
        let b = base(ns.clone(), DbmsKind::Postgres, FoldRule::Exact).unwrap();
        let snap = apply(&b, full.result);
        let brief = pg_pass(&mut c, &ns, Mode::Brief, Level::Names, None).await;
        passes.push(("brief", brief.report.log));
        a.batch(&format!("ALTER TABLE {s}.x ADD COLUMN b int"))
            .await;
        let inc = pg_pass(&mut c, &ns, Mode::Incremental, Level::Sources, Some(&snap)).await;
        assert_eq!(inc.report.full_reason, None, "{}", t.id);
        passes.push(("incremental", inc.report.log));
        let sys = pg_pass(
            &mut c,
            &pg_ns(&t, "pg_catalog"),
            Mode::System,
            Level::Structure,
            None,
        )
        .await;
        passes.push(("system", sys.report.log));
        let (_, check) = pass::check_namespace(&mut c, &ns, &snap, &cancel)
            .await
            .unwrap_or_else(|e| panic!("{}: check: {e:?}", t.id));
        passes.push(("check-namespace", check.log));
        let key = key_of(&snap, ObjectKind::Table, &["x"]);
        let (_, stamps) = state::stamps(&mut c, std::slice::from_ref(&key), &cancel)
            .await
            .unwrap_or_else(|e| panic!("{}: stamps: {e:?}", t.id));
        passes.push(("stamps", stamps.log));
        let frag = pass::fragment(&mut c, &ns, &[key], Level::Structure, None, &cancel)
            .await
            .unwrap_or_else(|e| panic!("{}: fragment: {e:?}", t.id));
        passes.push(("fragment", frag.report.log));
        for (what, log) in &passes {
            assert!(log.ran(name::CANARY), "{} {what}: no canary", t.id);
            assert_eq!(
                log.names().last(),
                Some(&name::CANARY),
                "{} {what}: the canary runs last",
                t.id
            );
        }
        // Negative control: the same `Canary` text reads `f` in a transaction that holds an
        // XID and `t` in one that does not.
        let version = full.report.version;
        let canary = pack()
            .unwrap()
            .render(
                name::CANARY,
                &ctx(version, Mode::Full, Level::Structure, &Binds::default()),
            )
            .unwrap()
            .remove(0);
        a.batch("BEGIN").await;
        let without = a.scalar(canary.sql()).await;
        a.batch("SELECT pg_catalog.txid_current()").await;
        let with = a.scalar(canary.sql()).await;
        a.batch("ROLLBACK").await;
        assert_eq!(
            (without.as_str(), with.as_str()),
            ("t", "f"),
            "{}: the canary must see an assigned XID",
            t.id
        );
        note(
            &t.id,
            "canary",
            &format!("passes={} negative_control=held", passes.len()),
        );
        a.close().await;
        drop(c);
        pg.shutdown().await;
        wall(&t, "pg_canary", started);
    }
}

// ---------------------------------------------------------------------------------------------
// A changed server identity (a restart: another postmaster start time) forces `Full`

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_identity_change_forces_full_reason_restart() {
    if !docker_enabled() {
        return;
    }
    let _serial = serial().await;
    let mut tsv = Tsv::new("pg-identity");
    for t in pg_targets() {
        let started = Instant::now();
        let s = unique("hvm4_t28_ident");
        let _g = DropSchemas {
            t: t.clone(),
            names: vec![s.clone()],
        };
        let mut a = admin(&t).await;
        a.batch(&format!(
            "CREATE SCHEMA {s}; CREATE TABLE {s}.x (id int PRIMARY KEY, a int)"
        ))
        .await;
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let ns = pg_ns(&t, &s);
        let first = pg_full(&mut c, &ns, Level::Structure).await;
        let Watermark::PgXid { w, identity } = first.meta().watermark.clone() else {
            panic!("{}: a PostgreSQL watermark", t.id)
        };
        // A container restart is the controller's; the cached watermark is injected instead.
        let mut restarted = identity;
        restarted.start_time_micros += 1;
        let prev = forged(
            &mut c,
            &ns,
            Level::Structure,
            Watermark::PgXid {
                w,
                identity: restarted,
            },
        )
        .await;
        a.batch(&format!("ALTER TABLE {s}.x ADD COLUMN z int"))
            .await;
        let p = pg_pass(
            &mut c,
            &ns,
            Mode::Incremental,
            Level::Structure,
            Some(&prev),
        )
        .await;
        assert_eq!(p.report.full_reason, Some(FullReason::Restart), "{}", t.id);
        let next = apply(&prev, p.result.clone());
        assert_equal(
            &format!("{} restart", t.id),
            &next,
            &pg_full(&mut c, &ns, Level::Structure).await,
        );
        assert!(has(&next, ObjectKind::Column, &["x", "z"]), "{}", t.id);
        tsv.row(&t.id, "restart-full", "n/a", &queries(&p).to_string());
        cell(&t.id, "restart-full");
        // The full pass's own watermark carries the real identity: the next pass is
        // incremental again.
        a.batch(&format!("COMMENT ON TABLE {s}.x IS 'after'")).await;
        let p = pg_pass(
            &mut c,
            &ns,
            Mode::Incremental,
            Level::Structure,
            Some(&next),
        )
        .await;
        assert_eq!(p.report.full_reason, None, "{}", t.id);
        let after = apply(&next, p.result.clone());
        assert_equal(
            &format!("{} after restart", t.id),
            &after,
            &pg_full(&mut c, &ns, Level::Structure).await,
        );
        tsv.row(
            &t.id,
            "after-restart-incremental",
            &changed_rows(&p).to_string(),
            &queries(&p).to_string(),
        );
        cell(&t.id, "after-restart-incremental");
        a.close().await;
        drop(c);
        pg.shutdown().await;
        wall(&t, "pg_identity_restart", started);
    }
}

// ---------------------------------------------------------------------------------------------
// #pg-xmin-freeze: pg17wrap, a cluster jumped past 2^31 XIDs by `pg_resetwal` (pg-wrap/init.sh)

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_xmin_freeze_between_reads_zero_ancient_rows() {
    if !docker_enabled() {
        return;
    }
    let _serial = serial().await;
    let Some(t) = require_target("pg17wrap") else {
        note("pg17wrap", "skipped", "reason=not-in-this-set");
        return;
    };
    let started = Instant::now();
    let mut tsv = Tsv::new("pg-xmin-freeze");
    let mut a = admin(&t).await;
    // The jump is real before anything relies on it (R-W5-1).
    let xmax: u64 = a
        .scalar("SELECT pg_catalog.pg_snapshot_xmax(pg_catalog.pg_current_snapshot())::text")
        .await
        .parse()
        .expect("an xid8");
    note(
        &t.id,
        "xmax",
        &format!("pg_snapshot_xmax={xmax} above_2^31={}", xmax > 1 << 31),
    );
    assert!(
        xmax > 1 << 31,
        "pg17wrap: the cluster did not jump ({xmax})"
    );
    // `public` holds `w`, written and frozen before the jump (its raw xmin is ancient).
    let ns = pg_ns(&t, "public");
    let pg = Pg::connect(&t).await;
    let mut c = pg.meta().await;
    let first = pg_full(&mut c, &ns, Level::Structure).await;
    assert!(has(&first, ObjectKind::Table, &["w"]), "pg17wrap: table w");
    let Watermark::PgXid { identity, .. } = first.meta().watermark.clone() else {
        panic!("pg17wrap: a PostgreSQL watermark")
    };
    // 1. A previous watermark W = 1000 with the current identity: the span to the next XID is
    //    past what 32-bit age() can express, so the pass is Full(wraparound).
    let prev = forged(
        &mut c,
        &ns,
        Level::Structure,
        Watermark::PgXid { w: 1000, identity },
    )
    .await;
    let p = pg_pass(
        &mut c,
        &ns,
        Mode::Incremental,
        Level::Structure,
        Some(&prev),
    )
    .await;
    let span = p.report.snap_xmax - 1000;
    note(
        &t.id,
        "wraparound",
        &format!(
            "span={span} limit={SPAN_LIMIT} reason={}",
            p.report.full_reason.map_or("none", FullReason::code)
        ),
    );
    assert!(span >= SPAN_LIMIT, "pg17wrap: span {span}");
    assert_eq!(
        p.report.full_reason,
        Some(FullReason::Wraparound),
        "pg17wrap"
    );
    let wrapped = apply(&prev, p.result.clone());
    assert_equal(
        "pg17wrap wraparound",
        &wrapped,
        &pg_full(&mut c, &ns, Level::Structure).await,
    );
    tsv.row(&t.id, "wraparound-full", "n/a", &queries(&p).to_string());
    cell(&t.id, "wraparound-full");
    // 2. The next incremental pass from that Full's watermark re-reads no ancient object:
    //    `age(xmin) BETWEEN 0 AND age(W)` never matches a frozen pre-jump row.
    let p = pg_pass(
        &mut c,
        &ns,
        Mode::Incremental,
        Level::Structure,
        Some(&wrapped),
    )
    .await;
    assert_eq!(p.report.full_reason, None, "pg17wrap: incremental");
    assert_eq!(changed_rows(&p), 0, "pg17wrap: changed rows");
    assert!(reads(&p).is_empty(), "pg17wrap: re-read {:?}", reads(&p));
    let quiet = apply(&wrapped, p.result.clone());
    assert_equal(
        "pg17wrap after wraparound",
        &quiet,
        &pg_full(&mut c, &ns, Level::Structure).await,
    );
    tsv.row(&t.id, "zero-ancient-rows", "0", &queries(&p).to_string());
    cell(&t.id, "zero-ancient-rows");
    // 2b. A table written after the jump is young: it alone is re-read.
    let young = unique("hvm4_t28_young");
    let _drop_young = SqlOnDrop {
        t: t.clone(),
        sql: format!("DROP TABLE IF EXISTS public.{young}"),
    };
    a.batch(&format!("CREATE TABLE public.{young} (a int)"))
        .await;
    let p = pg_pass(
        &mut c,
        &ns,
        Mode::Incremental,
        Level::Structure,
        Some(&quiet),
    )
    .await;
    assert_eq!(p.report.full_reason, None, "pg17wrap: young");
    let next = apply(&quiet, p.result.clone());
    assert_equal(
        "pg17wrap young table",
        &next,
        &pg_full(&mut c, &ns, Level::Structure).await,
    );
    assert!(has(&next, ObjectKind::Table, &[young.as_str()]), "pg17wrap");
    tsv.row(
        &t.id,
        "young-table-after-jump",
        &changed_rows(&p).to_string(),
        &queries(&p).to_string(),
    );
    cell(&t.id, "young-table-after-jump");
    // 3. Regression: the predicate written `age(xmin) <= age(W)` matches the frozen pre-jump
    //    rows (negative age); `BETWEEN 0 AND age(W)` matches none of them.
    let Watermark::PgXid { w, .. } = wrapped.meta().watermark.clone() else {
        panic!("pg17wrap: a PostgreSQL watermark")
    };
    let w32 = w & u64::from(u32::MAX);
    let counts = a
        .scalar(&format!(
            "SELECT count(*) FILTER (WHERE pg_catalog.age(c.xmin) <= pg_catalog.age('{w32}'::pg_catalog.xid))::text \
             || ' ' || count(*) FILTER (WHERE c.xmin::text::bigint < 2147483648 \
                 AND pg_catalog.age(c.xmin) BETWEEN 0 AND pg_catalog.age('{w32}'::pg_catalog.xid))::text \
             || ' ' || count(*)::text \
             FROM pg_catalog.pg_class c"
        ))
        .await;
    let n: Vec<u64> = counts
        .split(' ')
        .map(|x| x.parse().expect("a count"))
        .collect();
    note(
        &t.id,
        "pg_class",
        &format!(
            "le_matches={} ancient_between_matches={} rows={} ver01_le_matches=220 ver01_rows=416",
            n[0], n[1], n[2]
        ),
    );
    assert!(n[0] > 0, "pg17wrap: `<=` must match the frozen rows");
    assert_eq!(n[1], 0, "pg17wrap: BETWEEN matched an ancient row");
    a.close().await;
    drop(c);
    pg.shutdown().await;
    wall(&t, "pg_xmin_freeze", started);
}

// ---------------------------------------------------------------------------------------------
// HV-INTRO-14 on the matrix: a tr-TR ICU database, and the process locale

/// A database `hvm4_t28_<slug>_…` with ICU locale `tr-TR`, dropped when the guard goes.
async fn tr_database(t: &DockerTarget, slug: &str) -> (String, SqlOnDrop) {
    let db = unique(&format!("hvm4_t28_{slug}"));
    let guard = SqlOnDrop {
        t: t.clone(),
        sql: format!("DROP DATABASE IF EXISTS {db} WITH (FORCE)"),
    };
    let mut a = admin(t).await;
    a.batch(&format!(
        "CREATE DATABASE {db} TEMPLATE template0 ENCODING 'UTF8' LOCALE_PROVIDER icu \
         ICU_LOCALE 'tr-TR' LC_COLLATE 'C' LC_CTYPE 'C'"
    ))
    .await;
    a.close().await;
    let mut a = admin_on(t, Some(&db)).await;
    a.batch("CREATE TABLE ITEMS (ID int, \"İsim\" text); CREATE TABLE \"Müşteri\" (\"ıd\" int)")
        .await;
    a.close().await;
    (db, guard)
}

/// Every statement the pack can send at `version`, rendered with fixed binds, as text.
fn rendered_pack(version: u32, ns_oid: u32, rel: u32) -> Vec<String> {
    let p = pack().unwrap();
    let mut out = Vec::new();
    for section in p.sections() {
        let mut seen = std::collections::BTreeSet::new();
        for v in p.reachable_variants(section) {
            let binds = Binds {
                ns: Some(if v.mode == Mode::System { 11 } else { ns_oid }),
                w: Some(3),
                schema: Some("public".into()),
                oids: Some(vec![rel]),
            };
            let cx = ctx(version, v.mode, v.level, &binds);
            for q in p.render(section, &cx).unwrap() {
                let line = format!("{section} {q:?}");
                if seen.insert(line.clone()) {
                    out.push(line);
                }
            }
        }
    }
    out
}

fn oid(o: Option<ServerId>) -> u32 {
    match o {
        Some(ServerId::Oid(x)) => x,
        other => panic!("an oid: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_tr_icu_no_lower_or_ilike_in_query_log() {
    if !docker_enabled() {
        return;
    }
    let _serial = serial().await;
    let folding =
        regex::Regex::new(r"(?i)\blower\s*\(|\bupper\s*\(|\bilike\b|\bcollate\b").unwrap();
    for t in tr_targets() {
        let started = Instant::now();
        let (db, _drop) = tr_database(&t, "tr").await;
        // The server's own trap (alg/02 #pg-turkish), recorded.
        let mut a = admin_on(&t, Some(&db)).await;
        let lowered = a.scalar("SELECT pg_catalog.lower('ITEMS')").await;
        let trap = a
            .scalar(
                "SELECT count(*)::text FROM pg_catalog.pg_class \
                 WHERE pg_catalog.lower(relname::text) = pg_catalog.lower('ITEMS')",
            )
            .await;
        note(
            &t.id,
            "tr-icu",
            &format!("lower_ITEMS={lowered} lower_match_rows={trap}"),
        );
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta_on(Some(db.as_str())).await;
        let ns = NsKey::schema([db.as_str(), "public"]);
        let p = pg_pass(&mut c, &ns, Mode::Full, Level::Sources, None).await;
        let version = p.report.version;
        let b = base(ns.clone(), DbmsKind::Postgres, FoldRule::Exact).unwrap();
        let n = apply(&b, p.result.clone());
        // Unquoted `ITEMS` was stored as `items`; the typed `ITEMS` finds it.
        assert!(has(&n, ObjectKind::Table, &["items"]), "{}", t.id);
        assert!(
            matches!(
                n.match_named(
                    KindGroup::Relations,
                    "ITEMS",
                    false,
                    &IdentCtx::new(DialectId::Postgres)
                ),
                Lookup::Found(_)
            ),
            "{}: ITEMS",
            t.id
        );
        // No case folding in any statement the pack can send, and every section the pass ran
        // is one of those.
        let texts = rendered_pack(
            version,
            oid(n.root().server_id()),
            oid(key_of(&n, ObjectKind::Table, &["items"]).server_id),
        );
        for line in &texts {
            assert!(!folding.is_match(line), "{}: {line}", t.id);
        }
        for s in p.report.log.names() {
            assert!(
                texts.iter().any(|l| l.starts_with(&format!("{s} "))),
                "{}: {s} is not a pack section",
                t.id
            );
        }
        // One incremental step in the Turkish database, against the oracle.
        a.batch("ALTER TABLE items ADD COLUMN \"İl\" text").await;
        let p = pg_pass(&mut c, &ns, Mode::Incremental, Level::Sources, Some(&n)).await;
        assert_eq!(p.report.full_reason, None, "{}", t.id);
        let next = apply(&n, p.result);
        assert_equal(
            &format!("{} tr-icu", t.id),
            &next,
            &pg_full(&mut c, &ns, Level::Sources).await,
        );
        cell(&t.id, "tr-icu-add-column");
        note(
            &t.id,
            "tr-icu",
            &format!("statements={} folding=none", texts.len()),
        );
        a.close().await;
        drop(c);
        pg.shutdown().await;
        wall(&t, "pg_tr_icu", started);
    }
}

/// The child process's environment: the target id and the database (the parent sets them only
/// on the child's command line; nothing here calls `set_var`).
const CHILD_TARGET: &str = "HV_T28_CHILD_TARGET";
const CHILD_DB: &str = "HV_T28_CHILD_DB";
const BEGIN: &str = "HV-T28-CHILD-BEGIN";
const END: &str = "HV-T28-CHILD-END";

/// Runs this test binary's [`locale_child_reads_the_catalog`] under `LANG=LC_ALL=locale`.
fn child_output(t: &DockerTarget, db: &str, locale: &str) -> String {
    let exe = std::env::current_exe().expect("the test binary");
    let out = std::process::Command::new(exe)
        .args([
            "--exact",
            "locale_child_reads_the_catalog",
            "--ignored",
            "--nocapture",
            "--test-threads",
            "1",
        ])
        .env("LANG", locale)
        .env("LC_ALL", locale)
        .env(CHILD_TARGET, &t.id)
        .env(CHILD_DB, db)
        .output()
        .expect("the child runs");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{} {locale}: the child failed: {}",
        t.id,
        String::from_utf8_lossy(&out.stderr)
    );
    let start = text.find(BEGIN).expect("the child's begin marker") + BEGIN.len();
    let end = text.find(END).expect("the child's end marker");
    text[start..end].to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn catalog_bytes_identical_under_lang_tr_and_c() {
    if !docker_enabled() {
        return;
    }
    let _serial = serial().await;
    // Whether the client host has `tr_TR` (fix round 1, m-5): without it glibc falls back to
    // C and the two children are trivially alike, so the cell says which case it measured.
    // std and the pure-Rust drivers read no locale either way: the test is a regression guard.
    let installed = tr_locale_installed();
    let case = match installed {
        Some(true) => "lang-tr-installed-eq-lang-c",
        Some(false) => "lang-tr-not-installed-eq-lang-c",
        None => "lang-tr-unknown-eq-lang-c",
    };
    for t in tr_targets() {
        let started = Instant::now();
        note(
            &t.id,
            "client-locale",
            &format!(
                "tr_TR_installed={}",
                installed.map_or("unknown", |b| if b { "true" } else { "false" })
            ),
        );
        let (db, _drop) = tr_database(&t, "lang").await;
        let (tt, d) = (t.clone(), db.clone());
        let tr = tokio::task::spawn_blocking(move || child_output(&tt, &d, "tr_TR.UTF-8"))
            .await
            .expect("joined");
        let (tt, d) = (t.clone(), db.clone());
        let c = tokio::task::spawn_blocking(move || child_output(&tt, &d, "C"))
            .await
            .expect("joined");
        assert!(tr.lines().count() > 10, "{}: the child printed {tr}", t.id);
        if tr != c {
            let only_tr: Vec<_> = tr.lines().filter(|l| !c.lines().any(|x| x == *l)).collect();
            let only_c: Vec<_> = c.lines().filter(|l| !tr.lines().any(|x| x == *l)).collect();
            panic!(
                "{}: LANG=tr_TR.UTF-8 and LANG=C differ\n only tr: {only_tr:#?}\n only C: {only_c:#?}",
                t.id
            );
        }
        note(
            &t.id,
            "lang",
            &format!("bytes={} lines={}", tr.len(), tr.lines().count()),
        );
        cell(&t.id, case);
        wall(&t, "catalog_bytes_lang", started);
    }
}

/// Whether `locale -a` on this (client) host lists a `tr_TR` locale; `None` when it cannot run.
fn tr_locale_installed() -> Option<bool> {
    let out = std::process::Command::new("locale")
        .arg("-a")
        .output()
        .ok()?;
    out.status.success().then(|| {
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .any(|l| l.to_ascii_lowercase().starts_with("tr_tr"))
    })
}

/// **A helper, not a test and not a gate cell.** The child process of
/// [`catalog_bytes_identical_under_lang_tr_and_c`]. In every normal `--ignored` run it returns
/// at once with an empty body (it does nothing unless the parent set [`CHILD_TARGET`] on the
/// child's command). As the child, it prints every statement the pack can send at the
/// server's version and the canonical snapshot of a full pass of `<db>.public`, between
/// markers. It prints no `HV-FACT`, so it is never counted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn locale_child_reads_the_catalog() {
    let (Ok(id), Ok(db)) = (std::env::var(CHILD_TARGET), std::env::var(CHILD_DB)) else {
        return;
    };
    let t = require_target(&id).expect("the parent's target");
    let pg = Pg::connect(&t).await;
    let mut c = pg.meta_on(Some(db.as_str())).await;
    let ns = NsKey::schema([db.as_str(), "public"]);
    let p = pg_pass(&mut c, &ns, Mode::Full, Level::Sources, None).await;
    let b = base(ns.clone(), DbmsKind::Postgres, FoldRule::Exact).unwrap();
    let n = apply(&b, p.result.clone());
    let mut lines = rendered_pack(
        p.report.version,
        oid(n.root().server_id()),
        oid(key_of(&n, ObjectKind::Table, &["items"]).server_id),
    );
    lines.push(format!("sections {:?}", p.report.log.names()));
    lines.extend(canon(&n));
    let mut text = format!("{BEGIN}\n");
    for l in lines {
        text.push_str(&l.replace('\n', "\\n"));
        text.push('\n');
    }
    text.push_str(END);
    text.push('\n');
    {
        use std::io::Write as _;
        let mut out = std::io::stdout().lock();
        out.write_all(text.as_bytes()).expect("stdout");
        out.flush().expect("stdout");
    }
    drop(c);
    pg.shutdown().await;
}

// ---------------------------------------------------------------------------------------------
// HV-INTRO-11 on the matrix: a revoked USAGE never deletes silently

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn revoke_usage_fires_mass_drop_guard() {
    if !docker_enabled() {
        return;
    }
    let _serial = serial().await;
    const TABLES: u32 = 24;
    for t in tr_targets() {
        let started = Instant::now();
        let role = unique("hvm4_t28_rv");
        let s = unique("hvm4_t28_rvs");
        // Dropped in reverse: the USAGE restore, then the schema, then the role.
        let _role = SqlOnDrop {
            t: t.clone(),
            sql: format!(
                "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = '{role}') \
                 THEN EXECUTE 'DROP OWNED BY {role}'; EXECUTE 'DROP ROLE {role}'; END IF; END $$"
            ),
        };
        let _schema = DropSchemas {
            t: t.clone(),
            names: vec![s.clone()],
        };
        let _restore = SqlOnDrop {
            t: t.clone(),
            sql: format!(
                "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname = '{s}') \
                 AND EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = '{role}') \
                 THEN EXECUTE 'GRANT USAGE ON SCHEMA {s} TO {role}'; END IF; END $$"
            ),
        };
        let mut a = admin(&t).await;
        // The role signs in with the target's own throwaway password.
        a.batch(&format!(
            "CREATE ROLE {role} LOGIN PASSWORD '{}'",
            t.password
        ))
        .await;
        let tables: String = (1..=TABLES)
            .map(|i| format!("CREATE TABLE {s}.t{i} (a int);"))
            .collect();
        a.batch(&format!(
            "CREATE SCHEMA {s}; {tables} GRANT USAGE ON SCHEMA {s} TO {role}; \
             GRANT SELECT ON ALL TABLES IN SCHEMA {s} TO {role}"
        ))
        .await;
        let as_role = DockerTarget {
            user: role.clone(),
            ..t.clone()
        };
        let pg = Pg::connect(&as_role).await;
        let mut c = pg.meta().await;
        let ns = pg_ns(&t, &s);
        let prev = pg_full(&mut c, &ns, Level::Structure).await;
        let count = |n: &NamespaceSnapshot| {
            n.objs()
                .iter()
                .filter(|o| o.kind() == ObjectKind::Table)
                .count()
        };
        assert_eq!(count(&prev), TABLES as usize, "{}", t.id);
        a.batch(&format!("REVOKE USAGE ON SCHEMA {s} FROM {role}"))
            .await;
        let mut r = admin(&as_role).await;
        let usage = r
            .scalar(&format!(
                "SELECT pg_catalog.has_schema_privilege('{s}', 'USAGE')::text"
            ))
            .await;
        r.close().await;
        assert_eq!(usage, "false", "{}: USAGE revoked", t.id);
        // A background pass after the REVOKE: never a silent deletion. Accepted outcomes:
        // existence still lists the tables (PostgreSQL's catalogs ignore schema USAGE) and
        // nothing is dropped; it lists fewer and the mass-drop guard holds the result; the
        // table existence set failed closed (`ExistSet::Failed`: the merge deletes nothing);
        // or the pass is refused with `Permission` (nothing is published).
        let after = pass::introspect(
            &mut c,
            &ns,
            PassOpts {
                mode: Mode::Incremental,
                level: Level::Structure,
                prev: Some(&prev),
                ..PassOpts::default()
            },
            &CancellationToken::new(),
        )
        .await;
        let outcome_a = match after {
            Err(e) => {
                assert!(
                    matches!(e, hacivat_introspect::IntroError::Permission { .. }),
                    "{}: {e:?}",
                    t.id
                );
                "pass_refused_permission".to_string()
            }
            Ok(p) => {
                let listed = match p.result.existent.get(&ObjectKind::Table) {
                    Some(ExistSet::Ok(ids)) => Some(ids.len()),
                    Some(ExistSet::Failed(_)) => None,
                    other => panic!("{}: table existence {other:?}", t.id),
                };
                let outcome =
                    merge(Some(&prev), p.result, &MergePolicy::default()).expect("the merge runs");
                let shown = match &outcome {
                    MergeOutcome::Applied { delta, .. } => {
                        assert!(
                            delta.dropped.is_empty(),
                            "{}: a silent deletion after REVOKE: {:?}",
                            t.id,
                            delta.dropped
                        );
                        "applied_no_drops".to_string()
                    }
                    MergeOutcome::PendingConfirmation { vanished, .. } => {
                        format!("pending_confirmation vanished={vanished}")
                    }
                };
                if listed == Some(0) {
                    assert!(
                        matches!(outcome, MergeOutcome::PendingConfirmation { .. }),
                        "{}: existence empty but no guard",
                        t.id
                    );
                }
                format!(
                    "existence_tables={} outcome={shown}",
                    listed.map_or_else(|| "failed_closed".to_string(), |n| n.to_string())
                )
            }
        };
        note(&t.id, "revoke-usage", &outcome_a);
        drop(c);
        pg.shutdown().await;
        // The guard on a live pass (part B), on a fresh session with USAGE given back, so the
        // outcome above cannot decide this part: most tables really go, and a background merge
        // asks first.
        a.batch(&format!("GRANT USAGE ON SCHEMA {s} TO {role}"))
            .await;
        let drops: String = (1..=20).map(|i| format!("DROP TABLE {s}.t{i};")).collect();
        a.batch(&drops).await;
        let pg = Pg::connect(&as_role).await;
        let mut c = pg.meta().await;
        let p = pg_pass(
            &mut c,
            &ns,
            Mode::Incremental,
            Level::Structure,
            Some(&prev),
        )
        .await;
        let outcome = merge(Some(&prev), p.result, &MergePolicy::default()).expect("merge");
        assert!(
            matches!(
                outcome,
                MergeOutcome::PendingConfirmation { vanished: 20, .. }
            ),
            "{}: the guard: {outcome:?}",
            t.id
        );
        note(
            &t.id,
            "mass-drop-guard",
            "outcome=pending_confirmation vanished=20",
        );
        a.close().await;
        drop(c);
        pg.shutdown().await;
        wall(&t, "revoke_usage", started);
    }
}

// ---------------------------------------------------------------------------------------------
// MySQL and MariaDB helpers

const MYSQL: [&str; 2] = ["mysql80", "mysql84"];
const MARIADB: [&str; 3] = ["mariadb106", "mariadb1011", "mariadb114"];
const LONG_ALTER: [&str; 5] = [
    "mysql80",
    "mysql84",
    "mariadb106",
    "mariadb1011",
    "mariadb114",
];

fn my_targets(ids: &[&str]) -> Vec<DockerTarget> {
    ids.iter().filter_map(|id| require_target(id)).collect()
}

fn match_all() -> hacivat_introspect::filter::CompiledFilter {
    hacivat_introspect::filter::parse_filter(
        "",
        hacivat_introspect::pack::DialectId::MySql,
        hacivat_introspect::filter::FilterRole::Introspect,
    )
    .expect("the empty filter")
}

/// One pass on `db` merged onto `prev` (or the empty namespace) as a user refresh.
async fn my_pass(
    t: &DockerTarget,
    db: &my::Db,
    mode: hacivat_introspect::Mode,
    prev: Option<&NamespaceSnapshot>,
) -> (NamespaceSnapshot, NamespaceResult, PassStats) {
    let mut c = my::meta(t, &db.name).await;
    let (res, stats) = MySqlIntrospector { flavor: t.engine }
        .introspect_with_stats(
            &mut c,
            &db.ns(),
            mode,
            Level::Structure,
            &match_all(),
            prev,
            &CancellationToken::new(),
        )
        .await
        .unwrap_or_else(|e| panic!("{}: {mode:?}: {e:?}", t.id));
    let start = match prev {
        Some(p) => p.clone(),
        None => base(db.ns(), t.engine, FoldRule::Exact).expect("a base"),
    };
    (apply(&start, res.clone()), res, stats)
}

fn my_w(s: &NamespaceSnapshot) -> i64 {
    match s.meta().watermark {
        Watermark::MyTime { at_utc_micros, .. } => at_utc_micros,
        ref w => panic!("a MySQL watermark: {w:?}"),
    }
}

fn reread(stats: &PassStats) -> Vec<String> {
    stats.relations.iter().map(|n| n.to_string()).collect()
}

/// An incremental pass over `prev` checked against a fresh full pass (the G4 oracle).
async fn my_cell(
    t: &DockerTarget,
    db: &my::Db,
    prev: &NamespaceSnapshot,
    case: &str,
) -> (NamespaceSnapshot, NamespaceResult, PassStats) {
    let (inc, res, stats) = my_pass(t, db, hacivat_introspect::Mode::Incremental, Some(prev)).await;
    let (full, _, _) = my_pass(t, db, hacivat_introspect::Mode::Full, None).await;
    assert_equal(&format!("{} {case}", t.id), &inc, &full);
    (inc, res, stats)
}

/// Waits until the server clock is two seconds past `prev`'s watermark and takes one
/// incremental pass, so objects stamped before it leave the R-M4-39 window and the next pass
/// re-reads exactly what the next step changes.
async fn settle(
    t: &DockerTarget,
    db: &my::Db,
    clock: &mut my::Clock,
    prev: &NamespaceSnapshot,
) -> NamespaceSnapshot {
    clock.wait_past(my_w(prev) + 2_000_000).await;
    let (s, _, stats) = my_pass(t, db, hacivat_introspect::Mode::Incremental, Some(prev)).await;
    assert_eq!(stats.full_reason, None, "{}: settle", t.id);
    s
}

/// ver/01 #mysql-is-times: the ALTER list on table `tt`; `true` when the step changes `tt`.
const MY_STEPS: [(&str, &str, bool); 9] = [
    (
        "create",
        "CREATE TABLE tt (id INT PRIMARY KEY, a INT)",
        true,
    ),
    ("insert", "INSERT INTO tt VALUES (1, 1)", false),
    (
        "add-column-instant",
        "ALTER TABLE tt ADD COLUMN b INT, ALGORITHM=INSTANT",
        true,
    ),
    (
        "add-index-inplace",
        "ALTER TABLE tt ADD INDEX ix_a (a), ALGORITHM=INPLACE",
        true,
    ),
    ("comment", "ALTER TABLE tt COMMENT = 'tt table'", true),
    (
        "modify-type-copy",
        "ALTER TABLE tt MODIFY a BIGINT, ALGORITHM=COPY",
        true,
    ),
    (
        "rename-column",
        "ALTER TABLE tt RENAME COLUMN b TO b2",
        true,
    ),
    (
        "create-trigger",
        "CREATE TRIGGER tt_bi BEFORE INSERT ON tt FOR EACH ROW SET NEW.a = NEW.a",
        true,
    ),
    (
        "add-column-copy",
        "ALTER TABLE tt ADD COLUMN c INT, ALGORITHM=COPY",
        true,
    ),
];

const TT_CREATE_TIME: &str = "SELECT CAST(CREATE_TIME AS CHAR) FROM INFORMATION_SCHEMA.TABLES \
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'tt'";

// ---------------------------------------------------------------------------------------------
// #mysql-is-times

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn mysql_is_times_matrix_incremental_equals_full() {
    if !docker_enabled() {
        return;
    }
    let _serial = serial().await;
    let mut tsv = Tsv::new("mysql-is-times");
    for t in my_targets(&MYSQL) {
        let started = Instant::now();
        let db = my::Db::create(&t, "istimes").await;
        db.run("CREATE TABLE other (id INT PRIMARY KEY)").await;
        let mut clock = my::Clock::open(&t).await;
        let (mut prev, _, _) = my_pass(&t, &db, hacivat_introspect::Mode::Full, None).await;
        for (id, sql, changes_tt) in MY_STEPS {
            let label = format!("{} {id}", t.id);
            prev = settle(&t, &db, &mut clock, &prev).await;
            let before = my::rows(&t, Some(&db.name), TT_CREATE_TIME).await;
            db.run(sql).await;
            if id == "create-trigger" {
                // ver/01: CREATE TRIGGER leaves CREATE_TIME alone; the marker's
                // TRIGGERS.CREATED part catches it.
                let after = my::rows(&t, Some(&db.name), TT_CREATE_TIME).await;
                assert_eq!(before, after, "{label}: CREATE_TIME moved");
                note(&t.id, "create-trigger", "create_time_unchanged=true");
            }
            let (next, _, stats) = my_cell(&t, &db, &prev, id).await;
            assert_eq!(stats.full_reason, None, "{label}: incremental");
            let want: Vec<String> = if changes_tt {
                vec!["tt".into()]
            } else {
                Vec::new()
            };
            assert_eq!(reread(&stats), want, "{label}: re-read");
            assert!(
                stats.routines.is_empty() && stats.events.is_empty(),
                "{label}: {stats:?}"
            );
            tsv.row(&t.id, id, &stats.relations.len().to_string(), "n/a");
            cell(&t.id, id);
            prev = next;
        }
        clock.close().await;
        wall(&t, "mysql_is_times_matrix", started);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn mysql_same_second_ddl_caught() {
    if !docker_enabled() {
        return;
    }
    let _serial = serial().await;
    for t in my_targets(&MYSQL) {
        let started = Instant::now();
        let db = my::Db::create(&t, "samesec").await;
        db.run("CREATE TABLE customers (id INT PRIMARY KEY)").await;
        // DDL, a full pass and DDL inside one server second (when the three cross a second
        // boundary the markers differ instead: caught too, and recorded).
        let mut clock = my::Clock::open(&t).await;
        let second = clock.now().await / 1_000_000;
        clock.wait_past((second + 1) * 1_000_000).await;
        db.run("ALTER TABLE customers ADD COLUMN a INT").await;
        let (s0, _, _) = my_pass(&t, &db, hacivat_introspect::Mode::Full, None).await;
        db.run("ALTER TABLE customers ADD COLUMN b INT").await;
        let created = my::rows(
            &t,
            Some(&db.name),
            "SELECT UNIX_TIMESTAMP(CREATE_TIME) FROM INFORMATION_SCHEMA.TABLES \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'customers'",
        )
        .await;
        let created: i64 = created[0][0]
            .split('.')
            .next()
            .unwrap_or_default()
            .parse()
            .expect("a second");
        let same = created == my_w(&s0) / 1_000_000;
        let (s1, _, stats) = my_cell(&t, &db, &s0, "same-second-ddl").await;
        assert_eq!(stats.full_reason, None, "{}", t.id);
        assert!(reread(&stats).contains(&"customers".into()), "{}", t.id);
        assert!(
            has(&s1, ObjectKind::Column, &["customers", "b"]),
            "{}",
            t.id
        );
        note(&t.id, "same-second-ddl", &format!("same_second={same}"));
        cell(&t.id, "same-second-ddl");
        clock.close().await;
        wall(&t, "mysql_same_second", started);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn mysql_global_time_zone_plus_three_markers_compare() {
    if !docker_enabled() {
        return;
    }
    let _serial = serial().await;
    for t in my_targets(&MYSQL) {
        let started = Instant::now();
        let was = my::rows(&t, None, "SELECT @@GLOBAL.time_zone").await[0][0].clone();
        // Restored when the guard goes, also after a failure.
        let _restore = my::GlobalGuard {
            t: t.clone(),
            name: "time_zone",
            value: was,
        };
        my::exec(
            &t,
            Login::Fixture,
            None,
            &hacivat_testkit::fixture_sql::set_global("time_zone", "+03:00"),
        )
        .await
        .unwrap_or_else(|e| panic!("{}: SET GLOBAL: {e}", t.id));
        let db = my::Db::create(&t, "tz").await;
        db.run("CREATE TABLE orders (id INT PRIMARY KEY)").await;
        let mut clock = my::Clock::open(&t).await;
        let (s0, _, _) = my_pass(&t, &db, hacivat_introspect::Mode::Full, None).await;
        // The metadata session runs in UTC whatever the global zone: out of the window a pass
        // re-reads nothing, and an ALTER is still seen.
        let s1 = settle(&t, &db, &mut clock, &s0).await;
        clock.wait_past(my_w(&s1) + 2_000_000).await;
        let (s2, _, stats) = my_cell(&t, &db, &s1, "tz-plus-three-quiet").await;
        assert!(reread(&stats).is_empty(), "{}: {stats:?}", t.id);
        cell(&t.id, "tz-plus-three-quiet");
        db.run("ALTER TABLE orders ADD COLUMN note TEXT").await;
        let (_, _, stats) = my_cell(&t, &db, &s2, "tz-plus-three-alter").await;
        assert_eq!(stats.full_reason, None, "{}", t.id);
        assert_eq!(reread(&stats), ["orders"], "{}", t.id);
        cell(&t.id, "tz-plus-three-alter");
        clock.close().await;
        wall(&t, "mysql_time_zone", started);
    }
}

/// F1 on the matrix: an `ALTER … ALGORITHM=COPY` on 400k rows with a pass while it runs;
/// MySQL stamps the start, MariaDB the end. The next pass sees the new column.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn long_alter_straddling_a_pass_is_seen() {
    if !docker_enabled() {
        return;
    }
    let _serial = serial().await;
    for t in my_targets(&LONG_ALTER) {
        let started = Instant::now();
        let db = my::Db::create(&t, "longalter").await;
        db.run("CREATE TABLE big (id INT PRIMARY KEY AUTO_INCREMENT, v INT)")
            .await;
        db.run("INSERT INTO big (v) VALUES (1),(2),(3),(4),(5),(6),(7),(8)")
            .await;
        for _ in 0..16 {
            db.run("INSERT INTO big (v) SELECT v FROM big").await;
            let n = my::rows(&t, Some(&db.name), "SELECT COUNT(*) FROM big").await;
            if n[0][0].parse::<u64>().unwrap_or(0) >= 400_000 {
                break;
            }
        }
        let (s0, _, _) = my_pass(&t, &db, hacivat_introspect::Mode::Full, None).await;
        let alter = {
            let (t, name) = (t.clone(), db.name.clone());
            tokio::spawn(async move {
                my::exec(
                    &t,
                    Login::Test,
                    Some(&name),
                    "ALTER TABLE big ADD COLUMN w INT, ALGORITHM=COPY",
                )
                .await
            })
        };
        // The pass starts once the ALTER is running (a condition on the process list; the
        // ALTER runs as the same login, so it is listed).
        let mut clock = my::Clock::open(&t).await;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            let running = clock
                .scalar(
                    "SELECT COUNT(*) FROM INFORMATION_SCHEMA.PROCESSLIST \
                     WHERE INFO LIKE 'ALTER TABLE big ADD COLUMN w%'",
                )
                .await;
            if running != "0" || alter.is_finished() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{}: the ALTER never ran",
                t.id
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let (mid, _, _) = my_pass(&t, &db, hacivat_introspect::Mode::Incremental, Some(&s0)).await;
        let straddled = !alter.is_finished();
        alter.await.expect("joined").expect("the ALTER");
        let (s1, _, stats) = my_cell(&t, &db, &mid, "long-alter").await;
        assert!(
            reread(&stats).contains(&"big".into()) || stats.full_reason.is_some(),
            "{}",
            t.id
        );
        assert!(has(&s1, ObjectKind::Column, &["big", "w"]), "{}", t.id);
        note(
            &t.id,
            "long-alter",
            &format!(
                "straddled={straddled} reason={}",
                stats.full_reason.unwrap_or("none")
            ),
        );
        // The cell names whether F1 was exercised (the ALTER still ran when the middle pass
        // ended), so a run where it was not is visible in the log (fix round 1, m-7).
        cell(
            &t.id,
            if straddled {
                "long-alter-straddled"
            } else {
                "long-alter-not-straddled"
            },
        );
        clock.close().await;
        wall(&t, "long_alter", started);
    }
}

// ---------------------------------------------------------------------------------------------
// MariaDB (R-M4-10): Full per namespace, equal to a fresh full pass after each ALTER

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn mariadb_full_per_namespace_equals_fresh_full_after_each_alter() {
    if !docker_enabled() {
        return;
    }
    let _serial = serial().await;
    let mut tsv = Tsv::new("mariadb");
    for t in my_targets(&MARIADB) {
        let started = Instant::now();
        let db = my::Db::create(&t, "mfull").await;
        db.run("CREATE TABLE other (id INT PRIMARY KEY)").await;
        let (mut prev, res, _) = my_pass(&t, &db, hacivat_introspect::Mode::Full, None).await;
        assert_eq!(res.new_watermark, Watermark::None, "{}", t.id);
        for (id, sql, _) in MY_STEPS {
            db.run(sql).await;
            let (next, res, stats) = my_cell(&t, &db, &prev, id).await;
            assert!(res.full, "{} {id}: full", t.id);
            assert_eq!(stats.full_reason, Some("flavor"), "{} {id}", t.id);
            tsv.row(&t.id, id, "n/a", "n/a");
            cell(&t.id, id);
            prev = next;
        }
        wall(&t, "mariadb_full_per_namespace", started);
    }
}
