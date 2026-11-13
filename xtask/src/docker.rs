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
//! - `pin` rewrites every `image:` and `FROM` tag of the compose file and the Dockerfiles as
//!   `tag@sha256:<index digest>` (read from the registry with `docker buildx imagetools
//!   inspect`; no container is touched). `check-pins` fails on any that has no digest.
//!
//! The main checkout is the parent of `git rev-parse --git-common-dir`, so a worktree resolves
//! the same artifacts directory as the main checkout.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};

/// How long `up` waits for every selected service to report healthy.
const WAIT_TIMEOUT_SECS: u32 = 180;
/// The crates whose `#[ignore = "docker"]` tests `test` runs when no `-p` is given.
const DEFAULT_TEST_CRATES: [&str; 5] = [
    "hacivat-driver-pg",
    "hacivat-driver-mysql",
    "hacivat-driver-sqlite",
    "hacivat-session",
    // ## m2b-t16-docker-matrix
    "hacivat-testkit",
];
const COMPOSE_FILE: &str = "docker/m2a/compose.yml";
/// The Dockerfiles whose `FROM` lines are pinned too.
const DOCKERFILES: [&str; 2] = ["docker/m2a/pg-tr/Dockerfile", "docker/m2a/sshd/Dockerfile"];
/// Images that `compose build` produces locally: never pulled, so never pinned.
const LOCAL_IMAGE_PREFIX: &str = "hacivat-m2a-";
const TARGETS_FILE: &str = "docker/m2a/targets.toml";
/// The service whose auto-generated CA certificate the MySQL TLS tests trust.
const CA_SERVICE: &str = "mysql84";
const CA_IN_CONTAINER: &str = "/var/lib/mysql/ca.pem";
const CA_ARTIFACT: &str = "mysql84-ca.pem";

const USAGE: &str = "usage: xtask docker up --set pr|full\n       \
                     xtask docker down\n       \
                     xtask docker status [--set pr|full]\n       \
                     xtask docker test --set pr|full [-p <crate>]...\n       \
                     xtask docker pin\n       \
                     xtask docker check-pins";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Up,
    Down,
    Status,
    Test,
    /// Rewrites every image tag with its digest (needs the network; run by the controller).
    Pin,
    /// Fails on any image or `FROM` without a digest.
    CheckPins,
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
        Some("pin") => Action::Pin,
        Some("check-pins") => Action::CheckPins,
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
        Action::Pin | Action::CheckPins if set.is_some() || !crates.is_empty() => {
            bail!("pin and check-pins take no arguments")
        }
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
    // The SSH bastions are compose services too (`[[ssh_target]]`, Task 16).
    let ssh = doc
        .get("ssh_target")
        .and_then(toml::Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let mut ids = Vec::new();
    for t in targets.iter().chain(ssh) {
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
    let artifacts = artifacts_dir(&main_checkout()?);
    std::fs::create_dir_all(&artifacts)
        .with_context(|| format!("creating {}", artifacts.display()))?;
    let status = Command::new("docker")
        .args(up_args(&compose, &services))
        .env("HV_ARTIFACTS", &artifacts)
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
        .env("HV_ARTIFACTS", artifacts_dir(&main_checkout()?))
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

// ## m2b-t16-docker-matrix

/// The image reference of an `image:` line of a compose file.
fn compose_ref(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("image:")?.trim();
    let reference = rest.split_whitespace().next()?;
    Some(reference.trim_matches(['"', '\'']))
}

/// The base-image reference of a `FROM [--flag…] <ref> [AS name]` line.
fn dockerfile_ref(line: &str) -> Option<&str> {
    let mut words = line.split_whitespace();
    if !words.next()?.eq_ignore_ascii_case("from") {
        return None;
    }
    words.find(|w| !w.starts_with("--"))
}

/// The reference on a pinnable line that still needs a digest.
fn needs_pin(line: &str) -> Option<&str> {
    let reference = compose_ref(line).or_else(|| dockerfile_ref(line))?;
    let local = reference.starts_with(LOCAL_IMAGE_PREFIX) || reference == "scratch";
    (!local && !reference.contains("@sha256:")).then_some(reference)
}

/// The `image:` tags of a compose file that have no `@sha256:` digest.
pub fn unpinned(compose: &str) -> Vec<String> {
    compose
        .lines()
        .filter(|l| compose_ref(l).is_some())
        .filter_map(needs_pin)
        .map(str::to_string)
        .collect()
}

/// The `FROM` bases of a Dockerfile that have no `@sha256:` digest.
pub fn unpinned_dockerfile(dockerfile: &str) -> Vec<String> {
    dockerfile
        .lines()
        .filter(|l| dockerfile_ref(l).is_some())
        .filter_map(needs_pin)
        .map(str::to_string)
        .collect()
}

/// `line` with `@<digest>` appended to its image reference (unchanged when it has none, or
/// is already pinned).
pub fn pin_line(line: &str, digest: &str) -> String {
    let Some(reference) = needs_pin(line) else {
        return line.to_string();
    };
    // `reference` is a subslice of `line`: split at its offset.
    let start = reference.as_ptr() as usize - line.as_ptr() as usize;
    let end = start + reference.len();
    format!("{}@{digest}{}", &line[..end], &line[end..])
}

/// Pins every pinnable line of `text`, asking `resolve` once per distinct tag.
pub fn pin_text(text: &str, mut resolve: impl FnMut(&str) -> Result<String>) -> Result<String> {
    let mut cache: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut out = String::with_capacity(text.len() + 256);
    for line in text.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        let pinned = match needs_pin(body) {
            Some(reference) => {
                let digest = match cache.get(reference) {
                    Some(d) => d.clone(),
                    None => {
                        let d = resolve(reference)?;
                        cache.insert(reference.to_string(), d.clone());
                        d
                    }
                };
                pin_line(body, &digest)
            }
            None => body.to_string(),
        };
        out.push_str(&pinned);
        if line.ends_with('\n') {
            out.push('\n');
        }
    }
    Ok(out)
}

/// The digest in the output of `docker buildx imagetools inspect --format '{{json
/// .Manifest.Digest}}'`: a JSON string `"sha256:…"`.
pub fn parse_digest(output: &str) -> Result<String> {
    let digest = output.trim().trim_matches('"');
    let hex = digest.strip_prefix("sha256:").unwrap_or_default();
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("unexpected digest output {:?}", output.trim());
    }
    Ok(digest.to_string())
}

/// The index (multi-platform) digest of `tag`, read from its registry; no container is touched.
fn resolve_digest(tag: &str) -> Result<String> {
    let out = Command::new("docker")
        .args(["buildx", "imagetools", "inspect", tag])
        .args(["--format", "{{json .Manifest.Digest}}"])
        .output()
        .context("running docker buildx imagetools inspect")?;
    if !out.status.success() {
        bail!(
            "cannot resolve {tag}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    parse_digest(&String::from_utf8_lossy(&out.stdout)).with_context(|| format!("resolving {tag}"))
}

/// The text of `services.<name>` in `compose` (its header line to the next sibling key).
#[cfg(test)]
pub fn service_block(compose: &str, name: &str) -> Option<String> {
    let header = format!("  {name}:");
    let mut lines = compose.lines().skip_while(|l| l.trim_end() != header);
    let first = lines.next()?;
    let mut block = vec![first];
    for l in lines {
        let top_level = !l.is_empty() && !l.starts_with(' ') && !l.starts_with('#');
        let sibling = l.starts_with("  ") && !l.starts_with("   ") && !l.trim().starts_with('#');
        if top_level || sibling {
            break;
        }
        block.push(l);
    }
    Some(block.join("\n"))
}

fn pinned_files(root: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    std::iter::once(COMPOSE_FILE)
        .chain(DOCKERFILES)
        .map(|p| root.join(p))
}

fn run_pin(root: &Path) -> Result<bool> {
    for path in pinned_files(root) {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let pinned = pin_text(&text, resolve_digest)?;
        if pinned != text {
            std::fs::write(&path, pinned).with_context(|| format!("writing {}", path.display()))?;
            println!("pinned {}", path.display());
        }
    }
    Ok(true)
}

fn run_check_pins(root: &Path) -> Result<bool> {
    let mut bad = Vec::new();
    for path in pinned_files(root) {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let missing = if path.ends_with("compose.yml") {
            unpinned(&text)
        } else {
            unpinned_dockerfile(&text)
        };
        bad.extend(
            missing
                .into_iter()
                .map(|r| format!("{}: {r}", path.display())),
        );
    }
    for b in &bad {
        eprintln!("unpinned: {b}");
    }
    if !bad.is_empty() {
        eprintln!("run `cargo xtask docker pin` (needs network) and commit the digests");
    }
    Ok(bad.is_empty())
}

fn run_inner(args: &[String]) -> Result<bool> {
    let cmd = parse_args(args)?;
    let root = repo_root()?;
    match cmd.action {
        Action::Pin => return run_pin(&root),
        Action::CheckPins => return run_check_pins(&root),
        _ => {}
    }
    ensure_engine()?;
    match cmd.action {
        Action::Pin | Action::CheckPins => unreachable!("handled above"),
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
            "pin --set pr",
            "check-pins -p hacivat-session",
        ] {
            assert!(parse_args(&args(bad)).is_err(), "{bad:?} parsed");
        }
    }

    #[test]
    fn repository_sets_name_the_services() {
        let text = std::fs::read_to_string(crate::workspace_root().join(TARGETS_FILE)).unwrap();
        assert_eq!(
            services_in_set(&text, "pr").unwrap(),
            ["pg17", "mysql84", "sshd-a", "sshd-b"]
        );
        let full = services_in_set(&text, "full").unwrap();
        // ## m2b-fix-f2-x509: and mysql84-x509 (its certificate generator is no target).
        assert_eq!(full.len(), 17);
        assert!(full.iter().any(|s| s == CA_SERVICE));
        assert!(full.iter().any(|s| s == "mysql84-x509"));
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
             -p hacivat-session -p hacivat-testkit -- --ignored"
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

    // ## m2b-t16-docker-matrix
    fn read(rel: &str) -> String {
        std::fs::read_to_string(crate::workspace_root().join(rel)).unwrap()
    }

    #[test]
    fn check_pins_flags_unpinned_lines() {
        assert_eq!(
            unpinned("services:\n a:\n  image: postgres:17\n b:\n  image: mysql:8.4@sha256:0123\n"),
            ["postgres:17"]
        );
        assert_eq!(unpinned_dockerfile("FROM alpine:3.20\n"), ["alpine:3.20"]);
        // A locally built image is not pulled; `scratch` and pinned bases are fine.
        assert!(unpinned("  image: hacivat-m2a-sshd:2\n").is_empty());
        assert!(
            unpinned_dockerfile("FROM scratch\nFROM --platform=x a:1@sha256:9 AS b\n").is_empty()
        );
        assert_eq!(unpinned_dockerfile("FROM --platform=x a:1 AS b\n"), ["a:1"]);
    }

    #[test]
    fn pin_rewrites_tag_with_digest() {
        assert_eq!(
            pin_line("    image: postgres:17", "sha256:abcd"),
            "    image: postgres:17@sha256:abcd"
        );
        assert_eq!(
            pin_line("FROM alpine:3.20", "sha256:ef"),
            "FROM alpine:3.20@sha256:ef"
        );
        assert_eq!(
            pin_line("FROM --platform=p alpine:3.20 AS b", "sha256:ef"),
            "FROM --platform=p alpine:3.20@sha256:ef AS b"
        );
        assert_eq!(
            pin_line("  image: mysql:8.4@sha256:0123", "sha256:ef"),
            "  image: mysql:8.4@sha256:0123"
        );
    }

    #[test]
    fn pin_text_resolves_each_tag_once_and_skips_local_images() {
        let mut asked = Vec::new();
        let out = pin_text(
            "  image: postgres:17\n  image: postgres:17\n  image: hacivat-m2a-sshd:2\n  x: y\n",
            |tag| {
                asked.push(tag.to_string());
                Ok("sha256:aa".to_string())
            },
        )
        .unwrap();
        assert_eq!(
            out,
            "  image: postgres:17@sha256:aa\n  image: postgres:17@sha256:aa\n  image: hacivat-m2a-sshd:2\n  x: y\n"
        );
        assert_eq!(asked, ["postgres:17"]);
    }

    #[test]
    fn digest_comes_from_the_json_output() {
        let good = format!("sha256:{}", "ab12".repeat(16));
        assert_eq!(parse_digest(&format!("\"{good}\"\n")).unwrap(), good);
        assert!(parse_digest("\"sha256:ab12\"").is_err());
        assert!(parse_digest("not json").is_err());
        assert!(parse_digest("\"latest\"").is_err());
    }

    #[test]
    fn pr_set_includes_sshd() {
        let ids = services_in_set(&read(TARGETS_FILE), "pr").unwrap();
        for id in ["pg17", "mysql84", "sshd-a", "sshd-b"] {
            assert!(ids.contains(&id.to_string()), "{id}");
        }
    }

    #[test]
    fn repo_compose_and_dockerfiles_are_pinned() {
        assert!(unpinned(&read(COMPOSE_FILE)).is_empty());
        for d in ["docker/m2a/pg-tr/Dockerfile", "docker/m2a/sshd/Dockerfile"] {
            assert!(unpinned_dockerfile(&read(d)).is_empty(), "{d}");
        }
    }

    #[test]
    fn sshd_b_is_not_published() {
        let compose = read(COMPOSE_FILE);
        let b = service_block(&compose, "sshd-b").unwrap();
        assert!(!b.contains("ports:"), "{b}");
        assert!(b.contains("ssh-inner"));
        let a = service_block(&compose, "sshd-a").unwrap();
        assert!(a.contains("127.0.0.1:2222:22"), "{a}");
        assert!(service_block(&compose, "nope").is_none());
    }

    #[test]
    fn pg_init_never_uses_noinherit() {
        assert!(
            !read("docker/m2a/init/pg.sql")
                .to_ascii_uppercase()
                .contains("NOINHERIT")
        );
    }

    #[test]
    fn pin_and_check_pins_parse() {
        assert_eq!(parse_args(&args("pin")).unwrap().action, Action::Pin);
        assert_eq!(
            parse_args(&args("check-pins")).unwrap().action,
            Action::CheckPins
        );
    }

    /// The sshd entrypoint run twice against a temp directory (a container restart): the
    /// second start must not die on the existing host key, and must publish a fresh one.
    #[cfg(unix)]
    #[test]
    fn sshd_entrypoint_survives_a_restart() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (bin, ssh, art, home) = (
            root.join("bin"),
            root.join("ssh"),
            root.join("artifacts"),
            root.join("home"),
        );
        for d in [&bin, &ssh, &art, &home] {
            std::fs::create_dir_all(d).unwrap();
        }
        let stub = |name: &str, body: &str| {
            let path = bin.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        // Like the real tool: refuses to overwrite an existing key file.
        stub(
            "ssh-keygen",
            "while [ $# -gt 1 ]; do [ \"$1\" = -f ] && f=\"$2\"; shift; done\n\
             [ -e \"$f\" ] && exit 1\n\
             echo \"key-$$-$(date +%s%N)\" > \"$f\"; echo \"pub-$$-$(date +%s%N)\" > \"$f.pub\"",
        );
        stub("chown", "exit 0");
        stub("sshd", "exit 0");
        for n in ["ssh-client-ed25519.pub", "ssh-client-ed25519-enc.pub"] {
            std::fs::write(art.join(n), "client-key\n").unwrap();
        }
        let scripts = crate::workspace_root().join("docker/m2a/sshd");
        let path = format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let start = || {
            Command::new("sh")
                .arg(scripts.join("entrypoint.sh"))
                .env("PATH", &path)
                .env("HV_SSHD_ID", "sshd-t")
                .env("HV_SSH_DIR", &ssh)
                .env("HV_ARTIFACTS", &art)
                .env("HV_HOME_SSH", &home)
                .env("HV_SSHD_BIN", bin.join("sshd"))
                .output()
                .unwrap()
        };
        let first = start();
        assert!(first.status.success(), "{first:?}");
        let pub1 = std::fs::read_to_string(art.join("ssh-host-sshd-t.pub")).unwrap();
        let second = start();
        assert!(second.status.success(), "{second:?}");
        let pub2 = std::fs::read_to_string(art.join("ssh-host-sshd-t.pub")).unwrap();
        assert_ne!(pub1, pub2, "the restart publishes a fresh host key");
        assert_eq!(
            pub2,
            std::fs::read_to_string(ssh.join("ssh_host_ed25519_key.pub")).unwrap()
        );
        let auth = std::fs::read_to_string(home.join("authorized_keys")).unwrap();
        assert_eq!(auth.lines().count(), 2);
    }
}
