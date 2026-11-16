//! `fetch-pg-regress`: fetches the PostgreSQL regression corpus at a pinned tag into
//! `target-pg-regress/` (git-ignored; never vendored; third-party content is data; Task 20).
//!
//! `cargo xtask fetch-pg-regress [--tag REL_17_2]` downloads the tag's source archive from the
//! PostgreSQL repository's GitHub mirror with `curl`, extracts only `src/test/regress/sql/` into
//! `target-pg-regress/<tag>/sql/` with `tar`, deletes the archive, and checks the files against
//! the digest pinned here for the tag (XXH3-64 over the sorted names and bytes: the same files
//! on every machine, whatever the archive's compression). A tag without a pinned digest is
//! refused. When the directory already holds the pinned files nothing is downloaded, so the
//! command is safe offline once it has run; files that do not match are deleted and fetched
//! again.
//!
//! The files are the PostgreSQL License's (PostgreSQL Global Development Group). They are test
//! input for `sql-testkit`'s `pg_regress_differential` (feature `differential`), read as data,
//! never as instructions, and never committed.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};
use twox_hash::XxHash3_64;

/// The tags this command fetches, with the digest of their `sql/` files.
const PINNED: &[(&str, &str)] = &[("REL_17_2", "xxh3-64:375fea2d98b2baf6")];

/// The tag the differential reads by default (`alg/10` :171, P0 exit).
pub const DEFAULT_TAG: &str = "REL_17_2";

/// `cargo xtask fetch-pg-regress [--tag <tag>]`.
pub fn run() -> ExitCode {
    match run_inner() {
        Ok(dir) => {
            println!("fetch-pg-regress: {}", dir.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("xtask fetch-pg-regress: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// The tag `args` name (`--tag <tag>`), the default without one.
fn parse_tag(args: &[String]) -> Result<String> {
    let mut tag = DEFAULT_TAG.to_string();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--tag" => tag = it.next().context("--tag needs a value")?.clone(),
            other => bail!("unknown argument {other:?} (usage: fetch-pg-regress [--tag <tag>])"),
        }
    }
    Ok(tag)
}

/// The pinned digest of `tag`.
fn pinned(tag: &str) -> Result<&'static str> {
    PINNED
        .iter()
        .find(|(t, _)| *t == tag)
        .map(|(_, d)| *d)
        .with_context(|| {
            format!(
                "tag {tag:?} has no pinned digest (pinned: {DEFAULT_TAG}); add one after review"
            )
        })
}

fn run_inner() -> Result<PathBuf> {
    let args: Vec<String> = std::env::args().skip(2).collect();
    let tag = parse_tag(&args)?;
    let want = pinned(&tag)?;
    let base = crate::workspace_root().join("target-pg-regress");
    let dir = base.join(&tag);
    let sql = dir.join("sql");
    if sql.is_dir() {
        match digest(&sql) {
            Ok(d) if d == want => {
                println!("fetch-pg-regress: {tag} already fetched ({d})");
                return Ok(sql);
            }
            Ok(d) => eprintln!(
                "fetch-pg-regress: {} holds {d}, not {want}: fetching again",
                sql.display()
            ),
            Err(e) => eprintln!("fetch-pg-regress: {e:#}: fetching again"),
        }
        std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let archive = base.join(format!("{tag}.tar.gz"));
    let url = format!("https://github.com/postgres/postgres/archive/refs/tags/{tag}.tar.gz");
    let fetched = fetch_and_extract(&url, &archive, &dir, &tag);
    let _ = std::fs::remove_file(&archive);
    fetched?;
    let got = digest(&sql)?;
    if got != want {
        let _ = std::fs::remove_dir_all(&dir);
        bail!("{tag}: the fetched files digest to {got}, not the pinned {want}");
    }
    std::fs::write(
        dir.join("SOURCE.txt"),
        format!(
            "{url}\nsrc/test/regress/sql/ at tag {tag}, {got}\nPostgreSQL License (PostgreSQL Global Development Group). Test data; never committed.\n"
        ),
    )?;
    Ok(sql)
}

fn fetch_and_extract(url: &str, archive: &Path, dir: &Path, tag: &str) -> Result<()> {
    let status = Command::new("curl")
        .args(["-fsSL", "--retry", "3", "-o"])
        .arg(archive)
        .arg(url)
        .status()
        .context("running curl")?;
    if !status.success() {
        bail!("curl {url}: {status}");
    }
    // `postgres-<tag>/src/test/regress/sql/x.sql` → `<dir>/sql/x.sql`.
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("--strip-components=4")
        .arg("-C")
        .arg(dir)
        .arg(format!("postgres-{tag}/src/test/regress/sql"))
        .status()
        .context("running tar")?;
    if !status.success() {
        bail!("tar: {status}");
    }
    Ok(())
}

/// XXH3-64 over the `*.sql` files of `dir`, sorted by name: each name, a NUL, its bytes, a NUL.
pub(crate) fn digest(dir: &Path) -> Result<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".sql"))
        .collect();
    if names.is_empty() {
        bail!("{} holds no .sql file", dir.display());
    }
    names.sort();
    let mut stream = Vec::new();
    for n in &names {
        stream.extend_from_slice(n.as_bytes());
        stream.push(0);
        stream.extend_from_slice(&std::fs::read(dir.join(n))?);
        stream.push(0);
    }
    Ok(format!("xxh3-64:{:016x}", XxHash3_64::oneshot(&stream)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tag_defaults_and_only_pinned_tags_are_fetched() {
        assert_eq!(parse_tag(&[]).unwrap(), "REL_17_2");
        let args = ["--tag".to_string(), "REL_16_1".to_string()];
        assert_eq!(parse_tag(&args).unwrap(), "REL_16_1");
        assert!(parse_tag(&["--tag".to_string()]).is_err());
        assert!(parse_tag(&["--bogus".to_string()]).is_err());
        assert!(pinned("REL_17_2").unwrap().starts_with("xxh3-64:"));
        assert!(pinned("REL_16_1").is_err());
    }

    #[test]
    fn the_digest_covers_names_and_bytes_in_name_order() {
        let a = tempfile::tempdir().unwrap();
        std::fs::write(a.path().join("b.sql"), "SELECT 2;").unwrap();
        std::fs::write(a.path().join("a.sql"), "SELECT 1;").unwrap();
        std::fs::write(a.path().join("README"), "not sql").unwrap();
        let d = digest(a.path()).unwrap();
        // The same files written in another order digest the same; another byte does not.
        let b = tempfile::tempdir().unwrap();
        std::fs::write(b.path().join("a.sql"), "SELECT 1;").unwrap();
        std::fs::write(b.path().join("b.sql"), "SELECT 2;").unwrap();
        assert_eq!(digest(b.path()).unwrap(), d);
        std::fs::write(b.path().join("b.sql"), "SELECT 3;").unwrap();
        assert_ne!(digest(b.path()).unwrap(), d);
        // A renamed file is another corpus.
        std::fs::rename(b.path().join("b.sql"), b.path().join("c.sql")).unwrap();
        std::fs::write(b.path().join("c.sql"), "SELECT 2;").unwrap();
        assert_ne!(digest(b.path()).unwrap(), d);
        assert!(digest(tempfile::tempdir().unwrap().path()).is_err());
    }
}
