//! UI text without a core dependency (R4): widgets resolve `hui-*` IDs through the
//! [`UiTextProvider`] installed as the [`UiText`] global; the app installs one backed by the
//! Fluent runtime (Task 13). [`EnglishDefaults`] serves `locales/en/hui.ftl` when no app is
//! present (Task 11).

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::{Arc, LazyLock};

use fluent_syntax::ast;
use hacivat_ui_core::gpui_surface::{App, Global, SharedString};

/// Renders UI messages by ID.
pub trait UiTextProvider: Send + Sync + 'static {
    fn text(&self, id: &str, args: &[(&'static str, UiArg)]) -> SharedString;
    fn attr(&self, id: &str, attr: &str) -> Option<SharedString>;
    /// Changes whenever the rendered texts may change (a language switch).
    fn generation(&self) -> u64;
}

/// A message argument.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UiArg {
    Count(u64),
    Text(SharedString),
}

/// The installed UI text provider.
#[derive(Clone)]
pub struct UiText(pub Arc<dyn UiTextProvider>);

impl std::fmt::Debug for UiText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UiText")
            .field("generation", &self.0.generation())
            .finish()
    }
}

impl Global for UiText {}

/// The message `id` from the installed provider, or `id` itself when none is installed.
pub fn ui_text(cx: &App, id: &str) -> SharedString {
    match cx.try_global::<UiText>() {
        Some(provider) => provider.0.text(id, &[]),
        None => SharedString::from(id.to_string()),
    }
}

/// English `hui-*` texts embedded from `locales/en/hui.ftl` (terms and shared messages from
/// `locales/en/common.ftl`), parsed once per process with `fluent-syntax` and no ICU.
///
/// Rendering follows the UI sink of `L10n` (Task 6), so the two agree on every `hui-*`
/// message (F26, tested by Task 13): placeables in a pattern of more than one element are
/// wrapped in U+2068/U+2069 (except message, term and string-literal references), a missing
/// variable renders `{$name}`, and plural selectors use the English cardinal rule. Functions
/// (`NUMBER`, …) render their first argument unformatted: the formatting functions live in
/// `hacivat-l10n` (Task 9) and need ICU. An unknown ID renders `⟦id⟧` in debug builds and the
/// ID itself in release builds.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnglishDefaults {
    _private: (),
}

impl EnglishDefaults {
    pub fn new() -> Self {
        LazyLock::force(&ENGLISH);
        Self::default()
    }
}

impl UiTextProvider for EnglishDefaults {
    fn text(&self, id: &str, args: &[(&'static str, UiArg)]) -> SharedString {
        match ENGLISH.messages.get(id) {
            Some(m) if m.value.is_some() => ENGLISH.format(m, None, args).into(),
            _ => unknown(id),
        }
    }

    fn attr(&self, id: &str, attr: &str) -> Option<SharedString> {
        let message = ENGLISH.messages.get(id)?;
        find_attr(&message.attributes, attr)?;
        Some(ENGLISH.format(message, Some(attr), &[]).into())
    }

    fn generation(&self) -> u64 {
        0
    }
}

fn unknown(id: &str) -> SharedString {
    if cfg!(debug_assertions) {
        format!("⟦{id}⟧").into()
    } else {
        SharedString::from(id.to_string())
    }
}

const HUI_FTL: &str = include_str!("../../../../locales/en/hui.ftl");
const COMMON_FTL: &str = include_str!("../../../../locales/en/common.ftl");

static ENGLISH: LazyLock<Catalog> = LazyLock::new(|| Catalog::parse(&[COMMON_FTL, HUI_FTL]));

const FSI: char = '\u{2068}';
const PDI: char = '\u{2069}';
/// Fluent's guard against runaway references (fluent-bundle's `MAX_PLACEABLES`).
const MAX_PLACEABLES: usize = 100;

type Pattern = ast::Pattern<&'static str>;

/// Messages and terms of the embedded English files. Junk entries are skipped (the catalog
/// checks of Task 10 reject them in CI).
#[derive(Default)]
struct Catalog {
    messages: HashMap<&'static str, ast::Message<&'static str>>,
    terms: HashMap<&'static str, ast::Term<&'static str>>,
}

/// A resolved value while formatting: a selector key or an argument.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Number(f64),
    Text(String),
    Missing,
}

#[derive(Clone, Copy)]
enum Args<'a> {
    Ui(&'a [(&'static str, UiArg)]),
    Local(&'a [(&'static str, Value)]),
}

impl Args<'_> {
    fn get(&self, name: &str) -> Value {
        match self {
            Args::Ui(args) => match args.iter().find(|(n, _)| *n == name) {
                Some((_, UiArg::Count(n))) => Value::Number(*n as f64),
                Some((_, UiArg::Text(t))) => Value::Text(t.to_string()),
                None => Value::Missing,
            },
            Args::Local(args) => args
                .iter()
                .find(|(n, _)| *n == name)
                .map_or(Value::Missing, |(_, v)| v.clone()),
        }
    }
}

struct Scope<'a> {
    args: Args<'a>,
    placeables: usize,
    /// Message and term IDs being resolved, to stop reference cycles.
    stack: Vec<&'static str>,
}

fn find_attr<'p>(attrs: &'p [ast::Attribute<&'static str>], name: &str) -> Option<&'p Pattern> {
    attrs.iter().find(|a| a.id.name == name).map(|a| &a.value)
}

impl Catalog {
    fn parse(sources: &[&'static str]) -> Self {
        let mut catalog = Catalog::default();
        for source in sources {
            let resource = match fluent_syntax::parser::parse(*source) {
                Ok(r) => r,
                Err((r, _)) => r,
            };
            for entry in resource.body {
                match entry {
                    ast::Entry::Message(m) => {
                        catalog.messages.insert(m.id.name, m);
                    }
                    ast::Entry::Term(t) => {
                        catalog.terms.insert(t.id.name, t);
                    }
                    _ => {}
                }
            }
        }
        catalog
    }

    /// Renders `message`'s value, or its attribute `attr`; empty when that pattern is absent.
    fn format(
        &self,
        message: &ast::Message<&'static str>,
        attr: Option<&str>,
        args: &[(&'static str, UiArg)],
    ) -> String {
        let pattern = match attr {
            Some(a) => find_attr(&message.attributes, a),
            None => message.value.as_ref(),
        };
        let mut scope = Scope {
            args: Args::Ui(args),
            placeables: 0,
            stack: vec![message.id.name],
        };
        let mut out = String::new();
        if let Some(pattern) = pattern {
            self.write_pattern(&mut out, pattern, &mut scope);
        }
        out
    }

    fn write_pattern(&self, w: &mut String, pattern: &Pattern, scope: &mut Scope<'_>) {
        let len = pattern.elements.len();
        for element in &pattern.elements {
            match element {
                ast::PatternElement::TextElement { value } => w.push_str(value),
                ast::PatternElement::Placeable { expression } => {
                    scope.placeables += 1;
                    if scope.placeables > MAX_PLACEABLES {
                        return;
                    }
                    let isolate = len > 1
                        && !matches!(
                            expression,
                            ast::Expression::Inline(
                                ast::InlineExpression::MessageReference { .. }
                                    | ast::InlineExpression::TermReference { .. }
                                    | ast::InlineExpression::StringLiteral { .. }
                            )
                        );
                    if isolate {
                        w.push(FSI);
                    }
                    self.write_expression(w, expression, scope);
                    if isolate {
                        w.push(PDI);
                    }
                }
            }
        }
    }

    fn write_expression(
        &self,
        w: &mut String,
        expression: &ast::Expression<&'static str>,
        scope: &mut Scope<'_>,
    ) {
        match expression {
            ast::Expression::Inline(inline) => self.write_inline(w, inline, scope),
            ast::Expression::Select { selector, variants } => {
                let key = self.resolve_value(selector, scope);
                let chosen = variants
                    .iter()
                    .find(|v| variant_matches(&v.key, &key))
                    .or_else(|| variants.iter().find(|v| v.default));
                if let Some(variant) = chosen {
                    self.write_pattern(w, &variant.value, scope);
                }
            }
        }
    }

    fn write_inline(
        &self,
        w: &mut String,
        inline: &ast::InlineExpression<&'static str>,
        scope: &mut Scope<'_>,
    ) {
        use ast::InlineExpression as I;
        match inline {
            I::StringLiteral { value } => w.push_str(&unescape(value)),
            I::NumberLiteral { value } => w.push_str(value),
            I::VariableReference { id } => match scope.args.get(id.name) {
                Value::Number(n) => {
                    let _ = write!(w, "{n}");
                }
                Value::Text(t) => w.push_str(&t),
                Value::Missing => {
                    let _ = write!(w, "{{${}}}", id.name);
                }
            },
            I::MessageReference { id, attribute } => {
                let pattern = self.messages.get(id.name).and_then(|m| match attribute {
                    Some(a) => find_attr(&m.attributes, a.name),
                    None => m.value.as_ref(),
                });
                match pattern {
                    Some(p) if !scope.stack.contains(&id.name) => {
                        scope.stack.push(id.name);
                        self.write_pattern(w, p, scope);
                        scope.stack.pop();
                    }
                    _ => {
                        let _ = match attribute {
                            Some(a) => write!(w, "{{{}.{}}}", id.name, a.name),
                            None => write!(w, "{{{}}}", id.name),
                        };
                    }
                }
            }
            I::TermReference {
                id,
                attribute,
                arguments,
            } => {
                let pattern = self.terms.get(id.name).and_then(|t| match attribute {
                    Some(a) => find_attr(&t.attributes, a.name),
                    None => Some(&t.value),
                });
                match pattern {
                    Some(p) if !scope.stack.contains(&id.name) => {
                        // A term sees only its own named arguments.
                        let local: Vec<(&'static str, Value)> = arguments
                            .iter()
                            .flat_map(|a| a.named.iter())
                            .map(|n| (n.name.name, self.literal_value(&n.value)))
                            .collect();
                        let mut inner = Scope {
                            args: Args::Local(&local),
                            placeables: scope.placeables,
                            stack: std::mem::take(&mut scope.stack),
                        };
                        inner.stack.push(id.name);
                        self.write_pattern(w, p, &mut inner);
                        inner.stack.pop();
                        scope.stack = inner.stack;
                        scope.placeables = inner.placeables;
                    }
                    _ => {
                        let _ = write!(w, "{{-{}}}", id.name);
                    }
                }
            }
            I::FunctionReference { arguments, .. } => match arguments.positional.first() {
                Some(first) => self.write_inline(w, first, scope),
                None => w.push_str("{???}"),
            },
            I::Placeable { expression } => self.write_expression(w, expression, scope),
        }
    }

    /// The value a selector compares against its variant keys.
    fn resolve_value(
        &self,
        inline: &ast::InlineExpression<&'static str>,
        scope: &mut Scope<'_>,
    ) -> Value {
        use ast::InlineExpression as I;
        match inline {
            I::VariableReference { id } => scope.args.get(id.name),
            I::FunctionReference { arguments, .. } => match arguments.positional.first() {
                Some(first) => self.resolve_value(first, scope),
                None => Value::Missing,
            },
            I::NumberLiteral { .. } | I::StringLiteral { .. } => self.literal_value(inline),
            other => {
                let mut s = String::new();
                self.write_inline(&mut s, other, scope);
                Value::Text(s)
            }
        }
    }

    fn literal_value(&self, inline: &ast::InlineExpression<&'static str>) -> Value {
        match inline {
            ast::InlineExpression::NumberLiteral { value } => {
                value.parse().map_or(Value::Missing, Value::Number)
            }
            ast::InlineExpression::StringLiteral { value } => Value::Text(unescape(value)),
            _ => Value::Missing,
        }
    }
}

fn variant_matches(key: &ast::VariantKey<&'static str>, value: &Value) -> bool {
    match (key, value) {
        (ast::VariantKey::Identifier { name }, Value::Text(t)) => name == t,
        (ast::VariantKey::Identifier { name }, Value::Number(n)) => *name == english_plural(*n),
        (ast::VariantKey::NumberLiteral { value: k }, Value::Number(n)) => {
            k.parse::<f64>().is_ok_and(|k| k == *n)
        }
        _ => false,
    }
}

/// The English cardinal plural category (CLDR: `one` for exactly 1, else `other`).
fn english_plural(n: f64) -> &'static str {
    if n == 1.0 { "one" } else { "other" }
}

/// Fluent string-literal escapes: `\\`, `\"`, `\uXXXX`, `\UXXXXXX`.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some(e @ ('u' | 'U')) => {
                let n = if e == 'u' { 4 } else { 6 };
                let hex: String = chars.by_ref().take(n).collect();
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(ch) => out.push(ch),
                    None => out.push('\u{FFFD}'),
                }
            }
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    struct Fixed;

    impl UiTextProvider for Fixed {
        fn text(&self, id: &str, _: &[(&'static str, UiArg)]) -> SharedString {
            SharedString::from(format!("<{id}>"))
        }
        fn attr(&self, _: &str, _: &str) -> Option<SharedString> {
            None
        }
        fn generation(&self) -> u64 {
            1
        }
    }

    #[gpui::test]
    fn ui_text_without_provider_returns_the_id(cx: &mut TestAppContext) {
        cx.update(|cx| assert_eq!(ui_text(cx, "hui-x"), "hui-x"));
    }

    #[gpui::test]
    fn ui_text_uses_the_installed_provider(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(UiText(Arc::new(Fixed)));
            assert_eq!(ui_text(cx, "hui-x"), "<hui-x>");
        });
    }

    const FIXTURE: &str = r#"
-brand = Hacivat
    .gender = masculine
-thing = { $case ->
    [acc] şeyi
   *[nom] şey
}
hui-plain = Plain text
hui-brand = About { -brand }
hui-term-args = Bir { -thing(case: "acc") } gördüm
hui-term-attr = { -brand.gender ->
    [masculine] he
   *[other] they
}
hui-count = { $count ->
    [0] No rows
    [one] One row
   *[other] { $count } rows
}
hui-name = Open { $name }
hui-alone = { $name }
hui-ref = See: { hui-plain }
hui-attr = Value
    .title = Title of { $name }
hui-literal = { "A\"b" }
hui-number = { NUMBER($count) } items
hui-missing = Hello { $nobody }
hui-cycle-a = { hui-cycle-b }
hui-cycle-b = { hui-cycle-a }
hui-multi =
    First line
    second line
"#;

    fn fixture() -> &'static Catalog {
        static C: LazyLock<Catalog> = LazyLock::new(|| Catalog::parse(&[FIXTURE]));
        &C
    }

    fn render(id: &str, args: &[(&'static str, UiArg)]) -> String {
        let c = fixture();
        c.format(&c.messages[id], None, args)
    }

    fn count(n: u64) -> [(&'static str, UiArg); 1] {
        [("count", UiArg::Count(n))]
    }

    fn name(s: &'static str) -> [(&'static str, UiArg); 1] {
        [("name", UiArg::Text(s.into()))]
    }

    #[test]
    fn plain_and_multiline_text() {
        assert_eq!(render("hui-plain", &[]), "Plain text");
        assert_eq!(render("hui-multi", &[]), "First line\nsecond line");
    }

    #[test]
    fn variables_are_isolated_in_patterns_of_more_than_one_element() {
        assert_eq!(
            render("hui-name", &name("orders")),
            "Open \u{2068}orders\u{2069}"
        );
        assert_eq!(render("hui-alone", &name("orders")), "orders");
    }

    #[test]
    fn references_and_literals_are_not_isolated() {
        assert_eq!(render("hui-brand", &[]), "About Hacivat");
        assert_eq!(render("hui-ref", &[]), "See: Plain text");
        assert_eq!(render("hui-literal", &[]), "A\"b");
    }

    #[test]
    fn terms_take_their_own_arguments_and_attributes() {
        assert_eq!(render("hui-term-args", &[]), "Bir şeyi gördüm");
        assert_eq!(render("hui-term-attr", &[]), "he");
    }

    #[test]
    fn plural_selects_exact_number_then_english_category_then_default() {
        assert_eq!(render("hui-count", &count(0)), "No rows");
        assert_eq!(render("hui-count", &count(1)), "One row");
        assert_eq!(render("hui-count", &count(2)), "\u{2068}2\u{2069} rows");
    }

    #[test]
    fn functions_render_their_argument_and_missing_things_render_fluent_markers() {
        assert_eq!(render("hui-number", &count(3)), "\u{2068}3\u{2069} items");
        assert_eq!(
            render("hui-missing", &[]),
            "Hello \u{2068}{$nobody}\u{2069}"
        );
        assert_eq!(render("hui-cycle-a", &[]), "{hui-cycle-a}");
    }

    #[test]
    fn attributes_render_with_their_pattern() {
        let c = fixture();
        assert_eq!(
            c.format(&c.messages["hui-attr"], Some("title"), &name("x")),
            "Title of \u{2068}x\u{2069}"
        );
    }

    #[test]
    fn embedded_catalog_has_common_terms() {
        assert_eq!(
            EnglishDefaults::new().text("cm-brand-hacivat-caps", &[]),
            "HACIVAT"
        );
    }
}
