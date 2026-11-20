// ## m4-t31-golden-guard
//! Gate G5's apply round trip (M4 Task 31; alg/03 HV-DDL-07 tests 10 and 11): a scratch
//! schema (PostgreSQL), database (MySQL, MariaDB) or file (SQLite) is seeded, introspected and
//! each object's CREATE script generated (the first render, this run's golden); the scratch is
//! emptied and those scripts applied in order; a second introspection renders every object
//! again. The two renders must be the same bytes, and the two models equal (keys, flags and
//! props, with PostgreSQL oids masked). A difference is a finding for Task 19, never blessed.
//!
//! Round trips run per object, in the seed's creation order: whole-schema scripts are ordered
//! by kind and then name, not by dependency (T19 carry; the dependency order is M7's).
//! PostgreSQL views, routines and triggers are not in the PostgreSQL seed: M4 keeps no
//! PostgreSQL source text (its `Sources` facet is not implemented), so their script is
//! `NotLoaded` by design; the MySQL-family and SQLite seeds cover server-text kinds.
//!
//! `pg_rendered_scripts_parse` is the PostgreSQL parse check of every PostgreSQL golden file
//! of `hacivat-ddl` (byte-equal to its render, `golden_stability.rs`): the server parses a whole simple-query string before it runs any of it, so the first error of
//! a golden sent as one string is a syntax error (class 42601) only when some statement of it
//! does not parse; a missing object is a later, semantic error. (libpg_query through
//! `sql-oracle-pg` would need a dev-dependency this task does not own.) The round trips'
//! rendered scripts are accepted by every server outright.
//!
//! Run: `cargo test -p hacivat-introspect --no-run`, then
//! `<main>/target/docker/run-docker-tests.sh --set full -p hacivat-introspect --test
//! docker_ddl_roundtrip` (PR set: pg17, mysql84). Every Docker test is `#[ignore = "docker"]`,
//! returns early unless `HACIVAT_DOCKER=1`, takes its targets only from `require_target`, and
//! creates its own `hvm4_t31_*` schema or database, dropped by a guard, also on panic.
//! `sqlite_roundtrip` runs in-process on temp files.

#![cfg(feature = "session")]

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use common::g4::fact;
use common::my::{self, Login};
use common::pg::{DropSchemas, Pg, admin, unique};
use hacivat_ddl::produce::{DdlSources, create_script_with};
use hacivat_ddl::script::{DelimStyle, Script};
use hacivat_ddl::{DdlTarget, GenOptions};
use hacivat_driver::{
    Driver, DriverConnect, DriverRegistry, Endpoint, EngineCatalog, InsecureAuth,
};
use hacivat_introspect::Introspector;
use hacivat_introspect::filter::{CompiledFilter, FilterRole, parse_filter};
use hacivat_introspect::introspector::FacetRows;
use hacivat_introspect::mysql::MySqlIntrospector;
use hacivat_introspect::pack::Mode;
use hacivat_introspect::pg::pass::{self, PassOpts};
use hacivat_introspect::sqlite::SqliteIntrospector;
use hacivat_meta::ids::SourceRef;
use hacivat_meta::merge::{MergePolicy, NamespaceResult, Trigger, base, merge};
use hacivat_meta::name::{FoldRule, IdentCtx};
use hacivat_meta::props::FacetKind;
use hacivat_meta::snapshot::{Catalog, CatalogChange, ContainerKey, LoadState, ServerFacts};
use hacivat_meta::view::SnapshotView;
use hacivat_meta::{Level, Name, NamespaceSnapshot, NsKey, ObjectKey, ObjectKind, Watermark};
use hacivat_net::ssh::{KnownHosts, TunnelManager};
use hacivat_proto::{DbmsKind, ProfileId};
use hacivat_secrets::{
    AuthProviderId, CredentialFiles, MemoryBackend, Retention, SecretBackend, SecretStore,
};
use hacivat_session::budget_for;
use hacivat_session::meta::{MetaConn, MetaConnSpec};
use hacivat_session::pipeline::{ApprovedRoots, ConnectDeps};
use hacivat_session::profile::{Profile, ProfileEndpoint};
use hacivat_session::session::SessionManager;
use hacivat_testkit::{DockerTarget, docker_enabled, require_target};
use sql_dialect::{DialectId, ServerVersion};
use tokio_util::sync::CancellationToken;

// ---- shared ------------------------------------------------------------------------------------

const PG_TARGETS: [&str; 5] = ["pg14", "pg15", "pg16", "pg17", "pg18"];
const MYSQL_TARGETS: [&str; 2] = ["mysql80", "mysql84"];
const MARIADB_TARGETS: [&str; 3] = ["mariadb106", "mariadb1011", "mariadb114"];

fn targets(ids: &[&str]) -> Vec<DockerTarget> {
    if !docker_enabled() {
        return Vec::new();
    }
    ids.iter().filter_map(|id| require_target(id)).collect()
}

fn n(s: &str) -> Name {
    Name::from(s)
}

/// Source texts read for one introspection: by `SourceRef` and by object (`Sources` facet).
#[derive(Default)]
struct Texts {
    by_ref: BTreeMap<u128, Name>,
    by_key: Vec<(ObjectKey, Name)>,
}

impl Texts {
    fn of(rows: Vec<(ObjectKey, Name)>) -> Texts {
        let mut t = Texts::default();
        for (k, text) in rows {
            t.by_ref.insert(SourceRef::of(&text).hash, text.clone());
            t.by_key.push((k, text));
        }
        t
    }
}

impl DdlSources for Texts {
    fn text(&self, r: &SourceRef) -> Option<Name> {
        self.by_ref.get(&r.hash).cloned()
    }

    fn server_text(&self, object: &ObjectKey) -> Option<Name> {
        self.by_key
            .iter()
            .find(|(k, _)| k.same_object(object))
            .map(|(_, t)| t.clone())
    }
}

/// One introspection of the scratch: its namespace, server facts and source texts.
struct Loaded {
    ns: NamespaceSnapshot,
    facts: ServerFacts,
    texts: Texts,
}

/// `r` merged into the empty namespace `ns` (a first, user-triggered load).
fn merged(ns: &NsKey, dbms: DbmsKind, fold: FoldRule, r: NamespaceResult) -> NamespaceSnapshot {
    let b = base(ns.clone(), dbms, fold).expect("a base");
    let policy = MergePolicy {
        trigger: Trigger::User,
        guard_min_objects: u32::MAX,
        ..MergePolicy::default()
    };
    merge(Some(&b), r, &policy)
        .expect("the merge runs")
        .next()
        .clone()
}

/// A published catalog holding `l.ns` loaded at `Structure`, with its schema list loaded.
fn view_of(l: &Loaded, database: Option<&str>) -> SnapshotView {
    let catalog = Catalog::new(Arc::new(l.facts.clone()));
    let publisher = catalog.publisher().expect("the only publisher");
    let loaded = LoadState::Loaded {
        level: Level::Structure,
        at: SystemTime::UNIX_EPOCH,
        watermark: Watermark::None,
        stale: false,
    };
    let db = database.map(n);
    publisher
        .publish(vec![
            CatalogChange::SetState(ContainerKey::Namespace(l.ns.key().clone()), loaded.clone()),
            CatalogChange::Put(Arc::new(l.ns.clone())),
            CatalogChange::SetState(ContainerKey::schema_list(db.as_ref()), loaded),
        ])
        .expect("publish");
    SnapshotView::new(catalog.current())
}

/// The version the app's DDL view gives a server (`ddl_view::model::server_version`).
fn version_of(dbms: DbmsKind, num: u32) -> ServerVersion {
    let part = |v: u32| u16::try_from(v).unwrap_or(u16::MAX);
    match dbms {
        DbmsKind::Postgres if num >= 100_000 => {
            ServerVersion::new(part(num / 10_000), 0, part(num % 10_000))
        }
        DbmsKind::Postgres | DbmsKind::MySql | DbmsKind::MariaDb => {
            ServerVersion::new(part(num / 10_000), part((num / 100) % 100), part(num % 100))
        }
        _ => ServerVersion::new(0, 0, 0),
    }
}

fn dialect_of(dbms: DbmsKind) -> DialectId {
    match dbms {
        DbmsKind::MySql => DialectId::MySql,
        DbmsKind::MariaDb => DialectId::MariaDb,
        DbmsKind::Sqlite => DialectId::Sqlite,
        _ => DialectId::Postgres,
    }
}

/// The target of `facts`'s server with `schema` current (the scratch).
fn target_of(facts: &ServerFacts, schema: &str) -> DdlTarget {
    let mut t = DdlTarget::new(
        dialect_of(facts.dbms),
        version_of(facts.dbms, facts.version_num),
    );
    t.flavor = facts.flavor;
    t.current_schema = Some(n(schema));
    t.quote.standard_conforming_strings = facts
        .standard_conforming_strings
        .or((facts.dbms == DbmsKind::Postgres).then_some(true));
    t.quote.no_backslash_escapes = facts.no_backslash_escapes;
    t
}

/// The key of the top-level object `name` of `kind` in `ns`.
fn key_of(ns: &NamespaceSnapshot, kind: ObjectKind, name: &str) -> ObjectKey {
    let depth = ns.key().path.len() + 1;
    let all: Vec<ObjectKey> = (0..ns.len() as u32)
        .filter_map(|i| ns.object_key(i))
        .collect();
    all.iter()
        .find(|k| k.kind == kind && k.path.len() == depth && &*k.path[depth - 1] == name)
        .cloned()
        .unwrap_or_else(|| {
            let top: Vec<_> = all.iter().filter(|k| k.path.len() == depth).collect();
            panic!("{kind:?} {name} not in the namespace; top level: {top:?}")
        })
}

/// Every key of `ns` whose kind is in `kinds`.
fn keys_of(ns: &NamespaceSnapshot, kinds: &[ObjectKind]) -> Vec<ObjectKey> {
    (0..ns.len() as u32)
        .filter_map(|i| ns.object_key(i))
        .filter(|k| kinds.contains(&k.kind))
        .collect()
}

/// Every object of `ns` as text, sorted: key (no server id), flags and props, with PostgreSQL
/// oids masked (a re-created object has new ones). State marks are left out (creation times).
fn canon(ns: &NamespaceSnapshot) -> Vec<String> {
    let oid = regex::Regex::new(r"Oid\(\d+\)").expect("regex");
    let mut out: Vec<String> = (0..ns.len() as u32)
        .map(|i| {
            let o = ns.obj(i).expect("object");
            let mut k = ns.object_key(i).expect("key");
            k.server_id = None;
            let text = format!("{k:?} {:?} {:?}", o.flags(), o.props());
            oid.replace_all(&text, "Oid(_)").into_owned()
        })
        .collect();
    out.sort();
    out
}

/// Each object's CREATE script, in `objects` order.
fn render_each(
    l: &Loaded,
    database: Option<&str>,
    schema: &str,
    objects: &[(ObjectKind, &str)],
    opts: &GenOptions,
) -> Vec<(String, Script)> {
    let view = view_of(l, database);
    let target = target_of(&l.facts, schema);
    objects
        .iter()
        .map(|(kind, name)| {
            let key = key_of(&l.ns, *kind, name);
            let s = create_script_with(&view, &l.texts, std::slice::from_ref(&key), &target, opts)
                .unwrap_or_else(|e| panic!("{kind:?} {name}: no script: {e:?}"));
            assert!(!s.stmts.is_empty(), "{kind:?} {name}: an empty script");
            (format!("{kind:?} {name}"), s)
        })
        .collect()
}

/// The two renders are the same bytes per object (with the MySQL client form too), and the
/// two models are equal.
fn assert_round_trip(
    label: &str,
    first: (&Loaded, &[(String, Script)]),
    second: (&Loaded, &[(String, Script)]),
) {
    assert_eq!(first.1.len(), second.1.len());
    for ((what, a), (_, b)) in first.1.iter().zip(second.1) {
        for style in [DelimStyle::Semicolon, DelimStyle::MySqlDelimiter] {
            let (a, b) = (a.text(style), b.text(style));
            assert!(
                a.as_bytes() == b.as_bytes(),
                "{label}: {what} ({style:?}) changed in the round trip\n--- first\n{a}\n--- second\n{b}"
            );
        }
    }
    let (ca, cb) = (canon(&first.0.ns), canon(&second.0.ns));
    assert_eq!(ca, cb, "{label}: the models differ");
    let stmts: usize = first.1.iter().map(|(_, s)| s.stmts.len()).sum();
    fact(&format!(
        "g5 roundtrip target={label} objects={} statements={stmts} model_objects={} bytes_identical=yes model_equal=yes",
        first.1.len(),
        ca.len()
    ));
}

// ---- PostgreSQL -------------------------------------------------------------------------------

const PG_SEED: &str = r#"
CREATE TYPE {s}.mood AS ENUM ('sad', 'ok', 'happy');
CREATE SEQUENCE {s}.invoice_no START 1000 INCREMENT 5;
CREATE TABLE {s}.customers (
    id bigint GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,
    email text NOT NULL,
    "Ad" varchar(80) DEFAULT 'x',
    ılık {s}.mood,
    score numeric(10,2) CHECK (score >= 0),
    CONSTRAINT customers_email_uq UNIQUE (email)
);
COMMENT ON TABLE {s}.customers IS 'Müşteriler';
COMMENT ON COLUMN {s}.customers.email IS 'Login';
CREATE TABLE {s}.orders (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    customer_id bigint NOT NULL REFERENCES {s}.customers (id) ON DELETE CASCADE,
    placed_at timestamptz NOT NULL DEFAULT now(),
    note text
);
CREATE INDEX orders_customer_idx ON {s}.orders (customer_id, placed_at DESC);
CREATE INDEX orders_note_lower ON {s}.orders (lower(note)) WHERE note IS NOT NULL;
"#;

const PG_OBJECTS: &[(ObjectKind, &str)] = &[
    (ObjectKind::EnumType, "mood"),
    (ObjectKind::Sequence, "invoice_no"),
    (ObjectKind::Table, "customers"),
    (ObjectKind::Table, "orders"),
];

async fn pg_load(pg: &Pg, schema: &str) -> Loaded {
    let ns = NsKey::schema([pg.t.database.as_str(), schema]);
    let mut c = pg.meta().await;
    let p = pass::introspect(
        &mut c,
        &ns,
        PassOpts {
            mode: Mode::Full,
            level: Level::Structure,
            ..PassOpts::default()
        },
        &CancellationToken::new(),
    )
    .await
    .unwrap_or_else(|e| panic!("{}: the pass: {e:?}", pg.t.id));
    let facts = p
        .report
        .facts
        .clone()
        .unwrap_or_else(|| panic!("{}: the pass read no server facts", pg.t.id));
    Loaded {
        ns: merged(&ns, DbmsKind::Postgres, FoldRule::Exact, p.result),
        facts,
        texts: Texts::default(),
    }
}

fn pg_opts() -> GenOptions {
    GenOptions {
        skip_grants: true,
        ..GenOptions::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_roundtrip_apply_introspect_render_is_identical() {
    for t in targets(&PG_TARGETS) {
        let schema = unique("hvm4_t31_rt");
        let _drop = DropSchemas {
            t: t.clone(),
            names: vec![schema.clone()],
        };
        let mut a = admin(&t).await;
        a.batch(&format!(
            "CREATE SCHEMA {schema};{}",
            PG_SEED.replace("{s}", &schema)
        ))
        .await;
        a.close().await;
        let pg = Pg::connect(&t).await;
        let db = pg.t.database.clone();

        let first = pg_load(&pg, &schema).await;
        let scripts = render_each(&first, Some(&db), &schema, PG_OBJECTS, &pg_opts());

        let mut a = admin(&t).await;
        a.batch(&format!(
            "DROP SCHEMA {schema} CASCADE; CREATE SCHEMA {schema}; SET search_path TO {schema}"
        ))
        .await;
        for (_, s) in &scripts {
            for stmt in &s.stmts {
                a.batch(&stmt.text).await;
            }
        }
        a.close().await;

        let second = pg_load(&pg, &schema).await;
        let again = render_each(&second, Some(&db), &schema, PG_OBJECTS, &pg_opts());
        assert_round_trip(&t.id, (&first, &scripts), (&second, &again));
        pg.shutdown().await;
    }
}

/// Every PostgreSQL golden of `hacivat-ddl` (its `golden_stability` test proves each file is
/// the bytes its case renders), by file name, sorted.
fn pg_goldens() -> Vec<(String, String)> {
    let dir: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "..",
        "hacivat-ddl",
        "tests",
        "golden",
        "pg",
    ]
    .iter()
    .collect();
    let mut out: Vec<(String, String)> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| {
            let p = e.expect("entry").path();
            let name = p
                .file_name()
                .expect("a name")
                .to_string_lossy()
                .into_owned();
            (name, std::fs::read_to_string(&p).expect("a UTF-8 golden"))
        })
        .collect();
    out.sort();
    out
}

/// The lowest PostgreSQL major whose grammar has a golden's syntax (every other PG golden parses
/// on every supported major, 14 and later).
fn min_major(rel: &str) -> u32 {
    match rel {
        // `NOT ENFORCED` on CHECK and FOREIGN KEY constraints is PostgreSQL 18 grammar.
        "not_enforced_pg18.sql" => 18,
        _ => 14,
    }
}

/// The PostgreSQL syntax-error SQLSTATEs (class 42 codes the raw parser and scanner raise).
const SYNTAX: [&str; 3] = ["42601", "42602", "42622"];

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_rendered_scripts_parse() {
    for t in targets(&PG_TARGETS) {
        let schema = unique("hvm4_t31_parse");
        let _drop = DropSchemas {
            t: t.clone(),
            names: vec![schema.clone()],
        };
        let mut a = admin(&t).await;
        a.batch(&format!(
            "CREATE SCHEMA {schema}; SET search_path TO {schema}"
        ))
        .await;
        let num: u32 = a
            .scalar("SELECT current_setting('server_version_num')")
            .await
            .parse()
            .expect("server_version_num");
        let major = num / 10_000;
        let (mut cases, mut ran, mut semantic, mut newer) = (0, 0, 0, 0);
        for (rel, text) in pg_goldens() {
            a.batch("BEGIN").await;
            let got = a.live().raw().batch(&text).await;
            if major < min_major(&rel) {
                // Not this major's grammar: the server's answer is recorded, not judged.
                let state = got
                    .err()
                    .and_then(|e| e.server.and_then(|m| m.sqlstate))
                    .unwrap_or_else(|| "accepted".into());
                fact(&format!(
                    "g5 pg-parse target={} golden={rel} needs_major={} answer={state}",
                    t.id,
                    min_major(&rel)
                ));
                newer += 1;
                a.batch("ROLLBACK").await;
                continue;
            }
            cases += 1;
            match got {
                Ok(_) => ran += 1,
                Err(e) => {
                    let state = e.server.as_ref().and_then(|m| m.sqlstate.clone());
                    assert!(
                        state.as_deref().is_some_and(|s| !SYNTAX.contains(&s)),
                        "{}: {} does not parse (or failed without a server code): {:?} {state:?}",
                        t.id,
                        rel,
                        e.kind
                    );
                    semantic += 1;
                }
            }
            a.batch("ROLLBACK").await;
        }
        a.close().await;
        assert!(cases > 0, "PostgreSQL goldens exist");
        fact(&format!(
            "g5 pg-parse target={} major={major} goldens={cases} ran={ran} parsed_semantic_error={semantic} syntax_errors=0 skipped_newer_grammar={newer}",
            t.id
        ));
    }
}

// ---- MySQL and MariaDB --------------------------------------------------------------------

const MY_SEED: &[&str] = &[
    "CREATE TABLE customers (
        id bigint unsigned NOT NULL AUTO_INCREMENT,
        email varchar(120) NOT NULL,
        name varchar(80) DEFAULT 'x' COMMENT 'Display name',
        score decimal(10,2) DEFAULT NULL,
        PRIMARY KEY (id),
        UNIQUE KEY customers_email_uq (email),
        CONSTRAINT customers_score_ck CHECK (score >= 0)
    ) ENGINE=InnoDB COMMENT='Customers'",
    "CREATE TABLE orders (
        id bigint unsigned NOT NULL AUTO_INCREMENT PRIMARY KEY,
        customer_id bigint unsigned NOT NULL,
        placed_at timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
        note varchar(200),
        KEY orders_customer_idx (customer_id, placed_at),
        CONSTRAINT orders_customer_fk FOREIGN KEY (customer_id) REFERENCES customers (id)
            ON DELETE CASCADE
    ) ENGINE=InnoDB",
    "CREATE TRIGGER orders_touch BEFORE INSERT ON orders FOR EACH ROW \
     SET NEW.note = COALESCE(NEW.note, '')",
    "CREATE VIEW v_open AS SELECT id, note FROM orders WHERE note IS NOT NULL",
    "CREATE FUNCTION f_total(c bigint) RETURNS int DETERMINISTIC READS SQL DATA \
     BEGIN DECLARE k int; SELECT COUNT(*) INTO k FROM orders WHERE customer_id = c; RETURN k; END",
];

const MY_OBJECTS: &[(ObjectKind, &str)] = &[
    (ObjectKind::Table, "customers"),
    (ObjectKind::Table, "orders"),
    (ObjectKind::View, "v_open"),
    (ObjectKind::Routine, "f_total"),
];

/// Drops a MySQL-family database when it goes, also on panic.
struct DropDb {
    t: DockerTarget,
    name: String,
}

impl Drop for DropDb {
    fn drop(&mut self) {
        my::exec_blocking(
            &self.t,
            Login::Test,
            &format!("DROP DATABASE IF EXISTS `{}`", self.name),
        );
    }
}

async fn my_run(t: &DockerTarget, db: Option<&str>, sql: &str) {
    my::exec(t, Login::Test, db, sql)
        .await
        .unwrap_or_else(|code| {
            let shown = sql.split('\'').next().unwrap_or_default();
            panic!("{}: `{shown}…`: {code}", t.id)
        });
}

async fn my_load(t: &DockerTarget, db: &str) -> Loaded {
    let ns = NsKey::schema([db]);
    let i = MySqlIntrospector { flavor: t.engine };
    let mut c = my::meta(t, db).await;
    let (_, facts) = i
        .server_facts(&mut c)
        .await
        .unwrap_or_else(|e| panic!("{}: server facts: {e:?}", t.id));
    let filter: CompiledFilter =
        parse_filter("", DialectId::MySql, FilterRole::Introspect).expect("the empty filter");
    let r = i
        .introspect(
            &mut c,
            &ns,
            Mode::Full,
            Level::Structure,
            &filter,
            None,
            &CancellationToken::new(),
        )
        .await
        .unwrap_or_else(|e| panic!("{}: the pass: {e:?}", t.id));
    let snap = merged(&ns, t.engine, FoldRule::Exact, r);
    let keys = keys_of(
        &snap,
        &[
            ObjectKind::Table,
            ObjectKind::View,
            ObjectKind::Routine,
            ObjectKind::Procedure,
            ObjectKind::Trigger,
            ObjectKind::Event,
        ],
    );
    let rows = match i.facet(&mut c, &ns, &keys, FacetKind::Sources).await {
        Ok(FacetRows::Sources(rows)) => rows,
        other => panic!("{}: the sources facet: {other:?}", t.id),
    };
    Loaded {
        ns: snap,
        facts,
        texts: Texts::of(rows),
    }
}

async fn my_round_trip(t: &DockerTarget) {
    let db = unique("hvm4_t31_rt");
    let _drop = DropDb {
        t: t.clone(),
        name: db.clone(),
    };
    my_run(t, None, &format!("CREATE DATABASE `{db}`")).await;
    for sql in MY_SEED {
        my_run(t, Some(&db), sql).await;
    }
    let opts = GenOptions {
        skip_grants: true,
        ..GenOptions::default()
    };
    let first = my_load(t, &db).await;
    let scripts = render_each(&first, None, &db, MY_OBJECTS, &opts);

    my_run(t, None, &format!("DROP DATABASE `{db}`")).await;
    my_run(t, None, &format!("CREATE DATABASE `{db}`")).await;
    for (_, s) in &scripts {
        for stmt in &s.stmts {
            my_run(t, Some(&db), &stmt.text).await;
        }
    }

    let second = my_load(t, &db).await;
    let again = render_each(&second, None, &db, MY_OBJECTS, &opts);
    assert_round_trip(&t.id, (&first, &scripts), (&second, &again));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn mysql_roundtrip() {
    for t in targets(&MYSQL_TARGETS) {
        my_round_trip(&t).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn mariadb_roundtrip() {
    for t in targets(&MARIADB_TARGETS) {
        my_round_trip(&t).await;
    }
}

// ---- SQLite (in-process) ---------------------------------------------------------------------

const LITE_SEED: &[&str] = &[
    "CREATE TABLE authors (id INTEGER PRIMARY KEY, name TEXT NOT NULL COLLATE NOCASE UNIQUE)",
    "CREATE TABLE notes (
        id INTEGER PRIMARY KEY,
        author_id INTEGER REFERENCES authors (id) ON DELETE CASCADE,
        body TEXT NOT NULL CHECK (length(body) < 1000),
        words INTEGER GENERATED ALWAYS AS (length(body)) VIRTUAL
    ) STRICT",
    "CREATE INDEX notes_author_idx ON notes (author_id) WHERE author_id IS NOT NULL",
    "CREATE TRIGGER notes_touch AFTER INSERT ON notes BEGIN \
     UPDATE authors SET name = name WHERE id = NEW.author_id; END",
    "CREATE VIEW v_notes AS SELECT n.id, a.name FROM notes n JOIN authors a ON a.id = n.author_id",
];

const LITE_OBJECTS: &[(ObjectKind, &str)] = &[
    (ObjectKind::Table, "authors"),
    (ObjectKind::Table, "notes"),
    (ObjectKind::View, "v_notes"),
];

fn lite_deps() -> ConnectDeps {
    let backend: Arc<dyn SecretBackend> = Arc::new(MemoryBackend::new());
    let known_hosts = KnownHosts::load(
        Vec::new(),
        PathBuf::from("/nonexistent/hacivat/known_hosts"),
    );
    let tunnels = TunnelManager::new(known_hosts.clone(), Default::default());
    ConnectDeps {
        drivers: Arc::new(hacivat_session::default_drivers()),
        catalog: EngineCatalog::embedded(),
        secrets: Arc::new(SecretStore::new(Some(backend))),
        files: CredentialFiles {
            pgpass: None,
            pg_service: None,
        },
        transport_wrap: None,
        extra_steps: Vec::new(),
        approved_roots: ApprovedRoots::for_user(),
        timeout_override: None,
        tunnels,
        known_hosts,
    }
}

/// Creates `path` and runs `stmts` on it, one at a time, through the real driver.
async fn lite_file(path: &Path, stmts: &[String]) {
    let registry: DriverRegistry = hacivat_session::default_drivers();
    let driver: Arc<dyn Driver> = registry
        .resolve(DbmsKind::Sqlite)
        .expect("the SQLite driver");
    let mut conn = driver
        .connect(DriverConnect {
            endpoint: Endpoint::File {
                path: path.to_path_buf(),
                create_if_missing: true,
                read_only: false,
            },
            tls: None,
            user: None,
            password: None,
            database: None,
            application_name: "t31".into(),
            read_only: false,
            options: Vec::new(),
            connect_timeout: Duration::from_secs(10),
            events: Arc::new(|_| {}),
            insecure_auth: InsecureAuth::default(),
        })
        .await
        .expect("the file opens");
    for sql in stmts {
        if let Err(e) = conn.batch(sql).await {
            panic!("{}: `{sql}`: {:?}", path.display(), e.kind);
        }
    }
    conn.close().await;
}

/// A full pass over `main` of `path` on the session manager's reader, and the sources facet.
async fn lite_load(path: &Path) -> Loaded {
    let profile = Profile {
        id: "6f1c2a4e-1d16-4a16-8a16-000000000031"
            .parse::<ProfileId>()
            .expect("an id"),
        name: "t31".into(),
        engine: DbmsKind::Sqlite,
        endpoint: ProfileEndpoint::File {
            path: path.to_path_buf(),
            create_if_missing: false,
        },
        user: None,
        auth: AuthProviderId::None,
        retention: Retention::Never,
        ..Profile::default()
    };
    let sm = SessionManager::new(
        tokio::runtime::Handle::current(),
        Arc::new(profile.clone()),
        Arc::new(lite_deps()),
        Arc::new(budget_for(&profile)),
    );
    let mut c: MetaConn = sm
        .meta_conn(
            MetaConnSpec {
                profile: profile.id,
                database: None,
                statement_timeout: Duration::from_secs(60),
            },
            &CancellationToken::new(),
        )
        .await
        .expect("a reader");
    let ns = NsKey::schema(["main"]);
    let i = SqliteIntrospector::default();
    let filter = parse_filter("", DialectId::Sqlite, FilterRole::Introspect).expect("filter");
    let r = i
        .introspect(
            &mut c,
            &ns,
            Mode::Full,
            Level::Structure,
            &filter,
            None,
            &CancellationToken::new(),
        )
        .await
        .unwrap_or_else(|e| panic!("the pass: {e:?}"));
    let snap = merged(&ns, DbmsKind::Sqlite, FoldRule::AsciiCaseless, r);
    let keys = keys_of(
        &snap,
        &[
            ObjectKind::Table,
            ObjectKind::View,
            ObjectKind::Index,
            ObjectKind::Trigger,
        ],
    );
    let rows = match i.facet(&mut c, &ns, &keys, FacetKind::Sources).await {
        Ok(FacetRows::Sources(rows)) => rows,
        other => panic!("the sources facet: {other:?}"),
    };
    drop(c);
    sm.close_all().await;
    Loaded {
        ns: snap,
        facts: ServerFacts::new(DbmsKind::Sqlite, IdentCtx::new(DialectId::Sqlite)),
        texts: Texts::of(rows),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_roundtrip() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (a, b) = (dir.path().join("a.db"), dir.path().join("b.db"));
    let seed: Vec<String> = LITE_SEED.iter().map(|s| s.to_string()).collect();
    lite_file(&a, &seed).await;
    let opts = GenOptions::default();

    let first = lite_load(&a).await;
    let scripts = render_each(&first, None, "main", LITE_OBJECTS, &opts);
    let stmts: Vec<String> = scripts
        .iter()
        .flat_map(|(_, s)| s.stmts.iter().map(|st| st.text.clone()))
        .collect();
    lite_file(&b, &stmts).await;

    let second = lite_load(&b).await;
    let again = render_each(&second, None, "main", LITE_OBJECTS, &opts);
    assert_round_trip("sqlite", (&first, &scripts), (&second, &again));
}
