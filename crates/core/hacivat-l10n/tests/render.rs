//! Message resolution with per-message fallback, sinks, typed IDs and the atomic swap
//! (I18N-04, I18N-07). The format locale is `en-US` until Task 9 adds the ICU formatters.

#[cfg(not(debug_assertions))]
use std::time::{Duration, Instant};

use hacivat_l10n::{
    ArgValue, Catalogs, FormatInputs, IntoFluentArgs, L10n, LanguageIdentifier, MsgRef, Negotiated,
    Runtime, Sink, build_runtime,
};
#[cfg(not(debug_assertions))]
use hacivat_l10n::{NegotiationCtx, PrefSource, negotiate};

const EN_TOML: &str = "tag = \"en\"\nnative_name = \"English\"\nenglish_name = \"English\"\nshort_name = \"EN\"\ndirection = \"ltr\"\nstatus = \"stable\"\n";
const TR_TOML: &str = "tag = \"tr\"\nnative_name = \"Türkçe\"\nenglish_name = \"Turkish\"\nshort_name = \"TR\"\ndirection = \"ltr\"\nstatus = \"beta\"\n";

fn negotiated(chain: &[&str], format_locale: &str) -> Negotiated {
    Negotiated {
        chain: chain
            .iter()
            .map(|t| t.parse::<LanguageIdentifier>().unwrap())
            .collect(),
        format_locale: format_locale.into(),
        diags: Vec::new(),
    }
}

/// An `L10n` over in-memory catalogs: `tr` holds `(tag, hacivat-ui.ftl source)` pairs for the
/// translation, `en` the same for English.
fn l10n(chain: &[&str], tr: &[(&str, &str)], en: &[(&str, &str)]) -> L10n {
    let mut files: Vec<(String, String)> = vec![
        ("en/locale.toml".into(), EN_TOML.into()),
        ("tr/locale.toml".into(), TR_TOML.into()),
    ];
    for (tag, src) in tr.iter().chain(en) {
        files.push((format!("{tag}/hacivat-ui.ftl"), src.to_string()));
    }
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    let c = Catalogs::from_memory(&refs);
    let n = negotiated(chain, "en-US");
    L10n::new(build_runtime(&c, &n, &FormatInputs::new(&n, "UTC"), 1))
}

fn rt(generation: u64) -> Runtime {
    Runtime {
        generation,
        ..Runtime::default()
    }
}

struct NameArg(String);

impl IntoFluentArgs for NameArg {
    fn into_args(self) -> Vec<(&'static str, ArgValue)> {
        vec![("name", ArgValue::Text(self.0))]
    }
}

#[cfg(not(debug_assertions))]
fn os(tags: &[&str]) -> PrefSource {
    PrefSource::OsList(tags.iter().map(|t| t.to_string()).collect())
}

#[cfg(not(debug_assertions))]
fn neg(sources: &[PrefSource], os_format: Option<&str>) -> Negotiated {
    let ctx = NegotiationCtx {
        sources: sources.to_vec(),
        os_format: os_format.map(str::to_string),
        ..NegotiationCtx::default()
    };
    negotiate(&ctx, Catalogs::embedded().locales())
}

#[test]
fn missing_tr_message_falls_back_to_en() {
    let l = l10n(
        &["tr", "en"],
        &[("tr", "hv-a = A-tr")],
        &[("en", "hv-a = A\nhv-b = B")],
    );
    assert_eq!(l.t_dynamic("hv-b", vec![]), "B");
}

#[cfg(not(debug_assertions))]
#[test]
fn tr_typo_variable_falls_back_in_release() {
    let l = l10n(
        &["tr", "en"],
        &[("tr", "hv-r = { $cuont } satır")],
        &[("en", "hv-r = { $count } rows")],
    );
    assert_eq!(
        l.t_dynamic("hv-r", vec![("count", ArgValue::Count(2))]),
        "\u{2068}2\u{2069} rows"
    );
}

#[cfg(debug_assertions)]
#[test]
fn tr_typo_variable_is_flagged_in_debug() {
    let l = l10n(
        &["tr", "en"],
        &[("tr", "hv-r = { $cuont } satır")],
        &[("en", "hv-r = { $count } rows")],
    );
    assert!(
        l.t_dynamic("hv-r", vec![("count", ArgValue::Count(2))])
            .starts_with("⚠[")
    );
    // The flagged text is the Turkish attempt, unresolved variable and all.
    assert_eq!(
        l.t_dynamic("hv-r", vec![("count", ArgValue::Count(2))]),
        "⚠[\u{2068}{$cuont}\u{2069} satır]"
    );
}

#[test]
fn ui_sink_isolates_and_machine_sink_does_not() {
    const SAVED: MsgRef<NameArg> = MsgRef::new("hv-saved");
    let l = l10n(
        &["tr", "en"],
        &[("tr", "hv-saved = Bağlantı { $name } kaydedildi.")],
        &[("en", "hv-saved = Connection { $name } saved.")],
    );
    assert_eq!(
        l.t(SAVED, NameArg("prod".into())),
        "Bağlantı \u{2068}prod\u{2069} kaydedildi."
    );
    assert_eq!(
        l.t_machine(SAVED, NameArg("prod".into())),
        "Bağlantı prod kaydedildi."
    );
}

#[test]
fn stale_build_is_discarded() {
    let l = L10n::new(rt(1));
    assert!(l.install_if_newer(rt(3)));
    assert!(!l.install_if_newer(rt(2)));
    assert_eq!(l.generation(), 3);
}

#[test]
fn reserved_generations_are_unique_and_increasing() {
    let l = L10n::new(rt(5));
    let a = l.reserve_generation();
    let b = l.reserve_generation();
    assert!(a > 5 && b > a);
    assert!(l.install_if_newer(rt(b)));
    assert!(!l.install_if_newer(rt(a)));
    assert_eq!(l.generation(), b);
}

#[test]
fn typed_ids_reject_wrong_args() {
    trybuild::TestCases::new().compile_fail("tests/ui/wrong_args.rs");
}

#[cfg(not(debug_assertions))]
#[test]
fn cold_parse_en_tr_under_10ms() {
    let c = Catalogs::embedded();
    let n = neg(&[os(&["tr-TR"])], None);
    let f = FormatInputs::new(&n, "UTC");
    let mut t: Vec<Duration> = (0..20u64)
        .map(|g| {
            let s = Instant::now();
            let _ = build_runtime(&c, &n, &f, g);
            s.elapsed()
        })
        .collect();
    t.sort();
    eprintln!("cold_parse_en_tr: median {:?} (20 runs)", t[10]);
    assert!(t[10] <= Duration::from_millis(10), "median {:?}", t[10]);
}

// Beyond the brief's rows: the load itself (embedded scan + parse) also fits the budget.
#[cfg(not(debug_assertions))]
#[test]
fn cold_load_and_build_en_tr_under_10ms() {
    let n = neg(&[os(&["tr-TR"])], None);
    let f = FormatInputs::new(&n, "UTC");
    let mut t: Vec<Duration> = (0..20u64)
        .map(|g| {
            let s = Instant::now();
            let c = Catalogs::embedded();
            let _ = build_runtime(&c, &n, &f, g);
            s.elapsed()
        })
        .collect();
    t.sort();
    eprintln!("cold_load_and_build_en_tr: median {:?} (20 runs)", t[10]);
    assert!(t[10] <= Duration::from_millis(10), "median {:?}", t[10]);
}

// I18N-04 steps 5–9: further rows.

#[test]
fn attribute_falls_back_per_message() {
    const MENU: MsgRef<hacivat_l10n::NoArgs> = MsgRef::new("hv-menu");
    let l = l10n(
        &["tr", "en"],
        &[("tr", "hv-menu = Menü\n    .title = Menü başlığı")],
        &[(
            "en",
            "hv-menu = Menu\n    .title = Menu title\n    .hint = Open the menu",
        )],
    );
    assert_eq!(
        l.t_attr(MENU, "title", hacivat_l10n::NoArgs).as_deref(),
        Some("Menü başlığı")
    );
    assert_eq!(
        l.t_attr(MENU, "hint", hacivat_l10n::NoArgs).as_deref(),
        Some("Open the menu")
    );
    assert_eq!(l.t_attr(MENU, "nope", hacivat_l10n::NoArgs), None);
}

/// A caller error: English needs `$name` too, so it is not the translation's fault.
fn missing_name() -> String {
    let l = l10n(
        &["tr", "en"],
        &[("tr", "hv-saved = Bağlantı { $name } kaydedildi.")],
        &[("en", "hv-saved = Connection { $name } saved.")],
    );
    l.t_machine(
        MsgRef::<hacivat_l10n::NoArgs>::new("hv-saved"),
        hacivat_l10n::NoArgs,
    )
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "hv-saved")]
fn missing_argument_asserts_in_debug() {
    missing_name();
}

#[cfg(not(debug_assertions))]
#[test]
fn missing_argument_keeps_the_translation_in_release() {
    // No fallback: the Turkish text is kept, with the unresolved variable shown.
    assert_eq!(missing_name(), "Bağlantı {$name} kaydedildi.");
}

// Fix round 1 (M1): which bundle answered.

#[test]
fn rendered_reports_the_locale_used_and_fallback() {
    const A: MsgRef<hacivat_l10n::NoArgs> = MsgRef::new("hv-a");
    const B: MsgRef<hacivat_l10n::NoArgs> = MsgRef::new("hv-b");
    let l = l10n(
        &["tr", "en"],
        &[("tr", "hv-a = A-tr")],
        &[("en", "hv-a = A\nhv-b = B")],
    );
    let a = l.t_rendered(A, hacivat_l10n::NoArgs, Sink::Ui);
    assert_eq!(
        (
            a.text.as_str(),
            a.locale_used.to_string(),
            a.fell_back,
            a.missing
        ),
        ("A-tr", "tr".to_string(), false, false)
    );
    let b = l.t_rendered(B, hacivat_l10n::NoArgs, Sink::Ui);
    assert_eq!(
        (
            b.text.as_str(),
            b.locale_used.to_string(),
            b.fell_back,
            b.missing
        ),
        ("B", "en".to_string(), true, false)
    );
}

#[cfg(not(debug_assertions))]
#[test]
fn rendered_reports_structural_fallback_in_release() {
    let l = l10n(
        &["tr", "en"],
        &[("tr", "hv-r = { $cuont } satır")],
        &[("en", "hv-r = { $count } rows")],
    );
    let r = l.t_rendered(MsgRef::<CountArg>::new("hv-r"), CountArg(2), Sink::Machine);
    assert_eq!(
        (r.text.as_str(), r.locale_used.to_string(), r.fell_back),
        ("2 rows", "en".to_string(), true)
    );
}

#[test]
fn rendered_marks_a_missing_message() {
    let l = l10n(&["tr", "en"], &[], &[("en", "hv-a = A")]);
    let r = l.t_rendered(
        MsgRef::<hacivat_l10n::NoArgs>::new("hv-nope"),
        hacivat_l10n::NoArgs,
        Sink::Ui,
    );
    assert!(r.missing && r.fell_back);
    assert_ne!(r.text, "hv-nope");
}

// Fix round 1 (M2): plural variants are chosen by the UI language's CLDR rules.

struct CountArg(u64);

impl IntoFluentArgs for CountArg {
    fn into_args(self) -> Vec<(&'static str, ArgValue)> {
        vec![("count", ArgValue::Count(self.0))]
    }
}

#[test]
fn turkish_plural_selection() {
    const ROWS: MsgRef<CountArg> = MsgRef::new("hv-rows");
    let l = l10n(
        &["tr", "en"],
        &[(
            "tr",
            "hv-rows = { $count ->\n    [one] tek satır\n   *[other] { $count } satır\n}",
        )],
        &[(
            "en",
            "hv-rows = { $count ->\n    [one] one row\n   *[other] { $count } rows\n}",
        )],
    );
    assert_eq!(l.t_machine(ROWS, CountArg(1)), "tek satır");
    assert_eq!(l.t_machine(ROWS, CountArg(2)), "2 satır");
    assert_eq!(l.t_machine(ROWS, CountArg(0)), "0 satır");
}

// Fix round 1 (M9): concurrent switches; the newest reservation always ends up installed.

#[test]
fn concurrent_installs_keep_the_newest_generation() {
    let l = std::sync::Arc::new(L10n::new(rt(0)));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let l = l.clone();
            std::thread::spawn(move || {
                let mut highest = 0;
                for _ in 0..2_000 {
                    let g = l.reserve_generation();
                    highest = highest.max(g);
                    l.install_if_newer(rt(g));
                    assert!(
                        l.generation() >= g,
                        "installed {} < own {g}",
                        l.generation()
                    );
                }
                highest
            })
        })
        .collect();
    let highest = workers
        .into_iter()
        .map(|w| w.join().unwrap())
        .max()
        .unwrap();
    assert_eq!(highest, 4_000);
    assert_eq!(l.generation(), highest);
    assert!(!l.install_if_newer(rt(highest)));
}

#[cfg(debug_assertions)]
#[test]
fn unknown_id_is_bracketed_in_debug() {
    let l = l10n(&["tr", "en"], &[], &[("en", "hv-a = A")]);
    assert_eq!(l.t_dynamic("hv-nope", vec![]), "⟦hv-nope⟧");
}

#[cfg(not(debug_assertions))]
#[test]
fn unknown_id_renders_cm_error_unknown_in_release() {
    let c = Catalogs::embedded();
    let n = negotiated(&["tr", "en"], "tr-TR");
    let l = L10n::new(build_runtime(&c, &n, &FormatInputs::new(&n, "UTC"), 1));
    assert_eq!(
        l.t_dynamic("hv-nope", vec![]),
        "Metin bulunamadı: \u{2068}hv-nope\u{2069}"
    );
}

#[test]
fn generated_ids_render_from_the_embedded_catalogs() {
    use hacivat_l10n::ids::cm;
    let c = Catalogs::embedded();
    let n = negotiated(&["tr", "en"], "tr-TR");
    let l = L10n::new(build_runtime(&c, &n, &FormatInputs::new(&n, "UTC"), 1));
    assert_eq!(l.t(cm::BRAND_HACIVAT_CAPS, hacivat_l10n::NoArgs), "HACIVAT");
    assert_eq!(
        l.t_machine(
            cm::ERROR_UNKNOWN,
            cm::CmErrorUnknown {
                code: hacivat_l10n::Text("hv-x".into())
            }
        ),
        "Metin bulunamadı: hv-x"
    );
    assert_eq!(cm::ERROR_UNKNOWN.id, "cm-error-unknown");
}

#[test]
fn english_layer_ignores_the_ui_chain() {
    let l = l10n(
        &["tr", "en"],
        &[("tr", "hv-a = Tablo")],
        &[("en", "hv-a = Table")],
    );
    assert_eq!(l.t_english("hv-a"), "Table");
    assert_eq!(l.t_dynamic("hv-a", vec![]), "Tablo");
}

#[test]
fn swapped_runtime_changes_language() {
    let c = Catalogs::embedded();
    let en = negotiated(&["en"], "en-US");
    let l = L10n::new(build_runtime(&c, &en, &FormatInputs::new(&en, "UTC"), 1));
    assert_eq!(
        l.t_machine(
            hacivat_l10n::ids::cm::ERROR_UNKNOWN,
            hacivat_l10n::ids::cm::CmErrorUnknown {
                code: hacivat_l10n::Text("x".into())
            }
        ),
        "Text not found: x"
    );
    let tr = negotiated(&["tr", "en"], "tr-TR");
    let g = l.reserve_generation();
    assert!(l.install_if_newer(build_runtime(&c, &tr, &FormatInputs::new(&tr, "UTC"), g)));
    assert_eq!(l.snapshot().format.format_locale, "tr-TR");
    assert_eq!(
        l.t_machine(
            hacivat_l10n::ids::cm::ERROR_UNKNOWN,
            hacivat_l10n::ids::cm::CmErrorUnknown {
                code: hacivat_l10n::Text("x".into())
            }
        ),
        "Metin bulunamadı: x"
    );
}

#[test]
fn renders_on_other_threads_during_a_swap() {
    let l = std::sync::Arc::new(l10n(
        &["tr", "en"],
        &[("tr", "hv-a = Tablo")],
        &[("en", "hv-a = Table")],
    ));
    let reader = {
        let l = l.clone();
        std::thread::spawn(move || {
            (0..200)
                .map(|_| l.t_dynamic("hv-a", vec![]))
                .all(|s| s == "Tablo" || s == "Table")
        })
    };
    let n = negotiated(&["en"], "en-US");
    let c = Catalogs::from_memory(&[
        ("en/locale.toml", EN_TOML),
        ("en/hacivat-ui.ftl", "hv-a = Table"),
    ]);
    let g = l.reserve_generation();
    assert!(l.install_if_newer(build_runtime(&c, &n, &FormatInputs::new(&n, "UTC"), g)));
    assert!(reader.join().unwrap());
    assert_eq!(l.t_dynamic("hv-a", vec![]), "Table");
}
