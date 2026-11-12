//! Docker test targets for the engine tests (R17): which servers `docker/m2a/targets.toml`
//! declares, whether Docker tests are enabled, and where Docker-produced files live. Only ever
//! a dev-dependency. Bodies in Task 11.

use std::path::PathBuf;

use hacivat_proto::DbmsKind;

/// How a target's server is set up for TLS (R17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TlsDefault {
    None,
    SelfSigned,
}

/// One server of the Docker compose project (R17). Its credentials are throwaway test values.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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

/// Whether Docker tests run: `HACIVAT_DOCKER=1` (set by `cargo xtask docker test`).
pub fn docker_enabled() -> bool {
    docker_enabled_from(std::env::var("HACIVAT_DOCKER").ok().as_deref())
}

/// [`docker_enabled`] for a given `HACIVAT_DOCKER` value, so tests never touch the environment
/// (P25, Task 11). The stub answers `false`.
pub fn docker_enabled_from(_v: Option<&str>) -> bool {
    false
}

/// The targets of `engine` (all with `None`) from `docker/m2a/targets.toml` (Task 11).
pub fn targets(_engine: Option<DbmsKind>) -> Vec<DockerTarget> {
    Vec::new()
}

/// The target `id` (Task 11).
pub fn target(_id: &str) -> Option<DockerTarget> {
    None
}

/// A Docker-produced file under `HACIVAT_DOCKER_ARTIFACTS` (P25, Task 11).
pub fn artifact(_name: &str) -> PathBuf {
    PathBuf::new()
}
