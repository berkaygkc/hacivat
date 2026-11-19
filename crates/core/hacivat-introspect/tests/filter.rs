//! Object filter grammar, view and scope roles, and bind-only pushdown (M4 Task 12; alg/02
//! HV-INTRO-03 test cases 1–8, edge cases 1–13; MSSQL rows excluded per R-M4-4). Test cases 9
//! and 10 need Docker (`lower_case_table_names = 1`, a tr-ICU database): their client half is
//! here; the server half is Task 17's and Task 28's.

use std::sync::Arc;
use std::time::SystemTime;

use hacivat_introspect::filter::{
    CompiledFilter, FilterError, FilterRole, FilterWarning, FilterWarningKind, MAX_PUSHDOWN_ITEMS,
    SqlPredicate, parse_filter,
};
use hacivat_introspect::pack::{DraftPack, Mode, PackCtx, PackFlags, Params};
use hacivat_introspect::scope::NsInfo;
use hacivat_meta::fixture::ns_pg;
use hacivat_meta::name::{IdentCtx, Name, NamePart};
use hacivat_meta::namespace::{Level, Watermark};
use hacivat_meta::snapshot::{Catalog, CatalogChange, ContainerKey, LoadState, ServerFacts};
use hacivat_meta::view::{Lookup, ModelView, ScopeFilter, SnapshotView, UnknownReason};
use hacivat_meta::{KindGroup, NsKey, ObjectKind};
use hacivat_proto::DbmsKind;
use hacivat_values::{BindValue, LogicalType};
use sql_dialect::DialectId;

const PG: DialectId = DialectId::Postgres;
const MY: DialectId = DialectId::MySql;
const LITE: DialectId = DialectId::Sqlite;

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

fn filter(s: &str) -> CompiledFilter {
    filter_on(s, PG)
}

fn filter_on(s: &str, d: DialectId) -> CompiledFilter {
    parse_filter(s, d, FilterRole::Introspect).unwrap_or_else(|e| panic!("{s:?} parses: {e:?}"))
}

fn ctx() -> IdentCtx {
    IdentCtx {
        encoding: Some(Arc::from("UTF8")),
        ..IdentCtx::new(PG)
    }
}

fn my(lctn: u8) -> IdentCtx {
    IdentCtx {
        lower_case_table_names: lctn,
        ..IdentCtx::new(MY)
    }
}

fn passes(f: &CompiledFilter, kind: ObjectKind, name: &str) -> bool {
    f.matches(kind, &n(name))
}

// ---- matching (test cases 1–4, edge cases 1–8) -------------------------------------------

#[test]
fn typed_filter_semantics() {
    // Test 1.
    let f = filter("table:-OLD_*||routine:DO*||view:V_*");
    assert!(passes(&f, ObjectKind::Table, "orders"));
    assert!(!passes(&f, ObjectKind::Table, "OLD_orders"));
    // PostgreSQL names are exact: `old_orders` is not `OLD_*`.
    assert!(passes(&f, ObjectKind::Table, "old_orders"));
    // `table` also covers partitions.
    assert!(!passes(&f, ObjectKind::Partition, "OLD_2020"));
    assert!(passes(&f, ObjectKind::Routine, "DO_work"));
    assert!(passes(&f, ObjectKind::Procedure, "DOIT"));
    assert!(!passes(&f, ObjectKind::Routine, "undo"));
    assert!(passes(&f, ObjectKind::View, "V_sales"));
    assert!(!passes(&f, ObjectKind::View, "sales"));
    // No item applies to sequences.
    assert!(passes(&f, ObjectKind::Sequence, "OLD_seq"));
    assert!(passes(&f, ObjectKind::Sequence, "anything"));
}

#[test]
fn prefix_glob_selects_only_that_kind() {
    // Test 2.
    let f = filter("table:actor*");
    assert!(passes(&f, ObjectKind::Table, "actor"));
    assert!(passes(&f, ObjectKind::Table, "actor_info"));
    assert!(!passes(&f, ObjectKind::Table, "film_actor"));
    assert!(passes(&f, ObjectKind::View, "film_actor"));
}

#[test]
fn untyped_filter_applies_to_every_kind() {
    // Test 3.
    let f = filter("*_tmp");
    for k in [
        ObjectKind::Table,
        ObjectKind::View,
        ObjectKind::Sequence,
        ObjectKind::Routine,
        ObjectKind::Unknown,
    ] {
        assert!(passes(&f, k, "x_tmp"), "{k:?}");
        assert!(!passes(&f, k, "x_tmp2"), "{k:?}");
    }
}

#[test]
fn regex_items_are_full_match() {
    // Test 4.
    let f = filter(r"table:{t\d+}");
    assert!(passes(&f, ObjectKind::Table, "t1"));
    assert!(passes(&f, ObjectKind::Table, "t22"));
    assert!(!passes(&f, ObjectKind::Table, "t1x"));
    assert!(!passes(&f, ObjectKind::Table, "at1"));
}

#[test]
fn underscore_and_percent_are_literal_in_globs() {
    // Edge 1.
    let f = filter("my_table||50%");
    assert!(passes(&f, ObjectKind::Table, "my_table"));
    assert!(!passes(&f, ObjectKind::Table, "myXtable"));
    assert!(passes(&f, ObjectKind::Table, "50%"));
    assert!(!passes(&f, ObjectKind::Table, "500"));
}

#[test]
fn escaped_star_is_literal() {
    // Edge 2.
    let f = filter(r"my\*table");
    assert!(passes(&f, ObjectKind::Table, "my*table"));
    assert!(!passes(&f, ObjectKind::Table, "myXtable"));
}

#[test]
fn double_bar_inside_a_regex_and_a_single_bar_in_a_glob() {
    // Edges 3 and 4.
    // `{a||b}` is one regex item (a, the empty name or b), then the glob `c|d`.
    let f = filter("{a||b}||c|d");
    assert!(passes(&f, ObjectKind::Table, "a"));
    assert!(passes(&f, ObjectKind::Table, "b"));
    assert!(passes(&f, ObjectKind::Table, "c|d"));
    assert!(!passes(&f, ObjectKind::Table, "a||b"));
    assert!(!passes(&f, ObjectKind::Table, "c"));
    assert!(!passes(&f, ObjectKind::Table, "d"));
    let f = filter("{a|b}");
    assert!(passes(&f, ObjectKind::Table, "a"));
    assert!(passes(&f, ObjectKind::Table, "b"));
    // A brace inside a glob is literal and never hides the separator.
    let f = filter("a{b||c}");
    assert!(passes(&f, ObjectKind::Table, "a{b"));
    assert!(passes(&f, ObjectKind::Table, "c}"));
    assert!(!passes(&f, ObjectKind::Table, "a{b||c}"));
}

#[test]
fn empty_items_are_ignored_with_a_warning() {
    // Edge 5.
    let f = filter("a||||b||");
    assert!(passes(&f, ObjectKind::Table, "a"));
    assert!(passes(&f, ObjectKind::Table, "b"));
    assert!(!passes(&f, ObjectKind::Table, "c"));
    assert_eq!(
        f.warnings(),
        &[
            FilterWarning {
                column: 4,
                kind: FilterWarningKind::EmptyItem
            },
            FilterWarning {
                column: 9,
                kind: FilterWarningKind::EmptyItem
            },
        ]
    );
    assert!(filter("").is_empty() && filter("  ").is_empty());
    assert!(passes(&filter(""), ObjectKind::Table, "anything"));
}

#[test]
fn only_negative_items_keep_everything_else() {
    // Edge 6.
    let f = filter("-tmp_*||-*_old");
    assert!(passes(&f, ObjectKind::Table, "orders"));
    assert!(!passes(&f, ObjectKind::Table, "tmp_1"));
    assert!(!passes(&f, ObjectKind::View, "v_old"));
}

#[test]
fn unknown_kind_token_is_an_error() {
    // Edge 7.
    assert_eq!(
        parse_filter("tabel:orders", PG, FilterRole::View),
        Err(FilterError::UnknownKind {
            item: "tabel:orders".into()
        })
    );
    // A kind the dialect does not have is unknown there.
    assert!(matches!(
        parse_filter("x||event:e*", PG, FilterRole::View),
        Err(FilterError::UnknownKind { item }) if item == "event:e*"
    ));
    assert!(parse_filter("event:e*", MY, FilterRole::View).is_ok());
    assert!(matches!(
        parse_filter("mview:m", MY, FilterRole::View),
        Err(FilterError::UnknownKind { .. })
    ));
    assert!(matches!(
        parse_filter("schema:s", PG, FilterRole::View),
        Err(FilterError::UnknownKind { .. })
    ));
    // A literal colon is escaped.
    assert!(passes(&filter(r"a\:b"), ObjectKind::Table, "a:b"));
}

#[test]
fn bad_regex_and_bad_glob_name_the_item() {
    assert_eq!(
        parse_filter("x||table:{(}", PG, FilterRole::View),
        Err(FilterError::BadRegex {
            item: "table:{(}".into()
        })
    );
    assert!(matches!(
        parse_filter("{a", PG, FilterRole::View),
        Err(FilterError::BadRegex { .. })
    ));
    assert!(matches!(
        parse_filter("{a}b", PG, FilterRole::View),
        Err(FilterError::BadRegex { .. })
    ));
    assert_eq!(
        parse_filter(" a[b ", PG, FilterRole::View),
        Err(FilterError::BadGlob { item: "a[b".into() })
    );
    for bad in ["table:", "-", r"x\"] {
        assert!(
            matches!(
                parse_filter(bad, PG, FilterRole::View),
                Err(FilterError::BadGlob { .. })
            ),
            "{bad:?}"
        );
    }
}

#[test]
fn kind_aliases() {
    // Edge 8.
    let f = filter("function:f*||mview:m*");
    assert!(passes(&f, ObjectKind::Routine, "f1"));
    assert!(passes(&f, ObjectKind::Aggregate, "f_sum"));
    assert!(!passes(&f, ObjectKind::Procedure, "p1"));
    assert!(passes(&f, ObjectKind::MaterializedView, "m1"));
    assert!(!passes(&f, ObjectKind::MaterializedView, "x1"));
    assert!(passes(&filter("PROCEDURE:p*"), ObjectKind::Routine, "p1"));
    assert!(passes(
        &filter("materialized_view:m*"),
        ObjectKind::MaterializedView,
        "m1"
    ));
}

#[test]
fn routine_signature_items_match_the_signature_form() {
    // Edge 9.
    let f = filter(r"routine:sig:{f\(integer\)}||routine:-g*");
    assert!(f.matches_signature(ObjectKind::Routine, "f", "f(integer)"));
    assert!(!f.matches_signature(ObjectKind::Routine, "f", "f(text)"));
    assert!(!f.matches_signature(ObjectKind::Routine, "g", "g(integer)"));
    // Without a signature the item is skipped; a scope filter is then unsure.
    assert!(f.matches(ObjectKind::Routine, &n("f")));
    assert!(f.excludes(Some(ObjectKind::Routine), Some("f")));
    assert!(
        f.pushdown(ObjectKind::Routine, PG, &ctx()).is_none(),
        "a signature item is never pushed"
    );
}

#[test]
fn scope_filters_never_apply_to_system_namespaces() {
    // Edge 10: built-in types and functions come from the snapshots.
    let sys = NsInfo {
        key: NsKey::schema(["db", "pg_catalog"]),
        system: true,
    };
    let user = NsInfo {
        key: NsKey::schema(["db", "public"]),
        system: false,
    };
    let scope = filter("-*");
    assert!(!scope.applies_to(&sys));
    assert!(scope.applies_to(&user));
    let view = parse_filter("-*", PG, FilterRole::View).unwrap();
    assert!(view.applies_to(&sys));
}

#[test]
fn turkish_names_follow_the_server_rule_never_locale_folding() {
    // Edge 11: PostgreSQL is exact; SQLite folds ASCII only; MySQL caseless classes too.
    let f = filter("*fiş*");
    assert!(passes(&f, ObjectKind::Table, "x_fiş_y"));
    assert!(!passes(&f, ObjectKind::Table, "X_FİŞ_Y"));
    assert!(!passes(&f, ObjectKind::Table, "FIŞ"));
    let s = filter_on("*fiş*", LITE);
    assert!(passes(&s, ObjectKind::Table, "FIş"));
    assert!(!passes(&s, ObjectKind::Table, "FİŞ"));
    assert!(!passes(&s, ObjectKind::Table, "fış"));
    let m = filter_on("column:i*", MY);
    assert!(passes(&m, ObjectKind::Column, "ID"));
    assert!(!passes(&m, ObjectKind::Column, "İD"));
    assert!(!passes(&m, ObjectKind::Column, "ıd"));
}

#[test]
fn mysql_table_case_follows_lower_case_table_names() {
    // Test 9's client half: `table:items` against `Items`.
    let f0 = filter_on("table:items", MY).with_ident(my(0));
    assert!(!passes(&f0, ObjectKind::Table, "Items"));
    let f1 = filter_on("table:items", MY).with_ident(my(1));
    assert!(passes(&f1, ObjectKind::Table, "Items"));
    // Routine and column names fold whatever the setting.
    assert!(passes(
        &filter_on("routine:do*", MY).with_ident(my(0)),
        ObjectKind::Routine,
        "DoIt"
    ));
}

#[test]
fn pg_quoted_upper_case_table_needs_the_exact_filter() {
    // Test 10's client half: `ITEMS` created quoted.
    assert!(!passes(&filter("table:items"), ObjectKind::Table, "ITEMS"));
    assert!(passes(&filter("table:ITEMS"), ObjectKind::Table, "ITEMS"));
}

// ---- roles (test cases 7 and 8) -------------------------------------------------------------

/// The engine's scope-filter adapter (Task 24 writes the real one): the profile's scope
/// filter for every namespace.
struct ScopeOf(CompiledFilter);

impl ScopeFilter for ScopeOf {
    fn excludes(&self, _: &NsKey, kind: Option<ObjectKind>, name: Option<&str>) -> bool {
        self.0.excludes(kind, name)
    }
}

/// `public` loaded at `Structure` with the tables of `names` that `load` lets through (what an
/// introspection pass under that scope filter reads).
fn snapshot_after_load(load: &CompiledFilter, names: &[&str]) -> SnapshotView {
    let db = "db";
    let mut ns = ns_pg(db, "public");
    let mut oid = 16384;
    for t in names {
        if load.matches(ObjectKind::Table, &n(t)) {
            ns = ns.table_oid(t, oid, &["id"]);
            oid += 1;
        }
    }
    let ns = ns.finish();
    let catalog = Catalog::new(Arc::new(ServerFacts::new(DbmsKind::Postgres, ctx())));
    let publisher = catalog.publisher().unwrap();
    publisher
        .publish(vec![
            CatalogChange::SetState(
                ContainerKey::Namespace(ns.key().clone()),
                LoadState::Loaded {
                    level: Level::Structure,
                    at: SystemTime::UNIX_EPOCH,
                    watermark: Watermark::None,
                    stale: false,
                },
            ),
            CatalogChange::Put(Arc::new(ns)),
            CatalogChange::SetState(
                ContainerKey::schema_list(Some(&n(db))),
                LoadState::Loaded {
                    level: Level::Names,
                    at: SystemTime::UNIX_EPOCH,
                    watermark: Watermark::None,
                    stale: false,
                },
            ),
        ])
        .unwrap();
    SnapshotView::new(catalog.current()).with_search_path(
        hacivat_meta::search_path::SearchPath::exact(Some(n(db)), [NsKey::schema([db, "public"])]),
    )
}

fn lookup(v: &SnapshotView, name: &str) -> Lookup<String> {
    match v.lookup(
        &[NamePart {
            text: n(name),
            quoted: false,
        }],
        KindGroup::Relations,
    ) {
        Lookup::Found(h) => Lookup::Found(h.obj().name().to_string()),
        Lookup::Missing(r) => Lookup::Missing(r),
        Lookup::Unknown(r) => Lookup::Unknown(r),
    }
}

#[test]
fn view_filter_hides_but_scope_filter_unloads() {
    let names = ["audit_log", "orders"];
    // Test 7: a view filter ("hide like this") hides `audit_*` in the explorer, but the
    // objects are loaded, and the view filter never reaches the lookup view.
    let hide = parse_filter("table:-audit_*", PG, FilterRole::View).unwrap();
    assert!(!hide.matches(ObjectKind::Table, &n("audit_log")));
    assert!(!hide.excludes(Some(ObjectKind::Table), Some("audit_log")));
    assert!(hide.pushdown(ObjectKind::Table, PG, &ctx()).is_none());
    let everything = filter("");
    let v =
        snapshot_after_load(&everything, &names).with_scope_filter(Arc::new(ScopeOf(hide.clone())));
    assert_eq!(lookup(&v, "audit_log"), Lookup::Found("audit_log".into()));
    assert!(matches!(lookup(&v, "audit_trail"), Lookup::Missing(_)));
    // Test 8: the same text as a scope filter leaves `audit_*` unread, so completion says
    // `Unknown(OutOfScope)` (no "unknown table" diagnostic), never `Missing`.
    let scope = filter("table:-audit_*");
    let v = snapshot_after_load(&scope, &names).with_scope_filter(Arc::new(ScopeOf(scope)));
    assert_eq!(
        lookup(&v, "audit_log"),
        Lookup::Unknown(UnknownReason::OutOfScope)
    );
    assert_eq!(
        lookup(&v, "AUDIT_LOG"),
        Lookup::Unknown(UnknownReason::OutOfScope)
    );
    assert_eq!(lookup(&v, "orders"), Lookup::Found("orders".into()));
    assert!(matches!(lookup(&v, "invoices"), Lookup::Missing(_)));
}

#[test]
fn excludes_is_unsure_in_the_safe_direction() {
    let f = filter("table:keep_*");
    assert!(f.excludes(Some(ObjectKind::Table), Some("other")));
    assert!(!f.excludes(Some(ObjectKind::Table), Some("keep_1")));
    // The probe as typed may be folded by the server: any spelling that fails counts.
    assert!(f.excludes(Some(ObjectKind::Table), Some("KEEP_1")));
    assert!(f.excludes(Some(ObjectKind::Table), None));
    assert!(!f.excludes(Some(ObjectKind::Sequence), Some("other")));
    assert!(f.excludes(None, Some("other")));
    assert!(!filter("").excludes(None, None));
    assert!(filter("-x*").excludes(None, Some("X1")));
}

// ---- pushdown (test cases 5 and 6, edge 12) ---------------------------------------------------

#[test]
fn pg_pushdown_ord_star_binds_ord_percent() {
    // Test 5.
    let p = filter("table:ord*")
        .pushdown(ObjectKind::Table, PG, &ctx())
        .unwrap();
    assert!(p.sql_fragment.contains("LIKE") && p.sql_fragment.contains("ESCAPE"));
    assert_eq!(p.binds, [text("ord%")]);
    // The client matcher still decides.
    assert!(!filter("table:ord*").matches(ObjectKind::Table, &n("xord")));
}

#[test]
fn pg_pushdown_escapes_underscore() {
    // Test 6.
    assert_eq!(
        filter("table:a_b")
            .pushdown(ObjectKind::Table, PG, &ctx())
            .unwrap()
            .binds,
        [text(r"a\_b")]
    );
    assert_eq!(
        filter(r"table:50%\\?[xy]*")
            .pushdown(ObjectKind::Table, PG, &ctx())
            .unwrap()
            .binds,
        [text(r"50\%\\__%")]
    );
}

#[test]
fn positive_items_become_one_bind_each() {
    let p = filter("table:a*||table:b?||*_x||table:a*||view:v*")
        .pushdown(ObjectKind::Table, PG, &ctx())
        .unwrap();
    assert_eq!(p.binds, [text("a%"), text("b_"), text(r"%\_x")]);
    let runs: Vec<_> = p.runs().collect();
    assert_eq!(runs.len(), 3);
    assert_eq!(
        runs[1],
        vec![
            (n(SqlPredicate::PATTERN_BIND), text("b_")),
            (n(SqlPredicate::ESCAPE_BIND), text("\\")),
        ]
    );
}

#[test]
fn negative_and_regex_items_are_never_pushed() {
    let f = filter("table:-tmp_*||table:{t\\d+}||table:ord*");
    // A positive regex the server cannot apply: no pushdown at all (the answer must be a
    // superset).
    assert!(f.pushdown(ObjectKind::Table, PG, &ctx()).is_none());
    let f = filter("table:-tmp_*||table:ord*");
    assert_eq!(
        f.pushdown(ObjectKind::Table, PG, &ctx()).unwrap().binds,
        [text("ord%")]
    );
    // Only negatives, or nothing applies: nothing to push.
    assert!(
        filter("-tmp_*")
            .pushdown(ObjectKind::Table, PG, &ctx())
            .is_none()
    );
    assert!(
        filter("view:v*")
            .pushdown(ObjectKind::Table, PG, &ctx())
            .is_none()
    );
    // A match-everything glob cuts nothing.
    assert!(
        filter("table:ord*||*")
            .pushdown(ObjectKind::Table, PG, &ctx())
            .is_none()
    );
}

#[test]
fn case_insensitive_items_never_pushed_on_pg() {
    // Every PostgreSQL name class is exact, so a pushed item compares as the client does
    // (`C` collation); the fragment never folds case.
    let p = filter("table:Ord*")
        .pushdown(ObjectKind::Table, PG, &ctx())
        .unwrap();
    assert_eq!(p.binds, [text("Ord%")]);
    for word in ["ILIKE", "LOWER", "UPPER", "COLLATE"] {
        assert!(
            !p.sql_fragment.to_ascii_uppercase().contains(word),
            "{word}"
        );
    }
    // A PostgreSQL fork whose rule is not pinned (Redshift folds quoted names, CockroachDB
    // folds Unicode) gets nothing pushed: the item's case cannot be matched to the server's.
    for fork in [
        sql_dialect::PgWireFlavor::Redshift,
        sql_dialect::PgWireFlavor::Cockroach,
    ] {
        let d = DialectId::PgWire(fork);
        let f = parse_filter("table:ord*", d, FilterRole::Introspect).unwrap();
        assert!(
            f.pushdown(ObjectKind::Table, d, &IdentCtx::new(d))
                .is_none()
        );
    }
    // A filter compiled for one dialect never pushes to another's query.
    assert!(
        filter("table:ord*")
            .pushdown(ObjectKind::Table, MY, &my(0))
            .is_none()
    );
}

#[test]
fn pg_non_ascii_pattern_needs_a_utf8_server() {
    let f = filter("table:fiş*");
    assert_eq!(
        f.pushdown(ObjectKind::Table, PG, &ctx()).unwrap().binds,
        [text("fiş%")]
    );
    let latin1 = IdentCtx {
        encoding: Some(Arc::from("LATIN1")),
        ..IdentCtx::new(PG)
    };
    assert!(f.pushdown(ObjectKind::Table, PG, &latin1).is_none());
    assert!(
        f.pushdown(ObjectKind::Table, PG, &IdentCtx::new(PG))
            .is_none()
    );
    assert!(
        filter("table:ord*")
            .pushdown(ObjectKind::Table, PG, &latin1)
            .is_some()
    );
}

#[test]
fn mysql_lctn1_pushdown_allowed_for_table_names_only() {
    let f = filter_on(
        "table:Ord*||view:V*||routine:do*||column:c*||trigger:t*",
        MY,
    );
    for lctn in [0, 1, 2] {
        // The filter compares by the server's rule (`with_ident`); pushdown refuses another.
        let f = f.clone().with_ident(my(lctn));
        assert!(
            f.pushdown(ObjectKind::Table, MY, &my(lctn)).is_some(),
            "lctn {lctn}"
        );
        assert!(f.pushdown(ObjectKind::View, MY, &my(lctn)).is_some());
        assert!(f.pushdown(ObjectKind::Routine, MY, &my(lctn)).is_none());
        assert!(f.pushdown(ObjectKind::Column, MY, &my(lctn)).is_none());
        assert!(f.pushdown(ObjectKind::Trigger, MY, &my(lctn)).is_none());
    }
    // MariaDB shares the rule; a character outside the BMP never reaches `utf8mb3`.
    assert!(
        f.pushdown(
            ObjectKind::Table,
            DialectId::MariaDb,
            &IdentCtx::new(DialectId::MariaDb)
        )
        .is_some()
    );
    assert!(
        filter_on("table:😀*", MY)
            .pushdown(ObjectKind::Table, MY, &my(0))
            .is_none()
    );
    assert!(
        filter_on("table:ş*", MY)
            .pushdown(ObjectKind::Table, MY, &my(0))
            .is_some()
    );
}

#[test]
fn a_multi_kind_section_gets_the_union_only_when_every_kind_is_pushable() {
    // Fix round 1, I2: a `pg_class` section returns tables and views together.
    let relations = [ObjectKind::Table, ObjectKind::View];
    let f = filter("table:ord*");
    assert!(f.pushdown(ObjectKind::Table, PG, &ctx()).is_some());
    assert!(f.pushdown_kinds(&relations, PG, &ctx()).is_none());
    let f = filter("table:ord*||view:v_*||table:v_*||*_tmp");
    assert_eq!(
        f.pushdown_kinds(&relations, PG, &ctx()).unwrap().binds,
        [text("ord%"), text(r"v\_%"), text(r"%\_tmp")]
    );
    // `table` covers partitions, so a section returning both may push.
    assert!(
        filter("table:ord*")
            .pushdown_kinds(&[ObjectKind::Table, ObjectKind::Partition], PG, &ctx())
            .is_some()
    );
    assert!(
        filter("table:ord*")
            .pushdown_kinds(&[], PG, &ctx())
            .is_none()
    );
    // The 200-pattern cap counts the union.
    let items = (0..150)
        .map(|i| format!("table:t{i}_*||view:v{i}_*"))
        .collect::<Vec<_>>()
        .join("||");
    assert!(
        filter(&items)
            .pushdown(ObjectKind::Table, PG, &ctx())
            .is_some()
    );
    assert!(
        filter(&items)
            .pushdown_kinds(&relations, PG, &ctx())
            .is_none()
    );
}

#[test]
fn sqlite_non_ascii_pattern_is_not_pushed() {
    let ident = IdentCtx::new(LITE);
    assert_eq!(
        filter_on("table:ord*", LITE)
            .pushdown(ObjectKind::Table, LITE, &ident)
            .unwrap()
            .binds,
        [text("ord%")]
    );
    assert!(
        filter_on("table:fiş*", LITE)
            .pushdown(ObjectKind::Table, LITE, &ident)
            .is_none()
    );
    assert!(
        filter_on("table:ord*||table:fiş*", LITE)
            .pushdown(ObjectKind::Table, LITE, &ident)
            .is_none()
    );
}

#[test]
fn over_200_items_no_pushdown() {
    // Edge 12.
    let items = |count: usize| {
        (0..count)
            .map(|i| format!("table:t{i}_*"))
            .collect::<Vec<_>>()
            .join("||")
    };
    let at = filter(&items(MAX_PUSHDOWN_ITEMS));
    assert_eq!(
        at.pushdown(ObjectKind::Table, PG, &ctx())
            .unwrap()
            .binds
            .len(),
        MAX_PUSHDOWN_ITEMS
    );
    let over = filter(&items(MAX_PUSHDOWN_ITEMS + 1));
    assert!(over.pushdown(ObjectKind::Table, PG, &ctx()).is_none());
    assert!(over.matches(ObjectKind::Table, &n("t200_x")));
    assert!(!over.matches(ObjectKind::Table, &n("t201_x")));
}

#[test]
fn view_filters_are_never_pushed() {
    let v = parse_filter("table:ord*", PG, FilterRole::View).unwrap();
    assert!(v.pushdown(ObjectKind::Table, PG, &ctx()).is_none());
}

#[test]
fn nul_is_never_sent() {
    let f = filter("table:a\u{0}b*");
    assert!(f.pushdown(ObjectKind::Table, PG, &ctx()).is_none());
    assert!(f.matches(ObjectKind::Table, &n("a\u{0}bc")));
}

#[test]
fn hostile_pattern_is_a_bind_never_sql() {
    let hostile = "x'; drop table t; --";
    let f = filter(&format!("table:{hostile}*||table:$1*||table:?*"));
    let p = f.pushdown(ObjectKind::Table, PG, &ctx()).unwrap();
    assert_eq!(p.sql_fragment, SqlPredicate::FRAGMENT);
    assert!(!p.sql_fragment.contains("drop") && !p.sql_fragment.contains('\''));
    assert_eq!(
        p.binds,
        [text("x'; drop table t; --%"), text("$1%"), text("_%")]
    );
    // Rendered through a pack section, the text holds placeholders only: the hostile text
    // reaches the server as bind values, on every dialect.
    for (d, pack, marks) in [
        (
            PG,
            "--- name: T ---\n--- returns: name text ---\n\
             SELECT c.relname FROM pg_catalog.pg_class c\n\
             WHERE c.relnamespace = :ns\n  AND c.relname ",
            ["$1", "$2", "$3"],
        ),
        (
            MY,
            "--- name: T ---\n--- returns: name text ---\n\
             SELECT TABLE_NAME FROM INFORMATION_SCHEMA.TABLES\n\
             WHERE TABLE_SCHEMA = :ns\n  AND TABLE_NAME ",
            ["?", "?", "?"],
        ),
        (
            LITE,
            "--- name: T ---\n--- returns: name text ---\n\
             SELECT name FROM sqlite_schema\n\
             WHERE type = :ns\n  AND name ",
            ["?", "?", "?"],
        ),
    ] {
        let f = filter_on(&format!("table:{hostile}*"), d);
        let ident = match d {
            DialectId::MySql => my(0),
            _ => ctx(),
        };
        let ident = IdentCtx {
            dialect: d,
            ..ident
        };
        let p = f
            .pushdown(ObjectKind::Table, d, &ident)
            .unwrap_or_else(|| panic!("{d:?} pushes"));
        let draft = DraftPack::parse(d, &format!("{pack}{}\n", p.sql_fragment))
            .unwrap_or_else(|e| panic!("{d:?}: the fragment is pack text: {e:?}"));
        for mut run in p.runs() {
            run.insert(0, (n("ns"), text("public")));
            let q = draft.render("T", &pack_ctx(d, run)).unwrap();
            let sql = q[0].sql();
            assert!(!sql.contains("drop"), "{d:?}: {sql}");
            for m in marks {
                assert!(sql.contains(m), "{d:?}: {sql}");
            }
            assert_eq!(q[0].binds()[1], text("x'; drop table t; --%"));
            assert_eq!(q[0].binds()[2], text("\\"));
        }
    }
}

#[test]
fn pushdown_composes_with_a_list_bind_conjunct() {
    // R-W1-7: a list bind is a top-level AND conjunct `<col> IN :[x]`, once per section; the
    // pushdown conjunct sits beside it and the pack's list-bind checks accept the section.
    let pack = format!(
        "--- name: T ---\n--- returns: name text ---\n\
         SELECT c.relname FROM pg_catalog.pg_class c\n\
         WHERE c.oid IN :[oids]\n  AND c.relname {}\n",
        SqlPredicate::FRAGMENT
    );
    let draft = DraftPack::parse(PG, &pack).expect("a list bind and the fragment parse");
    let p = filter("table:ord*")
        .pushdown(ObjectKind::Table, PG, &ctx())
        .unwrap();
    let run = p.runs().next().unwrap();
    let mut c = pack_ctx(PG, run);
    c.params.lists = vec![(
        n("oids"),
        (1..=1500).map(|i| text(&i.to_string())).collect(),
    )];
    let qs = draft.render("T", &c).unwrap();
    assert_eq!(qs.len(), 2, "chunked at 1,000");
    for q in &qs {
        assert!(q.sql().contains("LIKE") && q.binds().contains(&text("ord%")));
    }
}

fn pack_ctx(d: DialectId, scalars: Vec<(Name, BindValue)>) -> PackCtx {
    PackCtx {
        dialect: d,
        version: 0,
        flavor: match d {
            DialectId::MySql => DbmsKind::MySql,
            DialectId::Sqlite => DbmsKind::Sqlite,
            _ => DbmsKind::Postgres,
        },
        mode: Mode::Full,
        level: Level::Structure,
        flags: PackFlags::empty(),
        params: Params {
            scalars,
            lists: Vec::new(),
        },
    }
}
