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
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetsFile {
    target: Vec<RawTarget>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTarget {
    id: String,
    engine: DbmsKind,
    major: String,
    port: u16,
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
    let file: TargetsFile = toml::from_str(TARGETS_TOML)
        .unwrap_or_else(|e| panic!("docker/m2a/targets.toml does not parse: {e}"));
    file.target
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
        })
        .collect()
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
