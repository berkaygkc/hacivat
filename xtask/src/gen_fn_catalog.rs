//! `gen-fn-catalog --check | --write`: the offline freshness check of `sql-safety`'s function
//! data against `crates/sql/sql-safety/data/functions.digest` (Task 3). The catalog read itself
//! is a Docker test, `hacivat-exec/tests/gen_fn_catalog.rs` (P-M3b-13); xtask has no database
//! client.
//!
//! The digest must equal `sql_safety::functions::data::digest_of_files` (R1-M3): the same files
//! in the same order ([`DIGEST_FILES`], compared with the list between the `digest-files`
//! markers of `sql-safety/src/functions/data.rs` by a test) and the same stream: for each file
//! its path, `\n`, its byte length in decimal, `\n`, its bytes with CRLF read as LF; XXH3-64
//! with seed 0, written `xxh3-64:<16 hex digits>`. xtask takes no SQL crate, so the list and
//! the algorithm are copied, and both crates pin the same known answer in a test.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use twox_hash::XxHash3_64;

/// The files the digest covers, in digest order, relative to the data directory. A copy of
/// `sql_safety::functions::data::DIGEST_FILES`.
pub const DIGEST_FILES: [&str; 21] = [
    "pg/allow.toml",
    "pg/deny.toml",
    "pg/operators.toml",
    "pg/types.toml",
    "pg/set_allow.toml",
    "mysql/manual-index.txt",
    "mysql/allow.toml",
    "mysql/deny.toml",
    "mysql/operators.toml",
    "mysql/types.toml",
    "mysql/set_allow.toml",
    "mariadb/allow.toml",
    "mariadb/deny.toml",
    "mariadb/operators.toml",
    "mariadb/types.toml",
    "mariadb/set_allow.toml",
    "sqlite/allow.toml",
    "sqlite/deny.toml",
    "sqlite/operators.toml",
    "sqlite/types.toml",
    "sqlite/set_allow.toml",
];

/// `crates/sql/sql-safety/data` under `root`.
pub fn data_dir(root: &Path) -> PathBuf {
    root.join("crates/sql/sql-safety/data")
}

/// The digest of `files` (`(path, bytes)` in order).
pub fn digest_of_files<'a>(files: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> String {
    let mut stream = Vec::new();
    for (path, bytes) in files {
        let text = lf(bytes);
        stream.extend_from_slice(path.as_bytes());
        stream.push(b'\n');
        stream.extend_from_slice(text.len().to_string().as_bytes());
        stream.push(b'\n');
        stream.extend_from_slice(&text);
    }
    format!("xxh3-64:{:016x}", XxHash3_64::oneshot(&stream))
}

/// `bytes` with every CRLF read as LF.
fn lf(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            continue;
        }
        out.push(b);
    }
    out
}

/// The digest of the data files under `dir`.
pub fn digest_of_dir(dir: &Path) -> Result<String> {
    let mut files = Vec::with_capacity(DIGEST_FILES.len());
    for f in DIGEST_FILES {
        let path = dir.join(f);
        files.push((
            f,
            std::fs::read(&path).with_context(|| path.display().to_string())?,
        ));
    }
    Ok(digest_of_files(
        files.iter().map(|(f, b)| (*f, b.as_slice())),
    ))
}

/// `--check`: whether `functions.digest` under `dir` holds the data's digest. `--write`:
/// rewrites it.
pub fn run_in(dir: &Path, write: bool) -> Result<bool> {
    let want = digest_of_dir(dir)?;
    let path = dir.join("functions.digest");
    if write {
        std::fs::write(&path, format!("{want}\n")).with_context(|| path.display().to_string())?;
        println!("gen-fn-catalog: wrote {want} to {}", path.display());
        return Ok(true);
    }
    let have = std::fs::read_to_string(&path).with_context(|| path.display().to_string())?;
    if have.trim() == want {
        println!("gen-fn-catalog: {} is fresh ({want})", path.display());
        Ok(true)
    } else {
        eprintln!(
            "gen-fn-catalog: {} is stale: it holds {:?}, the data hashes to {want}. Review the \
             data change, then run `cargo xtask gen-fn-catalog --write`.",
            path.display(),
            have.trim()
        );
        Ok(false)
    }
}

fn parse_args(args: &[String]) -> Result<bool> {
    match args {
        [flag] if flag == "--check" => Ok(false),
        [flag] if flag == "--write" => Ok(true),
        _ => bail!("usage: xtask gen-fn-catalog --check | --write"),
    }
}

/// `cargo xtask gen-fn-catalog --check | --write`.
pub fn run() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(2).collect();
    let result =
        parse_args(&args).and_then(|write| run_in(&data_dir(&crate::workspace_root()), write));
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("xtask gen-fn-catalog: {e:#}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list between the `digest-files` markers of `sql-safety`'s `functions/data.rs`.
    fn sql_safety_list() -> Vec<String> {
        let path = crate::workspace_root().join("crates/sql/sql-safety/src/functions/data.rs");
        let src = std::fs::read_to_string(&path).unwrap();
        let start = src.find("// digest-files:begin").expect("begin marker");
        let end = src.find("// digest-files:end").expect("end marker");
        src[start..end]
            .split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn the_file_list_matches_sql_safety_in_set_and_order() {
        let theirs = sql_safety_list();
        let ours: Vec<String> = DIGEST_FILES.iter().map(|f| f.to_string()).collect();
        assert_eq!(ours, theirs);
    }

    #[test]
    fn the_algorithm_matches_sql_safety_on_a_known_answer() {
        // The same vector is pinned in `sql-safety/tests/functions.rs`.
        assert_eq!(
            digest_of_files([("x", b"ab".as_slice()), ("y", b"c\r\n".as_slice())]),
            KNOWN_ANSWER
        );
    }

    #[test]
    fn the_checked_in_digest_is_fresh() {
        assert!(run_in(&data_dir(&crate::workspace_root()), false).unwrap());
    }

    #[test]
    fn a_changed_data_file_makes_the_digest_stale() {
        let tmp = tempfile::tempdir().unwrap();
        let src = data_dir(&crate::workspace_root());
        for f in DIGEST_FILES {
            let to = tmp.path().join(f);
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::copy(src.join(f), &to).unwrap();
        }
        std::fs::copy(
            src.join("functions.digest"),
            tmp.path().join("functions.digest"),
        )
        .unwrap();
        assert!(run_in(tmp.path(), false).unwrap());
        let allow = tmp.path().join("pg/allow.toml");
        let text = std::fs::read_to_string(&allow).unwrap();
        std::fs::write(
            &allow,
            text.replacen("reviewed = false", "reviewed = true", 1),
        )
        .unwrap();
        assert!(!run_in(tmp.path(), false).unwrap());
        assert!(run_in(tmp.path(), true).unwrap());
        assert!(run_in(tmp.path(), false).unwrap());
        std::fs::remove_file(tmp.path().join("sqlite/types.toml")).unwrap();
        assert!(run_in(tmp.path(), false).is_err());
    }

    #[test]
    fn arguments() {
        let a = |s: &[&str]| parse_args(&s.iter().map(|x| x.to_string()).collect::<Vec<_>>());
        assert!(!a(&["--check"]).unwrap());
        assert!(a(&["--write"]).unwrap());
        assert!(a(&[]).is_err());
        assert!(a(&["--check", "--write"]).is_err());
        assert!(a(&["--from-docker"]).is_err());
    }

    const KNOWN_ANSWER: &str = "xxh3-64:f1ee6490ac0306c7";
}
