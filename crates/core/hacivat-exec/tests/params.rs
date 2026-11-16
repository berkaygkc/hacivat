//! M3b Task 15: parameter preparation (HV-EXEC-03 binding; SQL-PARAM-01..03 through Task 6's
//! `scan_params`): prompts, binds in slot order, the rewrite that is classified and sent
//! (P-M3b-2), refusals (any unbindable placeholder refuses the unit, nothing is sent), masking
//! and the offset map.

mod common;

use common::gate_fx::{Values, ctx_of, exec_units, prepared_on, text_value};
use hacivat_exec::{
    OffsetMap, Prepared, PreparedUnit, mask_secrets, param_diag_message_id, prepare_units,
};
use hacivat_proto::{DbmsKind, ProfileId};
use hacivat_values::BindValue;
use sql_split::{ParamDiag, ParamSettings};

fn prepare(
    engine: DbmsKind,
    texts: &[&str],
    values: &[(&str, &str)],
    s: ParamSettings,
) -> Prepared {
    prepare_units(
        exec_units(texts),
        &ctx_of(engine),
        &s,
        &Values::of(values),
        &ProfileId::new(),
    )
}

fn pg(texts: &[&str], values: &[(&str, &str)]) -> Prepared {
    prepare(DbmsKind::Postgres, texts, values, ParamSettings::default())
}

fn ready(p: Prepared) -> Vec<PreparedUnit> {
    match p {
        Prepared::Ready(u) => u,
        other => panic!("{other:?}"),
    }
}

fn refused(p: Prepared) -> (u32, std::ops::Range<usize>, ParamDiag) {
    match p {
        Prepared::Refused { unit, at, diag } => (unit, at, diag),
        other => panic!("not refused: {other:?}"),
    }
}

/// `text` prepared on PostgreSQL with `values`, placeholders in DDL bindable (the plan's
/// `prepared`).
fn prepared(text: &str, values: &[(&str, &str)]) -> PreparedUnit {
    prepared_on(DbmsKind::Postgres, text, values)
}

fn texts(binds: &[BindValue]) -> Vec<&str> {
    binds
        .iter()
        .map(|b| match b {
            BindValue::Typed { text, .. } => &**text,
            BindValue::Null => "NULL",
        })
        .collect()
}

#[test]
fn masking_alter_role_password() {
    assert!(
        !prepared("ALTER ROLE app PASSWORD :pwd", &[("pwd", "s3cret")])
            .masked
            .contains("s3cret")
    );
    // `alg/20` HV-EXEC-03 case 7: the text says `PASSWORD $1`; the value travels as a bind.
    let u = prepared("ALTER ROLE app PASSWORD :pwd", &[("pwd", "s3cret")]);
    assert_eq!(&*u.final_text, "ALTER ROLE app PASSWORD $1");
    assert_eq!(&*u.masked, "ALTER ROLE app PASSWORD $1");
    assert_eq!(texts(&u.binds), ["s3cret"]);
    // Without `in_ddl` the placeholder in DDL refuses the unit (Task 6's rule).
    let (_, _, diag) = refused(pg(&["ALTER ROLE app PASSWORD :pwd"], &[("pwd", "s3cret")]));
    assert_eq!(diag, ParamDiag::ParamInDdl);
}

#[test]
fn literal_secrets_are_masked() {
    for (engine, sql) in [
        (DbmsKind::Postgres, "ALTER ROLE app WITH PASSWORD 's3cret'"),
        (
            DbmsKind::Postgres,
            "CREATE USER bob PASSWORD 's3cret' VALID UNTIL 'infinity'",
        ),
        (
            DbmsKind::Postgres,
            "SELECT dblink_connect('host=db password=s3cret')",
        ),
        (
            DbmsKind::Postgres,
            "SELECT dblink_connect('host=db pwd=s3cret')",
        ),
        (DbmsKind::Postgres, "ALTER ROLE app PASSWORD $$s3cret$$"),
        (DbmsKind::MySql, "CREATE USER bob IDENTIFIED BY 's3cret'"),
        (
            DbmsKind::MySql,
            "ALTER USER bob IDENTIFIED WITH mysql_native_password BY \"s3cret\"",
        ),
        (DbmsKind::MySql, "SET PASSWORD FOR bob = 's3cret'"),
        (
            DbmsKind::MySql,
            "CREATE USER bob /*!80000 IDENTIFIED BY 's3cret' */",
        ),
        (
            DbmsKind::MariaDb,
            "GRANT ALL ON *.* TO bob IDENTIFIED BY 's3cret'",
        ),
        // Unterminated: the rest of the text is masked.
        (DbmsKind::Postgres, "ALTER ROLE app PASSWORD 's3cret"),
    ] {
        let u = prepared_on(engine, sql, &[]);
        assert_eq!(&*u.final_text, sql, "sent as written");
        assert!(
            !u.masked.contains("s3cret"),
            "{engine:?} {sql}: {}",
            u.masked
        );
        assert!(u.masked.contains("***"), "{sql}");
    }
    // A password mentioned anywhere masks every literal (over-masking is accepted: the masked
    // text is never sent); text that names no password is untouched.
    let u = prepared("SELECT 'a', 'password_hint' FROM t /* note */", &[]);
    assert_eq!(&*u.masked, "SELECT '***', '***' FROM t /* *** */");
    let u = prepared("SELECT 'hello' FROM t WHERE a = :a", &[("a", "1")]);
    assert_eq!(&*u.masked, "SELECT 'hello' FROM t WHERE a = $1");
    // The masked text never holds a bind value.
    for (sql, v) in [
        ("SELECT :a", "s3cret"),
        ("UPDATE users SET password = :p WHERE id = 1", "s3cret"),
    ] {
        let u = prepared(sql, &[("a", v), ("p", v)]);
        assert!(!u.masked.contains(v), "{sql}");
    }
    let ctx = ctx_of(DbmsKind::Sqlite);
    assert!(
        !mask_secrets("ATTACH 'x.db' AS a KEY 'k' -- password 's3cret'", &ctx).contains("s3cret")
    );
}

#[test]
fn more_than_65535_binds_refused() {
    let text = format!("SELECT {}1", "?, ".repeat(65_536));
    let (unit, at, diag) = refused(prepare(
        DbmsKind::MySql,
        &["SELECT 1", &text],
        &[],
        ParamSettings::default(),
    ));
    assert_eq!((unit, diag), (1, ParamDiag::TooManyBinds));
    // At the first placeholder past the limit, in document offsets (the second unit starts at
    // byte 10).
    assert_eq!(at.start, 10 + "SELECT ".len() + 65_535 * 3);
    // 65,535 is still fine: it asks for every value.
    let text = format!("SELECT {}1", "?, ".repeat(65_535));
    match prepare(DbmsKind::MySql, &[&text], &[], ParamSettings::default()) {
        Prepared::NeedsValues(p) => assert_eq!(p.len(), 65_535),
        other => panic!("{other:?}"),
    }
}

#[test]
fn offset_map_round_trips() {
    for (text, values) in [
        (
            "SELECT :a, :bb, :a FROM t WHERE x = :ccc",
            vec![("a", "1"), ("bb", "2"), ("ccc", "3")],
        ),
        (
            "SELECT * FROM t WHERE müşteri = :müşteri_adı AND x = 1",
            vec![("müşteri_adı", "x")],
        ),
        ("SELECT $1, :a", vec![]),
    ] {
        let p = pg(
            &["SELECT 0", text],
            &values.iter().map(|(a, b)| (*a, *b)).collect::<Vec<_>>(),
        );
        let u = match p {
            Prepared::Ready(mut u) => u.remove(1),
            Prepared::Refused { diag, .. } => {
                assert_eq!(diag, ParamDiag::MixedStyles, "{text}");
                continue;
            }
            other => panic!("{other:?}"),
        };
        let start = u.unit.doc_range.start;
        assert_eq!(start, 10);
        // Walk both texts: every byte outside a placeholder maps back to itself.
        let (orig, fin) = (text.as_bytes(), u.final_text.as_bytes());
        let (mut i, mut j) = (0, 0);
        while i < orig.len() && j < fin.len() {
            if orig[i] == b':' && fin[j] == b'$' {
                // A placeholder: every offset inside it maps to its start.
                let ph_end = i
                    + 1
                    + text[i + 1..]
                        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                        .unwrap_or(text.len() - i - 1);
                let rep_end = j
                    + 1
                    + u.final_text[j + 1..]
                        .find(|c: char| !c.is_ascii_digit())
                        .unwrap_or(fin.len() - j - 1);
                for k in j..rep_end {
                    assert_eq!(u.offsets.to_doc(k), start + i, "{text}: inside at {k}");
                }
                i = ph_end;
                j = rep_end;
                continue;
            }
            assert_eq!(orig[i], fin[j]);
            assert_eq!(u.offsets.to_doc(j), start + i, "{text}: byte {j}");
            i += 1;
            j += 1;
        }
        assert_eq!((i, j), (orig.len(), fin.len()));
        assert_eq!(
            u.offsets.to_doc(fin.len()),
            start + orig.len(),
            "the end maps to the end"
        );
    }
    // A unit with no placeholder maps by identity.
    let u = ready(pg(&["SELECT 0", "SELECT 1"], &[])).remove(1);
    assert_eq!(u.offsets, OffsetMap::identity(10));
    assert_eq!(u.offsets.to_doc(3), 13);
}

#[test]
fn needs_values_names_each_parameter_once_with_its_remembered_value() {
    match pg(
        &["SELECT :a, :b", "SELECT :b, :a, :c"],
        &[("b", "remembered")],
    ) {
        Prepared::NeedsValues(p) => {
            let names: Vec<(&str, u32)> = p.iter().map(|q| (&*q.name, q.occurrences)).collect();
            assert_eq!(names, [("a", 2), ("b", 2), ("c", 1)]);
            assert_eq!(p[1].remembered, Some(text_value("remembered")));
            assert_eq!(
                (p[0].remembered.clone(), p[2].remembered.clone()),
                (None, None)
            );
        }
        other => panic!("{other:?}"),
    }
    // Every value known: ready, each unit rewritten for itself (one name shared across units
    // is one value).
    let u = ready(pg(
        &["SELECT :a, :b", "SELECT :b, :a, :c"],
        &[("a", "1"), ("b", "2"), ("c", "3")],
    ));
    assert_eq!(&*u[0].final_text, "SELECT $1, $2");
    assert_eq!(texts(&u[0].binds), ["1", "2"]);
    assert_eq!(&*u[1].final_text, "SELECT $1, $2, $3");
    assert_eq!(texts(&u[1].binds), ["2", "1", "3"]);
    // Names compare exactly (`:Id` is not `:id`), Turkish letters included.
    match pg(&["SELECT :Id, :id, :İl, :il"], &[("id", "1")]) {
        Prepared::NeedsValues(p) => {
            assert_eq!(
                p.iter().map(|q| &*q.name).collect::<Vec<_>>(),
                ["Id", "id", "İl", "il"]
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn binds_follow_the_driver_slots() {
    // MySQL repeats a name once per occurrence; positional `?` keep their order.
    let u = prepared_on(
        DbmsKind::MySql,
        "SELECT :a, :b, :a",
        &[("a", "x"), ("b", "y")],
    );
    assert_eq!(&*u.final_text, "SELECT ?, ?, ?");
    assert_eq!(texts(&u.binds), ["x", "y", "x"]);
    let u = prepared_on(DbmsKind::MySql, "SELECT ?, ?", &[("?1", "x"), ("?2", "y")]);
    assert_eq!(texts(&u.binds), ["x", "y"]);
    // PostgreSQL positionals bind by their number.
    let u = prepared_on(
        DbmsKind::Postgres,
        "SELECT $2, $1",
        &[("$1", "one"), ("$2", "two")],
    );
    assert_eq!(&*u.final_text, "SELECT $2, $1");
    assert_eq!(texts(&u.binds), ["one", "two"]);
    // SQLite keeps its native forms.
    let u = prepared_on(
        DbmsKind::Sqlite,
        "SELECT ?2, ?1",
        &[("?1", "one"), ("?2", "two")],
    );
    assert_eq!(texts(&u.binds), ["one", "two"]);
    // A slot no parameter binds (`$2` alone: the server expects `$1` too) refuses the unit.
    let (unit, at, diag) = refused(pg(&["SELECT $2"], &[("$2", "x")]));
    assert_eq!((unit, at, diag), (0, 7..9, ParamDiag::Unbindable));
    // NULL is a value.
    let store = Values(vec![("a".into(), BindValue::Null)]);
    match prepare_units(
        exec_units(&["SELECT :a"]),
        &ctx_of(DbmsKind::Postgres),
        &ParamSettings::default(),
        &store,
        &ProfileId::new(),
    ) {
        Prepared::Ready(u) => assert_eq!(&*u[0].binds, [BindValue::Null]),
        other => panic!("{other:?}"),
    }
}

#[test]
fn any_unbindable_placeholder_refuses_the_unit() {
    // The first unit that cannot be sent refuses the run, at its document range; nothing of
    // it is sent (earlier units included: the caller sends nothing on `Refused`).
    for (engine, text, diag) in [
        (
            DbmsKind::Postgres,
            "SELECT :\"col\" FROM t",
            ParamDiag::Unbindable,
        ),
        (DbmsKind::Postgres, "SELECT $0", ParamDiag::Unbindable),
        (DbmsKind::Postgres, "SELECT $01", ParamDiag::Unbindable),
        (DbmsKind::Postgres, "SELECT :a, $1", ParamDiag::MixedStyles),
        (
            DbmsKind::Postgres,
            "CREATE TABLE t (a int DEFAULT :a)",
            ParamDiag::ParamInDdl,
        ),
        (
            DbmsKind::Postgres,
            "SELECT ${id} WHERE a = :a",
            ParamDiag::TemplateVariable,
        ),
        (
            DbmsKind::MySql,
            "SELECT 1 /*!50000 , ? */",
            ParamDiag::Unbindable,
        ),
        (DbmsKind::MySql, "CALL p(?)", ParamDiag::Unbindable),
        (
            DbmsKind::Postgres,
            "SELECT :a; SELECT :b",
            ParamDiag::Unbindable,
        ),
    ] {
        let p = prepare(
            engine,
            &["SELECT 1", text],
            &[("a", "1"), ("b", "2"), ("col", "c")],
            ParamSettings::default(),
        );
        let (unit, at, d) = refused(p);
        assert_eq!((unit, d), (1, diag), "{engine:?} {text}");
        assert!(
            at.start >= 10 && at.end <= 10 + text.len() && at.start <= at.end,
            "{text}: {at:?}"
        );
    }
    // A template variable alone (no placeholder) refuses too.
    let (_, _, d) = refused(pg(&["SELECT ${col} FROM t"], &[]));
    assert_eq!(d, ParamDiag::TemplateVariable);
    // `?` on PostgreSQL with the setting off is an operator: no refusal, sent as written.
    let u = ready(pg(&["SELECT j ? 'k' FROM t"], &[])).remove(0);
    assert_eq!(&*u.final_text, "SELECT j ? 'k' FROM t");
    assert!(u.binds.is_empty());
    // Markers in strings, comments and dollar bodies are no placeholders.
    let u = ready(pg(&["SELECT ':a', $$ :b $$ -- :c\n"], &[])).remove(0);
    assert!(u.binds.is_empty());
}

#[test]
fn every_param_diag_has_a_message() {
    let all = [
        ParamDiag::TemplateVariable,
        ParamDiag::QuestionMarkOff,
        ParamDiag::ParamInDdl,
        ParamDiag::MixedStyles,
        ParamDiag::TooManyBinds,
        ParamDiag::Unbindable,
    ];
    let ids: Vec<&str> = all.iter().map(|d| param_diag_message_id(*d)).collect();
    let ftl = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../locales/en/hacivat-core.ftl"
    ))
    .unwrap();
    for (i, id) in ids.iter().enumerate() {
        assert!(id.starts_with("exec-param-"), "{id}");
        assert!(!ids[..i].contains(id), "{id} twice");
        assert!(
            ftl.lines().any(|l| l.starts_with(&format!("{id} ="))),
            "{id} missing"
        );
    }
}
