//! Connection profiles (Task 12): the `connections.toml` store, URL field mapping and the
//! naming and environment rules (HV-SEC-05 profile split, D-004, R22, R23, P7, P29).

use std::path::{Path, PathBuf};

use hacivat_driver::url::{FieldId, ParseOutcome, ParseResult, parse};
use hacivat_net::{ClientCertPaths, TlsMode, TlsSettings};
use hacivat_proto::{DbmsKind, EnvTag, ImportSource, Origin, ProfileId, TagSource};
use hacivat_secrets::{AuthProviderId, ExposeSecret, Retention};
use hacivat_session::profile::{
    EngineChange, HintSource, Profile, ProfileDiag, ProfileDraft, ProfileEndpoint, ProfileStore,
    StoreError, TransportSpec, apply_parse, copy_url, display_url, duplicate_name,
    is_local_endpoint, needs_environment_prompt, production_hint, suggest_name, to_fields,
};

// ---- helpers ---------------------------------------------------------------------------------

/// A fresh temp directory that lives until the process ends.
fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp dir")
}

/// A path named `name` in a fresh temp directory (the file does not exist yet).
fn tmp(name: &str) -> PathBuf {
    tempdir().keep().join(name)
}

/// `connections.toml` with `text`, in a fresh temp directory.
fn write_tmp(text: &str) -> PathBuf {
    let path = tmp("connections.toml");
    std::fs::write(&path, text).expect("write the file");
    path
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("read the file")
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .expect("metadata")
        .permissions()
        .mode()
}

fn server(host: &str) -> ProfileEndpoint {
    ProfileEndpoint::Server {
        host: host.into(),
        port: None,
    }
}

fn file(path: &str) -> ProfileEndpoint {
    ProfileEndpoint::File {
        path: path.into(),
        create_if_missing: false,
    }
}

/// A PostgreSQL profile for `host` and `database`.
fn pg(host: &str, database: Option<&str>) -> Profile {
    Profile {
        endpoint: server(host),
        database: database.map(str::to_string),
        ..Profile::default()
    }
}

fn sqlite(path: &str) -> Profile {
    Profile {
        engine: DbmsKind::Sqlite,
        endpoint: file(path),
        max_connections: 1,
        ..Profile::default()
    }
}

fn sqlite_ro(path: &str) -> Profile {
    Profile {
        read_only: true,
        ..sqlite(path)
    }
}

/// A remote PostgreSQL profile tagged `env`.
fn remote(env: Option<EnvTag>) -> Profile {
    Profile {
        environment: env,
        ..pg("db.acme.io", Some("orders"))
    }
}

/// `orders` on `db-prod-us.acme.io:5433` as `deploy`, verifying the server fully.
fn pg_full() -> Profile {
    Profile {
        name: "orders-prod-us".into(),
        endpoint: ProfileEndpoint::Server {
            host: "db-prod-us.acme.io".into(),
            port: Some(5433),
        },
        database: Some("orders".into()),
        user: Some("deploy".into()),
        tls: TlsSettings {
            mode: TlsMode::VerifyFull,
            ..TlsSettings::default()
        },
        ..Profile::default()
    }
}

/// A profile with every field away from its default.
fn full_profile() -> Profile {
    Profile {
        id: "6f9619ff-8b86-d011-b42d-00c04fc964ff".parse().unwrap(),
        name: "orders-prod-us".into(),
        engine: DbmsKind::MySql,
        environment: Some(EnvTag::Production),
        env_source: TagSource::User,
        read_only: true,
        endpoint: ProfileEndpoint::Server {
            host: "db-prod-us.acme.io".into(),
            port: Some(3307),
        },
        database: Some("İzmir".into()),
        user: Some("deploy".into()),
        auth: AuthProviderId::ClientCert,
        retention: Retention::ForSession,
        tls: TlsSettings {
            mode: TlsMode::VerifyFull,
            ca_file: Some("/etc/ssl/ca.pem".into()),
            client_cert: Some(ClientCertPaths {
                cert: "/etc/ssl/client.pem".into(),
                key: "/etc/ssl/client.key".into(),
            }),
        },
        transport: TransportSpec::Karagoz {
            org: "acme".into(),
            agent_id: "agent-1".into(),
            resource_id: "res-9".into(),
        },
        connect_timeout_s: 30,
        keepalive_s: Some(60),
        application_name: "hv \"quoted\"".into(),
        max_connections: 8,
        extra: vec![
            ("charset".into(), "utf8mb4".into()),
            ("charset".into(), "latin1".into()),
        ],
        origin: Origin::Imported {
            source: ImportSource::PgService,
        },
        allow_unsupported_server: true,
    }
}

/// The parse outcome of `url` (it must be recognised).
fn outcome(url: &str) -> ParseOutcome {
    match parse(url, None) {
        ParseResult::Recognised(o) => o,
        ParseResult::Unrecognised(r) => panic!("not recognised: {r:?}"),
    }
}

// ---- the store -------------------------------------------------------------------------------

#[test]
fn round_trip_preserves_every_field() {
    let (store, _) = ProfileStore::open(tmp("connections.toml"));
    let p = full_profile();
    store.save(std::slice::from_ref(&p)).unwrap();
    assert_eq!(ProfileStore::open(store.path().into()).1.profiles, vec![p]);
}

#[test]
fn round_trip_covers_every_endpoint_transport_and_origin() {
    let socket = Profile {
        endpoint: ProfileEndpoint::Socket {
            path: "/var/run/postgresql".into(),
            port: Some(5433),
        },
        transport: TransportSpec::SshTunnel,
        origin: Origin::PerdeManaged { org: "acme".into() },
        env_source: TagSource::Org,
        environment: Some(EnvTag::Staging),
        ..Profile::default()
    };
    let sqlite_file = Profile {
        endpoint: ProfileEndpoint::File {
            path: "/Users/x/app.db".into(),
            create_if_missing: true,
        },
        origin: Origin::DeepLink,
        ..sqlite("/unused")
    };
    let plain = Profile {
        origin: Origin::Mcp,
        ..pg("h", None)
    };
    let url = Profile {
        origin: Origin::Imported {
            source: ImportSource::Url,
        },
        ..pg("h2", Some("d"))
    };
    let all = vec![socket, sqlite_file, plain, url];
    let (store, _) = ProfileStore::open(tmp("connections.toml"));
    store.save(&all).unwrap();
    let loaded = ProfileStore::open(store.path().into()).1;
    assert_eq!(loaded.profiles, all);
    assert!(loaded.diags.is_empty(), "{:?}", loaded.diags);
}

#[test]
fn password_key_typed_into_file_is_ignored_and_reported() {
    let path = write_tmp(
        "version = 1\n[[connection]]\nid = \"6f9619ff-8b86-d011-b42d-00c04fc964ff\"\nname = \"a\"\nengine = \"postgres\"\nhost = \"h\"\npassword = \"typed-pw\"\n",
    );
    let (store, loaded) = ProfileStore::open(path.clone());
    assert_eq!(loaded.diags, [ProfileDiag::SecretKeyIgnored { index: 0 }]);
    store.save(&loaded.profiles).unwrap();
    assert!(!read(&path).contains("typed-pw"));
}

#[test]
fn every_secret_key_spelling_is_dropped_and_never_echoed() {
    let path = write_tmp(
        "version = 1\n[[connection]]\nid = \"6f9619ff-8b86-d011-b42d-00c04fc964ff\"\nname = \"a\"\n\
         engine = \"postgres\"\nhost = \"h\"\nPWD = \"typed-1\"\npasswd = \"typed-2\"\n\
         tls.mode = \"require\"\ntls.sslpassword = \"typed-3\"\n\
         extra = [{ key = \"Password\", value = \"typed-4\" }, { key = \"k\", value = \"v\" }]\n",
    );
    let (store, loaded) = ProfileStore::open(path.clone());
    assert_eq!(loaded.diags, [ProfileDiag::SecretKeyIgnored { index: 0 }]);
    assert_eq!(loaded.profiles[0].extra, vec![("k".into(), "v".into())]);
    assert!(!format!("{loaded:?}").contains("typed-"));
    store.save(&loaded.profiles).unwrap();
    let text = read(&path);
    assert!(!text.contains("typed-"), "{text}");
    assert!(text.contains("key = \"k\""), "{text}");
}

#[test]
fn unknown_keys_are_preserved_on_save() {
    let path = write_tmp(
        "version = 1\nfuture_table.k = 1\n[[connection]]\nid = \"6f9619ff-8b86-d011-b42d-00c04fc964ff\"\nname = \"a\"\nengine = \"postgres\"\nhost = \"h\"\ncolour = \"x\"\n",
    );
    let (store, loaded) = ProfileStore::open(path.clone());
    store.save(&loaded.profiles).unwrap();
    let text = read(&path);
    assert!(text.contains("colour = \"x\""), "{text}");
    assert!(text.contains("future_table"), "{text}");
}

#[test]
fn comments_and_an_existing_version_are_kept() {
    let path = write_tmp("# hand-written\nversion = 1 # current\n");
    let (store, loaded) = ProfileStore::open(path.clone());
    store.save(&loaded.profiles).unwrap();
    let text = read(&path);
    assert!(
        text.starts_with("# hand-written\nversion = 1 # current\n"),
        "{text}"
    );
}

#[test]
fn saving_drops_deleted_profiles_and_keeps_entries_this_version_cannot_read() {
    let path = write_tmp(
        "version = 1\n\
         [[connection]]\nid = \"6f9619ff-8b86-d011-b42d-00c04fc964ff\"\nname = \"a\"\nengine = \"postgres\"\nhost = \"h\"\n\
         [[connection]]\nid = \"7f9619ff-8b86-d011-b42d-00c04fc964ff\"\nname = \"b\"\nengine = \"postgres\"\nhost = \"h\"\n\
         [[connection]]\nid = \"8f9619ff-8b86-d011-b42d-00c04fc964ff\"\nname = \"c\"\nengine = \"oracle9\"\nhost = \"h\"\n",
    );
    let (store, loaded) = ProfileStore::open(path.clone());
    assert_eq!(loaded.profiles.len(), 2);
    store.save(&loaded.profiles[..1]).unwrap();
    let text = read(&path);
    assert!(text.contains("name = \"a\""), "{text}");
    assert!(!text.contains("name = \"b\""), "{text}");
    assert!(text.contains("engine = \"oracle9\""), "{text}");
}

#[test]
fn newer_version_is_read_only() {
    let path = write_tmp("version = 2\n");
    let (store, loaded) = ProfileStore::open(path.clone());
    assert!(store.read_only());
    assert_eq!(loaded.diags, [ProfileDiag::NewerVersion { found: 2 }]);
    assert_eq!(store.save(&[]), Err(StoreError::ReadOnly));
    assert_eq!(read(&path), "version = 2\n");
}

#[test]
fn unparsable_file_is_quarantined() {
    let path = write_tmp("[[connection");
    let (_, loaded) = ProfileStore::open(path.clone());
    assert!(loaded.profiles.is_empty());
    assert!(
        loaded
            .quarantined
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("connections.toml.corrupt-")
    );
}

#[test]
fn missing_file_loads_nothing_and_is_writable() {
    let (store, loaded) = ProfileStore::open(tmp("connections.toml"));
    assert_eq!(loaded, Default::default());
    assert!(!store.read_only());
    store.save(&[]).unwrap();
    assert!(read(store.path()).contains("version = 1"));
}

#[test]
fn duplicate_ids_keep_the_first() {
    let id = "6f9619ff-8b86-d011-b42d-00c04fc964ff";
    let entry = |name: &str| {
        format!(
            "[[connection]]\nid = \"{id}\"\nname = \"{name}\"\nengine = \"postgres\"\nhost = \"h\"\n"
        )
    };
    let (_, loaded) = ProfileStore::open(write_tmp(&format!(
        "version = 1\n{}{}",
        entry("a"),
        entry("b")
    )));
    assert_eq!(loaded.profiles.len(), 1);
    assert_eq!(loaded.profiles[0].name, "a");
    assert_eq!(
        loaded.diags,
        [ProfileDiag::DuplicateId {
            id: id.parse().unwrap()
        }]
    );
}

#[test]
fn unknown_engine_entry_skipped() {
    let (_, loaded) = ProfileStore::open(write_tmp(
        "version = 1\n[[connection]]\nid = \"6f9619ff-8b86-d011-b42d-00c04fc964ff\"\nname = \"a\"\nengine = \"oracle9\"\nhost = \"h\"\n",
    ));
    assert!(loaded.profiles.is_empty());
    assert_eq!(
        loaded.diags,
        [ProfileDiag::UnknownEngine {
            index: 0,
            engine: "oracle9".into()
        }]
    );
}

#[test]
fn invalid_entries_are_skipped_with_the_offending_key() {
    let (_, loaded) = ProfileStore::open(write_tmp(
        "version = 1\n\
         [[connection]]\nname = \"no-id\"\nengine = \"postgres\"\n\
         [[connection]]\nid = \"not-a-uuid\"\nname = \"a\"\nengine = \"postgres\"\n\
         [[connection]]\nid = \"6f9619ff-8b86-d011-b42d-00c04fc964ff\"\nname = \"b\"\nengine = \"postgres\"\nport = 70000\n\
         [[connection]]\nid = \"7f9619ff-8b86-d011-b42d-00c04fc964ff\"\nname = \"ok\"\nengine = \"postgres\"\nhost = \"h\"\n",
    ));
    assert_eq!(loaded.profiles.len(), 1);
    assert_eq!(loaded.profiles[0].name, "ok");
    let reasons: Vec<(usize, &str)> = loaded
        .diags
        .iter()
        .map(|d| match d {
            ProfileDiag::InvalidEntry { index, reason } => (*index, reason.as_str()),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(reasons, [(0, "id"), (1, "id"), (2, "port")]);
}

#[cfg(unix)]
#[test]
fn file_is_0600_and_dir_0700() {
    let dir = tempdir();
    let path = dir.path().join("fresh").join("connections.toml");
    let (store, _) = ProfileStore::open(path.clone());
    store.save(&[full_profile()]).unwrap();
    assert_eq!(mode(&path) & 0o777, 0o600);
    assert_eq!(mode(path.parent().unwrap()) & 0o777, 0o700);
}

// ---- rules -----------------------------------------------------------------------------------

#[test]
fn local_endpoints() {
    for h in [
        "localhost",
        "LOCALHOST",
        "localhost.",
        "127.0.0.1",
        "127.5.5.5",
        "::1",
    ] {
        assert!(is_local_endpoint(&server(h)), "{h}");
    }
    for h in ["10.0.0.5", "192.168.1.10", "db.acme.io"] {
        assert!(!is_local_endpoint(&server(h)), "{h}");
    }
    assert!(is_local_endpoint(&ProfileEndpoint::Socket {
        path: "/tmp".into(),
        port: None
    }));
    assert!(is_local_endpoint(&file("/a.db")));
}

#[test]
fn prod_suggestion_ascii_only() {
    use HintSource::*; // whole tokens split on [-._/:0-9] (ledger ruling, P7)
    assert_eq!(
        production_hint("db-prod-us.acme.io", ""),
        Some(("prod", Host))
    );
    assert_eq!(production_hint("", "orders-PRD"), Some(("prd", Name)));
    assert_eq!(production_hint("live-db", ""), Some(("live", Host)));
    assert_eq!(production_hint("delivery-db", ""), None);
    assert_eq!(
        production_hint("pg.production.acme.io", ""),
        Some(("prod", Host))
    );
    assert_eq!(production_hint("prod1.acme.io", ""), Some(("prod", Host)));
    assert_eq!(production_hint("pröd", ""), None);
    assert_eq!(production_hint("PRİD-db", ""), None);
    assert_eq!(production_hint("staging", "orders"), None);
    assert_eq!(
        production_hint("db-prod.acme.io", "orders-live"),
        Some(("prod", Host))
    );
}

#[test]
fn prod_suggestion_never_matches_substrings() {
    for text in [
        "delivery",
        "product",
        "proddb",
        "olive",
        "liver",
        "reproduction",
    ] {
        assert_eq!(production_hint(text, text), None, "{text}");
    }
    assert_eq!(
        production_hint("10.0.0.5:5432", "a/live_b"),
        Some(("live", HintSource::Name))
    );
}

#[test]
fn turkish_names_are_ascii_folded_only() {
    // Review Focus 3 (P39)
    assert!(is_local_endpoint(&server("LOCALHOST")));
    assert!(!is_local_endpoint(&server("LOCALHOST\u{130}")));
    assert_eq!(
        suggest_name(&pg("İstanbul-db.acme.io", Some("siparis"))),
        "siparis-İstanbul"
    );
}

#[test]
fn suggest_name_matches_the_mock() {
    assert_eq!(
        suggest_name(&pg("db-prod-us.acme.io", Some("orders"))),
        "orders-prod-us"
    );
    assert_eq!(suggest_name(&pg("localhost", Some("shop"))), "shop-local");
    assert_eq!(
        suggest_name(&pg("pg-orders.acme.io", Some("orders"))),
        "orders"
    );
    assert_eq!(suggest_name(&sqlite("/Users/x/app.db")), "app.db");
    assert_eq!(suggest_name(&pg("", None)), "");
}

#[test]
fn suggest_name_edges() {
    assert_eq!(suggest_name(&pg("localhost", None)), "local-local");
    assert_eq!(suggest_name(&pg("db.acme.io", None)), "db");
    assert_eq!(
        suggest_name(&pg("ORDERS-db.acme.io", Some("orders_x"))),
        "orders_x"
    );
    assert_eq!(
        suggest_name(&sqlite("C:\\data\\shop.sqlite")),
        "shop.sqlite"
    );
}

#[test]
fn duplicate_names() {
    assert_eq!(
        duplicate_name("orders-prod", &["orders-prod"]),
        "orders-prod-2"
    );
    assert_eq!(
        duplicate_name("orders-prod", &["orders-prod", "orders-prod-2"]),
        "orders-prod-3"
    );
}

#[test]
fn environment_prompt_rule() {
    assert!(needs_environment_prompt(&remote(None)));
    assert!(!needs_environment_prompt(&remote(Some(EnvTag::Dev))));
    assert!(!needs_environment_prompt(&pg("localhost", None)));
}

// ---- field mapping ---------------------------------------------------------------------------

#[test]
fn apply_parse_fills_and_moves_password() {
    let mut d = ProfileDraft::new();
    let r = apply_parse(
        &mut d,
        outcome(
            "postgres://deploy:s3cret@db-prod-us.acme.io:5433/orders?sslmode=verify-full&application_name=hv&foo=bar",
        ),
        false,
    );
    assert_eq!(
        d.profile.endpoint,
        ProfileEndpoint::Server {
            host: "db-prod-us.acme.io".into(),
            port: Some(5433)
        }
    );
    assert_eq!(
        (d.profile.database.as_deref(), d.profile.user.as_deref()),
        (Some("orders"), Some("deploy"))
    );
    // Never `assert_eq!` on a secret: a failure would print it.
    assert!(
        d.password.as_ref().unwrap().expose_secret() == "s3cret",
        "password mismatch (values not shown)"
    );
    assert_eq!(d.profile.tls.mode, TlsMode::VerifyFull);
    assert_eq!(d.profile.application_name, "hv");
    assert_eq!(d.profile.extra, vec![("foo".into(), "bar".into())]);
    assert!(r.password_moved);
    assert_eq!(r.extras_kept, ["foo"]);
    assert_eq!(
        r.filled,
        [
            FieldId::Host,
            FieldId::Port,
            FieldId::Database,
            FieldId::User,
            FieldId::Password
        ]
    );
    assert!(!format!("{r:?}").contains("s3cret"));
}

#[test]
fn apply_parse_engine_rules() {
    let mut untouched = ProfileDraft::new();
    assert_eq!(
        apply_parse(&mut untouched, outcome("mysql://h/db"), false).engine,
        Some(EngineChange::Switched {
            from: DbmsKind::Postgres,
            to: DbmsKind::MySql
        })
    );
    let mut typed = ProfileDraft::new();
    typed.touched.insert(FieldId::Host);
    let r = apply_parse(&mut typed, outcome("mysql://h/db"), false);
    assert_eq!(
        r.engine,
        Some(EngineChange::Mismatch {
            url: DbmsKind::MySql,
            form: DbmsKind::Postgres
        })
    );
    assert!(r.filled.is_empty());
    assert_eq!(typed.profile.engine, DbmsKind::Postgres);
}

#[test]
fn apply_parse_keeps_touched_fields_and_forces_on_request() {
    let mut d = ProfileDraft::new();
    d.profile.endpoint = server("typed.example");
    d.touched.insert(FieldId::Host);
    let r = apply_parse(&mut d, outcome("postgres://u@h:5555/db"), false);
    assert_eq!(
        d.profile.endpoint,
        ProfileEndpoint::Server {
            host: "typed.example".into(),
            port: Some(5555)
        }
    );
    assert_eq!(r.filled, [FieldId::Port, FieldId::Database, FieldId::User]);
    assert_eq!(r.engine, None);

    let r = apply_parse(&mut d, outcome("mysql://h/db"), true);
    assert_eq!(
        r.engine,
        Some(EngineChange::Switched {
            from: DbmsKind::Postgres,
            to: DbmsKind::MySql
        })
    );
    assert_eq!(d.profile.engine, DbmsKind::MySql);
}

#[test]
fn apply_parse_sqlite_and_the_key_passphrase() {
    let mut d = ProfileDraft::new();
    let r = apply_parse(&mut d, outcome("file:/Users/x/app.db?mode=ro"), false);
    assert_eq!(d.profile.engine, DbmsKind::Sqlite);
    assert_eq!(d.profile.endpoint, file("/Users/x/app.db"));
    assert!(d.profile.read_only);
    assert_eq!(d.profile.max_connections, 1);
    assert_eq!(r.filled, [FieldId::File]);

    let mut d = ProfileDraft::new();
    apply_parse(
        &mut d,
        outcome("postgres://u@h/db?sslmode=verify-full&sslpassword=k3y-pass"),
        false,
    );
    assert!(d.password.is_none());
    assert!(
        d.tls_key_passphrase.as_ref().unwrap().expose_secret() == "k3y-pass",
        "passphrase mismatch (values not shown)"
    );
    assert!(d.profile.extra.is_empty());
    assert!(!format!("{d:?}").contains("k3y-pass"));
}

#[test]
fn copy_url_omits_password() {
    let c = copy_url(&pg_full()).unwrap();
    assert_eq!(
        c.url,
        "postgresql://deploy@db-prod-us.acme.io:5433/orders?sslmode=verify-full"
    );
    assert!(c.omitted_password);
    assert_eq!(
        copy_url(&sqlite_ro("/Users/x/app.db")).unwrap().url,
        "file:/Users/x/app.db?mode=ro"
    );
}

#[test]
fn display_url_masks_and_copy_round_trips() {
    assert_eq!(
        display_url(&pg_full(), true).as_deref(),
        Some("postgresql://deploy:****@db-prod-us.acme.io:5433/orders?sslmode=verify-full")
    );
    let mut p = pg_full();
    p.engine = DbmsKind::MySql;
    p.application_name = "hv".into();
    p.extra = vec![("charset".into(), "utf8mb4".into())];
    let c = copy_url(&p).unwrap();
    assert_eq!(
        c.url,
        "mysql://deploy@db-prod-us.acme.io:5433/orders?ssl-mode=VERIFY_IDENTITY&application_name=hv&charset=utf8mb4"
    );
    let mut d = ProfileDraft::new();
    apply_parse(&mut d, outcome(&c.url), false);
    assert_eq!(
        (
            d.profile.engine,
            d.profile.tls.mode,
            d.profile.extra.clone()
        ),
        (DbmsKind::MySql, TlsMode::VerifyFull, p.extra.clone())
    );
    assert_eq!(d.profile.application_name, "hv");
    let (conn_type, fields, extra) = to_fields(&p);
    assert_eq!(conn_type, "default");
    assert_eq!(
        fields.values.get(&FieldId::Host).map(String::as_str),
        Some("db-prod-us.acme.io")
    );
    assert_eq!(extra.last(), Some(&("charset".into(), "utf8mb4".into())));
    assert!(display_url(&pg("", None), false).is_none());
}

#[test]
fn ids_survive_the_store_as_strings() {
    let id = ProfileId::new();
    let (store, _) = ProfileStore::open(tmp("connections.toml"));
    store
        .save(&[Profile {
            id,
            ..pg("h", None)
        }])
        .unwrap();
    assert!(read(store.path()).contains(&format!("id = \"{id}\"")));
}
