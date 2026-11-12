//! Every reachable error class renders its `error-conn-*` title and body in English and Turkish
//! (HV-CONN-03, Task 10). The golden files are reviewed by hand; regenerate them with
//! `UPDATE_GOLDEN=1 cargo test -p hacivat-driver --test error_fluent_golden`.

use std::path::PathBuf;

use hacivat_driver::errors::reachable_classes;
use hacivat_l10n::{
    ArgValue, Catalogs, FormatInputs, IntoFluentArgs, L10n, MsgRef, Negotiated, Sink, build_runtime,
};
use hacivat_proto::ErrClass;

/// The `error-conn-*` arguments. Every one is `(String)`, the port and the version included, so a
/// port never gets grouping separators (P20).
#[derive(Clone)]
struct ErrArgs(Vec<(&'static str, ArgValue)>);

impl IntoFluentArgs for ErrArgs {
    fn into_args(self) -> Vec<(&'static str, ArgValue)> {
        self.0
    }
}

fn text(s: &str) -> ArgValue {
    ArgValue::Text(s.to_string())
}

fn args(
    name: &str,
    host: &str,
    port: u16,
    user: &str,
    db: &str,
    engine: &str,
    version: &str,
) -> ErrArgs {
    ErrArgs(vec![
        ("name", text(name)),
        ("host", text(host)),
        ("port", text(&port.to_string())),
        ("user", text(user)),
        ("db", text(db)),
        ("engine", text(engine)),
        ("version", text(version)),
        ("file", text("")),
    ])
}

fn l10n(lang: &str) -> L10n {
    let mut chain = vec![lang.parse().unwrap()];
    if lang != "en" {
        chain.push("en".parse().unwrap());
    }
    let neg = Negotiated {
        chain,
        format_locale: lang.into(),
        diags: Vec::new(),
    };
    L10n::new(build_runtime(
        &Catalogs::embedded(),
        &neg,
        &FormatInputs::new(&neg, "UTC"),
        1,
    ))
}

/// Renders one message for the machine sink; panics when it is missing or falls back to English.
fn render(l: &L10n, lang: &str, id: String, a: &ErrArgs) -> String {
    let id: &'static str = Box::leak(id.into_boxed_str());
    let r = l.t_rendered(MsgRef::<ErrArgs>::new(id), a.clone(), Sink::Machine);
    assert!(!r.missing, "{id} is missing");
    assert!(!r.fell_back, "{id} has no {lang} translation");
    r.text
}

/// `<class id> title: …` and `<class id> body: …` for every reachable class, one per line.
fn render_all(lang: &str, a: ErrArgs) -> String {
    let l = l10n(lang);
    let mut out = String::new();
    for class in reachable_classes() {
        let stem = class.fluent_stem();
        for part in ["title", "body"] {
            let line = render(&l, lang, format!("{stem}-{part}"), &a);
            out.push_str(&format!("{} {part}: {line}\n", class.id()));
        }
    }
    out
}

fn golden_path(lang: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/error-conn.{lang}.txt"))
}

fn golden(lang: &str) -> String {
    let path = golden_path(lang);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e} (run with UPDATE_GOLDEN=1)", path.display()))
}

/// Writes the golden file when `UPDATE_GOLDEN` is set.
fn maybe_update(lang: &str, got: &str) {
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(golden_path(lang), got).unwrap();
    }
}

#[test]
fn every_reachable_class_renders_in_en_and_tr() {
    for lang in ["en", "tr"] {
        let got = render_all(
            lang,
            args(
                "orders-prod",
                "db-prod-us.acme.io",
                5432,
                "deploy",
                "orders_v2",
                "PostgreSQL",
                "9.6.24",
            ),
        );
        maybe_update(lang, &got);
        assert_eq!(got.lines().count(), 2 * 26, "a title and a body per class");
        assert_eq!(got, golden(lang));
        assert!(
            !got.contains('{') && !got.contains("error-conn-"),
            "unresolved message in {lang}"
        );
    }
}

#[test]
fn the_mock_texts_are_verbatim() {
    let a = args(
        "orders-prod",
        "db-prod-us.acme.io",
        5432,
        "deploy",
        "orders_v2",
        "PostgreSQL",
        "9.6.24",
    );
    let l = l10n("en");
    let t = |class: ErrClass, part: &str| {
        render(&l, "en", format!("{}-{part}", class.fluent_stem()), &a)
    };
    assert_eq!(
        t(ErrClass::AuthInvalid, "title"),
        "The server rejected the password for deploy."
    );
    assert_eq!(
        t(ErrClass::NetworkDns, "title"),
        "db-prod-us.acme.io couldn't be found."
    );
    assert_eq!(
        t(ErrClass::TlsUntrusted, "title"),
        "db-prod-us.acme.io sent a certificate this Mac doesn't trust."
    );
    assert_eq!(
        t(ErrClass::ConfigUnknownDatabase, "title"),
        "The database orders_v2 doesn't exist on db-prod-us.acme.io."
    );
    assert_eq!(
        t(ErrClass::ServerUnsupportedVersion, "title"),
        "db-prod-us.acme.io runs PostgreSQL 9.6.24, which Hacivat doesn't support."
    );
    assert_eq!(
        t(ErrClass::ServerUnsupportedVersion, "body"),
        "Hacivat supports PostgreSQL 14 and later. Its schema reading and safety checks rely on newer system catalogs."
    );
}

#[test]
fn ports_are_never_grouped() {
    let a = args("x", "db", 54321, "u", "d", "PostgreSQL", "16.4");
    for lang in ["en", "tr"] {
        let l = l10n(lang);
        let got = render(
            &l,
            lang,
            ErrClass::NetworkRefused.fluent_stem() + "-title",
            &a,
        );
        assert!(got.contains("54321"), "{lang}: {got}");
    }
}

#[test]
fn unsupported_version_names_the_floor_of_each_engine() {
    let l = l10n("en");
    let body = |engine: &str| {
        let a = args("x", "db", 3306, "u", "d", engine, "5.7.44");
        render(
            &l,
            "en",
            ErrClass::ServerUnsupportedVersion.fluent_stem() + "-body",
            &a,
        )
    };
    assert!(body("MySQL").starts_with("Hacivat supports MySQL 8.0 and later."));
    assert!(body("MariaDB").starts_with("Hacivat supports MariaDB 10.6 and later."));
    assert!(body("PostgreSQL").starts_with("Hacivat supports PostgreSQL 14 and later."));
}

#[test]
fn sqlite_texts_name_the_file_not_a_host() {
    let a = ErrArgs(vec![
        ("name", text("notes")),
        ("host", text("")),
        ("port", text("")),
        ("user", text("")),
        ("db", text("")),
        ("engine", text("SQLite")),
        ("version", text("")),
        ("file", text("/data/notes.db")),
    ]);
    for lang in ["en", "tr"] {
        let l = l10n(lang);
        for class in [
            ErrClass::ConfigUnknownDatabase,
            ErrClass::ConfigInvalid,
            ErrClass::ServerReadOnly,
        ] {
            let title = render(&l, lang, class.fluent_stem() + "-title", &a);
            assert!(
                title.contains("/data/notes.db"),
                "{lang} {class:?}: {title}"
            );
        }
    }
}
