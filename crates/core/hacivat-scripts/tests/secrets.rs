// ## m4b-t06-secrets
//! M4b Task 6: the secret-literal scanner (HV-SCRIPT-10; R-M4b-17, -28; alg/28 C9).
//!
//! `scan(t)` feeds the scanner the history redactor's spans exactly as the app does: it maps
//! `hacivat_exec::history::secret_spans(t)` onto [`SpanKind`] one to one (`hacivat-exec` is a
//! dev-dependency only, R-M4b-28).
//!
//! R-C0-4: no vendor-shaped token appears in this file as one literal. Every token fixture,
//! and every PEM private-key header, is joined from split parts at compile or run time.

use std::ops::Range;
use std::time::Instant;

use hacivat_exec::history::{SecretSpanKind, secret_spans};
use hacivat_scripts::SecretKind::{AccessToken, ConnectionString, Jwt, Password, PrivateKey};
use hacivat_scripts::{SecretFinding, SecretKind, SpanKind, scan_secrets};

/// The app's mapping (R-M4b-28): `Value` → `Password`, `ConnectionValue` → `ConnectionString`,
/// `Armed` → `Other`.
fn spans(t: &str) -> Vec<(Range<usize>, SpanKind)> {
    secret_spans(t)
        .into_iter()
        .map(|(r, k)| {
            let k = match k {
                SecretSpanKind::Value => SpanKind::Password,
                SecretSpanKind::ConnectionValue => SpanKind::ConnectionString,
                SecretSpanKind::Armed => SpanKind::Other,
            };
            (r, k)
        })
        .collect()
}

fn scan(t: &str) -> Vec<SecretFinding> {
    scan_secrets(t, &spans(t))
}

/// A PEM header line for `label` (`"OPENSSH "`, `"RSA "`, `""`), joined at run time.
fn pem_header(label: &str) -> String {
    format!(
        "{}BEGIN {label}{}",
        "-----",
        concat!("PRIVATE", " KEY-----")
    )
}

fn kinds(f: &[SecretFinding]) -> Vec<SecretKind> {
    f.iter().map(|f| f.kind).collect()
}

#[test]
fn secret_table() {
    // (text, expected kind) — the plan's 30 rows (18 reported, 12 quiet), fixed.
    let openssh = pem_header("OPENSSH ");
    let rsa = pem_header("RSA ");
    let warn: [(&str, SecretKind); 18] = [
        ("CREATE USER app PASSWORD 's3cret';", Password),
        ("ALTER ROLE app WITH PASSWORD 's3cret'", Password),
        ("CREATE USER 'a'@'%' IDENTIFIED BY 'tiger';", Password),
        ("ALTER USER a IDENTIFIED BY tiger", Password),
        ("SET PASSWORD FOR a = 'x1'", Password),
        (
            "SELECT dblink_connect('host=h user=u password=pw')",
            ConnectionString,
        ),
        ("-- prod: postgresql://u:pw@db/orders", ConnectionString),
        ("SELECT 'AKIAIOSFODNN7EXAMPLE'", AccessToken),
        (
            concat!("-- gh", "p_0123456789abcdefghijABCDEFGHIJ012345"),
            AccessToken,
        ),
        (
            concat!("SELECT 'xox", "b-1234-5678-abcdefghijkl'"),
            AccessToken,
        ),
        (
            concat!("-- sk_", "live_0123456789abcdefABCDEF01"),
            AccessToken,
        ),
        (
            concat!("'AI", "zaSyA-0123456789abcdefghijklmnopqrstu'"),
            AccessToken,
        ),
        (openssh.as_str(), PrivateKey),
        (rsa.as_str(), PrivateKey),
        (
            concat!("'eyJ", "hbGciOiJIUzI1NiJ9.eyJ", "zdWIiOiIxIn0.c2ln'"),
            Jwt,
        ),
        ("CREATE USER app PASSWORD 'İDENTİFİED'", Password),
        ("İDENTİFİED BY 'x'", Password),
        ("SELECT pgp_sym_encrypt(data, 'k3y')", Password),
    ];
    let quiet: [&str; 12] = [
        "SELECT password_hash FROM users",
        "ALTER ROLE app PASSWORD NULL",
        "CREATE USER app PASSWORD :pw",
        "CREATE USER app PASSWORD '***'",
        "CREATE USER app PASSWORD ''",
        "CREATE USER app PASSWORD '${APP_PW}'",
        "SELECT 'AKIA' || x",
        "-- postgresql://u@db/orders",
        "CREATE USER app PASSWORD 's3cret'; -- hacivat:allow-secret",
        "SELECT '-----BEGIN CERTIFICATE-----'",
        "SELECT 'eyJ'",
        "UPDATE t SET passwd_changed = now()",
    ];
    for (t, k) in &warn {
        let f = scan(t);
        assert!(f.iter().any(|f| f.kind == *k), "{t}: {:?}", kinds(&f));
    }
    for t in quiet {
        let f = scan(t);
        assert!(f.is_empty(), "{t}: {:?}", kinds(&f));
    }
}

#[test]
fn allow_comment_silences_one_line() {
    let t = "CREATE USER a PASSWORD 'x'; -- hacivat:allow-secret\nCREATE USER b PASSWORD 'y';";
    let f = scan(t);
    assert_eq!(f.len(), 1, "{:?}", kinds(&f));
    assert_eq!(f[0].line, 1);
    // A block comment counts too; a marker outside any comment does not.
    for t in [
        "CREATE USER a PASSWORD 'x'; /* hacivat:allow-secret */",
        "CREATE USER a PASSWORD 'x'; --  hacivat:allow-secret and why",
    ] {
        assert!(scan(t).is_empty(), "{t}");
    }
    // ## m4b-t06-secrets (fix round 1, m1): the marker must be inside a comment in both
    // readings, so `#` (a comment in MySQL only) does not silence, and neither does a quote.
    for t in [
        "CREATE USER a PASSWORD 'x' hacivat:allow-secret",
        "CREATE USER a PASSWORD 'x'; # hacivat:allow-secret",
        "CREATE USER a PASSWORD 'pw -- hacivat:allow-secret'",
        "CREATE USER a PASSWORD 'p#ss'; SELECT 'hacivat:allow-secret'",
        "SELECT data #> '{a}', 'hacivat:allow-secret'; CREATE USER a PASSWORD 's3cret'",
        "SELECT 'it\\'s -- '; CREATE USER a PASSWORD 's3cret'; SELECT ' hacivat:allow-secret'",
    ] {
        assert_eq!(kinds(&scan(t)), [Password], "{t}");
    }
}

#[test]
fn finding_never_holds_the_value() {
    let dbg = format!("{:?}", scan("CREATE USER a PASSWORD 's3cret'"));
    assert!(dbg.contains("Password"), "{dbg}");
    assert!(!dbg.contains("s3cret"), "{dbg}");
    let token = concat!("gh", "p_0123456789abcdefghijABCDEFGHIJ012345");
    let t = format!("-- {token}\nSELECT dblink_connect('host=h password=hv_test_pw')");
    let dbg = format!("{:?}", scan(&t));
    assert!(
        dbg.contains("AccessToken") && dbg.contains("ConnectionString"),
        "{dbg}"
    );
    assert!(!dbg.contains(token), "{dbg}");
    assert!(!dbg.contains("hv_test_pw"), "{dbg}");
}

#[test]
fn token_formats_need_no_history_spans() {
    assert!(
        scan_secrets("SELECT 'AKIAIOSFODNN7EXAMPLE'", &[])
            .iter()
            .any(|f| f.kind == AccessToken)
    );
}

#[test]
fn line_numbers_are_zero_based_and_crlf_safe() {
    let t =
        "SELECT 1;\r\nCREATE USER a PASSWORD 'x';\r\n\r\n-- İ ş\r\nSELECT 'AKIAIOSFODNN7EXAMPLE';";
    let f = scan(t);
    let lines: Vec<(u32, SecretKind)> = f.iter().map(|f| (f.line, f.kind)).collect();
    assert_eq!(lines, [(1, Password), (4, AccessToken)]);
    // The ranges point at the literals.
    assert_eq!(&t[f[0].range.clone()], "'x'");
    assert_eq!(&t[f[1].range.clone()], "AKIAIOSFODNN7EXAMPLE");
    // An allow comment ending in `\r` still silences its own line only.
    let t = "CREATE USER a PASSWORD 'x'; -- hacivat:allow-secret\r\nCREATE USER b PASSWORD 'y';";
    let f = scan(t);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].line, 1);
}

#[test]
fn every_pattern_row_names_its_source() {
    let table: toml::Table = toml::from_str(include_str!("../data/secret_patterns.toml")).unwrap();
    let rows = table["pattern"].as_array().unwrap();
    assert!(rows.len() >= 9, "{}", rows.len());
    let mut ids = Vec::new();
    for row in rows {
        let row = row.as_table().unwrap();
        let id = row["id"].as_str().unwrap();
        assert!(!id.is_empty() && !ids.contains(&id), "{id}");
        ids.push(id);
        let kind = row["kind"].as_str().unwrap();
        assert!(
            [
                "access_token",
                "test_key",
                "private_key",
                "jwt",
                "connection_string",
                "password"
            ]
            .contains(&kind),
            "{id}: {kind}"
        );
        regex::Regex::new(row["regex"].as_str().unwrap()).unwrap();
        // The source is one of the vendor or RFC documents alg/28 cites (S9–S16, S21, S22).
        let source = row["source"].as_str().unwrap();
        let cited = [9, 10, 11, 12, 13, 14, 15, 16, 21, 22]
            .iter()
            .any(|n| source.starts_with(&format!("alg/28 S{n} ")));
        assert!(cited, "{id}: {source}");
        for key in row.keys() {
            assert!(
                ["id", "kind", "regex", "source", "check", "min_len"].contains(&key.as_str()),
                "{id}: {key}"
            );
        }
    }
    // The scanner loaded this same table: a table that failed to load answers `Unscanned`.
    assert!(scan_secrets("SELECT 1", &[]).is_empty());
}

#[test]
fn history_redaction_is_unchanged() {
    // `hacivat-exec`'s own redaction tests stay green unchanged (`tests/history.rs`,
    // `src/history/redact.rs`); here: every statement the history masks yields spans, and the
    // value-position ones are what the warning reports.
    for (t, k) in [
        ("ALTER ROLE app PASSWORD 's3cret'", SecretSpanKind::Value),
        (
            "CREATE SUBSCRIPTION s CONNECTION 'host=h user=u password=s3cret' PUBLICATION p",
            SecretSpanKind::ConnectionValue,
        ),
        (
            "SELECT * FROM users WHERE password_hash = 's3cret'",
            SecretSpanKind::Armed,
        ),
    ] {
        let s = secret_spans(t);
        let at = t.find("s3cret").unwrap();
        assert!(
            s.iter()
                .any(|(r, kind)| *kind == k && r.start <= at && at + 6 <= r.end),
            "{t}: {s:?}"
        );
    }
    assert!(secret_spans("SELECT * FROM t WHERE a = 'x'").is_empty());
}

#[test]
fn armed_strings_and_column_names_are_not_reported() {
    // C9: the history masks every string of an armed statement; the warning reports only the
    // value positions.
    for t in [
        "SELECT password_hash FROM users WHERE password_hash = 'x'",
        "SELECT * FROM users WHERE password_reset_at > '2030-01-01'",
        "COMMENT ON COLUMN users.pw IS 'the password hash'",
        "INSERT INTO t (note) VALUES ('change your password')",
        "SELECT api_key FROM t",
    ] {
        let f = scan(t);
        assert!(f.is_empty(), "{t}: {:?}", kinds(&f));
    }
}

#[test]
fn placeholders_and_parameters_are_not_reported() {
    for t in [
        "CREATE USER app PASSWORD $1",
        "CREATE USER app PASSWORD ?",
        "SET PASSWORD FOR a = @pw",
        "CREATE USER app PASSWORD '<password>'",
        "CREATE USER app PASSWORD '{{secret}}'",
        "CREATE USER app PASSWORD '  '",
        "SELECT dblink_connect('host=h password=${PW}')",
        "-- postgresql://u:${PW}@db/orders",
        concat!("SELECT 'pk_", "live_0123456789abcdefABCDEF01'"),
    ] {
        let f = scan(t);
        assert!(f.is_empty(), "{t}: {:?}", kinds(&f));
    }
    // A literal default is still a password.
    assert_eq!(
        kinds(&scan("CREATE USER app PASSWORD 'changeme'")),
        [Password]
    );
}

#[test]
fn more_reported_forms() {
    let rows: [(&str, SecretKind); 9] = [
        (
            "ALTER USER u IDENTIFIED WITH mysql_native_password BY 'pw'",
            Password,
        ),
        ("ALTER LOGIN l WITH PASSWORD = 's3cret'", Password),
        ("PRAGMA key = 's3cret'", Password),
        ("ATTACH 'x.db' AS x KEY 's3cret'", Password),
        ("/* CREATE USER a PASSWORD 's3cret' */ SELECT 1", Password),
        (
            "DO $$ BEGIN ALTER ROLE r PASSWORD 's3cret'; END $$",
            Password,
        ),
        (
            "DO $$ BEGIN EXECUTE 'ALTER ROLE r PASSWORD ''s3cret'''; END $$",
            Password,
        ),
        ("\\connect host=h password=s3cret", ConnectionString),
        ("SELECT 'redis://:s3cret@cache:6379'", ConnectionString),
    ];
    for (t, k) in rows {
        let f = scan(t);
        assert!(f.iter().any(|f| f.kind == k), "{t}: {:?}", kinds(&f));
        // Each finding points at the value, never at the whole statement.
        for f in &f {
            assert!(t[f.range.clone()].contains("s3cret") || t[f.range.clone()].contains("pw"));
        }
    }
    // Test-mode keys are a lower-severity kind; the PEM forms of RFC 7468 count.
    let test_key = concat!("SELECT 'rk_", "test_0123456789abcdefABCDEF01'");
    assert_eq!(kinds(&scan(test_key)), [SecretKind::TestKey]);
    for label in ["", "ENCRYPTED ", "EC ", "DSA "] {
        let t = pem_header(label);
        assert_eq!(kinds(&scan(&t)), [PrivateKey], "{label}");
    }
    // A Slack app token and a fine-grained GitHub token.
    for t in [
        concat!("-- xa", "pp-1-A0123456789-0123456789-abcdef"),
        concat!("-- github", "_pat_0123456789ABCDEFGHIJKL_0123456789"),
        concat!("SELECT 'AS", "IAIOSFODNN7EXAMPLE'"),
    ] {
        assert_eq!(kinds(&scan(t)), [AccessToken], "{t}");
    }
    // An unsecured JWT (empty signature) still counts; a three-part base64url run whose header
    // is not a JSON object naming `alg` does not.
    assert_eq!(
        kinds(&scan(concat!(
            "'eyJ",
            "hbGciOiJub25lIn0.eyJ",
            "zdWIiOiIxIn0.'"
        ))),
        [Jwt]
    );
    assert!(scan(concat!("'eyJ", "zdWIiOiIxIn0.eyJ", "zdWIiOiIxIn0.c2ln'")).is_empty());
}

#[test]
fn one_finding_per_literal() {
    // Both of the redactor's readings and the URI row find the same value: one finding.
    let f = scan("SELECT dblink_connect('postgresql://u:s3cret@h/db')");
    assert_eq!(kinds(&f), [ConnectionString]);
    let f = scan("CREATE USER a PASSWORD 'x';\nCREATE USER b PASSWORD 'y';");
    assert_eq!(f.iter().map(|f| f.line).collect::<Vec<_>>(), [0, 1]);
}

#[test]
fn hostile_spans_never_panic() {
    // Spans out of range or inside a character: reported (fail closed), never a panic.
    let t = "SELECT 'ş'";
    let reversed = Range { start: 10, end: 9 };
    for bad in [9..10, 5..400, 400..500, reversed] {
        let f = scan_secrets(t, &[(bad.clone(), SpanKind::Password)]);
        assert_eq!(kinds(&f), [Password], "{bad:?}");
        let r = f[0].range.clone();
        assert!(t.get(r.clone()).is_some(), "{bad:?} → {r:?}");
    }
}

// ## m4b-t06-secrets (fix round 1)
/// The literals `scan` reports in `t`, for assertions on what a finding points at.
fn reported(t: &str) -> Vec<(&str, SecretKind)> {
    scan(t)
        .into_iter()
        .map(|f| (&t[f.range.clone()], f.kind))
        .collect()
}

#[test]
fn comment_prose_is_quiet() {
    // I1: a bare word after a secret word in a comment is prose, never a value.
    for t in [
        "-- remember to rotate the password monthly",
        "-- password policy: at least twelve characters",
        "-- TODO change password later",
        "# password reset flow",
        "/* the password column holds bcrypt hashes */",
        "/* the password column holds … */",
        "-- IDENTIFIED BY is MySQL syntax",
    ] {
        assert!(scan(t).is_empty(), "{t}: {:?}", reported(t));
    }
    // A quoted value, a `password=` value, a URI password or a token in a comment still is.
    assert_eq!(
        reported("/* CREATE USER a PASSWORD 's3cret' */ SELECT 1"),
        [("'s3cret'", Password)]
    );
    assert_eq!(
        reported("-- password=s3cret"),
        [("s3cret", ConnectionString)]
    );
}

#[test]
fn pair_names_follow_c9_inside_strings() {
    // I2: a pair whose name only mentions a secret is no value.
    for t in [
        "SELECT '{\"password_changed_at\": \"2024-01-01\"}'",
        "SELECT '{\"password_policy\": \"strict\"}'::jsonb",
        "INSERT INTO cfg VALUES ('password_min_length=12')",
        "INSERT INTO cfg VALUES ('password_hash=abc')",
    ] {
        assert!(scan(t).is_empty(), "{t}: {:?}", reported(t));
    }
    for t in [
        "SELECT dblink_connect('host=h user=u password=s3cret')",
        "SELECT 'Server=h;Pwd=s3cret;'",
        "SELECT '{\"access_token\": \"s3cret\"}'",
        "SELECT '{\"apiKey\":\"s3cret\"}'",
        "SELECT 'aws_secret_access_key=s3cret'",
        "SELECT 'Authorization: Bearer s3cret'",
    ] {
        assert_eq!(reported(t), [("s3cret", ConnectionString)], "{t}");
    }
}

#[test]
fn only_a_crypto_key_is_reported() {
    // m2: the key or password argument, never the data, the options or a nested call's
    // argument. The key positions: `crypt` argument 1 (the password), `pgp_pub_decrypt`
    // arguments 2 and 3, argument 2 of every other crypto function.
    assert_eq!(
        reported("SELECT pgp_sym_encrypt('Alice SSN', 'k3y', 'cipher-algo=aes256')"),
        [("'k3y'", Password)]
    );
    assert_eq!(
        reported("SELECT crypt('newpw', gen_salt('bf'))"),
        [("'newpw'", Password)]
    );
}

#[test]
fn one_finding_per_literal_across_kinds() {
    // m3: a history value and a token row over one literal are one finding.
    let t = "-- jdbc:postgresql://h/db?user=u&password=s3cret";
    assert_eq!(reported(t), [("s3cret", ConnectionString)]);
    let t = "ALTER ROLE app PASSWORD 'AKIAIOSFODNN7EXAMPLE'";
    assert_eq!(kinds(&scan(t)), [AccessToken]);
}

#[test]
fn near_miss_token_shapes_are_quiet() {
    for t in [
        // m4: 20 capitals with no digit after the prefix.
        "SELECT * FROM ASIAPACIFICCUSTOMERS",
        // m5: too short, or a page name.
        concat!("-- xo", "xb-1-2"),
        concat!("-- see the xa", "pp-dev-guide-v2 page"),
    ] {
        assert!(scan(t).is_empty(), "{t}: {:?}", reported(t));
    }
    assert_eq!(kinds(&scan("SELECT 'AKIAIOSFODNN7EXAMPLE'")), [AccessToken]);
}

#[test]
fn more_templates_are_quiet() {
    // m6: printf, Python, shell, CI and redaction placeholders.
    for t in [
        "CREATE USER app PASSWORD '%s'",
        "CREATE USER app PASSWORD '%(pw)s'",
        "CREATE USER app PASSWORD '{pw}'",
        "CREATE USER app PASSWORD '$PW'",
        "CREATE USER app PASSWORD '${{ secrets.PW }}'",
        "CREATE USER app PASSWORD '{{ vault.db.pw }}'",
        "CREATE USER app PASSWORD '[REDACTED]'",
    ] {
        assert!(scan(t).is_empty(), "{t}: {:?}", reported(t));
    }
    // m11: a quoted `'null'` is a literal, not SQL NULL: reported.
    assert_eq!(kinds(&scan("CREATE USER app PASSWORD 'null'")), [Password]);
}

#[test]
fn sp_addlogin_and_pgp_private_keys_are_reported() {
    for t in [
        "EXEC sp_addlogin 'l', 's3cret'",
        "EXECUTE sys.sp_addlogin @loginame = N'l', @passwd = N's3cret', @defdb = 'master'",
    ] {
        assert_eq!(reported(t), [("'s3cret'", Password)], "{t}");
    }
    assert!(scan("EXECUTE sp_addlogin 'Victoria', '<password>';").is_empty());
    let pgp = format!(
        "{}BEGIN PGP {}",
        "-----",
        concat!("PRIVATE KEY", " BLOCK-----")
    );
    assert_eq!(kinds(&scan(&pgp)), [PrivateKey]);
    let public = format!("{}BEGIN PGP PUBLIC KEY BLOCK{}", "-----", "-----");
    assert!(scan(&public).is_empty());
}

// ## m4b-t06-secrets (fix round 2)
#[test]
fn glued_password_keys_are_reported() {
    // I-fr1-1: a key that ends with the secret word names the value.
    for t in [
        "-- PGPASSWORD=s3cret psql -h db -U app",
        "SELECT 'PGPASSWORD=s3cret'",
        "SELECT 'host=h sslpassword=s3cret'",
        "SELECT 'host=h dbpassword=s3cret'",
    ] {
        assert_eq!(reported(t), [("s3cret", ConnectionString)], "{t}");
    }
    // One that only starts with it, or ends with another word, does not.
    for t in [
        "SELECT 'passwordhash=abc'",
        "SELECT 'password_hash=abc'",
        "SELECT 'password_policy=strict'",
        "SELECT 'password_file=/etc/pw'",
    ] {
        assert!(scan(t).is_empty(), "{t}: {:?}", reported(t));
    }
}

#[test]
fn a_hash_glued_to_a_name_hides_no_value() {
    // n1: MySQL reads `#1 …` as a comment, the standard reading as an Oracle name; warn.
    let t = "CREATE USER app#1 IDENTIFIED BY tiger";
    assert_eq!(reported(t), [("tiger", Password)]);
    // A `#` comment that starts a line or follows white space stays prose.
    for t in ["# password reset flow", "SELECT 1; # password reset flow"] {
        assert!(scan(t).is_empty(), "{t}: {:?}", reported(t));
    }
}

#[test]
fn a_brace_template_holds_a_name() {
    // n2: `{…}` is a placeholder only when it holds a name.
    assert_eq!(
        reported("CREATE USER app PASSWORD '{s3cr3t-P@ss!}'"),
        [("'{s3cr3t-P@ss!}'", Password)]
    );
    for t in [
        "CREATE USER app PASSWORD '{pw}'",
        "CREATE USER app PASSWORD '{ db.pw-1 }'",
        "CREATE USER app PASSWORD '${APP_PW}'",
    ] {
        assert!(scan(t).is_empty(), "{t}: {:?}", reported(t));
    }
}

#[test]
fn the_allow_marker_needs_a_space_after_the_dashes() {
    // n4: MySQL takes `--` as a comment only before white space, and the marker must be in a
    // comment in both readings, so `--hacivat:allow-secret` does not silence its line.
    assert_eq!(
        kinds(&scan("CREATE USER a PASSWORD 'x'; --hacivat:allow-secret")),
        [Password]
    );
    assert!(scan("CREATE USER a PASSWORD 'x'; -- hacivat:allow-secret").is_empty());
}

#[test]
fn a_vertical_tab_does_not_hang_the_scan() {
    // Found by the byte-identity test: M3's redactor never returned on a vertical tab outside
    // quotes, so the spans, and the scan with them, never came back.
    let t = "SELECT\u{b}1;\nCREATE USER a PASSWORD\u{b}'s3cret'";
    assert_eq!(reported(t), [("'s3cret'", Password)]);
}
// ## m4b-t06-secrets

#[test]
#[ignore = "perf"]
fn scan_of_2mb_under_20ms() {
    if std::env::var("HACIVAT_PERF").as_deref() != Ok("1") {
        return;
    }
    let block = concat!(
        "SELECT a, b, c FROM orders WHERE id = 42 AND note = 'İstanbul şube';\n",
        "UPDATE users SET passwd_changed = now() WHERE id = 7;\n",
        "-- a comment about nothing in particular, with a URL https://example.org/x\n",
        "ALTER ROLE app PASSWORD 's3cret';\n",
    );
    let text = block.repeat(2 * 1024 * 1024 / block.len() + 1);
    // The history spans are the app's input (measured, not asserted: the redactor is M3b's).
    let start = Instant::now();
    let spans = spans(&text);
    let spans_took = start.elapsed();
    let start = Instant::now();
    let f = scan_secrets(&text, &spans);
    let took = start.elapsed();
    assert!(!f.is_empty());
    println!(
        "HV-FACT t06 scan_bytes={} findings={} scan_ms={} spans_ms={}",
        text.len(),
        f.len(),
        took.as_millis(),
        spans_took.as_millis()
    );
    assert!(took.as_millis() < 20, "{took:?}");
}
// ## m4b-t06-secrets
