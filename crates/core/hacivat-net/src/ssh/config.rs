//! `~/.ssh/config` import (HV-SEC-01, Task 4): hosts, jump chains and the notices the import
//! shows. Every value passes the secret-scan invariant (`is_secret`) before it reaches a
//! profile. Task 1 stubs import nothing.

use std::io;
use std::path::{Path, PathBuf};

use super::{HostKeyPolicy, SshSpec};

/// Why a config host cannot become a tunnel.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConfigIssue {
    ProxyCommand,
    UnsupportedToken(String),
    /// A value looked like a secret; it is never echoed.
    SecretInValue,
    IncludeTooDeep,
    NoHostName,
}

/// Something the import skipped or changed, shown to the user.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConfigNotice {
    MatchSkipped { line: usize },
    UserKnownHostsIgnored { alias: String },
    StrictNoMappedToAsk { alias: String },
    IdentityAgentIgnored { alias: String },
    IncludeTooDeep { line: usize },
}

/// One `Host` block, resolved.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct SshConfigHost {
    pub alias: String,
    pub hostname: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub identity_files: Vec<PathBuf>,
    pub identities_only: bool,
    pub proxy_jump: Vec<String>,
    pub server_alive_interval: Option<u32>,
    pub strict_host_key_checking: Option<HostKeyPolicy>,
    /// Set when the host cannot be imported.
    pub unusable: Option<ConfigIssue>,
}

/// A parsed `~/.ssh/config`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct SshConfig {
    pub hosts: Vec<SshConfigHost>,
    pub notices: Vec<ConfigNotice>,
}

/// Reads the config at `path` (`~` is `home`, `%u` is `local_user`); every value passes
/// `is_secret` (callers pass `hacivat_driver::url::carries_secret`). Blocking: call it on the
/// core runtime. Task 1 stub: imports nothing.
pub fn read_ssh_config(
    _path: &Path,
    _home: &Path,
    _local_user: &str,
    _is_secret: &dyn Fn(&str) -> bool,
) -> io::Result<SshConfig> {
    Ok(SshConfig::default())
}

/// The tunnel of `alias`, following `ProxyJump` into hops. Task 1 stub: no host resolves.
pub fn resolve_alias(_cfg: &SshConfig, _alias: &str) -> Result<SshSpec, ConfigIssue> {
    Err(ConfigIssue::NoHostName)
}
