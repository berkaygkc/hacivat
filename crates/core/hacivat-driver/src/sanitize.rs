//! Option sanitizing: which driver options a profile of a given origin may carry (HV-DRV-05).
//!
//! The steps, per option:
//! 1. The key is folded with ASCII lowercase only (a Turkish `İ` never folds to `i`).
//! 2. A key held for review (a startup script) from an untrusted origin is quarantined.
//! 3. The always-deny list applies to every origin, before the allow-list.
//! 4. From an untrusted origin (anything but `UserTyped`), a key outside the allow-list is
//!    unknown, and PostgreSQL `options` may only set the catalog's safe GUCs.
//! 5. Values arrive decoded once (the URL parser decodes; a typed value is literal), so any
//!    `%XX` escape left in a value is a second encoding layer and is rejected.
//!
//! Paths (option values that name files) from an untrusted origin must lie inside an approved
//! root after `~` expansion and canonicalization, with symlinks resolved. The user's "unsafe"
//! toggle lifts steps 3 and 4, the escape check and the path rule, never the quarantine.

use std::path::{Component, Path, PathBuf};

use hacivat_proto::{DbmsKind, Origin};

use crate::catalog::EngineCatalog;

/// Why an option was rejected (HV-DRV-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViolationReason {
    UnknownOption,
    DoubleEncoding,
    PathOutsideRoots,
    AlwaysDenied,
    QuarantinedScript,
}

/// One rejected option (HV-DRV-05).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Violation {
    pub key: String,
    pub reason: ViolationReason,
}

/// What [`sanitize`] checks (HV-DRV-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SanitizeInput<'a> {
    pub kind: DbmsKind,
    pub origin: &'a Origin,
    pub options: &'a [(String, String)],
    /// Options that name files, with their paths.
    pub paths: &'a [(String, PathBuf)],
    /// The user allowed options outside the allow-list.
    pub unsafe_toggle: bool,
    pub approved_roots: &'a [PathBuf],
}

/// Keys that run statements or commands; from an untrusted origin they wait for review.
const QUARANTINED: [&str; 3] = ["startup_script", "before_connect_command", "init_sql"];

/// The PostgreSQL option whose `-c name=value` items are checked against the safe GUCs.
const PG_OPTIONS: &str = "options";

/// The options to pass to the driver, or every violation in input order (options, then
/// paths). Allowed keys come back in the catalog's spelling (`SSLMODE` gives `sslmode`); other
/// keys and every value come back as given.
pub fn sanitize(
    input: &SanitizeInput<'_>,
    catalog: &EngineCatalog,
) -> Result<Vec<(String, String)>, Vec<Violation>> {
    let spec = catalog.get(input.kind);
    let empty: &[String] = &[];
    let allow = spec.map_or(empty, |s| &s.option_allow);
    let deny = spec.map_or(empty, |s| &s.always_deny);
    let gucs = spec.map_or(empty, |s| &s.safe_gucs);
    let trusted = matches!(input.origin, Origin::UserTyped);
    let lifted = input.unsafe_toggle;

    let mut out = Vec::with_capacity(input.options.len());
    let mut violations = Vec::new();
    for (key, value) in input.options {
        let folded = key.to_ascii_lowercase();
        let find = |list: &[String]| {
            list.iter()
                .find(|k| k.eq_ignore_ascii_case(&folded))
                .cloned()
        };
        let allowed = find(allow);
        let reason = if !trusted && QUARANTINED.contains(&folded.as_str()) {
            Some(ViolationReason::QuarantinedScript)
        } else if lifted {
            None
        } else if find(deny).is_some() {
            Some(ViolationReason::AlwaysDenied)
        } else if !trusted && allowed.is_none() {
            Some(ViolationReason::UnknownOption)
        } else if has_escape(value) {
            Some(ViolationReason::DoubleEncoding)
        } else if !trusted
            && input.kind == DbmsKind::Postgres
            && folded == PG_OPTIONS
            && !pg_options_safe(value, gucs)
        {
            Some(ViolationReason::AlwaysDenied)
        } else {
            None
        };
        match reason {
            Some(reason) => violations.push(Violation {
                key: key.clone(),
                reason,
            }),
            None => out.push((allowed.unwrap_or_else(|| key.clone()), value.clone())),
        }
    }

    if !trusted && !lifted {
        let roots: Vec<PathBuf> = input
            .approved_roots
            .iter()
            .filter_map(|r| resolve(r))
            .collect();
        for (key, path) in input.paths {
            let inside = resolve(path).is_some_and(|p| roots.iter().any(|r| p.starts_with(r)));
            if !inside {
                violations.push(Violation {
                    key: key.clone(),
                    reason: ViolationReason::PathOutsideRoots,
                });
            }
        }
    }

    if violations.is_empty() {
        Ok(out)
    } else {
        Err(violations)
    }
}

/// `s` holds a `%XX` escape (two ASCII hex digits).
fn has_escape(s: &str) -> bool {
    s.as_bytes()
        .windows(3)
        .any(|w| w[0] == b'%' && w[1].is_ascii_hexdigit() && w[2].is_ascii_hexdigit())
}

/// Every item of a PostgreSQL `options` value is `-c name=value`, `-cname=value` or
/// `--name=value`, and every name is a safe GUC (ASCII case-insensitive, as the server is).
fn pg_options_safe(value: &str, gucs: &[String]) -> bool {
    let mut words = value.split_ascii_whitespace();
    let mut any = false;
    while let Some(word) = words.next() {
        let setting = if word == "-c" {
            match words.next() {
                Some(s) => s,
                None => return false,
            }
        } else if let Some(s) = word.strip_prefix("--").or_else(|| word.strip_prefix("-c")) {
            s
        } else {
            return false;
        };
        let Some((name, _)) = setting.split_once('=') else {
            return false;
        };
        if !gucs.iter().any(|g| g.eq_ignore_ascii_case(name)) {
            return false;
        }
        any = true;
    }
    any
}

/// `p` with a leading `~` expanded, made absolute and canonicalized one component at a time:
/// a component that exists is resolved through the file system (symlinks included), so every
/// prefix is either canonical or does not exist, and `..` can always drop the last component.
/// `None` when it cannot be resolved (no home directory, a dangling symlink, an I/O error).
fn resolve(p: &Path) -> Option<PathBuf> {
    let expanded = match p.strip_prefix("~") {
        Ok(rest) => std::env::home_dir()?.join(rest),
        Err(_) => p.to_path_buf(),
    };
    let absolute = std::path::absolute(&expanded).ok()?;
    let mut out = PathBuf::new();
    let mut exists = true;
    for c in absolute.components() {
        match c {
            Component::Prefix(_) | Component::RootDir => out.push(c),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(name) => {
                out.push(name);
                if exists {
                    match out.symlink_metadata() {
                        Ok(_) => out = out.canonicalize().ok()?,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => exists = false,
                        Err(_) => return None,
                    }
                }
            }
        }
    }
    Some(out)
}
