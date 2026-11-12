//! `cargo xtask l10n check` (I18N-17) and the hard-coded-string lint (I18N-23).
//!
//! ```text
//! cargo xtask l10n check [--root <locales dir>] [--code <dir>]... [--json <out>]
//! ```
//!
//! Without `--root` the repository's `locales/` tree and every `crates/*/*/src` directory are
//! checked. With `--root`, the code scan runs only for the `--code` directories given.

mod code_scan;
mod rules;
mod smoke;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rules::{FtlFile, finding};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

/// One problem: `{rule, severity, locale, file, line, id}` (I18N-17).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub rule: &'static str,
    pub severity: Severity,
    pub locale: String,
    pub file: String,
    pub line: u32,
    pub id: String,
}

/// Translated and total units (a message value or attribute) of one locale and domain.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Completeness {
    locale: String,
    domain: String,
    translated: usize,
    total: usize,
}

struct Report {
    findings: Vec<Finding>,
    completeness: Vec<Completeness>,
}

/// Checks the catalogs under `root` and, when `code_roots` is not empty, the Rust sources.
#[cfg(test)]
pub fn check(root: &Path, code_roots: &[&Path]) -> Vec<Finding> {
    report(root, code_roots).findings
}

fn report(root: &Path, code_roots: &[&Path]) -> Report {
    let mut out = Vec::new();
    let mut completeness = Vec::new();

    let mut locales: BTreeMap<String, Vec<FtlFile>> = BTreeMap::new();
    let mut folders: Vec<String> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('_') && !n.starts_with('.'))
        .collect();
    folders.sort();

    for folder in &folders {
        if rules::is_pseudo(folder) {
            out.push(finding("L029", Severity::Error, folder, folder, 0, folder));
            continue;
        }
        let files = read_locale(root, folder, &mut out);
        locales.insert(folder.clone(), files);
    }

    let Some(en_files) = locales.get("en") else {
        out.push(finding(
            "L001",
            Severity::Error,
            "en",
            "en",
            0,
            "no `en` folder",
        ));
        return Report {
            findings: out,
            completeness,
        };
    };
    let en = rules::English::new(en_files);

    for (locale, files) in &locales {
        rules::check_ids(files, locale, &mut out);
        let present = rules::check_locale(files, locale, &en, &mut out);
        completeness.extend(completeness_rows(locale, &present, &en));
    }

    smoke::check(&locales, &en, &mut out);

    if !code_roots.is_empty() {
        let en_ids: BTreeSet<String> = en.msgs.keys().map(|k| (*k).to_string()).collect();
        let allow = read_allowlist(root, &mut out);
        let base = root.parent().unwrap_or(root);
        let scan = code_scan::scan(code_roots, &en_ids, &allow, base);
        out.extend(scan.findings);
        unused_ids(root, en_files, &en_ids, &scan.used_ids, &mut out);
    }

    out.sort_by(|a, b| {
        (a.severity, &a.locale, &a.file, a.line, a.rule, &a.id)
            .cmp(&(b.severity, &b.locale, &b.file, b.line, b.rule, &b.id))
    });
    out.dedup();
    Report {
        findings: out,
        completeness,
    }
}

/// Reads one locale folder: manifest (L001), file hygiene (L002, L018, L021) and messages.
fn read_locale(root: &Path, folder: &str, out: &mut Vec<Finding>) -> Vec<FtlFile> {
    let dir = root.join(folder);
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();

    let manifest = format!("{folder}/locale.toml");
    if names.iter().any(|n| n == "locale.toml") {
        if let Some(text) = rules::read_text(&dir.join("locale.toml"), &manifest, folder, out) {
            rules::check_manifest(&text, folder, &manifest, out);
        }
    } else {
        out.push(finding(
            "L001",
            Severity::Warning,
            folder,
            &manifest,
            0,
            "missing locale.toml",
        ));
    }

    let mut files = Vec::new();
    for name in names.iter().filter(|n| n.ends_with(".ftl")) {
        let rel = format!("{folder}/{name}");
        let stem = name.trim_end_matches(".ftl");
        let well_named = !stem.is_empty()
            && stem
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !well_named {
            out.push(finding("L004", Severity::Error, folder, &rel, 0, name));
        }
        let Some(text) = rules::read_text(&dir.join(name), &rel, folder, out) else {
            continue;
        };
        rules::scan_text(&text, &rel, folder, out);
        let mut file = rules::parse_ftl(text.clone(), stem, &rel, folder, out);
        file.src = text;
        files.push(file);
    }
    files
}

/// Units translated per domain of `locale`, for the completeness report.
fn completeness_rows(
    locale: &str,
    present: &BTreeSet<String>,
    en: &rules::English<'_>,
) -> Vec<Completeness> {
    let mut rows: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for (id, (msg, domain)) in &en.msgs {
        let units = usize::from(msg.has_value) + msg.attrs.len();
        let row = rows.entry(domain).or_default();
        row.1 += units;
        if present.contains(*id) {
            row.0 += units;
        }
    }
    rows.into_iter()
        .map(|(domain, (translated, total))| Completeness {
            locale: locale.to_string(),
            domain: domain.to_string(),
            translated,
            total,
        })
        .collect()
}

/// L025: English IDs nothing references.
fn unused_ids(
    root: &Path,
    en_files: &[FtlFile],
    en_ids: &BTreeSet<String>,
    used: &BTreeSet<String>,
    out: &mut Vec<Finding>,
) {
    let families = dynamic_families(root);
    let mut referenced: BTreeSet<&str> = used.iter().map(String::as_str).collect();
    for m in en_files.iter().flat_map(|f| &f.msgs) {
        referenced.extend(m.refs.iter().map(String::as_str));
    }
    for f in en_files {
        for m in &f.msgs {
            if !en_ids.contains(&m.id) || referenced.contains(m.id.as_str()) {
                continue;
            }
            let dynamic = families.iter().any(|p| match p.strip_suffix('*') {
                Some(prefix) => m.id.starts_with(prefix),
                None => m.id == *p,
            });
            if !dynamic {
                out.push(finding(
                    "L025",
                    Severity::Warning,
                    "en",
                    &f.rel,
                    m.line,
                    &m.id,
                ));
            }
        }
    }
}

fn read_toml(path: &Path) -> Option<toml::Table> {
    std::fs::read_to_string(path).ok()?.parse().ok()
}

fn dynamic_families(root: &Path) -> Vec<String> {
    read_toml(&root.join("_schema/dynamic-ids.toml"))
        .and_then(|t| t.get("families").cloned())
        .and_then(|v| v.as_array().cloned())
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
}

/// The allow-listed hard-coded literals (`locales/_schema/literal-allowlist.toml`, F19).
fn read_allowlist(root: &Path, out: &mut Vec<Finding>) -> BTreeSet<String> {
    let path = root.join("_schema/literal-allowlist.toml");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return BTreeSet::new();
    };
    match text.parse::<toml::Table>() {
        Ok(t) => t
            .get("literals")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        Err(_) => {
            let rel = "_schema/literal-allowlist.toml";
            out.push(finding(
                "L003",
                Severity::Error,
                "",
                rel,
                1,
                "not valid TOML",
            ));
            BTreeSet::new()
        }
    }
}

/// Every `crates/*/*/src` directory of the workspace.
pub fn repo_code_roots() -> Vec<PathBuf> {
    let crates = crate::workspace_root().join("crates");
    let mut roots = Vec::new();
    for group in std::fs::read_dir(&crates).into_iter().flatten().flatten() {
        for krate in std::fs::read_dir(group.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            let src = krate.path().join("src");
            if src.is_dir() {
                roots.push(src);
            }
        }
    }
    roots.sort();
    roots
}

fn rule_summary(rule: &str) -> &'static str {
    match rule {
        "L001" => "locale.toml manifest / folder tag",
        "L002" => "file encoding: UTF-8, no BOM, LF, NFC, at most 1 MB",
        "L003" => "unparsable Fluent text",
        "L004" => "message ID grammar or prefix does not match the domain file",
        "L005" => "duplicate message ID",
        "L006" => "variable documentation incomplete or malformed",
        "L007" => "NUMBER/DATETIME/LIST option or function not allowed",
        "L008" => "message not in English (obsolete)",
        "L009" => "message missing from the locale",
        "L010" => "value/attribute shape differs from English",
        "L011" => "variable not in the English signature",
        "L012" => "English variable absent from the translation",
        "L013" => "plural key not in the locale's CLDR cardinal set",
        "L014" => "CLDR plural category missing",
        "L015" => "reference to a message or term the locale cannot resolve",
        "L017" => "markup tags differ from English",
        "L018" => "forbidden character or sequence",
        "L021" => "Turkish suffix directly after a placeable",
        "L023" => "access key not in the label or not unique in its container",
        "L024" => "code uses an undefined message ID",
        "L025" => "English message never referenced",
        "L027" => "message fails to render with sample arguments",
        "L029" => "pseudo-locale folder committed",
        "L900" => "hard-coded UI text or string assembly",
        _ => "",
    }
}

fn print_report(findings: &[Finding]) {
    for f in findings {
        let at = if f.line == 0 {
            f.file.clone()
        } else {
            format!("{}:{}", f.file, f.line)
        };
        let id = if f.id.is_empty() {
            String::new()
        } else {
            format!(" `{}`", f.id)
        };
        println!(
            "{} {} {at}{id}: {}",
            f.severity.as_str(),
            f.rule,
            rule_summary(f.rule)
        );
    }
}

fn write_json(path: &Path, report: &Report) -> std::io::Result<()> {
    let findings: Vec<_> = report
        .findings
        .iter()
        .map(|f| {
            serde_json::json!({
                "rule": f.rule,
                "severity": f.severity.as_str(),
                "locale": f.locale,
                "file": f.file,
                "line": f.line,
                "id": f.id,
            })
        })
        .collect();
    let completeness: Vec<_> = report
        .completeness
        .iter()
        .map(|c| {
            serde_json::json!({
                "locale": c.locale,
                "domain": c.domain,
                "translated": c.translated,
                "total": c.total,
            })
        })
        .collect();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let doc = serde_json::json!({ "findings": findings, "completeness": completeness });
    std::fs::write(
        path,
        serde_json::to_string_pretty(&doc).unwrap_or_default() + "\n",
    )
}

pub fn run() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(2).collect();
    if args.first().map(String::as_str) != Some("check") {
        eprintln!("usage: xtask l10n check [--root <dir>] [--code <dir>]... [--json <out>]");
        return ExitCode::from(2);
    }
    let mut root = None;
    let mut json = None;
    let mut code: Vec<PathBuf> = Vec::new();
    let mut it = args.iter().skip(1);
    while let Some(arg) = it.next() {
        let slot = match arg.as_str() {
            "--root" => &mut root,
            "--json" => &mut json,
            "--code" => {
                match it.next() {
                    Some(v) => code.push(PathBuf::from(v)),
                    None => return usage("--code needs a directory"),
                }
                continue;
            }
            other => return usage(&format!("unknown argument `{other}`")),
        };
        match it.next() {
            Some(v) => *slot = Some(PathBuf::from(v)),
            None => return usage(&format!("`{arg}` needs a value")),
        }
    }
    let custom_root = root.is_some();
    let root = root.unwrap_or_else(|| crate::workspace_root().join("locales"));
    if !custom_root {
        code = repo_code_roots();
    }
    let json = json.unwrap_or_else(|| crate::workspace_root().join("target/l10n-report.json"));

    let code_refs: Vec<&Path> = code.iter().map(PathBuf::as_path).collect();
    let report = report(&root, &code_refs);
    print_report(&report.findings);
    if let Err(e) = write_json(&json, &report) {
        eprintln!("cannot write {}: {e}", json.display());
        return ExitCode::from(2);
    }
    let errors = report
        .findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .count();
    let warnings = report.findings.len() - errors;
    println!("{warnings} warnings");
    println!("l10n-check: {errors} errors");
    if errors == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn usage(msg: &str) -> ExitCode {
    eprintln!("xtask l10n: {msg}");
    ExitCode::from(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fx(name: &str) -> PathBuf {
        crate::workspace_root()
            .join("xtask/tests/fixtures/l10n")
            .join(name)
    }

    fn rules_of(f: &[Finding]) -> Vec<&'static str> {
        f.iter().map(|x| x.rule).collect()
    }

    fn errors(f: &[Finding]) -> Vec<&Finding> {
        f.iter().filter(|x| x.severity == Severity::Error).collect()
    }

    #[test]
    fn rules_fire_on_fixtures() {
        for (fixture, rule) in [
            ("typo-var", "L011"),
            ("obsolete", "L008"),
            ("missing", "L009"),
            ("tr-few", "L013"),
            ("rlo", "L018"),
            ("dropped-code-tag", "L017"),
            ("undocumented-var", "L006"),
            ("bad-id", "L004"),
            ("tr_TR-folder", "L001"),
            ("nfd", "L002"),
            ("pseudo-folder", "L029"),
            ("junk", "L003"),
            ("dup-across-domains", "L005"),
            ("suffix-after-placeable", "L021"),
            ("bad-option", "L007"),
            ("drops-var", "L012"),
            ("undefined-ref", "L015"),
            ("undefined-ref", "L027"),
            ("accesskey", "L023"),
        ] {
            assert!(
                rules_of(&check(&fx(fixture), &[])).contains(&rule),
                "{fixture} -> {rule}"
            );
        }
    }

    /// The IDs flagged by `rule` in `fixture`.
    fn ids_of(fixture: &str, rule: &str) -> Vec<String> {
        let mut ids: Vec<String> = check(&fx(fixture), &[])
            .into_iter()
            .filter(|f| f.rule == rule)
            .map(|f| f.id)
            .collect();
        ids.dedup();
        ids
    }

    #[test]
    fn clean_fixture_exits_zero() {
        let f = check(&fx("clean"), &[]);
        assert!(errors(&f).is_empty(), "{f:?}");
        // Clean Turkish one/other plurals raise neither a dead-variant nor a missing-category finding.
        assert!(
            f.iter().all(|x| x.rule != "L013" && x.rule != "L014"),
            "{f:?}"
        );
        assert!(f.is_empty(), "{f:?}");
    }

    #[test]
    fn only_the_wrong_message_is_flagged() {
        // Every fixture has several correct messages next to the one broken one.
        assert_eq!(ids_of("typo-var", "L011"), ["cm-saved"]);
        assert_eq!(ids_of("drops-var", "L012"), ["cm-saved"]);
        assert_eq!(ids_of("dropped-code-tag", "L017"), ["cm-hint"]);
        assert_eq!(ids_of("tr-few", "L013"), ["cm-rows"]);
        assert_eq!(ids_of("obsolete", "L008"), ["cm-gone"]);
        assert_eq!(ids_of("missing", "L009"), ["cm-ok"]);
        assert_eq!(
            ids_of("bad-option", "L007"),
            ["cm-size NUMBER(bogusOption)"]
        );
        assert_eq!(ids_of("undefined-ref", "L027"), ["cm-hint"]);
        assert_eq!(ids_of("undefined-ref", "L015"), ["cm-hint -> nope"]);
        assert_eq!(
            ids_of("accesskey", "L023"),
            ["hv-menu-file-save", "hv-menu-file-close"]
        );
    }

    #[test]
    fn code_scan_flags_unknown_id_unused_id_concat_and_literal() {
        let code = fx("code");
        let f = check(&fx("clean"), &[&code]);
        for r in ["L024", "L025", "L900"] {
            assert!(rules_of(&f).contains(&r), "{r}: {f:?}");
        }
        assert!(
            f.iter()
                .any(|x| x.rule == "L900" && x.id.contains("format!"))
        );
        // The allow-listed literal and the dynamic family stay quiet.
        assert!(!f.iter().any(|x| x.id == "Allowed text"), "{f:?}");
        assert!(
            !f.iter()
                .any(|x| x.rule == "L025" && x.id.starts_with("hui-")),
            "{f:?}"
        );
    }

    #[test]
    fn missing_translations_lower_completeness() {
        let r = report(&fx("missing"), &[]);
        let tr = r
            .completeness
            .iter()
            .find(|c| c.locale == "tr" && c.domain == "common")
            .unwrap();
        assert!(tr.translated < tr.total, "{tr:?}");
    }

    #[test]
    #[ignore = "repo-wide: run by CI l10n-check and the gate (F19)"]
    fn repo_catalogs_are_clean() {
        let roots = repo_code_roots();
        let refs: Vec<&Path> = roots.iter().map(PathBuf::as_path).collect();
        let f = check(&crate::workspace_root().join("locales"), &refs);
        assert!(errors(&f).is_empty(), "{:#?}", errors(&f));
    }
}
