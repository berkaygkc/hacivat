//! Generated typed IDs with `(Decimal)` arguments (R15, P20). `build.rs` reads only
//! `locales/en`, so this uses the real settings regional preview.

use hacivat_l10n::ids;
use hacivat_l10n::{
    ArgValue, Catalogs, ExactDecimal, FormatInputs, IntoFluentArgs, L10n, MsgRef, Negotiated, Sink,
    WhenValue, build_runtime,
};

/// Renders `msg` in English with format locale `locale`, machine sink.
fn render_in<A: IntoFluentArgs>(locale: &str, msg: MsgRef<A>, args: A) -> String {
    let neg = Negotiated {
        chain: vec!["en".parse().unwrap()],
        format_locale: locale.into(),
        diags: Vec::new(),
    };
    let rt = build_runtime(
        &Catalogs::embedded(),
        &neg,
        &FormatInputs::new(&neg, "UTC"),
        1,
    );
    L10n::new(rt).t_rendered(msg, args, Sink::Machine).text
}

#[test]
fn decimal_args_format_by_region() {
    let a = |n: &str| ids::hv::HvSettingsRegionalPreview {
        number: ExactDecimal(n.into()),
        when: WhenValue::Local {
            year: 2026,
            month: 11,
            day: 11,
            hour: 14,
            minute: 5,
            second: 0,
        },
    };
    assert!(
        render_in("tr-TR", ids::hv::SETTINGS_REGIONAL_PREVIEW, a("1234.5")).starts_with("1.234,5")
    );
    assert!(
        render_in("en-US", ids::hv::SETTINGS_REGIONAL_PREVIEW, a("1234.5")).starts_with("1,234.5")
    );
}

#[test]
fn decimal_args_keep_every_digit() {
    // Exact text, never through f64: 0.1 + 0.2 style noise cannot appear.
    assert_eq!(
        ArgValue::from(ExactDecimal("12345678901234567.25".into())),
        ArgValue::Decimal(ExactDecimal("12345678901234567.25".into()))
    );
    let a = ids::hv::HvSettingsRegionalPreview {
        number: ExactDecimal("12345678901234567.25".into()),
        when: WhenValue::Local {
            year: 2026,
            month: 1,
            day: 2,
            hour: 3,
            minute: 4,
            second: 0,
        },
    };
    assert!(
        render_in("en-US", ids::hv::SETTINGS_REGIONAL_PREVIEW, a)
            .starts_with("12,345,678,901,234,567.25")
    );
}
