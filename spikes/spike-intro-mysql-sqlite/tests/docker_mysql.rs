//! The MySQL/MariaDB introspector on every MySQL-family line of the active set (`mysql84` on
//! `pr`; `mysql80`, `mariadb106`, `mariadb1011`, `mariadb114` too on `full`):
//!
//! - every object shape the M4 catalog names (generated/invisible columns, functional and
//!   descending keys, PK/UK/FK/CHECK, views, routines with parameters, triggers, events,
//!   comments), read through a **read-only** metadata connection with bound names;
//! - the ver/01 `#mysql-is-times` DDL matrix: after each step the MySQL incremental pass,
//!   merged onto the previous snapshot, equals a fresh full pass; on every server the step's
//!   effect on `CREATE_TIME` is printed (`CTIME` lines: the MariaDB-marker evidence);
//! - a long `ALGORITHM=COPY` ALTER straddling a pass;
//! - the 10k-table / 160k-column measurement (`HACIVAT_INTRO_PERF=1`, mysql84 only).
//!
//! Fixtures are tables/views/routines/triggers/events named `hvi4_…` in `hv_test` (the login
//! cannot create databases), created and dropped by these tests only.
//! `#[ignore = "docker"]`; run with
//! `<main>/target/docker/run-docker-tests.sh --set full -p spike-intro-mysql-sqlite --test docker_mysql`.

#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_driver::{DriverConnect, Endpoint, InsecureAuth, RawConn};
use hacivat_driver_mysql::{MySqlConn, connect_conn};
use hacivat_net::{
    AddressClass, Target, TlsMode, TlsParams, TlsSettings, build_tls_params, direct,
    install_crypto_provider,
};
use hacivat_proto::DbmsKind;
use hacivat_testkit::{DockerTarget, TlsDefault, docker_enabled, targets};
use secrecy::SecretString;
use spike_intro_mysql_sqlite::conn::{CatalogConn, TypedCatalog};
use spike_intro_mysql_sqlite::model::*;
use spike_intro_mysql_sqlite::mysql::{self, Flavor, ServerCtx};

const SCHEMA: &str = "hv_test";
const P: &str = "hvi4_";

/// The tests share `hv_test` and compare whole-schema snapshots: one at a time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn mysql_family() -> Vec<DockerTarget> {
    let mut ts = targets(Some(DbmsKind::MySql));
    ts.extend(targets(Some(DbmsKind::MariaDb)));
    assert!(!ts.is_empty(), "no MySQL-family target in the selected set");
    ts
}

fn params(t: &DockerTarget) -> Option<TlsParams> {
    install_crypto_provider();
    let mode = match t.tls_default {
        TlsDefault::SelfSigned => TlsMode::Require,
        TlsDefault::None => TlsMode::Auto,
    };
    let settings = TlsSettings {
        mode,
        ..TlsSettings::default()
    };
    build_tls_params(&settings, &t.host, t.port, AddressClass::Loopback, None)
        .unwrap_or_else(|e| panic!("{}: {e:?}", t.id))
}

fn request(t: &DockerTarget, read_only: bool) -> DriverConnect {
    let addr: SocketAddr = format!("{}:{}", t.host, t.port).parse().unwrap();
    let target = Target::Tcp {
        host: t.host.clone(),
        port: t.port,
    };
    DriverConnect {
        endpoint: Endpoint::Network {
            factory: direct(target.clone(), vec![addr], secs(5)),
            direct: Some(target),
            resolved: vec![addr],
            tls_host: t.host.clone(),
        },
        tls: params(t),
        user: Some(t.user.clone()),
        password: Some(SecretString::from(t.password.clone())),
        database: Some(t.database.clone()),
        application_name: "Hacivat (metadata)".into(),
        read_only,
        options: vec![],
        connect_timeout: secs(20),
        events: Arc::new(|_| {}),
        insecure_auth: InsecureAuth::Allow,
    }
}

async fn connect(t: &DockerTarget, read_only: bool) -> MySqlConn {
    match connect_conn(request(t, read_only)).await {
        Ok(c) => c,
        Err(e) => panic!("{}: {:?}", t.id, e.kind),
    }
}

/// Runs `sql` on the writer; panics with the server code on failure.
async fn ddl(t: &DockerTarget, w: &mut MySqlConn, sql: &str) {
    if let Err(e) = w.batch(sql).await {
        panic!(
            "{}: {sql}: {:?} {:?}",
            t.id,
            e.kind,
            e.server.as_ref().map(|s| (s.vendor_code, s.text.clone()))
        );
    }
}

/// Runs `sql` on the writer; `Err((code, text))` instead of a panic.
async fn try_ddl(w: &mut MySqlConn, sql: &str) -> Result<(), (Option<i64>, String)> {
    w.batch(sql).await.map_err(|e| {
        (
            e.server.as_ref().and_then(|s| s.vendor_code),
            e.server.map(|s| s.text).unwrap_or_default(),
        )
    })
}

/// Drops every `hvi4_…` object of `hv_test` (left over by an interrupted run).
async fn cleanup(t: &DockerTarget, w: &mut MySqlConn, prefix: &str) {
    let like = format!("{}%", prefix.replace('_', "\\_"));
    let mut c = TypedCatalog::new(w);
    let q = |sql: &'static str| sql;
    let trig = c
        .rows(
            q("SELECT TRIGGER_NAME FROM INFORMATION_SCHEMA.TRIGGERS WHERE TRIGGER_SCHEMA = ? AND TRIGGER_NAME LIKE ?"),
            &[SCHEMA, &like],
        )
        .await
        .unwrap();
    let ev = c
        .rows(
            q("SELECT EVENT_NAME FROM INFORMATION_SCHEMA.EVENTS WHERE EVENT_SCHEMA = ? AND EVENT_NAME LIKE ?"),
            &[SCHEMA, &like],
        )
        .await
        .unwrap();
    let rt = c
        .rows(
            q("SELECT ROUTINE_NAME, ROUTINE_TYPE FROM INFORMATION_SCHEMA.ROUTINES WHERE ROUTINE_SCHEMA = ? AND ROUTINE_NAME LIKE ?"),
            &[SCHEMA, &like],
        )
        .await
        .unwrap();
    let rel = c
        .rows(
            q("SELECT TABLE_NAME, TABLE_TYPE FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA = ? AND TABLE_NAME LIKE ?"),
            &[SCHEMA, &like],
        )
        .await
        .unwrap();
    drop(c);
    let mut stmts: Vec<String> = Vec::new();
    stmts.extend(
        trig.iter()
            .map(|r| format!("DROP TRIGGER IF EXISTS `{}`", r.str(0))),
    );
    stmts.extend(
        ev.iter()
            .map(|r| format!("DROP EVENT IF EXISTS `{}`", r.str(0))),
    );
    stmts.extend(
        rt.iter()
            .map(|r| format!("DROP {} IF EXISTS `{}`", r.str(1), r.str(0))),
    );
    stmts.extend(
        rel.iter()
            .filter(|r| r.str(1) == "VIEW")
            .map(|r| format!("DROP VIEW IF EXISTS `{}`", r.str(0))),
    );
    let tables: Vec<String> = rel
        .iter()
        .filter(|r| r.str(1) != "VIEW")
        .map(|r| format!("`{}`", r.str(0)))
        .collect();
    for s in stmts {
        ddl(t, w, &s).await;
    }
    ddl(t, w, "SET FOREIGN_KEY_CHECKS = 0").await;
    for chunk in tables.chunks(200) {
        ddl(t, w, &format!("DROP TABLE IF EXISTS {}", chunk.join(", "))).await;
    }
    ddl(t, w, "SET FOREIGN_KEY_CHECKS = 1").await;
}

/// `a == b`, failing with the first differing object (snapshots are large).
fn assert_same(a: &SchemaSnapshot, b: &SchemaSnapshot, what: &str) {
    if a == b {
        return;
    }
    let an: Vec<&str> = a.relations.iter().map(|r| r.name.as_str()).collect();
    let bn: Vec<&str> = b.relations.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(an, bn, "{what}: relation names");
    for (x, y) in a.relations.iter().zip(&b.relations) {
        assert_eq!(x, y, "{what}: relation {}", x.name);
    }
    assert_eq!(a.routines, b.routines, "{what}: routines");
    assert_eq!(a.triggers, b.triggers, "{what}: triggers");
    assert_eq!(a.events, b.events, "{what}: events");
    assert_eq!(a, b, "{what}");
}

fn supports_desc(ctx: &ServerCtx) -> bool {
    ctx.flavor == Flavor::MySql || ctx.at_least((10, 8, 0))
}

fn fixture(ctx: &ServerCtx) -> Vec<String> {
    let my = ctx.flavor == Flavor::MySql;
    let mut v = vec![
        format!(
            "CREATE TABLE {P}parent (id INT PRIMARY KEY AUTO_INCREMENT, \
             code VARCHAR(20) NOT NULL COMMENT 'the code', region VARCHAR(10) DEFAULT 'eu', \
             touched TIMESTAMP NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP, \
             UNIQUE KEY {P}uq_code (code)) COMMENT='parents'"
        ),
        format!(
            "CREATE TABLE {P}child (a INT NOT NULL, b VARCHAR(10) NOT NULL, parent_id INT, \
             total INT AS (a * 2) VIRTUAL, label VARCHAR(20) AS (UPPER(b)) STORED, \
             secret INT INVISIBLE, PRIMARY KEY (b, a), KEY {P}ix_desc (parent_id DESC, b(4)), \
             CONSTRAINT {P}fk_parent FOREIGN KEY (parent_id) REFERENCES {P}parent (id) \
             ON DELETE CASCADE, CONSTRAINT {P}chk_a CHECK (a > 0))"
        ),
        format!(
            "CREATE VIEW {P}v_child AS SELECT c.a, c.b, c.total, p.code \
             FROM {P}child c JOIN {P}parent p ON p.id = c.parent_id"
        ),
        format!(
            "CREATE FUNCTION {P}f_double(x INT) RETURNS INT DETERMINISTIC NO SQL \
             COMMENT 'doubles' RETURN x * 2"
        ),
        format!("CREATE PROCEDURE {P}p_touch(IN n INT, OUT m INT) NO SQL BEGIN SET m = n + 1; END"),
        format!(
            "CREATE TRIGGER {P}trg_child BEFORE INSERT ON {P}child FOR EACH ROW SET NEW.a = NEW.a"
        ),
        format!(
            "CREATE EVENT {P}ev_tick ON SCHEDULE EVERY 1 DAY STARTS '2030-01-01 00:00:00' \
             DISABLE COMMENT 'ticks' DO SELECT 1"
        ),
    ];
    if my && ctx.at_least((8, 0, 13)) {
        v.push(format!(
            "CREATE INDEX {P}ix_fn ON {P}parent ((LOWER(code)))"
        ));
    }
    if ctx.flavor == Flavor::MariaDb {
        v.push(format!("CREATE SEQUENCE {P}seq START WITH 10"));
    }
    v
}

async fn ctx_of(t: &DockerTarget, r: &mut MySqlConn) -> ServerCtx {
    let mut c = TypedCatalog::new(r);
    let ctx = mysql::server_ctx(&mut c).await.unwrap();
    println!(
        "SERVER {}: {} flavor={:?} lctn={} uuid={}",
        t.id,
        ctx.version_text,
        ctx.flavor,
        ctx.lower_case_table_names,
        ctx.server_uuid.is_some()
    );
    ctx
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "docker"]
async fn full_introspection_reads_every_shape() {
    let _serial = SERIAL.lock().await;
    if !docker_enabled() {
        return;
    }
    for t in mysql_family() {
        let mut w = connect(&t, false).await;
        let mut r = connect(&t, true).await;
        let ctx = ctx_of(&t, &mut r).await;
        cleanup(&t, &mut w, P).await;
        let mut created: Vec<String> = Vec::new();
        for sql in fixture(&ctx) {
            match try_ddl(&mut w, &sql).await {
                Ok(()) => created.push(sql),
                Err((code, text)) => println!("SKIP {}: {code:?} {text}: {sql}", t.id),
            }
        }
        let mut c = TypedCatalog::new(&mut r);
        let ns = mysql::list_namespaces(&mut c).await.unwrap();
        assert!(
            ns.iter().any(|n| n.name == SCHEMA && !n.system),
            "{}: {ns:?}",
            t.id
        );
        assert!(
            ns.iter()
                .any(|n| n.name == "information_schema" && n.system)
        );

        let started = Instant::now();
        let (s, w0) = mysql::introspect(&mut c, &ctx, SCHEMA, Level::Sources)
            .await
            .unwrap_or_else(|e| panic!("{}: {e:?}", t.id));
        println!(
            "FULL {}: {} relations, {} columns, {} queries, {:?}",
            t.id,
            s.relations.len(),
            s.column_count(),
            c.log.len(),
            started.elapsed()
        );
        assert!(!w0.at_utc.is_empty());
        let id = &t.id;

        let parent = s.relation(&format!("{P}parent")).unwrap();
        assert_eq!(parent.kind, Some(RelationKind::Table));
        assert_eq!(parent.engine.as_deref(), Some("InnoDB"));
        assert_eq!(parent.comment.as_deref(), Some("parents"), "{id}");
        assert!(parent.marker.is_some(), "{id}: CREATE_TIME marker");
        let pid = parent.column("id").unwrap();
        assert!(
            pid.auto_increment && !pid.nullable && pid.ordinal == 1,
            "{id}: {pid:?}"
        );
        assert_eq!(
            parent.column("code").unwrap().comment.as_deref(),
            Some("the code")
        );
        // Literal defaults: MySQL unquoted, MariaDB quoted (verbatim, never re-quoted).
        let region = parent.column("region").unwrap().default.clone().unwrap();
        println!("DEFAULT {id}: region default text = {region}");
        match ctx.flavor {
            Flavor::MySql => assert_eq!(region, "eu", "{id}"),
            Flavor::MariaDb => assert_eq!(region, "'eu'", "{id}"),
        }
        let touched = parent.column("touched").unwrap();
        assert!(touched.on_update.is_some(), "{id}: {touched:?}");
        println!(
            "DEFAULT {id}: touched default = {:?}, on_update = {:?}",
            touched.default, touched.on_update
        );
        // PK, UK (named after its index).
        let pk = parent.constraint("PRIMARY").unwrap();
        assert_eq!(
            (&pk.kind, pk.columns.as_slice()),
            (&ConstraintKind::PrimaryKey, &["id".to_owned()][..])
        );
        let uq = parent.constraint(&format!("{P}uq_code")).unwrap();
        assert_eq!(uq.kind, ConstraintKind::Unique);
        assert!(parent.index("PRIMARY").unwrap().primary);
        if ctx.flavor == Flavor::MySql {
            let f = parent.index(&format!("{P}ix_fn")).unwrap();
            assert_eq!(f.columns.len(), 1);
            assert_eq!(
                f.columns[0].name, None,
                "{id}: functional key part has no column"
            );
            let e = f.columns[0].expression.as_deref().unwrap();
            assert!(e.to_ascii_lowercase().contains("lower"), "{id}: {e}");
        }

        let child = s.relation(&format!("{P}child")).unwrap();
        let cols: Vec<&str> = child.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            cols,
            ["a", "b", "parent_id", "total", "label", "secret"],
            "{id}"
        );
        let total = child.column("total").unwrap();
        assert_eq!(
            total.generated.as_ref().map(|g| g.0),
            Some(GeneratedKind::Virtual),
            "{id}"
        );
        assert!(
            total
                .generated
                .as_ref()
                .unwrap()
                .1
                .as_deref()
                .unwrap()
                .contains('2'),
            "{id}: {total:?}"
        );
        let label = child.column("label").unwrap();
        assert_eq!(
            label.generated.as_ref().map(|g| g.0),
            Some(GeneratedKind::Stored),
            "{id}"
        );
        assert!(child.column("secret").unwrap().hidden, "{id}: INVISIBLE");
        assert_eq!(
            child.constraint("PRIMARY").unwrap().columns,
            ["b", "a"],
            "{id}"
        );
        let fk = child.constraint(&format!("{P}fk_parent")).unwrap();
        let ConstraintKind::ForeignKey(f) = &fk.kind else {
            panic!("{id}: {fk:?}")
        };
        assert_eq!(f.ref_table, format!("{P}parent"));
        assert_eq!(f.ref_schema.as_deref(), Some(SCHEMA));
        assert_eq!(f.ref_columns, [Some("id".to_owned())]);
        assert_eq!(f.on_delete.as_deref(), Some("CASCADE"));
        println!(
            "FKDEFAULT {id}: unspecified ON UPDATE reads {:?}",
            f.on_update
        );
        assert!(
            matches!(f.on_update.as_deref(), Some("RESTRICT" | "NO ACTION")),
            "{id}"
        );
        assert_eq!(fk.columns, ["parent_id"]);
        let chk = child.constraint(&format!("{P}chk_a")).unwrap();
        let ConstraintKind::Check { clause } = &chk.kind else {
            panic!("{id}")
        };
        println!("CHECK {id}: {clause}");
        assert!(
            clause.contains('a') && clause.contains('0'),
            "{id}: {clause}"
        );
        let ix = child.index(&format!("{P}ix_desc")).unwrap();
        assert_eq!(ix.columns.len(), 2);
        assert_eq!(ix.columns[1].sub_part, Some(4), "{id}: prefix length");
        assert_eq!(
            ix.columns[0].descending,
            supports_desc(&ctx),
            "{id}: DESC key part"
        );

        let v = s.relation(&format!("{P}v_child")).unwrap();
        assert_eq!(v.kind, Some(RelationKind::View));
        assert!(
            v.marker.as_deref().is_some_and(|h| h.len() == 64),
            "{id}: view hash"
        );
        let vd = v.view.as_ref().unwrap();
        assert!(vd.definition.as_deref().unwrap().contains("total"), "{id}");
        let vcols: Vec<&str> = v.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(vcols, ["a", "b", "total", "code"], "{id}");

        let rt_ok = created.iter().any(|s| s.contains("FUNCTION"));
        if rt_ok {
            let f = s.routine(&format!("{P}f_double")).unwrap();
            assert_eq!(f.kind, Some(RoutineKind::Function));
            // MySQL `int`, MariaDB `int(11)` (display width kept).
            assert!(
                matches!(f.returns.as_deref(), Some("int" | "int(11)")),
                "{id}: {f:?}"
            );
            assert_eq!(f.deterministic, Some(true));
            assert_eq!(f.comment.as_deref(), Some("doubles"));
            assert_eq!(f.params.len(), 1);
            assert_eq!(f.params[0].name.as_deref(), Some("x"));
            assert!(f.definition.as_deref().unwrap().contains("x * 2"), "{id}");
            assert!(f.marker.is_some());
        }
        if created.iter().any(|s| s.contains("PROCEDURE")) {
            let p = s.routine(&format!("{P}p_touch")).unwrap();
            assert_eq!(p.kind, Some(RoutineKind::Procedure));
            let modes: Vec<Option<&str>> = p.params.iter().map(|p| p.mode.as_deref()).collect();
            assert_eq!(modes, [Some("IN"), Some("OUT")], "{id}");
        }
        if created.iter().any(|s| s.contains("TRIGGER")) {
            let tg = s.trigger(&format!("{P}trg_child")).unwrap();
            assert_eq!(tg.table, format!("{P}child"));
            assert_eq!(
                (tg.timing.as_deref(), tg.event.as_deref()),
                (Some("BEFORE"), Some("INSERT"))
            );
            assert!(tg.body.is_some() && tg.marker.is_some());
        }
        if created.iter().any(|s| s.contains("EVENT")) {
            let ev = s.event(&format!("{P}ev_tick")).unwrap();
            assert_eq!(ev.event_type.as_deref(), Some("RECURRING"));
            assert_eq!(ev.status.as_deref(), Some("DISABLED"));
            assert_eq!(ev.comment.as_deref(), Some("ticks"));
            assert!(ev.body.as_deref().unwrap().contains("SELECT 1"));
        }
        if ctx.flavor == Flavor::MariaDb && created.iter().any(|s| s.contains("SEQUENCE")) {
            let sq = s.relation(&format!("{P}seq")).unwrap();
            assert_eq!(sq.kind, Some(RelationKind::Sequence), "{id}");
        }

        // `Structure` = `Sources` minus bodies; `Names` = relations without columns.
        let (st, _) = mysql::introspect(&mut c, &ctx, SCHEMA, Level::Structure)
            .await
            .unwrap();
        assert!(
            st.relation(&format!("{P}v_child"))
                .unwrap()
                .view
                .as_ref()
                .unwrap()
                .definition
                .is_none()
        );
        assert_eq!(st.column_count(), s.column_count());
        let (nm, _) = mysql::introspect(&mut c, &ctx, SCHEMA, Level::Names)
            .await
            .unwrap();
        assert_eq!(nm.relations.len(), s.relations.len());
        assert_eq!(nm.column_count(), 0);

        // HV-INTRO-14: names are bound exactly; I_S compares binary on MySQL (lctn=0), so the
        // case variant is not found. MariaDB's I_S collation is case-insensitive: printed.
        let upper = format!("{P}PARENT");
        let state = mysql::check_tables(&mut c, SCHEMA, &[format!("{P}parent"), upper.clone()])
            .await
            .unwrap();
        assert!(state[&format!("{P}parent")].is_some());
        println!("CASE {id}: check_state({upper}) = {:?}", state[&upper]);
        if ctx.flavor == Flavor::MySql {
            assert_eq!(state[&upper], None, "{id}");
        }
        drop(c);
        cleanup(&t, &mut w, P).await;
        Box::new(w).close().await;
        Box::new(r).close().await;
    }
}

/// One DDL step of the ver/01 `#mysql-is-times` matrix, run on `{P}tt`.
fn steps() -> Vec<(&'static str, String)> {
    let tt = format!("{P}tt");
    vec![
        ("insert", format!("INSERT INTO {tt} (id, a) VALUES (1, 1)")),
        ("add column", format!("ALTER TABLE {tt} ADD COLUMN b INT")),
        ("add index", format!("ALTER TABLE {tt} ADD INDEX {P}ia (a)")),
        ("table comment", format!("ALTER TABLE {tt} COMMENT = 'c1'")),
        (
            "modify type (copy)",
            format!("ALTER TABLE {tt} MODIFY a BIGINT"),
        ),
        (
            "rename column",
            format!("ALTER TABLE {tt} RENAME COLUMN b TO b2"),
        ),
        (
            "set default",
            format!("ALTER TABLE {tt} ALTER COLUMN b2 SET DEFAULT 7"),
        ),
        (
            "column comment",
            format!("ALTER TABLE {tt} MODIFY b2 INT DEFAULT 7 COMMENT 'cc'"),
        ),
        (
            "rename index",
            format!("ALTER TABLE {tt} RENAME INDEX {P}ia TO {P}ib"),
        ),
        (
            "add check",
            format!("ALTER TABLE {tt} ADD CONSTRAINT {P}chk CHECK (a >= 0)"),
        ),
        (
            "add fk",
            format!(
                "ALTER TABLE {tt} ADD CONSTRAINT {P}fk FOREIGN KEY (b2) REFERENCES {P}ref (id)"
            ),
        ),
        (
            "drop fk",
            format!("ALTER TABLE {tt} DROP FOREIGN KEY {P}fk"),
        ),
        (
            "create trigger",
            format!("CREATE TRIGGER {P}trg BEFORE INSERT ON {tt} FOR EACH ROW SET NEW.a = NEW.a"),
        ),
        ("drop trigger", format!("DROP TRIGGER {P}trg")),
        (
            "create trigger 2",
            format!("CREATE TRIGGER {P}trg2 BEFORE UPDATE ON {tt} FOR EACH ROW SET NEW.a = NEW.a"),
        ),
        ("rename table", format!("RENAME TABLE {tt} TO {tt}_r")),
        ("rename back", format!("RENAME TABLE {tt}_r TO {tt}")),
        (
            "replace view",
            format!("CREATE OR REPLACE VIEW {P}vv AS SELECT id, a, 2 AS two FROM {tt}"),
        ),
        (
            "base type under view",
            format!("ALTER TABLE {tt} MODIFY a DECIMAL(10,2)"),
        ),
        (
            "create function",
            format!("CREATE FUNCTION {P}fn() RETURNS INT DETERMINISTIC NO SQL RETURN 1"),
        ),
        (
            "alter function",
            format!("ALTER FUNCTION {P}fn COMMENT 'changed'"),
        ),
        (
            "recreate function",
            format!(
                "DROP FUNCTION {P}fn; CREATE FUNCTION {P}fn() RETURNS INT DETERMINISTIC NO SQL RETURN 2"
            ),
        ),
        (
            "create event",
            format!(
                "CREATE EVENT {P}ev ON SCHEDULE EVERY 1 HOUR STARTS '2030-01-01 00:00:00' DISABLE DO SELECT 1"
            ),
        ),
        ("alter event", format!("ALTER EVENT {P}ev COMMENT 'e2'")),
        ("truncate", format!("TRUNCATE TABLE {tt}")),
        ("optimize", format!("OPTIMIZE TABLE {tt}")),
        (
            "add partitioned",
            format!("CREATE TABLE {P}pt (id INT PRIMARY KEY) PARTITION BY HASH (id) PARTITIONS 2"),
        ),
        (
            "add partition",
            format!("ALTER TABLE {P}pt ADD PARTITION PARTITIONS 1"),
        ),
        ("drop table", format!("DROP TABLE {P}pt")),
    ]
}

async fn create_time(c: &mut dyn CatalogConn, table: &str) -> Option<String> {
    c.rows(
        "SELECT CAST(CREATE_TIME AS CHAR) FROM INFORMATION_SCHEMA.TABLES \
         WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?",
        &[SCHEMA, table],
    )
    .await
    .unwrap()
    .first()
    .and_then(|r| r.text(0))
}

/// The ver/01 matrix as incremental refresh: on MySQL each step's incremental pass merged onto
/// the previous snapshot equals a full pass, with no pause between DDL and pass (the same
/// second as the watermark). On every server the step's effect on `CREATE_TIME` is recorded
/// with a 1.1 s pause so a change is visible at 1-second granularity.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "docker"]
async fn incremental_matrix_matches_full() {
    let _serial = SERIAL.lock().await;
    if !docker_enabled() {
        return;
    }
    for t in mysql_family() {
        let mut w = connect(&t, false).await;
        let mut r = connect(&t, true).await;
        let ctx = ctx_of(&t, &mut r).await;
        let id = t.id.clone();
        cleanup(&t, &mut w, P).await;
        ddl(
            &t,
            &mut w,
            &format!("CREATE TABLE {P}ref (id INT PRIMARY KEY)"),
        )
        .await;
        ddl(
            &t,
            &mut w,
            &format!("CREATE TABLE {P}tt (id INT PRIMARY KEY, a INT)"),
        )
        .await;
        ddl(
            &t,
            &mut w,
            &format!("CREATE VIEW {P}vv AS SELECT id, a FROM {P}tt"),
        )
        .await;
        // Past the 1-second overlap, so the baseline's watermark does not cover the fixture.
        tokio::time::sleep(Duration::from_millis(1_100)).await;
        let mut c = TypedCatalog::new(&mut r);
        let (mut snap, mut wm) = mysql::introspect(&mut c, &ctx, SCHEMA, Level::Sources)
            .await
            .unwrap();
        let tt = format!("{P}tt");
        for (step, sql) in steps() {
            let before = create_time(&mut c, &tt).await;
            if let Err((code, text)) = try_ddl(&mut w, &sql).await {
                println!("SKIP {id} {step}: {code:?} {text}");
                continue;
            }
            // Incremental right away (same second as the DDL and, often, as `wm`).
            let log0 = c.log.len();
            let (next, next_wm, stats) =
                mysql::incremental(&mut c, &ctx, &snap, &wm, Level::Sources)
                    .await
                    .unwrap_or_else(|e| panic!("{id} {step}: {e:?}"));
            let inc_queries = c.log.len() - log0;
            let (full, _) = mysql::introspect(&mut c, &ctx, SCHEMA, Level::Sources)
                .await
                .unwrap();
            assert_same(&next, &full, &format!("{id} {step}"));
            if ctx.incremental_supported() {
                assert_eq!(stats.full, None, "{id} {step}");
                if step == "insert" {
                    assert!(
                        stats.relations_reread.is_empty(),
                        "{id}: an INSERT re-reads no relation: {stats:?}"
                    );
                }
            }
            println!(
                "INC {id} {step}: full={:?} rel={:?} rt={:?} tg={:?} ev={:?} queries={inc_queries}",
                stats.full,
                stats.relations_reread,
                stats.routines_reread,
                stats.triggers_reread,
                stats.events_reread
            );
            snap = next;
            wm = next_wm;
            // The CREATE_TIME evidence: wait past the second, then compare.
            tokio::time::sleep(Duration::from_millis(1_100)).await;
            if step != "insert" && !step.starts_with("rename") {
                // A second probe DDL-free read after the pause.
            }
            let after = create_time(&mut c, &tt).await;
            println!(
                "CTIME {id} {step}: {} ({before:?} -> {after:?})",
                if before == after {
                    "unchanged"
                } else {
                    "changed"
                }
            );
        }
        drop(c);
        cleanup(&t, &mut w, P).await;
        Box::new(w).close().await;
        Box::new(r).close().await;
    }
}

/// A long `ALGORITHM=COPY` ALTER that straddles an incremental pass: the next pass still sees
/// it (marker equality, not only `>= W − 1 s`). Records whether `CREATE_TIME` is the ALTER's
/// start or end time.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "docker"]
async fn long_alter_straddling_a_pass_is_seen() {
    let _serial = SERIAL.lock().await;
    if !docker_enabled() {
        return;
    }
    for t in mysql_family() {
        let mut w = connect(&t, false).await;
        let mut r = connect(&t, true).await;
        let ctx = ctx_of(&t, &mut r).await;
        let id = t.id.clone();
        cleanup(&t, &mut w, P).await;
        let big = format!("{P}big");
        ddl(
            &t,
            &mut w,
            &format!("CREATE TABLE {big} (id INT PRIMARY KEY, pad CHAR(200) NOT NULL DEFAULT 'x')"),
        )
        .await;
        ddl(
            &t,
            &mut w,
            &format!(
                "INSERT INTO {big} (id) WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < 1000) \
                 SELECT a.n * 1000 + b.n FROM s a JOIN s b WHERE a.n <= 400"
            ),
        )
        .await;
        let mut c = TypedCatalog::new(&mut r);
        let (snap, wm) = mysql::introspect(&mut c, &ctx, SCHEMA, Level::Structure)
            .await
            .unwrap();
        let ct0 = create_time(&mut c, &big).await;
        tokio::time::sleep(Duration::from_millis(1_100)).await;
        let mut wc = TypedCatalog::new(&mut w);
        let start = wc
            .rows("SELECT CAST(UTC_TIMESTAMP(6) AS CHAR)", &[])
            .await
            .unwrap()[0]
            .str(0)
            .to_owned();
        drop(wc);
        let alter = format!("ALTER TABLE {big} ADD COLUMN extra INT, ALGORITHM=COPY");
        let handle = tokio::spawn(async move {
            let r = w.batch(&alter).await;
            (w, r)
        });
        // A pass while the ALTER runs.
        tokio::time::sleep(Duration::from_millis(1_500)).await;
        let (mid, mid_wm, mid_stats) =
            mysql::incremental(&mut c, &ctx, &snap, &wm, Level::Structure)
                .await
                .unwrap();
        let (mut w, res) = handle.await.unwrap();
        res.unwrap_or_else(|e| panic!("{id}: {e:?}"));
        let mut wc = TypedCatalog::new(&mut w);
        let end = wc
            .rows("SELECT CAST(UTC_TIMESTAMP(6) AS CHAR)", &[])
            .await
            .unwrap()[0]
            .str(0)
            .to_owned();
        drop(wc);
        let ct1 = create_time(&mut c, &big).await;
        // Would the alg/02 window alone (`CREATE_TIME >= W − 1 s`, W = the mid pass) catch it?
        let window = c
            .rows(
                "SELECT CAST(? AS DATETIME(6)) >= CAST(? AS DATETIME(6)) - INTERVAL 1 SECOND",
                &[ct1.as_deref().unwrap_or(""), &mid_wm.at_utc],
            )
            .await
            .unwrap()[0]
            .flag(0);
        println!(
            "LONGALTER {id}: start={start} end={end} create_time {ct0:?} -> {ct1:?}; mid-pass reread {:?} at {}; window-only catches it: {window}",
            mid_stats.relations_reread, mid_wm.at_utc
        );
        // Give the pass past the overlap window: only marker equality can catch it now.
        tokio::time::sleep(Duration::from_millis(2_100)).await;
        let (after, _, stats) = mysql::incremental(&mut c, &ctx, &mid, &mid_wm, Level::Structure)
            .await
            .unwrap();
        let (full, _) = mysql::introspect(&mut c, &ctx, SCHEMA, Level::Structure)
            .await
            .unwrap();
        assert_same(&after, &full, &format!("{id} long alter"));
        assert!(
            after.relation(&big).unwrap().column("extra").is_some(),
            "{id}: {stats:?}"
        );
        drop(c);
        cleanup(&t, &mut w, P).await;
        Box::new(w).close().await;
        Box::new(r).close().await;
    }
}

/// The scale measurement: 10,000 tables × 16 columns (160,000) plus one index and one FK each,
/// on mysql84. `HACIVAT_INTRO_PERF=1` to run (it takes minutes to build).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "docker"]
async fn perf_10k_tables_160k_columns() {
    let _serial = SERIAL.lock().await;
    if !docker_enabled() || std::env::var("HACIVAT_INTRO_PERF").ok().as_deref() != Some("1") {
        return;
    }
    let only = std::env::var("HACIVAT_INTRO_PERF_TARGET").unwrap_or_else(|_| "mysql84".into());
    let Some(t) = mysql_family().into_iter().find(|t| t.id == only) else {
        return;
    };
    let pp = format!("{P}perf_");
    let mut w = connect(&t, false).await;
    let mut r = connect(&t, true).await;
    let ctx = ctx_of(&t, &mut r).await;
    let t0 = Instant::now();
    cleanup(&t, &mut w, &pp).await;
    println!("PERF {}: cleanup {:?}", t.id, t0.elapsed());
    let t0 = Instant::now();
    const N: usize = 10_000;
    let mut batch = Vec::new();
    for i in 0..N {
        let cols: Vec<String> = (0..16)
            .map(|c| match c {
                0 => "id INT NOT NULL PRIMARY KEY".to_owned(),
                1 => "ref_id INT NULL".to_owned(),
                _ => format!("c{c} VARCHAR(32) NOT NULL DEFAULT 'x' COMMENT 'col {c}'"),
            })
            .collect();
        let fk = if i > 0 {
            format!(
                ", KEY ix_ref (ref_id), CONSTRAINT {pp}fk{i:05} FOREIGN KEY (ref_id) REFERENCES {pp}{:05} (id)",
                i - 1
            )
        } else {
            String::new()
        };
        batch.push(format!(
            "CREATE TABLE {pp}{i:05} ({}, KEY ix_c2 (c2, c3){fk})",
            cols.join(", ")
        ));
        if batch.len() == 50 {
            ddl(&t, &mut w, &batch.join("; ")).await;
            batch.clear();
        }
        if i % 1000 == 999 {
            println!("PERF {}: {} tables in {:?}", t.id, i + 1, t0.elapsed());
        }
    }
    if !batch.is_empty() {
        ddl(&t, &mut w, &batch.join("; ")).await;
    }
    println!("PERF {}: built {N} tables in {:?}", t.id, t0.elapsed());

    let mut c = TypedCatalog::new(&mut r);
    let mut snap = None;
    for level in [Level::Names, Level::Structure, Level::Sources] {
        c.rows_read = 0;
        let q0 = c.log.len();
        let t1 = Instant::now();
        let (s, wm) = mysql::introspect(&mut c, &ctx, SCHEMA, level)
            .await
            .unwrap();
        let took = t1.elapsed();
        let mine = s
            .relations
            .iter()
            .filter(|r| r.name.starts_with(&pp))
            .count();
        assert_eq!(mine, N);
        if level >= Level::Structure {
            let cols: usize = s
                .relations
                .iter()
                .filter(|r| r.name.starts_with(&pp))
                .map(|r| r.columns.len())
                .sum();
            assert_eq!(cols, 160_000);
        }
        println!(
            "PERF {} {level:?}: {} relations, {} columns, {} rows, {} queries, {:?}",
            t.id,
            s.relations.len(),
            s.column_count(),
            c.rows_read,
            c.log.len() - q0,
            took
        );
        if level == Level::Structure {
            snap = Some((s, wm));
        }
    }
    let (s, wm) = snap.unwrap();
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let t1 = Instant::now();
    let (s2, wm2, st) = mysql::incremental(&mut c, &ctx, &s, &wm, Level::Structure)
        .await
        .unwrap();
    println!(
        "PERF {} incremental (no change): {:?}, reread {} relations",
        t.id,
        t1.elapsed(),
        st.relations_reread.len()
    );
    drop(c);
    ddl(
        &t,
        &mut w,
        &format!("ALTER TABLE {pp}05000 ADD COLUMN extra INT"),
    )
    .await;
    let mut c = TypedCatalog::new(&mut r);
    let t1 = Instant::now();
    let (s3, _, st) = mysql::incremental(&mut c, &ctx, &s2, &wm2, Level::Structure)
        .await
        .unwrap();
    println!(
        "PERF {} incremental (1 ALTER): {:?}, reread {:?}",
        t.id,
        t1.elapsed(),
        st.relations_reread
    );
    assert!(
        s3.relation(&format!("{pp}05000"))
            .unwrap()
            .column("extra")
            .is_some()
    );
    drop(c);
    let t1 = Instant::now();
    cleanup(&t, &mut w, &pp).await;
    println!("PERF {}: dropped in {:?}", t.id, t1.elapsed());
    Box::new(w).close().await;
    Box::new(r).close().await;
}

/// Two facts the M4 plan leans on (printed, `PROBE` lines): whether the test login may create
/// a database (plan Task 17 wants one per test), and whether `INFORMATION_SCHEMA` reads inside
/// `START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY` (plan Task 5 `begin_snapshot`) see
/// DDL committed after the snapshot began (whether the snapshot covers catalog reads).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "docker"]
async fn snapshot_and_privilege_probes() {
    let _serial = SERIAL.lock().await;
    if !docker_enabled() {
        return;
    }
    for t in mysql_family() {
        let mut w = connect(&t, false).await;
        let mut r = connect(&t, true).await;
        let _ctx = ctx_of(&t, &mut r).await;
        let id = t.id.clone();
        cleanup(&t, &mut w, P).await;
        let db = format!("{P}probe_db");
        match try_ddl(&mut w, &format!("CREATE DATABASE {db}")).await {
            Ok(()) => {
                println!("PROBE {id} create database: allowed");
                ddl(&t, &mut w, &format!("DROP DATABASE {db}")).await;
            }
            Err((code, text)) => println!("PROBE {id} create database: refused {code:?} {text}"),
        }
        let snap = format!("{P}snap");
        ddl(&t, &mut w, &format!("CREATE TABLE {snap} (a INT)")).await;
        let count = "SELECT COUNT(*) FROM INFORMATION_SCHEMA.COLUMNS \
                     WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?";
        let mut c = TypedCatalog::new(&mut r);
        let txn = c
            .control("START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY")
            .await;
        println!(
            "PROBE {id} begin snapshot: {:?}",
            txn.as_ref().err().map(|e| e.vendor_code())
        );
        let before = c.rows(count, &[SCHEMA, &snap]).await.unwrap()[0].u32(0);
        let ct_before = create_time(&mut c, &snap).await;
        drop(c);
        tokio::time::sleep(Duration::from_millis(1_100)).await;
        ddl(&t, &mut w, &format!("ALTER TABLE {snap} ADD COLUMN b INT")).await;
        ddl(&t, &mut w, &format!("CREATE TABLE {snap}2 (a INT)")).await;
        let mut c = TypedCatalog::new(&mut r);
        let after = c.rows(count, &[SCHEMA, &snap]).await.unwrap()[0].u32(0);
        let ct_after = create_time(&mut c, &snap).await;
        let new_seen = create_time(&mut c, &format!("{snap}2")).await.is_some();
        let ro = c
            .control(&format!("CREATE TABLE {snap}3 (a INT)"))
            .await
            .err()
            .and_then(|e| e.vendor_code());
        let _ = c.control("COMMIT").await;
        println!(
            "PROBE {id} I_S inside consistent snapshot: columns {before:?} -> {after:?} ({}), \
             create_time {}, new table {}, write in txn refused with {ro:?}",
            if before == after {
                "frozen"
            } else {
                "sees later DDL"
            },
            if ct_before == ct_after {
                "frozen"
            } else {
                "sees later DDL"
            },
            if new_seen { "visible" } else { "invisible" }
        );
        drop(c);
        cleanup(&t, &mut w, P).await;
        Box::new(w).close().await;
        Box::new(r).close().await;
    }
}
