//! The catalog-wide template check (E3, P14): every catalog template compiles, and parse
//! inverts render on it, for generated field values that include IPv6 hosts, non-ASCII
//! database names and URL-reserved characters.

use hacivat_driver::EngineCatalog;
use hacivat_driver::url::{
    FieldId, FieldMap, ParseOutcome, ParseResult, RenderTarget, Template, compile, parse, render,
};
use hacivat_proto::DbmsKind;

fn ok(r: ParseResult) -> ParseOutcome {
    match r {
        ParseResult::Recognised(o) => o,
        ParseResult::Unrecognised(why) => panic!("unrecognised: {why:?}"),
    }
}

const SAMPLES: usize = 50;

const HOSTS: [&str; 9] = [
    "h",
    "db.example.com",
    "::1",
    "127.0.0.1",
    "2001:db8::1",
    "veritabanı.örnek",
    "localhost",
    "my-host_1",
    "ın.İ",
];
const USERS: [&str; 9] = [
    "u",
    "a@b",
    "alice",
    "üser",
    "user name",
    "a:b",
    "o'neil",
    "x%y",
    "a/b?c#d",
];
const DATABASES: [&str; 10] = [
    "app",
    "İzmir",
    "ırmak",
    "данные",
    "my db",
    "a/b",
    "a?b",
    "50%",
    "x&y=z",
    "a,b;c",
];
const SOCKETS: [&str; 5] = [
    "/var/run/postgresql",
    "/tmp/mysql.sock",
    "/tmp/my dir/sock",
    "/tmp/данные/s",
    "/a;b&c?d#e=f",
];
const FILES: [&str; 7] = [
    "/Users/x/app.db",
    "/tmp/my db.sqlite",
    r"C:\data\app.db",
    "relative.db",
    "/tmp/данные.db",
    "/tmp/a#b?c&d.db",
    "/tmp/50%.db",
];

/// `SAMPLES` field maps for `t`, cycling through the pools above. Required fields (host, socket,
/// file) are always set; the others come and go. The password is never in a `FieldMap`.
fn sample_fields(kind: DbmsKind, t: &Template) -> Vec<FieldMap> {
    let default_port = EngineCatalog::embedded()
        .get(kind)
        .and_then(|s| s.default_port)
        .map_or_else(|| "1".to_string(), |p| p.to_string());
    let ports = [default_port.as_str(), "6543", "05432", "65535"];
    let has = |f: FieldId| t.field_set.contains(&f);
    let mut out = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES {
        let mut m = FieldMap::default();
        let mut put = |f: FieldId, v: &str| {
            m.values.insert(f, v.to_string());
        };
        if has(FieldId::Host) {
            put(FieldId::Host, HOSTS[i % HOSTS.len()]);
        }
        if has(FieldId::Socket) {
            put(FieldId::Socket, SOCKETS[i % SOCKETS.len()]);
        }
        if has(FieldId::File) {
            put(FieldId::File, FILES[i % FILES.len()]);
        }
        if has(FieldId::Port) && i % 3 != 0 {
            put(FieldId::Port, ports[(i / 3) % ports.len()]);
        }
        if has(FieldId::User) && i % 4 != 1 {
            put(FieldId::User, USERS[(i * 7 + 1) % USERS.len()]);
        }
        if has(FieldId::Database) && i % 5 != 2 {
            put(FieldId::Database, DATABASES[(i * 3 + 1) % DATABASES.len()]);
        }
        out.push(m);
    }
    // The samples cover what the brief names, wherever the template can carry it.
    let any = |f: FieldId, p: &dyn Fn(&str) -> bool| {
        !has(f) || out.iter().any(|m| m.values.get(&f).is_some_and(|v| p(v)))
    };
    assert!(any(FieldId::Host, &|v| v.contains(':')), "IPv6 host");
    assert!(
        any(FieldId::Database, &|v| !v.is_ascii()),
        "non-ASCII database"
    );
    assert!(any(FieldId::User, &|v| v.contains('@')), "'@' in user");
    out
}

#[test]
fn every_catalog_template_compiles_and_round_trips() {
    let mut checked = 0;
    for spec in EngineCatalog::embedded().engines() {
        for ct in &spec.conn_types {
            if ct.template.is_empty() {
                continue;
            }
            let t = compile(&ct.template, &ct.id).unwrap();
            for fields in sample_fields(spec.kind, &t) {
                let url = render(&t, &fields, &[], RenderTarget::Copy).unwrap();
                let o = ok(parse(&url, Some(spec.kind)));
                assert_eq!(o.fields.values, fields.values, "{url}");
                assert_eq!(
                    (o.kind, o.conn_type.as_str()),
                    (spec.kind, ct.id.as_str()),
                    "{url}"
                );
                assert!(o.extra.is_empty() && o.secrets.is_empty(), "{url}");
                assert!(o.mapped.is_empty() && o.warnings.is_empty(), "{url}");
                checked += 1;
            }
        }
    }
    // Every non-empty template of the four engines: 2 + 2 + 2 + 2.
    assert_eq!(checked, 8 * SAMPLES);
}

#[test]
fn extras_round_trip_in_order() {
    let extra: Vec<(String, String)> = vec![
        ("k".into(), "v w".into()),
        ("x.y[0]".into(), "a=b&c+d".into()),
        ("k".into(), "данные; %41".into()),
    ];
    for spec in EngineCatalog::embedded().engines() {
        for ct in &spec.conn_types {
            if ct.template.is_empty() {
                continue;
            }
            let t = compile(&ct.template, &ct.id).unwrap();
            if !t.has_passthrough {
                continue;
            }
            let fields = sample_fields(spec.kind, &t).swap_remove(4);
            let url = render(&t, &fields, &extra, RenderTarget::Copy).unwrap();
            let o = ok(parse(&url, Some(spec.kind)));
            assert_eq!(o.fields.values, fields.values, "{url}");
            assert_eq!(o.extra, extra, "{url}");
        }
    }
}
