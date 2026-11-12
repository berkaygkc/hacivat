//! Message resolution with per-message fallback, sinks, typed IDs and the atomic swap
//! (I18N-04, I18N-07). The format locale is `en-US` until Task 9 adds the ICU formatters.

#[cfg(not(debug_assertions))]
use std::time::{Duration, Instant};

use hacivat_l10n::{
    ArgValue, Catalogs, FormatInputs, IntoFluentArgs, L10n, LanguageIdentifier, MsgRef, Negotiated,
    Runtime, build_runtime,
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

#[test]
fn missing_argument_keeps_the_translation() {
    // A caller error (English needs `$name` too) is not the translation's fault: no fallback.
    let l = l10n(
        &["tr", "en"],
        &[("tr", "hv-saved = Bağlantı { $name } kaydedildi.")],
        &[("en", "hv-saved = Connection { $name } saved.")],
    );
    assert_eq!(
        l.t_machine(
            MsgRef::<hacivat_l10n::NoArgs>::new("hv-saved"),
            hacivat_l10n::NoArgs
        ),
        "Bağlantı {$name} kaydedildi."
    );
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
