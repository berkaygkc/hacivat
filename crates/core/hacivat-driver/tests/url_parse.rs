//! Parsing pasted connection text (HV-DRV-04): native forms, the template fallback, secret
//! extraction, parameter mapping and the paste rule (R21).
//!
//! Fixture passwords (`s3cret`, `p@ss`) appear only in asserts that prove where they went.

use hacivat_driver::url::{
    FieldId, MappedTo, ParseOutcome, ParseResult, ParseWarning, UnrecognisedReason,
    looks_like_connection, parse,
};
use hacivat_net::TlsMode;
use hacivat_proto::DbmsKind;
use secrecy::ExposeSecret;

fn ok(r: ParseResult) -> ParseOutcome {
    match r {
        ParseResult::Recognised(o) => o,
        ParseResult::Unrecognised(why) => panic!("unrecognised: {why:?}"),
    }
}

fn fv(o: &ParseOutcome, f: FieldId) -> String {
    o.fields
        .values
        .get(&f)
        .cloned()
        .unwrap_or_else(|| panic!("no {f:?} in {:?}", o.fields))
}

/// The one secret of `o`, which must be the password.
fn password(o: &ParseOutcome) -> &str {
    assert_eq!(o.secrets.len(), 1, "one secret");
    assert_eq!(o.secrets[0].0, FieldId::Password);
    o.secrets[0].1.expose_secret()
}

// ---- The brief's vectors (HV-DRV-04) ----

#[test]
fn pg_jdbc_with_sslmode() {
    let o = ok(parse(
        "jdbc:postgresql://h:6543/d?sslmode=verify-full&user=u",
        None,
    ));
    assert_eq!(
        (o.kind, o.conn_type.as_str()),
        (DbmsKind::Postgres, "default")
    );
    assert_eq!(fv(&o, FieldId::Port), "6543");
    assert_eq!(fv(&o, FieldId::User), "u");
    assert_eq!(o.mapped[0].to, MappedTo::TlsMode(TlsMode::VerifyFull));
}

#[test]
fn pg_ipv6_password_is_moved() {
    let o = ok(parse("postgres://u:p%40ss@[::1]/d", None));
    assert_eq!(fv(&o, FieldId::Host), "::1");
    assert_eq!(o.secrets[0].1.expose_secret(), "p@ss");
    assert!(o.warnings.contains(&ParseWarning::PasswordMoved));
    assert!(!o.fields.values.contains_key(&FieldId::Password));
}

#[test]
fn libpq_dsn_socket() {
    let o = ok(parse(
        "host=/var/run/postgresql dbname=app user=alice",
        None,
    ));
    assert_eq!(o.conn_type, "unix_socket");
    assert_eq!(fv(&o, FieldId::Socket), "/var/run/postgresql");
}

#[test]
fn dsn_keys_fold_ascii_only() {
    assert_eq!(fv(&ok(parse("HOST=h DBNAME=d", None)), FieldId::Host), "h");
    assert_eq!(
        ok(parse("host=h dbname=d İSER=x", None)).extra,
        vec![("İSER".to_string(), "x".to_string())]
    );
}

#[test]
fn mysql_extras_exact() {
    let o = ok(parse(
        "jdbc:mysql://localhost/demo?empty=&custom.option[0]=a%20b&expression=a=b=c&path=/tmp/данные; x&other=last",
        None,
    ));
    let want: Vec<(String, String)> = vec![
        ("empty".into(), "".into()),
        ("custom.option[0]".into(), "a b".into()),
        ("expression".into(), "a=b=c".into()),
        ("path".into(), "/tmp/данные; x".into()),
        ("other".into(), "last".into()),
    ];
    assert_eq!(o.extra, want);
}

#[test]
fn malformed_query_is_unrecognised() {
    assert_eq!(
        parse("jdbc:mysql://h/demo?valid=value&=missing", None).unrecognised(),
        Some(UnrecognisedReason::NoMatch)
    );
}

#[test]
fn other_engine_is_reported_not_switched() {
    assert_eq!(
        ok(parse("mysql://h/db", Some(DbmsKind::Postgres))).kind,
        DbmsKind::MySql
    );
}

#[test]
fn mariadb_scheme_and_sqlite_forms() {
    assert_eq!(ok(parse("mariadb://u@h/db", None)).kind, DbmsKind::MariaDb);
    let s = ok(parse("file:/Users/x/app.db?mode=ro", None));
    assert_eq!(
        (s.kind, fv(&s, FieldId::File)),
        (DbmsKind::Sqlite, "/Users/x/app.db".into())
    );
    assert!(s.mapped.iter().any(|m| m.to == MappedTo::ReadOnly(true)));
    assert_eq!(ok(parse(r"C:\data\app.db", None)).conn_type, "file");
}

#[test]
fn url_paste_edge_vectors() {
    assert_eq!(
        fv(&ok(parse("  postgres://u@h/db\n", None)), FieldId::Host),
        "h"
    );
    assert_eq!(
        ok(parse("postgres://u:p:w%2F@h/db", None)).secrets[0]
            .1
            .expose_secret(),
        "p:w/"
    );
    let m = ok(parse("postgresql://first:5432,second:5433/demo", None));
    assert_eq!(fv(&m, FieldId::Host), "first");
    assert!(m.warnings.contains(&ParseWarning::MultipleHosts {
        used: "first".into(),
        dropped: 1
    }));
    assert_eq!(
        parse(&"a".repeat(64 * 1024 + 1), None).unrecognised(),
        Some(UnrecognisedReason::TooLong)
    );
    assert_eq!(
        parse("garbage", None).unrecognised(),
        Some(UnrecognisedReason::NoMatch)
    );
    assert_eq!(parse("postgres://u@h/db", None).unrecognised(), None);
    assert_eq!(
        ok(parse("postgres://u@h/db?user=a&user=b", None)).warnings,
        vec![ParseWarning::DuplicateKey("user".into())]
    );
}

#[test]
fn non_ascii_database_survives_percent_decoding() {
    // Review Focus 3 (P39)
    assert_eq!(
        fv(
            &ok(parse("postgres://u@h/%C4%B0zmir", None)),
            FieldId::Database
        ),
        "İzmir"
    );
    assert_eq!(
        fv(
            &ok(parse("postgres://u@h/%C4%B1rmak", None)),
            FieldId::Database
        ),
        "ırmak"
    );
}

#[test]
fn looks_like_connection_rule() {
    for s in [
        "postgres://h",
        " JDBC:mysql://h/d",
        "file:/a.db",
        "host=h dbname=d",
    ] {
        assert!(looks_like_connection(s), "{s}");
    }
    for s in ["hunter2", "my password", "postgres", "https://example.com"] {
        assert!(!looks_like_connection(s), "{s}");
    }
}

// ---- Secrets ----

#[test]
fn password_like_params_move_to_the_secret() {
    for key in ["password", "PASSWORD", "pass", "Pwd", "passwd"] {
        let o = ok(parse(&format!("postgres://u@h/db?{key}=s3cret&a=b"), None));
        assert!(password(&o) == "s3cret", "{key}");
        assert_eq!(o.extra, vec![("a".to_string(), "b".to_string())], "{key}");
        assert!(o.warnings.contains(&ParseWarning::PasswordMoved), "{key}");
    }
}

#[test]
fn a_percent_encoded_password_key_is_still_a_password() {
    // `pass%77ord` decodes (once) to `password`: it must not stay in the extras.
    let o = ok(parse("postgres://u@h/db?pass%77ord=s3cret", None));
    assert!(password(&o) == "s3cret");
    assert!(o.extra.is_empty(), "{:?}", o.extra);
    let m = ok(parse("mysql://u@h/db?PASS%57ORD=s3cret&x=1", None));
    assert!(password(&m) == "s3cret");
    assert_eq!(m.extra, vec![("x".to_string(), "1".to_string())]);
}

#[test]
fn an_ssl_key_passphrase_never_stays_in_the_extras() {
    let o = ok(parse(
        "postgres://u@h/db?sslpassword=s3cret&sslkey=/k.pem",
        None,
    ));
    assert!(o.secrets.is_empty());
    assert_eq!(o.extra, vec![("sslkey".to_string(), "/k.pem".to_string())]);
    assert!(!format!("{o:?}").contains("s3cret"));
}

#[test]
fn the_last_password_wins_and_is_reported_once() {
    let o = ok(parse("postgres://u:first@h/db?password=s3cret", None));
    assert!(password(&o) == "s3cret");
    assert_eq!(
        o.warnings,
        vec![
            ParseWarning::DuplicateKey("password".into()),
            ParseWarning::PasswordMoved
        ]
    );
}

#[test]
fn dsn_password_is_moved_with_quotes_and_escapes() {
    let o = ok(parse(
        r"host=h user=u password='s3 c\'r\\et' application_name='my app'",
        None,
    ));
    assert!(password(&o) == r"s3 c'r\et");
    assert!(!o.extra.iter().any(|(k, _)| k == "password"));
    assert_eq!(o.mapped[0].to, MappedTo::ApplicationName);
    assert_eq!(o.mapped[0].value, "my app");
}

#[test]
fn secrets_never_reach_debug_output() {
    for text in [
        "postgres://u:s3cret@h/db",
        "mysql://u@h/db?pwd=s3cret",
        "host=h password=s3cret",
        "postgresql://u:s3cret@h/db?a=b#frag",
    ] {
        let r = parse(text, None);
        assert!(r.unrecognised().is_none(), "{text}");
        assert!(!format!("{r:?}").contains("s3cret"), "{text}");
    }
}

// ---- Mapping ----

#[test]
fn tls_modes_map_from_every_spelling() {
    let cases = [
        ("postgres://h?sslmode=prefer", TlsMode::Auto),
        ("postgres://h?sslmode=disable", TlsMode::Disable),
        ("postgres://h?sslmode=require", TlsMode::Require),
        ("postgres://h?sslmode=verify-ca", TlsMode::VerifyCa),
        ("postgres://h?sslmode=verify-full", TlsMode::VerifyFull),
        ("mysql://h?sslMode=VERIFY_IDENTITY", TlsMode::VerifyFull),
        ("mysql://h?ssl-mode=REQUIRED", TlsMode::Require),
        ("mysql://h?sslMode=DISABLED", TlsMode::Disable),
        ("mysql://h?useSSL=true", TlsMode::Require),
        ("mysql://h?useSSL=false", TlsMode::Disable),
        ("host=h sslmode=verify-ca", TlsMode::VerifyCa),
    ];
    for (text, mode) in cases {
        let o = ok(parse(text, None));
        assert_eq!(o.mapped.len(), 1, "{text}");
        assert_eq!(o.mapped[0].to, MappedTo::TlsMode(mode), "{text}");
        assert!(o.extra.is_empty(), "{text}");
    }
    // A value Hacivat has no mode for stays an extra, for the driver to see.
    let o = ok(parse("postgres://h?sslmode=allow", None));
    assert!(o.mapped.is_empty());
    assert_eq!(o.extra, vec![("sslmode".to_string(), "allow".to_string())]);
}

#[test]
fn settings_map_and_options_stay_an_extra() {
    let o = ok(parse(
        "postgres://h/db?application_name=etl&connect_timeout=7&options=-c%20search_path%3Dx",
        None,
    ));
    let to: Vec<&MappedTo> = o.mapped.iter().map(|m| &m.to).collect();
    assert_eq!(
        to,
        [
            &MappedTo::ApplicationName,
            &MappedTo::ConnectTimeout(7),
            &MappedTo::PgOptions
        ]
    );
    assert_eq!(o.mapped[0].value, "etl");
    assert_eq!(
        o.extra,
        vec![("options".to_string(), "-c search_path=x".to_string())]
    );
    // Not a number: kept as typed.
    let bad = ok(parse("postgres://h?connect_timeout=soon", None));
    assert!(bad.mapped.is_empty());
    assert_eq!(bad.extra.len(), 1);
    // `mode=ro` means read-only only for SQLite.
    let pg = ok(parse("postgres://h?mode=ro", None));
    assert!(pg.mapped.is_empty());
}

#[test]
fn a_repeated_setting_keeps_the_last_value() {
    let o = ok(parse("postgres://h?sslmode=require&sslmode=disable", None));
    assert_eq!(o.mapped.len(), 1);
    assert_eq!(o.mapped[0].to, MappedTo::TlsMode(TlsMode::Disable));
    assert_eq!(
        o.warnings,
        vec![ParseWarning::DuplicateKey("sslmode".into())]
    );
    // A repeated extra keeps both, in order.
    let e = ok(parse("postgres://h?k=1&k=2", None));
    assert_eq!(
        e.extra,
        vec![
            ("k".to_string(), "1".to_string()),
            ("k".to_string(), "2".to_string())
        ]
    );
    assert_eq!(e.warnings, vec![ParseWarning::DuplicateKey("k".into())]);
}

// ---- Decoding ----

#[test]
fn values_are_decoded_exactly_once() {
    let o = ok(parse("postgres://h/a%2520b?opt=%2561", None));
    assert_eq!(fv(&o, FieldId::Database), "a%20b");
    assert_eq!(o.extra, vec![("opt".to_string(), "%61".to_string())]);
    // A DSN has no percent escapes at all.
    let d = ok(parse("host=h opt=%41", None));
    assert_eq!(d.extra, vec![("opt".to_string(), "%41".to_string())]);
}

#[test]
fn invalid_escapes_are_unrecognised() {
    for text in [
        "postgres://h/%4G",
        "postgres://h/db?a=%",
        "postgres://u:%FF@h/db",
        "file:/a%2.db",
    ] {
        assert_eq!(
            parse(text, None).unrecognised(),
            Some(UnrecognisedReason::NoMatch),
            "{text}"
        );
    }
}

#[test]
fn leading_zero_port_is_kept_and_bad_ports_refused() {
    assert_eq!(
        fv(&ok(parse("postgres://h:05432/db", None)), FieldId::Port),
        "05432"
    );
    for text in [
        "postgres://h:99999/db",
        "postgres://h:5x/db",
        "host=h port=abc",
    ] {
        assert_eq!(
            parse(text, None).unrecognised(),
            Some(UnrecognisedReason::NoMatch),
            "{text}"
        );
    }
}

// ---- PostgreSQL forms ----

#[test]
fn pg_socket_forms() {
    let q = ok(parse(
        "postgresql://u@:5433/app?host=/var/run/postgresql",
        None,
    ));
    assert_eq!(q.conn_type, "unix_socket");
    assert_eq!(fv(&q, FieldId::Socket), "/var/run/postgresql");
    assert_eq!(fv(&q, FieldId::Port), "5433");
    assert!(!q.fields.values.contains_key(&FieldId::Host));
    let h = ok(parse("postgresql://%2Fvar%2Frun%2Fpostgresql/app", None));
    assert_eq!(h.conn_type, "unix_socket");
    assert_eq!(fv(&h, FieldId::Socket), "/var/run/postgresql");
    // A query host that is a name overrides the authority, with a warning.
    let n = ok(parse("postgresql://a/db?host=b", None));
    assert_eq!(
        (n.conn_type.as_str(), fv(&n, FieldId::Host)),
        ("default", "b".into())
    );
    assert_eq!(n.warnings, vec![ParseWarning::DuplicateKey("host".into())]);
}

#[test]
fn pg_jdbc_short_form_and_query_fields() {
    let o = ok(parse("jdbc:postgresql:app?user=u", None));
    assert_eq!(o.kind, DbmsKind::Postgres);
    assert_eq!(fv(&o, FieldId::Host), "localhost");
    assert_eq!(fv(&o, FieldId::Database), "app");
    assert_eq!(fv(&o, FieldId::User), "u");
    let q = ok(parse("postgres:///?dbname=d&port=6000&host=h", None));
    assert_eq!(
        (
            fv(&q, FieldId::Database),
            fv(&q, FieldId::Port),
            fv(&q, FieldId::Host)
        ),
        ("d".into(), "6000".into(), "h".into())
    );
}

#[test]
fn dsn_multi_host_and_libpq_whitespace() {
    let o = ok(parse("host=a,b port=5432,5433 dbname = d\tuser=u", None));
    assert_eq!(fv(&o, FieldId::Host), "a");
    assert_eq!(fv(&o, FieldId::Port), "5432");
    assert_eq!(fv(&o, FieldId::Database), "d");
    assert_eq!(fv(&o, FieldId::User), "u");
    assert!(o.warnings.contains(&ParseWarning::MultipleHosts {
        used: "a".into(),
        dropped: 1
    }));
    // libpq reads the value after `=` and any whitespace.
    let e = ok(parse("host= h dbname=", None));
    assert_eq!(fv(&e, FieldId::Host), "h");
    assert!(!e.fields.values.contains_key(&FieldId::Database));
}

#[test]
fn not_a_dsn() {
    for text in [
        "a=b",
        "host='unterminated",
        "host h",
        "=x host=h",
        "host=h\u{1}",
        "the answer is x=1",
    ] {
        assert_eq!(
            parse(text, None).unrecognised(),
            Some(UnrecognisedReason::NoMatch),
            "{text}"
        );
    }
}

// ---- MySQL forms ----

#[test]
fn mysql_socket_and_query_user() {
    let o = ok(parse(
        "mysql://u@localhost/db?socket=/tmp/mysql.sock&x=1",
        None,
    ));
    assert_eq!(o.conn_type, "unix_socket");
    assert_eq!(fv(&o, FieldId::Socket), "/tmp/mysql.sock");
    assert!(!o.fields.values.contains_key(&FieldId::Host));
    assert_eq!(o.extra, vec![("x".to_string(), "1".to_string())]);
    // A socket with a remote host or a port is not a socket connection: kept as an extra.
    let r = ok(parse("mysql://h:3307/db?socket=/tmp/s", None));
    assert_eq!(r.conn_type, "default");
    assert_eq!(r.extra, vec![("socket".to_string(), "/tmp/s".to_string())]);
    let u = ok(parse("jdbc:mariadb://h/db?user=app", None));
    assert_eq!(
        (u.kind, fv(&u, FieldId::User)),
        (DbmsKind::MariaDb, "app".into())
    );
}

#[test]
fn schemes_fold_ascii_case_only() {
    assert_eq!(ok(parse("PostgreSQL://h", None)).kind, DbmsKind::Postgres);
    assert_eq!(ok(parse("MySQL://h", None)).kind, DbmsKind::MySql);
    assert_eq!(ok(parse("FILE:/a.db", None)).kind, DbmsKind::Sqlite);
    // `İ` is not `I`: no Unicode folding.
    assert_eq!(parse("jdbc:mysql://h/db", None).unrecognised(), None);
    assert_eq!(
        parse("JDBC:MİSQL://h/db", None).unrecognised(),
        Some(UnrecognisedReason::NoMatch)
    );
}

// ---- SQLite forms ----

#[test]
fn sqlite_forms() {
    let m = ok(parse(":memory:", None));
    assert_eq!((m.kind, m.conn_type.as_str()), (DbmsKind::Sqlite, "memory"));
    assert!(m.fields.values.is_empty());
    let u = ok(parse("file:///Users/x/my%20app.db", None));
    assert_eq!(fv(&u, FieldId::File), "/Users/x/my app.db");
    let w = ok(parse("file:///C:/data/app.db", None));
    assert_eq!(fv(&w, FieldId::File), "C:/data/app.db");
    let l = ok(parse("file://localhost/tmp/a.db", None));
    assert_eq!(fv(&l, FieldId::File), "/tmp/a.db");
    let j = ok(parse("jdbc:sqlite:/tmp/app.sqlite", None));
    assert_eq!(
        (j.conn_type.as_str(), fv(&j, FieldId::File)),
        ("file", "/tmp/app.sqlite".into())
    );
    assert_eq!(ok(parse("jdbc:sqlite::memory:", None)).conn_type, "memory");
    assert_eq!(ok(parse("file::memory:", None)).conn_type, "memory");
    // Plain paths: kept as typed, no environment expansion.
    for p in [
        "/Users/x/app.db",
        "~/data/app.sqlite3",
        "./a.db3",
        "app.db",
        r"\\srv\share\a.db",
        "C:/%USERNAME%/a.db",
    ] {
        let o = ok(parse(p, None));
        assert_eq!(
            (o.kind, fv(&o, FieldId::File)),
            (DbmsKind::Sqlite, p.to_string()),
            "{p}"
        );
    }
    // A path that does not name a SQLite file is not taken for one.
    for p in ["/var/run/postgresql", "notes.txt", "example.com"] {
        assert_eq!(
            parse(p, None).unrecognised(),
            Some(UnrecognisedReason::NoMatch),
            "{p}"
        );
    }
}

// ---- The template fallback ----

#[test]
fn the_template_fallback_takes_what_the_native_parser_refuses() {
    // A raw `#` is not part of a libpq URI, but the catalog template's parameter value allows
    // it, so the template matcher recognises the text; the password still moves.
    let o = ok(parse(
        "postgresql://u:s3cret@h/db?a=b#c&pass%77ord=x2",
        Some(DbmsKind::Postgres),
    ));
    assert_eq!(
        (o.kind, o.conn_type.as_str()),
        (DbmsKind::Postgres, "default")
    );
    assert_eq!(fv(&o, FieldId::User), "u");
    assert_eq!(o.extra, vec![("a".to_string(), "b#c".to_string())]);
    assert!(password(&o) == "x2");
    assert!(o.warnings.contains(&ParseWarning::PasswordMoved));
}

// ---- Limits ----

#[test]
fn many_params_are_capped_and_fast() {
    let query = |n: usize| {
        let p: Vec<String> = (0..n).map(|i| format!("k{i}=v")).collect();
        format!("postgres://h/db?{}", p.join("&"))
    };
    assert_eq!(ok(parse(&query(1024), None)).extra.len(), 1024);
    let start = std::time::Instant::now();
    let big = query(5000);
    assert!(big.len() < 64 * 1024);
    assert_eq!(
        parse(&big, None).unrecognised(),
        Some(UnrecognisedReason::NoMatch)
    );
    let dsn: Vec<String> = (0..5000).map(|i| format!("k{i}=v")).collect();
    assert_eq!(
        parse(&format!("host=h {}", dsn.join(" ")), None).unrecognised(),
        Some(UnrecognisedReason::NoMatch)
    );
    assert!(
        start.elapsed() < std::time::Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn empty_and_long_inputs() {
    assert_eq!(
        parse(" \n\t", None).unrecognised(),
        Some(UnrecognisedReason::Empty)
    );
    let exactly = format!(
        "postgres://h/{}",
        "a".repeat(64 * 1024 - "postgres://h/".len())
    );
    assert_eq!(exactly.len(), 64 * 1024);
    assert_eq!(parse(&exactly, None).unrecognised(), None);
}

#[test]
fn looks_like_connection_follows_the_mock() {
    for s in [
        "\tpostgresql://h",
        "mariadb://h",
        "jdbc:postgres://h",
        "FILE:x.db",
        "x host = h",
        "(dbname=d)",
    ] {
        assert!(looks_like_connection(s), "{s}");
    }
    for s in [
        "jdbc:sqlite:/a.db",
        "my_host=h",
        "hostname=h",
        "host= ",
        "HOST=h",
        "postgres:/h",
        "mysql+srv://h",
    ] {
        assert!(!looks_like_connection(s), "{s}");
    }
}
