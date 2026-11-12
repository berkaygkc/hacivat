//! L027: render every message of every locale through `hacivat-l10n` with sample arguments typed
//! from the English signature. Any render error (an unresolved reference, a cyclic or empty
//! pattern, a variable English does not have) fails.

use std::collections::BTreeMap;

use hacivat_l10n::args::{ExactDecimal, IntoFluentArgs, WhenValue};
use hacivat_l10n::{
    ArgValue, Catalogs, FormatInputs, L10n, MsgRef, Negotiated, Sink, build_runtime,
};
use hacivat_l10n_schema::VarType;

use super::rules::{English, FtlFile, finding};
use super::{Finding, Severity};

/// Dynamically built arguments.
struct Dyn(Vec<(&'static str, ArgValue)>);

impl IntoFluentArgs for Dyn {
    fn into_args(self) -> Vec<(&'static str, ArgValue)> {
        self.0
    }
}

/// The number samples of I18N-17 L027: 0, 1, 2, 5, 21, 1000000 and 1.5.
fn number_samples() -> Vec<ArgValue> {
    let mut v: Vec<ArgValue> = [0, 1, 2, 5, 21, 1_000_000].map(ArgValue::Count).to_vec();
    v.push(ArgValue::Decimal(ExactDecimal("1.5".into())));
    v
}

fn sample(ty: &VarType, number: &ArgValue, long: bool) -> ArgValue {
    match ty {
        VarType::Number => number.clone(),
        VarType::String if long => ArgValue::Text("W".repeat(120)),
        VarType::String => ArgValue::Text("Sample".into()),
        VarType::DateTime => ArgValue::When(WhenValue::Instant {
            unix_ms: 1_700_000_000_000,
        }),
        VarType::List => ArgValue::List(vec!["a".into(), "b".into(), "c".into()]),
        VarType::Enum(values) => ArgValue::Text(values.first().cloned().unwrap_or_default()),
    }
}

/// Leaks a name: the l10n API wants `&'static str`, and xtask is a short-lived process.
fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

fn failed_text(text: &str) -> bool {
    text.starts_with("\u{26A0}[")
}

/// Renders every message of `locales` (folder name to files) and reports L027.
pub fn check(locales: &BTreeMap<String, Vec<FtlFile>>, en: &English<'_>, out: &mut Vec<Finding>) {
    let raw: Vec<(&str, &str)> = locales
        .values()
        .flatten()
        .map(|f| (f.rel.as_str(), f.src.as_str()))
        .collect();
    let cat = Catalogs::from_memory(&raw);

    for (folder, files) in locales {
        let Ok(tag) = folder.parse::<hacivat_l10n::LanguageIdentifier>() else {
            continue;
        };
        let mut chain = vec![tag];
        if folder != "en" {
            chain.push("en".parse().unwrap_or_default());
        }
        let neg = Negotiated {
            chain,
            format_locale: folder.clone(),
            diags: Vec::new(),
        };
        let fmt = FormatInputs::new(&neg, "UTC");
        let l10n = L10n::new(build_runtime(&cat, &neg, &fmt, 1));

        for f in files {
            for m in &f.msgs {
                let Some(sig) = en.sigs.get(m.id.as_str()) else {
                    continue;
                };
                // A variable English lacks is reported as L011, not rendered.
                if m.vars.iter().any(|v| !sig.vars.iter().any(|(n, _)| n == v)) {
                    continue;
                }
                let id = leak(&m.id);
                let mut failed = false;
                for (k, number) in number_samples().iter().enumerate() {
                    let args = || {
                        Dyn(sig
                            .vars
                            .iter()
                            .map(|(n, t)| (leak(n), sample(t, number, k % 2 == 1)))
                            .collect())
                    };
                    if m.has_value {
                        let r = l10n.t_rendered(MsgRef::<Dyn>::new(id), args(), Sink::Ui);
                        failed |=
                            r.missing || failed_text(&r.text) || (r.fell_back && folder != "en");
                    }
                    for attr in &m.attrs {
                        let text = l10n.t_attr(MsgRef::<Dyn>::new(id), attr, args());
                        failed |= text.is_none_or(|t| failed_text(&t));
                    }
                }
                if failed {
                    out.push(finding(
                        "L027",
                        Severity::Error,
                        folder,
                        &f.rel,
                        m.line,
                        &m.id,
                    ));
                }
            }
        }
    }
}
