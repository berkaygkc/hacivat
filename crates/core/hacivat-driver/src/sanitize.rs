//! Option sanitizing: which driver options a profile of a given origin may carry (HV-DRV-05).
//!
//! Values arrive decoded exactly once. Task 7's URL parser owns that single percent-decode, and
//! a value typed into a form is literal. `sanitize` never decodes again: any `%XX` escape left in
//! a key or a value is a second encoding layer and is rejected.
//!
//! The steps, per option:
//! 1. A NUL or other C0 control character (or DEL) in the key or the value is rejected for every
//!    origin, whatever the unsafe toggle says (`AlwaysDenied`).
//! 2. The key is folded with ASCII lowercase only (a Turkish `İ` never folds to `i`).
//! 3. A key held for review (a startup script) from an untrusted origin is quarantined.
//! 4. The always-deny list applies to every origin, before the allow-list.
//! 5. From an untrusted origin (anything but `UserTyped`), a key outside the allow-list is
//!    unknown.
//! 6. A `%XX` escape in the key or the value is `DoubleEncoding`.
//! 7. From an untrusted origin, PostgreSQL `options` is split the way the server's
//!    `pg_split_opts` splits it (C `isspace`, `\` escapes), may hold no backslash, and may only
//!    set the catalog's safe GUCs (`AlwaysDenied` otherwise).
//! 8. From an untrusted origin, an option whose key is one of the catalog's `path_keys` names a
//!    file, so its value takes the path rule (`sslrootcert=system` is not a path).
//!
//! The path rule: a share or device path (`hacivat_net::is_network_path`) is refused before
//! any file system call; otherwise the path, after `~` expansion and canonicalization with
//! symlinks resolved, must lie inside an approved root. It applies to the `path_keys` options and to every entry
//! of `SanitizeInput::paths`. The user's "unsafe" toggle lifts steps 4 to 8 and the path rule,
//! never step 1 or the quarantine.

use std::path::{Component, Path, PathBuf};

use hacivat_net::is_network_path;
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
    /// Driver options, each value decoded exactly once (by the URL parser) or literal.
    pub options: &'a [(String, String)],
    /// Files named outside `options` (the profile's TLS files), with their paths. Options whose
    /// key is a catalog `path_keys` entry are path-checked without being listed here.
    pub paths: &'a [(String, PathBuf)],
    /// The user allowed options outside the allow-list.
    pub unsafe_toggle: bool,
    pub approved_roots: &'a [PathBuf],
}

/// Keys that run statements or commands; from an untrusted origin they wait for review.
const QUARANTINED: [&str; 3] = ["startup_script", "before_connect_command", "init_sql"];

/// The PostgreSQL option whose `-c name=value` items are checked against the safe GUCs.
const PG_OPTIONS: &str = "options";

/// The `sslrootcert` value that means "the system's trust store", not a file (libpq).
const PG_SYSTEM_ROOTS: (&str, &str) = ("sslrootcert", "system");

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
    let path_keys = spec.map_or(empty, |s| &s.path_keys);
    let trusted = matches!(input.origin, Origin::UserTyped);
    let lifted = input.unsafe_toggle;
    let check_paths = !trusted && !lifted;
    let roots: Vec<PathBuf> = if check_paths {
        input
            .approved_roots
            .iter()
            .filter(|r| !is_network_path(r))
            .filter_map(|r| resolve(r))
            .collect()
    } else {
        Vec::new()
    };
    // A share or a device is refused before `resolve` touches it (N2).
    let inside_roots = |p: &Path| {
        !is_network_path(p) && resolve(p).is_some_and(|p| roots.iter().any(|r| p.starts_with(r)))
    };

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
        let is_pg_options = input.kind == DbmsKind::Postgres && folded == PG_OPTIONS;
        let names_file = find(path_keys).is_some()
            && !(input.kind == DbmsKind::Postgres
                && folded == PG_SYSTEM_ROOTS.0
                && value == PG_SYSTEM_ROOTS.1);
        let reason = if has_control(key) || has_control(value) {
            Some(ViolationReason::AlwaysDenied)
        } else if !trusted && QUARANTINED.contains(&folded.as_str()) {
            Some(ViolationReason::QuarantinedScript)
        } else if lifted {
            None
        } else if find(deny).is_some() {
            Some(ViolationReason::AlwaysDenied)
        } else if !trusted && allowed.is_none() {
            Some(ViolationReason::UnknownOption)
        } else if has_escape(key) || has_escape(value) {
            Some(ViolationReason::DoubleEncoding)
        } else if !trusted && is_pg_options && !pg_options_safe(value, gucs) {
            Some(ViolationReason::AlwaysDenied)
        } else if check_paths && names_file && !inside_roots(Path::new(value)) {
            Some(ViolationReason::PathOutsideRoots)
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

    if check_paths {
        for (key, path) in input.paths {
            if !inside_roots(path) {
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

/// `s` holds NUL, another C0 control character or DEL.
fn has_control(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_control())
}

/// `s` holds a `%XX` escape (two ASCII hex digits).
fn has_escape(s: &str) -> bool {
    s.as_bytes()
        .windows(3)
        .any(|w| w[0] == b'%' && w[1].is_ascii_hexdigit() && w[2].is_ascii_hexdigit())
}

/// C `isspace` in the "C" locale, which the server's `pg_split_opts` uses: space, `\t`, `\n`,
/// `\v`, `\f` and `\r`.
fn c_isspace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0B' | '\x0C' | '\r')
}

/// The words of a PostgreSQL `options` value, split as the server's `pg_split_opts` does:
/// on C `isspace`, with `\` making the next character literal.
fn pg_split_opts(value: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut chars = value.chars().peekable();
    loop {
        while chars.next_if(|c| c_isspace(*c)).is_some() {}
        if chars.peek().is_none() {
            return words;
        }
        let mut word = String::new();
        while let Some(c) = chars.next_if(|c| !c_isspace(*c)) {
            if c == '\\' {
                if let Some(next) = chars.next() {
                    word.push(next);
                }
            } else {
                word.push(c);
            }
        }
        words.push(word);
    }
}

/// Every item of a PostgreSQL `options` value is `-c name=value`, `-cname=value` or
/// `--name=value`, and every name is a safe GUC. Names compare ASCII case-insensitively with
/// `-` read as `_`, as the server's `ParseLongOption` does. A backslash is refused outright:
/// an untrusted value has no reason to escape anything.
fn pg_options_safe(value: &str, gucs: &[String]) -> bool {
    if value.contains('\\') {
        return false;
    }
    let words = pg_split_opts(value);
    let mut words = words.iter();
    let mut any = false;
    while let Some(word) = words.next() {
        let setting = if word == "-c" {
            match words.next() {
                Some(s) => s.as_str(),
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
        let name = name.replace('-', "_");
        if !gucs.iter().any(|g| g.eq_ignore_ascii_case(&name)) {
            return false;
        }
        any = true;
    }
    any
}

// ## m2b-t18-guarded-exec

/// The settings that change which role a PostgreSQL session runs as. On a read-only
/// connection either one would make the role probe judge another identity than the one the
/// guard's reset (`SET SESSION AUTHORIZATION DEFAULT`, `RESET ALL`) returns to (Task 18 fix
/// round 2, I-3b).
const PG_ROLE_SETTINGS: [&str; 2] = ["role", "session_authorization"];

/// Whether a PostgreSQL `options` value sets `role` or `session_authorization`. It is read the
/// way the server reads it: split as `pg_split_opts` splits it (C `isspace`, `\` escapes),
/// then each `-c name=value`, `-cname=value`, `-c` followed by a separate `name=value`, or
/// `--name=value` item; names compare ASCII case-insensitively, with `-` read as `_` and
/// surrounding whitespace ignored. A dangling `-c` sets nothing.
pub fn pg_options_switch_role(value: &str) -> bool {
    let words = pg_split_opts(value);
    let mut words = words.iter();
    while let Some(word) = words.next() {
        let setting = if word == "-c" {
            words.next().map(String::as_str)
        } else {
            word.strip_prefix("--").or_else(|| word.strip_prefix("-c"))
        };
        let Some((name, _)) = setting.and_then(|s| s.split_once('=')) else {
            continue;
        };
        let name = name.trim_matches(c_isspace).replace('-', "_");
        if PG_ROLE_SETTINGS
            .iter()
            .any(|r| r.eq_ignore_ascii_case(&name))
        {
            return true;
        }
    }
    false
}

/// The options a read-only profile may not carry, whatever its origin and the unsafe toggle:
/// a PostgreSQL `options` value that switches the role ([`pg_options_switch_role`]), as
/// `AlwaysDenied`. A read-write profile keeps them (`-c role=` is a legitimate feature).
pub fn read_only_violations(kind: DbmsKind, options: &[(String, String)]) -> Vec<Violation> {
    if kind != DbmsKind::Postgres {
        return Vec::new();
    }
    options
        .iter()
        .filter(|(key, value)| {
            key.eq_ignore_ascii_case(PG_OPTIONS) && pg_options_switch_role(value)
        })
        .map(|(key, _)| Violation {
            key: key.clone(),
            reason: ViolationReason::AlwaysDenied,
        })
        .collect()
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
                    hacivat_net::paths::fs_probe::note(&out);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn gucs() -> Vec<String> {
        ["search_path", "statement_timeout"]
            .map(String::from)
            .to_vec()
    }

    #[test]
    fn options_split_on_c_isspace_with_backslash_escapes() {
        assert_eq!(
            pg_split_opts("-c a=1\x0B-c\x0Cb=2\r\n\t c"),
            ["-c", "a=1", "-c", "b=2", "c"]
        );
        assert_eq!(pg_split_opts(r"-c a=x\ y \\z"), ["-c", r"a=x y", r"\z"]);
        assert!(pg_split_opts(" \x0B ").is_empty());
    }

    #[test]
    fn the_guc_check_sees_the_words_the_server_sees() {
        // Even if a control character got this far, the split matches the server's.
        assert!(!pg_options_safe(
            "-c search_path=a\x0B-c\x0Bsession_preload_libraries=x",
            &gucs()
        ));
        assert!(pg_options_safe(
            "-c search_path=a\x0B-c\x0Bstatement_timeout=1",
            &gucs()
        ));
        assert!(!pg_options_safe(r"-c search_path=a\ b", &gucs()));
    }
}
