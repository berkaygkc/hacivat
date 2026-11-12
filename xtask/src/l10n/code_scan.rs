//! The Rust source scan: unknown and unused message IDs (L024, L025) and the string-assembly
//! and hard-coded-literal lints (I18N-23, rule `L900`). A token scan; `syn` is not needed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::rules::finding;
use super::{Finding, Severity};

/// Function names that return translated text (I18N-23 rule 1).
const TEXT_FNS: [&str; 6] = ["t", "tr", "text", "l10n", "ui_text", "fluent"];

/// ID prefixes that double as `hacivat_l10n::ids` module names.
const ID_MODULES: [&str; 10] = [
    "cm", "hui", "hv", "hc", "kg", "ps", "pw", "pe", "db", "error",
];

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Punct(char),
    Other,
}

struct Token {
    tok: Tok,
    line: u32,
}

/// What the scan found.
pub struct Scan {
    pub findings: Vec<Finding>,
    /// Message IDs referenced by literals or `ids::<ns>::<CONST>` paths.
    pub used_ids: BTreeSet<String>,
}

/// Scans every `.rs` file under `roots`. `en_ids` are the defined English IDs; `allow` the
/// allow-listed literals; `display_base` is stripped from reported file names.
pub fn scan(
    roots: &[&Path],
    en_ids: &BTreeSet<String>,
    allow: &BTreeSet<String>,
    display_base: &Path,
) -> Scan {
    let mut result = Scan {
        findings: Vec::new(),
        used_ids: BTreeSet::new(),
    };
    let mut files = Vec::new();
    for root in roots {
        collect_rs(root, &mut files);
    }
    files.sort();
    files.dedup();
    for file in files {
        let Ok(src) = std::fs::read_to_string(&file) else {
            continue;
        };
        let display = file.strip_prefix(display_base).unwrap_or(&file);
        let display = display.to_string_lossy().replace('\\', "/");
        scan_source(&src, &display, en_ids, allow, &mut result);
    }
    result
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    if dir.is_file() {
        out.push(dir.to_path_buf());
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "target") {
                collect_rs(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Whether the hard-coded-literal lint applies to this file (`hacivat-ui` and the app crates).
fn literal_lint_applies(display: &str) -> bool {
    let padded = format!("/{display}");
    padded.contains("/crates/ui/hacivat-ui/src/")
        || padded.split_once("/crates/app/").is_some_and(|(_, rest)| {
            rest.split_once('/')
                .is_some_and(|(_, r)| r.starts_with("src/"))
        })
}

fn scan_source(
    src: &str,
    display: &str,
    en_ids: &BTreeSet<String>,
    allow: &BTreeSet<String>,
    result: &mut Scan,
) {
    let toks = lex(src);
    let mentions_ids = toks.iter().any(|t| t.tok == Tok::Ident("ids".into()));
    let literals = literal_lint_applies(display);
    let mut push = |rule: &'static str, line: u32, id: &str| {
        result
            .findings
            .push(finding(rule, Severity::Error, "", display, line, id));
    };
    let mut used = Vec::new();

    let mut i = 0;
    while i < toks.len() {
        if let Some(next) = skip_test_item(&toks, i) {
            i = next;
            continue;
        }
        let line = toks[i].line;
        match &toks[i].tok {
            Tok::Str(s) => {
                if hacivat_l10n_schema::is_valid_id(s) {
                    used.push((s.clone(), line));
                }
            }
            Tok::Ident(name) => {
                // ids::<ns>::<CONST>, or <ns>::<CONST> in a file that mentions `ids`.
                if let Some((ns, konst, len)) = id_path(&toks, i, mentions_ids) {
                    used.push((id_of(&ns, &konst), line));
                    i += len;
                    continue;
                }
                if name == "format"
                    && is(&toks, i + 1, '!')
                    && is(&toks, i + 2, '(')
                    && let Some(template) = format_assembly(&toks, i + 2)
                {
                    push("L900", line, &format!("format!(\"{template}\" ..)"));
                }
                if TEXT_FNS.contains(&name.as_str())
                    && is(&toks, i + 1, '(')
                    && let Some(close) = matching(&toks, i + 1)
                    && is(&toks, close + 1, '+')
                    && (is_str(&toks, close + 2) || is_text_call(&toks, close + 2))
                {
                    push("L900", line, &format!("{name}(..) + .."));
                }
                if literals {
                    hardcoded(&toks, i, allow, &mut push);
                }
            }
            _ => {}
        }
        i += 1;
    }

    for (id, line) in used {
        if !en_ids.contains(&id) {
            result
                .findings
                .push(finding("L024", Severity::Error, "", display, line, &id));
        }
        result.used_ids.insert(id);
    }
}

/// Reports a string literal with letters passed straight to a UI text sink.
fn hardcoded(
    toks: &[Token],
    i: usize,
    allow: &BTreeSet<String>,
    push: &mut impl FnMut(&'static str, u32, &str),
) {
    let Tok::Ident(name) = &toks[i].tok else {
        return;
    };
    let (open, needs_fluent) = match name.as_str() {
        "child" if i > 0 && is(toks, i - 1, '.') => (i + 1, false),
        "SharedString" if is_path(toks, i + 1, &["from"]) => (i + 4, false),
        "Label" if is_path(toks, i + 1, &["new"]) => (i + 4, true),
        _ => return,
    };
    if !is(toks, open, '(') {
        return;
    }
    let Some(Tok::Str(lit)) = toks.get(open + 1).map(|t| &t.tok) else {
        return;
    };
    if !lit.chars().any(char::is_alphabetic) || allow.contains(lit) {
        return;
    }
    if needs_fluent {
        let close = matching(toks, open).unwrap_or(toks.len());
        let fluent = (open..close).any(|j| {
            matches!(&toks[j].tok, Tok::Ident(n) if n == "TextSource")
                && is_path(toks, j + 1, &["Fluent"])
        });
        if !fluent {
            return;
        }
    }
    push("L900", toks[open + 1].line, lit);
}

/// `format!` whose template has two or more placeholders and whose arguments call a text
/// function. Returns the template.
fn format_assembly(toks: &[Token], open: usize) -> Option<String> {
    let Some(Tok::Str(template)) = toks.get(open + 1).map(|t| &t.tok) else {
        return None;
    };
    let close = matching(toks, open)?;
    let placeholders = template.replace("{{", "").matches('{').count();
    let calls_text = (open + 2..close).any(|j| is_text_call(toks, j));
    (placeholders >= 2 && calls_text).then(|| template.clone())
}

fn is_text_call(toks: &[Token], j: usize) -> bool {
    matches!(toks.get(j).map(|t| &t.tok), Some(Tok::Ident(n)) if TEXT_FNS.contains(&n.as_str()))
        && is(toks, j + 1, '(')
}

/// The `ids::<ns>::<CONST>` path starting at `i` as `(ns, CONST, token count)`.
fn id_path(toks: &[Token], i: usize, bare_ok: bool) -> Option<(String, String, usize)> {
    let Tok::Ident(first) = &toks[i].tok else {
        return None;
    };
    let (ns_at, skip) = if first == "ids" && is_colons(toks, i + 1) {
        (i + 3, 3)
    } else if bare_ok && ID_MODULES.contains(&first.as_str()) {
        (i, 0)
    } else {
        return None;
    };
    let Tok::Ident(ns) = &toks.get(ns_at)?.tok else {
        return None;
    };
    if !ID_MODULES.contains(&ns.as_str()) || !is_colons(toks, ns_at + 1) {
        return None;
    }
    let Tok::Ident(konst) = &toks.get(ns_at + 3)?.tok else {
        return None;
    };
    let shouty = konst.len() > 1
        && konst.starts_with(|c: char| c.is_ascii_uppercase() || c == '_')
        && konst
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    shouty.then(|| (ns.clone(), konst.clone(), skip + 4))
}

/// The message ID a generated constant stands for (`cm::ERROR_UNKNOWN` is `cm-error-unknown`).
fn id_of(ns: &str, konst: &str) -> String {
    let body = konst
        .strip_prefix('_')
        .filter(|r| r.starts_with(|c: char| c.is_ascii_digit()))
        .unwrap_or(konst);
    format!("{ns}-{}", body.to_ascii_lowercase().replace('_', "-"))
}

/// Skips an item annotated `#[cfg(test)]`, `#[test]` or `#[gpui::test]`; returns the index after it.
fn skip_test_item(toks: &[Token], i: usize) -> Option<usize> {
    if !(is(toks, i, '#') && is(toks, i + 1, '[')) {
        return None;
    }
    let mut j = i;
    let mut test = false;
    while is(toks, j, '#') && is(toks, j + 1, '[') {
        let close = matching(toks, j + 1)?;
        let idents: Vec<&str> = toks[j + 2..close]
            .iter()
            .filter_map(|t| match &t.tok {
                Tok::Ident(n) => Some(n.as_str()),
                _ => None,
            })
            .collect();
        let cfg_test =
            idents.first() == Some(&"cfg") && idents.contains(&"test") && !idents.contains(&"not");
        let test_attr = idents.last() == Some(&"test") && idents.first() != Some(&"cfg");
        test |= cfg_test || test_attr;
        j = close + 1;
    }
    if !test {
        return None;
    }
    // The item ends at its matching `}` or at a `;` outside any braces.
    let mut depth = 0usize;
    while let Some(t) = toks.get(j) {
        match t.tok {
            Tok::Punct('{') => depth += 1,
            Tok::Punct('}') => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(j + 1);
                }
            }
            Tok::Punct(';') if depth == 0 => return Some(j + 1),
            _ => {}
        }
        j += 1;
    }
    Some(toks.len())
}

fn is(toks: &[Token], i: usize, c: char) -> bool {
    matches!(toks.get(i), Some(Token { tok: Tok::Punct(p), .. }) if *p == c)
}

fn is_str(toks: &[Token], i: usize) -> bool {
    matches!(
        toks.get(i),
        Some(Token {
            tok: Tok::Str(_),
            ..
        })
    )
}

fn is_colons(toks: &[Token], i: usize) -> bool {
    is(toks, i, ':') && is(toks, i + 1, ':')
}

/// `::a::b` starting at `i` (so `is_path(toks, i, &["new"])` matches `::new`).
fn is_path(toks: &[Token], i: usize, names: &[&str]) -> bool {
    let mut at = i;
    for name in names {
        if !is_colons(toks, at)
            || !matches!(toks.get(at + 2), Some(Token { tok: Tok::Ident(n), .. }) if n == name)
        {
            return false;
        }
        at += 3;
    }
    true
}

/// The index of the bracket closing the one at `open`.
fn matching(toks: &[Token], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (j, t) in toks.iter().enumerate().skip(open) {
        match t.tok {
            Tok::Punct('(' | '[' | '{') => depth += 1,
            Tok::Punct(')' | ']' | '}') => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(j);
                }
            }
            _ => {}
        }
    }
    None
}

/// A minimal Rust lexer: identifiers, string literals (decoded), single-character punctuation.
/// Comments, char literals, lifetimes and numbers are consumed.
fn lex(src: &str) -> Vec<Token> {
    let chars: Vec<char> = src.chars().collect();
    let mut toks = Vec::new();
    let mut line = 1u32;
    let mut i = 0;
    let at = |i: usize| chars.get(i).copied().unwrap_or('\0');
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c == '/' && at(i + 1) == '/' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && at(i + 1) == '*' {
            let mut depth = 1;
            i += 2;
            while i < chars.len() && depth > 0 {
                if chars[i] == '/' && at(i + 1) == '*' {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && at(i + 1) == '/' {
                    depth -= 1;
                    i += 2;
                } else {
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
        } else if let Some((text, next)) = raw_string(&chars, i) {
            line +=
                u32::try_from(chars[i..next].iter().filter(|&&c| c == '\n').count()).unwrap_or(0);
            toks.push(Token {
                tok: Tok::Str(text),
                line,
            });
            i = next;
        } else if c == '"' || (c == 'b' && at(i + 1) == '"') {
            let start_line = line;
            i += if c == 'b' { 2 } else { 1 };
            let mut text = String::new();
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' {
                    i = unescape(&chars, i, &mut text, &mut line);
                } else {
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    text.push(chars[i]);
                    i += 1;
                }
            }
            i += 1;
            toks.push(Token {
                tok: Tok::Str(text),
                line: start_line,
            });
        } else if c == '\'' {
            // A char literal (`'x'`, `'\n'`) or a lifetime (`'a`).
            if at(i + 1) == '\\' {
                i += 2;
                while i < chars.len() && chars[i] != '\'' {
                    i += 1;
                }
                i += 1;
                toks.push(Token {
                    tok: Tok::Other,
                    line,
                });
            } else if at(i + 2) == '\'' {
                i += 3;
                toks.push(Token {
                    tok: Tok::Other,
                    line,
                });
            } else {
                i += 1;
            }
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let ident: String = chars[start..i].iter().collect();
            toks.push(Token {
                tok: Tok::Ident(ident),
                line,
            });
        } else if c.is_ascii_digit() {
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            toks.push(Token {
                tok: Tok::Other,
                line,
            });
        } else {
            toks.push(Token {
                tok: Tok::Punct(c),
                line,
            });
            i += 1;
        }
    }
    toks
}

/// `r"..."` / `r#"..."#` (optionally `b`-prefixed) starting at `i`: the text and the next index.
fn raw_string(chars: &[char], i: usize) -> Option<(String, usize)> {
    let mut j = i;
    if chars.get(j) == Some(&'b') {
        j += 1;
    }
    if chars.get(j) != Some(&'r') {
        return None;
    }
    j += 1;
    let hashes = chars[j..].iter().take_while(|&&c| c == '#').count();
    j += hashes;
    if chars.get(j) != Some(&'"') {
        return None;
    }
    j += 1;
    let start = j;
    while j < chars.len() {
        if chars[j] == '"'
            && chars[j + 1..]
                .iter()
                .take(hashes)
                .filter(|&&c| c == '#')
                .count()
                == hashes
        {
            let text: String = chars[start..j].iter().collect();
            return Some((text, j + 1 + hashes));
        }
        j += 1;
    }
    None
}

/// Decodes the escape at `chars[i]` (a backslash) into `out`; returns the next index.
fn unescape(chars: &[char], i: usize, out: &mut String, line: &mut u32) -> usize {
    let Some(&e) = chars.get(i + 1) else {
        return i + 1;
    };
    match e {
        'n' => out.push('\n'),
        't' => out.push('\t'),
        'r' => out.push('\r'),
        '0' => out.push('\0'),
        '\n' => {
            // A line continuation eats the following indentation.
            *line += 1;
            let mut j = i + 2;
            while chars.get(j).is_some_and(|c| c.is_whitespace()) {
                if chars[j] == '\n' {
                    *line += 1;
                }
                j += 1;
            }
            return j;
        }
        'u' if chars.get(i + 2) == Some(&'{') => {
            let end = chars[i..]
                .iter()
                .position(|&c| c == '}')
                .map_or(chars.len(), |p| i + p);
            let hex: String = chars[i + 3..end.min(chars.len())]
                .iter()
                .filter(|c| **c != '_')
                .collect();
            if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                out.push(c);
            }
            return end + 1;
        }
        'x' => {
            let hex: String = chars[i + 2..].iter().take(2).collect();
            if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                out.push(c);
            }
            return i + 4;
        }
        other => out.push(other),
    }
    i + 2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str, display: &str, en: &[&str], allow: &[&str]) -> Scan {
        let en: BTreeSet<String> = en.iter().map(|s| s.to_string()).collect();
        let allow: BTreeSet<String> = allow.iter().map(|s| s.to_string()).collect();
        let mut result = Scan {
            findings: Vec::new(),
            used_ids: BTreeSet::new(),
        };
        scan_source(src, display, &en, &allow, &mut result);
        result
    }

    fn rules(s: &Scan) -> Vec<(&str, &str)> {
        s.findings.iter().map(|f| (f.rule, f.id.as_str())).collect()
    }

    #[test]
    fn test_modules_comments_and_chars_are_ignored() {
        let src = r#"
            // .child("Not code")
            fn view() { let _ = '"'; let _ = "hv-known"; }
            #[cfg(test)]
            mod tests { fn t() { x.child("Test only"); y("hv-nope"); } }
        "#;
        let s = run(src, "crates/ui/hacivat-ui/src/a.rs", &["hv-known"], &[]);
        assert!(s.findings.is_empty(), "{:?}", rules(&s));
        assert!(s.used_ids.contains("hv-known"));
    }

    #[test]
    fn literal_sinks_are_flagged_unless_allowed_or_out_of_scope() {
        let src = r#"
            fn v() {
                a.child("Open"); a.child("orders-prod"); a.child("…"); a.child(name);
                SharedString::from("Close");
                Label::new("Save", TextSource::Fluent, s, c);
                Label::new("orders", TextSource::Catalog, s, c);
            }
        "#;
        let s = run(src, "crates/ui/hacivat-ui/src/a.rs", &[], &["orders-prod"]);
        assert_eq!(
            rules(&s),
            [("L900", "Open"), ("L900", "Close"), ("L900", "Save")]
        );
        let out_of_scope = run(src, "crates/core/hacivat-l10n/src/a.rs", &[], &[]);
        assert!(out_of_scope.findings.is_empty());
        let app = run(src, "crates/app/hacivat-app/src/a.rs", &[], &[]);
        assert_eq!(app.findings.len(), 4);
    }

    #[test]
    fn ids_paths_resolve_to_message_ids() {
        let src = "use hacivat_l10n::ids; fn f() { t(ids::cm::ERROR_UNKNOWN); t(ids::hv::NOPE_X); t(hv::_2FA); }";
        let s = run(src, "x.rs", &["cm-error-unknown", "hv-2fa"], &[]);
        assert_eq!(rules(&s), [("L024", "hv-nope-x")]);
        assert!(s.used_ids.contains("cm-error-unknown") && s.used_ids.contains("hv-2fa"));
    }

    #[test]
    fn concatenation_of_translated_text_is_flagged() {
        let src = r#"fn f() {
            let a = format!("{} {}", l10n.t(DELETE), name);
            let b = t(A) + " " + &t(B);
            let c = format!("{} rows", n);
        }"#;
        let s = run(src, "x.rs", &[], &[]);
        assert_eq!(s.findings.len(), 2, "{:?}", rules(&s));
        assert!(s.findings[0].id.contains("format!"));
    }
}
