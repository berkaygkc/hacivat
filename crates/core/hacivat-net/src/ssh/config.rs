//! `~/.ssh/config` import (HV-SEC-01, B12, Task 4): hosts, jump chains and the notices the
//! import shows. Its own parser, no new crate. Every imported value passes the secret-scan
//! invariant (`is_secret`) before it reaches a profile; a hit makes the entry unusable and the
//! value is never kept or echoed.
//!
//! Supported, as OpenSSH reads them: `Host` patterns (`*`, `?`, `!`), first-value-wins, `Include`
//! (`~`, a `*` glob in the last segment, relative to `~/.ssh`, at most 8 levels; conditional
//! inside a `Host` block), and the keys `HostName`, `User`, `Port`, `IdentityFile`,
//! `IdentitiesOnly`, `ProxyJump`, `ProxyCommand` (unusable), `ServerAliveInterval`,
//! `StrictHostKeyChecking`, `UserKnownHostsFile` and `IdentityAgent` (both noted, not
//! imported). `Match` blocks are skipped with a notice. The import only reads files: nothing in
//! a config is ever executed.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::known_hosts::{glob, read_lines};
use super::{HopSpec, HostKeyPolicy, SshAuth, SshSpec};

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

/// How deep `Include` nests (OpenSSH's limit).
const MAX_INCLUDE_DEPTH: usize = 8;
/// How many files one import reads at most: a glob that includes itself would otherwise read
/// `files^8` of them.
const MAX_FILES: usize = 256;
/// The most bytes read from one config file.
const MAX_CONFIG_BYTES: u64 = 16 << 20;
/// How deep a `ProxyJump` chain recurses through jump hosts' own `ProxyJump`.
const MAX_JUMP_DEPTH: usize = 8;
/// The most hops a tunnel has: `hacivat_secrets::MAX_SSH_HOP` (8) is the last hop index with a
/// secret slot.
const MAX_HOPS: usize = 9;
const DEFAULT_PORT: u16 = 22;

/// Reads the config at `path` (`~` is `home`, `%u` is `local_user`); every value passes
/// `is_secret` (callers pass `hacivat_driver::url::carries_secret`). Blocking: call it on the
/// core runtime.
///
/// `hosts` lists every concrete alias of a `Host` line (no wildcard, not negated) in file order,
/// then every jump host a `ProxyJump` names that no `Host` line declares (it is resolved through
/// the wildcard blocks, as `ssh -J` would). A host's `user` is OpenSSH's effective user: its
/// `User`, else `local_user`. Values are read as UTF-8, lossily.
pub fn read_ssh_config(
    path: &Path,
    home: &Path,
    local_user: &str,
    is_secret: &dyn Fn(&str) -> bool,
) -> io::Result<SshConfig> {
    let mut parser = Parser {
        home,
        is_secret,
        blocks: Vec::new(),
        aliases: Vec::new(),
        notices: Vec::new(),
        files: 0,
    };
    parser.file(path, 0, &[])?;
    let Parser {
        blocks,
        aliases,
        mut notices,
        ..
    } = parser;
    let mut hosts: Vec<SshConfigHost> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue = aliases;
    let mut next = 0;
    while next < queue.len() {
        let alias = queue[next].clone();
        next += 1;
        if !seen.insert(alias.to_ascii_lowercase()) {
            continue;
        }
        let host = resolve_host(&blocks, &alias, home, local_user, &mut notices);
        for jump in &host.proxy_jump {
            if let Some(spec) = JumpSpec::parse(jump) {
                queue.push(spec.host);
            }
        }
        hosts.push(host);
    }
    Ok(SshConfig { hosts, notices })
}

/// The tunnel of `alias`, following `ProxyJump` into hops: `ProxyJump j1,j2` on `a` gives
/// `[j1, j2, a]`. As with `ssh -J`, the first jump host follows its own `ProxyJump`; the later
/// ones are reached through the earlier ones. A cycle (or a chain over 9 hops) is
/// `IncludeTooDeep`; an alias the config does not know is `NoHostName`; an unusable hop makes
/// the chain unusable.
pub fn resolve_alias(cfg: &SshConfig, alias: &str) -> Result<SshSpec, ConfigIssue> {
    let host = find(cfg, alias)?;
    let mut hops = Vec::new();
    push_hops(cfg, host, None, None, true, 0, &mut hops)?;
    Ok(SshSpec {
        hops,
        host_keys: host.strict_host_key_checking.unwrap_or_default(),
        keepalive_s: host.server_alive_interval.filter(|s| *s > 0),
    })
}

fn find<'a>(cfg: &'a SshConfig, alias: &str) -> Result<&'a SshConfigHost, ConfigIssue> {
    cfg.hosts
        .iter()
        .find(|h| h.alias.eq_ignore_ascii_case(alias))
        .ok_or(ConfigIssue::NoHostName)
}

fn push_hops(
    cfg: &SshConfig,
    host: &SshConfigHost,
    user: Option<&str>,
    port: Option<u16>,
    follow_jumps: bool,
    depth: usize,
    hops: &mut Vec<HopSpec>,
) -> Result<(), ConfigIssue> {
    if depth > MAX_JUMP_DEPTH {
        return Err(ConfigIssue::IncludeTooDeep);
    }
    if let Some(issue) = &host.unusable {
        return Err(issue.clone());
    }
    if follow_jumps {
        for (i, jump) in host.proxy_jump.iter().enumerate() {
            let spec = JumpSpec::parse(jump).ok_or(ConfigIssue::NoHostName)?;
            let jump_host = find(cfg, &spec.host)?;
            push_hops(
                cfg,
                jump_host,
                spec.user.as_deref(),
                spec.port,
                i == 0,
                depth + 1,
                hops,
            )?;
        }
    }
    hops.push(HopSpec {
        host: host.hostname.clone().unwrap_or_else(|| host.alias.clone()),
        port: port.or(host.port).unwrap_or(DEFAULT_PORT),
        user: user
            .map(str::to_string)
            .or_else(|| host.user.clone())
            .unwrap_or_default(),
        auth: match host.identity_files.first() {
            Some(path) => SshAuth::KeyFile { path: path.clone() },
            None => SshAuth::Agent,
        },
        identities_only: host.identities_only,
        identity_files: host.identity_files.clone(),
    });
    if hops.len() > MAX_HOPS {
        return Err(ConfigIssue::IncludeTooDeep);
    }
    Ok(())
}

/// `[user@]host[:port]` or `ssh://[user@]host[:port]`; an IPv6 host is in brackets.
struct JumpSpec {
    user: Option<String>,
    host: String,
    port: Option<u16>,
}

impl JumpSpec {
    fn parse(spec: &str) -> Option<JumpSpec> {
        let spec = spec.strip_prefix("ssh://").unwrap_or(spec);
        let (user, rest) = match spec.rsplit_once('@') {
            Some((user, rest)) if !user.is_empty() => (Some(user.to_string()), rest),
            Some(_) => return None,
            None => (None, spec),
        };
        let (host, port) = if let Some(bracketed) = rest.strip_prefix('[') {
            let (host, after) = bracketed.split_once(']')?;
            match after {
                "" => (host, None),
                _ => (host, Some(after.strip_prefix(':')?)),
            }
        } else if rest.matches(':').count() == 1 {
            let (host, port) = rest.split_once(':')?;
            (host, Some(port))
        } else {
            (rest, None)
        };
        let port = match port {
            Some(p) => Some(p.parse::<u16>().ok().filter(|p| *p > 0)?),
            None => None,
        };
        (!host.is_empty()).then(|| JumpSpec {
            user,
            host: host.to_string(),
            port,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    HostName,
    User,
    Port,
    IdentityFile,
    IdentitiesOnly,
    ProxyJump,
    ProxyCommand,
    ServerAliveInterval,
    StrictHostKeyChecking,
    UserKnownHostsFile,
    IdentityAgent,
}

impl Key {
    fn from_keyword(keyword: &str) -> Option<Key> {
        const KEYS: [(&str, Key); 11] = [
            ("hostname", Key::HostName),
            ("user", Key::User),
            ("port", Key::Port),
            ("identityfile", Key::IdentityFile),
            ("identitiesonly", Key::IdentitiesOnly),
            ("proxyjump", Key::ProxyJump),
            ("proxycommand", Key::ProxyCommand),
            ("serveraliveinterval", Key::ServerAliveInterval),
            ("stricthostkeychecking", Key::StrictHostKeyChecking),
            ("userknownhostsfile", Key::UserKnownHostsFile),
            ("identityagent", Key::IdentityAgent),
        ];
        KEYS.iter()
            .find(|(name, _)| keyword.eq_ignore_ascii_case(name))
            .map(|(_, key)| *key)
    }
}

/// One imported line. A secret-looking value is not kept.
#[derive(Debug)]
struct Directive {
    key: Key,
    /// The first argument; `None` when the line looked like a secret.
    value: Option<String>,
}

/// A run of directives and the host patterns it applies to.
#[derive(Debug)]
struct Block {
    /// The patterns of the `Host` blocks enclosing the `Include`s this block came through: each
    /// must match too.
    guards: Vec<Vec<String>>,
    /// `None`: the lines before any `Host` (every host). `Some(vec![])` matches nothing (a
    /// skipped `Match` block).
    patterns: Option<Vec<String>>,
    directives: Vec<Directive>,
}

impl Block {
    fn applies_to(&self, name: &str) -> bool {
        self.guards.iter().all(|g| patterns_match(g, name))
            && self
                .patterns
                .as_ref()
                .is_none_or(|p| patterns_match(p, name))
    }
}

/// OpenSSH's `Host` matching: a negated pattern that matches excludes the block; otherwise any
/// positive match applies it. ASCII case is ignored.
fn patterns_match(patterns: &[String], name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let mut positive = false;
    for pattern in patterns {
        let pattern = pattern.to_ascii_lowercase();
        match pattern.strip_prefix('!') {
            Some(negated) if glob(negated.as_bytes(), name.as_bytes()) => return false,
            Some(_) => {}
            None => positive |= glob(pattern.as_bytes(), name.as_bytes()),
        }
    }
    positive
}

struct Parser<'a> {
    home: &'a Path,
    is_secret: &'a dyn Fn(&str) -> bool,
    blocks: Vec<Block>,
    aliases: Vec<String>,
    notices: Vec<ConfigNotice>,
    files: usize,
}

impl Parser<'_> {
    /// Reads one file; `guards` are the patterns of the `Host` blocks around the `Include` that
    /// named it. Only the top-level file's errors surface; an unreadable include is skipped.
    fn file(&mut self, path: &Path, depth: usize, guards: &[Vec<String>]) -> io::Result<()> {
        self.files += 1;
        let mut lines = Vec::new();
        read_lines(path, MAX_CONFIG_BYTES, |no, line| {
            lines.push((no, String::from_utf8_lossy(line).into_owned()));
        })?;
        let mut current = self.open_block(guards, None);
        let mut in_match = false;
        for (no, line) in lines {
            let Some((keyword, args)) = tokenize(&line) else {
                continue;
            };
            if keyword.eq_ignore_ascii_case("host") {
                in_match = false;
                let patterns: Vec<String> =
                    args.into_iter().filter(|p| !(self.is_secret)(p)).collect();
                for p in &patterns {
                    if !p.starts_with('!') && !p.contains(['*', '?']) {
                        self.aliases.push(p.clone());
                    }
                }
                current = self.open_block(guards, Some(patterns));
            } else if keyword.eq_ignore_ascii_case("match") {
                in_match = true;
                self.notices.push(ConfigNotice::MatchSkipped { line: no });
                current = self.open_block(guards, Some(Vec::new()));
            } else if in_match {
                continue;
            } else if keyword.eq_ignore_ascii_case("include") {
                let patterns = self.blocks[current].patterns.clone();
                let mut inner = guards.to_vec();
                inner.extend(patterns.clone());
                for arg in &args {
                    self.include(arg, no, depth, &inner);
                }
                // The rest of this file continues the block the `Include` sat in.
                current = self.open_block(guards, patterns);
            } else if let Some(key) = Key::from_keyword(&keyword) {
                let secret = (self.is_secret)(&args.join(" "));
                let value = (!secret).then(|| args.into_iter().next().unwrap_or_default());
                self.blocks[current]
                    .directives
                    .push(Directive { key, value });
            }
        }
        Ok(())
    }

    fn open_block(&mut self, guards: &[Vec<String>], patterns: Option<Vec<String>>) -> usize {
        self.blocks.push(Block {
            guards: guards.to_vec(),
            patterns,
            directives: Vec::new(),
        });
        self.blocks.len() - 1
    }

    /// One `Include` argument: `~` expanded, relative to `~/.ssh`, a `*` glob in the last
    /// segment, files in lexical order. Missing files are skipped, as OpenSSH does.
    fn include(&mut self, arg: &str, line: usize, depth: usize, guards: &[Vec<String>]) {
        let path = match arg.strip_prefix("~/") {
            Some(rest) => self.home.join(rest),
            None if arg == "~" => self.home.to_path_buf(),
            None => self.home.join(".ssh").join(arg),
        };
        let mut paths = Vec::new();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        match (path.parent(), name) {
            (Some(dir), Some(name)) if name.contains('*') => {
                if dir.to_string_lossy().contains('*') {
                    return;
                }
                if let Ok(entries) = fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let file = entry.file_name().to_string_lossy().into_owned();
                        if !file.starts_with('.') && glob(name.as_bytes(), file.as_bytes()) {
                            paths.push(entry.path());
                        }
                    }
                }
                paths.sort();
            }
            _ => paths.push(path),
        }
        for path in paths {
            if depth + 1 > MAX_INCLUDE_DEPTH || self.files >= MAX_FILES {
                let notice = ConfigNotice::IncludeTooDeep { line };
                if !self.notices.contains(&notice) {
                    self.notices.push(notice);
                }
                return;
            }
            let _ = self.file(&path, depth + 1, guards);
        }
    }
}

/// `keyword args...`: `keyword=value` and `keyword = value` too, double quotes group, an unquoted
/// `#` starts a comment. `None` for blank and comment lines.
fn tokenize(line: &str) -> Option<(String, Vec<String>)> {
    let line = line.trim_start();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let end = line
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(line.len());
    let keyword = line[..end].to_string();
    let mut rest = line[end..].trim_start();
    if let Some(after) = rest.strip_prefix('=') {
        rest = after.trim_start();
    }
    let mut args = Vec::new();
    let mut chars = rest.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let Some(&first) = chars.peek() else { break };
        if first == '#' {
            break;
        }
        let mut arg = String::new();
        if first == '"' {
            chars.next();
            for c in chars.by_ref() {
                if c == '"' {
                    break;
                }
                arg.push(c);
            }
        } else {
            while let Some(c) = chars.next_if(|c| !c.is_whitespace()) {
                arg.push(c);
            }
        }
        args.push(arg);
    }
    Some((keyword, args))
}

/// The settings of one alias: every block that applies, in order, first value wins
/// (`IdentityFile` accumulates). Tokens expand after all blocks are read.
fn resolve_host(
    blocks: &[Block],
    alias: &str,
    home: &Path,
    local_user: &str,
    notices: &mut Vec<ConfigNotice>,
) -> SshConfigHost {
    let mut host = SshConfigHost {
        alias: alias.to_string(),
        ..SshConfigHost::default()
    };
    let mut set: HashSet<Key> = HashSet::new();
    let mut hostname = None;
    let mut identities: Vec<String> = Vec::new();
    let mut proxy_set = false;
    for block in blocks.iter().filter(|b| b.applies_to(alias)) {
        for d in &block.directives {
            let first = match d.key {
                Key::IdentityFile => true,
                Key::ProxyJump | Key::ProxyCommand => !std::mem::replace(&mut proxy_set, true),
                key => set.insert(key),
            };
            if !first {
                continue;
            }
            let Some(word) = d.value.as_deref() else {
                host.unusable.get_or_insert(ConfigIssue::SecretInValue);
                continue;
            };
            match d.key {
                Key::HostName => hostname = Some(word.to_string()),
                Key::User => host.user = Some(word.to_string()),
                Key::Port => host.port = word.parse::<u16>().ok().filter(|p| *p > 0),
                Key::IdentityFile => {
                    if !word.eq_ignore_ascii_case("none") && !word.is_empty() {
                        identities.push(word.to_string());
                    }
                }
                Key::IdentitiesOnly => host.identities_only = yes(word),
                Key::ProxyJump => {
                    if !word.eq_ignore_ascii_case("none") {
                        host.proxy_jump = word
                            .split(',')
                            .filter(|j| !j.is_empty())
                            .map(str::to_string)
                            .collect();
                    }
                }
                Key::ProxyCommand => {
                    if !word.eq_ignore_ascii_case("none") {
                        host.unusable.get_or_insert(ConfigIssue::ProxyCommand);
                    }
                }
                Key::ServerAliveInterval => host.server_alive_interval = word.parse().ok(),
                Key::StrictHostKeyChecking => {
                    host.strict_host_key_checking = match word.to_ascii_lowercase().as_str() {
                        "yes" => Some(HostKeyPolicy::Strict),
                        "accept-new" => Some(HostKeyPolicy::AcceptNew),
                        "ask" => Some(HostKeyPolicy::Ask),
                        "no" | "off" => {
                            let alias = alias.to_string();
                            notice(ConfigNotice::StrictNoMappedToAsk { alias }, notices);
                            Some(HostKeyPolicy::Ask)
                        }
                        _ => None,
                    }
                }
                Key::UserKnownHostsFile => {
                    let alias = alias.to_string();
                    notice(ConfigNotice::UserKnownHostsIgnored { alias }, notices);
                }
                Key::IdentityAgent => {
                    if word != "SSH_AUTH_SOCK" {
                        let alias = alias.to_string();
                        notice(ConfigNotice::IdentityAgentIgnored { alias }, notices);
                    }
                }
            }
        }
    }
    let user = host
        .user
        .get_or_insert_with(|| local_user.to_string())
        .clone();
    let port = host.port.unwrap_or(DEFAULT_PORT).to_string();
    let home_s = home.to_string_lossy().into_owned();
    let mut tokens = Tokens {
        home: &home_s,
        host: alias,
        port: &port,
        user: &user,
        local_user,
    };
    if let Some(name) = hostname {
        match tokens.expand(&name) {
            Ok(name) => host.hostname = Some(name),
            Err(token) => {
                host.unusable
                    .get_or_insert(ConfigIssue::UnsupportedToken(token));
            }
        }
    }
    let resolved = host.hostname.clone().unwrap_or_else(|| alias.to_string());
    tokens.host = &resolved;
    for raw in identities {
        match expand_path(&raw, home, &tokens) {
            Ok(path) if !host.identity_files.contains(&path) => host.identity_files.push(path),
            Ok(_) => {}
            Err(token) => {
                host.unusable
                    .get_or_insert(ConfigIssue::UnsupportedToken(token));
            }
        }
    }
    for jump in &host.proxy_jump {
        if let Some(token) = jump.find('%').map(|i| token_at(jump, i)) {
            host.unusable
                .get_or_insert(ConfigIssue::UnsupportedToken(token));
        }
    }
    host
}

/// Adds `n` unless it is already there.
fn notice(n: ConfigNotice, notices: &mut Vec<ConfigNotice>) {
    if !notices.contains(&n) {
        notices.push(n);
    }
}

fn yes(word: &str) -> bool {
    word.eq_ignore_ascii_case("yes")
}

/// The `%` tokens B12 supports: `%d %h %p %r %u %%`.
struct Tokens<'a> {
    home: &'a str,
    host: &'a str,
    port: &'a str,
    user: &'a str,
    local_user: &'a str,
}

impl Tokens<'_> {
    /// `value` with its tokens expanded, or the first unsupported token.
    fn expand(&self, value: &str) -> Result<String, String> {
        let mut out = String::with_capacity(value.len());
        let mut chars = value.char_indices();
        while let Some((i, c)) = chars.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            match chars.next().map(|(_, c)| c) {
                Some('%') => out.push('%'),
                Some('d') => out.push_str(self.home),
                Some('h') => out.push_str(self.host),
                Some('p') => out.push_str(self.port),
                Some('r') => out.push_str(self.user),
                Some('u') => out.push_str(self.local_user),
                _ => return Err(token_at(value, i)),
            }
        }
        Ok(out)
    }
}

/// The token starting at byte `i` (a `%`), for the notice: `%` and the next character.
fn token_at(value: &str, i: usize) -> String {
    value[i..].chars().take(2).collect()
}

/// An `IdentityFile` value: `~` is `home`, tokens expand, a relative path is under `home` (an
/// app has no shell working directory). `~user` is unsupported.
fn expand_path(raw: &str, home: &Path, tokens: &Tokens<'_>) -> Result<PathBuf, String> {
    let (base, rest) = if raw == "~" {
        (Some(home), "")
    } else if let Some(rest) = raw.strip_prefix("~/") {
        (Some(home), rest)
    } else if raw.starts_with('~') {
        return Err("~".into());
    } else {
        (None, raw)
    };
    let rest = tokens.expand(rest)?;
    Ok(match base {
        Some(home) if rest.is_empty() => home.to_path_buf(),
        Some(home) => home.join(rest),
        None => home.join(rest),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_handles_equals_quotes_and_comments() {
        assert_eq!(tokenize("  # x"), None);
        assert_eq!(tokenize(""), None);
        assert_eq!(
            tokenize("HostName=a.io"),
            Some(("HostName".into(), vec!["a.io".into()]))
        );
        assert_eq!(
            tokenize("User = \"a b\" c # d"),
            Some(("User".into(), vec!["a b".into(), "c".into()]))
        );
        assert_eq!(
            tokenize("IdentityFile \"unterminated"),
            Some(("IdentityFile".into(), vec!["unterminated".into()]))
        );
    }

    #[test]
    fn jump_specs_parse() {
        let parse = |s| JumpSpec::parse(s).map(|j| (j.user, j.host, j.port));
        assert_eq!(parse("j"), Some((None, "j".into(), None)));
        assert_eq!(
            parse("u@j:2"),
            Some((Some("u".into()), "j".into(), Some(2)))
        );
        assert_eq!(
            parse("ssh://u@[::1]:2"),
            Some((Some("u".into()), "::1".into(), Some(2)))
        );
        assert_eq!(parse("::1"), Some((None, "::1".into(), None)));
        assert_eq!(parse("j:0"), None);
        assert_eq!(parse("j:x"), None);
        assert_eq!(parse("@j"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("[::1"), None);
    }

    #[test]
    fn an_unsupported_token_is_reported_whole_and_a_trailing_percent_too() {
        let t = Tokens {
            home: "/h",
            host: "h",
            port: "22",
            user: "u",
            local_user: "l",
        };
        assert_eq!(t.expand("%d/%h:%p/%r/%u/%%"), Ok("/h/h:22/u/l/%".into()));
        assert_eq!(t.expand("a%C"), Err("%C".into()));
        assert_eq!(t.expand("a%"), Err("%".into()));
        assert_eq!(t.expand("%é"), Err("%é".into()));
    }
}
