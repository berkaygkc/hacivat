//! `docker/m2a/targets.toml` and `docker/m2a/compose.yml` describe the same servers (R17, P25).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use hacivat_proto::DbmsKind::{MariaDb as Maria, MySql as My, Postgres as Pg};
use hacivat_testkit::{
    TlsDefault, all_targets_unfiltered, artifact_in, docker_enabled_from, ids_in_set,
    targets_in_set,
};
use yaml_rust2::{Yaml, YamlLoader};

/// One compose service, as far as these tests look at it.
struct Service {
    container_name: String,
    bind: String,
    host_port: u16,
    has_healthcheck: bool,
}

fn compose_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../docker/m2a/compose.yml")
}

fn compose_doc() -> Yaml {
    let text = std::fs::read_to_string(compose_path()).expect("read docker/m2a/compose.yml");
    YamlLoader::load_from_str(&text)
        .expect("compose.yml is YAML")
        .into_iter()
        .next()
        .expect("compose.yml has a document")
}

/// `services.<id>` with its single `bind:host:container` port mapping.
fn compose_services() -> BTreeMap<String, Service> {
    let doc = compose_doc();
    let services = doc["services"].as_hash().expect("services is a mapping");
    services
        .iter()
        .map(|(id, s)| {
            let id = id.as_str().expect("service id").to_string();
            let ports = s["ports"].as_vec().expect("ports is a list");
            assert_eq!(ports.len(), 1, "{id}: one port mapping");
            let mapping = ports[0].as_str().expect("port mapping is a string");
            let parts: Vec<&str> = mapping.split(':').collect();
            assert_eq!(parts.len(), 3, "{id}: bind:host:container, got {mapping}");
            let service = Service {
                container_name: s["container_name"].as_str().unwrap_or_default().to_string(),
                bind: parts[0].to_string(),
                host_port: parts[1].parse().expect("host port"),
                has_healthcheck: !s["healthcheck"]["test"].is_badvalue(),
            };
            (id, service)
        })
        .collect()
}

#[test]
fn targets_and_compose_agree() {
    let compose = compose_services();
    let targets = all_targets_unfiltered();
    assert_eq!(
        targets.len(),
        compose.len(),
        "one target per compose service"
    );
    for t in targets {
        let s = compose
            .get(&t.id)
            .unwrap_or_else(|| panic!("no service {}", t.id));
        assert_eq!(s.host_port, t.port);
        assert_eq!(s.bind, "127.0.0.1");
        assert!(s.has_healthcheck, "{} has no healthcheck", t.id);
        assert_eq!(t.host, "127.0.0.1");
    }
}

#[test]
fn full_set_covers_the_m2a_matrix() {
    let majors: BTreeSet<_> = all_targets_unfiltered()
        .into_iter()
        .map(|t| (t.engine, t.major))
        .collect();
    for (e, m) in [
        (Pg, "14"),
        (Pg, "15"),
        (Pg, "16"),
        (Pg, "17"),
        (Pg, "18"),
        (My, "8.0"),
        (My, "8.4"),
        (Maria, "10.6"),
        (Maria, "10.11"),
        (Maria, "11.4"),
    ] {
        assert!(majors.contains(&(e, m.to_string())), "{e:?} {m}");
    }
    // The unsupported fixture below the PostgreSQL floor.
    assert!(majors.contains(&(Pg, "13".to_string())));
}

#[test]
fn docker_enabled_only_for_one() {
    assert!(!docker_enabled_from(None));
    assert!(docker_enabled_from(Some("1")));
    assert!(!docker_enabled_from(Some("0")));
    assert!(!docker_enabled_from(Some("")));
    assert!(!docker_enabled_from(Some("true")));
}

#[test]
fn compose_names_the_project_and_containers() {
    let c = compose_doc();
    assert_eq!(c["name"].as_str(), Some("hacivat-m2a"));
    let services = compose_services();
    for t in all_targets_unfiltered() {
        assert_eq!(services[&t.id].container_name, format!("m2a-{}", t.id));
    }
}

#[test]
fn pr_set_is_pg17_and_mysql84() {
    assert_eq!(ids_in_set("pr"), ["mysql84", "pg17"]);
}

#[test]
fn full_set_is_every_target() {
    let all: Vec<String> = all_targets_unfiltered().into_iter().map(|t| t.id).collect();
    let mut sorted = all.clone();
    sorted.sort();
    assert_eq!(ids_in_set("full"), sorted);
    assert_eq!(all.len(), 13);
}

#[test]
fn set_filter_and_engine_filter_combine() {
    let pr_pg: Vec<String> = targets_in_set(Some("pr"), Some(Pg))
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(pr_pg, ["pg17"]);
    let full_maria: Vec<String> = targets_in_set(Some("full"), Some(Maria))
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(full_maria, ["mariadb106", "mariadb1011", "mariadb114"]);
    // No set selected: every target.
    assert_eq!(targets_in_set(None, None).len(), 13);
    assert!(targets_in_set(Some("nightly"), None).is_empty());
}

#[test]
fn targets_carry_the_test_database_and_user() {
    for t in all_targets_unfiltered() {
        assert_eq!(t.user, "hv_test", "{}", t.id);
        assert_eq!(t.database, "hv_test", "{}", t.id);
        assert!(!t.password.is_empty(), "{}", t.id);
    }
}

#[test]
fn tls_defaults_are_the_observed_ones() {
    let tls: BTreeMap<String, TlsDefault> = all_targets_unfiltered()
        .into_iter()
        .map(|t| (t.id, t.tls_default))
        .collect();
    assert_eq!(tls["pg17"], TlsDefault::None);
    assert_eq!(tls["pg17-tls"], TlsDefault::SelfSigned);
    assert_eq!(tls["mysql84"], TlsDefault::SelfSigned);
}

#[test]
fn debug_output_redacts_the_password() {
    for t in all_targets_unfiltered() {
        let shown = format!("{t:?}");
        assert!(!shown.contains(&t.password), "{}: password in Debug", t.id);
        assert!(shown.contains(&t.id));
    }
}

#[test]
fn artifact_is_under_the_artifacts_dir() {
    assert_eq!(
        artifact_in(Some("/x/target/docker"), "mysql84-ca.pem"),
        PathBuf::from("/x/target/docker/mysql84-ca.pem")
    );
}

#[test]
#[should_panic(expected = "HACIVAT_DOCKER_ARTIFACTS")]
fn artifact_without_the_dir_panics_clearly() {
    let _ = artifact_in(None, "mysql84-ca.pem");
}
