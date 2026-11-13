//! `~/.ssh/config` import (HV-SEC-01, B12, Task 4): host patterns, first-value-wins,
//! `ProxyJump` chains, `%` tokens, `Include` (globs, loops, depth), notices and the secret scan.
//! Every config lives in a temp directory: no test reads the real `~/.ssh`.

use std::fs;
use std::path::{Path, PathBuf};

use hacivat_net::ssh::{
    ConfigIssue, ConfigNotice, HostKeyPolicy, SshAuth, SshConfig, SshConfigHost, read_ssh_config,
    resolve_alias,
};
use tempfile::TempDir;

/// Stands in for `hacivat_driver::url::carries_secret` (the session layer passes the real one;
/// `hacivat-net` cannot depend on the driver crate): a `key=value` pair naming a password.
fn is_secret(v: &str) -> bool {
    let v = v.to_ascii_lowercase();
    ["password=", "passwd=", "pwd="]
        .iter()
        .any(|k| v.contains(k))
}

/// Reads `files` written under `<tmp>/home/.ssh/`, starting at `config`, with `~` = `<tmp>/home`.
fn read_in(files: &[(&str, &str)]) -> (SshConfig, TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let ssh = home.join(".ssh");
    for (name, body) in files {
        let path = ssh.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, body).unwrap();
    }
    let cfg = read_ssh_config(&ssh.join("config"), &home, "me", &is_secret).unwrap();
    (cfg, tmp)
}

fn cfg(body: &str) -> SshConfig {
    read_in(&[("config", body)]).0
}

/// The config read with `~` = `home` (a path that need not exist).
fn cfg_home(body: &str, home: &str) -> SshConfig {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config");
    fs::write(&path, body).unwrap();
    read_ssh_config(&path, Path::new(home), "me", &is_secret).unwrap()
}

fn cfg_files(files: &[(&str, &str)]) -> SshConfig {
    read_in(files).0
}

fn find<'a>(c: &'a SshConfig, alias: &str) -> Option<&'a SshConfigHost> {
    c.hosts.iter().find(|h| h.alias == alias)
}

fn host<'a>(c: &'a SshConfig, alias: &str) -> &'a SshConfigHost {
    find(c, alias).unwrap_or_else(|| panic!("no host {alias} in {c:?}"))
}

fn aliases(c: &SshConfig) -> Vec<&str> {
    c.hosts.iter().map(|h| h.alias.as_str()).collect()
}

/// Copies `tests/fixtures/ssh` to `<tmp>/home/.ssh`.
fn fixture_home() -> (TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ssh");
    copy_dir(&src, &home.join(".ssh"));
    (tmp, home)
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

// The brief's tests (Task 4 Step 1).

#[test]
fn host_patterns_first_value_wins() {
    let c = cfg("Host *.acme.io !secret.acme.io\n User deploy\nHost db.acme.io\n User other\n");
    assert_eq!(host(&c, "db.acme.io").user.as_deref(), Some("deploy"));
    assert!(find(&c, "secret.acme.io").is_none());
}

#[test]
fn proxy_jump_chain_resolves_to_hops() {
    let c = cfg(
        "Host bastion\n HostName b.acme.io\n ProxyJump jump-eu,jump-2\nHost jump-eu\n HostName j.acme.io\n User u1\nHost jump-2\n HostName k.acme.io\n",
    );
    let s = resolve_alias(&c, "bastion").unwrap();
    assert_eq!(
        s.hops.iter().map(|h| h.host.as_str()).collect::<Vec<_>>(),
        ["j.acme.io", "k.acme.io", "b.acme.io"]
    );
}

#[test]
fn identity_file_tokens_expand() {
    let c = cfg_home(
        "Host b\n HostName b.io\n User u\n IdentityFile ~/.ssh/%r@%h_%p\n",
        "/home/me",
    );
    assert_eq!(
        host(&c, "b").identity_files,
        [PathBuf::from("/home/me/.ssh/u@b.io_22")]
    );
}

#[test]
fn unsupported_token_marks_entry_unusable() {
    assert_eq!(
        host(&cfg("Host b\n IdentityFile ~/.ssh/%C\n"), "b").unusable,
        Some(ConfigIssue::UnsupportedToken("%C".into()))
    );
}

#[test]
fn proxycommand_entries_are_listed_but_unusable() {
    assert_eq!(
        host(&cfg("Host p\n ProxyCommand nc %h %p\n"), "p").unusable,
        Some(ConfigIssue::ProxyCommand)
    );
}

#[test]
fn include_loop_stops_at_depth_8() {
    // No hang, no stack overflow.
    let c = cfg_files(&[
        ("config", "Include a\nHost x\n HostName x.io\n"),
        ("a", "Include config\n"),
    ]);
    assert!(
        c.notices
            .iter()
            .any(|n| matches!(n, ConfigNotice::IncludeTooDeep { .. }))
    );
    assert!(find(&c, "x").is_some()); // Q32
}

#[test]
fn hostname_percent_h_expands() {
    // Q43
    assert_eq!(
        host(&cfg("Host db\n HostName %h.acme.io\n"), "db")
            .hostname
            .as_deref(),
        Some("db.acme.io")
    );
}

#[test]
fn identity_agent_is_noted_not_imported() {
    // Q43
    assert!(
        cfg("Host b\n IdentityAgent ~/.1password/agent.sock\n")
            .notices
            .iter()
            .any(|n| matches!(n, ConfigNotice::IdentityAgentIgnored { alias } if alias == "b"))
    );
}

#[test]
fn match_blocks_are_skipped_with_a_notice() {
    assert!(matches!(
        cfg("Match host x\n User y\n").notices[0],
        ConfigNotice::MatchSkipped { line: 1 }
    ));
}

#[test]
fn secret_looking_values_are_refused() {
    assert_eq!(
        host(
            &cfg("Host b\n HostName h\n IdentityFile /k?password=x\n"),
            "b"
        )
        .unusable,
        Some(ConfigIssue::SecretInValue)
    );
}

#[test]
fn strict_host_key_checking_never_maps_to_accept_all() {
    for (v, p) in [
        ("yes", HostKeyPolicy::Strict),
        ("accept-new", HostKeyPolicy::AcceptNew),
        ("no", HostKeyPolicy::Ask),
        ("off", HostKeyPolicy::Ask),
        ("ask", HostKeyPolicy::Ask),
    ] {
        assert_eq!(
            host(&cfg(&format!("Host b\n StrictHostKeyChecking {v}\n")), "b")
                .strict_host_key_checking,
            Some(p)
        );
    }
}

// Beyond the brief.

#[test]
fn a_negated_pattern_excludes_its_host_from_the_block() {
    let c = cfg(
        "Host *.acme.io !secret.acme.io\n User deploy\nHost db.acme.io secret.acme.io\n Port 2200\n",
    );
    assert_eq!(host(&c, "db.acme.io").user.as_deref(), Some("deploy"));
    // The block does not apply: the user stays OpenSSH's default, the local user.
    assert_eq!(host(&c, "secret.acme.io").user.as_deref(), Some("me"));
    assert_eq!(host(&c, "secret.acme.io").port, Some(2200));
    // Wildcard and negated patterns are not hosts of their own.
    assert_eq!(aliases(&c), ["db.acme.io", "secret.acme.io"]);
}

#[test]
fn patterns_match_ascii_case_insensitively_and_question_marks_match_one_char() {
    let c = cfg("Host DB?.ACME.io\n User deploy\nHost db1.acme.io db10.acme.io\n Port 1\n");
    assert_eq!(host(&c, "db1.acme.io").user.as_deref(), Some("deploy"));
    assert_eq!(host(&c, "db10.acme.io").user.as_deref(), Some("me"));
}

#[test]
fn the_fixture_imports_with_include_globs_and_host_star_last() {
    let (_tmp, home) = fixture_home();
    let c = read_ssh_config(&home.join(".ssh/config"), &home, "me", &is_secret).unwrap();
    // Included hosts come first, in lexical file order; README.txt is not a `*.conf` file.
    assert_eq!(aliases(&c), ["jump-eu", "reports", "db-eu", "legacy"]);
    let db = host(&c, "db-eu");
    assert_eq!(db.hostname.as_deref(), Some("db.eu.acme.io"));
    assert_eq!(db.port, Some(2222));
    assert_eq!(db.user.as_deref(), Some("deploy"));
    assert_eq!(db.identity_files, [home.join(".ssh/id_ed25519")]);
    assert!(db.identities_only);
    assert_eq!(db.proxy_jump, ["jump-eu"]);
    assert_eq!(db.server_alive_interval, Some(30));
    assert_eq!(db.strict_host_key_checking, Some(HostKeyPolicy::AcceptNew));
    assert_eq!(db.unusable, None);
    assert_eq!(host(&c, "jump-eu").user.as_deref(), Some("hop"));
    assert_eq!(
        host(&c, "reports").identity_files,
        [home.join(".ssh/my key")]
    );
    assert_eq!(
        host(&c, "reports").hostname.as_deref(),
        Some("reports.acme.io")
    );
    assert_eq!(host(&c, "legacy").unusable, Some(ConfigIssue::ProxyCommand));
    assert!(c.notices.is_empty(), "{:?}", c.notices);

    let spec = resolve_alias(&c, "db-eu").unwrap();
    let hops: Vec<_> = spec
        .hops
        .iter()
        .map(|h| (h.host.as_str(), h.port, h.user.as_str(), &h.auth))
        .collect();
    assert_eq!(
        hops,
        [
            ("j.eu.acme.io", 22, "hop", &SshAuth::Agent),
            (
                "db.eu.acme.io",
                2222,
                "deploy",
                &SshAuth::KeyFile {
                    path: home.join(".ssh/id_ed25519")
                }
            ),
        ]
    );
    assert!(spec.hops[1].identities_only);
    // db-eu says accept-new but jump-eu sets nothing (Ask): the chain takes the stricter.
    assert_eq!(spec.host_keys, HostKeyPolicy::Ask);
    assert_eq!(spec.keepalive_s, Some(30));
    assert_eq!(
        resolve_alias(&c, "legacy").unwrap_err(),
        ConfigIssue::ProxyCommand
    );
}

#[test]
fn crlf_comments_and_equals_signs_parse() {
    let c = cfg(
        "# comment\r\nHost b   # trailing comment\r\n  HostName=b.acme.io\r\n  Port = 2022\r\n\tUser \"dev ops\"\r\n",
    );
    let b = host(&c, "b");
    assert_eq!(b.hostname.as_deref(), Some("b.acme.io"));
    assert_eq!(b.port, Some(2022));
    assert_eq!(b.user.as_deref(), Some("dev ops"));
}

#[test]
fn keywords_are_case_insensitive() {
    let c = cfg("HOST b\n hostname b.acme.io\n PORT 2022\n");
    assert_eq!(host(&c, "b").hostname.as_deref(), Some("b.acme.io"));
    assert_eq!(host(&c, "b").port, Some(2022));
}

#[test]
fn global_lines_before_any_host_apply_to_every_host() {
    let c = cfg("User global\nHost a\n User mine\nHost b\n HostName b.io\n");
    assert_eq!(host(&c, "a").user.as_deref(), Some("global"));
    assert_eq!(host(&c, "b").user.as_deref(), Some("global"));
}

#[test]
fn identity_files_accumulate_and_expand_every_token() {
    let c = cfg_home(
        "Host b\n HostName b.io\n Port 2200\n IdentityFile %d/k_%u_%r\n IdentityFile /keys/100%%\nHost *\n IdentityFile ~/.ssh/id_ed25519\n",
        "/home/me",
    );
    assert_eq!(
        host(&c, "b").identity_files,
        [
            PathBuf::from("/home/me/k_me_me"),
            PathBuf::from("/keys/100%"),
            PathBuf::from("/home/me/.ssh/id_ed25519"),
        ]
    );
}

#[test]
fn an_unsupported_token_in_hostname_marks_the_entry_unusable() {
    assert_eq!(
        host(&cfg("Host b\n HostName %L.acme.io\n"), "b").unusable,
        Some(ConfigIssue::UnsupportedToken("%L".into()))
    );
}

#[test]
fn proxy_jump_accepts_user_host_port_and_ssh_uris() {
    let c = cfg(
        "Host db\n HostName db.acme.io\n ProxyJump ops@j1.acme.io:2201,ssh://root@[2001:db8::1]:2202\n",
    );
    let s = resolve_alias(&c, "db").unwrap();
    let hops: Vec<_> = s
        .hops
        .iter()
        .map(|h| (h.host.as_str(), h.port, h.user.as_str()))
        .collect();
    assert_eq!(
        hops,
        [
            ("j1.acme.io", 2201, "ops"),
            ("2001:db8::1", 2202, "root"),
            ("db.acme.io", 22, "me"),
        ]
    );
}

#[test]
fn an_undeclared_jump_host_still_gets_the_wildcard_blocks() {
    let c = cfg("Host db\n ProxyJump gw.acme.io\nHost *.acme.io\n User ops\n Port 2201\n");
    let s = resolve_alias(&c, "db").unwrap();
    assert_eq!(s.hops[0].host, "gw.acme.io");
    assert_eq!(s.hops[0].user, "ops");
    assert_eq!(s.hops[0].port, 2201);
    assert_eq!(s.hops[1].host, "db");
}

#[test]
fn the_first_jump_follows_its_own_proxy_jump_like_openssh() {
    // `ssh -J j1,j2 a` connects to `j1` through j1's own `ProxyJump`; `j2` is reached with
    // `-J j1`, so its own `ProxyJump` is overridden.
    let c = cfg(
        "Host a\n ProxyJump j1,j2\nHost j1\n ProxyJump j0\nHost j2\n ProxyJump nowhere\nHost j0\n",
    );
    let s = resolve_alias(&c, "a").unwrap();
    assert_eq!(
        s.hops.iter().map(|h| h.host.as_str()).collect::<Vec<_>>(),
        ["j0", "j1", "j2", "a"]
    );
}

#[test]
fn a_proxy_jump_cycle_is_an_error_not_a_hang() {
    let c = cfg("Host a\n ProxyJump b\nHost b\n ProxyJump a\n");
    assert_eq!(
        resolve_alias(&c, "a").unwrap_err(),
        ConfigIssue::IncludeTooDeep
    );
    let c = cfg("Host a\n ProxyJump a\n");
    assert_eq!(
        resolve_alias(&c, "a").unwrap_err(),
        ConfigIssue::IncludeTooDeep
    );
}

#[test]
fn proxy_jump_none_and_proxy_command_none_disable_the_proxy() {
    let c = cfg("Host a\n ProxyJump none\n ProxyCommand nc %h %p\nHost b\n ProxyCommand none\n");
    assert_eq!(host(&c, "a").unusable, None);
    assert_eq!(resolve_alias(&c, "a").unwrap().hops.len(), 1);
    assert_eq!(host(&c, "b").unusable, None);
}

#[test]
fn the_first_of_proxy_jump_and_proxy_command_wins() {
    let c = cfg(
        "Host a\n ProxyJump j\n ProxyCommand nc %h %p\nHost b\n ProxyCommand nc %h %p\n ProxyJump j\n",
    );
    assert_eq!(host(&c, "a").unusable, None);
    assert_eq!(host(&c, "b").unusable, Some(ConfigIssue::ProxyCommand));
    assert!(host(&c, "b").proxy_jump.is_empty());
}

#[test]
fn an_unknown_alias_does_not_resolve() {
    assert_eq!(
        resolve_alias(&cfg("Host a\n"), "zzz").unwrap_err(),
        ConfigIssue::NoHostName
    );
}

#[test]
fn an_unusable_jump_makes_the_chain_unusable() {
    let c = cfg("Host a\n ProxyJump j\nHost j\n ProxyCommand nc %h %p\n");
    assert_eq!(host(&c, "a").unusable, None);
    assert_eq!(
        resolve_alias(&c, "a").unwrap_err(),
        ConfigIssue::ProxyCommand
    );
}

#[test]
fn notices_name_the_alias_never_the_value() {
    let c = cfg(
        "Host a\n StrictHostKeyChecking no\n UserKnownHostsFile /dev/null\n IdentityAgent ~/x.sock\n",
    );
    assert_eq!(
        c.notices,
        [
            ConfigNotice::StrictNoMappedToAsk { alias: "a".into() },
            ConfigNotice::UserKnownHostsIgnored { alias: "a".into() },
            ConfigNotice::IdentityAgentIgnored { alias: "a".into() },
        ]
    );
}

#[test]
fn match_skips_its_lines_until_the_next_host() {
    let c = cfg("Host a\n User u\nMatch user x\n User y\n Include nothing\nHost b\n User z\n");
    assert_eq!(c.notices, [ConfigNotice::MatchSkipped { line: 3 }]);
    assert_eq!(host(&c, "a").user.as_deref(), Some("u"));
    assert_eq!(host(&c, "b").user.as_deref(), Some("z"));
}

#[test]
fn an_include_inside_a_host_block_is_conditional() {
    let c = cfg_files(&[
        ("config", "Host a\n Include inc\nHost b\n HostName b.io\n"),
        ("inc", "User from-inc\nHost c\n Port 2200\n"),
    ]);
    assert_eq!(host(&c, "a").user.as_deref(), Some("from-inc"));
    assert_eq!(host(&c, "b").user.as_deref(), Some("me"));
    // `Host c` sits in a file included under `Host a`: it only applies to a name that matches
    // `a` too, so `c` never gets it and is not listed.
    assert!(find(&c, "c").is_none());
    assert_eq!(aliases(&c), ["a", "b"]);
}

// Fix round 1.

#[test]
fn a_strict_bastion_is_not_weakened_by_a_wildcard_accept_new() {
    // I1: OpenSSH checks the bastion with `yes`; one policy for the chain is the strictest.
    let c = cfg(
        "Host bastion\n StrictHostKeyChecking yes\nHost db\n ProxyJump bastion\nHost *\n StrictHostKeyChecking accept-new\n",
    );
    assert_eq!(
        host(&c, "bastion").strict_host_key_checking,
        Some(HostKeyPolicy::Strict)
    );
    let spec = resolve_alias(&c, "db").unwrap();
    assert_eq!(spec.hops.len(), 2);
    assert_eq!(spec.host_keys, HostKeyPolicy::Strict);
}

#[test]
fn the_chain_policy_is_the_strictest_hop_and_an_unset_hop_counts_as_ask() {
    let policy = |body: &str| resolve_alias(&cfg(body), "db").unwrap().host_keys;
    // Every hop accept-new: AcceptNew.
    assert_eq!(
        policy("Host db\n ProxyJump j\nHost *\n StrictHostKeyChecking accept-new\n"),
        HostKeyPolicy::AcceptNew
    );
    // The jump host sets nothing: it counts as Ask.
    assert_eq!(
        policy("Host db\n ProxyJump j\n StrictHostKeyChecking accept-new\n"),
        HostKeyPolicy::Ask
    );
    // The final host is stricter than its jumps.
    assert_eq!(
        policy(
            "Host db\n ProxyJump j\n StrictHostKeyChecking yes\nHost j\n StrictHostKeyChecking accept-new\n"
        ),
        HostKeyPolicy::Strict
    );
    // A strict hop deep in the first jump's own chain still counts.
    assert_eq!(
        policy(
            "Host db\n ProxyJump j1\nHost j1\n ProxyJump j0\nHost j0\n StrictHostKeyChecking yes\nHost *\n StrictHostKeyChecking accept-new\n"
        ),
        HostKeyPolicy::Strict
    );
}

#[test]
fn a_password_in_a_proxy_jump_uri_is_dropped_and_noted() {
    let c = cfg("Host db\n ProxyJump ssh://admin:hv_test_pw@jump.acme.io:2201,ops:hv_test_pw@j2\n");
    assert!(!format!("{c:?}").contains("hv_test_pw"));
    assert_eq!(
        host(&c, "db").proxy_jump,
        ["ssh://admin@jump.acme.io:2201", "ops@j2"]
    );
    assert_eq!(
        c.notices,
        [ConfigNotice::ProxyJumpPasswordIgnored { alias: "db".into() }]
    );
    let spec = resolve_alias(&c, "db").unwrap();
    assert!(!format!("{spec:?}").contains("hv_test_pw"));
    let hops: Vec<_> = spec
        .hops
        .iter()
        .map(|h| (h.host.as_str(), h.port, h.user.as_str()))
        .collect();
    assert_eq!(
        hops,
        [
            ("jump.acme.io", 2201, "admin"),
            ("j2", 22, "ops"),
            ("db", 22, "me")
        ]
    );
}

#[test]
fn a_keyword_without_a_value_is_ignored() {
    let c = cfg("Host b\n HostName\n User\n Port\nHost *\n HostName fallback.acme.io\n");
    let b = host(&c, "b");
    assert_eq!(b.hostname.as_deref(), Some("fallback.acme.io"));
    assert_eq!(b.user.as_deref(), Some("me"));
    assert_eq!(
        resolve_alias(&c, "b").unwrap().hops[0].host,
        "fallback.acme.io"
    );
}

#[test]
fn a_large_config_imports_in_linear_time() {
    // M1: 20,000 hosts with a wildcard block. Quadratic resolution took ~6.8 s for 8,000 hosts
    // in a debug build (so minutes for this); the bound only catches that regression.
    let mut body = String::new();
    for i in 0..20_000 {
        body.push_str(&format!("Host h{i}\n HostName h{i}.acme.io\n"));
    }
    body.push_str("Host *\n User deploy\n");
    let c = cfg(&body);
    assert_eq!(c.hosts.len(), 20_000);
    assert_eq!(host(&c, "h19999").user.as_deref(), Some("deploy"));
}

// Round 1 re-review.

#[test]
fn a_percent_encoded_proxy_jump_password_is_dropped_too() {
    // R1: OpenSSH URL-decodes the user of an `ssh://` URI, so `%3A` is the same `:`.
    let c = cfg(
        "Host db\n ProxyJump ssh://u%3Ahv_test_pw@j1,SSH://v%3ahv_test_pw@j2,w%3Ahv_test_pw@j3\n",
    );
    let shown = format!("{c:?}");
    assert!(!shown.contains("hv_test_pw"), "{shown}");
    assert!(!shown.contains("%3A") && !shown.contains("%3a"), "{shown}");
    assert_eq!(
        host(&c, "db").proxy_jump,
        ["ssh://u@j1", "SSH://v@j2", "w@j3"]
    );
    assert_eq!(
        c.notices,
        [ConfigNotice::ProxyJumpPasswordIgnored { alias: "db".into() }]
    );
    let spec = resolve_alias(&c, "db").unwrap();
    assert!(!format!("{spec:?}").contains("hv_test_pw"));
    let users: Vec<_> = spec.hops.iter().map(|h| h.user.as_str()).collect();
    assert_eq!(users, ["u", "v", "w", "me"]);
}

#[test]
fn an_empty_or_unknown_strict_host_key_checking_counts_as_ask() {
    // R2: the valueless line takes the first-value slot; a later accept-new never wins.
    for line in [" StrictHostKeyChecking\n", " StrictHostKeyChecking maybe\n"] {
        let c = cfg(&format!(
            "Host db\n{line}Host *\n StrictHostKeyChecking accept-new\n"
        ));
        assert_eq!(
            host(&c, "db").strict_host_key_checking,
            Some(HostKeyPolicy::Ask),
            "{line:?}"
        );
        assert_eq!(
            resolve_alias(&c, "db").unwrap().host_keys,
            HostKeyPolicy::Ask
        );
    }
}

#[test]
fn a_secret_value_is_never_echoed() {
    let c = cfg("Host b\n HostName h\n User admin?password=hv_test_pw\n");
    assert_eq!(host(&c, "b").unusable, Some(ConfigIssue::SecretInValue));
    assert_eq!(host(&c, "b").user.as_deref(), Some("me"));
    assert!(!format!("{c:?}").contains("hv_test_pw"));
}

#[test]
fn a_secret_looking_alias_is_dropped() {
    let c = cfg("Host x?password=hv_test_pw ok\n HostName h\n");
    assert_eq!(aliases(&c), ["ok"]);
    assert!(!format!("{c:?}").contains("hv_test_pw"));
}

#[test]
fn a_secret_in_proxy_jump_is_refused() {
    let c = cfg("Host b\n ProxyJump u:pwd=hv_test_pw@j\n");
    assert_eq!(host(&c, "b").unusable, Some(ConfigIssue::SecretInValue));
    assert!(!format!("{c:?}").contains("hv_test_pw"));
}

#[test]
fn a_missing_include_is_ignored_and_a_missing_config_is_an_error() {
    let c = cfg("Include does-not-exist /nonexistent/*.conf\nHost a\n");
    assert_eq!(aliases(&c), ["a"]);
    assert!(c.notices.is_empty());
    let tmp = tempfile::tempdir().unwrap();
    assert!(read_ssh_config(&tmp.path().join("config"), tmp.path(), "me", &is_secret).is_err());
}

#[test]
fn a_self_including_glob_is_bounded() {
    // Every file includes every file: without a budget this would read 3^8 files.
    let body = "Include d/*\nHost a\n";
    let c = cfg_files(&[
        ("config", "Include d/*\n"),
        ("d/1", body),
        ("d/2", body),
        ("d/3", body),
    ]);
    assert_eq!(aliases(&c), ["a"]);
    assert!(
        c.notices
            .iter()
            .any(|n| matches!(n, ConfigNotice::IncludeTooDeep { .. }))
    );
}

#[test]
fn a_huge_line_and_non_utf8_bytes_do_not_panic() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config");
    let mut body = b"Host big\n User ".to_vec();
    body.extend(std::iter::repeat_n(b'x', 10 << 20));
    body.extend_from_slice(b"\nHost bin\xff\xfe\n HostName ok.acme.io\nHost after\n Port 2200\n");
    fs::write(&path, &body).unwrap();
    let c = read_ssh_config(&path, tmp.path(), "me", &is_secret).unwrap();
    // The 10 MiB line is skipped; the hosts after it are read.
    assert_eq!(host(&c, "big").user.as_deref(), Some("me"));
    assert_eq!(host(&c, "after").port, Some(2200));
    assert_eq!(c.hosts.len(), 3);
}

#[cfg(unix)]
#[test]
fn an_include_of_a_device_or_fifo_is_never_read() {
    let tmp = tempfile::tempdir().unwrap();
    let fifo = tmp.path().join("fifo");
    // `mkfifo` is POSIX; a FIFO with no writer would block a naive reader forever.
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let path = tmp.path().join("config");
    let mut body = String::from("Include /dev/zero\n");
    if made {
        body.push_str(&format!("Include {}\n", fifo.display()));
    }
    body.push_str("Host a\n");
    fs::write(&path, body).unwrap();
    let c = read_ssh_config(&path, tmp.path(), "me", &is_secret).unwrap();
    assert_eq!(aliases(&c), ["a"]);
}

#[cfg(unix)]
#[test]
fn an_include_may_follow_a_symlink_and_only_reads_it() {
    let tmp = tempfile::tempdir().unwrap();
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    let target = outside.join("hosts.conf");
    // Were this executed, it would create `ran`.
    fs::write(&target, "Host linked\n ProxyCommand touch ran\n").unwrap();
    let home = tmp.path().join("home");
    fs::create_dir_all(home.join(".ssh")).unwrap();
    std::os::unix::fs::symlink(&target, home.join(".ssh/linked.conf")).unwrap();
    fs::write(home.join(".ssh/config"), "Include linked.conf\n").unwrap();
    let c = read_ssh_config(&home.join(".ssh/config"), &home, "me", &is_secret).unwrap();
    assert_eq!(host(&c, "linked").unusable, Some(ConfigIssue::ProxyCommand));
    assert!(!tmp.path().join("ran").exists());
    assert!(!home.join(".ssh/ran").exists());
}

#[test]
fn keepalive_zero_keeps_the_tunnel_default() {
    let c = cfg("Host a\n ServerAliveInterval 0\nHost b\n ServerAliveInterval 15\n");
    assert_eq!(resolve_alias(&c, "a").unwrap().keepalive_s, None);
    assert_eq!(resolve_alias(&c, "b").unwrap().keepalive_s, Some(15));
    assert_eq!(
        resolve_alias(&c, "a").unwrap().host_keys,
        HostKeyPolicy::Ask
    );
}
