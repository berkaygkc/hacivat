//! Golden DDL per dialect × kind × case (plan-m4 Task 19; R-M4-16). Bless with
//! `UPDATE_GOLDEN=1 cargo test -p hacivat-ddl --test golden` and review every file by hand;
//! CI fails when a golden changes without the flag (Task 31 `golden-guard`).
//!
//! Every PostgreSQL golden is also parsed by libpg_query (alg/03 HV-DDL-07 test 11): its
//! statement count must equal the script's.

mod fixtures;

use fixtures::*;
use hacivat_ddl::{DelimStyle, GenOptions, NoticeCode, Placement, Qualify, Script};
use hacivat_meta::ObjectKind as K;
use hacivat_proto::DbmsKind;
use sql_dialect::DialectId;

fn pg_case(rel: &str, s: &Script) {
    let text = s.text(DelimStyle::Semicolon);
    check_golden(rel, &text);
    let parsed = sql_oracle_pg::statement_count(&text)
        .unwrap_or_else(|e| panic!("{rel} does not parse: {e:?}\n{text}"));
    assert_eq!(parsed, s.stmts.len(), "{rel}: statement count");
}

fn has(s: &Script, code: NoticeCode) -> bool {
    s.notices.iter().any(|n| n.code == code)
}

// ---- PostgreSQL ---------------------------------------------------------------------------

#[test]
fn golden_pg_table_full() {
    let fx = pg();
    let s = fx.script(
        &[fx.key(K::Table, &["shop", "orders"])],
        &pg_target(),
        &GenOptions::default(),
    );
    pg_case("pg/table_full", &s);
    // Grants are loaded for `orders`; the identity sequence is implicit.
    assert!(!has(&s, NoticeCode::GrantsNotLoaded));
}

#[test]
fn golden_pg_zero_column_table() {
    let fx = pg();
    let s = fx.script(
        &[fx.key(K::Table, &["shop", "empty"])],
        &pg_target(),
        &GenOptions::default(),
    );
    pg_case("pg/zero_column_table", &s);
    assert!(
        s.text(DelimStyle::Semicolon)
            .starts_with("CREATE TABLE empty ();\n")
    );
}

#[test]
fn golden_pg_partitioned_parent_and_partition() {
    let fx = pg();
    let keys = [
        fx.key(K::Table, &["shop", "events_2026"]),
        fx.key(K::Table, &["shop", "events"]),
    ];
    let s = fx.script(
        &keys,
        &pg_target(),
        &GenOptions {
            skip_grants: true,
            ..Default::default()
        },
    );
    pg_case("pg/partitioned", &s);
}

#[test]
fn golden_pg_identity_vs_serial_origin() {
    let fx = pg();
    let keys = [
        fx.key(K::Table, &["shop", "tickets"]),
        fx.key(K::Sequence, &["shop", "invoice_no"]),
        fx.key(K::Table, &["shop", "orders"]),
    ];
    let s = fx.script(
        &keys,
        &pg_target(),
        &GenOptions {
            skip_grants: true,
            ..Default::default()
        },
    );
    pg_case("pg/serial_and_sequence", &s);
    let text = s.text(DelimStyle::Semicolon);
    assert!(
        !text.contains("orders_id_seq"),
        "identity sequences are implicit"
    );
}

#[test]
fn golden_pg_view_server_text() {
    let fx = pg();
    let keys = [
        fx.key(K::MaterializedView, &["shop", "order_totals"]),
        fx.key(K::View, &["shop", "active_orders"]),
    ];
    let s = fx.script(
        &keys,
        &pg_target(),
        &GenOptions {
            or_replace: true,
            skip_grants: true,
            ..Default::default()
        },
    );
    pg_case("pg/views_or_replace", &s);
}

#[test]
fn golden_pg_enum_domain_composite_range() {
    let fx = pg();
    let keys = [
        fx.key(K::CompositeType, &["shop", "address"]),
        fx.key(K::Domain, &["shop", "posint"]),
        fx.key(K::RangeType, &["shop", "floatrange"]),
        fx.key(K::EnumType, &["shop", "mood"]),
    ];
    let s = fx.script(&keys, &pg_target(), &GenOptions::default());
    pg_case("pg/types", &s);
}

#[test]
fn golden_pg_function_and_trigger_server_text() {
    let fx = pg();
    let keys = [
        fx.key(K::Table, &["shop", "items"]),
        fx.routine(K::Routine, &["shop", "touch"], ""),
        fx.routine(K::Routine, &["shop", "add_tax"], "numeric, numeric"),
    ];
    let s = fx.script(
        &keys,
        &pg_target(),
        &GenOptions {
            or_replace: true,
            skip_grants: true,
            ..Default::default()
        },
    );
    pg_case("pg/function_and_trigger", &s);
}

#[test]
fn golden_pg_turkish_and_mixed_case_identifiers_quoted() {
    let fx = pg();
    let s = fx.script(
        &[fx.key(K::Table, &["shop", "MÜŞTERİ"])],
        &pg_target(),
        &GenOptions {
            qualify: Qualify::Always,
            skip_grants: true,
            surrogate_names: true,
            ..Default::default()
        },
    );
    pg_case("pg/turkish_mixed_case", &s);
}

#[test]
fn golden_pg_fk_cycle_postponed() {
    let fx = pg();
    let keys = [
        fx.key(K::Table, &["shop", "b"]),
        fx.key(K::Table, &["shop", "a"]),
    ];
    let s = fx.script(
        &keys,
        &pg_target(),
        &GenOptions {
            skip_grants: true,
            ..Default::default()
        },
    );
    pg_case("pg/cycle_postponed", &s);
}

#[test]
fn golden_pg_inside_column() {
    let fx = pg();
    let s = fx.script(
        &[fx.key(K::Table, &["shop", "inline_keys"])],
        &pg_target(),
        &GenOptions {
            placement: Placement::InsideColumn,
            skip_grants: true,
            ..Default::default()
        },
    );
    pg_case("pg/inside_column", &s);
}

#[test]
fn golden_pg_after_table() {
    let fx = pg();
    let s = fx.script(
        &[fx.key(K::Table, &["shop", "inline_keys"])],
        &pg_target(),
        &GenOptions {
            placement: Placement::AfterTable,
            skip_grants: true,
            surrogate_names: true,
            ..Default::default()
        },
    );
    pg_case("pg/after_table", &s);
}

#[test]
fn golden_pg_unlogged_storage_if_not_exists() {
    let fx = pg();
    let s = fx.script(
        &[fx.key(K::Table, &["shop", "logs"])],
        &pg_target(),
        &GenOptions {
            if_not_exists: true,
            skip_grants: true,
            ..Default::default()
        },
    );
    pg_case("pg/unlogged_storage", &s);
}

#[test]
fn golden_pg_schema_expanded() {
    let fx = pg();
    let s = fx.script(
        &[fx.key(K::Schema, &["tiny"])],
        &pg_target(),
        &GenOptions {
            skip_grants: true,
            ..Default::default()
        },
    );
    pg_case("pg/schema_tiny", &s);
}

#[test]
fn golden_pg_qualify_never_skip_owner() {
    let fx = pg();
    let s = fx.script(
        &[fx.key(K::Table, &["shop", "orders"])],
        &pg_target(),
        &GenOptions {
            qualify: Qualify::Never,
            skip_grants: true,
            skip_owner: true,
            ..Default::default()
        },
    );
    pg_case("pg/qualify_never", &s);
}

// ---- MySQL --------------------------------------------------------------------------------

fn my() -> (Fx, hacivat_ddl::DdlTarget) {
    (mysql(DbmsKind::MySql), mysql_target(DialectId::MySql))
}

#[test]
fn golden_mysql_table_engine_charset_inline_comment_table_level_fk() {
    let (fx, t) = my();
    let s = fx.script(
        &[fx.key(K::Table, &["shop", "orders"])],
        &t,
        &GenOptions::default(),
    );
    check_golden("mysql/table_full", &s.text(DelimStyle::Semicolon));
    assert!(has(&s, NoticeCode::GrantsPartial));
}

#[test]
fn golden_mysql_after_table_keeps_the_auto_increment_key() {
    let (fx, t) = my();
    let s = fx.script(
        &[fx.key(K::Table, &["shop", "orders"])],
        &t,
        &GenOptions {
            placement: Placement::AfterTable,
            skip_grants: true,
            ..Default::default()
        },
    );
    check_golden("mysql/after_table", &s.text(DelimStyle::Semicolon));
}

fn my_server_text_keys(fx: &Fx) -> Vec<hacivat_meta::ObjectKey> {
    vec![
        fx.key(K::View, &["shop", "v_open"]),
        fx.key(K::Procedure, &["shop", "close_order"]),
        fx.key(K::Routine, &["shop", "tax"]),
    ]
}

#[test]
fn golden_mysql_view_and_routine_with_definer_and_skip_definer() {
    let (fx, t) = my();
    let keys = my_server_text_keys(&fx);
    let kept = fx.script(
        &keys,
        &t,
        &GenOptions {
            skip_grants: true,
            ..Default::default()
        },
    );
    check_golden(
        "mysql/view_routine_definer",
        &kept.text(DelimStyle::Semicolon),
    );
    check_golden(
        "mysql/view_routine_file",
        &kept.text(DelimStyle::MySqlDelimiter),
    );
    let skipped = fx.script(
        &keys,
        &t,
        &GenOptions {
            skip_definer: true,
            or_replace: true,
            skip_grants: true,
            ..Default::default()
        },
    );
    let text = skipped.text(DelimStyle::Semicolon);
    check_golden("mysql/view_routine_skip_definer", &text);
    assert!(!text.contains("DEFINER="));
}

#[test]
fn golden_mysql_executable_comment_flagged() {
    let (fx, t) = my();
    let s = fx.script(
        &[fx.key(K::View, &["shop", "v_hint"])],
        &t,
        &GenOptions {
            skip_grants: true,
            ..Default::default()
        },
    );
    check_golden("mysql/executable_comment", &s.text(DelimStyle::Semicolon));
    assert!(has(&s, NoticeCode::WarnExecutableComment));
}

#[test]
fn golden_mysql_reserved_and_turkish_names() {
    let (fx, t) = my();
    let s = fx.script(
        &[fx.key(K::Table, &["shop", "order"])],
        &t,
        &GenOptions {
            placement: Placement::InsideColumn,
            qualify: Qualify::Always,
            skip_grants: true,
            ..Default::default()
        },
    );
    check_golden("mysql/reserved_turkish", &s.text(DelimStyle::Semicolon));
}

#[test]
fn golden_mysql_no_backslash_escapes() {
    let (fx, mut t) = my();
    t.mysql_no_backslash_escapes = true;
    let s = fx.script(
        &[fx.key(K::Table, &["shop", "orders"])],
        &t,
        &GenOptions {
            skip_grants: true,
            ..Default::default()
        },
    );
    check_golden("mysql/no_backslash_escapes", &s.text(DelimStyle::Semicolon));
}

// ---- MariaDB ------------------------------------------------------------------------------

#[test]
fn golden_mariadb_table() {
    let fx = mysql(DbmsKind::MariaDb);
    let t = mysql_target(DialectId::MariaDb);
    let s = fx.script(
        &[fx.key(K::Table, &["shop", "orders"])],
        &t,
        &GenOptions {
            skip_grants: true,
            ..Default::default()
        },
    );
    check_golden("mariadb/table", &s.text(DelimStyle::Semicolon));
}

#[test]
fn golden_mariadb_sequence_and_or_replace() {
    let fx = mysql(DbmsKind::MariaDb);
    let t = mysql_target(DialectId::MariaDb);
    let keys = [
        fx.key(K::Sequence, &["shop", "invoice_seq"]),
        fx.key(K::Procedure, &["shop", "close_order"]),
        fx.key(K::Trigger, &["shop", "orders", "orders_bi"]),
        fx.key(K::View, &["shop", "v_open"]),
    ];
    let s = fx.script(
        &keys,
        &t,
        &GenOptions {
            or_replace: true,
            skip_definer: true,
            skip_grants: true,
            ..Default::default()
        },
    );
    check_golden(
        "mariadb/sequence_or_replace",
        &s.text(DelimStyle::MySqlDelimiter),
    );
}

// ---- SQLite -------------------------------------------------------------------------------

#[test]
fn golden_sqlite_table_view_trigger() {
    let fx = sqlite();
    let keys = [
        fx.key(K::View, &["main", "v_notes"]),
        fx.key(K::Table, &["main", "notes"]),
        fx.key(K::Table, &["main", "authors"]),
    ];
    let s = fx.script(&keys, &sqlite_target(), &GenOptions::default());
    let text = s.text(DelimStyle::Semicolon);
    check_golden("sqlite/table_full", &text);
}

#[test]
fn golden_sqlite_comments_never_emitted() {
    let fx = sqlite();
    let text = fx.text(
        &[fx.key(K::Table, &["main", "notes"])],
        &sqlite_target(),
        &GenOptions::default(),
    );
    assert!(!text.contains("never written"), "{text}");
    assert!(!text.contains("COMMENT"), "{text}");
}

#[test]
fn golden_sqlite_strict_without_rowid_generated_columns() {
    let fx = sqlite();
    let s = fx.script(
        &[fx.key(K::Table, &["main", "prices"])],
        &sqlite_target(),
        &GenOptions::default(),
    );
    check_golden(
        "sqlite/strict_without_rowid_generated",
        &s.text(DelimStyle::Semicolon),
    );
}

#[test]
fn golden_sqlite_cycle_forced_inside_with_notice() {
    let fx = sqlite();
    let s = fx.script(
        &[
            fx.key(K::Table, &["main", "b"]),
            fx.key(K::Table, &["main", "a"]),
        ],
        &sqlite_target(),
        &GenOptions {
            placement: Placement::AfterTable,
            ..Default::default()
        },
    );
    check_golden("sqlite/cycle_forced", &s.text(DelimStyle::Semicolon));
    assert!(has(&s, NoticeCode::SqlitePlacementForced));
    assert!(!s.text(DelimStyle::Semicolon).contains("ALTER TABLE"));
}

#[test]
fn golden_sqlite_attached_schema() {
    let fx = sqlite();
    let s = fx.script(
        &[fx.key(K::Table, &["aux", "kv"])],
        &sqlite_target(),
        &GenOptions::default(),
    );
    check_golden("sqlite/attached", &s.text(DelimStyle::Semicolon));
}

#[test]
fn golden_sqlite_inside_column() {
    let fx = sqlite();
    let s = fx.script(
        &[fx.key(K::Table, &["main", "notes"])],
        &sqlite_target(),
        &GenOptions {
            placement: Placement::InsideColumn,
            ..Default::default()
        },
    );
    check_golden("sqlite/inside_column", &s.text(DelimStyle::Semicolon));
}
