//! Connection profiles in M2b (Task 12; B30): the `[connection.ssh]` table, TLS pins, the
//! plaintext consent and the session defaults in `connections.toml`. A key at its default is
//! never written, so an M2a file round-trips byte for byte.

use std::path::{Path, PathBuf};

use hacivat_driver::EngineCatalog;
use hacivat_net::ssh::{HopSpec, HostKeyPolicy, SshAuth, SshSpec};
use hacivat_net::{CertPin, TlsMode};
use hacivat_proto::DbmsKind;
use hacivat_secrets::Retention;
use hacivat_session::pipeline::{IssueCode, validate};
use hacivat_session::profile::{
    LoadedProfiles, Profile, ProfileDiag, ProfileEndpoint, ProfileStore, TransportSpec,
};
use hacivat_session::session::{IsolationLevel, TxMode};

// ---- helpers ---------------------------------------------------------------------------------

const ID: &str = "6f9619ff-8b86-d011-b42d-00c04fc964ff";

fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp dir")
}

/// A store on a missing `connections.toml` in a fresh directory (kept alive by the guard).
fn store_tmp() -> (ProfileStore, tempfile::TempDir) {
    let dir = tempdir();
    let (store, _) = ProfileStore::open(dir.path().join("connections.toml"));
    (store, dir)
}

fn reload(s: &ProfileStore) -> Vec<Profile> {
    ProfileStore::open(s.path().to_path_buf()).1.profiles
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("read the file")
}

/// What saving `p` alone into a new file writes.
fn save_to_string(p: &Profile) -> String {
    let (s, _dir) = store_tmp();
    s.save(std::slice::from_ref(p)).expect("save");
    read(s.path())
}

/// A loaded file, with its store and the directory that holds it.
struct Report {
    store: ProfileStore,
    loaded: LoadedProfiles,
    _dir: tempfile::TempDir,
}

impl Report {
    fn diags(&self) -> &[ProfileDiag] {
        &self.loaded.diags
    }
}

fn load_report(text: &str) -> Report {
    let dir = tempdir();
    let path = dir.path().join("connections.toml");
    std::fs::write(&path, text).expect("write the file");
    let (store, loaded) = ProfileStore::open(path);
    Report {
        store,
        loaded,
        _dir: dir,
    }
}

fn load_from_string(text: &str) -> Vec<Profile> {
    load_report(text).loaded.profiles
}

/// Saves what `r` loaded back into its file and returns the file.
fn resave_report(r: &Report) -> String {
    r.store.save(&r.loaded.profiles).expect("save");
    read(r.store.path())
}

/// Loads `text` and saves what loaded back.
fn resave(text: &str) -> String {
    resave_report(&load_report(text))
}

/// An entry of `connections.toml` with `more` lines after the server keys.
fn entry(more: &str) -> String {
    format!(
        "version = 1\n\n[[connection]]\nid = \"{ID}\"\nname = \"orders\"\nengine = \"postgres\"\n\
         host = \"db.internal\"\n{more}"
    )
}

fn hop(host: &str, user: &str, auth: SshAuth) -> HopSpec {
    HopSpec {
        host: host.into(),
        port: 22,
        user: user.into(),
        auth,
        identities_only: false,
        identity_files: Vec::new(),
    }
}

fn key_file(path: &str) -> SshAuth {
    SshAuth::KeyFile { path: path.into() }
}

fn ssh_profile(hops: Vec<HopSpec>) -> Profile {
    Profile {
        id: ID.parse().unwrap(),
        name: "orders".into(),
        endpoint: ProfileEndpoint::Server {
            host: "db.internal".into(),
            port: Some(5432),
        },
        user: Some("deploy".into()),
        transport: TransportSpec::SshTunnel(SshSpec {
            hops,
            ..Default::default()
        }),
        ..Profile::default()
    }
}

fn pin(endpoint: &str, sha256: [u8; 32]) -> CertPin {
    CertPin {
        endpoint: endpoint.into(),
        sha256,
    }
}

fn with_pins(pins: &[CertPin]) -> Profile {
    let mut p = ssh_profile(Vec::new());
    p.transport = TransportSpec::Direct;
    p.tls.mode = TlsMode::Require;
    p.tls.pins = pins.to_vec();
    p
}

fn codes(p: &Profile) -> Vec<IssueCode> {
    validate(p, EngineCatalog::embedded())
        .into_iter()
        .map(|i| i.code)
        .collect()
}

// ---- SSH -------------------------------------------------------------------------------------

#[test]
fn ssh_profile_round_trips() {
    let p = ssh_profile(vec![
        hop("j.acme.io", "u1", SshAuth::Agent),
        hop("b.acme.io", "deploy", key_file("/home/me/.ssh/id_ed25519")),
    ]);
    let (s, _dir) = store_tmp();
    s.save(std::slice::from_ref(&p)).unwrap();
    assert_eq!(reload(&s)[0], p);

    // Every key of the table, with values other than the defaults.
    let mut every = ssh_profile(vec![
        HopSpec {
            port: 2222,
            identities_only: true,
            identity_files: vec![
                PathBuf::from("/home/me/.ssh/id_rsa"),
                PathBuf::from("/home/me/.ssh/id_ecdsa"),
            ],
            ..hop("j.acme.io", "u1", SshAuth::Password)
        },
        hop("k.acme.io", "u2", SshAuth::KeyboardInteractive),
        hop("b.acme.io", "deploy", key_file("/home/me/.ssh/id_ed25519")),
    ]);
    if let TransportSpec::SshTunnel(spec) = &mut every.transport {
        spec.host_keys = HostKeyPolicy::Strict;
        spec.keepalive_s = Some(30);
    }
    every.ssh_retention = Retention::UntilRestart;
    every.tls.pins = vec![pin("db:5432", [7; 32])];
    let text = save_to_string(&every);
    for line in [
        "transport = \"ssh_tunnel\"",
        "[connection.ssh]",
        "host_keys = \"strict\"",
        "keepalive_s = 30",
        "[[connection.ssh.hops]]",
        "host = \"j.acme.io\"",
        "port = 2222",
        "auth = \"password\"",
        "auth = \"keyboard_interactive\"",
        "auth = \"key_file\"",
        "key_file = \"/home/me/.ssh/id_ed25519\"",
        "identities_only = true",
        "ssh_retention = \"until_restart\"",
    ] {
        assert!(text.contains(line), "{line} in:\n{text}");
    }
    // Defaults are left out: port 22, `agent` is written (it names the method), no key file
    // for a hop that does not use one.
    assert_eq!(text.matches("port = 22\n").count(), 0, "{text}");
    assert_eq!(text.matches("key_file = ").count(), 1, "{text}");
    assert_eq!(load_from_string(&text), vec![every]);
}

/// Each entry's `[connection.ssh]` table stays with its entry, in a new file and when an entry
/// of an existing file becomes an SSH profile.
#[test]
fn ssh_tables_stay_with_their_entries() {
    let second_id = "7f9619ff-8b86-d011-b42d-00c04fc964ff";
    let first = ssh_profile(vec![hop("j1", "u1", SshAuth::Agent)]);
    let second = Profile {
        id: second_id.parse().unwrap(),
        name: "second".into(),
        ..ssh_profile(vec![
            hop("j2", "u2", SshAuth::Password),
            hop("b2", "u3", SshAuth::Agent),
        ])
    };
    let plain = Profile {
        id: "8f9619ff-8b86-d011-b42d-00c04fc964ff".parse().unwrap(),
        name: "plain".into(),
        transport: TransportSpec::Direct,
        ..first.clone()
    };
    let (s, _dir) = store_tmp();
    s.save(&[first.clone(), plain.clone(), second.clone()])
        .unwrap();
    assert_eq!(
        reload(&s),
        vec![first.clone(), plain.clone(), second.clone()]
    );

    // An existing file whose first entry turns into an SSH profile.
    let r = load_report(&format!(
        "{}\n[[connection]]\nid = \"{second_id}\"\nname = \"second\"\nengine = \"postgres\"\n\
         host = \"h2\"\n",
        entry("")
    ));
    let mut loaded = r.loaded.profiles.clone();
    assert_eq!(loaded.len(), 2);
    loaded[0].transport = first.transport.clone();
    r.store.save(&loaded).unwrap();
    let back = reload(&r.store);
    assert_eq!(back, loaded, "{}", read(r.store.path()));
    assert_eq!(back[1].transport, TransportSpec::Direct);
}

#[test]
fn unknown_ssh_keys_preserved_and_typed_password_ignored() {
    for secret in [
        "password",
        "passphrase",
        "secret",
        "Key_Passphrase",
        "client_secret",
        "token",
        "otp",
        "kbd_answer",
    ] {
        let r = load_report(&entry(&format!(
            "transport = \"ssh_tunnel\"\n\n[connection.ssh]\nfuture = 1\n{secret} = \"hv_test_pw\"\n\n\
             [[connection.ssh.hops]]\nhost = \"b.acme.io\"\nuser = \"deploy\"\nauth = \"agent\"\n\
             hop_future = \"kept\"\n{secret} = \"hv_test_pw\"\n"
        )));
        assert!(
            r.diags()
                .iter()
                .any(|d| matches!(d, ProfileDiag::SecretKeyIgnored { index: 0 })),
            "{secret}: {:?}",
            r.diags()
        );
        assert_eq!(r.loaded.profiles.len(), 1, "{secret}");
        let saved = resave_report(&r);
        assert!(saved.contains("future = 1"), "{saved}");
        assert!(saved.contains("hop_future = \"kept\""), "{saved}");
        assert!(!saved.contains("hv_test_pw"), "{saved}");
        assert!(!saved.contains(&format!("{secret} =")), "{saved}");
    }
}

#[test]
fn ssh_transport_without_table_loads_and_validate_reports() {
    let p = load_from_string(&entry("transport = \"ssh_tunnel\"\n"))[0].clone();
    assert_eq!(p.transport, TransportSpec::SshTunnel(SshSpec::default()));
    assert!(
        validate(&p, EngineCatalog::embedded())
            .iter()
            .any(|i| i.code == IssueCode::SshHostMissing)
    );
    // A hop without host, user or auth loads too, so Validate can say what is missing.
    let p = load_from_string(&entry(
        "transport = \"ssh_tunnel\"\n[[connection.ssh.hops]]\nauth = \"key_file\"\n",
    ))[0]
        .clone();
    let found = codes(&p);
    for code in [
        IssueCode::SshHostMissing,
        IssueCode::SshUserMissing,
        IssueCode::SshKeyFileMissing,
    ] {
        assert!(found.contains(&code), "{code:?}: {found:?}");
    }
}

/// Fix round 1, M3: a `key_file` next to another method is never used; it loads with a notice
/// and the save that drops it is no surprise.
#[test]
fn an_unused_key_file_is_reported_then_dropped() {
    let hop_with = |auth: &str| {
        entry(&format!(
            "transport = \"ssh_tunnel\"\n[[connection.ssh.hops]]\nhost = \"b\"\nuser = \"u\"\n\
             {auth}key_file = \"/k/id_ed25519\"\n"
        ))
    };
    let ignored = |r: &Report| {
        r.diags()
            .iter()
            .any(|d| matches!(d, ProfileDiag::ValueIgnored { index: 0, key } if key == "ssh.hops.key_file"))
    };
    for auth in ["agent", "password", "keyboard_interactive"] {
        let r = load_report(&hop_with(&format!("auth = \"{auth}\"\n")));
        assert!(ignored(&r), "{auth}: {:?}", r.diags());
        assert!(!resave_report(&r).contains("key_file"), "{auth}");
    }
    // Used: no notice, and the key file stays.
    for auth in ["auth = \"key_file\"\n", ""] {
        let r = load_report(&hop_with(auth));
        assert!(!ignored(&r), "{auth:?}: {:?}", r.diags());
        let TransportSpec::SshTunnel(spec) = &r.loaded.profiles[0].transport else {
            panic!("ssh")
        };
        assert_eq!(spec.hops[0].auth, key_file("/k/id_ed25519"));
        assert!(resave_report(&r).contains("key_file = \"/k/id_ed25519\""));
    }
}

#[test]
fn a_malformed_ssh_table_skips_the_entry_and_keeps_it_on_disk() {
    for bad in [
        "[connection.ssh]\nhost_keys = \"trust_everything\"\n",
        "[connection.ssh]\nkeepalive_s = -1\n",
        "[[connection.ssh.hops]]\nhost = \"b\"\nauth = \"telnet\"\n",
        "[[connection.ssh.hops]]\nhost = \"b\"\nport = 0\n",
        "[[connection.ssh.hops]]\nhost = 7\n",
        "[[connection.ssh.hops]]\nhost = \"b\"\nidentity_files = [1]\n",
        "[connection.ssh]\nhops = \"b\"\n",
    ] {
        let text = entry(&format!("transport = \"ssh_tunnel\"\n{bad}"));
        let r = load_report(&text);
        assert!(r.loaded.profiles.is_empty(), "{bad}");
        assert!(
            r.diags()
                .iter()
                .any(|d| matches!(d, ProfileDiag::InvalidEntry { index: 0, .. })),
            "{bad}: {:?}",
            r.diags()
        );
        // The entry this version cannot read stays as it is.
        r.store.save(&[]).unwrap();
        assert_eq!(read(r.store.path()), text, "{bad}");
    }
}

#[test]
fn hops_edited_in_place_keep_their_unknown_keys_and_direct_drops_the_table() {
    let r = load_report(&entry(
        "transport = \"ssh_tunnel\"\n\n[connection.ssh]\nfuture = 1\n\n\
         [[connection.ssh.hops]]\nhost = \"j\"\nuser = \"u\"\nauth = \"agent\"\nnote = \"first\"\n\n\
         [[connection.ssh.hops]]\nhost = \"b\"\nuser = \"u\"\nauth = \"agent\"\nnote = \"second\"\n",
    ));
    let mut p = r.loaded.profiles[0].clone();
    let TransportSpec::SshTunnel(spec) = &mut p.transport else {
        panic!("ssh")
    };
    spec.hops[1].auth = key_file("/k");
    spec.hops.truncate(1);
    spec.hops[0].host = "j2".into();
    r.store.save(&[p.clone()]).unwrap();
    let text = read(r.store.path());
    assert!(text.contains("note = \"first\""), "{text}");
    assert!(!text.contains("second"), "{text}");
    assert_eq!(reload(&r.store), vec![p.clone()]);

    p.transport = TransportSpec::Direct;
    r.store.save(&[p.clone()]).unwrap();
    let text = read(r.store.path());
    assert!(!text.contains("ssh"), "{text}");
    assert_eq!(reload(&r.store), vec![p]);
}

// ---- TLS -------------------------------------------------------------------------------------

#[test]
fn pins_round_trip_as_hex() {
    let p = with_pins(&[pin("db:5432", [0xAB; 32])]);
    let toml = save_to_string(&p);
    assert!(toml.contains(&"ab".repeat(32)), "{toml}");
    assert!(!toml.contains("AB"), "{toml}");
    assert_eq!(load_from_string(&toml)[0].tls.pins, p.tls.pins);

    // Upper case is read too (ASCII only), and so is the array-of-tables form.
    let mut digest = [0u8; 32];
    for (i, b) in digest.iter_mut().enumerate() {
        *b = u8::try_from(i * 8).unwrap();
    }
    let upper: String = digest.iter().map(|b| format!("{b:02X}")).collect();
    let loaded = load_from_string(&entry(&format!(
        "tls.mode = \"require\"\ntls.pins = [{{ endpoint = \"db:5432\", sha256 = \"{upper}\" }}]\n"
    )));
    assert_eq!(loaded[0].tls.pins, vec![pin("db:5432", digest)]);
    let loaded = load_from_string(&entry(&format!(
        "\n[connection.tls]\nmode = \"require\"\n\n[[connection.tls.pins]]\nendpoint = \"a:1\"\n\
         sha256 = \"{upper}\"\n"
    )));
    assert_eq!(loaded[0].tls.pins, vec![pin("a:1", digest)]);
}

#[test]
fn a_malformed_pin_never_loads_without_it() {
    let hex = "ab".repeat(32);
    for bad in [
        format!(
            "tls.pins = [{{ endpoint = \"db:5432\", sha256 = \"{}\" }}]",
            &hex[..62]
        ),
        format!("tls.pins = [{{ endpoint = \"db:5432\", sha256 = \"{hex}00\" }}]"),
        format!(
            "tls.pins = [{{ endpoint = \"db:5432\", sha256 = \"{}zz\" }}]",
            &hex[..62]
        ),
        // A non-ASCII digit never folds into a hex digit.
        format!(
            "tls.pins = [{{ endpoint = \"db:5432\", sha256 = \"{}\u{0661}\" }}]",
            &hex[..62]
        ),
        format!("tls.pins = [{{ sha256 = \"{hex}\" }}]"),
        format!("tls.pins = [{{ endpoint = \"\", sha256 = \"{hex}\" }}]"),
        "tls.pins = \"db:5432\"".to_string(),
        "tls.allow_public_plaintext = \"yes\"".to_string(),
    ] {
        let text = entry(&format!("{bad}\n"));
        let r = load_report(&text);
        // The profile is not loaded at all (it would connect without its pin) and the entry
        // stays on disk as it is.
        assert!(r.loaded.profiles.is_empty(), "{bad}");
        assert!(
            r.diags()
                .iter()
                .any(|d| matches!(d, ProfileDiag::InvalidEntry { index: 0, .. })),
            "{bad}"
        );
        r.store.save(&[]).unwrap();
        assert_eq!(read(r.store.path()), text, "{bad}");
    }
}

#[test]
fn plaintext_consent_and_session_defaults_round_trip() {
    let mut p = with_pins(&[]);
    p.tls.allow_public_plaintext = true;
    p.single_session = true;
    p.tx_mode = Some(TxMode::Manual);
    p.isolation = Some(IsolationLevel::Serializable);
    p.idle_disconnect_s = Some(900);
    p.ssh_retention = Retention::Never;
    let text = save_to_string(&p);
    for line in [
        "tls.allow_public_plaintext = true",
        "single_session = true",
        "tx_mode = \"manual\"",
        "isolation = \"serializable\"",
        "idle_disconnect_s = 900",
        "ssh_retention = \"never\"",
    ] {
        assert!(text.contains(line), "{line} in:\n{text}");
    }
    assert_eq!(load_from_string(&text), vec![p.clone()]);

    for mode in TxMode::ALL {
        p.tx_mode = Some(mode);
        assert_eq!(load_from_string(&save_to_string(&p))[0].tx_mode, Some(mode));
    }
    for level in [
        IsolationLevel::Default,
        IsolationLevel::ReadUncommitted,
        IsolationLevel::ReadCommitted,
        IsolationLevel::RepeatableRead,
        IsolationLevel::Serializable,
    ] {
        p.isolation = Some(level);
        assert_eq!(
            load_from_string(&save_to_string(&p))[0].isolation,
            Some(level)
        );
    }
    for bad in [
        "tx_mode = \"sometimes\"",
        "isolation = \"snapshot\"",
        "idle_disconnect_s = 0",
        "single_session = 1",
        "ssh_retention = \"always\"",
    ] {
        assert!(
            load_from_string(&entry(&format!("{bad}\n"))).is_empty(),
            "{bad}"
        );
    }
}

#[test]
fn keys_at_their_default_are_not_written_and_are_removed_when_reset() {
    let mut p = with_pins(&[pin("db:5432", [1; 32])]);
    p.tls.allow_public_plaintext = true;
    p.single_session = true;
    p.tx_mode = Some(TxMode::Smart);
    p.isolation = Some(IsolationLevel::ReadCommitted);
    p.idle_disconnect_s = Some(60);
    p.ssh_retention = Retention::ForSession;
    let (s, _dir) = store_tmp();
    s.save(&[p.clone()]).unwrap();
    let reset = Profile {
        tls: hacivat_net::TlsSettings {
            mode: TlsMode::Require,
            ..Default::default()
        },
        single_session: false,
        tx_mode: None,
        isolation: None,
        idle_disconnect_s: None,
        ssh_retention: Retention::Forever,
        ..p
    };
    s.save(std::slice::from_ref(&reset)).unwrap();
    let text = read(s.path());
    for key in [
        "pins",
        "allow_public_plaintext",
        "single_session",
        "tx_mode",
        "isolation",
        "idle_disconnect_s",
        "ssh_retention",
        "ssh",
    ] {
        assert!(!text.contains(key), "{key} in:\n{text}");
    }
    assert_eq!(reload(&s), vec![reset]);
}

// ---- M2a files -------------------------------------------------------------------------------

#[test]
fn m2a_file_round_trips_verbatim() {
    let text = include_str!("fixtures/m2a_connections.toml");
    assert_eq!(resave(text), text);
    let loaded = load_from_string(text);
    assert_eq!(loaded.len(), 3);
    assert!(loaded.iter().all(|p| {
        !p.single_session
            && p.tx_mode.is_none()
            && p.isolation.is_none()
            && p.idle_disconnect_s.is_none()
            && p.ssh_retention == Retention::Forever
            && p.tls.pins.is_empty()
            && !p.tls.allow_public_plaintext
    }));
    assert_eq!(loaded[2].engine, DbmsKind::Sqlite);
}
