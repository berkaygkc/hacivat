//! URL template tests (Task 3, HV-DRV-02/03): compile, render for display, copy and the driver,
//! match typed text, and rewrite the endpoint. These use their own template strings; the
//! catalog-wide compile and round-trip test is Task 7's.

use std::time::{Duration, Instant};

use hacivat_driver::url::{
    FieldId, FieldMap, RenderError, RenderTarget, TemplateError, compile, match_template, render,
    render_driver, rewrite_endpoint,
};
use secrecy::{ExposeSecret, SecretString};

const PG_URI: &str = r"postgresql://[{user}[:{password}]@]{host:host}[:{port:port}]/{database}[\?<&,{:param}={:param}>]";

fn pg_uri() -> hacivat_driver::url::Template {
    compile(PG_URI, "default").unwrap()
}

fn f(pairs: &[(&str, &str)]) -> FieldMap {
    let mut m = FieldMap::default();
    for (k, v) in pairs {
        m.values.insert(FieldId::from_id(k).unwrap(), v.to_string());
    }
    m
}

fn extras(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn compile_vectors() {
    let t = compile(
        "postgresql://{host:host:localhost}[:{port:port:5432}]",
        "default",
    )
    .unwrap();
    assert_eq!(t.field_set, [FieldId::Host, FieldId::Port].into());
    assert_eq!(t.default_of(FieldId::Host), Some("localhost"));
    assert_eq!(t.default_of(FieldId::Port), Some("5432"));
    assert_eq!(
        compile("{host:ipv6:\\:\\:1}", "ipv6")
            .unwrap()
            .default_of(FieldId::Host),
        Some("::1")
    );
    assert!(
        compile("<,{:ident}={:param}>", "x")
            .unwrap()
            .has_passthrough
    );
    assert_eq!(
        compile("{host", "x").unwrap_err(),
        TemplateError::Unbalanced { pos: 0 }
    );
    assert_eq!(
        compile("{hots}", "x").unwrap_err(),
        TemplateError::UnknownField {
            name: "hots".into()
        }
    );
}

#[test]
fn compile_errors_and_shapes() {
    assert_eq!(
        compile("a[b{user}", "x").unwrap_err(),
        TemplateError::Unbalanced { pos: 1 }
    );
    assert_eq!(
        compile("ab]", "x").unwrap_err(),
        TemplateError::Unbalanced { pos: 2 }
    );
    assert_eq!(
        compile("x<,{host}", "x").unwrap_err(),
        TemplateError::Unbalanced { pos: 1 }
    );
    assert_eq!(
        compile("{port:port/[0-9}", "x").unwrap_err(),
        TemplateError::BadRegex { pos: 0 }
    );
    assert_eq!(
        compile("{port:port/(?\\:\\w\\{500\\})\\{500\\}}", "x").unwrap_err(),
        TemplateError::BadRegex { pos: 0 },
        "the regex size limit applies"
    );
    assert_eq!(
        compile("ab{host:nosuchtype}", "x").unwrap_err(),
        TemplateError::BadRegex { pos: 2 }
    );
    assert_eq!(
        compile("{host:host:h:%:extra}", "x").unwrap_err(),
        TemplateError::TooManyFields { pos: 0 }
    );
    let empty = compile("", "url_only").unwrap();
    assert!(empty.field_set.is_empty() && !empty.has_passthrough);
    assert_eq!(empty.id, "url_only");
    let hosts = compile(r"<\,,{host}[:{port}]>", "hosts").unwrap();
    assert_eq!(hosts.field_set, [FieldId::Host, FieldId::Port].into());
    assert!(!hosts.has_passthrough);
    let nested = compile("[[{user}:]{password}@]{host}", "x").unwrap();
    assert_eq!(
        nested.field_set,
        [FieldId::Host, FieldId::User, FieldId::Password].into()
    );
    assert_eq!(nested.default_of(FieldId::User), None);
}

#[test]
fn render_display_and_copy() {
    let t = pg_uri();
    assert_eq!(
        render(
            &t,
            &f(&[("host", "db"), ("database", "app")]),
            &[],
            RenderTarget::Display {
                has_password: false
            }
        )
        .unwrap(),
        "postgresql://db/app"
    );
    assert_eq!(
        render(
            &t,
            &f(&[("host", "db"), ("database", "app")]),
            &[("sslmode".into(), "require".into())],
            RenderTarget::Copy
        )
        .unwrap(),
        "postgresql://db/app?sslmode=require"
    );
    let u = f(&[("host", "db"), ("database", "app"), ("user", "a@b")]);
    assert_eq!(
        render(&t, &u, &[], RenderTarget::Display { has_password: true }).unwrap(),
        "postgresql://a%40b:****@db/app"
    );
    assert_eq!(
        render(&t, &u, &[], RenderTarget::Copy).unwrap(),
        "postgresql://a%40b@db/app"
    );
    assert_eq!(
        render(
            &t,
            &f(&[("host", "::1"), ("database", "app")]),
            &[],
            RenderTarget::Copy
        )
        .unwrap(),
        "postgresql://[::1]/app"
    );
    assert_eq!(
        render(
            &t,
            &f(&[("host", "db"), ("database", "данные")]),
            &[],
            RenderTarget::Copy
        )
        .unwrap(),
        "postgresql://db/%D0%B4%D0%B0%D0%BD%D0%BD%D1%8B%D0%B5"
    );
    assert_eq!(
        render(&t, &f(&[("database", "app")]), &[], RenderTarget::Copy).unwrap_err(),
        RenderError::MissingField(FieldId::Host)
    );
}

#[test]
fn render_rules() {
    let t = pg_uri();
    // A port renders; extras keep their order and are percent-encoded.
    assert_eq!(
        render(
            &t,
            &f(&[("host", "db"), ("port", "5433"), ("database", "app")]),
            &extras(&[("application_name", "my app"), ("options", "-c x=1&y")]),
            RenderTarget::Copy
        )
        .unwrap(),
        "postgresql://db:5433/app?application_name=my%20app&options=-c%20x=1%26y"
    );
    // An optional group renders only for a user value, never for a default alone.
    let d = compile("pg://{host:host:localhost}[:{port:port:5432}]", "x").unwrap();
    assert_eq!(
        render(&d, &FieldMap::default(), &[], RenderTarget::Copy).unwrap(),
        "pg://localhost"
    );
    // An empty value counts as missing.
    assert_eq!(
        render(
            &t,
            &f(&[("host", ""), ("database", "app")]),
            &[],
            RenderTarget::Copy
        )
        .unwrap_err(),
        RenderError::MissingField(FieldId::Host)
    );
    // A value for a field the template cannot carry is refused, not dropped.
    assert_eq!(
        render(
            &t,
            &f(&[("host", "db"), ("database", "app"), ("file", "/x.db")]),
            &[],
            RenderTarget::Copy
        )
        .unwrap_err(),
        RenderError::FieldNotAllowed(FieldId::File)
    );
    // Alternatives: the first with a user value wins.
    let alt = compile("s://[{user}@|{socket:path}!]{host}", "x").unwrap();
    assert_eq!(
        render(
            &alt,
            &f(&[("host", "h"), ("socket", "/tmp/s")]),
            &[],
            RenderTarget::Copy
        )
        .unwrap(),
        "s:///tmp/s!h"
    );
    // A host list renders every host, IPv6 bracketed.
    let hosts = compile(r"r://<\,,{host:host}[:{port:port}]>", "x").unwrap();
    let mut m = FieldMap {
        hosts: vec![("h1".into(), Some(7000)), ("::1".into(), None)],
        ..FieldMap::default()
    };
    m.values.insert(FieldId::Host, "h1".into());
    assert_eq!(
        render(&hosts, &m, &[], RenderTarget::Copy).unwrap(),
        "r://h1:7000,[::1]"
    );
}

#[test]
fn display_and_copy_never_contain_the_password() {
    let pw = SecretString::from("p:w@/%x");
    let fm = f(&[("host", "db"), ("database", "app"), ("user", "u")]);
    for target in [
        RenderTarget::Display { has_password: true },
        RenderTarget::Copy,
    ] {
        let s = render(&pg_uri(), &fm, &[], target).unwrap();
        assert!(!s.contains("p:w") && !s.contains("p%3Aw"), "{s}");
    }
    assert_eq!(
        render_driver(&pg_uri(), &fm, &[], &pw)
            .unwrap()
            .expose_secret(),
        "postgresql://u:p%3Aw%40%2F%25x@db/app"
    );
}

#[test]
fn without_a_user_the_credentials_group_disappears() {
    let fm = f(&[("host", "db"), ("database", "app")]);
    for t in [
        pg_uri(),
        compile("pg://[{user}:{password}@]{host}/{database}", "x").unwrap(),
    ] {
        assert_eq!(
            render(&t, &fm, &[], RenderTarget::Display { has_password: true })
                .unwrap()
                .split("//")
                .nth(1),
            Some("db/app")
        );
    }
    let flat = compile("pg://[{user}:{password}@]{host}/{database}", "x").unwrap();
    assert_eq!(
        render(
            &flat,
            &f(&[("host", "db"), ("database", "app"), ("user", "u")]),
            &[],
            RenderTarget::Display { has_password: true }
        )
        .unwrap(),
        "pg://u:****@db/app"
    );
}

#[test]
fn a_password_in_the_field_map_is_never_rendered() {
    // A matched map can carry the password (Task 7 moves it out); render must not trust it.
    let mut fm = f(&[("host", "db"), ("database", "app"), ("user", "u")]);
    fm.values.insert(FieldId::Password, "s3cret".into());
    for target in [
        RenderTarget::Display {
            has_password: false,
        },
        RenderTarget::Display { has_password: true },
        RenderTarget::Copy,
    ] {
        let s = render(&pg_uri(), &fm, &[], target).unwrap();
        assert!(!s.contains("s3cret"), "{s}");
    }
}

#[test]
fn rewrite_endpoint_vectors() {
    assert_eq!(
        rewrite_endpoint(
            &pg_uri(),
            "postgresql://db.example:5432/my%20db?host=db.example&port=5432#x",
            "127.0.0.1",
            49505
        )
        .unwrap(),
        "postgresql://127.0.0.1:49505/my%20db?host=db.example&port=5432#x"
    );
    assert!(matches!(
        rewrite_endpoint(
            &pg_uri(),
            "postgresql://first:5432,second:5432/demo",
            "127.0.0.1",
            49505
        ),
        Err(RenderError::CannotRedirect(_))
    ));
}

#[test]
fn rewrite_endpoint_edges() {
    // An IPv6 host is replaced brackets and all; non-ASCII text elsewhere is untouched.
    assert_eq!(
        rewrite_endpoint(
            &pg_uri(),
            "postgresql://[2001:db8::1]:5432/данные?option=%2520",
            "127.0.0.1",
            49505
        )
        .unwrap(),
        "postgresql://127.0.0.1:49505/данные?option=%2520"
    );
    // No port in the URL, but the template has an optional port after the host: inserted.
    assert_eq!(
        rewrite_endpoint(&pg_uri(), "postgresql://u@db/app", "::1", 49505).unwrap(),
        "postgresql://u@[::1]:49505/app"
    );
    // A template without a port placeholder cannot take the new port.
    let fixed = compile("pg://{host:host}:5432/{database}", "x").unwrap();
    assert_eq!(
        rewrite_endpoint(&fixed, "pg://db:5432/app", "127.0.0.1", 1),
        Err(RenderError::CannotRedirect("no_port".into()))
    );
    // A host list with more than one host is refused.
    let hosts = compile(r"r://<\,,{host:host}[:{port:port}]>", "x").unwrap();
    assert_eq!(
        rewrite_endpoint(&hosts, "r://a:1,b:2", "127.0.0.1", 9),
        Err(RenderError::CannotRedirect("multiple_hosts".into()))
    );
    assert_eq!(
        rewrite_endpoint(&hosts, "r://a:1", "127.0.0.1", 9).unwrap(),
        "r://127.0.0.1:9"
    );
    // A socket template has no host to move.
    let socket = compile(r"pg://[{user}@]/{database}\?host={socket:path}", "x").unwrap();
    assert_eq!(
        rewrite_endpoint(&socket, "pg:///app?host=/tmp", "127.0.0.1", 9),
        Err(RenderError::CannotRedirect("no_host".into()))
    );
    assert_eq!(
        rewrite_endpoint(&pg_uri(), "garbage", "127.0.0.1", 9),
        Err(RenderError::CannotRedirect("no_match".into()))
    );
}

#[test]
fn matcher_whole_input_scheme_case_and_budget() {
    let t = compile("jdbc:mysql://{host:host}[:{port:port}]/{database}", "jdbc").unwrap();
    assert_eq!(
        match_template(&t, "JDBC:mysql://h:3307/d")
            .unwrap()
            .0
            .values[&FieldId::Port],
        "3307"
    );
    assert!(match_template(&t, "jdbc:mysql://h/d trailing junk").is_none());
    let long = "a".repeat(64 * 1024 + 1);
    let start = Instant::now();
    assert!(match_template(&t, &long).is_none());
    assert!(start.elapsed() < Duration::from_millis(50));
}

#[test]
fn matcher_binds_fields_extras_and_passwords() {
    let (m, extra) = match_template(
        &pg_uri(),
        "postgresql://a%40b:p%3Aw@db:05432/%D0%B4%D0%B1?sslmode=require&empty=",
    )
    .unwrap();
    assert_eq!(m.values[&FieldId::User], "a@b");
    assert_eq!(m.values[&FieldId::Host], "db");
    assert_eq!(m.values[&FieldId::Port], "05432");
    assert_eq!(m.values[&FieldId::Database], "дб");
    // A `FieldMap` never holds a password: the public matcher drops it.
    assert!(!m.values.contains_key(&FieldId::Password));
    assert!(!format!("{m:?}").contains("p:w"));
    assert_eq!(extra, extras(&[("sslmode", "require"), ("empty", "")]));

    let (v6, _) = match_template(&pg_uri(), "postgresql://[::1]:5432/app").unwrap();
    assert_eq!(v6.values[&FieldId::Host], "::1");
    // Only the scheme folds case (`jdbc:mysql:` is all scheme); other literals do not.
    let jdbc = compile("jdbc:mysql://{host:host}/App", "x").unwrap();
    assert!(match_template(&jdbc, "Jdbc:MYSQL://h/App").is_some());
    assert!(match_template(&jdbc, "jdbc:mysql://h/app").is_none());
}

#[test]
fn matcher_rejects_bad_escapes_and_handles_lists() {
    assert!(match_template(&pg_uri(), "postgresql://db/a%4Gb").is_none());
    assert!(match_template(&pg_uri(), "postgresql://db/ab%").is_none());
    assert!(match_template(&pg_uri(), "postgresql://db/a%FFb").is_none());
    let hosts = compile(r"r://<\,,{host:host}[:{port:port}]>[/{database}]", "x").unwrap();
    let (m, _) = match_template(&hosts, "r://h1:7000,[::1],h3:7002/db").unwrap();
    assert_eq!(
        m.hosts,
        vec![
            ("h1".to_string(), Some(7000)),
            ("::1".to_string(), None),
            ("h3".to_string(), Some(7002))
        ]
    );
    assert_eq!(m.values[&FieldId::Host], "h1");
    assert_eq!(m.values[&FieldId::Database], "db");
    // A lazy path stops where the rest of the template can take over.
    let socket = compile(
        r"pg:///{database}\?host={socket:path}[&<&,{:ident}={:param}>]",
        "x",
    )
    .unwrap();
    let (s, extra) = match_template(&socket, "pg:///app?host=/var/run&sslmode=disable").unwrap();
    assert_eq!(s.values[&FieldId::Socket], "/var/run");
    assert_eq!(extra, extras(&[("sslmode", "disable")]));
    // A key may not be empty.
    let q = compile(r"q://{host}[\?<&,{:ident}={:param}>]", "x").unwrap();
    assert!(match_template(&q, "q://h?valid=value&=missing").is_none());
}

#[test]
fn matcher_budget_stops_pathological_templates() {
    let t = compile("{database:text/.+}{user:text/.+}{file:file/.+}x", "x").unwrap();
    let input = "a".repeat(60 * 1024);
    let start = Instant::now();
    assert!(match_template(&t, &input).is_none());
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn render_then_match_round_trips_on_these_templates() {
    // Keys are `ident` here: a `param` key may itself hold `=`, so it cannot round-trip.
    let t = compile(
        r"postgresql://[{user}[:{password}]@]{host:host}[:{port:port}][/{database}][\?<&,{:ident}={:param}>]",
        "default",
    )
    .unwrap();
    let fields = f(&[
        ("host", "db.example"),
        ("port", "6543"),
        ("database", "my db/ü?#"),
        ("user", "a@b:c"),
    ]);
    let extra = extras(&[("application_name", "x&y=z"), ("sslmode", "require")]);
    let url = render(&t, &fields, &extra, RenderTarget::Copy).unwrap();
    let (m, e) = match_template(&t, &url).unwrap();
    assert_eq!(m.values, fields.values, "{url}");
    assert_eq!(e, extra, "{url}");
}
