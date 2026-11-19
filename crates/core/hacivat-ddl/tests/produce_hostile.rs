//! T19 reviewer hostile probes (opus review, fix round 1). The probes that proved a defect
//! (`defect_*` in the review) are flipped to assert the fix; `view_order_is_by_name_not_dependency`
//! pins a recorded limit (carry m5: dependency order for whole-schema scripts is T31/M7's).

mod fixtures;

use std::sync::Arc;

use fixtures::*;
use hacivat_ddl::GenOptions;
use hacivat_ddl::produce::WARN_NOT_ENFORCED;
use hacivat_meta::namespace::{ObjFlags, Subtree};
use hacivat_meta::overlay::Overlays;
use hacivat_meta::props::*;
use hacivat_proto::DbmsKind;
use sql_dialect::DialectId;

fn pg_tab() -> TableProps {
    TableProps {
        ext: TableExt::Pg(PgTableExt {
            relkind: b'r',
            ..PgTableExt::default()
        }),
        ..TableProps::default()
    }
}

fn my_col(column_type: &str, extra: Option<&str>, nullable: bool) -> ColumnProps {
    ColumnProps {
        ext: Some(Arc::new(ColumnExt::MySql {
            column_type: Some(n(column_type)),
            extra: extra.map(n),
            charset: None,
        })),
        ..col("", column_type, nullable)
    }
}

fn my_fx(children: Vec<Subtree>) -> Fx {
    let ns = namespace(DbmsKind::MySql, &["shop"], Props::None, children);
    Fx {
        view: view_of(DbmsKind::MySql, vec![ns], Overlays::default()),
        src: MemSources::default(),
        dbms: DbmsKind::MySql,
    }
}

/// Names needing quotes in every position, a default holding a quote, an FK into another
/// schema (qualified under Auto), a comment with a quote.
#[test]
fn quoting_in_every_position_and_cross_schema_fk() {
    let mut t = table(
        "a\"b",
        TableProps {
            owner: Some(n("Own\"er")),
            comment: Some(n("it's")),
            ..pg_tab()
        },
    );
    add(&mut t, column("id", col("", "bigint", false)));
    add(
        &mut t,
        column(
            "c\"d",
            ColumnProps {
                default_raw: Some(n("'it''s'::text")),
                comment: Some(n("x'y")),
                ..col("", "text", true)
            },
        ),
    );
    add(&mut t, column("ref id", col("", "bigint", true)));
    add(&mut t, key(K::UniqueKey, "u\"q", &["c\"d"], false));
    add(
        &mut t,
        index("u\"q", ix(&["c\"d"], &[])).add_flags(ObjFlags::IMPLICIT),
    );
    add(&mut t, index("i x", ix(&["ref id"], &[true])));
    add(
        &mut t,
        fk_obj(
            "f\"k",
            fk(&["ref id"], target(&[DB, "Oth er", "T"]), &["id"]),
        ),
    );
    let mut other = table("T", pg_tab());
    add(&mut other, column("id", col("", "bigint", false)));
    let a = namespace(DbmsKind::Postgres, &[DB, "Sch\"x"], Props::None, vec![t]);
    let b = namespace(
        DbmsKind::Postgres,
        &[DB, "Oth er"],
        Props::None,
        vec![other],
    );
    let fx = Fx {
        view: view_of(DbmsKind::Postgres, vec![a, b], Overlays::default()),
        src: MemSources::default(),
        dbms: DbmsKind::Postgres,
    };
    let mut tgt = pg_target();
    tgt.current_schema = Some(n("Sch\"x"));
    let text = fx.text(&[fx.key(K::Table, &["Sch\"x", "a\"b"])], &tgt, &no_grants());
    assert!(text.contains("CREATE TABLE \"a\"\"b\" ("), "{text}");
    assert!(
        text.contains("\"c\"\"d\" text DEFAULT 'it''s'::text"),
        "{text}"
    );
    assert!(
        text.contains("CONSTRAINT \"u\"\"q\" UNIQUE (\"c\"\"d\")"),
        "{text}"
    );
    assert!(
        text.contains(
            "CONSTRAINT \"f\"\"k\" FOREIGN KEY (\"ref id\") REFERENCES \"Oth er\".\"T\" (id)"
        ),
        "{text}"
    );
    assert!(
        text.contains("CREATE INDEX \"i x\" ON \"a\"\"b\" (\"ref id\" DESC)"),
        "{text}"
    );
    assert!(
        text.contains("COMMENT ON COLUMN \"a\"\"b\".\"c\"\"d\" IS 'x''y'"),
        "{text}"
    );
    assert!(text.contains("OWNER TO \"Own\"\"er\""), "{text}");
    // Always-qualified: the schema with a quote.
    let all = fx.text(
        &[fx.key(K::Table, &["Sch\"x", "a\"b"])],
        &tgt,
        &GenOptions {
            qualify: Qualify::Always,
            ..no_grants()
        },
    );
    assert!(
        all.contains("CREATE TABLE \"Sch\"\"x\".\"a\"\"b\" ("),
        "{all}"
    );
}

/// MySQL: an INVISIBLE generated column writes `INVISIBLE` after the generated clause
/// (MySQL's grammar wants `type [COLLATE] GENERATED … [attrs]`). Review I4, flipped.
#[test]
fn mysql_invisible_generated_column_syntax() {
    let mut t = table("t", TableProps::default());
    add(&mut t, column("a", my_col("int", None, true)));
    add(
        &mut t,
        column(
            "g",
            ColumnProps {
                hidden: true,
                rare: rare(ColumnRare {
                    generated: Some(Generated {
                        kind: GeneratedKind::Virtual,
                        expr: n("(`a` + 1)"),
                    }),
                    ..ColumnRare::default()
                }),
                ..my_col("int", Some("VIRTUAL GENERATED INVISIBLE"), true)
            },
        ),
    );
    let fx = my_fx(vec![t]);
    let text = fx.text(
        &[fx.key(K::Table, &["shop", "t"])],
        &mysql_target(DialectId::MySql),
        &no_grants(),
    );
    assert!(
        text.contains("g int GENERATED ALWAYS AS ((`a` + 1)) VIRTUAL INVISIBLE"),
        "{text}"
    );
    assert!(!text.contains("int INVISIBLE GENERATED"), "{text}");
}

/// MySQL: `EXTRA` = `DEFAULT_GENERATED on update CURRENT_TIMESTAMP INVISIBLE` (an invisible
/// auto-updated timestamp): the ON UPDATE expression is one token, the trailing word is the
/// column attribute, written once. Review I4, flipped.
#[test]
fn mysql_on_update_stops_at_its_expression() {
    let mut t = table("t", TableProps::default());
    add(
        &mut t,
        column(
            "u",
            ColumnProps {
                hidden: true,
                default_raw: Some(n("CURRENT_TIMESTAMP")),
                ..my_col(
                    "timestamp",
                    Some("DEFAULT_GENERATED on update CURRENT_TIMESTAMP INVISIBLE"),
                    true,
                )
            },
        ),
    );
    let fx = my_fx(vec![t]);
    let text = fx.text(
        &[fx.key(K::Table, &["shop", "t"])],
        &mysql_target(DialectId::MySql),
        &no_grants(),
    );
    assert!(
        text.contains(
            "u timestamp NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP INVISIBLE\n"
        ),
        "{text}"
    );
    assert_eq!(text.matches("INVISIBLE").count(), 1, "{text}");
}

/// MySQL: a unique key whose backing index is DESC, INVISIBLE and commented keeps all three
/// (it is written from its index). Review I1, flipped.
#[test]
fn mysql_unique_key_index_properties_kept() {
    let mut t = table("t", TableProps::default());
    add(&mut t, column("a", my_col("int", None, false)));
    add(&mut t, key(K::UniqueKey, "uq_a", &["a"], false));
    let mut ip = ix(&["a"], &[true]);
    ip.key.unique = true;
    ip.key.invisible = true;
    ip.comment = Some(n("why"));
    add(&mut t, index("uq_a", ip).add_flags(ObjFlags::IMPLICIT));
    let fx = my_fx(vec![t]);
    let text = fx.text(
        &[fx.key(K::Table, &["shop", "t"])],
        &mysql_target(DialectId::MySql),
        &no_grants(),
    );
    assert!(!text.contains("UNIQUE KEY"), "{text}");
    assert!(
        text.contains("CREATE UNIQUE INDEX uq_a ON t (a DESC) COMMENT 'why' INVISIBLE;"),
        "{text}"
    );
}

/// PostgreSQL: a disabled trigger (`tgenabled = 'D'`, `enabled = false`) is re-created and
/// then disabled, as pg_dump does. Review I3, flipped.
#[test]
fn pg_disabled_trigger_stays_disabled() {
    let mut fx = tiny();
    let src = fx
        .src
        .add("CREATE TRIGGER tr BEFORE UPDATE ON shop.t FOR EACH ROW EXECUTE FUNCTION shop.f()");
    let mut t = table("t", pg_tab());
    add(&mut t, column("id", col("", "integer", false)));
    add(
        &mut t,
        obj(
            K::Trigger,
            "tr",
            Props::Trigger(Arc::new(TriggerProps {
                source: Some(src),
                enabled: false,
                ..TriggerProps::default()
            })),
        ),
    );
    let ns = namespace(DbmsKind::Postgres, &[DB, "shop"], Props::None, vec![t]);
    fx.view = view_of(DbmsKind::Postgres, vec![ns], Overlays::default());
    let s = fx.script(
        &[fx.key(K::Table, &["shop", "t"])],
        &pg_target(),
        &no_grants(),
    );
    let text = s.text(hacivat_ddl::script::DelimStyle::Semicolon);
    let create = text.find("CREATE TRIGGER tr").expect("trigger");
    let disable = text
        .find("ALTER TABLE t DISABLE TRIGGER tr;")
        .expect("disabled");
    assert!(create < disable, "{text}");
}

/// A NOT ENFORCED check (MySQL 8.0.16+ `ENFORCED = 'NO'`, PG 18 `conenforced = false`) is
/// written `NOT ENFORCED`, never as an enforced CHECK. Review I2, flipped.
#[test]
fn not_enforced_check_written_as_not_enforced() {
    let mut t = table("t", TableProps::default());
    add(&mut t, column("a", my_col("int", None, true)));
    add(
        &mut t,
        obj(
            K::Check,
            "t_chk",
            Props::Check(Arc::new(CheckProps {
                expr: n("(`a` > 0)"),
                not_valid: false,
                enforced: false,
            })),
        ),
    );
    let fx = my_fx(vec![t]);
    let s = fx.script(
        &[fx.key(K::Table, &["shop", "t"])],
        &mysql_target(DialectId::MySql),
        &no_grants(),
    );
    let text = s.text(hacivat_ddl::script::DelimStyle::Semicolon);
    assert!(
        text.contains("ALTER TABLE t ADD CONSTRAINT t_chk CHECK ((`a` > 0)) NOT ENFORCED;"),
        "{text}"
    );
    assert!(!text.contains("    CONSTRAINT t_chk"), "{text}");
    assert!(!s.has_notice(WARN_NOT_ENFORCED));
}

/// Views are ordered by name, not dependency: `a_v` (reading `b_v`) is created first.
#[test]
fn view_order_is_by_name_not_dependency() {
    let mut fx = tiny();
    let a = fx.src.add(" SELECT b_v.x\n   FROM shop.b_v");
    let b = fx.src.add(" SELECT 1 AS x");
    let v = |name: &str, s| {
        leaf(obj(
            K::View,
            name,
            Props::View(Arc::new(ViewProps {
                source: Some(s),
                ..ViewProps::default()
            })),
        ))
    };
    let ns = namespace(
        DbmsKind::Postgres,
        &[DB, "shop"],
        Props::None,
        vec![v("a_v", a), v("b_v", b)],
    );
    fx.view = view_of(DbmsKind::Postgres, vec![ns], Overlays::default());
    let text = fx.text(&[fx.key(K::Schema, &["shop"])], &pg_target(), &no_grants());
    assert!(text.find("VIEW a_v").unwrap() < text.find("VIEW b_v").unwrap());
}

/// A PostgreSQL index whose name starts `sqlite_autoindex_` is an ordinary index there.
/// Review m1, flipped.
#[test]
fn pg_index_named_like_a_sqlite_autoindex_is_kept() {
    let mut fx = tiny();
    let mut t = table("t", pg_tab());
    add(&mut t, column("id", col("", "integer", false)));
    add(&mut t, index("sqlite_autoindex_t_1", ix(&["id"], &[])));
    let ns = namespace(DbmsKind::Postgres, &[DB, "shop"], Props::None, vec![t]);
    fx.view = view_of(DbmsKind::Postgres, vec![ns], Overlays::default());
    let text = fx.text(
        &[fx.key(K::Table, &["shop", "t"])],
        &pg_target(),
        &no_grants(),
    );
    assert!(
        text.contains("CREATE INDEX sqlite_autoindex_t_1 ON t (id);"),
        "{text}"
    );
}

fn tiny() -> Fx {
    let ns = namespace(DbmsKind::Postgres, &[DB, "shop"], Props::None, vec![]);
    Fx {
        view: view_of(DbmsKind::Postgres, vec![ns], Overlays::default()),
        src: MemSources::default(),
        dbms: DbmsKind::Postgres,
    }
}
