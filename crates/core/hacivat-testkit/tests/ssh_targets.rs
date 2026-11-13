//! The SSH bastions of `docker/m2a/targets.toml` and compose.yml (Task 16).

use std::path::PathBuf;

use hacivat_testkit::{
    SSH_CLIENT_KEY_PASSPHRASE, ssh_client_key, ssh_client_key_in, ssh_target, ssh_targets_in_set,
};
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

#[test]
fn bastions_are_in_the_pr_and_full_sets() {
    for set in ["pr", "full"] {
        let ids: Vec<String> = ssh_targets_in_set(Some(set))
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ids, ["sshd-a", "sshd-b"], "{set}");
    }
    assert!(ssh_targets_in_set(Some("nightly")).is_empty());
}

#[test]
fn sshd_b_is_reached_through_sshd_a() {
    let all = ssh_targets_in_set(None);
    let a = all.iter().find(|t| t.id == "sshd-a").unwrap();
    let b = all.iter().find(|t| t.id == "sshd-b").unwrap();
    assert_eq!(
        (a.host.as_str(), a.port, a.via.as_deref()),
        ("127.0.0.1", 2222, None)
    );
    assert_eq!(b.via.as_deref(), Some("sshd-a"));
    assert_eq!(b.user, "hv_ssh");
}

#[test]
fn ssh_target_debug_redacts_the_password() {
    for t in ssh_targets_in_set(None) {
        assert!(!format!("{t:?}").contains(&t.password), "{}", t.id);
    }
}

#[test]
fn client_keys_live_under_the_artifacts_dir() {
    assert_eq!(
        ssh_client_key_in(Some("/x"), false),
        PathBuf::from("/x/ssh-client-ed25519")
    );
    assert_eq!(
        ssh_client_key_in(Some("/x"), true),
        PathBuf::from("/x/ssh-client-ed25519-enc")
    );
    assert_eq!(SSH_CLIENT_KEY_PASSPHRASE, "hv_test_pw");
}

#[tokio::test]
#[ignore = "docker"]
async fn ssh_targets_accept_the_generated_key() {
    if !hacivat_testkit::docker_enabled() {
        return;
    }
    let Some(t) = ssh_target("sshd-a") else {
        return;
    };
    let mut s = TcpStream::connect((t.host.as_str(), t.port)).await.unwrap();
    let mut banner = [0u8; 7];
    s.read_exact(&mut banner).await.unwrap();
    assert_eq!(&banner, b"SSH-2.0");
    assert!(ssh_client_key(false).exists());
    assert!(ssh_client_key(true).exists());
}
