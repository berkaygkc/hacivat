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
fn an_ssl_key_passphrase_is_carried_apart() {
    // Controller ruling (T7 fix round 1): `sslpassword`, any case or encoding, becomes
    // `tls_key_passphrase`, never an extra and never the password.
    // Failure messages name the case by index: the texts hold a fixture secret.
    for (i, text) in [
        "postgres://u@h/db?sslpassword=s3cret&sslkey=/k.pem",
        "postgres://u@h/db?SSLPassword=s3cret&sslkey=/k.pem",
        "postgres://u@h/db?ssl%70assword=s3cret&sslkey=/k.pem",
        "host=h sslpassword=s3cret sslkey=/k.pem",
        "jdbc:postgresql://h/db?sslpassword=s3cret&sslkey=/k.pem",
    ]
    .into_iter()
    .enumerate()
    {
        let o = ok(parse(text, None));
        assert!(o.secrets.is_empty(), "case {i}");
        let kp = o.tls_key_passphrase.as_ref();
        assert!(
            kp.is_some_and(|k| k.expose_secret() == "s3cret"),
            "case {i}"
        );
        assert_eq!(
            o.extra,
            vec![("sslkey".to_string(), "/k.pem".to_string())],
            "case {i}"
        );
        assert!(
            !o.warnings.contains(&ParseWarning::PasswordMoved),
            "case {i}"
        );
        assert!(!format!("{o:?}").contains("s3cret"), "case {i}");
    }
    // Both secrets at once, each in its own place; the last passphrase wins, an empty one
    // clears it.
    let both = ok(parse(
        "postgres://u:pw1@h/db?sslpassword=a&sslpassword=kp2",
        None,
    ));
    assert!(password(&both) == "pw1");
    assert!(both.tls_key_passphrase.unwrap().expose_secret() == "kp2");
    let cleared = ok(parse("postgres://h/db?sslpassword=a&sslpassword=", None));
    assert!(cleared.tls_key_passphrase.is_none());
    assert!(
        ok(parse("postgres://h/db", None))
            .tls_key_passphrase
            .is_none()
    );
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
    for (i, text) in [
        "postgres://u:s3cret@h/db",
        "mysql://u@h/db?pwd=s3cret",
        "host=h password=s3cret",
        "jdbc:postgresql://u:s3cret@h/db",
        "jdbc:mysql://h/db?password=s3cret",
        "jdbc:postgresql:db?password=s3cret",
        "file:/tmp/a.db?password=s3cret",
        "jdbc:sqlite:/tmp/a.db?pwd=s3cret",
        "postgresql://u:s3cret@first:5432,second:5433/db",
        "postgresql://u:s3cret@/db?host=/var/run/postgresql",
        "postgresql:///db?host=/sock&password=s3cret",
        "mysql://u:s3cret@localhost/db?socket=/tmp/mysql.sock",
        "mysql://u@localhost/db?socket=/tmp/s&pwd=s3cret",
        "postgres://h/db?sslpassword=s3cret",
        "host=/sock sslpassword=s3cret",
    ]
    .into_iter()
    .enumerate()
    {
        // Named by index: the texts hold a fixture secret.
        let r = parse(text, None);
        assert!(r.unrecognised().is_none(), "case {i}");
        assert!(!format!("{r:?}").contains("s3cret"), "case {i}");
    }
}

// ---- A refused URL is final (T7 fix round 1) ----

#[test]
fn a_claimed_url_that_is_refused_never_reaches_the_templates() {
    // A looser template used to take these, swallowing the password into the socket path.
    // Named by index: the texts hold a fixture secret.
    for (i, text) in [
        "postgresql:///db?host=/sock&password=SECRETPW&flag",
        "postgresql://u@/db?host=/sock&password=SECRETPW&",
        "mysql://u@localhost/db?socket=/tmp/s&pwd=SECRETPW&x",
        "mariadb://u@localhost/db?socket=/tmp/s&pwd=SECRETPW&",
        "jdbc:postgresql:///db?host=/sock&password=SECRETPW&flag",
        "postgresql://u:SECRETPW@h/db?a=b#c",
        "file:/tmp/a.db?password=SECRETPW&flag",
    ]
    .into_iter()
    .enumerate()
    {
        let r = parse(text, None);
        assert_eq!(
            r.unrecognised(),
            Some(UnrecognisedReason::NoMatch),
            "case {i}"
        );
        assert!(!format!("{r:?}").contains("SECRETPW"), "case {i}");
    }
}

#[test]
fn a_raw_at_sign_in_the_path_is_refused() {
    // `u:12/34@h`: an unencoded `/` in the password would put `34@h/db` in the database.
    for (i, text) in [
        "postgres://u:12/34@h/db",
        "mysql://u:12/34@h/db",
        "postgres://h/my@db",
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            parse(text, None).unrecognised(),
            Some(UnrecognisedReason::NoMatch),
            "case {i}"
        );
    }
    // Encoded, it is fine.
    let o = ok(parse("postgres://u:12%2F34@h/my%40db", None));
    assert!(password(&o) == "12/34");
    assert_eq!(fv(&o, FieldId::Database), "my@db");
}

#[test]
fn network_paths_never_become_sqlite_files() {
    // On Windows, opening one sends the user's NTLM hash to the named host.
    for text in [
        "file://server/share/a.db",
        "file:////server/share/a.db",
        "file://localhost//server/share/a.db",
        r"file:\\server\share\a.db",
        r"file:\\?\UNC\server\share\a.db",
        r"file:\\.\pipe\a.db",
        "file:%2F%2Fserver/share/a.db",
        r"file:/\server\share\a.db",
        r"jdbc:sqlite:\\server\share\a.db",
        r"\\server\share\a.db",
        r"\\?\UNC\server\share\a.db",
        r"\\.\C:\a.db",
        "//server/share/a.db",
        r"/\server\share\a.db",
    ] {
        assert_eq!(
            parse(text, None).unrecognised(),
            Some(UnrecognisedReason::NoMatch),
            "{text}"
        );
    }
}

#[test]
fn a_refused_dsn_ending_in_db_is_not_a_path() {
    // Not a DSN (`x.db` has no `=`), and not a SQLite path either: it carries a password.
    let r = parse("host=h password=SECRETPW x.db", None);
    assert_eq!(r.unrecognised(), Some(UnrecognisedReason::NoMatch));
}

// ---- The outcome invariant (T7 fix round 2) ----

/// The re-review's probes: each once left the password inside a field, an extra or a file name.
#[test]
fn no_recognised_value_carries_a_swallowed_secret() {
    // Named by index: the texts hold a fixture secret.
    for (i, text) in [
        // Whitespace around `=` (libpq, ADO).
        "host=h password = SECRETPW x.db",
        "Server=h;Pwd =SECRETPW;a.db",
        "Server=h; Pwd = SECRETPW; a.db",
        // Native query, socket and file values.
        "postgresql:///db?host=/sock;password=SECRETPW",
        "mysql://u@localhost/db?socket=/tmp/s;pwd=SECRETPW",
        "postgres://h/db?a=x;password=SECRETPW",
        "file:/tmp/a.db?x=1;password=SECRETPW",
        "file:/tmp/a.db;password=SECRETPW",
        "postgresql:///db?host=/sock%26PassWord%3DSECRETPW",
        "postgres://h/db?application_name=x,pwd=SECRETPW",
        "jdbc:sqlite:/tmp/a.db;passwd = SECRETPW",
        // An encoded `=`.
        "host=h PassWord%3DSECRETPW x.db",
        "postgres://h/a%3Bpassword%3DSECRETPW",
    ]
    .into_iter()
    .enumerate()
    {
        for preferred in [None, Some(DbmsKind::Postgres), Some(DbmsKind::Sqlite)] {
            let r = parse(text, preferred);
            assert_eq!(
                r.unrecognised(),
                Some(UnrecognisedReason::NoMatch),
                "case {i}"
            );
            assert!(!format!("{r:?}").contains("SECRETPW"), "case {i}");
        }
    }
}

/// Where the native parser owns the key, the secret is moved, not refused.
#[test]
fn an_owned_secret_key_is_still_moved() {
    for (i, text) in [
        "host=h password = SECRETPW",
        "host=h\tpassword=\tSECRETPW dbname=d",
        "postgres://h/db?a=x&password=SECRETPW",
        "mysql://u@localhost/db?socket=/tmp/s&pwd=SECRETPW",
        "postgresql:///db?host=/sock&PassWord=SECRETPW",
        "file:/tmp/a.db?x=1&password=SECRETPW",
    ]
    .into_iter()
    .enumerate()
    {
        let o = ok(parse(text, None));
        assert!(password(&o) == "SECRETPW", "case {i}");
        assert!(!format!("{o:?}").contains("SECRETPW"), "case {i}");
    }
}

/// A value that merely mentions `password`, with no `=` after it, is kept.
#[test]
fn a_value_that_only_mentions_password_is_kept() {
    let db = ok(parse("postgres://h/password_db", None));
    assert_eq!(fv(&db, FieldId::Database), "password_db");
    let sock = ok(parse("postgresql:///db?host=/run/passwordless", None));
    assert_eq!(fv(&sock, FieldId::Socket), "/run/passwordless");
    let file = ok(parse("file:/tmp/password.db", None));
    assert_eq!(fv(&file, FieldId::File), "/tmp/password.db");
    let plain = ok(parse("/tmp/passwords; old.db", None));
    assert_eq!(fv(&plain, FieldId::File), "/tmp/passwords; old.db");
    let extra = ok(parse(
        "postgres://h/db?note=no%20password%20here&x=pass;word",
        None,
    ));
    assert_eq!(
        extra.extra,
        vec![
            ("note".to_string(), "no password here".to_string()),
            ("x".to_string(), "pass;word".to_string())
        ]
    );
    let user = ok(parse("host=h user=password", None));
    assert_eq!(fv(&user, FieldId::User), "password");
}

// ---- Fix round 3: Unicode whitespace, allowlisted keys, socket network paths ----

const ENGINES: [Option<DbmsKind>; 4] = [
    None,
    Some(DbmsKind::Postgres),
    Some(DbmsKind::MySql),
    Some(DbmsKind::Sqlite),
];

#[test]
fn round_three_swallowed_secrets_are_refused() {
    // Named by index: the texts hold a fixture secret.
    for (i, text) in [
        // Unicode whitespace (rich-text pastes): NBSP and U+3000.
        "host=h password\u{a0}=SECRETPW x.db",
        "Server=h;\u{a0}Pwd=SECRETPW;a.db",
        "postgres://h/db?application_name=x%C2%A0password=SECRETPW",
        "postgres://h/db?application_name=x%E3%80%80password=SECRETPW",
        "postgres://h/db?x=y\u{3000}pwd\u{3000}=SECRETPW",
        // Any non-key character ends a key: quotes, brace, pipe, colon, slash, plus.
        "postgres://h/db?x='password=SECRETPW'",
        "postgres://h/db?x=\"password\"=SECRETPW",
        "postgres://h/db?x={password=SECRETPW}",
        "postgres://h/db?x=a|password=SECRETPW",
        "postgres://h/db?x=a:password=SECRETPW",
        "postgresql:///db?host=/tmp/password=SECRETPW",
        "mysql://u@localhost/db?socket=/tmp/pwd=SECRETPW",
        "postgres://h/db?x=x%2Bpassword=SECRETPW",
        "postgres://h/db?x=x+password=SECRETPW",
        "host=h options='-c x=1,password=SECRETPW'",
    ]
    .into_iter()
    .enumerate()
    {
        for preferred in ENGINES {
            let r = parse(text, preferred);
            assert_eq!(
                r.unrecognised(),
                Some(UnrecognisedReason::NoMatch),
                "case {i}"
            );
            assert!(!format!("{r:?}").contains("SECRETPW"), "case {i}");
        }
    }
}

#[test]
fn a_rich_text_dsn_moves_its_password() {
    // The DSN splits on any Unicode whitespace, so the password is moved, not swallowed.
    for (i, (text, db)) in [
        (
            "host=h\u{a0}password=SECRETPW\u{a0}dbname=x.db",
            Some("x.db"),
        ),
        ("host=h\u{3000}password=SECRETPW", None),
        (
            "host=h \u{a0}password\u{a0}=\u{a0}SECRETPW dbname=d",
            Some("d"),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        for preferred in ENGINES {
            let o = ok(parse(text, preferred));
            assert_eq!(fv(&o, FieldId::Host), "h", "case {i}");
            assert_eq!(
                o.fields.values.get(&FieldId::Database).map(String::as_str),
                db,
                "case {i}"
            );
            assert!(password(&o) == "SECRETPW", "case {i}");
            assert!(!format!("{o:?}").contains("SECRETPW"), "case {i}");
        }
    }
}

#[test]
fn sockets_on_other_machines_are_refused() {
    for text in [
        r"mysql://u@localhost/db?socket=\\evil\pipe\mysql",
        "mysql://u@localhost/db?socket=//evil/pipe/mysql",
        "mysql://u@localhost/db?socket=%5C%5Cevil%5Cpipe%5Cmysql",
        "postgresql:///db?host=//evil/share",
        "postgresql:///db?host=%2F%2Fevil%2Fshare",
        "postgresql://%2F%2Fevil%2Fshare/db",
        "host=//evil/share dbname=d",
        "host='//evil/share' dbname=d",
    ] {
        for preferred in ENGINES {
            assert_eq!(
                parse(text, preferred).unrecognised(),
                Some(UnrecognisedReason::NoMatch),
                "{text}"
            );
        }
    }
    // A local socket is fine.
    let ok_sock = ok(parse("mysql://u@localhost/db?socket=/tmp/mysql.sock", None));
    assert_eq!(fv(&ok_sock, FieldId::Socket), "/tmp/mysql.sock");
}

#[test]
fn round_three_legitimate_values_still_parse() {
    let opts = ok(parse(
        "postgres://h/db?options=-c%20search_path%3Dpublic",
        None,
    ));
    assert_eq!(
        opts.extra,
        vec![("options".to_string(), "-c search_path=public".to_string())]
    );
    let enc = ok(parse(
        "postgres://h/db?options=-c%20password_encryption%3Dscram-sha-256",
        None,
    ));
    assert_eq!(enc.extra[0].1, "-c password_encryption=scram-sha-256");
    let dsn = ok(parse("host=h options='-c search_path=public'", None));
    assert_eq!(dsn.extra[0].1, "-c search_path=public");
    let app = ok(parse(
        "postgres://h/db?application_name=password-reset-service",
        None,
    ));
    assert_eq!(app.mapped[0].value, "password-reset-service");
    let db = ok(parse("postgres://h/password_db", None));
    assert_eq!(fv(&db, FieldId::Database), "password_db");
    let sock = ok(parse("postgresql:///db?host=/run/passwordless", None));
    assert_eq!(fv(&sock, FieldId::Socket), "/run/passwordless");
    let user = ok(parse("host=h user=password", None));
    assert_eq!(fv(&user, FieldId::User), "password");
    let tr = ok(parse("postgres://h/%C5%9Fifre_db", None));
    assert_eq!(fv(&tr, FieldId::Database), "şifre_db");
    let tr_file = ok(parse("/tmp/şifre.db", None));
    assert_eq!(fv(&tr_file, FieldId::File), "/tmp/şifre.db");
    let tr_uri = ok(parse("file:/tmp/%C5%9Fifre.db", None));
    assert_eq!(fv(&tr_uri, FieldId::File), "/tmp/şifre.db");
    // Accepted cost of the allowlist rule: a file named like `password=…` is refused.
    assert_eq!(
        parse("/tmp/password=1.db", None).unrecognised(),
        Some(UnrecognisedReason::NoMatch)
    );
}

#[test]
fn nt_object_namespace_paths_are_refused() {
    for text in [
        r"\??\UNC\server\share\a.db",
        r"\??\C:\data\a.db",
        "/??/UNC/server/share/a.db",
        r"file:\??\UNC\server\share\a.db",
        "file:%5C%3F%3F%5CUNC%5Cserver%5Cshare%5Ca.db",
        "file:%2F%3F%3F%2FUNC%2Fserver%2Fshare%2Fa.db",
        r"jdbc:sqlite:\??\UNC\server\share\a.db",
        "jdbc:sqlite:%5C??%5CUNC%5Cserver%5Cshare%5Ca.db",
        "%5C%3F%3F%5CUNC%5Cserver%5Cshare%5Ca.db",
    ] {
        assert_eq!(
            parse(text, None).unrecognised(),
            Some(UnrecognisedReason::NoMatch),
            "{text}"
        );
    }
}

#[test]
fn port_zero_is_refused() {
    for text in [
        "postgres://h:0/db",
        "postgres://h:00000/db",
        "host=h port=0",
        "mysql://h:0",
    ] {
        assert_eq!(
            parse(text, None).unrecognised(),
            Some(UnrecognisedReason::NoMatch),
            "{text}"
        );
    }
    assert_eq!(
        fv(&ok(parse("postgres://h:1/db", None)), FieldId::Port),
        "1"
    );
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
        r"C:\data\app.db",
        "C:/%USERNAME%/a.db",
    ] {
        let o = ok(parse(p, None));
        assert_eq!(
            (o.kind, fv(&o, FieldId::File)),
            (DbmsKind::Sqlite, p.to_string()),
            "{p}"
        );
    }
    // A path that does not name a local SQLite file is not taken for one (a UNC path would
    // send the user's NTLM hash to the host it names).
    for p in [
        "/var/run/postgresql",
        "notes.txt",
        "example.com",
        r"\\srv\share\a.db",
    ] {
        assert_eq!(
            parse(p, None).unrecognised(),
            Some(UnrecognisedReason::NoMatch),
            "{p}"
        );
    }
}

// ---- The template fallback ----

#[test]
fn the_template_fallback_never_retries_a_native_refusal() {
    // A raw `#` is not part of a libpq URI. The catalog template's parameter value would allow
    // it, but the native parser claimed the text by its scheme, so its refusal is final, for
    // every preferred engine. (The fallback itself is covered by `url::parse::tests`.)
    for preferred in [
        None,
        Some(DbmsKind::Postgres),
        Some(DbmsKind::MySql),
        Some(DbmsKind::Sqlite),
    ] {
        let r = parse("postgresql://u:s3cret@h/db?a=b#c&pass%77ord=x2", preferred);
        assert_eq!(r.unrecognised(), Some(UnrecognisedReason::NoMatch));
    }
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

// ---- Follow-up: shell-snippet and dash/underscore-prefixed secret keys ----

#[test]
fn shell_snippet_and_prefixed_secret_keys_never_stay_in_the_clear() {
    for (i, text) in [
        "PGPASSWORD=SECRETPW host=h dbname=d",
        "pgpassword=SECRETPW host=h",
        "MYSQL_PWD=SECRETPW host=h",
        "host=h MYSQL_PWD=SECRETPW",
        "postgres://h/db?options=--password=SECRETPW",
        "postgres://h/db?options=--pwd=SECRETPW",
        "postgres://h/db?a-password=SECRETPW",
        "postgres://h/db?x_password=SECRETPW",
        "postgres://h/db?x_pwd=SECRETPW",
        "postgres://h/db?x=y&--password=SECRETPW",
        "postgres://h/db?x=y&-password=SECRETPW",
        "postgres://h/db?x=y&db_password=SECRETPW",
        "postgres://h/db?x=y&PGPASSWORD=SECRETPW",
        "mysql://u@h/db?MYSQL_PWD=SECRETPW",
        "host=h options='--password=SECRETPW'",
        "host=h options=--password=SECRETPW",
        r#"postgres://h/db?x=\"password\"=SECRETPW"#,
        r#"postgres://h/db?x=\'pwd\'=SECRETPW"#,
        r#"host=h options=\"password\"=SECRETPW"#,
    ]
    .into_iter()
    .enumerate()
    {
        for preferred in ENGINES {
            let r = parse(text, preferred);
            assert!(!format!("{r:?}").contains("SECRETPW"), "case {i}");
            match r {
                // Refused outright, or moved to the secret by a parser that owns the key.
                ParseResult::Recognised(o) => {
                    assert!(password(&o) == "SECRETPW", "case {i} {preferred:?}");
                    assert!(
                        o.extra
                            .iter()
                            .all(|(k, v)| !v.contains("SECRETPW") && !k.contains("SECRETPW")),
                        "case {i}"
                    );
                }
                other => assert_eq!(
                    other.unrecognised(),
                    Some(UnrecognisedReason::NoMatch),
                    "case {i}"
                ),
            }
        }
    }
}

#[test]
fn legitimate_password_words_still_parse() {
    let opts = ok(parse(
        "postgres://h/db?options=-c%20search_path%3Dpublic",
        None,
    ));
    assert_eq!(opts.extra[0].1, "-c search_path=public");
    let enc = ok(parse(
        "host=h options='-c password_encryption=scram-sha-256'",
        None,
    ));
    assert_eq!(enc.extra[0].1, "-c password_encryption=scram-sha-256");
    let app = ok(parse(
        "postgres://h/db?application_name=password-reset-service",
        None,
    ));
    assert_eq!(app.mapped[0].value, "password-reset-service");
    for preferred in ENGINES {
        let db = ok(parse("postgres://h/password_db", preferred));
        assert_eq!(fv(&db, FieldId::Database), "password_db");
        let dsn = ok(parse("host=h dbname=password_db", preferred));
        assert_eq!(fv(&dsn, FieldId::Database), "password_db");
        let user = ok(parse("host=h user=password", preferred));
        assert_eq!(fv(&user, FieldId::User), "password");
    }
}

#[test]
fn only_owner_keys_move_and_other_secret_keys_are_refused() {
    for (i, text) in [
        "postgres://h/db?ssl_password=SECRETPW",
        "postgres://h/db?sslkey_password=SECRETPW",
        "postgres://h/db?x_password=SECRETPW",
        "postgres://h/db?a-pwd=SECRETPW",
        "host=h ssl_password=SECRETPW",
        "host=h x_password=SECRETPW dbname=d",
        "mysql://u@h/db?db_password=SECRETPW",
    ]
    .into_iter()
    .enumerate()
    {
        for preferred in ENGINES {
            let r = parse(text, preferred);
            assert!(!format!("{r:?}").contains("SECRETPW"), "case {i}");
            assert_eq!(
                r.unrecognised(),
                Some(UnrecognisedReason::NoMatch),
                "case {i}"
            );
        }
    }
    for preferred in ENGINES {
        let o = ok(parse("PGPASSWORD=SECRETPW host=h dbname=d", preferred));
        assert!(password(&o) == "SECRETPW");
        assert!(!format!("{o:?}").contains("SECRETPW"));
        let o = ok(parse("postgres://h/db?sslpassword=SECRETPW", preferred));
        assert!(o.secrets.is_empty());
        assert!(o.tls_key_passphrase.is_some());
        assert!(!format!("{o:?}").contains("SECRETPW"));
    }
}

// ## m2a-t21-shots

#[test]
fn the_public_secret_key_rule_covers_the_import_names() {
    use hacivat_driver::url::is_secret_param;
    for key in [
        "password",
        "PASSWORD",
        "sslpassword",
        "sslkeypassword",
        "SSLKeyPassword",
        "pgpassword",
        "mysql_pwd",
        "--password",
        "db_password",
        "x-pwd",
    ] {
        assert!(is_secret_param(key), "{key}");
    }
    for key in [
        "password_db",
        "user",
        "sslmode",
        "sslkey",
        "application_name",
    ] {
        assert!(!is_secret_param(key), "{key}");
    }
}

#[test]
fn a_url_carrying_sslkeypassword_is_refused_and_never_shows_it() {
    for preferred in ENGINES {
        let r = parse("postgres://h/db?sslkeypassword=SECRETPW", preferred);
        assert_eq!(
            r.unrecognised(),
            Some(UnrecognisedReason::NoMatch),
            "{preferred:?}"
        );
        assert!(!format!("{r:?}").contains("SECRETPW"));
        assert!(!format!("{r:#?}").contains("SECRETPW"));
    }
}
