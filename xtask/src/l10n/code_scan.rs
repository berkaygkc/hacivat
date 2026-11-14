//! The Rust source scan: unknown and unused message IDs (L024, L025) and the string-assembly
//! and hard-coded-literal lints (I18N-23, rule `L900`). A token scan; `syn` is not needed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::rules::finding;
use super::{Finding, Severity};

/// ID prefixes that double as `hacivat_l10n::ids` module names.
const ID_MODULES: [&str; 12] = [
    "cm", "hui", "hv", "hc", "kg", "ps", "pw", "pe", "db", "error",
    // ## m3a-t01b-seams: the output log's messages (`hacivat-core.ftl`, Task 23; P24).
    "exec", "cursor",
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

/// Whether the UI lints apply to this file (`hacivat-ui` and the app crates).
fn ui_scope(display: &str) -> bool {
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
    let ui = ui_scope(display);
    let first_new = result.findings.len();
    let mut push = |rule: &'static str, line: u32, id: &str| {
        result
            .findings
            .push(finding(rule, Severity::Error, "", display, line, id));
    };
    // (ID, line, whether it must exist)
    let mut used: Vec<(String, u32, bool)> = Vec::new();

    let mut i = 0;
    while i < toks.len() {
        if let Some(next) = skip_test_item(&toks, i) {
            i = next;
            continue;
        }
        let line = toks[i].line;
        match &toks[i].tok {
            Tok::Str(s) => {
                // Any literal naming a defined message counts as a use; only ID-consuming
                // calls (below) make an unknown one an error.
                if en_ids.contains(s) {
                    used.push((s.clone(), line, false));
                }
            }
            Tok::Punct('+') if ui && !is(&toks, i + 1, '=') && !in_identifier_call(&toks, i) => {
                let (l, r) = (left_operand(&toks, i), right_operand(&toks, i));
                let text = Operand::Text;
                let lit_or_text = |o: Operand| matches!(o, Operand::Text | Operand::Lit);
                if (l == text && lit_or_text(r)) || (r == text && lit_or_text(l)) {
                    push("L900", line, "translated text + ..");
                }
            }
            Tok::Ident(name) => {
                // ids::<ns>::<CONST>, or <ns>::<CONST> in a file that mentions `ids`.
                if let Some((ns, konst, len)) = id_path(&toks, i, mentions_ids) {
                    used.push((id_of(&ns, &konst), line, true));
                    i += len;
                    continue;
                }
                // The first argument of an ID-consuming call is a message ID.
                let id_arg = if is_text_call(&toks, i) {
                    Some(i + 1)
                } else if name == "MsgRef" && is_path(&toks, i + 1, &["new"]) {
                    Some(i + 4)
                } else {
                    None
                };
                if let Some(open) = id_arg
                    && let Some(Tok::Str(s)) = toks.get(open + 1).map(|t| &t.tok)
                    && hacivat_l10n_schema::is_valid_id(s)
                {
                    used.push((s.clone(), line, true));
                }
                if ui {
                    if name == "format"
                        && !in_identifier_call(&toks, i)
                        && is(&toks, i + 1, '!')
                        && is(&toks, i + 2, '(')
                        && let Some(template) = format_assembly(&toks, i + 2)
                    {
                        push("L900", line, &format!("format!(\"{template}\" ..)"));
                    }
                    hardcoded(&toks, i, allow, &mut push);
                }
            }
            _ => {}
        }
        i += 1;
    }

    let mut seen = BTreeSet::new();
    let mut fresh = result.findings.split_off(first_new);
    fresh.retain(|f| seen.insert((f.rule, f.line, f.id.clone())));
    result.findings.extend(fresh);

    for (id, line, must_exist) in used {
        if must_exist && !en_ids.contains(&id) {
            result
                .findings
                .push(finding("L024", Severity::Error, "", display, line, &id));
        }
        result.used_ids.insert(id);
    }
}

/// Reports string literals (and `format!` templates with words) passed as the first argument
/// of a UI text sink: `.child(..)`, `SharedString::from(..)` / `::new_static(..)` and
/// `Label::new(.., TextSource::Fluent, ..)`.
fn hardcoded(
    toks: &[Token],
    i: usize,
    allow: &BTreeSet<String>,
    push: &mut impl FnMut(&'static str, u32, &str),
) {
    let Tok::Ident(name) = &toks[i].tok else {
        return;
    };
    if in_identifier_call(toks, i) {
        return;
    }
    let (open, needs_fluent) = match name.as_str() {
        "child" if i > 0 && is(toks, i - 1, '.') => (i + 1, false),
        "SharedString" if is_path(toks, i + 1, &["from"]) => (i + 4, false),
        "SharedString" if is_path(toks, i + 1, &["new_static"]) => (i + 4, false),
        "Label" if is_path(toks, i + 1, &["new"]) => (i + 4, true),
        _ => return,
    };
    let Some(close) = is(toks, open, '(').then(|| matching(toks, open)).flatten() else {
        return;
    };
    if needs_fluent {
        let fluent = (open..close).any(|j| {
            matches!(&toks[j].tok, Tok::Ident(n) if n == "TextSource")
                && is_path(toks, j + 1, &["Fluent"])
        });
        if !fluent {
            return;
        }
    }
    // Walk the first argument: literals outside nested calls, through `if`/`match` braces.
    let (mut parens, mut braces) = (0i32, 0i32);
    let mut j = open + 1;
    while j < close {
        match &toks[j].tok {
            Tok::Punct('(' | '[') => parens += 1,
            Tok::Punct(')' | ']') => parens -= 1,
            Tok::Punct('{') => braces += 1,
            Tok::Punct('}') => braces -= 1,
            Tok::Punct(',') if parens == 0 && braces == 0 => break,
            Tok::Str(lit) if parens == 0 => {
                if lit.chars().any(char::is_alphabetic) && !allow.contains(lit) {
                    push("L900", toks[j].line, lit);
                }
            }
            Tok::Ident(n)
                if n == "format" && parens == 0 && is(toks, j + 1, '!') && is(toks, j + 2, '(') =>
            {
                if let Some(Tok::Str(t)) = toks.get(j + 3).map(|t| &t.tok)
                    && has_words(t)
                    && !allow.contains(t)
                {
                    push("L900", toks[j].line, &format!("format!(\"{t}\" ..)"));
                }
                j = matching(toks, j + 2).unwrap_or(close);
            }
            _ => {}
        }
        j += 1;
    }
}

/// Whether token `i` sits inside the arguments of a call that takes an element identifier or a
/// key, not text: `.id(..)`, `.debug_selector(..)`, `.key_context(..)` or an `ElementId`
/// constructor (`ElementId::from(..)`, `ElementId::Name(..)`, `ElementId::NamedInteger(..)`).
fn in_identifier_call(toks: &[Token], i: usize) -> bool {
    let mut depth = 0usize;
    for j in (i.saturating_sub(400)..i).rev() {
        match toks[j].tok {
            Tok::Punct(')' | ']' | '}') => depth += 1,
            Tok::Punct(';') if depth == 0 => return false,
            Tok::Punct('(' | '[' | '{') => {
                if depth > 0 {
                    depth -= 1;
                    continue;
                }
                // Only a `(` directly after the callee is a call; `id { .. }` is a block.
                if !matches!(toks[j].tok, Tok::Punct('(')) {
                    continue;
                }
                let Some(Tok::Ident(callee)) = j.checked_sub(1).map(|k| &toks[k].tok) else {
                    continue;
                };
                let keyed = matches!(callee.as_str(), "id" | "debug_selector" | "key_context");
                let constructor = j >= 4
                    && is_colons(toks, j - 3)
                    && matches!(&toks[j - 4].tok, Tok::Ident(t) if t == "ElementId");
                if keyed || constructor {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Whether a format template has letters outside its `{..}` placeholders.
fn has_words(template: &str) -> bool {
    let mut depth = 0u32;
    template.chars().any(|c| match c {
        '{' => {
            depth += 1;
            false
        }
        '}' => {
            depth = depth.saturating_sub(1);
            false
        }
        c => depth == 0 && c.is_alphabetic(),
    })
}

/// `format!` that assembles translated text: its arguments call a text function and its
/// template has two placeholders, or one placeholder next to words. Returns the template.
fn format_assembly(toks: &[Token], open: usize) -> Option<String> {
    let Some(Tok::Str(template)) = toks.get(open + 1).map(|t| &t.tok) else {
        return None;
    };
    let close = matching(toks, open)?;
    let placeholders = template.replace("{{", "").matches('{').count();
    let calls_text = (open + 2..close).any(|j| is_text_call(toks, j));
    let assembles = placeholders >= 2 || (placeholders == 1 && has_words(template));
    (calls_text && assembles).then(|| template.clone())
}

/// Calls that always return translated text or consume message IDs, on any receiver.
const ALWAYS_TEXT_FNS: [&str; 6] = [
    "ui_text",
    "t_dynamic",
    "t_attr",
    "t_machine",
    "t_rendered",
    "t_english",
];

/// Generic names that count only on an `l10n` receiver or path.
const GENERIC_TEXT_FNS: [&str; 4] = ["t", "tr", "text", "fluent"];

/// Whether the identifier at `j` is a call of a translated-text function.
fn is_text_call(toks: &[Token], j: usize) -> bool {
    let Some(Token {
        tok: Tok::Ident(name),
        ..
    }) = toks.get(j)
    else {
        return false;
    };
    is(toks, j + 1, '(')
        && (ALWAYS_TEXT_FNS.contains(&name.as_str())
            || (GENERIC_TEXT_FNS.contains(&name.as_str()) && receiver_is_l10n(toks, j)))
}

/// `l10n.t(..)`, `self.l10n.t(..)`, `l10n(cx).t(..)`, `hacivat_l10n::t(..)`, `ids::..`.
fn receiver_is_l10n(toks: &[Token], j: usize) -> bool {
    let named = |n: &str| n.contains("l10n") || n == "ui_text" || n == "provider";
    if j >= 2 && is(toks, j - 1, '.') {
        return match &toks[j - 2].tok {
            Tok::Ident(n) => named(n),
            Tok::Punct(')') => matching_back(toks, j - 2)
                .and_then(|open| open.checked_sub(1))
                .is_some_and(|k| matches!(&toks[k].tok, Tok::Ident(n) if named(n))),
            _ => false,
        };
    }
    j >= 3
        && is_colons(toks, j - 2)
        && matches!(&toks[j - 3].tok, Tok::Ident(n) if named(n) || n == "ids")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operand {
    Text,
    Lit,
    Other,
}

/// Conversions that keep an operand's kind (`"x".to_string()`, `t(A).clone()`).
const PASS_THROUGH: [&str; 5] = ["to_string", "to_owned", "clone", "into", "as_str"];

/// The kind of the expression ending right before the `+` at `plus`.
fn left_operand(toks: &[Token], plus: usize) -> Operand {
    let mut end = plus;
    loop {
        let Some(last) = end.checked_sub(1) else {
            return Operand::Other;
        };
        if matches!(toks[last].tok, Tok::Str(_)) {
            return Operand::Lit;
        }
        if !is(toks, last, ')') {
            return Operand::Other;
        }
        let Some(open) = matching_back(toks, last) else {
            return Operand::Other;
        };
        let Some(callee) = open.checked_sub(1) else {
            return Operand::Other;
        };
        if is_text_call(toks, callee) {
            return Operand::Text;
        }
        let strips =
            matches!(&toks[callee].tok, Tok::Ident(n) if PASS_THROUGH.contains(&n.as_str()));
        if strips && callee >= 1 && is(toks, callee - 1, '.') {
            end = callee - 1;
        } else {
            return Operand::Other;
        }
    }
}

/// The kind of the expression starting right after the `+` at `plus`.
fn right_operand(toks: &[Token], plus: usize) -> Operand {
    let mut k = plus + 1;
    if is(toks, k, '&') {
        k += 1;
    }
    if is_str(toks, k) {
        return Operand::Lit;
    }
    for _ in 0..3 {
        if !matches!(
            toks.get(k),
            Some(Token {
                tok: Tok::Ident(_),
                ..
            })
        ) {
            break;
        }
        if is_text_call(toks, k) {
            return Operand::Text;
        }
        if is(toks, k + 1, '.') {
            k += 2;
        } else if is_colons(toks, k + 1) {
            k += 3;
        } else {
            break;
        }
    }
    Operand::Other
}

/// The index of the bracket opening the one at `close`.
fn matching_back(toks: &[Token], close: usize) -> Option<usize> {
    let mut depth = 0usize;
    for j in (0..=close).rev() {
        match toks[j].tok {
            Tok::Punct(')' | ']' | '}') => depth += 1,
            Tok::Punct('(' | '[' | '{') => {
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

    const UI: &str = "crates/ui/hacivat-ui/src/a.rs";
    const CORE: &str = "crates/core/hacivat-l10n/src/a.rs";

    fn run(src: &str, display: &str, en: &[&str], allow: &[&str]) -> Scan {
        let en: BTreeSet<String> = en.iter().map(|s| (*s).to_string()).collect();
        let allow: BTreeSet<String> = allow.iter().map(|s| (*s).to_string()).collect();
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

    fn flagged(src: &str) -> usize {
        run(src, UI, &[], &[]).findings.len()
    }

    #[test]
    fn test_modules_comments_and_chars_are_ignored() {
        let src = r#"
            // .child("Not code")
            fn view() { let _ = '"'; let _ = "hv-known"; }
            #[cfg(test)]
            mod tests { fn t() { x.child("Test only"); l10n.t_dynamic("hv-nope"); } }
        "#;
        let s = run(src, UI, &["hv-known"], &[]);
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
        let s = run(src, UI, &[], &["orders-prod"]);
        assert_eq!(
            rules(&s),
            [("L900", "Open"), ("L900", "Close"), ("L900", "Save")]
        );
        assert!(run(src, CORE, &[], &[]).findings.is_empty());
        assert_eq!(
            run(src, "crates/app/hacivat-app/src/a.rs", &[], &[])
                .findings
                .len(),
            4
        );
    }

    #[test]
    fn literal_forms_in_the_first_argument_are_flagged() {
        assert_eq!(flagged(r#"fn f() { a.child("Open".into()); }"#), 1);
        assert_eq!(flagged(r#"fn f() { a.child("Open".to_string()); }"#), 1);
        assert_eq!(
            flagged(r#"fn f() { a.child(if c { "A" } else { "B" }); }"#),
            2
        );
        assert_eq!(
            flagged(r#"fn f() { SharedString::new_static("Static"); }"#),
            1
        );
        assert_eq!(flagged(r#"fn f() { a.child(format!("{} rows", n)); }"#), 1);
        assert_eq!(
            flagged(r#"fn f() { a.child(match k { K::A => "One", _ => "Two" }); }"#),
            2
        );
        // Nested calls, ids and non-words are not UI text.
        assert_eq!(
            flagged(r#"fn f() { a.child(div().id("save").child(x)); }"#),
            0
        );
        assert_eq!(flagged(r#"fn f() { a.child(format!("{}", n)); }"#), 0);
        assert_eq!(flagged(r#"fn f() { a.child("×".into()); }"#), 0);
    }

    #[test]
    fn element_identifiers_are_not_ui_text() {
        for src in [
            // The exact patterns of hacivat-ui panes.rs.
            r#"fn f() { div().id(ElementId::from((SharedString::from("tab-strip"), pane.0 as usize))); }"#,
            r#"fn f() { div().id(ElementId::from((SharedString::from("tab"), n))); }"#,
            r#"fn f() { div().id(ElementId::from((SharedString::from("tab-close"), n))); }"#,
            r#"fn f() { let id = ElementId::from((SharedString::from("tab"), n)); }"#,
            r#"fn f() { ElementId::Name(SharedString::from("tab")); }"#,
            r#"fn f() { ElementId::NamedInteger(SharedString::from("tab"), 3); }"#,
            r#"fn f() { div().id(SharedString::from("tab")); }"#,
            r#"fn f() { div().id(format!("tab-{n}")); }"#,
            r#"fn f() { div().debug_selector(|| SharedString::from("tab").to_string()); }"#,
            r#"fn f() { div().key_context(SharedString::from("Editor")); }"#,
        ] {
            assert_eq!(flagged(src), 0, "{src}");
        }
        // Assembly inside an identifier call is not UI text either.
        for src in [
            r#"fn f() { div().id(format!("{} {}", l10n.t(A), b)); }"#,
            r#"fn f() { div().id(l10n.t(A) + " " + &l10n.t(B)); }"#,
        ] {
            assert_eq!(flagged(src), 0, "{src}");
        }
        assert_eq!(
            flagged(r#"fn f() { let s = format!("{} {}", l10n.t(A), b); }"#),
            1
        );
        // A block after a variable named `id` is not a call.
        assert_eq!(
            flagged(r#"fn f() { match id { _ => a.child(SharedString::from("Open")) } }"#),
            1
        );
        assert_eq!(
            flagged(r#"fn f() { if let Some(x) = id { a.child("Open"); } }"#),
            1
        );
        // Text next to an identifier is still text.
        assert_eq!(
            flagged(r#"fn f() { div().id("x").child(SharedString::from("Open")); }"#),
            1
        );
        assert_eq!(
            flagged(r#"fn f() { div().child(SharedString::from("Open")); }"#),
            1
        );
    }

    #[test]
    fn ids_paths_resolve_to_message_ids() {
        let src = "use hacivat_l10n::ids; fn f() { t(ids::cm::ERROR_UNKNOWN); t(ids::hv::NOPE_X); t(hv::_2FA); }";
        let s = run(src, CORE, &["cm-error-unknown", "hv-2fa"], &[]);
        assert_eq!(rules(&s), [("L024", "hv-nope-x")]);
        assert!(s.used_ids.contains("cm-error-unknown") && s.used_ids.contains("hv-2fa"));
    }

    #[test]
    fn only_id_consuming_calls_make_a_literal_an_id() {
        let src = r#"
            fn f() {
                route("error-page"); lookup("db-main"); a.child(x).id("hv-not-a-message");
                let _ = "cm-ok";
                l10n.t_dynamic("hv-missing"); ui_text("hui-missing"); MsgRef::new("hv-gone");
            }
        "#;
        for display in [UI, CORE] {
            let s = run(src, display, &["cm-ok"], &[]);
            let l024: Vec<&str> = s
                .findings
                .iter()
                .filter(|f| f.rule == "L024")
                .map(|f| f.id.as_str())
                .collect();
            assert_eq!(l024, ["hv-missing", "hui-missing", "hv-gone"], "{display}");
            assert!(s.used_ids.contains("cm-ok"));
        }
    }

    #[test]
    fn concatenation_of_translated_text_is_flagged() {
        for src in [
            r#"fn f() { let a = format!("{} {}", l10n.t(DELETE), name); }"#,
            r#"fn f() { let a = format!("{} rows", l10n.t(N)); }"#,
            r#"fn f() { let a = format!("{} (beta)", self.l10n.t(X)); }"#,
            r#"fn f() { let a = format!("{} {}", ui_text(A), b); }"#,
            r#"fn f() { let a = l10n.t(A) + " " + &l10n.t(B); }"#,
            r#"fn f() { let a = l10n.t(A) + &l10n.t(B); }"#,
            r#"fn f() { let a = "Delete ".to_string() + &l10n.t(B); }"#,
            r#"fn f() { let a = l10n.t(A).to_string() + "!"; }"#,
            r#"fn f() { let a = l10n(cx).t(A) + ": "; }"#,
        ] {
            let s = run(src, UI, &[], &[]);
            assert_eq!(s.findings.len(), 1, "{src}: {:?}", rules(&s));
        }
        let s = run(
            r#"fn f() { format!("{} {}", l10n.t(A), b); }"#,
            UI,
            &[],
            &[],
        );
        assert!(s.findings[0].id.contains("format!"));
    }

    #[test]
    fn plain_formatting_and_non_ui_code_are_not_concatenation() {
        for src in [
            r#"fn f() { format!("{}:{}", text(x), id); }"#,
            r#"fn f() { format!("{} rows", n); }"#,
            r#"fn f() { format!("{}", l10n.t(A)); }"#,
            r#"fn f() { let a = text(x) + "!"; let n = 1 + 2; let s = a + b; }"#,
        ] {
            assert_eq!(flagged(src), 0, "{src}");
        }
        let src = r#"fn f() { format!("{} {}", l10n.t(A), b); l10n.t(A) + " "; a.child("Open"); }"#;
        assert!(run(src, CORE, &[], &[]).findings.is_empty());
        let src = r#"fn f() { format!("{}:{}", text(x), id); }"#;
        assert!(run(src, CORE, &[], &[]).findings.is_empty());
    }
}
