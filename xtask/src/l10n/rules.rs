//! Catalog rules of I18N-17 (L001-L021, L029): file hygiene, manifests, and the per-message
//! comparison of every locale against English.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use fluent_syntax::ast;
use fluent_syntax::parser;
use hacivat_l10n_schema::{Signature, VarType};
use icu_locale::Locale;
use icu_normalizer::ComposingNormalizerBorrowed;
use icu_plurals::{PluralCategory, PluralRules};

use super::{Finding, Severity};

/// Largest allowed catalog file (L002).
const MAX_FILE_BYTES: usize = 1 << 20;

/// The message ID prefixes each domain file may define (I18N-01 step 2).
pub fn domain_prefixes(domain: &str) -> Option<&'static [&'static str]> {
    Some(match domain {
        "common" => &["cm"],
        "hui" => &["hui"],
        "hacivat-ui" => &["hv"],
        "hacivat-core" => &["hc", "db", "error"],
        "karagoz-cli" => &["kg"],
        "perde-server" => &["ps", "db", "error"],
        "perde-web" => &["pw"],
        "perde-email" => &["pe"],
        _ => return None,
    })
}

/// Pseudo-locale folders that must never be committed (L029).
const PSEUDO_FOLDERS: [&str; 3] = ["en-XA", "tr-XT", "ar-XB"];

const CATEGORY_NAMES: [&str; 6] = ["zero", "one", "two", "few", "many", "other"];

/// One message as the rules see it.
#[derive(Debug, Default, Clone)]
pub struct Msg {
    pub id: String,
    pub line: u32,
    pub has_value: bool,
    pub attrs: Vec<String>,
    /// Every variable the message uses, in any attribute or variant.
    pub vars: BTreeSet<String>,
    pub selectors: Vec<Selector>,
    /// Markup tags with their largest per-variant count (L017).
    pub markup: BTreeMap<String, usize>,
    /// Message references (`{ cm-other }`), used by L015 and L025.
    pub refs: Vec<String>,
    /// Term references (`{ -brand }`, without the dash), used by L015.
    pub terms: Vec<String>,
    /// `FUNCTION(option)` pairs outside the option allow-list (L007).
    pub bad_options: Vec<String>,
    /// The plain text of the value (placeables dropped), the access-key label (L023).
    pub value_text: String,
    /// The text of the `.accesskey` attribute (L023).
    pub accesskey: Option<String>,
}

/// A select expression that may be a plural select.
#[derive(Debug, Clone)]
pub struct Selector {
    pub var: Option<String>,
    pub number_fn: bool,
    pub ordinal: bool,
    /// Identifier keys (`one`, `few`), not numeric literals.
    pub keys: Vec<String>,
}

/// One parsed `.ftl` file.
pub struct FtlFile {
    pub domain: String,
    /// Path relative to the catalog root, `/`-separated (`tr/common.ftl`).
    pub rel: String,
    pub src: String,
    pub msgs: Vec<Msg>,
    /// Terms defined in this file (without the dash).
    pub terms: BTreeSet<String>,
    pub signatures: Vec<Signature>,
}

/// Reads `path` as a catalog file, reporting L002 problems. `None` when it is not UTF-8 text.
pub fn read_text(path: &Path, rel: &str, locale: &str, out: &mut Vec<Finding>) -> Option<String> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            out.push(finding(
                "L002",
                Severity::Error,
                locale,
                rel,
                0,
                &format!("unreadable: {e}"),
            ));
            return None;
        }
    };
    if bytes.len() > MAX_FILE_BYTES {
        out.push(finding(
            "L002",
            Severity::Error,
            locale,
            rel,
            0,
            "file larger than 1 MB",
        ));
    }
    let Ok(text) = String::from_utf8(bytes) else {
        out.push(finding(
            "L002",
            Severity::Error,
            locale,
            rel,
            0,
            "not valid UTF-8",
        ));
        return None;
    };
    if text.starts_with('\u{FEFF}') {
        out.push(finding(
            "L002",
            Severity::Error,
            locale,
            rel,
            1,
            "byte order mark",
        ));
    }
    let nfc = ComposingNormalizerBorrowed::new_nfc();
    let mut crlf = false;
    let mut nfd = false;
    for (n, line) in text.split('\n').enumerate() {
        let line_no = u32::try_from(n + 1).unwrap_or(u32::MAX);
        if line.ends_with('\r') && !crlf {
            crlf = true;
            out.push(finding(
                "L002",
                Severity::Error,
                locale,
                rel,
                line_no,
                "CRLF line ending",
            ));
        }
        if !nfd && !nfc.is_normalized(line) {
            nfd = true;
            out.push(finding(
                "L002",
                Severity::Error,
                locale,
                rel,
                line_no,
                "not NFC",
            ));
        }
    }
    Some(text)
}

/// L018 (forbidden characters) on every line, and L021 (Turkish suffix after a placeable) for
/// Turkish.
pub fn scan_text(text: &str, rel: &str, locale: &str, out: &mut Vec<Finding>) {
    let turkish = locale == "tr" || locale.starts_with("tr-");
    for (n, line) in text.split('\n').enumerate() {
        let line_no = u32::try_from(n + 1).unwrap_or(u32::MAX);
        for (i, c) in line.char_indices() {
            let bad = matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
                || ((c as u32) < 0x20 && c != '\r')
                || (c == '\u{FEFF}' && !(n == 0 && i == 0));
            if bad {
                let detail = format!("U+{:04X}", c as u32);
                out.push(finding(
                    "L018",
                    Severity::Error,
                    locale,
                    rel,
                    line_no,
                    &detail,
                ));
            }
        }
        for needle in ["<script", "javascript:"] {
            if contains_ascii_ci(line, needle) {
                out.push(finding(
                    "L018",
                    Severity::Error,
                    locale,
                    rel,
                    line_no,
                    needle,
                ));
            }
        }
        if turkish && !line.trim_start().starts_with('#') && suffix_after_placeable(line) {
            out.push(finding(
                "L021",
                Severity::Warning,
                locale,
                rel,
                line_no,
                "suffix after placeable",
            ));
        }
    }
}

fn contains_ascii_ci(haystack: &str, needle: &str) -> bool {
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

/// `\}\s*'[a-zçğıöşü]+` (I18N-17 L021).
fn suffix_after_placeable(line: &str) -> bool {
    let chars: Vec<char> = line.chars().collect();
    chars.iter().enumerate().any(|(i, &c)| {
        if c != '}' {
            return false;
        }
        let mut j = i + 1;
        while chars.get(j).is_some_and(|c| c.is_whitespace()) {
            j += 1;
        }
        if !matches!(chars.get(j), Some('\'' | '\u{2019}')) {
            return false;
        }
        chars
            .get(j + 1)
            .is_some_and(|c| c.is_ascii_lowercase() || "çğıöşü".contains(*c))
    })
}

/// L001: the `locale.toml` manifest of folder `folder`.
pub fn check_manifest(text: &str, folder: &str, rel: &str, out: &mut Vec<Finding>) {
    let mut err = |detail: &str| out.push(finding("L001", Severity::Error, folder, rel, 1, detail));
    let table: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(_) => {
            err("not valid TOML");
            return;
        }
    };
    let nfc = ComposingNormalizerBorrowed::new_nfc();
    for key in [
        "tag",
        "native_name",
        "english_name",
        "short_name",
        "direction",
        "status",
    ] {
        match table.get(key).and_then(toml::Value::as_str) {
            None | Some("") => err(&format!("`{key}` missing or not text")),
            Some(v) => {
                let allowed: &[&str] = match key {
                    "direction" => &["ltr", "rtl"],
                    "status" => &["stable", "beta", "experimental"],
                    _ => &[],
                };
                if !allowed.is_empty() && !allowed.contains(&v) {
                    err(&format!("`{key}` must be one of {allowed:?}"));
                }
                if key == "native_name" && !nfc.is_normalized(v) {
                    err("`native_name` is not NFC");
                }
                if key == "tag" && v != folder {
                    err("`tag` differs from the folder name");
                }
            }
        }
    }
    if canonical_tag(folder).is_none() {
        err("folder name is not a canonical BCP 47 tag");
    }
}

/// `folder` as a locale when it is a canonical BCP 47 tag.
fn canonical_tag(folder: &str) -> Option<Locale> {
    let mut locale = Locale::try_from_str(folder).ok()?;
    if !locale.extensions.is_empty() {
        return None;
    }
    icu_locale::LocaleCanonicalizer::new_extended().canonicalize(&mut locale);
    (locale.to_string() == folder).then_some(locale)
}

/// Whether `folder` is a committed pseudo-locale (L029).
pub fn is_pseudo(folder: &str) -> bool {
    PSEUDO_FOLDERS
        .iter()
        .any(|p| p.eq_ignore_ascii_case(folder))
}

/// Parses `src`, reporting L003/L004/L005/L006 (L006 only for English, where docs live).
pub fn parse_ftl(
    src: String,
    domain: &str,
    rel: &str,
    locale: &str,
    out: &mut Vec<Finding>,
) -> FtlFile {
    use hacivat_l10n_schema::SchemaErrorKind as K;
    let (signatures, errors) = hacivat_l10n_schema::extract_partial(&src, rel);
    for e in errors {
        let (rule, id) = match &e.kind {
            K::Junk => ("L003", String::new()),
            K::InvalidId { id } => ("L004", id.clone()),
            K::DuplicateId { id } => ("L005", id.clone()),
            K::UndocumentedVariable { id, .. } | K::BadVariableDoc { id, .. } => {
                if locale != "en" {
                    continue;
                }
                ("L006", id.clone())
            }
        };
        out.push(Finding {
            rule,
            severity: Severity::Error,
            locale: locale.to_string(),
            file: rel.to_string(),
            line: e.line,
            id,
        });
    }
    let resource = match parser::parse(src.as_str()) {
        Ok(r) | Err((r, _)) => r,
    };
    let terms = resource
        .body
        .iter()
        .filter_map(|entry| match entry {
            ast::Entry::Term(t) => Some(t.id.name.to_string()),
            _ => None,
        })
        .collect();
    let msgs = resource
        .body
        .iter()
        .filter_map(|entry| match entry {
            ast::Entry::Message(m) => Some(analyze(&src, m)),
            _ => None,
        })
        .collect();
    FtlFile {
        domain: domain.to_string(),
        rel: rel.to_string(),
        msgs,
        terms,
        signatures,
        src: String::new(),
    }
}

fn analyze(src: &str, m: &ast::Message<&str>) -> Msg {
    let id = m.id.name.to_string();
    let mut msg = Msg {
        line: message_line(src, &id),
        has_value: m.value.is_some(),
        attrs: m.attributes.iter().map(|a| a.id.name.to_string()).collect(),
        id,
        ..Msg::default()
    };
    msg.value_text = m.value.as_ref().map(plain_text).unwrap_or_default();
    msg.accesskey = m
        .attributes
        .iter()
        .find(|a| a.id.name == "accesskey")
        .map(|a| plain_text(&a.value).trim().to_string());
    let mut patterns: Vec<&ast::Pattern<&str>> = m.value.iter().collect();
    patterns.extend(m.attributes.iter().map(|a| &a.value));
    for p in patterns {
        walk_pattern(p, &mut msg);
        for (tag, n) in markup_counts(p) {
            let slot = msg.markup.entry(tag).or_insert(0);
            *slot = (*slot).max(n);
        }
    }
    msg
}

/// The text elements of a pattern, placeables dropped.
fn plain_text(p: &ast::Pattern<&str>) -> String {
    p.elements
        .iter()
        .filter_map(|el| match el {
            ast::PatternElement::TextElement { value } => Some(*value),
            ast::PatternElement::Placeable { .. } => None,
        })
        .collect()
}

fn walk_pattern(p: &ast::Pattern<&str>, msg: &mut Msg) {
    for el in &p.elements {
        if let ast::PatternElement::Placeable { expression } = el {
            walk_expression(expression, msg);
        }
    }
}

fn walk_expression(e: &ast::Expression<&str>, msg: &mut Msg) {
    match e {
        ast::Expression::Inline(i) => walk_inline(i, msg),
        ast::Expression::Select { selector, variants } => {
            walk_inline(selector, msg);
            let (var, number_fn, ordinal) = describe_selector(selector);
            let keys = variants
                .iter()
                .filter_map(|v| match &v.key {
                    ast::VariantKey::Identifier { name } => Some((*name).to_string()),
                    ast::VariantKey::NumberLiteral { .. } => None,
                })
                .collect();
            msg.selectors.push(Selector {
                var,
                number_fn,
                ordinal,
                keys,
            });
            for v in variants {
                walk_pattern(&v.value, msg);
            }
        }
    }
}

/// `(variable, is NUMBER(..), is ordinal)` of a selector.
fn describe_selector(s: &ast::InlineExpression<&str>) -> (Option<String>, bool, bool) {
    match s {
        ast::InlineExpression::VariableReference { id } => {
            (Some(id.name.to_string()), false, false)
        }
        ast::InlineExpression::FunctionReference { id, arguments } if id.name == "NUMBER" => {
            let var = arguments.positional.first().and_then(|p| match p {
                ast::InlineExpression::VariableReference { id } => Some(id.name.to_string()),
                _ => None,
            });
            let ordinal = arguments.named.iter().any(|n| {
                n.name.name == "type"
                    && matches!(&n.value, ast::InlineExpression::StringLiteral { value } if *value == "ordinal")
            });
            (var, true, ordinal)
        }
        _ => (None, false, false),
    }
}

fn walk_inline(i: &ast::InlineExpression<&str>, msg: &mut Msg) {
    match i {
        ast::InlineExpression::VariableReference { id } => {
            msg.vars.insert(id.name.to_string());
        }
        ast::InlineExpression::FunctionReference { id, arguments } => {
            check_options(id.name, arguments, msg);
            walk_call(arguments, msg);
        }
        ast::InlineExpression::TermReference { id, arguments, .. } => {
            msg.terms.push(id.name.to_string());
            if let Some(a) = arguments {
                walk_call(a, msg);
            }
        }
        ast::InlineExpression::MessageReference { id, .. } => msg.refs.push(id.name.to_string()),
        ast::InlineExpression::Placeable { expression } => walk_expression(expression, msg),
        ast::InlineExpression::StringLiteral { .. }
        | ast::InlineExpression::NumberLiteral { .. } => {}
    }
}

/// The named options each built-in function accepts (L007).
fn allowed_options(function: &str) -> Option<&'static [&'static str]> {
    Some(match function {
        "NUMBER" => &[
            "minimumFractionDigits",
            "maximumFractionDigits",
            "minimumIntegerDigits",
            "minimumSignificantDigits",
            "maximumSignificantDigits",
            "useGrouping",
            "style",
            "currency",
            "currencyDisplay",
            "type",
        ],
        "DATETIME" => &[
            "dateStyle",
            "timeStyle",
            "year",
            "month",
            "day",
            "weekday",
            "hour",
            "minute",
            "second",
            "hour12",
            "hourCycle",
            "timeZoneName",
            "era",
        ],
        "LIST" => &["type", "style"],
        _ => return None,
    })
}

fn check_options(function: &str, args: &ast::CallArguments<&str>, msg: &mut Msg) {
    match allowed_options(function) {
        Some(allowed) => {
            for n in args
                .named
                .iter()
                .filter(|n| !allowed.contains(&n.name.name))
            {
                msg.bad_options.push(format!("{function}({})", n.name.name));
            }
        }
        None => msg.bad_options.push(format!("{function}()")),
    }
}

fn walk_call(a: &ast::CallArguments<&str>, msg: &mut Msg) {
    for p in &a.positional {
        walk_inline(p, msg);
    }
    for n in &a.named {
        walk_inline(&n.value, msg);
    }
}

/// Markup tags of a pattern; across the variants of a select the largest count wins.
fn markup_counts(p: &ast::Pattern<&str>) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for el in &p.elements {
        match el {
            ast::PatternElement::TextElement { value } => {
                for tag in markup_tags(value) {
                    *counts.entry(tag).or_insert(0) += 1;
                }
            }
            ast::PatternElement::Placeable { expression } => {
                let mut best = BTreeMap::new();
                collect_markup(expression, &mut best);
                for (tag, n) in best {
                    *counts.entry(tag).or_insert(0) += n;
                }
            }
        }
    }
    counts
}

fn collect_markup(e: &ast::Expression<&str>, best: &mut BTreeMap<String, usize>) {
    match e {
        ast::Expression::Inline(ast::InlineExpression::Placeable { expression }) => {
            collect_markup(expression, best);
        }
        ast::Expression::Inline(_) => {}
        ast::Expression::Select { variants, .. } => {
            for v in variants {
                for (tag, n) in markup_counts(&v.value) {
                    let slot = best.entry(tag).or_insert(0);
                    *slot = (*slot).max(n);
                }
            }
        }
    }
}

/// `<b>`, `</b>`, `<a data-l10n-name="x">` as normalized tokens.
fn markup_tags(text: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('>') else { break };
        let body = &rest[..end];
        let after_slash = body.strip_prefix('/').unwrap_or(body);
        if after_slash.starts_with(|c: char| c.is_ascii_alphabetic()) {
            let name: String = body
                .chars()
                .take_while(|c| *c == '/' || c.is_ascii_alphanumeric())
                .collect();
            let named = body.split("data-l10n-name=").nth(1).map(|v| {
                v.trim_start_matches(['"', '\''])
                    .chars()
                    .take_while(|c| !matches!(c, '"' | '\'' | ' '))
                    .collect::<String>()
            });
            tags.push(match named {
                Some(n) => format!("{name}[{n}]"),
                None => name,
            });
        }
        rest = &rest[end + 1..];
    }
    tags
}

/// The 1-based line where message `id` is defined.
fn message_line(src: &str, id: &str) -> u32 {
    for (n, line) in src.split('\n').enumerate() {
        if let Some(rest) = line.strip_prefix(id)
            && rest.trim_start_matches([' ', '\t']).starts_with('=')
        {
            return u32::try_from(n + 1).unwrap_or(u32::MAX);
        }
    }
    1
}

/// English: for each ID, its message plus domain, and (when well formed) its signature.
pub struct English<'a> {
    pub msgs: BTreeMap<&'a str, (&'a Msg, &'a str)>,
    pub sigs: BTreeMap<&'a str, &'a Signature>,
}

impl<'a> English<'a> {
    pub fn new(files: &'a [FtlFile]) -> English<'a> {
        let mut msgs = BTreeMap::new();
        let mut sigs = BTreeMap::new();
        for f in files {
            for m in &f.msgs {
                msgs.entry(m.id.as_str()).or_insert((m, f.domain.as_str()));
            }
            for s in &f.signatures {
                sigs.entry(s.id.as_str()).or_insert(s);
            }
        }
        English { msgs, sigs }
    }
}

/// Prefix/domain (L004) and cross-domain duplicate (L005) rules for one locale's files.
pub fn check_ids(files: &[FtlFile], locale: &str, out: &mut Vec<Finding>) {
    let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
    for f in files {
        let prefixes = domain_prefixes(&f.domain);
        if prefixes.is_none() {
            let detail = format!("unknown domain `{}`", f.domain);
            out.push(finding("L004", Severity::Error, locale, &f.rel, 1, &detail));
        }
        for m in &f.msgs {
            if let (Some(prefixes), Some((prefix, _))) = (prefixes, m.id.split_once('-'))
                && hacivat_l10n_schema::is_valid_id(&m.id)
                && !prefixes.contains(&prefix)
            {
                out.push(finding(
                    "L004",
                    Severity::Error,
                    locale,
                    &f.rel,
                    m.line,
                    &m.id,
                ));
            }
            if let Some(first) = seen.get(m.id.as_str())
                && *first != f.rel
            {
                out.push(finding(
                    "L005",
                    Severity::Error,
                    locale,
                    &f.rel,
                    m.line,
                    &m.id,
                ));
            }
            seen.entry(m.id.as_str()).or_insert(f.rel.as_str());
        }
    }
}

/// L015: message and term references must resolve in the same file or in `common`.
pub fn check_refs(files: &[FtlFile], locale: &str, out: &mut Vec<Finding>) {
    let common: Vec<&FtlFile> = files.iter().filter(|f| f.domain == "common").collect();
    let defined_msg = |f: &FtlFile, id: &str| {
        f.msgs.iter().any(|m| m.id == id)
            || common.iter().any(|c| c.msgs.iter().any(|m| m.id == id))
    };
    let defined_term =
        |f: &FtlFile, t: &str| f.terms.contains(t) || common.iter().any(|c| c.terms.contains(t));
    for f in files {
        for m in &f.msgs {
            let bad_msg = m.refs.iter().filter(|r| !defined_msg(f, r));
            let bad_term = m.terms.iter().filter(|t| !defined_term(f, t));
            for r in bad_msg
                .map(String::as_str)
                .chain(bad_term.map(String::as_str))
            {
                let detail = format!("{} -> {r}", m.id);
                out.push(finding(
                    "L015",
                    Severity::Error,
                    locale,
                    &f.rel,
                    m.line,
                    &detail,
                ));
            }
        }
    }
}

/// L023: an `.accesskey` must be one character of the label and unique in its container (the
/// messages sharing the ID without its last segment). Case folding is ASCII-only; other
/// characters must match exactly (never Turkic folding).
pub fn check_access_keys(files: &[FtlFile], locale: &str, out: &mut Vec<Finding>) {
    let mut used: BTreeSet<(String, String)> = BTreeSet::new();
    for f in files {
        for m in &f.msgs {
            let Some(key) = &m.accesskey else { continue };
            let mut chars = key.chars();
            let (Some(k), None) = (chars.next(), chars.next()) else {
                out.push(finding(
                    "L023",
                    Severity::Warning,
                    locale,
                    &f.rel,
                    m.line,
                    &m.id,
                ));
                continue;
            };
            let folded = k.to_ascii_lowercase();
            let in_label = m
                .value_text
                .chars()
                .any(|c| c.to_ascii_lowercase() == folded);
            let container =
                m.id.rsplit_once('-')
                    .map_or(m.id.as_str(), |(c, _)| c)
                    .to_string();
            let fresh = used.insert((container, folded.to_string()));
            if !in_label || !fresh {
                out.push(finding(
                    "L023",
                    Severity::Warning,
                    locale,
                    &f.rel,
                    m.line,
                    &m.id,
                ));
            }
        }
    }
}

/// Plural selectors against the locale's CLDR cardinal categories (L013, L014).
fn check_plurals(m: &Msg, file: &str, locale: &str, en: &English<'_>, out: &mut Vec<Finding>) {
    let Some(cats) = cardinal_categories(locale) else {
        return;
    };
    for sel in m.selectors.iter().filter(|s| !s.ordinal) {
        let numeric_var = sel.var.as_deref().is_some_and(|v| {
            en.sigs
                .get(m.id.as_str())
                .is_some_and(|s| s.vars.iter().any(|(n, t)| n == v && *t == VarType::Number))
        });
        if !(sel.number_fn || numeric_var) {
            continue;
        }
        let named: Vec<&str> = sel
            .keys
            .iter()
            .map(String::as_str)
            .filter(|k| CATEGORY_NAMES.contains(k))
            .collect();
        for key in &named {
            if !cats.contains(key) {
                out.push(finding(
                    "L013",
                    Severity::Error,
                    locale,
                    file,
                    m.line,
                    &m.id,
                ));
            }
        }
        // `other` is the mandatory default variant.
        if !named.is_empty() && cats.iter().any(|c| *c != "other" && !named.contains(c)) {
            out.push(finding(
                "L014",
                Severity::Warning,
                locale,
                file,
                m.line,
                &m.id,
            ));
        }
    }
}

fn cardinal_categories(locale: &str) -> Option<Vec<&'static str>> {
    let loc = Locale::try_from_str(locale).ok()?;
    let rules = PluralRules::try_new_cardinal((&loc).into()).ok()?;
    Some(
        rules
            .categories()
            .map(|c| match c {
                PluralCategory::Zero => "zero",
                PluralCategory::One => "one",
                PluralCategory::Two => "two",
                PluralCategory::Few => "few",
                PluralCategory::Many => "many",
                PluralCategory::Other => "other",
            })
            .collect(),
    )
}

/// Compares one locale's messages with English (L008-L014, L017); returns the IDs it defines.
pub fn check_locale(
    files: &[FtlFile],
    locale: &str,
    en: &English<'_>,
    out: &mut Vec<Finding>,
) -> BTreeSet<String> {
    let mut present = BTreeSet::new();
    check_refs(files, locale, out);
    check_access_keys(files, locale, out);
    for f in files {
        for m in &f.msgs {
            for bad in &m.bad_options {
                out.push(finding(
                    "L007",
                    Severity::Error,
                    locale,
                    &f.rel,
                    m.line,
                    &format!("{} {bad}", m.id),
                ));
            }
        }
        for m in f
            .msgs
            .iter()
            .filter(|m| hacivat_l10n_schema::is_valid_id(&m.id))
        {
            present.insert(m.id.clone());
            check_plurals(m, &f.rel, locale, en, out);
            if locale == "en" {
                continue;
            }
            let Some(&(source, _)) = en.msgs.get(m.id.as_str()) else {
                out.push(finding(
                    "L008",
                    Severity::Error,
                    locale,
                    &f.rel,
                    m.line,
                    &m.id,
                ));
                continue;
            };
            check_shape(m, source, &f.rel, locale, out);
            if let Some(sig) = en.sigs.get(m.id.as_str()) {
                for var in &m.vars {
                    if !sig.vars.iter().any(|(n, _)| n == var) {
                        out.push(finding(
                            "L011",
                            Severity::Error,
                            locale,
                            &f.rel,
                            m.line,
                            &m.id,
                        ));
                    }
                }
                if sig.vars.iter().any(|(n, _)| !m.vars.contains(n)) {
                    out.push(finding(
                        "L012",
                        Severity::Warning,
                        locale,
                        &f.rel,
                        m.line,
                        &m.id,
                    ));
                }
            }
            if m.markup != source.markup {
                out.push(finding(
                    "L017",
                    Severity::Error,
                    locale,
                    &f.rel,
                    m.line,
                    &m.id,
                ));
            }
        }
    }
    if locale != "en" {
        for (id, (_, domain)) in &en.msgs {
            if !present.contains(*id) {
                let file = format!("{locale}/{domain}.ftl");
                out.push(finding("L009", Severity::Warning, locale, &file, 0, id));
            }
        }
    }
    present
}

/// Value and attribute shape against English (L010).
fn check_shape(m: &Msg, en: &Msg, file: &str, locale: &str, out: &mut Vec<Finding>) {
    let mut push = |sev| out.push(finding("L010", sev, locale, file, m.line, &m.id));
    if m.has_value && !en.has_value {
        push(Severity::Error);
    } else if !m.has_value && en.has_value {
        push(Severity::Warning);
    }
    if m.attrs.iter().any(|a| !en.attrs.contains(a)) {
        push(Severity::Error);
    }
    if en.attrs.iter().any(|a| !m.attrs.contains(a)) {
        push(Severity::Warning);
    }
}

pub fn finding(
    rule: &'static str,
    severity: Severity,
    locale: &str,
    file: &str,
    line: u32,
    id: &str,
) -> Finding {
    Finding {
        rule,
        severity,
        locale: locale.to_string(),
        file: file.to_string(),
        line,
        id: id.to_string(),
    }
}
