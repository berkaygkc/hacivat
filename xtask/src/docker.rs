//! `xtask docker up|down|status|test --set pr|full [-p <crate>…]`: the M2a Docker server
//! matrix (R17, P25).
//!
//! - `up` starts the selected services of `docker/m2a/compose.yml` (project `hacivat-m2a`),
//!   waits until every one is healthy (180 s), then copies `mysql84`'s `ca.pem` to
//!   `<main checkout>/target/docker/mysql84-ca.pem`. Only the controller (and CI) runs it.
//! - `down` stops the project and removes its containers and anonymous volumes.
//! - `status` prints each selected service's health; it fails when one is not healthy.
//! - `test` never runs `up` or `down`: when a selected service is not healthy it fails fast
//!   and names it. Otherwise it runs the Docker tests (`cargo test … -- --ignored`) with
//!   `HACIVAT_DOCKER=1`, `HACIVAT_DOCKER_SET` and `HACIVAT_DOCKER_ARTIFACTS`.
//!
//! The main checkout is the parent of `git rev-parse --git-common-dir`, so a worktree resolves
//! the same artifacts directory as the main checkout.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};

/// How long `up` waits for every selected service to report healthy.
const WAIT_TIMEOUT_SECS: u32 = 180;
/// The crates whose `#[ignore = "docker"]` tests `test` runs when no `-p` is given.
const DEFAULT_TEST_CRATES: [&str; 4] = [
    "hacivat-driver-pg",
    "hacivat-driver-mysql",
    "hacivat-driver-sqlite",
    "hacivat-session",
];
const COMPOSE_FILE: &str = "docker/m2a/compose.yml";
const TARGETS_FILE: &str = "docker/m2a/targets.toml";
/// The service whose auto-generated CA certificate the MySQL TLS tests trust.
const CA_SERVICE: &str = "mysql84";
const CA_IN_CONTAINER: &str = "/var/lib/mysql/ca.pem";
const CA_ARTIFACT: &str = "mysql84-ca.pem";

const USAGE: &str = "usage: xtask docker up --set pr|full\n       \
                     xtask docker down\n       \
                     xtask docker status [--set pr|full]\n       \
                     xtask docker test --set pr|full [-p <crate>]...";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Up,
    Down,
    Status,
    Test,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Cmd {
    pub action: Action,
    /// `None` only for `down` (and `status`, which then means `full`).
    pub set: Option<String>,
    /// `-p` crates for `test`; empty means [`DEFAULT_TEST_CRATES`].
    pub crates: Vec<String>,
}

/// Parses the arguments after `docker`.
pub fn parse_args(args: &[String]) -> Result<Cmd> {
    let mut it = args.iter();
    let action = match it.next().map(String::as_str) {
        Some("up") => Action::Up,
        Some("down") => Action::Down,
        Some("status") => Action::Status,
        Some("test") => Action::Test,
        other => bail!("unknown docker command {other:?}"),
    };
    let mut set = None;
    let mut crates = Vec::new();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--set" => {
                let value = it.next().context("--set needs a value (pr or full)")?;
                if value != "pr" && value != "full" {
                    bail!("unknown set {value:?}: expected pr or full");
                }
                set = Some(value.clone());
            }
            "-p" | "--package" => {
                let value = it.next().context("-p needs a crate name")?;
                crates.push(value.clone());
            }
            other => bail!("unexpected argument {other:?}"),
        }
    }
    match action {
        Action::Up | Action::Test if set.is_none() => bail!("--set pr|full is required"),
        Action::Down if set.is_some() => bail!("down stops the whole project; it takes no --set"),
        Action::Up | Action::Down | Action::Status if !crates.is_empty() => {
            bail!("-p is only for `docker test`")
        }
        _ => {}
    }
    Ok(Cmd {
        action,
        set,
        crates,
    })
}

/// The ids of the services in `set`, in `targets.toml` order.
pub fn services_in_set(targets_toml: &str, set: &str) -> Result<Vec<String>> {
    let doc: toml::Table = toml::from_str(targets_toml).context("targets.toml does not parse")?;
    let targets = doc
        .get("target")
        .and_then(toml::Value::as_array)
        .context("targets.toml has no [[target]] entries")?;
    let mut ids = Vec::new();
    for t in targets {
        let id = t
            .get("id")
            .and_then(toml::Value::as_str)
            .context("a target has no id")?;
        let sets = t
            .get("sets")
            .and_then(toml::Value::as_array)
            .with_context(|| format!("target {id} has no sets"))?;
        if sets.iter().any(|s| s.as_str() == Some(set)) {
            ids.push(id.to_string());
        }
    }
    if ids.is_empty() {
        bail!("no target is in the set {set:?}");
    }
    Ok(ids)
}

/// The container of a service (P25).
pub fn container_name(service: &str) -> String {
    format!("m2a-{service}")
}

/// A container's state as `docker inspect` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Health {
    Healthy,
    /// Any other health or container state (`starting`, `unhealthy`, `exited`, …).
    Not(String),
    /// No such container.
    Missing,
}

/// Reads the output of `docker inspect -f '{{if .State.Health}}…'` (`None`: inspect failed).
pub fn classify(inspect: Option<&str>) -> Health {
    match inspect.map(str::trim) {
        None | Some("") => Health::Missing,
        Some("healthy") => Health::Healthy,
        Some(other) => Health::Not(other.to_string()),
    }
}

/// The services that are not healthy, in order.
pub fn unhealthy(statuses: &[(String, Health)]) -> Vec<String> {
    statuses
        .iter()
        .filter(|(_, h)| *h != Health::Healthy)
        .map(|(id, _)| id.clone())
        .collect()
}

/// The main checkout from `git rev-parse --git-common-dir` (relative to `cwd` when relative).
pub fn main_checkout_from(common_dir: &str, cwd: &Path) -> PathBuf {
    let common = Path::new(common_dir.trim());
    let common = if common.is_absolute() {
        common.to_path_buf()
    } else {
        cwd.join(common)
    };
    common.parent().map(Path::to_path_buf).unwrap_or(common)
}

/// `<main checkout>/target/docker`.
pub fn artifacts_dir(main_checkout: &Path) -> PathBuf {
    main_checkout.join("target").join("docker")
}

/// The `cargo` arguments of `docker test`.
pub fn test_args(crates: &[String]) -> Vec<String> {
    let mut args = vec!["test".to_string()];
    let chosen: Vec<String> = if crates.is_empty() {
        DEFAULT_TEST_CRATES.iter().map(|c| c.to_string()).collect()
    } else {
        crates.to_vec()
    };
    for c in chosen {
        args.push("-p".to_string());
        args.push(c);
    }
    args.push("--".to_string());
    args.push("--ignored".to_string());
    args
}

/// The `docker compose` arguments of `up` for `services`.
pub fn up_args(compose_file: &Path, services: &[String]) -> Vec<String> {
    let mut args = vec![
        "compose".to_string(),
        "-f".to_string(),
        compose_file.display().to_string(),
        "up".to_string(),
        "--detach".to_string(),
        "--wait".to_string(),
        "--wait-timeout".to_string(),
        WAIT_TIMEOUT_SECS.to_string(),
    ];
    args.extend(services.iter().cloned());
    args
}

fn git(args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The checkout (or worktree) xtask runs in: its compose file and bind mounts.
fn repo_root() -> Result<PathBuf> {
    Ok(PathBuf::from(git(&["rev-parse", "--show-toplevel"])?))
}

fn main_checkout() -> Result<PathBuf> {
    let common = git(&["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    let cwd = std::env::current_dir().context("reading the current directory")?;
    Ok(main_checkout_from(&common, &cwd))
}

fn selected_services(root: &Path, set: &str) -> Result<Vec<String>> {
    let path = root.join(TARGETS_FILE);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    services_in_set(&text, set)
}

/// Why the Docker engine cannot be used, from the outcome of `docker info`: `Err(kind)` when
/// the binary could not be spawned, else `(exit success, stderr)`. `None`: the engine answers.
/// Without this check every service would read as `missing`, and the "run `docker up`"
/// advice would mislead.
pub fn engine_problem(info: Result<(bool, &str), std::io::ErrorKind>) -> Option<String> {
    const NOT_RUNNING: &str = "Docker is not running or not installed";
    match info {
        Ok((true, _)) => None,
        Err(std::io::ErrorKind::NotFound) => {
            Some(format!("{NOT_RUNNING}: no `docker` binary on PATH"))
        }
        Err(kind) => Some(format!(
            "{NOT_RUNNING}: could not run `docker info` ({kind})"
        )),
        Ok((false, stderr)) => match stderr.lines().map(str::trim).find(|l| !l.is_empty()) {
            Some(detail) => Some(format!("{NOT_RUNNING}: {detail}")),
            None => Some(format!("{NOT_RUNNING}: `docker info` failed")),
        },
    }
}

/// Bails with [`engine_problem`]'s message unless the Docker engine answers `docker info`.
fn ensure_engine() -> Result<()> {
    let out = Command::new("docker")
        .args(["info", "--format", "{{.ServerVersion}}"])
        .output();
    let problem = match &out {
        Ok(o) => engine_problem(Ok((
            o.status.success(),
            &String::from_utf8_lossy(&o.stderr),
        ))),
        Err(e) => engine_problem(Err(e.kind())),
    };
    match problem {
        Some(message) => bail!("{message}"),
        None => Ok(()),
    }
}

fn health_of(service: &str) -> Health {
    let out = Command::new("docker")
        .args([
            "inspect",
            "--format",
            "{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}",
            &container_name(service),
        ])
        .output();
    match out {
        Ok(o) if o.status.success() => classify(Some(&String::from_utf8_lossy(&o.stdout))),
        _ => classify(None),
    }
}

fn statuses(services: &[String]) -> Vec<(String, Health)> {
    services.iter().map(|s| (s.clone(), health_of(s))).collect()
}

fn print_statuses(statuses: &[(String, Health)]) {
    for (id, health) in statuses {
        let shown = match health {
            Health::Healthy => "healthy".to_string(),
            Health::Not(state) => state.clone(),
            Health::Missing => "missing".to_string(),
        };
        println!("{:<14} {:<18} {shown}", id, container_name(id));
    }
}

fn run_status(statuses: &[(String, Health)]) -> Result<bool> {
    print_statuses(statuses);
    Ok(unhealthy(statuses).is_empty())
}

fn run_up(root: &Path, set: &str) -> Result<bool> {
    let services = selected_services(root, set)?;
    let compose = root.join(COMPOSE_FILE);
    let status = Command::new("docker")
        .args(up_args(&compose, &services))
        .status()
        .context("running docker compose up")?;
    let now = statuses(&services);
    if !status.success() {
        print_statuses(&now);
        bail!(
            "docker compose up failed; not healthy: {}",
            unhealthy(&now).join(", ")
        );
    }
    let bad = unhealthy(&now);
    if !bad.is_empty() {
        print_statuses(&now);
        bail!("not healthy after up: {}", bad.join(", "));
    }
    if services.iter().any(|s| s == CA_SERVICE) {
        let dir = artifacts_dir(&main_checkout()?);
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let dest = dir.join(CA_ARTIFACT);
        let from = format!("{}:{CA_IN_CONTAINER}", container_name(CA_SERVICE));
        let ok = Command::new("docker")
            .args(["cp", &from, &dest.display().to_string()])
            .status()
            .context("running docker cp")?
            .success();
        if !ok {
            bail!("docker cp {from} failed");
        }
        println!("copied {from} to {}", dest.display());
    }
    print_statuses(&now);
    Ok(true)
}

fn run_down(root: &Path) -> Result<bool> {
    let compose = root.join(COMPOSE_FILE);
    let status = Command::new("docker")
        .args(["compose", "-f", &compose.display().to_string()])
        .args(["down", "--volumes", "--remove-orphans"])
        .status()
        .context("running docker compose down")?;
    Ok(status.success())
}

/// Runs the Docker tests of `crates` against the healthy `set`.
///
/// `-- --ignored` runs every `#[ignore]` test of those crates, not only the
/// `#[ignore = "docker"]` ones, so a Docker test must also gate itself on
/// `hacivat_testkit::docker_enabled()`.
fn run_test(root: &Path, set: &str, crates: &[String]) -> Result<bool> {
    let services = selected_services(root, set)?;
    let now = statuses(&services);
    let bad = unhealthy(&now);
    if !bad.is_empty() {
        print_statuses(&now);
        bail!(
            "not healthy: {}. `docker test` never starts or restarts services; \
             the controller runs `cargo xtask docker up --set {set}`",
            bad.join(", ")
        );
    }
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let artifacts = artifacts_dir(&main_checkout()?);
    let status = Command::new(cargo)
        .args(test_args(crates))
        .env("HACIVAT_DOCKER", "1")
        .env("HACIVAT_DOCKER_SET", set)
        .env("HACIVAT_DOCKER_ARTIFACTS", &artifacts)
        .status()
        .context("running cargo test")?;
    Ok(status.success())
}

fn run_inner(args: &[String]) -> Result<bool> {
    let cmd = parse_args(args)?;
    let root = repo_root()?;
    ensure_engine()?;
    match cmd.action {
        Action::Up => run_up(&root, cmd.set.as_deref().unwrap_or("full")),
        Action::Down => run_down(&root),
        Action::Status => {
            let services = selected_services(&root, cmd.set.as_deref().unwrap_or("full"))?;
            run_status(&statuses(&services))
        }
        Action::Test => run_test(&root, cmd.set.as_deref().unwrap_or("full"), &cmd.crates),
    }
}

pub fn run() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(2).collect();
    match run_inner(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(err) => {
            eprintln!("docker: {err:#}\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn parses_each_command() {
        assert_eq!(
            parse_args(&args("up --set full")).unwrap(),
            Cmd {
                action: Action::Up,
                set: Some("full".into()),
                crates: vec![]
            }
        );
        assert_eq!(parse_args(&args("down")).unwrap().action, Action::Down);
        assert_eq!(parse_args(&args("status")).unwrap().set, None);
        assert_eq!(
            parse_args(&args(
                "test --set pr -p hacivat-driver-pg -p hacivat-session"
            ))
            .unwrap(),
            Cmd {
                action: Action::Test,
                set: Some("pr".into()),
                crates: vec!["hacivat-driver-pg".into(), "hacivat-session".into()]
            }
        );
    }

    #[test]
    fn rejects_bad_arguments() {
        for bad in [
            "",
            "restart",
            "up",
            "test",
            "test --set nightly",
            "test --set",
            "test --set pr -p",
            "down --set pr",
            "up --set pr -p hacivat-driver-pg",
            "status --verbose",
        ] {
            assert!(parse_args(&args(bad)).is_err(), "{bad:?} parsed");
        }
    }

    #[test]
    fn repository_sets_name_the_services() {
        let text = std::fs::read_to_string(crate::workspace_root().join(TARGETS_FILE)).unwrap();
        assert_eq!(services_in_set(&text, "pr").unwrap(), ["pg17", "mysql84"]);
        let full = services_in_set(&text, "full").unwrap();
        assert_eq!(full.len(), 13);
        assert!(full.iter().any(|s| s == CA_SERVICE));
        assert!(services_in_set(&text, "nightly").is_err());
    }

    #[test]
    fn health_classification() {
        assert_eq!(classify(Some("healthy\n")), Health::Healthy);
        assert_eq!(classify(Some("starting")), Health::Not("starting".into()));
        assert_eq!(classify(Some("exited")), Health::Not("exited".into()));
        assert_eq!(classify(Some("")), Health::Missing);
        assert_eq!(classify(None), Health::Missing);
        let statuses = vec![
            ("pg17".to_string(), Health::Healthy),
            ("mysql84".to_string(), Health::Not("unhealthy".into())),
            ("pg18".to_string(), Health::Missing),
        ];
        assert_eq!(unhealthy(&statuses), ["mysql84", "pg18"]);
        assert_eq!(container_name("pg17-tls"), "m2a-pg17-tls");
    }

    #[test]
    fn an_unreachable_engine_is_named_not_reported_as_missing_services() {
        assert_eq!(engine_problem(Ok((true, ""))), None);
        let missing = engine_problem(Err(std::io::ErrorKind::NotFound)).unwrap();
        assert!(
            missing.starts_with("Docker is not running or not installed"),
            "{missing}"
        );
        assert!(missing.contains("no `docker` binary"), "{missing}");
        let down = engine_problem(Ok((
            false,
            "\nCannot connect to the Docker daemon at unix:///var/run/docker.sock. \
             Is the docker daemon running?\n",
        )))
        .unwrap();
        assert!(
            down.starts_with("Docker is not running or not installed: Cannot connect"),
            "{down}"
        );
        let silent = engine_problem(Ok((false, ""))).unwrap();
        assert!(silent.contains("`docker info` failed"), "{silent}");
        let denied = engine_problem(Err(std::io::ErrorKind::PermissionDenied)).unwrap();
        assert!(
            denied.starts_with("Docker is not running or not installed"),
            "{denied}"
        );
    }

    #[test]
    fn worktrees_resolve_the_main_checkout() {
        let cwd = Path::new("/repo/.claude/worktrees/a");
        assert_eq!(
            main_checkout_from("/repo/.git\n", cwd),
            PathBuf::from("/repo")
        );
        assert_eq!(
            main_checkout_from(".git", Path::new("/repo")),
            PathBuf::from("/repo")
        );
        assert_eq!(
            artifacts_dir(Path::new("/repo")),
            PathBuf::from("/repo/target/docker")
        );
    }

    #[test]
    fn test_runs_the_ignored_tests_of_the_chosen_crates() {
        assert_eq!(
            test_args(&[]).join(" "),
            "test -p hacivat-driver-pg -p hacivat-driver-mysql -p hacivat-driver-sqlite \
             -p hacivat-session -- --ignored"
        );
        assert_eq!(
            test_args(&["hacivat-session".to_string()]).join(" "),
            "test -p hacivat-session -- --ignored"
        );
    }

    #[test]
    fn up_waits_for_health_with_the_timeout() {
        let a = up_args(
            Path::new("/r/docker/m2a/compose.yml"),
            &args("pg17 mysql84"),
        );
        assert_eq!(
            a.join(" "),
            "compose -f /r/docker/m2a/compose.yml up --detach --wait --wait-timeout 180 pg17 mysql84"
        );
    }
}
