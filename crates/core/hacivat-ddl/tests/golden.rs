//! Golden DDL per dialect × kind × case (plan-m4 Task 19; R-M4-16). Bless with
//! `UPDATE_GOLDEN=1 cargo test -p hacivat-ddl --test golden` and review every file by hand;
//! CI fails when a golden changes without the flag (Task 31 `golden-guard`). The cases live in
//! `fixtures::CASES`, so the byte-stability tests cover every golden.

mod fixtures;

use fixtures::*;
use hacivat_ddl::produce::{
    GRANTS_NOT_LOADED, GRANTS_PARTIAL, POLICIES_NOT_LOADED, SQLITE_PLACEMENT_FORCED,
    UNSUPPORTED_KIND, WARN_EXECUTABLE_COMMENT,
};
use hacivat_ddl::script::DelimStyle;
use hacivat_ddl::{DdlError, GenOptions};
use hacivat_proto::DbmsKind;
use sql_dialect::DialectId;

fn golden(rel: &str) -> String {
    let text = case_text(rel);
    check_golden(rel, &text);
    text
}

// ---- PostgreSQL ----------------------------------------------------------------------------------

#[test]
fn golden_pg_table_full() {
    let text = golden("pg/table_full");
    let fx = pg();
    let s = fx.script(
        &[fx.key(K::Table, &["shop", "orders"])],
        &pg_target(),
        &opts(),
    );
    // Grants are loaded for `orders`; the identity sequence and the key indexes are implicit.
    assert!(!s.has_notice(GRANTS_NOT_LOADED));
    for absent in ["orders_id_seq", "INDEX orders_pkey", "INDEX orders_code_uq"] {
        assert!(!text.contains(absent), "{absent} must not be emitted");
    }
    // `customers` exists in `shop` and `sales`: the reference is qualified.
    assert!(text.contains("REFERENCES shop.customers (id)"));
    assert!(text.contains("CREATE TABLE orders ("));
}

#[test]
fn golden_pg_zero_column_table() {
    let text = golden("pg/zero_column_table");
    assert_eq!(text, "CREATE TABLE empty ();\n");
}

#[test]
fn golden_pg_partitioned_parent_and_partition() {
    let text = golden("pg/partitioned");
    let parent = text.find("CREATE TABLE events (").expect("parent");
    let child = text.find("CREATE TABLE events_2026").expect("partition");
    assert!(parent < child, "the parent comes first");
}

#[test]
fn golden_pg_inherited_columns_skipped() {
    let text = golden("pg/partition_inherited_skipped");
    assert!(
        !text.contains("payload jsonb"),
        "inherited columns are not repeated"
    );
    assert!(
        !text.contains("PRIMARY KEY"),
        "the inherited key is not repeated"
    );
    // An `INHERITS` child: the model has no parent list yet, so no script rather than a
    // table without its parents' columns.
    let fx = pg();
    let r = fx.try_script(
        &[fx.key(K::Table, &["sales", "child_t"])],
        &pg_target(),
        &no_grants(),
    );
    assert_eq!(r, Err(DdlError::NotLoaded));
}

#[test]
fn golden_pg_identity_vs_serial_origin() {
    let text = golden("pg/identity_vs_serial");
    assert!(
        !text.contains("orders_id_seq"),
        "identity sequences are implicit"
    );
    let seq = text
        .find("CREATE SEQUENCE tickets_id_seq")
        .expect("serial sequence");
    let table = text.find("CREATE TABLE tickets").expect("table");
    let owned = text
        .find("ALTER SEQUENCE tickets_id_seq OWNED BY tickets.id")
        .expect("owned by");
    assert!(seq < table && table < owned);
}

#[test]
fn golden_pg_view_server_text() {
    golden("pg/views_or_replace");
}

#[test]
fn golden_pg_expression_partial_include_index_from_server_text() {
    let text = golden("pg/indexes_server_text");
    assert!(
        text.contains("(lower(note) text_pattern_ops)"),
        "operator class kept"
    );
}

#[test]
fn golden_pg_enum_domain_composite_range() {
    golden("pg/types");
}

#[test]
fn golden_pg_function_and_trigger_server_text() {
    let text = golden("pg/function_and_trigger");
    assert!(text.contains("CREATE OR REPLACE TRIGGER items_touch"));
}

#[test]
fn golden_pg_turkish_and_mixed_case_identifiers_quoted() {
    let text = golden("pg/turkish_mixed_case");
    for q in [
        "\"MÜŞTERİ\"",
        "\"Ad\"",
        "\"order\"",
        "\"müşteri_no\"",
        "\"ılık\"",
        "\"MÜŞTERİ_pkey\"",
    ] {
        assert!(text.contains(q), "{q} is quoted");
    }
}

#[test]
fn golden_pg_always_qualified() {
    let text = golden("pg/schema_qualify_always");
    assert!(text.contains("CREATE TABLE shop.orders ("));
    assert!(!text.contains("OWNER TO"));
}

#[test]
fn row_security_table_emits_enable_and_policies_notice() {
    let text = golden("pg/row_security");
    assert!(text.contains("ALTER TABLE secrets ENABLE ROW LEVEL SECURITY;"));
    let fx = pg();
    let s = fx.script(
        &[fx.key(K::Table, &["shop", "secrets"])],
        &pg_target(),
        &no_grants(),
    );
    assert!(s.has_notice(POLICIES_NOT_LOADED));
}

// ---- MySQL / MariaDB -------------------------------------------------------------------------

#[test]
fn golden_mysql_table_engine_charset_inline_comment_table_level_fk() {
    let text = golden("mysql/table_full");
    let fx = mysql(DbmsKind::MySql);
    let s = fx.script(
        &[fx.key(K::Table, &["shop", "orders"])],
        &mysql_target(DialectId::MySql),
        &opts(),
    );
    assert!(s.has_notice(GRANTS_PARTIAL));
    assert!(
        text.contains(
            "    CONSTRAINT fk_customer FOREIGN KEY (customer_id) REFERENCES customers (id)"
        ),
        "FK is a table element"
    );
    assert!(
        !text.contains("INDEX `fk_customer`"),
        "the FK's own index is implicit"
    );
    assert!(text.contains("CREATE UNIQUE INDEX note_uq ON orders (note(20))"));
    assert!(text.contains("INVISIBLE"));
}

#[test]
fn golden_mysql_fk_after_an_index_of_the_same_name() {
    let text = golden("mysql/fk_after_same_named_index");
    let ix = text.find("CREATE INDEX lines_order").expect("index");
    let fk = text
        .find("ADD CONSTRAINT lines_order FOREIGN KEY")
        .expect("fk");
    assert!(ix < fk);
}

#[test]
fn golden_mysql_view_and_routine_with_definer_and_skip_definer() {
    let kept = golden("mysql/view_routine_definer");
    assert!(kept.contains("DEFINER=`app`@`%` PROCEDURE"));
    let skipped = golden("mysql/view_routine_skip_definer");
    assert!(!skipped.contains("DEFINER=`"), "definers removed");
    assert!(skipped.contains("CREATE OR REPLACE ALGORITHM=UNDEFINED"));
    assert!(!skipped.contains("OR REPLACE PROCEDURE"), "MySQL has none");
    let file = golden("mysql/view_routine_file");
    assert!(file.contains("DELIMITER ;;\n"));
}

#[test]
fn golden_mysql_executable_comment_flagged() {
    let text = golden("mysql/executable_comment");
    assert!(
        text.contains("/*!50017 DEFINER=`root`@`%`*/"),
        "kept verbatim"
    );
    let fx = mysql(DbmsKind::MySql);
    let s = fx.script(
        &[fx.key(K::Table, &["shop", "audit"])],
        &mysql_target(DialectId::MySql),
        &no_grants(),
    );
    assert!(s.has_notice(WARN_EXECUTABLE_COMMENT));
    let s = fx.script(
        &[fx.key(K::Table, &["shop", "customers"])],
        &mysql_target(DialectId::MySql),
        &no_grants(),
    );
    assert!(!s.has_notice(WARN_EXECUTABLE_COMMENT));
}

#[test]
fn golden_mysql_server_definition_is_show_create_text() {
    let text = golden("mysql/server_definition");
    assert!(text.starts_with("CREATE TABLE `orders` (\n  `id` bigint unsigned"));
}

#[test]
fn golden_mariadb_table() {
    let text = golden("mariadb/table");
    assert!(text.contains("COLLATE=utf8mb4_uca1400_ai_ci"));
    assert!(text.contains("IGNORED"), "MariaDB's invisible index word");
    let seq = golden("mariadb/sequence_or_replace");
    assert!(seq.contains("CREATE OR REPLACE PROCEDURE"));
}

#[test]
fn golden_mariadb_vs_mysql_defaults_verbatim() {
    let my = case_text("mysql/table_full");
    let maria = case_text("mariadb/table");
    assert!(my.contains("DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP"));
    assert!(maria.contains("DEFAULT current_timestamp() ON UPDATE current_timestamp()"));
}

// ---- SQLite ----------------------------------------------------------------------------------------

#[test]
fn golden_sqlite_server_text_verbatim_with_strict_without_rowid_generated_and_check() {
    let text = golden("sqlite/server_text");
    for verbatim in [
        ") STRICT, WITHOUT ROWID;",
        "GENERATED ALWAYS AS (length(body) - length(replace(body, ' ', '')) + 1) VIRTUAL",
        "CHECK (length(body) < 1000)",
        "name TEXT NOT NULL COLLATE NOCASE UNIQUE",
    ] {
        assert!(text.contains(verbatim), "{verbatim}");
    }
    assert!(
        !text.contains("sqlite_autoindex"),
        "autoindexes are implicit"
    );
    let authors = text.find("CREATE TABLE authors").expect("authors");
    let notes = text.find("CREATE TABLE notes").expect("notes");
    assert!(authors < notes, "FK order");
}

#[test]
fn golden_sqlite_comments_never_emitted() {
    let text = golden("sqlite/schema_expanded");
    assert!(!text.contains("COMMENT"));
    assert!(!text.contains("annotation") && !text.contains("Never written"));
    let fx = sqlite();
    let s = fx.script(&[fx.key(K::Schema, &["main"])], &sqlite_target(), &opts());
    assert!(!s.has_notice(GRANTS_NOT_LOADED), "SQLite has no grants");
    assert!(!s.has_notice(UNSUPPORTED_KIND));
    let s = fx.script(
        &[fx.key(K::Table, &["main", "notes"])],
        &sqlite_target(),
        &GenOptions {
            placement: Placement::AfterTable,
            ..opts()
        },
    );
    assert!(s.has_notice(SQLITE_PLACEMENT_FORCED));
}

// ---- every golden ------------------------------------------------------------------------------

#[test]
fn goldens_byte_stable_twice() {
    for c in CASES {
        let a = (c.make)();
        let b = (c.make)();
        assert_eq!(a, b, "{} differs between two runs", c.rel);
        assert!(
            !a.contains('\r') && !a.contains('\t'),
            "{}: LF and spaces only",
            c.rel
        );
        assert!(
            a.ends_with(";\n") || a.ends_with("DELIMITER ;\n"),
            "{}",
            c.rel
        );
    }
}

/// Run under `LC_ALL=tr_TR.UTF-8` (CI `turkey-test`, plan step 4): every golden is the same
/// bytes as the blessed file, so nothing reads the process locale.
#[test]
fn goldens_identical_under_tr_locale() {
    if std::env::var_os("UPDATE_GOLDEN").is_some_and(|v| v == "1") {
        return;
    }
    for c in CASES {
        let expected = std::fs::read(golden_path(c.rel)).expect("blessed golden");
        assert!(
            expected == (c.make)().as_bytes(),
            "{} differs from its golden",
            c.rel
        );
    }
}

#[test]
fn every_golden_file_has_a_case() {
    let root = golden_path("x");
    let root = root.parent().expect("golden dir");
    for dialect in ["pg", "mysql", "mariadb", "sqlite"] {
        for e in std::fs::read_dir(root.join(dialect)).expect("dialect dir") {
            let name = e.expect("entry").file_name().into_string().expect("utf-8");
            let rel = format!("{dialect}/{}", name.trim_end_matches(".sql"));
            assert!(CASES.iter().any(|c| c.rel == rel), "orphan golden {rel}");
        }
    }
    let _ = DelimStyle::Semicolon;
}
