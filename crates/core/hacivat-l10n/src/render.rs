//! Message rendering with per-message fallback along the UI chain (I18N-04 steps 3–9) and the
//! UI/machine sinks. Rendering runs inside the runtime's format context (`with_format_ctx`).
//!
//! Errors are classified per bundle:
//! - **Structural** (the translation's fault: a variable the English message does not use, an
//!   unresolved message/term/function reference, a cycle, too many placeables, no value):
//!   release builds discard the result and try the next bundle; debug builds return `⚠[text]`.
//! - **Caller** (a variable the English message also uses was not passed): the text is kept.
//!
//! A message missing from every bundle renders `cm-error-unknown` in release and `⟦id⟧` in
//! debug builds.

use std::collections::HashSet;

use fluent_bundle::resolver::ResolverError;
use fluent_bundle::resolver::errors::ReferenceKind;
use fluent_bundle::{FluentArgs, FluentError};
use fluent_syntax::ast;
use unic_langid::LanguageIdentifier;

use crate::args::{ArgValue, fluent_args};
use crate::format::with_format_ctx;
use crate::pseudo::wrap_message;
use crate::runtime::{Bundle, BundleSet, LocaleBundles, Runtime};

/// Where rendered text goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sink {
    /// On screen: placeables isolated with U+2068/U+2069.
    Ui,
    /// Clipboard, logs, OS surfaces: no isolation marks.
    Machine,
}

/// A rendered message.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Rendered {
    pub text: String,
    /// The bundle language the text came from.
    pub locale_used: LanguageIdentifier,
    /// Whether an earlier bundle of the chain was skipped.
    pub fell_back: bool,
}

/// Renders `id` (or its attribute `attr`) through the chain of `rt`. `None` when no bundle has
/// the message (or attribute).
pub fn render(
    rt: &Runtime,
    id: &str,
    attr: Option<&str>,
    args: Vec<(&'static str, ArgValue)>,
    sink: Sink,
) -> Option<Rendered> {
    let set: &BundleSet = &rt.bundles.0;
    let args = fluent_args(args);
    with_format_ctx(&rt.format, || {
        let last = set.locales.len().checked_sub(1)?;
        for (k, lb) in set.locales.iter().enumerate() {
            let bundle = pick(lb, sink);
            let Some(pattern) = pattern_of(bundle, id, attr) else {
                continue;
            };
            let mut errors = Vec::new();
            let text = bundle
                .format_pattern(pattern, Some(&args), &mut errors)
                .into_owned();
            let structural = errors
                .iter()
                .any(|e| is_structural(e, || english_vars(set, id, attr)));
            if structural {
                if cfg!(debug_assertions) {
                    return Some(Rendered {
                        text: format!("⚠[{text}]"),
                        locale_used: lb.lang.clone(),
                        fell_back: k > 0,
                    });
                }
                if k < last {
                    continue;
                }
            }
            let text = match (set.pseudo, sink) {
                (Some(_), Sink::Ui) => wrap_message(text),
                _ => text,
            };
            return Some(Rendered {
                text,
                locale_used: lb.lang.clone(),
                fell_back: k > 0,
            });
        }
        None
    })
}

/// Renders `id` from the `en` bundle alone, machine sink (the English search layer).
pub(crate) fn render_english(rt: &Runtime, id: &str) -> Option<String> {
    let set: &BundleSet = &rt.bundles.0;
    let en = english(set)?;
    let pattern = pattern_of(&en.machine, id, None)?;
    with_format_ctx(&rt.format, || {
        let mut errors = Vec::new();
        Some(
            en.machine
                .format_pattern(pattern, Some(&FluentArgs::new()), &mut errors)
                .into_owned(),
        )
    })
}

/// The text shown for an ID no bundle has: `⟦id⟧` in debug builds; in release builds
/// `cm-error-unknown` with `$code`, or the bare ID if that message is missing too.
pub(crate) fn render_missing(rt: &Runtime, id: &str, sink: Sink) -> String {
    if cfg!(debug_assertions) {
        return format!("⟦{id}⟧");
    }
    render(
        rt,
        "cm-error-unknown",
        None,
        vec![("code", ArgValue::Text(id.to_string()))],
        sink,
    )
    .map_or_else(|| id.to_string(), |r| r.text)
}

fn pick(lb: &LocaleBundles, sink: Sink) -> &Bundle {
    match sink {
        Sink::Ui => &lb.ui,
        Sink::Machine => &lb.machine,
    }
}

fn pattern_of<'b>(
    bundle: &'b Bundle,
    id: &str,
    attr: Option<&str>,
) -> Option<&'b ast::Pattern<&'b str>> {
    let msg = bundle.get_message(id)?;
    match attr {
        None => msg.value(),
        Some(a) => msg.get_attribute(a).map(|a| a.value()),
    }
}

fn english(set: &BundleSet) -> Option<&LocaleBundles> {
    set.locales.iter().find(|l| {
        l.lang.language.as_str() == "en" && l.lang.region.is_none() && l.lang.script.is_none()
    })
}

/// Whether `e` is the translation's fault. `english_vars` lists the variables the English
/// message uses; a missing variable English also uses is the caller's fault instead.
fn is_structural(e: &FluentError, english_vars: impl FnOnce() -> HashSet<String>) -> bool {
    match e {
        FluentError::ResolverError(ResolverError::Reference(ReferenceKind::Variable { id })) => {
            !english_vars().contains(id)
        }
        FluentError::ResolverError(
            ResolverError::Reference(_)
            | ResolverError::NoValue(_)
            | ResolverError::MissingDefault
            | ResolverError::Cyclic
            | ResolverError::TooManyPlaceables,
        ) => true,
        FluentError::Overriding { .. } | FluentError::ParserError(_) => false,
    }
}

/// The variables the English message (or attribute) uses, from the `en` bundle's AST.
fn english_vars(set: &BundleSet, id: &str, attr: Option<&str>) -> HashSet<String> {
    let mut vars = HashSet::new();
    if let Some(pattern) = english(set).and_then(|en| pattern_of(&en.machine, id, attr)) {
        pattern_vars(pattern, &mut vars);
    }
    vars
}

fn pattern_vars(p: &ast::Pattern<&str>, out: &mut HashSet<String>) {
    for el in &p.elements {
        if let ast::PatternElement::Placeable { expression } = el {
            expression_vars(expression, out);
        }
    }
}

fn expression_vars(e: &ast::Expression<&str>, out: &mut HashSet<String>) {
    match e {
        ast::Expression::Select { selector, variants } => {
            inline_vars(selector, out);
            for v in variants {
                pattern_vars(&v.value, out);
            }
        }
        ast::Expression::Inline(i) => inline_vars(i, out),
    }
}

fn inline_vars(i: &ast::InlineExpression<&str>, out: &mut HashSet<String>) {
    match i {
        ast::InlineExpression::VariableReference { id } => {
            out.insert(id.name.to_string());
        }
        ast::InlineExpression::FunctionReference { arguments, .. }
        | ast::InlineExpression::TermReference {
            arguments: Some(arguments),
            ..
        } => {
            for p in &arguments.positional {
                inline_vars(p, out);
            }
            for n in &arguments.named {
                inline_vars(&n.value, out);
            }
        }
        ast::InlineExpression::Placeable { expression } => expression_vars(expression, out),
        ast::InlineExpression::StringLiteral { .. }
        | ast::InlineExpression::NumberLiteral { .. }
        | ast::InlineExpression::MessageReference { .. }
        | ast::InlineExpression::TermReference {
            arguments: None, ..
        } => {}
    }
}
