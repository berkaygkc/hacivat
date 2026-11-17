//! IR renderers (plan-m4 Task 7): alg/03 HV-DDL-07 tests 1–2, the dialect rules of the
//! renderers, every alg/03 variant declared with the non-M4 ones refusing, and IR goldens.

mod fixtures;

use fixtures::{check_golden, n, names};
use hacivat_ddl::ir::*;
use hacivat_ddl::produce::qualify::QualifyPlan;
use hacivat_ddl::render::{RenderCtx, render_text};
use hacivat_ddl::{DdlOp, DdlTarget, RenderError, renderer};
use hacivat_meta::ObjectKind;
use hacivat_meta::props::{Deferrable, FkAction, FkMatch};
use sql_dialect::DialectId;

const DIALECTS: [DialectId; 4] = [
    DialectId::Postgres,
    DialectId::MySql,
    DialectId::MariaDb,
    DialectId::Sqlite,
];

fn render(d: DialectId, plan: &QualifyPlan, ops: &[DdlOp]) -> String {
    let t = DdlTarget::new(d);
    render_text(ops, &RenderCtx::new(&t, plan)).expect("render")
}

fn render_with(t: &DdlTarget, ops: &[DdlOp]) -> Result<String, RenderError> {
    render_text(ops, &RenderCtx::new(t, &QualifyPlan::always(t.dialect)))
}

fn col(name: &str, ty: &str) -> ColumnDef {
    ColumnDef {
        name: n(name),
        type_text: n(ty),
        nullable: true,
        ..Default::default()
    }
}

fn create_table(schema: &str, name: &str, cols: Vec<ColumnDef>) -> DdlOp {
    DdlOp::CreateTable(TableDef {
        name: QName::new(Some(schema), name),
        columns: cols,
        ..Default::default()
    })
}

fn pk(name: Option<&str>, cols: &[&str], position: Position) -> ConstraintDef {
    ConstraintDef {
        name: name.map(n),
        body: ConstraintBody::PrimaryKey {
            columns: names(cols),
        },
        deferrable: Deferrable::No,
        not_valid: false,
        position,
    }
}

fn fk_def(position: Position) -> ConstraintDef {
    ConstraintDef {
        name: Some(n("fk_customer")),
        body: ConstraintBody::ForeignKey {
            columns: names(&["customer_id"]),
            target: QName::new(Some("crm"), "customers"),
            target_columns: names(&["id"]),
            on_update: FkAction::NoAction,
            on_delete: FkAction::Cascade,
            match_kind: FkMatch::Simple,
        },
        deferrable: Deferrable::No,
        not_valid: false,
        position,
    }
}

#[test]
fn pg_table_canonical_type_null_omitted() {
    let op = create_table("test_schema", "test_table", vec![col("column1", "integer")]);
    assert_eq!(
        render(
            DialectId::Postgres,
            &QualifyPlan::always(DialectId::Postgres),
            &[op]
        ),
        "CREATE TABLE test_schema.test_table (\n    column1 integer\n);\n"
    );
}

#[test]
fn pg_comment_escapes_quote() {
    let op = DdlOp::Comment {
        target: CommentTarget {
            kind: ObjectKind::Column,
            name: QName::new(Some("test_schema"), "test_table"),
            column: Some(n("column1")),
            sig: None,
            on: None,
        },
        text: Some(n("PM's approver of record")),
    };
    assert_eq!(
        render(
            DialectId::Postgres,
            &QualifyPlan::always(DialectId::Postgres),
            &[op]
        ),
        "COMMENT ON COLUMN test_schema.test_table.column1 IS 'PM''s approver of record';\n"
    );
}

#[test]
fn comment_null_removes() {
    let op = DdlOp::Comment {
        target: CommentTarget {
            kind: ObjectKind::Table,
            name: QName::bare("t"),
            column: None,
            sig: None,
            on: None,
        },
        text: None,
    };
    assert_eq!(
        render(
            DialectId::Postgres,
            &QualifyPlan::never(DialectId::Postgres),
            &[op]
        ),
        "COMMENT ON TABLE t IS NULL;\n"
    );
}

#[test]
fn mysql_comment_escaping_respects_no_backslash_escapes() {
    let mut c = col("note", "text");
    c.comment = Some(n("it's C:\\tmp"));
    let op = create_table("shop", "t", vec![c]);
    let mut t = DdlTarget::new(DialectId::MySql);
    assert!(
        render_with(&t, std::slice::from_ref(&op))
            .unwrap()
            .contains("COMMENT 'it''s C:\\\\tmp'")
    );
    t.mysql_no_backslash_escapes = true;
    assert!(
        render_with(&t, &[op])
            .unwrap()
            .contains("COMMENT 'it''s C:\\tmp'")
    );
}

#[test]
fn mysql_fk_is_never_inline() {
    let op = DdlOp::CreateTable(TableDef {
        name: QName::new(Some("shop"), "orders"),
        columns: vec![col("customer_id", "bigint")],
        constraints: vec![fk_def(Position::InColumn(n("customer_id")))],
        ..Default::default()
    });
    let text = render_with(&DdlTarget::new(DialectId::MySql), &[op]).unwrap();
    assert_eq!(
        text,
        "CREATE TABLE shop.orders (\n    customer_id bigint,\n    CONSTRAINT fk_customer FOREIGN KEY (customer_id) REFERENCES crm.customers (id) ON DELETE CASCADE\n);\n"
    );
}

#[test]
fn inside_column_single_column_pk() {
    let op = DdlOp::CreateTable(TableDef {
        name: QName::bare("t"),
        columns: vec![col("id", "integer")],
        constraints: vec![pk(Some("pk"), &["id"], Position::InColumn(n("id")))],
        ..Default::default()
    });
    let text = render(
        DialectId::Postgres,
        &QualifyPlan::never(DialectId::Postgres),
        &[op],
    );
    assert!(
        text.contains("id integer CONSTRAINT pk PRIMARY KEY"),
        "{text}"
    );
}

#[test]
fn turkish_identifier_quoted_on_pg() {
    let op = create_table("public", "MÜŞTERİ", vec![col("ad", "text")]);
    let text = render(
        DialectId::Postgres,
        &QualifyPlan::always(DialectId::Postgres),
        &[op],
    );
    assert!(text.contains(r#"public."MÜŞTERİ""#), "{text}");
    assert!(text.contains("\n    ad text\n"), "{text}");
}

#[test]
fn reserved_word_is_quoted_per_dialect() {
    let op = create_table("s", "order", vec![col("x", "int")]);
    let plan = |d| QualifyPlan::never(d);
    assert!(
        render(
            DialectId::Postgres,
            &plan(DialectId::Postgres),
            std::slice::from_ref(&op)
        )
        .starts_with("CREATE TABLE \"order\" (")
    );
    assert!(
        render(
            DialectId::MySql,
            &plan(DialectId::MySql),
            std::slice::from_ref(&op)
        )
        .starts_with("CREATE TABLE `order` (")
    );
    assert!(
        render(
            DialectId::MariaDb,
            &plan(DialectId::MariaDb),
            std::slice::from_ref(&op)
        )
        .starts_with("CREATE TABLE `order` (")
    );
    assert!(
        render(DialectId::Sqlite, &plan(DialectId::Sqlite), &[op])
            .starts_with("CREATE TABLE \"order\" (")
    );
}

#[test]
fn server_text_is_never_rewritten() {
    let text = "CREATE VIEW \"Weird\" AS  select  1 /* keep */ ;\n\n";
    for d in DIALECTS {
        let op = DdlOp::Raw {
            sql: n(text),
            origin: RawOrigin::ServerText,
        };
        let out = render(d, &QualifyPlan::always(d), &[op]);
        // Only the trailing whitespace and the one `;` go; the script adds its own.
        assert_eq!(
            out, "CREATE VIEW \"Weird\" AS  select  1 /* keep */;\n",
            "{d:?}"
        );
    }
}

#[test]
fn nul_in_comment_is_refused() {
    let mut c = col("a", "int");
    c.comment = Some(n("x\0y"));
    for d in [DialectId::Postgres, DialectId::MySql] {
        let r = render_with(
            &DdlTarget::new(d),
            &[create_table("s", "t", vec![c.clone()])],
        );
        assert_eq!(r, Err(RenderError::Nul { what: "comment" }), "{d:?}");
    }
}

#[test]
fn sqlite_autoincrement_needs_its_inline_primary_key() {
    let mut c = col("id", "INTEGER");
    c.identity = Some(IdentityDef::AutoIncrement);
    let table = |position| {
        DdlOp::CreateTable(TableDef {
            name: QName::new(Some("main"), "t"),
            columns: vec![c.clone()],
            constraints: vec![pk(None, &["id"], position)],
            ..Default::default()
        })
    };
    let t = DdlTarget::new(DialectId::Sqlite);
    let plan = QualifyPlan::new(hacivat_ddl::Qualify::Auto, DialectId::Sqlite, None);
    let ctx = RenderCtx::new(&t, &plan);
    assert_eq!(
        render_text(&[table(Position::InColumn(n("id")))], &ctx).unwrap(),
        "CREATE TABLE t (\n    id INTEGER PRIMARY KEY AUTOINCREMENT\n);\n"
    );
    assert!(matches!(
        render_text(&[table(Position::InTable)], &ctx),
        Err(RenderError::Invalid(_))
    ));
}

/// One op of every alg/03 variant (A-8).
fn every_variant() -> Vec<DdlOp> {
    let q = QName::new(Some("s"), "t");
    let grant = GrantDef {
        kind: ObjectKind::Table,
        object: q.clone(),
        sig: None,
        privileges: names(&["SELECT"]),
        columns: vec![],
        grantee: n("r"),
        with_grant_option: false,
    };
    vec![
        DdlOp::CreateSchema(SchemaDef {
            name: n("s"),
            ..Default::default()
        }),
        create_table("s", "t", vec![col("a", "int")]),
        DdlOp::AlterTable {
            table: q.clone(),
            actions: vec![AlterAction::AddColumn(col("b", "int"))],
        },
        DdlOp::DropObject {
            kind: ObjectKind::Table,
            name: q.clone(),
            sig: None,
            if_exists: true,
            cascade: false,
        },
        DdlOp::CreateIndex(IndexDef {
            name: n("i"),
            table: q.clone(),
            parts: vec![IndexPartDef {
                column: Some(n("a")),
                ..Default::default()
            }],
            ..Default::default()
        }),
        DdlOp::CreateView(ViewDef {
            name: QName::new(Some("s"), "v"),
            kind: ViewKind::View,
            or_replace: false,
            body: ViewBody::Query(n("SELECT 1")),
            check_option: None,
            comment: None,
            column_comments: vec![],
        }),
        DdlOp::CreateRoutine(RoutineDef {
            name: QName::new(Some("s"), "f"),
            kind: ObjectKind::Routine,
            sig: Some(n("")),
            text: n("CREATE FUNCTION s.f() RETURNS int RETURN 1"),
            comment: None,
        }),
        DdlOp::CreateTrigger(TriggerDef {
            name: n("tr"),
            table: q.clone(),
            text: n("CREATE TRIGGER tr AFTER INSERT ON t FOR EACH ROW SELECT 1"),
            comment: None,
        }),
        DdlOp::CreateSequence(SeqDef {
            name: QName::new(Some("s"), "seq"),
            ..Default::default()
        }),
        DdlOp::AlterSequence {
            name: QName::new(Some("s"), "seq"),
            actions: vec![SeqAction::OwnedBy(None)],
        },
        DdlOp::CreateType(TypeDef {
            name: QName::new(Some("s"), "e"),
            body: TypeBody::Enum {
                labels: names(&["a"]),
            },
            comment: None,
        }),
        DdlOp::Rename {
            kind: ObjectKind::Table,
            from: q.clone(),
            to: n("u"),
        },
        DdlOp::SetSchema {
            kind: ObjectKind::Table,
            name: q.clone(),
            to: n("other"),
        },
        DdlOp::Comment {
            target: CommentTarget {
                kind: ObjectKind::Table,
                name: q.clone(),
                column: None,
                sig: None,
                on: None,
            },
            text: Some(n("c")),
        },
        DdlOp::Grant(grant.clone()),
        DdlOp::Revoke(grant),
        DdlOp::Owner {
            kind: ObjectKind::Table,
            name: q.clone(),
            sig: None,
            owner: n("o"),
        },
        DdlOp::TableRebuild(rebuild(q.clone())),
        DdlOp::Exchange {
            a: q.clone(),
            b: QName::new(Some("s"), "u"),
        },
        DdlOp::Raw {
            sql: n("SELECT 1"),
            origin: RawOrigin::ServerText,
        },
        DdlOp::Raw {
            sql: n("SELECT 1"),
            origin: RawOrigin::UserEdited,
        },
        DdlOp::Data(DataStep::RefreshMaterializedView(q)),
    ]
}

fn rebuild(q: QName) -> RebuildPlan {
    RebuildPlan::new(q, TableDef::default(), Vec::new())
}

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

#[test]
fn every_alg03_variant_is_declared_and_non_m4_ones_refuse() {
    let ops = every_variant();
    let mut seen: Vec<&str> = ops.iter().map(DdlOp::variant).collect();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 21, "{seen:?}");
    for d in DIALECTS {
        let r = renderer(d).expect("renderer");
        let t = DdlTarget::new(d);
        let plan = QualifyPlan::always(d);
        let ctx = RenderCtx::new(&t, &plan);
        for op in &ops {
            let res = r.render(op, &ctx);
            let user_edited = matches!(
                op,
                DdlOp::Raw {
                    origin: RawOrigin::UserEdited,
                    ..
                }
            );
            if NOT_IN_M4.contains(&op.variant()) || user_edited {
                assert!(
                    matches!(res, Err(RenderError::Unsupported { .. })),
                    "{d:?} {}: {res:?}",
                    op.variant()
                );
            }
        }
    }
}

#[test]
fn unsupported_variant_is_an_explicit_error() {
    let op = DdlOp::Rename {
        kind: ObjectKind::Table,
        from: QName::bare("a"),
        to: n("b"),
    };
    for d in DIALECTS {
        let t = DdlTarget::new(d);
        assert_eq!(
            render_with(&t, std::slice::from_ref(&op)),
            Err(RenderError::Unsupported {
                op: "Rename",
                dialect: d
            })
        );
    }
}

#[test]
fn sqlite_has_no_schemas_sequences_types_or_comments() {
    let t = DdlTarget::new(DialectId::Sqlite);
    for op in every_variant() {
        if matches!(
            op.variant(),
            "CreateSchema"
                | "CreateSequence"
                | "AlterSequence"
                | "CreateType"
                | "Comment"
                | "Grant"
                | "Owner"
        ) {
            assert!(
                matches!(
                    render_with(&t, std::slice::from_ref(&op)),
                    Err(RenderError::Unsupported { .. })
                ),
                "{}",
                op.variant()
            );
        }
    }
}

/// A rich table rendered by every dialect's renderer (IR golden, R-M4-16).
fn ir_table(d: DialectId) -> Vec<DdlOp> {
    let mut id = col(
        "id",
        if d == DialectId::Sqlite {
            "INTEGER"
        } else {
            "bigint"
        },
    );
    id.nullable = false;
    id.identity = Some(match d {
        DialectId::Postgres => IdentityDef::ByDefault,
        _ => IdentityDef::AutoIncrement,
    });
    let mut name = col("name", "varchar(80)");
    name.nullable = false;
    name.default = Some(n("'?'"));
    name.comment = Some(n("Display name"));
    let pk_pos = if d == DialectId::Sqlite {
        Position::InColumn(n("id"))
    } else {
        Position::InTable
    };
    let ext = match d {
        DialectId::Postgres => TableDefExt::Pg(PgTableDef {
            with: names(&["fillfactor=80"]),
            ..Default::default()
        }),
        DialectId::Sqlite => TableDefExt::Sqlite(SqliteTableDef {
            strict: false,
            without_rowid: false,
        }),
        _ => TableDefExt::MySql(MySqlTableDef {
            engine: Some(n("InnoDB")),
            ..Default::default()
        }),
    };
    let mut constraints = vec![
        pk(Some("people_pk"), &["id"], pk_pos),
        ConstraintDef {
            name: Some(n("people_name_ck")),
            body: ConstraintBody::Check {
                expr: n("name <> ''"),
            },
            deferrable: Deferrable::No,
            not_valid: false,
            position: Position::InTable,
        },
    ];
    if d != DialectId::Sqlite {
        constraints.push(fk_def(Position::InTable));
    }
    let mut cols = vec![id, name];
    if d != DialectId::Sqlite {
        cols.push(col("customer_id", "bigint"));
    }
    vec![
        DdlOp::CreateTable(TableDef {
            name: QName::new(Some("app"), "people"),
            columns: cols,
            constraints,
            comment: Some(n("People")),
            ext,
            ..Default::default()
        }),
        DdlOp::CreateIndex(IndexDef {
            name: n("people_name_idx"),
            table: QName::new(Some("app"), "people"),
            unique: true,
            parts: vec![IndexPartDef {
                column: Some(n("name")),
                desc: true,
                ..Default::default()
            }],
            ..Default::default()
        }),
    ]
}

#[test]
fn ir_goldens_byte_stable() {
    for (d, dir) in [
        (DialectId::Postgres, "pg"),
        (DialectId::MySql, "mysql"),
        (DialectId::Sqlite, "sqlite"),
    ] {
        let text = render(d, &QualifyPlan::always(d), &ir_table(d));
        check_golden(&format!("ir/{dir}/create_table"), &text);
        assert_eq!(text, render(d, &QualifyPlan::always(d), &ir_table(d)));
    }
}
