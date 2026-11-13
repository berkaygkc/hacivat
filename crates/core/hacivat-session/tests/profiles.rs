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
        // ## m2a-finalfix: the connect path's rule (RFC 6761), M3.
        "db.localhost",
        "API.LOCALHOST.",
        "[::1]",
        "::ffff:127.0.0.1",
    ] {
        assert!(is_local_endpoint(&server(h)), "{h}");
        assert_eq!(
            hacivat_net::classify_host(h, &[]),
            hacivat_net::AddressClass::Loopback,
            "{h}"
        );
    }
    for h in [
        "10.0.0.5",
        "192.168.1.10",
        "db.acme.io",
        "localhost.acme.io",
    ] {
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

// ## m2a-finalfix
/// I2: a pasted URL's TLS file options go to the typed settings, like its mapped ones, and are
/// reported as such; no extra is left to override them.
#[test]
fn apply_parse_folds_tls_file_options_into_the_typed_settings() {
    let mut d = ProfileDraft::new();
    let r = apply_parse(
        &mut d,
        outcome(
            "postgres://u@h/db?sslmode=allow&sslrootcert=/certs/ca.pem&sslcert=/certs/me.pem&sslkey=/certs/me.key&foo=bar",
        ),
        false,
    );
    assert_eq!(d.profile.tls.mode, TlsMode::Auto);
    assert_eq!(d.profile.tls.ca_file, Some(PathBuf::from("/certs/ca.pem")));
    assert_eq!(
        d.profile.tls.client_cert,
        Some(ClientCertPaths {
            cert: "/certs/me.pem".into(),
            key: "/certs/me.key".into()
        })
    );
    assert_eq!(d.profile.extra, vec![("foo".into(), "bar".into())]);
    assert_eq!(r.extras_kept, ["foo"]);
    let folded: Vec<&str> = r.folded.iter().map(|f| f.key.as_str()).collect();
    assert_eq!(folded, ["sslmode", "sslrootcert", "sslcert", "sslkey"]);
}

/// I2: an older file whose extras still name a TLS mode or a timeout loads them as the typed
/// settings (the form, the fixes and the connect then agree), and the next save writes them
/// typed.
#[test]
fn typed_options_left_in_an_older_file_load_as_typed_settings() {
    let path = write_tmp(
        "version = 1\n[[connection]]\nid = \"6f9619ff-8b86-d011-b42d-00c04fc964ff\"\nname = \"a\"\n\
         engine = \"postgres\"\nhost = \"db.acme.io\"\ntls.mode = \"auto\"\n\
         extra = [{ key = \"sslmode\", value = \"disable\" }, { key = \"connect_timeout\", value = \"9\" }, { key = \"k\", value = \"v\" }]\n",
    );
    let (store, loaded) = ProfileStore::open(path.clone());
    let p = &loaded.profiles[0];
    assert_eq!((p.tls.mode, p.connect_timeout_s), (TlsMode::Disable, 9));
    assert_eq!(p.extra, vec![("k".into(), "v".into())]);
    store.save(&loaded.profiles).unwrap();
    let text = read(&path);
    assert!(text.contains("tls.mode = \"disable\""), "{text}");
    assert!(!text.contains("sslmode"), "{text}");
}

/// I1: the store drops every key the shared rule calls a password, not only the five
/// spellings it used to know.
#[test]
fn the_store_drops_every_password_like_key() {
    let path = write_tmp(
        "version = 1\n[[connection]]\nid = \"6f9619ff-8b86-d011-b42d-00c04fc964ff\"\nname = \"a\"\n\
         engine = \"postgres\"\nhost = \"h\"\n\
         extra = [{ key = \"pgpassword\", value = \"typed-1\" }, { key = \"db_password\", value = \"typed-2\" }, { key = \"mysql_pwd\", value = \"typed-3\" }, { key = \"k\", value = \"v\" }]\n",
    );
    let (store, loaded) = ProfileStore::open(path.clone());
    assert_eq!(loaded.diags, [ProfileDiag::SecretKeyIgnored { index: 0 }]);
    assert_eq!(loaded.profiles[0].extra, vec![("k".into(), "v".into())]);
    let mut p = loaded.profiles[0].clone();
    p.extra.push(("x-password".into(), "typed-4".into()));
    store.save(&[p]).unwrap();
    let text = read(&path);
    assert!(!text.contains("typed-"), "{text}");
}

// ## m2a-finalfix (round 2, N1, N6)

/// Whether the store keeps an `extra` entry keyed `key` through a load and a save.
fn store_keeps(key: &str) -> bool {
    let quoted = key.replace('\\', "\\\\").replace('"', "\\\"");
    let path = write_tmp(&format!(
        "version = 1\n[[connection]]\nid = \"6f9619ff-8b86-d011-b42d-00c04fc964ff\"\nname = \"a\"\n\
         engine = \"postgres\"\nhost = \"h\"\n\
         extra = [{{ key = \"{quoted}\", value = \"typed-v\" }}]\n"
    ));
    let (store, loaded) = ProfileStore::open(path.clone());
    let kept_on_load = !loaded.profiles[0].extra.is_empty();
    let mut p = loaded.profiles[0].clone();
    p.extra = vec![(key.to_string(), "typed-v".to_string())];
    store.save(&[p]).unwrap();
    let kept_on_save = read(&path).contains("typed-v");
    assert_eq!(
        kept_on_load, kept_on_save,
        "{key:?}: load and save disagree"
    );
    kept_on_load
}

/// Whether the scrubber removes `key`'s value from an option list.
fn scrubber_removes(key: &str) -> bool {
    hacivat_secrets::scrub_options(&[(key.to_string(), "typed-v".to_string())])[0].1 != "typed-v"
}

/// N1: the store, the scrubber, the import and render (both through
/// `hacivat_driver::url::is_secret_param`, the same function) give one answer for every key:
/// the re-review's camelCase JDBC and passphrase keys, the earlier spellings, a key with
/// surrounding whitespace (N6), and plain options that are no secret.
#[test]
fn the_store_and_the_scrubber_agree_on_every_password_key() {
    let secret = [
        "password",
        "PASSWORD",
        " password",
        "password ",
        "\tpgpassword",
        "--password",
        "pass",
        "pwd",
        "passwd",
        "sslpassword",
        "sslkeypassword",
        "pgpassword",
        "mysql_pwd",
        "db_password",
        "x-pwd",
        "x-password",
        "password_db",
        "trustCertificateKeyStorePassword",
        "clientCertificateKeyStorePassword",
        "keyStorePassword",
        "trustStorePassword",
        "keyPassword",
        "password1",
        "password2",
        "password3",
        "passphrase",
        "ssl_passphrase",
        "tls_passphrase",
        "key_passphrase",
    ];
    let plain = [
        "sslmode",
        "user",
        "host",
        "dbname",
        "client_encoding",
        "target_session_attrs",
        "options",
        "characterEncoding",
    ];
    for key in secret {
        assert!(hacivat_driver::url::is_secret_param(key), "{key:?}");
        assert!(!store_keeps(key), "the store keeps {key:?}");
        assert!(scrubber_removes(key), "the scrubber keeps {key:?}");
    }
    for key in plain {
        assert!(!hacivat_driver::url::is_secret_param(key), "{key:?}");
        assert!(store_keeps(key), "the store drops {key:?}");
        assert!(!scrubber_removes(key), "the scrubber removes {key:?}");
    }
    // The scrubber is a documented superset: it also hides tokens and key material that are
    // not passwords (and so stay in a profile, like the `sslkey` path).
    for key in ["sslkey", "access_token", "client_secret"] {
        assert!(store_keeps(key) && scrubber_removes(key), "{key:?}");
    }
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

// ---- fix round 1 -----------------------------------------------------------------------------

const ID_A: &str = "6f9619ff-8b86-d011-b42d-00c04fc964ff";
const ID_B: &str = "7f9619ff-8b86-d011-b42d-00c04fc964ff";

fn entry(id: &str, name: &str, more: &str) -> String {
    format!(
        "[[connection]]\nid = \"{id}\"\nname = \"{name}\"\nengine = \"postgres\"\nhost = \"h\"\n{more}"
    )
}

#[test]
fn an_org_tag_counts_only_on_a_profile_the_org_manages() {
    let text = format!(
        "version = 1\n{}{}",
        entry(
            ID_A,
            "mine",
            "environment = \"dev\"\nenv_source = \"org\"\n"
        ),
        entry(
            ID_B,
            "managed",
            "environment = \"production\"\nenv_source = \"org\"\norigin = { kind = \"perde_managed\", org = \"acme\" }\n"
        ),
    );
    let (_, loaded) = ProfileStore::open(write_tmp(&text));
    assert_eq!(
        loaded.diags,
        [ProfileDiag::ValueIgnored {
            index: 0,
            key: "env_source".into()
        }]
    );
    let sources: Vec<(TagSource, Option<EnvTag>)> = loaded
        .profiles
        .iter()
        .map(|p| (p.env_source, p.environment))
        .collect();
    assert_eq!(
        sources,
        [
            (TagSource::User, Some(EnvTag::Dev)),
            (TagSource::Org, Some(EnvTag::Production))
        ]
    );
}

#[test]
fn an_unknown_environment_is_no_tag_reported_and_kept_on_disk() {
    let path = write_tmp(&format!(
        "version = 1\n{}",
        entry(ID_A, "a", "environment = \"uat\"\n")
    ));
    let (store, loaded) = ProfileStore::open(path.clone());
    assert_eq!(loaded.profiles.len(), 1);
    assert_eq!(loaded.profiles[0].environment, None);
    assert_eq!(
        loaded.diags,
        [ProfileDiag::ValueIgnored {
            index: 0,
            key: "environment".into()
        }]
    );
    store.save(&loaded.profiles).unwrap();
    assert!(read(&path).contains("environment = \"uat\""));
    let tagged = Profile {
        environment: Some(EnvTag::Dev),
        ..loaded.profiles[0].clone()
    };
    store.save(&[tagged]).unwrap();
    let text = read(&path);
    assert!(
        text.contains("environment = \"dev\"") && !text.contains("uat"),
        "{text}"
    );
}

#[test]
fn saving_keeps_a_duplicate_id_entry_and_entries_added_by_hand() {
    let path = write_tmp(&format!(
        "version = 1\n{}{}",
        entry(ID_A, "first", ""),
        entry(ID_A, "second", "")
    ));
    let (store, loaded) = ProfileStore::open(path.clone());
    assert_eq!(loaded.profiles.len(), 1);
    // An entry added by hand while the app runs.
    let mut text = read(&path);
    text.push_str(&entry(ID_B, "by-hand", ""));
    std::fs::write(&path, text).unwrap();

    store.save(&loaded.profiles).unwrap();
    let text = read(&path);
    for name in ["first", "second", "by-hand"] {
        assert!(
            text.contains(&format!("name = \"{name}\"")),
            "{name}: {text}"
        );
    }
    // A deleted profile's entries go, duplicates included; the hand-added one stays.
    store.save(&[]).unwrap();
    let text = read(&path);
    assert!(
        !text.contains("first") && !text.contains("second"),
        "{text}"
    );
    assert!(text.contains("name = \"by-hand\""), "{text}");
    let (_, reloaded) = ProfileStore::open(path);
    assert_eq!(reloaded.profiles.len(), 1);
    assert_eq!(reloaded.profiles[0].name, "by-hand");
}

#[test]
fn password_keys_outside_the_entries_are_stripped_and_reported() {
    let path = write_tmp(&format!(
        "version = 1\npassword = \"typed-1\"\n[future]\nPwd = \"typed-2\"\nkeep = 1\n{}",
        entry(ID_A, "a", "")
    ));
    let (store, loaded) = ProfileStore::open(path.clone());
    assert_eq!(loaded.diags, [ProfileDiag::SecretKeyOutsideEntries]);
    store.save(&loaded.profiles).unwrap();
    let text = read(&path);
    assert!(!text.contains("typed-"), "{text}");
    assert!(text.contains("keep = 1"), "{text}");
}

#[test]
fn a_file_broken_after_open_is_quarantined_by_save_and_reported() {
    let path = write_tmp("version = 1\n");
    let (store, _) = ProfileStore::open(path.clone());
    std::fs::write(&path, "[[connection").unwrap();
    let report = store.save_reporting(&[pg("h", None)]).unwrap();
    let aside = report.quarantined.expect("set aside");
    assert_eq!(read(&aside), "[[connection");
    assert!(read(&path).contains("[[connection]]"));
}

#[cfg(unix)]
mod unix_io {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn set_mode(path: &Path, mode: u32) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    /// Whether permission bits stop this process (they do not for root).
    fn permissions_apply(path: &Path) -> bool {
        std::fs::read(path).is_err()
    }

    #[test]
    fn an_unreadable_file_is_left_untouched_and_the_store_read_only() {
        let text = format!("version = 1\n{}", entry(ID_A, "a", ""));
        let path = write_tmp(&text);
        set_mode(&path, 0o000);
        if !permissions_apply(&path) {
            set_mode(&path, 0o600);
            return;
        }
        let (store, loaded) = ProfileStore::open(path.clone());
        set_mode(&path, 0o600);
        assert!(
            matches!(&loaded.diags[..], [ProfileDiag::Unreadable { reason }] if reason == "PermissionDenied"),
            "{:?}",
            loaded.diags
        );
        assert!(loaded.quarantined.is_none());
        assert!(store.read_only());
        assert_eq!(store.save(&[]), Err(StoreError::ReadOnly));
        assert_eq!(read(&path), text);
    }

    #[test]
    fn a_save_that_cannot_read_the_file_writes_nothing() {
        let text = format!("version = 1\n{}", entry(ID_A, "a", ""));
        let path = write_tmp(&text);
        let (store, loaded) = ProfileStore::open(path.clone());
        assert_eq!(loaded.profiles.len(), 1);
        set_mode(&path, 0o000);
        if !permissions_apply(&path) {
            set_mode(&path, 0o600);
            return;
        }
        let result = store.save(&[]);
        set_mode(&path, 0o600);
        assert_eq!(result, Err(StoreError::Io("PermissionDenied".into())));
        assert_eq!(read(&path), text);
    }

    #[test]
    fn a_failed_quarantine_is_reported_and_the_store_read_only() {
        let path = write_tmp("[[connection");
        let dir = path.parent().unwrap().to_path_buf();
        set_mode(&dir, 0o500);
        let probe = dir.join("probe");
        if std::fs::write(&probe, "x").is_ok() {
            // Root: permission bits do not apply.
            std::fs::remove_file(probe).unwrap();
            set_mode(&dir, 0o700);
            return;
        }
        let (store, loaded) = ProfileStore::open(path.clone());
        set_mode(&dir, 0o700);
        assert!(
            matches!(&loaded.diags[..], [ProfileDiag::QuarantineFailed { .. }]),
            "{:?}",
            loaded.diags
        );
        assert!(loaded.quarantined.is_none());
        assert!(store.read_only());
        assert_eq!(read(&path), "[[connection");
    }
}
