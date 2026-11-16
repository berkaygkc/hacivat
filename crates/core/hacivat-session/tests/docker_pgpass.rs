//! `.pgpass` parity with libpq (HV-SEC-04, Task 19): every vector of
//! `docker/m2a/pgpass-vectors/vectors.toml` through `pgpass_lookup` and through `psql -w` in
//! `m2a-pg17` (compose `container_name`, P25) must give the same outcome. Run with
//! `cargo xtask docker test --set pr -p hacivat-session` (or `--set full`).
//!
//! **Environment dependency.** The psql side relies on the official `postgres` image's
//! `pg_hba.conf`: loopback (`127.0.0.1/32`, `::1/128`) and the Unix socket are `trust`, and
//! only `host all all all scram-sha-256` asks for a password. A `trust`ed connection never
//! reads `PGPASSFILE`, so psql connects with `hostaddr=<the container's own IP>` (a
//! non-loopback address, scram) while `host=<vector host>` is what libpq looks the file up
//! under. If the image or its pg_hba changes (the container IP becomes trusted, or scram is
//! dropped), psql would skip the file: every vector would then come out `Accepted` or
//! `NoPassword` regardless of its lines, and the test fails loudly at `libpq == expect`
//! rather than passing silently.
//!
//! No password reaches the test output: the vectors name it `@PW@`, psql gets the file on
//! stdin, and an outcome is only `Accepted`, `Rejected` or `NoPassword`.

// Unix-only tests (file modes, Unix sockets, ssh-agent) leave their helpers unused on other
// systems; the lints still run in full on Unix.
#![cfg_attr(not(unix), allow(dead_code, unused_imports, unused_variables))]

mod docker_support;

use std::io::Write;
use std::process::{Command, Stdio};

use docker_support::*;
use hacivat_secrets::{ExposeSecret, pgpass_lookup};

/// The vectors, compiled in so the test never depends on its working directory.
const VECTORS_TOML: &str = include_str!("../../../../docker/m2a/pgpass-vectors/vectors.toml");

/// The container psql runs in (compose `container_name: m2a-pg17`).
const CONTAINER: &str = "m2a-pg17";

/// What a sign-in with the file's password does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// The matched password is the server's.
    Accepted,
    /// A password matched, and the server refuses it.
    Rejected,
    /// No line matched, or the matched password is empty (libpq never sends an empty one).
    NoPassword,
}

impl Outcome {
    fn from_id(id: &str) -> Outcome {
        match id {
            "accepted" => Outcome::Accepted,
            "rejected" => Outcome::Rejected,
            "no_password" => Outcome::NoPassword,
            other => panic!("unknown expect {other:?}"),
        }
    }
}

struct Vector {
    name: String,
    lines: Vec<String>,
    host: String,
    /// 0: no port given.
    port: u16,
    database: String,
    user: String,
    expect: Outcome,
}

impl Vector {
    /// The file's bytes, with `@PW@` replaced by the servers' password.
    fn file(&self) -> Vec<u8> {
        let password = docker_password();
        let mut out = String::new();
        for line in &self.lines {
            out.push_str(&line.replace("@PW@", password.expose_secret()));
            out.push('\n');
        }
        out.into_bytes()
    }
}

fn vectors() -> Vec<Vector> {
    let doc: toml_edit::DocumentMut = VECTORS_TOML.parse().expect("vectors.toml parses");
    let tables = doc["vector"]
        .as_array_of_tables()
        .expect("vectors.toml has [[vector]] tables");
    let text = |t: &toml_edit::Table, key: &str| -> String {
        t[key]
            .as_str()
            .unwrap_or_else(|| panic!("{key} is a string"))
            .to_string()
    };
    tables
        .iter()
        .map(|t| Vector {
            name: text(t, "name"),
            lines: t["lines"]
                .as_array()
                .expect("lines is an array")
                .iter()
                .map(|l| l.as_str().expect("a line is a string").to_string())
                .collect(),
            host: text(t, "host"),
            port: u16::try_from(t["port"].as_integer().expect("port is an integer"))
                .expect("a port"),
            database: text(t, "database"),
            user: text(t, "user"),
            expect: Outcome::from_id(&text(t, "expect")),
        })
        .collect()
}

/// Our outcome: `pgpass_lookup` over the vector's file (0600).
fn ours(v: &Vector) -> Outcome {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pgpass");
    std::fs::write(&path, v.file()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let found = pgpass_lookup(&path, &v.host, v.port, &v.database, Some(&v.user))
        .unwrap_or_else(|w| panic!("{}: the file was ignored: {w:?}", v.name));
    match found {
        None => Outcome::NoPassword,
        // B27 (m2b-t06): an empty password field is `None`.
        Some(m) if m.password.is_none() => Outcome::NoPassword,
        Some(m)
            if m.password.as_ref().map(|p| p.expose_secret())
                == Some(docker_password().expose_secret()) =>
        {
            Outcome::Accepted
        }
        Some(_) => Outcome::Rejected,
    }
}

/// A libpq connection-string value, quoted.
fn quoted(value: &str) -> String {
    let mut out = String::from("'");
    for c in value.chars() {
        if c == '\'' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// The script run in the container: the file arrives on stdin, `$1` is the connection string
/// without `hostaddr`, and the temporary file is removed afterwards.
///
/// The `hostaddr` trick: the official image's pg_hba trusts loopback and the socket, and a
/// trusted connection never reads `PGPASSFILE`. So the script dials the container's own
/// (non-loopback) address with `hostaddr`, where pg_hba's `host all all all scram-sha-256`
/// asks for a password, while libpq looks the file up under `host` (libpq: with both set,
/// `host` names the connection in the password file). If the image or pg_hba changes so that
/// this address is trusted, psql skips the file and the test fails at `libpq == expect`.
const SCRIPT: &str = r#"f=$(mktemp) || exit 90
cat > "$f"
chmod 600 "$f"
ip=$(hostname -i | cut -d ' ' -f 1)
env -i PATH="$PATH" LC_ALL=C PGPASSFILE="$f" psql -w -X -A -t -c 'select 1' "$1 hostaddr=$ip"
rc=$?
rm -f "$f"
exit $rc
"#;

/// libpq's outcome: `psql -w` in the container with `PGPASSFILE` holding the vector's file.
async fn psql(v: &Vector) -> Outcome {
    let mut conninfo = format!(
        "host={} dbname={} user={} sslmode=disable",
        quoted(&v.host),
        quoted(&v.database),
        quoted(&v.user)
    );
    if v.port != 0 {
        conninfo.push_str(&format!(" port={}", v.port));
    }
    let file = v.file();
    let name = v.name.clone();
    tokio::task::spawn_blocking(move || {
        let mut child = Command::new("docker")
            .args(["exec", "-i", CONTAINER, "sh", "-c", SCRIPT, "sh", &conninfo])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("docker runs");
        child
            .stdin
            .take()
            .expect("stdin is piped")
            .write_all(&file)
            .expect("the file reaches psql");
        let out = child.wait_with_output().expect("docker exec finishes");
        let stderr = String::from_utf8_lossy(&out.stderr);
        if out.status.success() {
            Outcome::Accepted
        } else if stderr.contains("password authentication failed") {
            Outcome::Rejected
        } else if stderr.contains("no password supplied") {
            Outcome::NoPassword
        } else if stderr.contains("database") && stderr.contains("does not exist") {
            // The server checks the password before the database (InitPostgres).
            Outcome::Accepted
        } else {
            panic!("{name}: unexpected psql result {:?}: {stderr}", out.status)
        }
    })
    .await
    .expect("the psql task finishes")
}

#[test]
fn the_vectors_cover_the_hv_sec_04_edge_list() {
    let names: Vec<String> = vectors().into_iter().map(|v| v.name).collect();
    assert_eq!(
        names,
        [
            "escaped-colon",
            "wildcard",
            "case-mismatch",
            "empty-password",
            "crlf",
            "localhost-vs-socket",
            "replication-database",
            "default-port",
            "trailing-space-kept",
            "first-match-wins",
        ]
    );
    for v in vectors() {
        assert_eq!(ours(&v), v.expect, "{}", v.name);
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn pgpass_parity_with_psql() {
    if !docker_enabled() {
        return;
    }
    if target_required("pg17").is_none() {
        return;
    }
    for v in vectors() {
        let libpq = psql(&v).await;
        assert_eq!(ours(&v), libpq, "{}", v.name);
        assert_eq!(libpq, v.expect, "{}", v.name);
    }
}
