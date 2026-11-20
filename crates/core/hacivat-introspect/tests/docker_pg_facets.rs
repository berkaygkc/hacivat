// ## m4-t16b-pg-facets
//! PostgreSQL facets and the server-level namespace on real servers (M4 Task 16b; R-M4-30):
//! grants (relation, column, routine, type and schema ACLs, decoded through `aclexplode`,
//! never as text), dependencies (`pg_depend` and view rules), row estimates on demand, the
//! roles of the `NsKind::Server` namespace, the invalidation of a loaded grants facet by the
//! pass that re-reads its object, and refused reads that fail instead of answering empty.
//!
//! Every test is `#[ignore = "docker"]`, returns early unless `HACIVAT_DOCKER=1`, takes its
//! targets only from `require_target`, and creates its own `hvm4_t16b_*` schema, roles (and,
//! for the refused reads, database), dropped by a guard, also on panic.
//!
//! Run: `<main>/target/docker/run-docker-tests.sh --set full -p hacivat-introspect --test
//! docker_pg_facets` (PR set: pg17 only).

mod common;

use std::collections::BTreeSet;

use common::pg::{PG, Pg, admin, admin_on, unique};
use hacivat_introspect::introspector::FacetRows;
use hacivat_introspect::pack::Mode;
use hacivat_introspect::pg::facets::{self, ObjGrants, PUBLIC};
use hacivat_introspect::pg::pass::{self, PassOpts};
use hacivat_introspect::pg::sections::{Binds, ctx};
use hacivat_introspect::pg::{PgIntrospector, sections};
use hacivat_introspect::{IntroError, Introspector};
use hacivat_meta::merge::{MergeOutcome, MergePolicy, NamespaceResult, Trigger, base, merge};
use hacivat_meta::name::FoldRule;
use hacivat_meta::props::{DepsKind, FacetKind, Grant, GrantsFacet, Props};
use hacivat_meta::{Level, Name, NamespaceSnapshot, NsKey, ObjectKey, ObjectKind, Ref, ServerId};
use hacivat_proto::DbmsKind;
use hacivat_session::meta::MetaConn;
use hacivat_testkit::{DockerTarget, docker_enabled, require_target};
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------------------------
// Helpers

fn targets() -> Vec<DockerTarget> {
    PG.into_iter().filter_map(require_target).collect()
}

fn ns_of(t: &DockerTarget, schema: &str) -> NsKey {
    NsKey::schema([t.database.as_str(), schema])
}

/// A quoted identifier (fixture text only; product code never builds SQL from names).
fn qi(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// `r` merged into `prev` (a user refresh).
fn apply(prev: &NamespaceSnapshot, r: NamespaceResult) -> NamespaceSnapshot {
    let policy = MergePolicy {
        trigger: Trigger::User,
        ..MergePolicy::default()
    };
    match merge(Some(prev), r, &policy).expect("merge") {
        MergeOutcome::Applied { next, .. } | MergeOutcome::PendingConfirmation { next, .. } => next,
    }
}

async fn pass_at(
    c: &mut MetaConn,
    ns: &NsKey,
    mode: Mode,
    prev: Option<&NamespaceSnapshot>,
) -> NamespaceResult {
    let cancel = CancellationToken::new();
    pass::introspect(
        c,
        ns,
        PassOpts {
            mode,
            level: Level::Structure,
            prev,
            ..PassOpts::default()
        },
        &cancel,
    )
    .await
    .unwrap_or_else(|e| panic!("{ns:?} {mode:?}: {e:?}"))
    .result
}

/// A fresh full pass at `Structure`, as a namespace.
async fn load(c: &mut MetaConn, ns: &NsKey) -> NamespaceSnapshot {
    let r = pass_at(c, ns, Mode::Full, None).await;
    let b = base(ns.clone(), DbmsKind::Postgres, FoldRule::Exact).unwrap();
    apply(&b, r)
}

/// The key of the top-level object `name` of `kind` (the namespace's schema for
/// `ObjectKind::Schema`).
fn key_of(n: &NamespaceSnapshot, kind: ObjectKind, name: &str) -> ObjectKey {
    let base = n.key().path.len();
    (0..n.len() as u32)
        .find_map(|i| {
            let k = n.object_key(i)?;
            let hit = if kind == ObjectKind::Schema {
                k.kind == kind && k.path.len() == base
            } else {
                k.kind == kind && k.path.len() == base + 1 && &*k.path[base] == name
            };
            hit.then_some(k)
        })
        .unwrap_or_else(|| panic!("{kind:?} {name} not found"))
}

fn oid_of(k: &ObjectKey) -> u32 {
    match k.server_id {
        Some(ServerId::Oid(o)) => o,
        _ => panic!("{k:?} has no oid"),
    }
}

fn g(grantee: &str, privilege: &str, columns: &[&str], grantable: bool) -> Grant {
    Grant {
        grantee: Name::from(grantee),
        privilege: Name::from(privilege),
        columns: columns.iter().map(|c| Name::from(*c)).collect(),
        grantable,
    }
}

fn grants_of<'a>(all: &'a [ObjGrants], k: &ObjectKey) -> &'a ObjGrants {
    all.iter()
        .find(|o| o.key.same_object(k))
        .unwrap_or_else(|| panic!("{k:?} not answered"))
}

/// What the introspector's `facet` answers for `keys` (`Grants`).
async fn facet_grants(
    c: &mut MetaConn,
    ns: &NsKey,
    keys: &[ObjectKey],
) -> Vec<(ObjectKey, GrantsFacet)> {
    match PgIntrospector.facet(c, ns, keys, FacetKind::Grants).await {
        Ok(FacetRows::Grants(v)) => v,
        other => panic!("{ns:?}: {other:?}"),
    }
}

async fn read_grants(c: &mut MetaConn, ns: &NsKey, keys: &[ObjectKey]) -> Vec<ObjGrants> {
    let cancel = CancellationToken::new();
    facets::grants(c, ns, keys, &cancel)
        .await
        .unwrap_or_else(|e| panic!("{ns:?}: {e:?}"))
        .0
}

fn r(kind: ObjectKind, path: &[&str], oid: Option<u32>) -> Ref {
    Ref {
        kind,
        path: path.iter().map(|p| Name::from(*p)).collect(),
        server_id: oid.map(ServerId::Oid),
    }
}

/// A schema and roles of `t`, dropped when it goes, also on panic: the schema (its objects
/// and their privileges) first, then each role's remaining privileges in `hv_test`, then the
/// role. Errors are ignored here (a role a failed test never created).
struct Fixture {
    t: DockerTarget,
    schema: String,
    roles: Vec<String>,
}

impl Fixture {
    /// Creates schema `hvm4_t16b_<slug>_…` and `roles`, then runs `ddl` with `{s}` the schema
    /// and `{r0}`, `{r1}`, … the quoted roles.
    async fn new(t: &DockerTarget, slug: &str, roles: &[String], ddl: &str) -> Fixture {
        let schema = unique(&format!("hvm4_t16b_{slug}"));
        let f = Fixture {
            t: t.clone(),
            schema: schema.clone(),
            roles: roles.to_vec(),
        };
        let mut a = admin(t).await;
        a.batch(&format!("CREATE SCHEMA {schema}")).await;
        for role in roles {
            a.batch(&format!("CREATE ROLE {}", qi(role))).await;
        }
        let mut text = ddl.replace("{s}", &schema);
        for (i, role) in roles.iter().enumerate() {
            text = text.replace(&format!("{{r{i}}}"), &qi(role));
        }
        if !text.is_empty() {
            a.batch(&text).await;
        }
        a.close().await;
        f
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let t = self.t.clone();
        let mut sql = format!("DROP SCHEMA IF EXISTS {} CASCADE;", qi(&self.schema));
        let roles = self.roles.clone();
        let rt = tokio::runtime::Handle::current();
        std::thread::scope(|s| {
            s.spawn(|| {
                rt.block_on(async {
                    let mut a = admin(&t).await;
                    let _ = a.live().raw().batch(&sql).await;
                    for role in &roles {
                        sql = format!("DROP OWNED BY {}", qi(role));
                        let _ = a.live().raw().batch(&sql).await;
                        sql = format!("DROP ROLE IF EXISTS {}", qi(role));
                        let _ = a.live().raw().batch(&sql).await;
                    }
                    a.close().await;
                });
            });
        });
    }
}

/// A role name with the characters an ACL text parser splits on (`=`, `/`, `,`, `"`, a
/// space): aclexplode returns its oid and `pg_roles` its exact name.
fn hostile_role() -> String {
    format!("{} =r/\"x\",y", unique("hvm4_t16b_r"))
}

// ---------------------------------------------------------------------------------------------
// Grants

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_grants_and_owner_read() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let role = hostile_role();
        let f = Fixture::new(
            &t,
            "grants",
            std::slice::from_ref(&role),
            "CREATE TABLE {s}.t (a int, b text, c int);
             CREATE TABLE {s}.plain (a int);
             CREATE VIEW {s}.v AS SELECT a FROM {s}.t;
             CREATE SEQUENCE {s}.sq;
             CREATE FUNCTION {s}.f(x int) RETURNS int LANGUAGE sql AS 'SELECT x';
             CREATE FUNCTION {s}.g() RETURNS int LANGUAGE sql AS 'SELECT 1';
             CREATE DOMAIN {s}.d AS int;
             CREATE TYPE {s}.mood AS ENUM ('sad', 'ok');
             GRANT SELECT ON {s}.t TO {r0} WITH GRANT OPTION;
             GRANT INSERT, UPDATE ON {s}.t TO PUBLIC;
             GRANT SELECT ON {s}.v TO {r0};
             GRANT USAGE ON SEQUENCE {s}.sq TO {r0};
             GRANT USAGE ON DOMAIN {s}.d TO {r0};
             GRANT USAGE, CREATE ON SCHEMA {s} TO {r0};
             REVOKE EXECUTE ON FUNCTION {s}.g() FROM PUBLIC;",
        )
        .await;
        let ns = ns_of(&t, &f.schema);
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let n = load(&mut c, &ns).await;
        let k = |kind, name| key_of(&n, kind, name);
        let (kt, kplain, kv, ksq, kf, kg, kd, kmood, ks) = (
            k(ObjectKind::Table, "t"),
            k(ObjectKind::Table, "plain"),
            k(ObjectKind::View, "v"),
            k(ObjectKind::Sequence, "sq"),
            k(ObjectKind::Routine, "f"),
            k(ObjectKind::Routine, "g"),
            k(ObjectKind::Domain, "d"),
            k(ObjectKind::EnumType, "mood"),
            k(ObjectKind::Schema, ""),
        );
        let keys = vec![
            kt.clone(),
            kplain.clone(),
            kv.clone(),
            ksq.clone(),
            kf.clone(),
            kg.clone(),
            kd.clone(),
            kmood.clone(),
            ks.clone(),
        ];
        // The owner is the structure's (Task 16), never a grants entry.
        let ti = n.find(&kt).unwrap();
        match n.obj(ti).unwrap().props() {
            Props::Table(p) => {
                assert_eq!(p.owner.as_deref(), Some(t.user.as_str()), "{}", t.id);
            }
            other => panic!("{}: {other:?}", t.id),
        }
        let all = read_grants(&mut c, &ns, &keys).await;
        assert_eq!(all.len(), keys.len(), "{}: every object answered", t.id);
        let entries = |k: &ObjectKey| grants_of(&all, k).facet.entries.clone();
        assert_eq!(
            entries(&kt),
            vec![
                g(PUBLIC, "INSERT", &[], false),
                g(PUBLIC, "UPDATE", &[], false),
                g(&role, "SELECT", &[], true),
            ],
            "{}",
            t.id
        );
        assert_eq!(entries(&kplain), Vec::new(), "{}: defaults only", t.id);
        assert_eq!(
            entries(&kv),
            vec![g(&role, "SELECT", &[], false)],
            "{}",
            t.id
        );
        assert_eq!(
            entries(&ksq),
            vec![g(&role, "USAGE", &[], false)],
            "{}",
            t.id
        );
        assert_eq!(
            entries(&kd),
            vec![g(&role, "USAGE", &[], false)],
            "{}",
            t.id
        );
        assert_eq!(
            entries(&kf),
            Vec::new(),
            "{}: PUBLIC EXECUTE is a default",
            t.id
        );
        assert_eq!(entries(&kmood), Vec::new(), "{}", t.id);
        assert_eq!(
            entries(&ks),
            vec![
                g(&role, "CREATE", &[], false),
                g(&role, "USAGE", &[], false),
            ],
            "{}",
            t.id
        );
        // REVOKE EXECUTE ... FROM PUBLIC: re-granting the facet would give it back.
        let gg = grants_of(&all, &kg);
        assert_eq!(
            gg.revoked,
            vec![g(PUBLIC, "EXECUTE", &[], false)],
            "{}",
            t.id
        );
        assert!(!gg.complete(), "{}", t.id);
        for o in all.iter().filter(|o| !o.key.same_object(&kg)) {
            assert!(o.complete(), "{}: {:?}", t.id, o.key);
        }
        // The introspector answers every complete object and leaves `g` not loaded.
        let rows = facet_grants(&mut c, &ns, &keys).await;
        assert_eq!(rows.len(), keys.len() - 1, "{}", t.id);
        assert!(!rows.iter().any(|(k, _)| k.same_object(&kg)), "{}", t.id);
        assert!(rows.iter().any(|(k, _)| k.same_object(&kt)), "{}", t.id);
        // A role named exactly PUBLIC would print as the keyword: its object is left out.
        // PostgreSQL 13-18 accept the name (only `public` and `none` are reserved), so the
        // step always runs (review m-8): a leftover from a killed run is dropped first, and a
        // refusal fails the test.
        let twin = PUBLIC.to_string();
        let mut a = admin(&t).await;
        a.batch(
            "DO $$ BEGIN
               IF EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'PUBLIC') THEN
                 EXECUTE 'DROP OWNED BY \"PUBLIC\"';
                 EXECUTE 'DROP ROLE \"PUBLIC\"';
               END IF;
             END $$",
        )
        .await;
        a.batch(&format!("CREATE ROLE {}", qi(&twin))).await;
        let _drop_twin = Fixture {
            t: t.clone(),
            schema: unique("hvm4_t16b_none"),
            roles: vec![twin.clone()],
        };
        a.batch(&format!(
            "GRANT SELECT ON {}.plain TO {}",
            f.schema,
            qi(&twin)
        ))
        .await;
        a.close().await;
        let all = read_grants(&mut c, &ns, std::slice::from_ref(&kplain)).await;
        assert!(all[0].opaque && !all[0].complete(), "{}: {all:?}", t.id);
        let rows = facet_grants(&mut c, &ns, std::slice::from_ref(&kplain)).await;
        assert!(rows.is_empty(), "{}: {rows:?}", t.id);
        let mut a = admin(&t).await;
        a.batch(&format!(
            "REVOKE SELECT ON {}.plain FROM {}",
            f.schema,
            qi(&twin)
        ))
        .await;
        a.close().await;
        drop(c);
        pg.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_column_grant_read() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let role = hostile_role();
        let f = Fixture::new(
            &t,
            "colgrant",
            std::slice::from_ref(&role),
            "CREATE TABLE {s}.t (a int, b int, c int);
             GRANT SELECT (c, a) ON {s}.t TO {r0};
             GRANT UPDATE (c) ON {s}.t TO {r0};
             GRANT REFERENCES (b) ON {s}.t TO PUBLIC;",
        )
        .await;
        let ns = ns_of(&t, &f.schema);
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let n = load(&mut c, &ns).await;
        let kt = key_of(&n, ObjectKind::Table, "t");
        let all = read_grants(&mut c, &ns, std::slice::from_ref(&kt)).await;
        let got = grants_of(&all, &kt);
        assert!(got.complete(), "{}: {got:?}", t.id);
        // Column entries only (no table-level grant), columns in attnum order.
        assert_eq!(
            got.facet.entries,
            vec![
                g(PUBLIC, "REFERENCES", &["b"], false),
                g(&role, "SELECT", &["a", "c"], false),
                g(&role, "UPDATE", &["c"], false),
            ],
            "{}",
            t.id
        );
        drop(c);
        pg.shutdown().await;
    }
}

// ---------------------------------------------------------------------------------------------
// Dependencies

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_view_dependencies_read() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let f = Fixture::new(
            &t,
            "deps",
            &[],
            "CREATE TABLE {s}.t (id int PRIMARY KEY, a int, b text);
             CREATE TYPE {s}.mood AS ENUM ('sad', 'ok');
             CREATE FUNCTION {s}.f(x int) RETURNS int LANGUAGE sql IMMUTABLE AS 'SELECT x';
             CREATE VIEW {s}.v AS SELECT a, {s}.f(id) AS fid, 'ok'::{s}.mood AS m FROM {s}.t;
             CREATE VIEW {s}.v2 AS SELECT a FROM {s}.v;
             CREATE SEQUENCE {s}.sq;
             CREATE FUNCTION {s}.trg() RETURNS trigger LANGUAGE plpgsql AS 'BEGIN RETURN NEW; END';
             CREATE TABLE {s}.child (id int DEFAULT nextval('{s}.sq'), pid int REFERENCES {s}.t (id), m {s}.mood[]);
             CREATE TRIGGER tr BEFORE INSERT ON {s}.child FOR EACH ROW EXECUTE FUNCTION {s}.trg();
             CREATE FUNCTION {s}.allowed(x int) RETURNS boolean LANGUAGE sql IMMUTABLE AS 'SELECT x > 0';
             CREATE FUNCTION {s}.sq2(x int) RETURNS int LANGUAGE sql IMMUTABLE AS 'SELECT x * x';
             CREATE TABLE {s}.pol (a int);
             CREATE POLICY p ON {s}.pol USING ({s}.allowed(a));",
        )
        .await;
        // Review m-3: an expression statistics object (14 and later) is a part of its table.
        let expr_stats = t.major.parse::<u32>().unwrap_or(0) >= 14;
        if expr_stats {
            let mut a = admin(&t).await;
            a.batch(&format!(
                "CREATE STATISTICS {0}.st ON ({0}.sq2(a)) FROM {0}.pol",
                f.schema
            ))
            .await;
            a.close().await;
        }
        let s = f.schema.as_str();
        let db = t.database.as_str();
        let ns = ns_of(&t, s);
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let n = load(&mut c, &ns).await;
        let k = |kind, name| key_of(&n, kind, name);
        let (kt, kv, kv2, kchild, kf, ktrg, kmood, ksq, ks) = (
            k(ObjectKind::Table, "t"),
            k(ObjectKind::View, "v"),
            k(ObjectKind::View, "v2"),
            k(ObjectKind::Table, "child"),
            k(ObjectKind::Routine, "f"),
            k(ObjectKind::Routine, "trg"),
            k(ObjectKind::EnumType, "mood"),
            k(ObjectKind::Sequence, "sq"),
            k(ObjectKind::Schema, ""),
        );
        let (kpol, kallowed, ksq2) = (
            k(ObjectKind::Table, "pol"),
            k(ObjectKind::Routine, "allowed"),
            k(ObjectKind::Routine, "sq2"),
        );
        let keys = vec![
            kt.clone(),
            kv.clone(),
            kv2.clone(),
            kchild.clone(),
            ktrg.clone(),
            kpol.clone(),
        ];
        let rows = match PgIntrospector
            .facet(&mut c, &ns, &keys, FacetKind::Dependencies)
            .await
        {
            Ok(FacetRows::Dependencies(v)) => v,
            other => panic!("{}: {other:?}", t.id),
        };
        assert_eq!(rows.len(), keys.len(), "{}", t.id);
        let deps = |key: &ObjectKey| {
            let (_, d) = rows
                .iter()
                .find(|(x, _)| x.same_object(key))
                .unwrap_or_else(|| panic!("{}: {key:?} not answered", t.id));
            assert_eq!(d.kind, DepsKind::Catalog);
            d.depends_on.clone()
        };
        let schema = r(ObjectKind::Schema, &[db, s], Some(oid_of(&ks)));
        let table_t = r(ObjectKind::Table, &[db, s, "t"], Some(oid_of(&kt)));
        // The view: its table, the columns it reads, the function and the type it uses. Its
        // rule records the columns (PostgreSQL 13 records no whole-table row); a column
        // dependency names its table as well, once, with the table's oid (CI 44656387989).
        let v = deps(&kv);
        for want in [
            schema.clone(),
            table_t.clone(),
            r(ObjectKind::Column, &[db, s, "t", "a"], None),
            r(ObjectKind::Column, &[db, s, "t", "id"], None),
            r(ObjectKind::Routine, &[db, s, "f"], Some(oid_of(&kf))),
            r(ObjectKind::EnumType, &[db, s, "mood"], Some(oid_of(&kmood))),
        ] {
            assert!(v.contains(&want), "{}: {want:?} not in {v:#?}", t.id);
        }
        assert!(
            !v.iter()
                .any(|x| x.server_id == kv.server_id && x.kind == ObjectKind::View),
            "{}: a view never depends on itself",
            t.id
        );
        assert!(
            !v.iter().any(|x| x.path.get(2).is_some_and(|p| &**p == "v")),
            "{}: nor on its own columns: {v:#?}",
            t.id
        );
        // A view on a view.
        let v2 = deps(&kv2);
        assert!(
            v2.contains(&r(ObjectKind::View, &[db, s, "v"], Some(oid_of(&kv)))),
            "{}: {v2:#?}",
            t.id
        );
        // A table through its parts: the FK (the referenced columns, hence their table, and
        // the key's index), the default (the sequence), the trigger (its function), a column
        // type (an array of the enum, named by its element).
        let child = deps(&kchild);
        for want in [
            schema.clone(),
            table_t.clone(),
            r(ObjectKind::Column, &[db, s, "t", "id"], None),
            r(ObjectKind::Sequence, &[db, s, "sq"], Some(oid_of(&ksq))),
            r(ObjectKind::Routine, &[db, s, "trg"], Some(oid_of(&ktrg))),
            r(ObjectKind::EnumType, &[db, s, "mood"], Some(oid_of(&kmood))),
        ] {
            assert!(
                child.contains(&want),
                "{}: {want:?} not in {child:#?}",
                t.id
            );
        }
        assert!(
            child
                .iter()
                .any(|x| x.kind == ObjectKind::Index && x.path.as_slice()[..3] == table_t.path[..]),
            "{}: the referenced key's index: {child:#?}",
            t.id
        );
        assert!(
            !child
                .iter()
                .any(|x| x.path.get(2).is_some_and(|p| &**p == "child")),
            "{}: nothing of its own: {child:#?}",
            t.id
        );
        // The table depends on its schema only (its own columns, key and index are parts).
        assert_eq!(deps(&kt), vec![schema.clone()], "{}", t.id);
        // A row-level security policy and an expression statistics object are parts: the
        // functions they call are the table's dependencies (review m-3).
        let pol = deps(&kpol);
        let allowed = r(
            ObjectKind::Routine,
            &[db, s, "allowed"],
            Some(oid_of(&kallowed)),
        );
        let sq2 = r(ObjectKind::Routine, &[db, s, "sq2"], Some(oid_of(&ksq2)));
        assert!(pol.contains(&allowed), "{}: {pol:#?}", t.id);
        assert_eq!(pol.contains(&sq2), expr_stats, "{}: {pol:#?}", t.id);
        assert!(
            !pol.iter()
                .any(|x| x.path.get(2).is_some_and(|p| &**p == "pol")),
            "{}: nothing of its own: {pol:#?}",
            t.id
        );
        // A PL/pgSQL function depends on its language (a catalog the section names through
        // pg_identify_object).
        let trg = deps(&ktrg);
        assert!(
            trg.iter().any(|x| x.kind == ObjectKind::Language
                && x.path.last().is_some_and(|p| &**p == "plpgsql")),
            "{}: {trg:#?}",
            t.id
        );
        // Deterministic: a second read answers the same.
        let again = match PgIntrospector
            .facet(&mut c, &ns, &keys, FacetKind::Dependencies)
            .await
        {
            Ok(FacetRows::Dependencies(v)) => v,
            other => panic!("{}: {other:?}", t.id),
        };
        assert_eq!(again, rows, "{}", t.id);
        drop(c);
        pg.shutdown().await;
    }
}

// ---------------------------------------------------------------------------------------------
// Roles

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_roles_listed_at_names_level() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let role = hostile_role();
        let _f = Fixture::new(&t, "roles", std::slice::from_ref(&role), "").await;
        let mut a = admin(&t).await;
        let role_oid: u32 = a
            .scalar(&format!(
                "SELECT oid::text FROM pg_catalog.pg_roles WHERE rolname = '{}'",
                role.replace('\'', "''")
            ))
            .await
            .parse()
            .unwrap();
        a.close().await;
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let all = hacivat_introspect::filter::parse_filter(
            "",
            sql_dialect::DialectId::Postgres,
            hacivat_introspect::filter::FilterRole::Introspect,
        )
        .unwrap();
        let cancel = CancellationToken::new();
        let result = PgIntrospector
            .introspect(
                &mut c,
                &NsKey::server(),
                Mode::Full,
                Level::Names,
                &all,
                None,
                &cancel,
            )
            .await
            .unwrap_or_else(|e| panic!("{}: {e:?}", t.id));
        assert!(result.full && result.level == Level::Names, "{}", t.id);
        let b = base(NsKey::server(), DbmsKind::Postgres, FoldRule::Exact).unwrap();
        let n = apply(&b, result);
        let roles: BTreeSet<(String, Option<ServerId>)> = (0..n.len() as u32)
            .filter_map(|i| n.obj(i))
            .filter(|o| o.kind() == ObjectKind::Role)
            .map(|o| (o.name().to_string(), o.server_id()))
            .collect();
        assert!(
            roles.contains(&(role.clone(), Some(ServerId::Oid(role_oid)))),
            "{}: {roles:?}",
            t.id
        );
        assert!(
            roles.iter().any(|(r, _)| *r == t.user),
            "{}: the login itself",
            t.id
        );
        // Only the probe, the role list and the canary ran.
        let (_, log) = facets::roles(&mut c, &cancel).await.unwrap();
        assert_eq!(
            log.names(),
            vec![
                sections::name::IDENTITY,
                facets::name::LIST_ROLES,
                sections::name::CANARY
            ],
            "{}",
            t.id
        );
        drop(c);
        pg.shutdown().await;
    }
}

// ---------------------------------------------------------------------------------------------
// Invalidation

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn grant_change_invalidates_a_loaded_grants_facet() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let role = hostile_role();
        let f = Fixture::new(
            &t,
            "inval",
            std::slice::from_ref(&role),
            "CREATE TABLE {s}.t (a int, b int);
             CREATE TABLE {s}.u (a int);
             CREATE FUNCTION {s}.f() RETURNS int LANGUAGE sql AS 'SELECT 1';
             CREATE TYPE {s}.mood AS ENUM ('sad', 'ok');",
        )
        .await;
        let ns = ns_of(&t, &f.schema);
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let n0 = load(&mut c, &ns).await;
        let (kt, ku, kf) = (
            key_of(&n0, ObjectKind::Table, "t"),
            key_of(&n0, ObjectKind::Table, "u"),
            key_of(&n0, ObjectKind::Routine, "f"),
        );
        let loaded = facet_grants(&mut c, &ns, std::slice::from_ref(&kt)).await;
        assert_eq!(loaded[0].1.entries, Vec::new(), "{}", t.id);
        let hits = |r: &NamespaceResult, k: &ObjectKey| {
            facets::invalidated(&ns, r).iter().any(|x| x.same_object(k))
        };
        let grant = |sql: String| {
            let t = t.clone();
            async move {
                let mut a = admin(&t).await;
                a.batch(&sql).await;
                a.close().await;
            }
        };
        // A table GRANT rewrites the pg_class row: the next pass re-reads the table.
        grant(format!("GRANT SELECT ON {}.t TO {}", f.schema, qi(&role))).await;
        let r1 = pass_at(&mut c, &ns, Mode::Incremental, Some(&n0)).await;
        assert!(hits(&r1, &kt), "{}: the table's grants are stale", t.id);
        assert!(
            !hits(&r1, &ku),
            "{}: an untouched table keeps its facet",
            t.id
        );
        let n1 = apply(&n0, r1);
        let reread = facet_grants(&mut c, &ns, std::slice::from_ref(&kt)).await;
        assert_eq!(
            reread[0].1.entries,
            vec![g(&role, "SELECT", &[], false)],
            "{}",
            t.id
        );
        // A column GRANT rewrites a pg_attribute row (Task 16 cascades it to the table).
        grant(format!(
            "GRANT UPDATE (b) ON {}.t TO {}",
            f.schema,
            qi(&role)
        ))
        .await;
        let r2 = pass_at(&mut c, &ns, Mode::Incremental, Some(&n1)).await;
        assert!(hits(&r2, &kt), "{}: column grant", t.id);
        let n2 = apply(&n1, r2);
        let reread = facet_grants(&mut c, &ns, std::slice::from_ref(&kt)).await;
        assert_eq!(
            reread[0].1.entries,
            vec![
                g(&role, "SELECT", &[], false),
                g(&role, "UPDATE", &["b"], false),
            ],
            "{}",
            t.id
        );
        // A routine GRANT rewrites its pg_proc row.
        grant(format!(
            "GRANT EXECUTE ON FUNCTION {}.f() TO {}",
            f.schema,
            qi(&role)
        ))
        .await;
        let r3 = pass_at(&mut c, &ns, Mode::Incremental, Some(&n2)).await;
        assert!(hits(&r3, &kf), "{}: routine grant", t.id);
        assert!(!hits(&r3, &kt), "{}: the table did not change", t.id);
        let n3 = apply(&n2, r3);
        // A type GRANT rewrites its pg_type row (review m-8).
        let kmood = key_of(&n3, ObjectKind::EnumType, "mood");
        grant(format!(
            "GRANT USAGE ON TYPE {}.mood TO {}",
            f.schema,
            qi(&role)
        ))
        .await;
        let r4 = pass_at(&mut c, &ns, Mode::Incremental, Some(&n3)).await;
        assert!(hits(&r4, &kmood), "{}: type grant", t.id);
        let n4 = apply(&n3, r4);
        let reread = facet_grants(&mut c, &ns, std::slice::from_ref(&kmood)).await;
        assert_eq!(
            reread[0].1.entries,
            vec![g(&role, "USAGE", &[], false)],
            "{}",
            t.id
        );
        // A schema GRANT rewrites the pg_namespace row: the namespace's own schema object.
        let ks = key_of(&n4, ObjectKind::Schema, "");
        grant(format!(
            "GRANT USAGE ON SCHEMA {} TO {}",
            f.schema,
            qi(&role)
        ))
        .await;
        let r5 = pass_at(&mut c, &ns, Mode::Incremental, Some(&n4)).await;
        assert!(hits(&r5, &ks), "{}: schema grant", t.id);
        let reread = facet_grants(&mut c, &ns, std::slice::from_ref(&ks)).await;
        assert_eq!(
            reread[0].1.entries,
            vec![g(&role, "USAGE", &[], false)],
            "{}",
            t.id
        );
        drop(c);
        pg.shutdown().await;
    }
}

// ---------------------------------------------------------------------------------------------
// Statistics (Task 16 review m-5)

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_statistics_facet_reads_the_current_row_estimate() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let f = Fixture::new(
            &t,
            "stats",
            &[],
            "CREATE TABLE {s}.t (a int) WITH (autovacuum_enabled = false);
             INSERT INTO {s}.t SELECT g FROM pg_catalog.generate_series(1, 1000) g;
             CREATE VIEW {s}.v AS SELECT a FROM {s}.t;",
        )
        .await;
        let ns = ns_of(&t, &f.schema);
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let n = load(&mut c, &ns).await;
        let (kt, kv) = (
            key_of(&n, ObjectKind::Table, "t"),
            key_of(&n, ObjectKind::View, "v"),
        );
        let stats = |rows: FacetRows| match rows {
            FacetRows::Statistics(v) => v,
            other => panic!("{other:?}"),
        };
        // Never analyzed (autovacuum off for the table): no estimate on 14+ (`-1`); 13
        // reports 0, which is indistinguishable from an empty table (review m-4, carried).
        let before = stats(
            PgIntrospector
                .facet(
                    &mut c,
                    &ns,
                    std::slice::from_ref(&kt),
                    FacetKind::Statistics,
                )
                .await
                .unwrap_or_else(|e| panic!("{}: {e:?}", t.id)),
        );
        let want = if t.major.parse::<u32>().unwrap_or(0) >= 14 {
            None
        } else {
            Some(0)
        };
        assert_eq!(before.len(), 1, "{}", t.id);
        assert_eq!(before[0].1.row_estimate, want, "{}: never analyzed", t.id);
        // ANALYZE updates reltuples in place (no new xmin): only the facet sees it.
        let mut a = admin(&t).await;
        a.batch(&format!("ANALYZE {}.t", f.schema)).await;
        a.close().await;
        let got = stats(
            PgIntrospector
                .facet(
                    &mut c,
                    &ns,
                    &[kt.clone(), kv.clone()],
                    FacetKind::Statistics,
                )
                .await
                .unwrap_or_else(|e| panic!("{}: {e:?}", t.id)),
        );
        assert_eq!(
            got.len(),
            1,
            "{}: a view has no row estimate: {got:?}",
            t.id
        );
        assert!(got[0].0.same_object(&kt), "{}", t.id);
        assert_eq!(got[0].1.row_estimate, Some(1000), "{}", t.id);
        drop(c);
        pg.shutdown().await;
    }
}

// ---------------------------------------------------------------------------------------------
// Refused reads

/// A login that may not read `pg_roles` (a hardened database): the role list and the grants
/// (which name grantees through `pg_roles`) fail with `Permission`, never answer empty; the
/// dependencies, which do not read it, still answer.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn refused_reads_are_errors_never_empty_lists() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let db = unique("hvm4_t16b_db");
        let login = unique("hvm4_t16b_lim");
        // Dropped when it goes, also on panic: the database first, then the login.
        struct DropDb {
            t: DockerTarget,
            db: String,
            login: String,
        }
        impl Drop for DropDb {
            fn drop(&mut self) {
                let (t, db, login) = (self.t.clone(), self.db.clone(), self.login.clone());
                let rt = tokio::runtime::Handle::current();
                std::thread::scope(|s| {
                    s.spawn(|| {
                        rt.block_on(async {
                            let mut a = admin(&t).await;
                            let _ = a
                                .live()
                                .raw()
                                .batch(&format!("DROP DATABASE IF EXISTS {db} WITH (FORCE)"))
                                .await;
                            let _ = a
                                .live()
                                .raw()
                                .batch(&format!("DROP ROLE IF EXISTS {login}"))
                                .await;
                            a.close().await;
                        });
                    });
                });
            }
        }
        let _g = DropDb {
            t: t.clone(),
            db: db.clone(),
            login: login.clone(),
        };
        let mut a = admin(&t).await;
        a.batch(&format!(
            "CREATE ROLE {login} LOGIN PASSWORD '{}'",
            t.password.replace('\'', "''")
        ))
        .await;
        a.batch(&format!("CREATE DATABASE {db}")).await;
        a.close().await;
        let mut a = admin_on(&t, Some(&db)).await;
        a.batch(
            "CREATE SCHEMA s;
             CREATE TABLE s.t (a int);
             REVOKE SELECT ON pg_catalog.pg_roles FROM PUBLIC;",
        )
        .await;
        let oid: u32 = a
            .scalar("SELECT 's.t'::pg_catalog.regclass::pg_catalog.oid::text")
            .await
            .parse()
            .unwrap();
        a.close().await;
        let mut lim = t.clone();
        lim.user = login.clone();
        lim.database = db.clone();
        let pg = Pg::connect(&lim).await;
        let mut c = pg.meta().await;
        let ns = NsKey::schema([db.as_str(), "s"]);
        let kt = ObjectKey {
            kind: ObjectKind::Table,
            path: [db.as_str(), "s", "t"]
                .into_iter()
                .map(Name::from)
                .collect(),
            server_id: Some(ServerId::Oid(oid)),
            overload_sig: None,
            raw_kind: None,
        };
        let refused = PgIntrospector
            .facet(&mut c, &ns, std::slice::from_ref(&kt), FacetKind::Grants)
            .await;
        assert!(
            matches!(refused, Err(IntroError::Permission { .. })),
            "{}: {refused:?}",
            t.id
        );
        let all = hacivat_introspect::filter::parse_filter(
            "",
            sql_dialect::DialectId::Postgres,
            hacivat_introspect::filter::FilterRole::Introspect,
        )
        .unwrap();
        let cancel = CancellationToken::new();
        let roles = PgIntrospector
            .introspect(
                &mut c,
                &NsKey::server(),
                Mode::Full,
                Level::Names,
                &all,
                None,
                &cancel,
            )
            .await;
        assert!(
            matches!(
                roles,
                Err(IntroError::Permission {
                    kind: ObjectKind::Role
                })
            ),
            "{}: {roles:?}",
            t.id
        );
        let deps = PgIntrospector
            .facet(
                &mut c,
                &ns,
                std::slice::from_ref(&kt),
                FacetKind::Dependencies,
            )
            .await;
        match deps {
            Ok(FacetRows::Dependencies(v)) => {
                assert_eq!(v.len(), 1, "{}", t.id);
                assert!(
                    v[0].1
                        .depends_on
                        .iter()
                        .any(|x| x.kind == ObjectKind::Schema),
                    "{}: {v:?}",
                    t.id
                );
            }
            other => panic!("{}: {other:?}", t.id),
        }
        drop(c);
        pg.shutdown().await;
    }
}

// ---------------------------------------------------------------------------------------------
// Every section

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn every_pg_facet_section_prepares_and_returns_its_declared_columns() {
    if !docker_enabled() {
        return;
    }
    for t in targets() {
        let role = hostile_role();
        let f = Fixture::new(
            &t,
            "every",
            std::slice::from_ref(&role),
            "CREATE TABLE {s}.t (a int PRIMARY KEY, b int);
             CREATE FUNCTION {s}.f() RETURNS int LANGUAGE sql AS 'SELECT 1';
             CREATE TYPE {s}.mood AS ENUM ('sad', 'ok');
             GRANT SELECT (b) ON {s}.t TO {r0};",
        )
        .await;
        let ns = ns_of(&t, &f.schema);
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let n = load(&mut c, &ns).await;
        let ns_oid = oid_of(&key_of(&n, ObjectKind::Schema, ""));
        let (t_oid, f_oid, mood_oid) = (
            oid_of(&key_of(&n, ObjectKind::Table, "t")),
            oid_of(&key_of(&n, ObjectKind::Routine, "f")),
            oid_of(&key_of(&n, ObjectKind::EnumType, "mood")),
        );
        let cancel = CancellationToken::new();
        let version = pass::server_identity(&mut c, &cancel)
            .await
            .unwrap()
            .version;
        let p = sections::pack().unwrap();
        let mut snap = c.begin_snapshot().await.unwrap();
        for section in facets::name::ALL {
            let oids = match section {
                facets::name::GRANTS_ROUTINE => vec![f_oid],
                facets::name::GRANTS_TYPE => vec![mood_oid],
                facets::name::DEPENDENCIES => vec![t_oid, f_oid, mood_oid],
                _ => vec![t_oid],
            };
            let decl: Vec<String> = p
                .section(section)
                .unwrap()
                .returns()
                .iter()
                .map(|c| c.name.to_string())
                .collect();
            let mut ran = 0;
            let mut seen = BTreeSet::new();
            for v in p.reachable_variants(section) {
                let binds = Binds {
                    ns: Some(ns_oid),
                    w: None,
                    schema: None,
                    oids: Some(oids.clone()),
                };
                for q in p
                    .render(section, &ctx(version, v.mode, v.level, &binds))
                    .unwrap()
                {
                    if !seen.insert(q.sql().to_string()) {
                        continue;
                    }
                    let rows = snap
                        .rows(q.as_meta_query())
                        .await
                        .unwrap_or_else(|e| panic!("{} {section}: {e:?}", t.id));
                    let got: Vec<String> = rows.columns.iter().map(|c| c.to_string()).collect();
                    assert_eq!(got, decl, "{} {section}", t.id);
                    assert!(!rows.rows.is_empty(), "{} {section}: no row", t.id);
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
