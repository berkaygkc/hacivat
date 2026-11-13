//! Docker test targets for the engine tests (R17): which servers `docker/m2a/targets.toml`
//! declares, whether Docker tests are enabled, and where Docker-produced files live. Only ever
//! a dev-dependency.
//!
//! The environment is read only by the thin wrappers ([`docker_enabled`], [`targets`],
//! [`target`], [`artifact`]); each has a pure `*_from`/`*_in` twin that the tests call, so no
//! test ever sets or removes an environment variable (P25).

use std::fmt;
use std::path::PathBuf;

use hacivat_proto::DbmsKind;
use serde::Deserialize;

/// `docker/m2a/targets.toml`, compiled in so a test never depends on its working directory.
const TARGETS_TOML: &str = include_str!("../../../../docker/m2a/targets.toml");

/// Every target listens on loopback only (compose.yml binds `127.0.0.1`).
const HOST: &str = "127.0.0.1";
/// The throwaway test credentials of every server in docker/m2a (never used outside tests).
const USER: &str = "hv_test";
const PASSWORD: &str = "hv_test_pw";
const DATABASE: &str = "hv_test";

/// How a target's server is set up for TLS (R17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TlsDefault {
    None,
    SelfSigned,
}

/// One server of the Docker compose project (R17). Its credentials are throwaway test values.
/// `Debug` still redacts the password, so a failing assertion never prints it.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct DockerTarget {
    pub id: String,
    pub engine: DbmsKind,
    pub major: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: String,
    pub tls_default: TlsDefault,
    /// The test sets it belongs to (`pr`, `full`).
    pub sets: Vec<String>,
    /// The in-network endpoint `(compose service, container port)`: what a test that goes
    /// through `sshd-a`/`sshd-b` dials (`pg17:5432`, not `127.0.0.1:54317`; Q6).
    pub internal: (String, u16),
}

impl fmt::Debug for DockerTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DockerTarget")
            .field("id", &self.id)
            .field("engine", &self.engine)
            .field("major", &self.major)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &"<redacted>")
            .field("database", &self.database)
            .field("tls_default", &self.tls_default)
            .field("sets", &self.sets)
            .field("internal", &self.internal)
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetsFile {
    target: Vec<RawTarget>,
    #[serde(default)]
    ssh_target: Vec<RawSshTarget>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSshTarget {
    id: String,
    host: String,
    port: u16,
    user: String,
    via: Option<String>,
    sets: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTarget {
    id: String,
    service: String,
    engine: DbmsKind,
    major: String,
    port: u16,
    internal_port: u16,
    tls_default: RawTls,
    sets: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawTls {
    None,
    SelfSigned,
}

/// Whether Docker tests run: `HACIVAT_DOCKER=1` (set by `cargo xtask docker test`).
pub fn docker_enabled() -> bool {
    docker_enabled_from(std::env::var("HACIVAT_DOCKER").ok().as_deref())
}

/// [`docker_enabled`] for a given `HACIVAT_DOCKER` value: true only for exactly `"1"`.
pub fn docker_enabled_from(v: Option<&str>) -> bool {
    v == Some("1")
}

/// Every target of `docker/m2a/targets.toml`, in file order, ignoring `HACIVAT_DOCKER_SET`.
///
/// # Panics
/// When the committed file does not parse (a repository bug, caught by this crate's tests).
pub fn all_targets_unfiltered() -> Vec<DockerTarget> {
    targets_file()
        .target
        .into_iter()
        .map(|t| DockerTarget {
            id: t.id,
            engine: t.engine,
            major: t.major,
            host: HOST.to_string(),
            port: t.port,
            user: USER.to_string(),
            password: PASSWORD.to_string(),
            database: DATABASE.to_string(),
            tls_default: match t.tls_default {
                RawTls::None => TlsDefault::None,
                RawTls::SelfSigned => TlsDefault::SelfSigned,
            },
            sets: t.sets,
            internal: (t.service, t.internal_port),
        })
        .collect()
}

fn targets_file() -> TargetsFile {
    toml::from_str(TARGETS_TOML)
        .unwrap_or_else(|e| panic!("docker/m2a/targets.toml does not parse: {e}"))
}

/// The ids of the targets in `set`, sorted.
pub fn ids_in_set(set: &str) -> Vec<String> {
    let mut ids: Vec<String> = targets_in_set(Some(set), None)
        .into_iter()
        .map(|t| t.id)
        .collect();
    ids.sort();
    ids
}

/// The targets in `set` (every target with `None`) of `engine` (every engine with `None`).
pub fn targets_in_set(set: Option<&str>, engine: Option<DbmsKind>) -> Vec<DockerTarget> {
    all_targets_unfiltered()
        .into_iter()
        .filter(|t| set.is_none_or(|s| t.sets.iter().any(|x| x == s)))
        .filter(|t| engine.is_none_or(|e| t.engine == e))
        .collect()
}

/// The targets of `engine` (all with `None`) in the set named by `HACIVAT_DOCKER_SET` (every
/// target when it is unset), so a `--set pr` run never reaches a server that is not up.
pub fn targets(engine: Option<DbmsKind>) -> Vec<DockerTarget> {
    targets_in_set(std::env::var("HACIVAT_DOCKER_SET").ok().as_deref(), engine)
}

/// The target `id`, when it is in the `HACIVAT_DOCKER_SET` set (see [`targets`]).
pub fn target(id: &str) -> Option<DockerTarget> {
    targets(None).into_iter().find(|t| t.id == id)
}

// ## m2b-t01-interfaces

/// The target `id` a set-gated Docker test needs (Q6). `None` (with `skip <id>: not in set
/// <set>` on stderr) when `id` is not in the active `HACIVAT_DOCKER_SET`, so the test returns
/// early. A target in the active set never skips silently: it must accept a TCP connection,
/// else this panics.
///
/// # Panics
/// When `id` is in the active set but its server does not accept a connection within 5 s.
pub fn require_target(id: &str) -> Option<DockerTarget> {
    let set = std::env::var("HACIVAT_DOCKER_SET").ok();
    let target = require_target_in_set(set.as_deref(), id)?;
    let addr = (target.host.as_str(), target.port);
    let reachable = std::net::ToSocketAddrs::to_socket_addrs(&addr)
        .ok()
        .and_then(|mut addrs| addrs.next())
        .is_some_and(|a| {
            std::net::TcpStream::connect_timeout(&a, std::time::Duration::from_secs(5)).is_ok()
        });
    assert!(
        reachable,
        "{id} is in set {} but its server at {}:{} is not healthy",
        set.as_deref().unwrap_or("<all>"),
        target.host,
        target.port
    );
    Some(target)
}

/// [`require_target`] for a given active set (`None`: every target), without the health check:
/// the pure core, like [`targets_in_set`].
pub fn require_target_in_set(set: Option<&str>, id: &str) -> Option<DockerTarget> {
    let found = targets_in_set(set, None).into_iter().find(|t| t.id == id);
    if found.is_none() {
        eprintln!("skip {id}: not in set {}", set.unwrap_or("<all>"));
    }
    found
}

#[cfg(feature = "ssh")]
pub mod agent;
pub mod chaos;
#[cfg(feature = "ssh")]
pub mod ssh_server;

/// A Docker-produced file: `$HACIVAT_DOCKER_ARTIFACTS/<name>` (P25). `cargo xtask docker`
/// sets the variable to `<main checkout>/target/docker`.
///
/// # Panics
/// When `HACIVAT_DOCKER_ARTIFACTS` is unset: only Docker tests call this, and they run through
/// `cargo xtask docker test`.
pub fn artifact(name: &str) -> PathBuf {
    artifact_in(
        std::env::var("HACIVAT_DOCKER_ARTIFACTS").ok().as_deref(),
        name,
    )
}

/// [`artifact`] for a given `HACIVAT_DOCKER_ARTIFACTS` value.
///
/// # Panics
/// When `dir` is `None`.
pub fn artifact_in(dir: Option<&str>, name: &str) -> PathBuf {
    let Some(dir) = dir else {
        panic!(
            "HACIVAT_DOCKER_ARTIFACTS is unset; Docker tests run only through \
             `cargo xtask docker test --set pr|full` (wanted the artifact {name})"
        );
    };
    PathBuf::from(dir).join(name)
}

// ## m2b-t16-ssh-targets

/// The password of the SSH bastions' user (throwaway, like [`PASSWORD`]).
const SSH_PASSWORD: &str = "hv_ssh_pw";
/// The passphrase of the encrypted generated client key (`ssh_client_key(true)`).
pub const SSH_CLIENT_KEY_PASSPHRASE: &str = "hv_test_pw";

/// One SSH bastion of the Docker compose project (`sshd-a`, `sshd-b`). `host` and `port` are
/// what the connecting side dials: the loopback port for `sshd-a`; for `sshd-b` (`via`
/// `sshd-a`) the in-network name and port, since it publishes nothing.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SshTarget {
    pub id: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    /// The bastion this one is reached through (ProxyJump), if any.
    pub via: Option<String>,
    pub sets: Vec<String>,
}

impl fmt::Debug for SshTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SshTarget")
            .field("id", &self.id)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &"<redacted>")
            .field("via", &self.via)
            .field("sets", &self.sets)
            .finish()
    }
}

/// The SSH targets in `set` (every one with `None`), in file order.
pub fn ssh_targets_in_set(set: Option<&str>) -> Vec<SshTarget> {
    targets_file()
        .ssh_target
        .into_iter()
        .filter(|t| set.is_none_or(|s| t.sets.iter().any(|x| x == s)))
        .map(|t| SshTarget {
            id: t.id,
            host: t.host,
            port: t.port,
            user: t.user,
            password: SSH_PASSWORD.to_string(),
            via: t.via,
            sets: t.sets,
        })
        .collect()
}

/// The SSH targets in the set named by `HACIVAT_DOCKER_SET` (all when it is unset).
pub fn ssh_targets() -> Vec<SshTarget> {
    ssh_targets_in_set(std::env::var("HACIVAT_DOCKER_SET").ok().as_deref())
}

/// The SSH target `id`, when it is in the `HACIVAT_DOCKER_SET` set.
pub fn ssh_target(id: &str) -> Option<SshTarget> {
    ssh_targets().into_iter().find(|t| t.id == id)
}

/// The generated client private key (`encrypted`: the one protected by
/// [`SSH_CLIENT_KEY_PASSPHRASE`]); its `.pub` sits next to it. Written by the `ssh-client-keys`
/// service into the artifacts directory.
pub fn ssh_client_key(encrypted: bool) -> PathBuf {
    ssh_client_key_in(
        std::env::var("HACIVAT_DOCKER_ARTIFACTS").ok().as_deref(),
        encrypted,
    )
}

/// [`ssh_client_key`] for a given `HACIVAT_DOCKER_ARTIFACTS` value.
///
/// # Panics
/// When `dir` is `None` (see [`artifact_in`]).
pub fn ssh_client_key_in(dir: Option<&str>, encrypted: bool) -> PathBuf {
    artifact_in(
        dir,
        if encrypted {
            "ssh-client-ed25519-enc"
        } else {
            "ssh-client-ed25519"
        },
    )
}

/// The public host key (`ssh-host-<id>.pub`) the bastion `id` generated at startup.
pub fn ssh_host_pub(id: &str) -> PathBuf {
    artifact(&format!("ssh-host-{id}.pub"))
}
