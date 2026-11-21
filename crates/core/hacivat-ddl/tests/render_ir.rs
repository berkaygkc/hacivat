//! M4 Task 7: the IR renderers (alg/03 HV-DDL-07 tests 1–2, the dialect rules of HV-DDL-07 and
//! -09 as renderers apply them, HV-DDL-11 options the renderers honour, every alg/03 variant
//! declared with the non-M4 ones refusing, and the IR goldens of R-M4-16).

use std::path::PathBuf;

use hacivat_ddl::DdlTarget;
use hacivat_ddl::ir::*;
use hacivat_ddl::options::{GenOptions, Placement};
use hacivat_ddl::render::{Delim, RenderCtx, RenderError, render_text, renderer};
use hacivat_meta::kind::ObjectKind;
use hacivat_meta::name::Name;
use hacivat_meta::props::{FkAction, FkMatch, Identity};
use smallvec::SmallVec;
use sql_dialect::{DialectId, ServerVersion};
use sql_ident::ident::Ident;
use sql_ident::quote::{QuoteCtx, QuoteError};

const DIALECTS: [DialectId; 4] = [
    DialectId::Postgres,
    DialectId::MySql,
    DialectId::MariaDb,
    DialectId::Sqlite,
];

// ---- helpers --------------------------------------------------------------------------------

fn n(s: &str) -> Name {
    Name::from(s)
}

fn names(v: &[&str]) -> Vec<Name> {
    v.iter().map(|s| n(s)).collect()
}

fn qn(parts: &[&str]) -> QualifiedName {
    QualifiedName {
        parts: parts
            .iter()
            .map(|p| Ident {
                text: n(p),
                quoted: false,
            })
            .collect::<SmallVec<[Ident; 3]>>(),
    }
}

/// `"a.b.c"` split at dots (test names hold none).
fn dotted(s: &str) -> QualifiedName {
    qn(&s.split('.').collect::<Vec<_>>())
}

fn col(name: &str, ty: &str) -> ColumnDef {
    ColumnDef {
        name: n(name),
        raw_type: n(ty),
        collation: None,
        nullable: true,
        default_raw: None,
        generated: None,
        identity: None,
        on_update: None,
        comment: None,
        invisible: false, // ## m4-t19-produce
    }
}

fn table(
    name: QualifiedName,
    columns: Vec<ColumnDef>,
    constraints: Vec<ConstraintDef>,
) -> TableDef {
    TableDef {
        name,
        if_not_exists: false,
        columns,
        constraints,
        comment: None,
        ext: TableDefExt::None,
    }
}

fn create_table(schema: &str, name: &str, cols: &[ColumnDef]) -> DdlOp {
    DdlOp::CreateTable(table(qn(&[schema, name]), cols.to_vec(), Vec::new()))
}

fn constraint(name: Option<&str>, body: ConstraintBody) -> ConstraintDef {
    ConstraintDef {
        name: name.map(n),
        body,
        deferrable: Deferrable::No,
        not_valid: false,
    }
}

fn pk(name: Option<&str>, cols: &[&str]) -> ConstraintDef {
    constraint(
        name,
        ConstraintBody::PrimaryKey {
            columns: names(cols),
        },
    )
}

fn fk_customer() -> ConstraintDef {
    constraint(
        Some("fk_customer"),
        ConstraintBody::ForeignKey {
            columns: names(&["customer_id"]),
            target: qn(&["crm", "customers"]),
            target_columns: names(&["id"]),
            on_update: FkAction::NoAction,
            on_delete: FkAction::Cascade,
            match_kind: FkMatch::Simple,
        },
    )
}

/// `t(id integer, CONSTRAINT <name> PRIMARY KEY (id))` (alg/03 HV-DDL-09 test 1).
fn t_with_pk(t: &str, column: &str, name: Option<&str>) -> DdlOp {
    DdlOp::CreateTable(table(
        qn(&[t]),
        vec![col(column, "integer")],
        vec![pk(name, &[column])],
    ))
}

/// `COMMENT ON COLUMN <schema.table.column>`.
fn comment_col(path: &str, text: &str) -> DdlOp {
    let (table, column) = path.rsplit_once('.').expect("schema.table.column");
    DdlOp::Comment {
        target: CommentTarget {
            kind: ObjectKind::Column,
            name: dotted(table),
            column: Some(n(column)),
            sig: None,
        },
        text: Some(n(text)),
    }
}

/// A target with every quoting setting known at the servers' defaults.
fn target(d: DialectId) -> DdlTarget {
    let mut t = DdlTarget::new(d, ServerVersion::new(17, 0, 0));
    t.quote = QuoteCtx {
        ansi_quotes: Some(false),
        no_backslash_escapes: Some(false),
        standard_conforming_strings: Some(true),
    };
    t
}

fn ctx_with(d: DialectId, opts: GenOptions) -> RenderCtx {
    RenderCtx::new(target(d), opts)
}

fn render_ctx(ops: &[DdlOp], ctx: &RenderCtx) -> Result<String, RenderError> {
    render_text(ops, ctx)
}

fn render(d: DialectId, ops: &[DdlOp]) -> String {
    render_ctx(ops, &ctx_with(d, GenOptions::default())).expect("render")
}

fn render_pg(op: &DdlOp) -> String {
    render(DialectId::Postgres, std::slice::from_ref(op))
}

fn render_pg_with(placement: Placement, op: &DdlOp) -> String {
    let opts = GenOptions {
        placement,
        ..GenOptions::default()
    };
    render_ctx(
        std::slice::from_ref(op),
        &ctx_with(DialectId::Postgres, opts),
    )
    .expect("render")
}

fn check_golden(rel: &str, actual: &str) {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests", "golden"]
        .iter()
        .collect::<PathBuf>()
        .join(format!("{rel}.sql"));
    if std::env::var_os("UPDATE_GOLDEN").is_some_and(|v| v == "1") {
        std::fs::create_dir_all(path.parent().expect("golden dir")).expect("mkdir");
        std::fs::write(&path, actual.as_bytes()).expect("write golden");
        return;
    }
    let expected = std::fs::read(&path).unwrap_or_else(|_| {
        panic!(
            "missing golden {}; bless with UPDATE_GOLDEN=1 and review it",
            path.display()
        )
    });
    assert!(
        expected == actual.as_bytes(),
        "golden {rel} differs\n--- expected\n{}\n--- actual\n{actual}",
        String::from_utf8_lossy(&expected)
    );
}

// ---- alg/03 HV-DDL-07 tests 1–2 -------------------------------------------------------------

#[test]
fn pg_table_canonical_type_null_omitted() {
    assert_eq!(
        render_pg(&create_table(
            "test_schema",
            "test_table",
            &[col("column1", "integer")]
        )),
        "CREATE TABLE test_schema.test_table (\n    column1 integer\n);\n"
    );
}

#[test]
fn pg_comment_escapes_quote() {
    assert_eq!(
        render_pg(&comment_col(
            "test_schema.test_table.column1",
            "PM's approver of record"
        )),
        "COMMENT ON COLUMN test_schema.test_table.column1 IS 'PM''s approver of record';\n"
    );
}

#[test]
fn comment_null_removes_and_routines_carry_their_signature() {
    let mut op = comment_col("app.t.c", "x");
    if let DdlOp::Comment { text, .. } = &mut op {
        *text = None;
    }
    assert_eq!(render_pg(&op), "COMMENT ON COLUMN app.t.c IS NULL;\n");
    let f = |sig: Option<&str>| DdlOp::Comment {
        target: CommentTarget {
            kind: ObjectKind::Routine,
            name: qn(&["app", "f"]),
            column: None,
            sig: sig.map(n),
        },
        text: Some(n("doc")),
    };
    assert_eq!(
        render_pg(&f(Some("integer, text"))),
        "COMMENT ON FUNCTION app.f(integer, text) IS 'doc';\n"
    );
    // A routine named without its argument types could name another overload: refused.
    assert!(matches!(
        render_text(
            &[f(None)],
            &ctx_with(DialectId::Postgres, GenOptions::default())
        ),
        Err(RenderError::Invalid(_))
    ));
}

// ---- dialect rules --------------------------------------------------------------------------

#[test]
fn mysql_comment_escaping_respects_no_backslash_escapes() {
    let mut c = col("note", "text");
    c.comment = Some(n("it's C:\\tmp"));
    let op = create_table("shop", "t", &[c]);
    let mut ctx = ctx_with(DialectId::MySql, GenOptions::default());
    let escapes_on = render_ctx(std::slice::from_ref(&op), &ctx).unwrap();
    assert!(
        escapes_on.contains("COMMENT 'it''s C:\\\\tmp'"),
        "{escapes_on}"
    );
    ctx.target.quote.no_backslash_escapes = Some(true);
    let escapes_off = render_ctx(std::slice::from_ref(&op), &ctx).unwrap();
    assert!(
        escapes_off.contains("COMMENT 'it''s C:\\tmp'"),
        "{escapes_off}"
    );
    // Unknown: the fail-safe form (the backslash doubled).
    ctx.target.quote.no_backslash_escapes = None;
    let unknown = render_ctx(&[op], &ctx).unwrap();
    assert_eq!(unknown, escapes_on);
}

#[test]
fn mysql_fk_is_never_inline() {
    let op = DdlOp::CreateTable(table(
        qn(&["shop", "orders"]),
        vec![col("customer_id", "bigint")],
        vec![fk_customer()],
    ));
    // MariaDB's keyword table has no reserved flag, so its keyword `id` is backticked there.
    for (d, id) in [(DialectId::MySql, "id"), (DialectId::MariaDb, "`id`")] {
        let opts = GenOptions {
            placement: Placement::InsideColumn,
            ..GenOptions::default()
        };
        let text = render_ctx(std::slice::from_ref(&op), &ctx_with(d, opts)).unwrap();
        assert_eq!(
            text,
            format!(
                "CREATE TABLE shop.orders (\n    customer_id bigint,\n    CONSTRAINT fk_customer FOREIGN KEY (customer_id) REFERENCES crm.customers ({id}) ON DELETE CASCADE\n);\n"
            ),
            "{d:?}"
        );
    }
    // PostgreSQL takes the same FK inline under InsideColumn.
    let pg = render_pg_with(Placement::InsideColumn, &op);
    assert!(
        pg.contains(
            "customer_id bigint CONSTRAINT fk_customer REFERENCES crm.customers (id) ON DELETE CASCADE"
        ),
        "{pg}"
    );
}

#[test]
fn inside_column_single_column_pk() {
    assert!(
        render_pg_with(Placement::InsideColumn, &t_with_pk("t", "id", Some("pk")))
            .contains("id integer CONSTRAINT pk PRIMARY KEY")
    );
    // InsideTable (the default) keeps it a table element.
    assert_eq!(
        render_pg(&t_with_pk("t", "id", Some("pk"))),
        "CREATE TABLE t (\n    id integer,\n    CONSTRAINT pk PRIMARY KEY (id)\n);\n"
    );
}

#[test]
fn after_table_placement_renders_alter_table_add() {
    let op = DdlOp::CreateTable(table(
        qn(&["app", "orders"]),
        vec![col("id", "integer"), col("customer_id", "bigint")],
        vec![pk(Some("orders_pk"), &["id"]), fk_customer()],
    ));
    assert_eq!(
        render_pg_with(Placement::AfterTable, &op),
        "CREATE TABLE app.orders (\n    id integer,\n    customer_id bigint\n);\n\n\
         ALTER TABLE app.orders ADD CONSTRAINT orders_pk PRIMARY KEY (id);\n\n\
         ALTER TABLE app.orders ADD CONSTRAINT fk_customer FOREIGN KEY (customer_id) REFERENCES crm.customers (id) ON DELETE CASCADE;\n"
    );
    // A postponed constraint (HV-DDL-09: cycles, later tables) is an `AlterTable` op of its
    // own; a `NOT VALID` check keeps that clause.
    let mut check = constraint(
        Some("amount_ck"),
        ConstraintBody::Check {
            expr: n("amount > 0"),
            no_inherit: false,
        },
    );
    check.not_valid = true;
    let postponed = DdlOp::AlterTable {
        table: qn(&["app", "orders"]),
        actions: vec![AlterAction::AddConstraint(check)],
    };
    assert_eq!(
        render_pg(&postponed),
        "ALTER TABLE app.orders ADD CONSTRAINT amount_ck CHECK (amount > 0) NOT VALID;\n"
    );
}

#[test]
fn turkish_identifier_quoted_on_pg() {
    let text = render_pg(&create_table("public", "MÜŞTERİ", &[col("ad", "text")]));
    assert!(text.contains(r#"public."MÜŞTERİ""#), "{text}");
    assert!(text.contains("\n    ad text\n"), "{text}");
    let mixed = render_pg(&create_table(
        "app",
        "t",
        &[col("Ad", "text"), col("ılık", "int")],
    ));
    assert!(
        mixed.contains("    \"Ad\" text,\n    \"ılık\" int\n"),
        "{mixed}"
    );
}

#[test]
fn reserved_word_is_quoted_per_dialect() {
    let op = DdlOp::CreateTable(table(qn(&["order"]), vec![col("x", "int")], Vec::new()));
    let starts = |d, prefix: &str| {
        let text = render(d, std::slice::from_ref(&op));
        assert!(text.starts_with(prefix), "{d:?}: {text}");
    };
    starts(DialectId::Postgres, "CREATE TABLE \"order\" (");
    starts(DialectId::MySql, "CREATE TABLE `order` (");
    starts(DialectId::MariaDb, "CREATE TABLE `order` (");
    starts(DialectId::Sqlite, "CREATE TABLE \"order\" (");
}

#[test]
fn server_text_is_never_rewritten() {
    let text = "CREATE VIEW \"Weird\" AS  select  1 /* keep */ ;\n\n";
    let opts = GenOptions {
        qualify: hacivat_ddl::options::Qualify::Never,
        ..GenOptions::default()
    };
    for d in DIALECTS {
        let op = DdlOp::Raw {
            sql: text.to_string(),
            origin: RawOrigin::ServerText,
        };
        // Only the trailing whitespace and the one `;` go; the script adds its own.
        assert_eq!(
            render_ctx(&[op], &ctx_with(d, opts)).unwrap(),
            "CREATE VIEW \"Weird\" AS  select  1 /* keep */;\n",
            "{d:?}"
        );
    }
    // Routine and trigger bodies and view queries are emitted as the server printed them.
    let body = "CREATE OR REPLACE FUNCTION app.f()\n RETURNS integer\n LANGUAGE sql\nAS $function$SELECT 1$function$\n";
    let routine = DdlOp::CreateRoutine(RoutineDef {
        name: qn(&["app", "f"]),
        kind: ObjectKind::Routine,
        sig: Some(n("")),
        text: n(body),
        comment: None,
    });
    assert_eq!(
        render_pg(&routine),
        "CREATE OR REPLACE FUNCTION app.f()\n RETURNS integer\n LANGUAGE sql\nAS $function$SELECT 1$function$;\n"
    );
    let view = DdlOp::CreateView(ViewDef {
        name: qn(&["app", "v"]),
        kind: ViewKind::View,
        or_replace: false,
        body: n(" SELECT t.id\n   FROM other.t;"),
        check_option: None,
        comment: None,
        column_comments: Vec::new(),
    });
    assert_eq!(
        render_pg(&view),
        "CREATE VIEW app.v AS\n SELECT t.id\n   FROM other.t;\n"
    );
}

#[test]
fn nul_in_comment_is_refused() {
    assert_eq!(
        render_text(
            &[comment_col("app.t.c", "x\0y")],
            &ctx_with(DialectId::Postgres, GenOptions::default())
        ),
        Err(RenderError::Quote(QuoteError::Nul))
    );
    let mut c = col("a", "int");
    c.comment = Some(n("x\0y"));
    for d in [DialectId::Postgres, DialectId::MySql, DialectId::MariaDb] {
        assert_eq!(
            render_text(
                &[create_table("s", "t", std::slice::from_ref(&c))],
                &ctx_with(d, GenOptions::default())
            ),
            Err(RenderError::Quote(QuoteError::Nul)),
            "{d:?}"
        );
    }
    // A NUL in a name is refused on every dialect.
    for d in DIALECTS {
        assert_eq!(
            render_text(
                &[create_table("s", "t\0", &[col("a", "int")])],
                &ctx_with(d, GenOptions::default())
            ),
            Err(RenderError::Quote(QuoteError::Nul)),
            "{d:?}"
        );
    }
}

#[test]
fn or_replace_applies_to_pg_view_not_table() {
    let opts = GenOptions {
        or_replace: true,
        ..GenOptions::default()
    };
    let ctx = ctx_with(DialectId::Postgres, opts);
    let view = |kind| {
        DdlOp::CreateView(ViewDef {
            name: qn(&["app", "v"]),
            kind,
            or_replace: false,
            body: n(" SELECT 1"),
            check_option: None,
            comment: None,
            column_comments: Vec::new(),
        })
    };
    assert_eq!(
        render_ctx(&[view(ViewKind::View)], &ctx).unwrap(),
        "CREATE OR REPLACE VIEW app.v AS\n SELECT 1;\n"
    );
    // No `OR REPLACE` exists for tables or materialized views.
    assert_eq!(
        render_ctx(&[view(ViewKind::Materialized)], &ctx).unwrap(),
        "CREATE MATERIALIZED VIEW app.v AS\n SELECT 1;\n"
    );
    assert_eq!(
        render_ctx(&[create_table("app", "t", &[col("a", "int")])], &ctx).unwrap(),
        "CREATE TABLE app.t (\n    a int\n);\n"
    );
    // PostgreSQL 14+ triggers take it; a constraint trigger and an older server do not.
    let trigger = |text: &str| {
        DdlOp::CreateTrigger(TriggerDef {
            name: n("tg"),
            table: qn(&["app", "t"]),
            text: n(text),
            comment: None,
        })
    };
    let plain = "CREATE TRIGGER tg BEFORE INSERT ON app.t FOR EACH ROW EXECUTE FUNCTION app.f()";
    assert_eq!(
        render_ctx(&[trigger(plain)], &ctx).unwrap(),
        "CREATE OR REPLACE TRIGGER tg BEFORE INSERT ON app.t FOR EACH ROW EXECUTE FUNCTION app.f();\n"
    );
    let constraint_trigger =
        "CREATE CONSTRAINT TRIGGER tg AFTER INSERT ON app.t FOR EACH ROW EXECUTE FUNCTION app.f()";
    assert_eq!(
        render_ctx(&[trigger(constraint_trigger)], &ctx).unwrap(),
        format!("{constraint_trigger};\n")
    );
    let mut pg13 = ctx.clone();
    pg13.target.version = ServerVersion::new(13, 0, 0);
    assert_eq!(
        render_ctx(&[trigger(plain)], &pg13).unwrap(),
        format!("{plain};\n")
    );
}

#[test]
fn mysql_skip_definer_drops_definer() {
    let text = "CREATE DEFINER=`root`@`%` PROCEDURE `p`()\nBEGIN\n  SELECT 1;\nEND";
    let op = DdlOp::CreateRoutine(RoutineDef {
        name: qn(&["shop", "p"]),
        kind: ObjectKind::Procedure,
        sig: None,
        text: n(text),
        comment: None,
    });
    let skip = GenOptions {
        skip_definer: true,
        ..GenOptions::default()
    };
    for d in [DialectId::MySql, DialectId::MariaDb] {
        let stmts = renderer(d).render(&op, &ctx_with(d, skip)).expect("render");
        assert_eq!(stmts.len(), 1);
        assert_eq!(
            stmts[0].text, "CREATE PROCEDURE `p`()\nBEGIN\n  SELECT 1;\nEND",
            "{d:?}"
        );
        // The body holds `;`: file output needs a client delimiter around it.
        assert_eq!(stmts[0].delimiter, Delim::Custom(n(";;")));
        // Without the option the text is the server's, definer included.
        let kept = renderer(d)
            .render(&op, &ctx_with(d, GenOptions::default()))
            .expect("render");
        assert_eq!(kept[0].text, text);
    }
    // A view's definer goes too; ALGORITHM and SQL SECURITY stay.
    let view = DdlOp::Raw {
        sql: "CREATE ALGORITHM=UNDEFINED DEFINER=`app`@`%` SQL SECURITY DEFINER VIEW `v` AS select 1 AS `a`".to_string(),
        origin: RawOrigin::ServerText,
    };
    assert_eq!(
        render_ctx(&[view], &ctx_with(DialectId::MySql, skip)).unwrap(),
        "CREATE ALGORITHM=UNDEFINED SQL SECURITY DEFINER VIEW `v` AS select 1 AS `a`;\n"
    );
    // A head of an unexpected shape (an executable comment) is left exactly as it is.
    let odd = "CREATE /*!50017 DEFINER=`root`@`%`*/ TRIGGER tg BEFORE INSERT ON t FOR EACH ROW SET @a = 1";
    let trigger = DdlOp::CreateTrigger(TriggerDef {
        name: n("tg"),
        table: qn(&["shop", "t"]),
        text: n(odd),
        comment: None,
    });
    assert_eq!(
        render_ctx(&[trigger], &ctx_with(DialectId::MySql, skip)).unwrap(),
        format!("{odd};\n")
    );
    assert!(hacivat_ddl::render::mysql::edit_head(odd, true, false).is_err());
}

#[test]
fn mysql_and_mariadb_default_texts_kept_verbatim_per_flavor() {
    // F9: each flavor reports its own default and ON UPDATE texts; the renderer emits them as
    // they are, never re-quoted or re-cased.
    let column = |default: &str, on_update: Option<&str>| {
        let mut c = col("x", "timestamp");
        c.nullable = false;
        c.default_raw = Some(n(default));
        c.on_update = on_update.map(n);
        c
    };
    let mut region = col("region", "varchar(8)");
    region.default_raw = Some(n("eu"));
    let my = DdlOp::CreateTable(table(
        qn(&["shop", "t"]),
        vec![
            column("CURRENT_TIMESTAMP", Some("CURRENT_TIMESTAMP")),
            region.clone(),
        ],
        Vec::new(),
    ));
    assert_eq!(
        render(DialectId::MySql, &[my]),
        "CREATE TABLE shop.t (\n    x timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,\n    region varchar(8) DEFAULT eu\n);\n"
    );
    region.default_raw = Some(n("'eu'"));
    let maria = DdlOp::CreateTable(table(
        qn(&["shop", "t"]),
        vec![
            column("current_timestamp()", Some("current_timestamp()")),
            region,
        ],
        Vec::new(),
    ));
    assert_eq!(
        render(DialectId::MariaDb, &[maria]),
        "CREATE TABLE shop.t (\n    x timestamp NOT NULL DEFAULT current_timestamp() ON UPDATE current_timestamp(),\n    region varchar(8) DEFAULT 'eu'\n);\n"
    );
}

#[test]
fn pg_row_security_renders_enable_statement() {
    let op = |enable| DdlOp::RowSecurity {
        table: qn(&["app", "accounts"]),
        enable,
    };
    assert_eq!(
        render_pg(&op(true)),
        "ALTER TABLE app.accounts ENABLE ROW LEVEL SECURITY;\n"
    );
    assert_eq!(
        render_pg(&op(false)),
        "ALTER TABLE app.accounts DISABLE ROW LEVEL SECURITY;\n"
    );
    for d in [DialectId::MySql, DialectId::MariaDb, DialectId::Sqlite] {
        assert!(matches!(
            renderer(d).render(&op(true), &ctx_with(d, GenOptions::default())),
            Err(RenderError::Unsupported { .. })
        ));
    }
}

#[test]
fn sqlite_autoincrement_needs_its_inline_primary_key() {
    let mut id = col("id", "INTEGER");
    id.identity = Some(Identity::AutoIncrement);
    let op = DdlOp::CreateTable(table(
        qn(&["main", "t"]),
        vec![id.clone()],
        vec![pk(None, &["id"])],
    ));
    let opts = GenOptions {
        qualify: hacivat_ddl::options::Qualify::Never,
        ..GenOptions::default()
    };
    // Whatever the placement mode, the key goes into its column.
    for placement in [
        Placement::InsideTable,
        Placement::AfterTable,
        Placement::InsideColumn,
    ] {
        let ctx = ctx_with(DialectId::Sqlite, GenOptions { placement, ..opts });
        assert_eq!(
            render_ctx(std::slice::from_ref(&op), &ctx).unwrap(),
            "CREATE TABLE main.t (\n    id INTEGER PRIMARY KEY AUTOINCREMENT\n);\n"
        );
    }
    // AUTOINCREMENT without a single-column primary key on that column is refused.
    let no_pk = DdlOp::CreateTable(table(qn(&["t"]), vec![id], Vec::new()));
    assert!(matches!(
        render_text(&[no_pk], &ctx_with(DialectId::Sqlite, opts)),
        Err(RenderError::Invalid(_))
    ));
}

#[test]
fn grants_and_owners() {
    let grant = |kind, object: QualifiedName, sig: Option<&str>, grantee: &str| GrantDef {
        kind,
        object,
        sig: sig.map(n),
        privileges: names(&["SELECT", "UPDATE"]),
        columns: Vec::new(),
        grantee: n(grantee),
        grantable: true,
    };
    assert_eq!(
        render_pg(&DdlOp::Grant(grant(
            ObjectKind::View,
            qn(&["app", "v"]),
            None,
            "Reader"
        ))),
        "GRANT SELECT, UPDATE ON TABLE app.v TO \"Reader\" WITH GRANT OPTION;\n"
    );
    let mut public = grant(
        ObjectKind::Routine,
        qn(&["app", "f"]),
        Some("integer"),
        "PUBLIC",
    );
    public.privileges = names(&["EXECUTE"]);
    public.grantable = false;
    assert_eq!(
        render_pg(&DdlOp::Grant(public.clone())),
        "GRANT EXECUTE ON FUNCTION app.f(integer) TO PUBLIC;\n"
    );
    public.sig = None;
    assert!(matches!(
        render_text(
            &[DdlOp::Grant(public)],
            &ctx_with(DialectId::Postgres, GenOptions::default())
        ),
        Err(RenderError::Invalid(_))
    ));
    // A privilege is a keyword list, never free text.
    let mut evil = grant(ObjectKind::Table, qn(&["app", "t"]), None, "r");
    evil.privileges = names(&["SELECT ON app.t TO r; DROP TABLE x; --"]);
    assert!(matches!(
        render_text(
            &[DdlOp::Grant(evil)],
            &ctx_with(DialectId::Postgres, GenOptions::default())
        ),
        Err(RenderError::Invalid(_))
    ));
    // MySQL: the grantee is the server's `'user'@'host'` form, validated.
    let mut my = grant(ObjectKind::Table, qn(&["shop", "t"]), None, "'app'@'%'");
    my.columns = names(&["a"]);
    assert_eq!(
        render(DialectId::MySql, &[DdlOp::Grant(my.clone())]),
        "GRANT SELECT (a), UPDATE (a) ON shop.t TO 'app'@'%' WITH GRANT OPTION;\n"
    );
    my.grantee = n("'app'@'%'; DROP TABLE t");
    assert!(matches!(
        render_text(
            &[DdlOp::Grant(my)],
            &ctx_with(DialectId::MySql, GenOptions::default())
        ),
        Err(RenderError::Invalid(_))
    ));
    let owner = |kind, sig: Option<&str>| DdlOp::Owner {
        kind,
        name: qn(&["app", "x"]),
        sig: sig.map(n),
        owner: n("app_owner"),
    };
    assert_eq!(
        render_pg(&owner(ObjectKind::Table, None)),
        "ALTER TABLE app.x OWNER TO app_owner;\n"
    );
    assert_eq!(
        render_pg(&owner(ObjectKind::Procedure, Some(""))),
        "ALTER PROCEDURE app.x() OWNER TO app_owner;\n"
    );
}

#[test]
fn pg_types_sequences_and_indexes() {
    let labels: std::sync::Arc<[Name]> = names(&["sad", "it's ok"]).into();
    let mood = DdlOp::CreateType(TypeDef {
        name: qn(&["app", "mood"]),
        body: hacivat_meta::props::TypeBody::Enum { labels },
        comment: Some(n("Mood")),
    });
    assert_eq!(
        render_pg(&mood),
        "CREATE TYPE app.mood AS ENUM (\n    'sad',\n    'it''s ok'\n);\n\nCOMMENT ON TYPE app.mood IS 'Mood';\n"
    );
    let domain = DdlOp::CreateType(TypeDef {
        name: qn(&["app", "pos"]),
        body: hacivat_meta::props::TypeBody::Domain {
            base: n("integer"),
            not_null: true,
            not_null_constraint: Some(n("pos_nn")),
            default_raw: Some(n("1")),
            checks: names(&["VALUE > 0"]),
        },
        comment: None,
    });
    assert_eq!(
        render_pg(&domain),
        "CREATE DOMAIN app.pos AS integer\n    DEFAULT 1\n    CONSTRAINT pos_nn NOT NULL\n    CHECK (VALUE > 0);\n"
    );
    let seq = DdlOp::CreateSequence(SeqDef {
        name: qn(&["app", "s"]),
        if_not_exists: false,
        data_type: Some(n("integer")),
        start: Some(1),
        increment: Some(1),
        min: None,
        max: None,
        cache: Some(1),
        cycle: false,
        owned_by: None,
        comment: None,
    });
    // Clauses equal to the server's defaults for that type are left out.
    assert_eq!(render_pg(&seq), "CREATE SEQUENCE app.s\n    AS integer;\n");
    let index = DdlOp::CreateIndex(IndexDef {
        name: n("orders_open_idx"),
        table: qn(&["app", "orders"]),
        unique: false,
        if_not_exists: false,
        concurrently: true,
        method: Some(n("btree")),
        parts: vec![
            IndexPartDef {
                column: Some(n("created_at")),
                desc: true,
                nulls_first: Some(false),
                ..IndexPartDef::default()
            },
            IndexPartDef {
                expr: Some(n("lower(note)")),
                ..IndexPartDef::default()
            },
        ],
        include: names(&["total"]),
        predicate: Some(n("(status = 'open'::text)")),
        nulls_not_distinct: false,
        comment: Some(n("Open orders")),
    });
    let stmts = renderer(DialectId::Postgres)
        .render(
            &index,
            &ctx_with(DialectId::Postgres, GenOptions::default()),
        )
        .unwrap();
    assert_eq!(
        stmts[0].text,
        "CREATE INDEX CONCURRENTLY orders_open_idx ON app.orders (created_at DESC NULLS LAST, (lower(note))) INCLUDE (total) WHERE (status = 'open'::text)"
    );
    assert_eq!(
        stmts[0].txn,
        hacivat_ddl::render::TxnClass::NonTransactional
    );
    assert_eq!(
        stmts[1].text,
        "COMMENT ON INDEX app.orders_open_idx IS 'Open orders'"
    );
}

// ---- every variant --------------------------------------------------------------------------

/// Variants M4 never renders, on any dialect (M7 fills them).
const NOT_IN_M4: [&str; 8] = [
    "AlterTable",
    "DropObject",
    "Rename",
    "SetSchema",
    "Revoke",
    "TableRebuild",
    "Exchange",
    "Data",
];

/// The M4 variants each dialect cannot express at all.
fn not_on(d: DialectId) -> &'static [&'static str] {
    match d {
        // SQLite has no stored routines either.
        DialectId::Sqlite => &[
            "CreateSchema",
            "CreateRoutine",
            "CreateSequence",
            "CreateType",
            "Comment",
            "Grant",
            "Owner",
            "RowSecurity",
        ],
        DialectId::MySql => &[
            "CreateSequence",
            "CreateType",
            "Comment",
            "Owner",
            "RowSecurity",
        ],
        DialectId::MariaDb => &["CreateType", "Comment", "Owner", "RowSecurity"],
        _ => &[],
    }
}

#[test]
fn every_alg03_variant_is_declared_and_non_m4_ones_refuse() {
    let ops = DdlOp::samples();
    let mut seen: Vec<&str> = ops.iter().map(DdlOp::variant).collect();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), 21, "{seen:?}");
    for d in DIALECTS {
        let ctx = ctx_with(d, GenOptions::default());
        for op in &ops {
            let res = renderer(d).render(op, &ctx);
            let v = op.variant();
            if NOT_IN_M4.contains(&v) || not_on(d).contains(&v) {
                assert!(
                    matches!(res, Err(RenderError::Unsupported { dialect, .. }) if dialect == d),
                    "{d:?} {v}: {res:?}"
                );
            } else if matches!(d, DialectId::MySql | DialectId::MariaDb) && v == "Grant" {
                // Supported, but the sample's grantee `reader` is not the server's
                // `'user'@'host'` form (`grants_and_owners` renders a valid one).
                assert!(
                    matches!(res, Err(RenderError::Invalid(_))),
                    "{d:?} {v}: {res:?}"
                );
            } else {
                // Every other M4 variant the dialect has renders (review M6: all four).
                assert!(res.is_ok(), "{d:?} {v}: {res:?}");
            }
        }
        // Text a user edited is M7's (source edit), never rendered in M4.
        let edited = DdlOp::Raw {
            sql: "SELECT 1".to_string(),
            origin: RawOrigin::UserEdited,
        };
        assert!(matches!(
            renderer(d).render(&edited, &ctx),
            Err(RenderError::Unsupported { .. })
        ));
    }
}

#[test]
fn unsupported_variant_is_an_explicit_error() {
    let op = DdlOp::Rename {
        kind: ObjectKind::Table,
        from: qn(&["a"]),
        to: n("b"),
    };
    for d in DIALECTS {
        assert_eq!(
            renderer(d).render(&op, &ctx_with(d, GenOptions::default())),
            Err(RenderError::Unsupported {
                dialect: d,
                op: "Rename"
            })
        );
    }
    // A dialect without a renderer refuses every op.
    assert!(matches!(
        renderer(DialectId::SqlServer)
            .render(&op, &ctx_with(DialectId::SqlServer, GenOptions::default())),
        Err(RenderError::Unsupported { .. })
    ));
}

// ---- goldens and locale ---------------------------------------------------------------------

/// A rich table rendered by each dialect's renderer (IR golden, R-M4-16), qualified as given.
fn ir_table(d: DialectId) -> Vec<DdlOp> {
    let sqlite = d == DialectId::Sqlite;
    let mut id = col("id", if sqlite { "INTEGER" } else { "bigint" });
    id.nullable = false;
    id.identity = Some(match d {
        DialectId::Postgres => Identity::ByDefault,
        _ => Identity::AutoIncrement,
    });
    let mut name = col("name", "varchar(80)");
    name.nullable = false;
    name.default_raw = Some(n("'?'"));
    name.comment = Some(n("Display name"));
    let ext = match d {
        DialectId::Postgres => TableDefExt::Pg(PgTableDef {
            with: names(&["fillfactor=80"]),
            ..PgTableDef::default()
        }),
        DialectId::Sqlite => TableDefExt::Sqlite(SqliteTableDef::default()),
        _ => TableDefExt::MySql(MySqlTableDef {
            engine: Some(n("InnoDB")),
            ..MySqlTableDef::default()
        }),
    };
    let mut constraints = vec![
        pk(Some("people_pk"), &["id"]),
        constraint(
            Some("people_name_ck"),
            ConstraintBody::Check {
                expr: n("name <> ''"),
                no_inherit: false,
            },
        ),
    ];
    let mut cols = vec![id, name];
    if !sqlite {
        constraints.push(fk_customer());
        cols.push(col("customer_id", "bigint"));
    }
    let mut people = table(qn(&["app", "people"]), cols, constraints);
    people.comment = Some(n("People"));
    people.ext = ext;
    vec![
        DdlOp::CreateTable(people),
        DdlOp::CreateIndex(IndexDef {
            name: n("people_name_idx"),
            table: qn(&["app", "people"]),
            unique: true,
            if_not_exists: false,
            concurrently: false,
            method: None,
            parts: vec![IndexPartDef {
                column: Some(n("name")),
                desc: true,
                ..IndexPartDef::default()
            }],
            include: Vec::new(),
            predicate: None,
            nulls_not_distinct: false,
            comment: None,
        }),
    ]
}

#[test]
fn ir_goldens_byte_stable() {
    for (d, dir) in [
        (DialectId::Postgres, "pg"),
        (DialectId::MySql, "mysql"),
        (DialectId::MariaDb, "mariadb"),
        (DialectId::Sqlite, "sqlite"),
    ] {
        let text = render(d, &ir_table(d));
        assert!(!text.contains('\r'), "{d:?}: CR in output");
        check_golden(&format!("ir/{dir}/create_table"), &text);
        assert_eq!(text, render(d, &ir_table(d)), "{d:?}: not byte-stable");
    }
}

/// Every probe op of [`output_is_identical_under_en_and_tr`] rendered on every dialect.
fn locale_probe_text() -> String {
    let mut out = String::new();
    for d in DIALECTS {
        let mut ops = ir_table(d);
        ops.push(create_table(
            "İNDEX",
            "ındex",
            &[col("TITLE", "int"), col("id", "int")],
        ));
        out.push_str(&format!("-- {d:?}\n"));
        out.push_str(&render(d, &ops));
    }
    out
}

const BEGIN: &str = "<<<hv-t07-render-begin>>>";
const END: &str = "<<<hv-t07-render-end>>>";

/// Prints [`locale_probe_text`]; run by [`output_is_identical_under_en_and_tr`] in a child.
#[test]
#[ignore = "helper: run in a child process by output_is_identical_under_en_and_tr"]
fn render_locale_probe() {
    println!("{BEGIN}");
    print!("{}", locale_probe_text());
    println!("{END}");
}

#[test]
fn output_is_identical_under_en_and_tr() {
    let here = locale_probe_text();
    for locale in ["en_US.UTF-8", "tr_TR.UTF-8"] {
        let exe = std::env::current_exe().expect("test binary");
        let out = std::process::Command::new(exe)
            .args([
                "--exact",
                "render_locale_probe",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("LC_ALL", locale)
            .env("LANG", locale)
            .output()
            .expect("run the probe");
        assert!(out.status.success(), "probe failed under {locale}: {out:?}");
        let stdout = String::from_utf8(out.stdout).expect("utf-8");
        let start = stdout.find(BEGIN).expect("begin marker") + BEGIN.len() + 1;
        let end = stdout.find(END).expect("end marker");
        assert_eq!(&stdout[start..end], here, "{locale}");
    }
}

#[test]
fn keyword_slots_are_never_free_text_or_quoted() {
    let view = |opt: &str| {
        DdlOp::CreateView(ViewDef {
            name: qn(&["app", "v"]),
            kind: ViewKind::View,
            or_replace: false,
            body: n("SELECT 1"),
            check_option: Some(n(opt)),
            comment: None,
            column_comments: Vec::new(),
        })
    };
    assert!(render_pg(&view("local")).contains("WITH LOCAL CHECK OPTION"));
    for d in [DialectId::Postgres, DialectId::MySql] {
        assert!(matches!(
            render_text(
                &[view("LOCAL CHECK OPTION; DROP TABLE t; --")],
                &ctx_with(d, GenOptions::default())
            ),
            Err(RenderError::Invalid(_))
        ));
    }
    // MySQL index methods are keywords: `HASH` stays bare even where MariaDB quotes its
    // keywords; anything else is refused.
    let index = |method: &str| {
        DdlOp::CreateIndex(IndexDef {
            name: n("k"),
            table: qn(&["shop", "t"]),
            unique: false,
            if_not_exists: false,
            concurrently: false,
            method: Some(n(method)),
            parts: vec![IndexPartDef {
                column: Some(n("a")),
                ..IndexPartDef::default()
            }],
            include: Vec::new(),
            predicate: None,
            nulls_not_distinct: false,
            comment: None,
        })
    };
    assert_eq!(
        render(DialectId::MariaDb, &[index("hash")]),
        "CREATE INDEX k ON shop.t (a) USING HASH;\n"
    );
    assert_eq!(
        render(DialectId::MySql, &[index("FULLTEXT")]),
        "CREATE FULLTEXT INDEX k ON shop.t (a);\n"
    );
    assert!(matches!(
        render_text(
            &[index("x) ; DROP")],
            &ctx_with(DialectId::MySql, GenOptions::default())
        ),
        Err(RenderError::Invalid(_))
    ));
    // Table options: names through quoting, ROW_FORMAT from its keyword list.
    let mut t = table(qn(&["shop", "t"]), vec![col("a", "int")], Vec::new());
    t.ext = TableDefExt::MySql(MySqlTableDef {
        engine: Some(n("InnoDB")),
        row_format: Some(n("dynamic")),
        charset: Some(n("utf8mb4")),
        collation: Some(n("utf8mb4 bin")),
    });
    assert_eq!(
        render(DialectId::MySql, &[DdlOp::CreateTable(t.clone())]),
        "CREATE TABLE shop.t (\n    a int\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=`utf8mb4 bin` ROW_FORMAT=DYNAMIC;\n"
    );
    t.ext = TableDefExt::MySql(MySqlTableDef {
        row_format: Some(n("DYNAMIC COMMENT='x'")),
        ..MySqlTableDef::default()
    });
    assert!(matches!(
        render_text(
            &[DdlOp::CreateTable(t)],
            &ctx_with(DialectId::MySql, GenOptions::default())
        ),
        Err(RenderError::Invalid(_))
    ));
}

/// Review M4: server text ending in a line comment gets its `;` on a line of its own, so the
/// comment cannot swallow the terminator.
#[test]
fn terminator_follows_a_trailing_line_comment_on_its_own_line() {
    let raw = |sql: &str| DdlOp::Raw {
        sql: sql.to_string(),
        origin: RawOrigin::ServerText,
    };
    for d in DIALECTS {
        assert_eq!(
            render(d, &[raw("CREATE VIEW v AS SELECT 1 -- note\n")]),
            "CREATE VIEW v AS SELECT 1 -- note\n;\n",
            "{d:?}"
        );
        // A comment on an earlier line does not move it.
        assert_eq!(
            render(d, &[raw("CREATE VIEW v AS -- note\nSELECT 1")]),
            "CREATE VIEW v AS -- note\nSELECT 1;\n",
            "{d:?}"
        );
    }
    assert_eq!(
        render(DialectId::MySql, &[raw("CREATE VIEW v AS SELECT 1 # note")]),
        "CREATE VIEW v AS SELECT 1 # note\n;\n"
    );
    assert_eq!(hacivat_ddl::render::terminator("SELECT 1"), ";\n");
}

/// Review M5: PostgreSQL has no `GRANT … ON EXTENSION` and no `ALTER EXTENSION … OWNER TO`;
/// only `COMMENT ON EXTENSION` exists.
#[test]
fn pg_refuses_grant_and_owner_where_the_statement_does_not_exist() {
    let ctx = ctx_with(DialectId::Postgres, GenOptions::default());
    for kind in [ObjectKind::Extension, ObjectKind::Index] {
        let grant = DdlOp::Grant(GrantDef {
            kind,
            object: qn(&["app", "x"]),
            sig: None,
            privileges: names(&["USAGE"]),
            columns: Vec::new(),
            grantee: n("r"),
            grantable: false,
        });
        let owner = DdlOp::Owner {
            kind,
            name: qn(&["app", "x"]),
            sig: None,
            owner: n("o"),
        };
        for op in [grant, owner] {
            assert!(
                matches!(
                    renderer(DialectId::Postgres).render(&op, &ctx),
                    Err(RenderError::Unsupported { .. })
                ),
                "{kind:?} {}",
                op.variant()
            );
        }
    }
    let comment = DdlOp::Comment {
        target: CommentTarget {
            kind: ObjectKind::Extension,
            name: qn(&["pg_trgm"]),
            column: None,
            sig: None,
        },
        text: Some(n("trigrams")),
    };
    assert_eq!(
        render_pg(&comment),
        "COMMENT ON EXTENSION pg_trgm IS 'trigrams';\n"
    );
}

// ## m4-t33a-hygiene (T16b review m-1 carry to T19's renderer)
/// PostgreSQL's role grammar reads `public` as PUBLIC and refuses `none`, quoted or not
/// (case-sensitively, after the lexer's folding), so a role of that name cannot be named in
/// `GRANT … TO` or `OWNER TO`: the renderer refuses it rather than grant to everyone. The
/// introspector already leaves such grants out (`KEYWORD_ROLE_NAMES`); this is the
/// renderer's own half. Other case spellings are ordinary roles and quoted; the reserved
/// role keywords (`current_user`, …) are quoted, so they name the role, never the keyword.
#[test]
fn pg_role_names_the_role_grammar_reads_as_keywords_are_refused() {
    let grant = |grantee: &str| {
        DdlOp::Grant(GrantDef {
            kind: ObjectKind::Table,
            object: qn(&["app", "t"]),
            sig: None,
            privileges: names(&["SELECT"]),
            columns: Vec::new(),
            grantee: n(grantee),
            grantable: false,
        })
    };
    let owner = |role: &str| DdlOp::Owner {
        kind: ObjectKind::Table,
        name: qn(&["app", "t"]),
        sig: None,
        owner: n(role),
    };
    let pg = ctx_with(DialectId::Postgres, GenOptions::default());
    for role in ["public", "none"] {
        for op in [grant(role), owner(role)] {
            assert!(
                matches!(
                    render_text(std::slice::from_ref(&op), &pg),
                    Err(RenderError::Invalid(_))
                ),
                "{role}: {}",
                op.variant()
            );
        }
    }
    // Fix round 1 (review N2): owners come from `relowner`, never from the grant sentinel, so
    // an owner `PUBLIC` is a role of that upper-case name. Quoted, PostgreSQL reads it as an
    // identifier that is not `public`: correct DDL for that role.
    assert_eq!(
        render_pg(&owner("PUBLIC")),
        "ALTER TABLE app.t OWNER TO \"PUBLIC\";\n"
    );
    // The grant's `PUBLIC` stays the keyword (the facet's sentinel).
    assert_eq!(
        render_pg(&grant("PUBLIC")),
        "GRANT SELECT ON TABLE app.t TO PUBLIC;\n"
    );
    assert_eq!(
        render_pg(&grant("Public")),
        "GRANT SELECT ON TABLE app.t TO \"Public\";\n"
    );
    assert_eq!(
        render_pg(&grant("current_user")),
        "GRANT SELECT ON TABLE app.t TO \"current_user\";\n"
    );
    assert_eq!(
        render_pg(&owner("session_user")),
        "ALTER TABLE app.t OWNER TO \"session_user\";\n"
    );
}
// ## m4-t33a-hygiene
