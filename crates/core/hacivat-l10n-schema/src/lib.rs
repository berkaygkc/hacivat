//! FTL signature extraction (the `# Variables:` blocks), shared by `hacivat-l10n`'s build
//! script and `xtask l10n check` (I18N-01 step 7, I18N-04 step 1, I18N-17 L003/L004/L006).
//!
//! A documented variable looks like this (the type is one of `String`, `Number`, `Decimal`,
//! `DateTime`, `List` or `Enum(a, b, ...)`):
//!
//! ```ftl
//! # Variables:
//! #   $count (Number) - number of rows fetched so far.
//! hv-status-rows-fetched = { $count } rows
//! ```

use std::fmt;

use fluent_syntax::ast;
use fluent_syntax::parser;

/// The documented type of a message variable.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum VarType {
    String,
    /// A count: an integer that selects plural variants.
    Number,
    /// An exact decimal passed as text (`ExactDecimal`), never through `f64`.
    Decimal,
    DateTime,
    List,
    /// One of a closed set of values (`Enum(table, view)`), passed as text.
    Enum(Vec<String>),
}

/// The English signature of one message.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Signature {
    pub id: String,
    /// The documented variables, in documentation order.
    pub vars: Vec<(String, VarType)>,
    /// Attribute names, in source order.
    pub attrs: Vec<String>,
    /// Variables used as (or inside) a select expression's selector.
    pub selector_vars: Vec<String>,
}

/// A problem in one FTL file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SchemaError {
    pub file: String,
    /// 1-based.
    pub line: u32,
    pub kind: SchemaErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SchemaErrorKind {
    /// Text the Fluent parser could not read (L003).
    Junk,
    /// A message ID outside the grammar `^(cm|hui|hv|hc|kg|ps|pw|pe|db|error)-[a-z0-9]+(-[a-z0-9]+)*$`
    /// (L004).
    InvalidId { id: String },
    /// The same message ID twice in one file.
    DuplicateId { id: String },
    /// A variable used by the message but missing from its `# Variables:` block (L006).
    UndocumentedVariable { id: String, var: String },
    /// A `# Variables:` line that is not `$name (Type) - description` with a known type (L006).
    BadVariableDoc { id: String, text: String },
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: ", self.file, self.line)?;
        match &self.kind {
            SchemaErrorKind::Junk => write!(f, "unparsable Fluent text"),
            SchemaErrorKind::InvalidId { id } => {
                write!(f, "message ID `{id}` breaks the ID grammar")
            }
            SchemaErrorKind::DuplicateId { id } => write!(f, "message ID `{id}` is defined twice"),
            SchemaErrorKind::UndocumentedVariable { id, var } => write!(
                f,
                "`{id}` uses `${var}`, which its `# Variables:` block does not document"
            ),
            SchemaErrorKind::BadVariableDoc { id, text } => write!(
                f,
                "`{id}` has a malformed `# Variables:` line `{text}` (want `$name (String|Number|Decimal|DateTime|List|Enum(a, b)) - description`)"
            ),
        }
    }
}

impl std::error::Error for SchemaError {}

/// The message signatures of `ftl`, or every problem found. `file` names the file in errors.
pub fn extract(ftl: &str, file: &str) -> Result<Vec<Signature>, Vec<SchemaError>> {
    let (sigs, errors) = extract_partial(ftl, file);
    if errors.is_empty() {
        Ok(sigs)
    } else {
        Err(errors)
    }
}

/// Like [`extract`], but returns the signatures it could read alongside the errors, so one bad
/// message does not hide the rest (`xtask l10n check`). A message with an error has no
/// signature.
pub fn extract_partial(ftl: &str, file: &str) -> (Vec<Signature>, Vec<SchemaError>) {
    let mut errors = Vec::new();
    let resource = match parser::parse(ftl) {
        Ok(r) => r,
        Err((r, parse_errors)) => {
            let mut lines: Vec<u32> = parse_errors
                .iter()
                .map(|e| junk_line(ftl, e.slice.clone().unwrap_or(e.pos.clone()).start))
                .collect();
            lines.dedup();
            errors.extend(lines.into_iter().map(|line| SchemaError {
                file: file.to_string(),
                line,
                kind: SchemaErrorKind::Junk,
            }));
            r
        }
    };

    let mut sigs: Vec<Signature> = Vec::new();
    for entry in &resource.body {
        let ast::Entry::Message(msg) = entry else {
            continue;
        };
        let id = msg.id.name;
        let line = message_line(ftl, id);
        let err = |kind| SchemaError {
            file: file.to_string(),
            line,
            kind,
        };
        if !is_valid_id(id) {
            errors.push(err(SchemaErrorKind::InvalidId { id: id.to_string() }));
            continue;
        }
        if sigs.iter().any(|s| s.id == id) {
            errors.push(err(SchemaErrorKind::DuplicateId { id: id.to_string() }));
            continue;
        }

        let mut used = Vars::default();
        if let Some(value) = &msg.value {
            used.pattern(value);
        }
        for attr in &msg.attributes {
            used.pattern(&attr.value);
        }

        let (documented, doc_errors) = documented_vars(msg.comment.as_ref());
        let mut ok = true;
        for text in doc_errors {
            errors.push(err(SchemaErrorKind::BadVariableDoc {
                id: id.to_string(),
                text,
            }));
            ok = false;
        }
        for var in &used.all {
            if !documented.iter().any(|(name, _)| name == var) {
                errors.push(err(SchemaErrorKind::UndocumentedVariable {
                    id: id.to_string(),
                    var: var.clone(),
                }));
                ok = false;
            }
        }
        if ok {
            sigs.push(Signature {
                id: id.to_string(),
                vars: documented,
                attrs: msg
                    .attributes
                    .iter()
                    .map(|a| a.id.name.to_string())
                    .collect(),
                selector_vars: used.selectors,
            });
        }
    }
    (sigs, errors)
}

/// Whether `id` follows the message ID grammar
/// `^(cm|hui|hv|hc|kg|ps|pw|pe|db|error)-[a-z0-9]+(-[a-z0-9]+)*$`.
pub fn is_valid_id(id: &str) -> bool {
    const PREFIXES: [&str; 10] = [
        "cm", "hui", "hv", "hc", "kg", "ps", "pw", "pe", "db", "error",
    ];
    let Some((prefix, rest)) = id.split_once('-') else {
        return false;
    };
    PREFIXES.contains(&prefix)
        && rest.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// The 1-based line of the first non-blank character at or after `offset`.
fn junk_line(src: &str, offset: usize) -> u32 {
    let offset = offset.min(src.len());
    let start = src[offset..]
        .find(|c: char| !c.is_whitespace())
        .map_or(offset, |i| offset + i);
    line_of(src, start)
}

fn line_of(src: &str, offset: usize) -> u32 {
    let newlines = src.as_bytes()[..offset]
        .iter()
        .filter(|&&b| b == b'\n')
        .count();
    u32::try_from(newlines + 1).unwrap_or(u32::MAX)
}

/// The 1-based line where message `id` is defined (the AST has no spans).
fn message_line(src: &str, id: &str) -> u32 {
    let mut offset = 0;
    for line in src.split_inclusive('\n') {
        if let Some(rest) = line.strip_prefix(id)
            && rest.trim_start_matches([' ', '\t']).starts_with('=')
        {
            return line_of(src, offset);
        }
        offset += line.len();
    }
    1
}

/// The variables documented in a message comment, plus the malformed `# Variables:` lines.
fn documented_vars(comment: Option<&ast::Comment<&str>>) -> (Vec<(String, VarType)>, Vec<String>) {
    let mut vars = Vec::new();
    let mut bad = Vec::new();
    let Some(comment) = comment else {
        return (vars, bad);
    };
    let mut lines = comment.content.iter().map(|l| l.trim());
    if lines.by_ref().all(|l| l != "Variables:") {
        return (vars, bad);
    }
    for line in lines {
        if !line.starts_with('$') {
            break;
        }
        match parse_var_doc(line) {
            Some(var) if !vars.iter().any(|(n, _)| *n == var.0) => vars.push(var),
            _ => bad.push(line.to_string()),
        }
    }
    (vars, bad)
}

/// Parses `$name (Type) - description`.
fn parse_var_doc(line: &str) -> Option<(String, VarType)> {
    let rest = line.strip_prefix('$')?;
    let name_len = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .unwrap_or(rest.len());
    let (name, rest) = rest.split_at(name_len);
    if !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return None;
    }
    let rest = rest.trim_start().strip_prefix('(')?;
    let mut depth = 1usize;
    let close = rest.char_indices().find_map(|(i, c)| {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        (depth == 0).then_some(i)
    })?;
    let ty = match rest[..close].trim() {
        "String" => VarType::String,
        "Number" => VarType::Number,
        "Decimal" => VarType::Decimal,
        "DateTime" => VarType::DateTime,
        "List" => VarType::List,
        other => {
            let inner = other.strip_prefix("Enum(")?.strip_suffix(')')?;
            let values: Vec<String> = inner
                .split([',', '|'])
                .map(|v| v.trim().trim_matches('"').to_string())
                .collect();
            if values.iter().any(String::is_empty) {
                return None;
            }
            VarType::Enum(values)
        }
    };
    Some((name.to_string(), ty))
}

/// Variables referenced by a pattern, in first-use order.
#[derive(Default)]
struct Vars {
    all: Vec<String>,
    selectors: Vec<String>,
}

impl Vars {
    fn pattern(&mut self, p: &ast::Pattern<&str>) {
        for el in &p.elements {
            if let ast::PatternElement::Placeable { expression } = el {
                self.expression(expression);
            }
        }
    }

    fn expression(&mut self, e: &ast::Expression<&str>) {
        match e {
            ast::Expression::Select { selector, variants } => {
                self.inline(selector);
                let mut selector_vars = Vars::default();
                selector_vars.inline(selector);
                for v in selector_vars.all {
                    push_unique(&mut self.selectors, v);
                }
                for variant in variants {
                    self.pattern(&variant.value);
                }
            }
            ast::Expression::Inline(i) => self.inline(i),
        }
    }

    fn inline(&mut self, i: &ast::InlineExpression<&str>) {
        match i {
            ast::InlineExpression::VariableReference { id } => {
                push_unique(&mut self.all, id.name.to_string());
            }
            ast::InlineExpression::FunctionReference { arguments, .. } => self.call(arguments),
            ast::InlineExpression::TermReference {
                arguments: Some(arguments),
                ..
            } => self.call(arguments),
            ast::InlineExpression::Placeable { expression } => self.expression(expression),
            ast::InlineExpression::StringLiteral { .. }
            | ast::InlineExpression::NumberLiteral { .. }
            | ast::InlineExpression::MessageReference { .. }
            | ast::InlineExpression::TermReference {
                arguments: None, ..
            } => {}
        }
    }

    fn call(&mut self, args: &ast::CallArguments<&str>) {
        for p in &args.positional {
            self.inline(p);
        }
        for n in &args.named {
            self.inline(&n.value);
        }
    }
}

fn push_unique(v: &mut Vec<String>, s: String) {
    if !v.contains(&s) {
        v.push(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(r: Result<Vec<Signature>, Vec<SchemaError>>) -> Vec<(u32, SchemaErrorKind)> {
        r.unwrap_err()
            .into_iter()
            .map(|e| (e.line, e.kind))
            .collect()
    }

    #[test]
    fn documented_variables_become_the_signature() {
        let ftl = "\
## group

# Shown while rows stream in.
# Variables:
#   $count (Number) - number of rows fetched so far.
#   $elapsed (String) - pre-formatted duration, e.g. \"1,2 sn\".
#   $kind (Enum(table, view)) - what was read.
# MaxLength: 40
hv-status-rows-fetched = { $count ->
        [one] One row in { $elapsed }
       *[other] { $count } rows in { $elapsed }
    }
    .title = { $kind }
hv-plain = Plain
";
        let sigs = extract(ftl, "en/hacivat-ui.ftl").unwrap();
        assert_eq!(
            sigs,
            vec![
                Signature {
                    id: "hv-status-rows-fetched".into(),
                    vars: vec![
                        ("count".into(), VarType::Number),
                        ("elapsed".into(), VarType::String),
                        (
                            "kind".into(),
                            VarType::Enum(vec!["table".into(), "view".into()])
                        ),
                    ],
                    attrs: vec!["title".into()],
                    selector_vars: vec!["count".into()],
                },
                Signature {
                    id: "hv-plain".into(),
                    vars: vec![],
                    attrs: vec![],
                    selector_vars: vec![],
                },
            ]
        );
    }

    #[test]
    fn undocumented_variable_is_an_error_with_its_line() {
        let ftl = "hv-a = A\n\nhv-saved = Connection { $name } saved.\n";
        assert_eq!(
            kinds(extract(ftl, "en/hacivat-ui.ftl")),
            vec![(
                3,
                SchemaErrorKind::UndocumentedVariable {
                    id: "hv-saved".into(),
                    var: "name".into()
                }
            )]
        );
    }

    #[test]
    fn variables_inside_functions_and_nested_placeables_count() {
        let ftl = "hv-a = { NUMBER($n, minimumFractionDigits: 1) } { LIST($d) } { { $c } }\n";
        let errs = kinds(extract(ftl, "en/hacivat-ui.ftl"));
        let vars: Vec<String> = errs
            .into_iter()
            .map(|(_, k)| match k {
                SchemaErrorKind::UndocumentedVariable { var, .. } => var,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(vars, ["n", "d", "c"]);
    }

    #[test]
    fn bad_docs_junk_ids_and_duplicates_are_reported_and_the_rest_survives() {
        let ftl = "\
# Variables:
#   $n (Integer) - not a type
hv-bad-doc = { $n }
hv-ok = OK
HV-Title = X
hv-ok = again


!!junk
";
        let (sigs, errs) = extract_partial(ftl, "en/hacivat-ui.ftl");
        assert_eq!(
            sigs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["hv-ok"]
        );
        let got: Vec<(u32, SchemaErrorKind)> = errs.into_iter().map(|e| (e.line, e.kind)).collect();
        assert_eq!(
            got,
            vec![
                (9, SchemaErrorKind::Junk),
                (
                    3,
                    SchemaErrorKind::BadVariableDoc {
                        id: "hv-bad-doc".into(),
                        text: "$n (Integer) - not a type".into()
                    }
                ),
                (
                    3,
                    SchemaErrorKind::UndocumentedVariable {
                        id: "hv-bad-doc".into(),
                        var: "n".into()
                    }
                ),
                (
                    5,
                    SchemaErrorKind::InvalidId {
                        id: "HV-Title".into()
                    }
                ),
                (4, SchemaErrorKind::DuplicateId { id: "hv-ok".into() }),
            ]
        );
    }

    #[test]
    fn decimal_is_a_variable_type() {
        let ftl = "# Variables:\n#   $seconds (Decimal) - elapsed time.\nhv-clock = { NUMBER($seconds, minimumFractionDigits: 2) } s\n";
        let sigs = extract(ftl, "en/hacivat-ui.ftl").unwrap();
        assert_eq!(
            sigs[0].vars,
            vec![("seconds".to_string(), VarType::Decimal)]
        );
        assert!(extract("# Variables:\n#   $s (decimal) - x\nhv-c = { $s }\n", "f").is_err());
    }

    #[test]
    fn id_grammar() {
        for ok in [
            "cm-error-unknown",
            "hv-tab-console-untitled",
            "hui-find-2",
            "error-x",
        ] {
            assert!(is_valid_id(ok), "{ok}");
        }
        for bad in ["HV-Title", "hv", "hv-", "hv--a", "xx-a", "hv-a_b", "hv-Ab"] {
            assert!(!is_valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn errors_display_file_and_line() {
        let e = SchemaError {
            file: "en/hacivat-ui.ftl".into(),
            line: 3,
            kind: SchemaErrorKind::UndocumentedVariable {
                id: "hv-saved".into(),
                var: "name".into(),
            },
        };
        assert_eq!(
            e.to_string(),
            "en/hacivat-ui.ftl:3: `hv-saved` uses `$name`, which its `# Variables:` block does not document"
        );
    }
}
