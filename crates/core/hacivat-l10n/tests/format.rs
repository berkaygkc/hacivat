//! ICU4X-backed `NUMBER`, `DATETIME`, `LIST` and the `hv_format` formatter (I18N-05), the locale
//! settings model (I18N-06) and display casing (INV-3, F33).

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use hacivat_l10n::{
    ArgValue, CatalogLocale, Catalogs, ExactDecimal, FormatCtx, FormatInputs, IntoFluentArgs, L10n,
    LanguageIdentifier, ListValue, MsgRef, Negotiated, OsLocale, Sink, WhenValue, build_runtime,
    case_for_display, effective, install_functions, negotiate, with_format_ctx,
};
use hacivat_settings::{LanguagePref, Settings, TimeZonePref};
use unic_langid::langid;

const SAME: &str = "{ $count -> [one] { $count } row *[other] { $count } rows }";

type Arg = (&'static str, FluentValue<'static>);

fn n(v: f64) -> Arg {
    ("n", FluentValue::from(v))
}

/// Days since 1970-01-01 of a proleptic Gregorian date (H. Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// An instant from `YYYY-MM-DDTHH:MM:SSZ`.
fn t(iso: &str) -> Arg {
    let p = |r: std::ops::Range<usize>| iso[r].parse::<i64>().unwrap();
    let days = days_from_civil(p(0..4), p(5..7), p(8..10));
    let secs = days * 86_400 + p(11..13) * 3600 + p(14..16) * 60 + p(17..19);
    let when = WhenValue::Instant {
        unix_ms: secs * 1000,
    };
    ("t", FluentValue::Custom(Box::new(when)))
}

fn local(year: i32, month: u8, day: u8, hour: u8, minute: u8) -> Arg {
    let when = WhenValue::Local {
        year,
        month,
        day,
        hour,
        minute,
        second: 0,
    };
    ("t", FluentValue::Custom(Box::new(when)))
}

fn l(items: &[&str]) -> Arg {
    let items = items.iter().map(|s| s.to_string()).collect();
    ("l", FluentValue::Custom(Box::new(ListValue(items))))
}

/// FTL for `id = pattern`. The rows write select expressions on one line for brevity; FTL needs
/// each variant, and the closing brace, on a line of its own.
fn ftl(id: &str, pattern: &str) -> String {
    let pattern = match pattern.strip_suffix('}') {
        Some(body) if pattern.contains("->") => format!(
            "{}\n}}",
            body.trim_end()
                .replace(" *[", "\n   *[")
                .replace(" [", "\n    [")
        ),
        _ => pattern.to_string(),
    };
    format!("{id} = {pattern}\n")
}

/// Formats `pattern` in an `en` bundle (no isolation marks) with format locale `locale`, UTC.
fn fmt(locale: &str, pattern: &str, arg: Arg) -> String {
    fmt_tz(locale, "UTC", pattern, arg)
}

fn fmt_tz(locale: &str, tz: &str, pattern: &str, arg: Arg) -> String {
    fmt_in(langid!("en"), locale, tz, pattern, arg)
}

fn fmt_in(ui: LanguageIdentifier, locale: &str, tz: &str, pattern: &str, arg: Arg) -> String {
    let mut b = FluentBundle::new_concurrent(vec![ui]);
    b.set_use_isolating(false);
    install_functions(&mut b);
    b.add_resource(FluentResource::try_new(ftl("x", pattern)).unwrap())
        .unwrap();
    let ctx = FormatCtx {
        format_locale: locale.into(),
        time_zone: tz.into(),
        generation: 1,
    };
    let mut args = FluentArgs::new();
    args.set(arg.0, arg.1);
    let msg = b.get_message("x").unwrap();
    let mut errors = Vec::new();
    let out = with_format_ctx(&ctx, || {
        b.format_pattern(msg.value().unwrap(), Some(&args), &mut errors)
            .into_owned()
    });
    assert!(errors.is_empty(), "{errors:?}");
    out
}

const EN_TOML: &str = "tag = \"en\"\nnative_name = \"English\"\nenglish_name = \"English\"\nshort_name = \"EN\"\ndirection = \"ltr\"\nstatus = \"stable\"\n";
const TR_TOML: &str = "tag = \"tr\"\nnative_name = \"Türkçe\"\nenglish_name = \"Turkish\"\nshort_name = \"TR\"\ndirection = \"ltr\"\nstatus = \"beta\"\n";

struct Args(Vec<(&'static str, ArgValue)>);

impl IntoFluentArgs for Args {
    fn into_args(self) -> Vec<(&'static str, ArgValue)> {
        self.0
    }
}

/// Renders `id` (defined as `pattern` in both `en` and `tr`) through a runtime whose chain is
/// `ui` and whose format locale is `format_locale`, machine sink.
fn render_with(
    ui: &[&str],
    format_locale: &str,
    id: &'static str,
    pattern: &str,
    args: Vec<(&'static str, ArgValue)>,
) -> String {
    let src = ftl(id, pattern);
    let files = [
        ("en/locale.toml", EN_TOML),
        ("tr/locale.toml", TR_TOML),
        ("en/hacivat-ui.ftl", src.as_str()),
        ("tr/hacivat-ui.ftl", src.as_str()),
    ];
    let cat = Catalogs::from_memory(&files);
    let neg = Negotiated {
        chain: ui.iter().map(|t| t.parse().unwrap()).collect(),
        format_locale: format_locale.into(),
        diags: Vec::new(),
    };
    let rt = build_runtime(&cat, &neg, &FormatInputs::new(&neg, "UTC"), 1);
    L10n::new(rt)
        .t_rendered(MsgRef::<Args>::new(id), Args(args), Sink::Machine)
        .text
}

fn render(ui: &[&str], format_locale: &str, id: &'static str, pattern: &str, count: u64) -> String {
    render_with(
        ui,
        format_locale,
        id,
        pattern,
        vec![("count", ArgValue::Count(count))],
    )
}

fn render_tr(pattern: &str, count: f64, format_locale: &str) -> String {
    let dec = ArgValue::Decimal(ExactDecimal(format!("{count}")));
    render_with(
        &["tr", "en"],
        format_locale,
        "hv-r",
        pattern,
        vec![("count", dec)],
    )
}

fn settings_lang(tag: &str) -> Settings {
    let mut s = Settings::default();
    s.ui.language = LanguagePref::Tag(tag.into());
    s
}

fn settings_tz(zone: &str) -> Settings {
    let mut s = Settings::default();
    s.ui.time_zone = TimeZonePref::Iana(zone.into());
    s
}

fn os(languages: &[&str], format: Option<&str>) -> OsLocale {
    OsLocale {
        languages: languages.iter().map(|s| s.to_string()).collect(),
        format: format.map(str::to_string),
        time_zone: None,
        env: Vec::new(),
    }
}

fn os_tz(languages: &[&str], zone: &str) -> OsLocale {
    OsLocale {
        time_zone: Some(zone.into()),
        ..os(languages, None)
    }
}

fn avail() -> Vec<CatalogLocale> {
    Catalogs::embedded().locales().to_vec()
}

// --- The brief's rows (I18N-05, I18N-06 normative values) ---

#[test]
fn number_tr_and_en() {
    assert_eq!(fmt("tr-TR", "{ NUMBER($n) }", n(1234.5)), "1.234,5");
    assert_eq!(fmt("en-US", "{ NUMBER($n) }", n(1234.5)), "1,234.5");
}

#[test]
fn percent_patterns() {
    assert_eq!(
        fmt("tr-TR", "{ NUMBER($n, style: \"percent\") }", n(0.25)),
        "%25"
    );
    assert_eq!(
        fmt("en-US", "{ NUMBER($n, style: \"percent\") }", n(0.25)),
        "25%"
    );
}

#[test]
fn ports_never_grouped() {
    assert_eq!(
        fmt("tr-TR", "{ NUMBER($n, useGrouping: \"false\") }", n(5432.0)),
        "5432"
    );
}

#[test]
fn dates_in_istanbul() {
    assert_eq!(
        fmt_tz(
            "tr-TR",
            "Europe/Istanbul",
            "{ DATETIME($t, dateStyle: \"long\") }",
            t("2026-11-11T14:05:09Z")
        ),
        "11 Kasım 2026"
    );
    assert_eq!(
        fmt_tz(
            "tr-TR",
            "Europe/Istanbul",
            "{ DATETIME($t, dateStyle: \"short\") }",
            t("2026-11-11T14:05:09Z")
        ),
        "11.11.2026"
    );
}

#[test]
fn instant_converts_to_zone_and_local_does_not() {
    assert_eq!(
        fmt_tz(
            "tr-TR",
            "Europe/Istanbul",
            "{ DATETIME($t, timeStyle: \"short\") }",
            t("2026-11-11T14:05:09Z")
        ),
        "17:05"
    );
    assert_eq!(
        fmt_tz(
            "tr-TR",
            "Europe/Istanbul",
            "{ DATETIME($t, timeStyle: \"short\") }",
            local(2026, 11, 11, 14, 5)
        ),
        "14:05"
    );
}

// ## m2b-t22-live (controller ruling): list conjunctions follow the UI language, the language
// of the bundle that renders the sentence; the regional format never changes them.
#[test]
fn lists() {
    assert_eq!(
        fmt_in(
            langid!("tr"),
            "en-US",
            "UTC",
            "{ LIST($l) }",
            l(&["a", "b", "c"])
        ),
        "a, b ve c"
    );
    assert_eq!(
        fmt_in(
            langid!("en"),
            "tr-TR",
            "UTC",
            "{ LIST($l) }",
            l(&["a", "b", "c"])
        ),
        "a, b, and c"
    );
    // A list argument placed straight into a sentence too.
    assert_eq!(
        fmt_in(langid!("tr"), "en-US", "UTC", "{ $l }", l(&["a", "b"])),
        "a ve b"
    );
    assert_eq!(
        fmt_in(langid!("en"), "tr-TR", "UTC", "{ $l }", l(&["a", "b"])),
        "a and b"
    );
}

#[test]
fn plural_by_ui_language_digits_by_format_locale() {
    assert_eq!(render(&["en"], "tr-TR", "hv-r", SAME, 1234), "1.234 rows");
    assert_eq!(render(&["en"], "tr-TR", "hv-r", SAME, 1), "1 row");
}

#[test]
fn turkish_count_message() {
    assert_eq!(
        render_tr("{ $count } satır", 1234.5, "tr-TR"),
        "1.234,5 satır"
    );
}

#[test]
fn unknown_option_keeps_default_formatting() {
    assert_eq!(
        fmt("en-US", "{ NUMBER($n, currency: \"TRY\") }", n(3.0)),
        "3"
    );
}

#[test]
fn english_ui_turkish_region_is_primary_case() {
    let e = effective(&settings_lang("en"), &os(&["tr-TR"], Some("tr-TR")), false);
    assert_eq!(negotiate(&e.ctx, &avail()).format_locale, "tr-TR");
}

#[test]
fn time_zone_setting_beats_os() {
    let e = effective(
        &settings_tz("Europe/Istanbul"),
        &os_tz(&["en-US"], "America/New_York"),
        false,
    );
    assert_eq!(e.time_zone, "Europe/Istanbul");
    let e = effective(
        &Settings::default(),
        &os_tz(&["en-US"], "America/New_York"),
        false,
    );
    assert_eq!(e.time_zone, "America/New_York");
}

#[test]
fn display_casing_is_turkish_aware() {
    assert_eq!(case_for_display("bilgi", &langid!("tr")), "BİLGİ");
    assert_eq!(case_for_display("bilgi", &langid!("en")), "BILGI");
}

// --- I18N-05 edge cases and test rows beyond the brief ---

#[test]
fn ui_turkish_with_us_format_groups_the_us_way() {
    // Edge 2: `tr` UI + `en-US` format.
    assert_eq!(
        render(&["tr", "en"], "en-US", "hv-r", "{ $count } satır", 1234),
        "1,234 satır"
    );
}

#[test]
fn decimals_select_plurals_by_ui_language() {
    // Edge 3: 1 with one fraction digit is `other` in English and `one` in Turkish.
    let p = "{ NUMBER($n, minimumFractionDigits: 1) -> [one] { NUMBER($n, minimumFractionDigits: 1) } satır *[other] { NUMBER($n, minimumFractionDigits: 1) } satırlar }";
    assert_eq!(
        fmt_in(langid!("tr"), "tr-TR", "UTC", p, n(1.0)),
        "1,0 satır"
    );
    let p = "{ NUMBER($n, minimumFractionDigits: 1) -> [one] { NUMBER($n, minimumFractionDigits: 1) } row *[other] { NUMBER($n, minimumFractionDigits: 1) } rows }";
    assert_eq!(fmt_in(langid!("en"), "en-US", "UTC", p, n(1.0)), "1.0 rows");
}

#[test]
fn exact_decimals_select_plurals_through_number() {
    // An `ExactDecimal` passed through `NUMBER` becomes a Fluent number, so it selects.
    let p = "{ NUMBER($count) -> [one] tek satır *[other] { NUMBER($count) } satır }";
    let one = ArgValue::Decimal(ExactDecimal("1".into()));
    assert_eq!(
        render_with(&["tr", "en"], "tr-TR", "hv-r", p, vec![("count", one)]),
        "tek satır"
    );
    let many = ArgValue::Decimal(ExactDecimal("1234.50".into()));
    assert_eq!(
        render_with(&["tr", "en"], "tr-TR", "hv-r", p, vec![("count", many)]),
        "1.234,50 satır"
    );
}

#[test]
fn exact_decimals_are_never_rounded_through_f64() {
    let dec = ArgValue::Decimal(ExactDecimal("12345678901234567890.123456789".into()));
    assert_eq!(
        render_with(&["en"], "tr-TR", "hv-r", "{ $count }", vec![("count", dec)]),
        "12.345.678.901.234.567.890,123456789"
    );
}

#[test]
fn not_finite_and_huge_numbers() {
    // Edges 4–6.
    assert_eq!(fmt("tr-TR", "{ $n }", n(f64::INFINITY)), "∞");
    assert_eq!(fmt("en-US", "{ $n }", n(f64::NEG_INFINITY)), "-∞");
    assert_eq!(fmt("tr-TR", "{ $n }", n(f64::NAN)), "NaN");
    assert_eq!(
        fmt("en-US", "{ $n }", n(1e21)),
        "1,000,000,000,000,000,000,000"
    );
    assert_eq!(fmt("tr-TR", "{ $n }", n(-0.0)), "0");
}

#[test]
fn percent_with_fraction_digits() {
    // Edge 7.
    let p = "{ NUMBER($n, style: \"percent\", maximumFractionDigits: 1) }";
    assert_eq!(fmt("tr-TR", p, n(0.255)), "%25,5");
    assert_eq!(fmt("en-US", p, n(0.255)), "25.5%");
    assert_eq!(
        fmt("tr-TR", "{ NUMBER($n, style: \"percent\") }", n(-0.25)),
        "-%25"
    );
}

#[test]
fn fraction_digit_options() {
    assert_eq!(
        fmt(
            "en-US",
            "{ NUMBER($n, maximumFractionDigits: 2) }",
            n(2.345)
        ),
        "2.35"
    );
    assert_eq!(
        fmt("tr-TR", "{ NUMBER($n, minimumFractionDigits: 2) }", n(5.0)),
        "5,00"
    );
    // Intl's default for decimals is at most three fraction digits.
    assert_eq!(fmt("en-US", "{ $n }", n(1.23456)), "1.235");
}

#[test]
fn numbering_system_extension_is_honoured() {
    // Edge 11: Arabic-Indic digits when the format locale asks for them.
    assert_eq!(fmt("tr-TR-u-nu-arab", "{ $n }", n(12.0)), "١٢");
}

#[test]
fn medium_and_combined_date_time_styles() {
    let instant = || t("2026-11-11T14:05:09Z");
    assert_eq!(
        fmt_tz(
            "tr-TR",
            "Europe/Istanbul",
            "{ DATETIME($t, timeStyle: \"medium\") }",
            instant()
        ),
        "17:05:09"
    );
    assert_eq!(
        fmt_tz(
            "tr-TR",
            "UTC",
            "{ DATETIME($t, dateStyle: \"short\", timeStyle: \"short\") }",
            local(2026, 11, 11, 14, 5)
        ),
        "11.11.2026 14:05"
    );
    // Without styles a date shows as a short date (Intl's default).
    assert_eq!(
        fmt("en-US", "{ $t }", local(2026, 11, 11, 14, 5)),
        "11/11/26"
    );
}

#[test]
fn datetime_takes_iso_strings_and_prints_others_raw() {
    let iso = ("t", FluentValue::from("2026-11-11T14:05:09Z"));
    assert_eq!(
        fmt_tz(
            "tr-TR",
            "Europe/Istanbul",
            "{ DATETIME($t, timeStyle: \"short\") }",
            iso
        ),
        "17:05"
    );
    // Edge 9: not ISO 8601 → the raw string.
    let raw = ("t", FluentValue::from("yesterday"));
    assert_eq!(
        fmt("tr-TR", "{ DATETIME($t, dateStyle: \"short\") }", raw),
        "yesterday"
    );
}

#[test]
fn unknown_time_zone_formats_in_utc() {
    // Edge 10.
    assert_eq!(
        fmt_tz(
            "tr-TR",
            "Mars/Olympus_Mons",
            "{ DATETIME($t, timeStyle: \"short\") }",
            t("2026-11-11T14:05:09Z")
        ),
        "14:05"
    );
}

#[test]
fn list_types_and_sizes() {
    // ## m2b-t22-live: by the UI language (see `lists`).
    assert_eq!(
        fmt_in(
            langid!("tr"),
            "en-US",
            "UTC",
            "{ LIST($l, type: \"or\") }",
            l(&["a", "b"])
        ),
        "a veya b"
    );
    assert_eq!(
        fmt_in(
            langid!("en"),
            "tr-TR",
            "UTC",
            "{ LIST($l, type: \"or\") }",
            l(&["a", "b"])
        ),
        "a or b"
    );
    // Edge 12.
    assert_eq!(fmt("tr-TR", "{ LIST($l) }", l(&["a"])), "a");
    assert_eq!(fmt("tr-TR", "{ LIST($l) }", l(&[])), "");
}

#[test]
fn format_context_is_re_entrant() {
    let tr = FormatCtx {
        format_locale: "tr-TR".into(),
        time_zone: "UTC".into(),
        generation: 3,
    };
    let en = FormatCtx {
        format_locale: "en-US".into(),
        ..tr.clone()
    };
    let mut b = FluentBundle::new_concurrent(vec![langid!("en")]);
    b.set_use_isolating(false);
    install_functions(&mut b);
    b.add_resource(FluentResource::try_new("x = { $n }\n".to_string()).unwrap())
        .unwrap();
    let mut args = FluentArgs::new();
    args.set("n", 1234.5);
    let render = || {
        let msg = b.get_message("x").unwrap();
        b.format_pattern(msg.value().unwrap(), Some(&args), &mut Vec::new())
            .into_owned()
    };
    let (outer_before, inner, outer_after) = with_format_ctx(&tr, || {
        let before = render();
        let inner = with_format_ctx(&en, render);
        (before, inner, render())
    });
    assert_eq!(
        (outer_before.as_str(), inner.as_str(), outer_after.as_str()),
        ("1.234,5", "1,234.5", "1.234,5")
    );
}

// --- I18N-06: the settings model ---

#[test]
fn ui_language_setting_comes_first() {
    let e = effective(&settings_lang("tr"), &os(&["en-US"], None), false);
    let neg = negotiate(&e.ctx, &avail());
    assert_eq!(neg.chain, vec![langid!("tr"), langid!("en")]);
    // An explicit UI language never sets the format locale.
    assert_eq!(neg.format_locale, "en-US");
}

#[test]
fn regional_format_setting_beats_the_os() {
    let mut s = Settings::default();
    s.ui.regional_format = LanguagePref::Tag("tr-TR".into());
    let e = effective(&s, &os(&["en-US"], Some("en-US")), false);
    assert_eq!(e.ctx.format_pref.as_deref(), Some("tr-TR"));
    assert_eq!(negotiate(&e.ctx, &avail()).format_locale, "tr-TR");
}

#[test]
fn os_list_and_environment_feed_negotiation() {
    let o = OsLocale {
        env: vec![("LANG", "tr_TR.UTF-8".into())],
        ..os(&[], None)
    };
    let neg = negotiate(&effective(&Settings::default(), &o, false).ctx, &avail());
    assert_eq!(neg.chain, vec![langid!("tr"), langid!("en")]);
    assert_eq!(neg.format_locale, "tr-TR");
    // `HACIVAT_LANG` beats the OS list.
    let o = OsLocale {
        env: vec![("HACIVAT_LANG", "en".into())],
        ..os(&["tr-TR"], None)
    };
    let neg = negotiate(&effective(&Settings::default(), &o, false).ctx, &avail());
    assert_eq!(neg.chain, vec![langid!("en")]);
}

#[test]
fn time_zone_falls_back_to_utc() {
    let e = effective(&Settings::default(), &os(&["en-US"], None), false);
    assert_eq!(e.time_zone, "UTC");
    // An unknown zone in the settings is treated as `system`.
    let e = effective(
        &settings_tz("Not/A_Zone"),
        &os_tz(&["en-US"], "Europe/Istanbul"),
        false,
    );
    assert_eq!(e.time_zone, "Europe/Istanbul");
}

#[test]
fn pseudo_permission_is_passed_through() {
    let e = effective(&settings_lang("en-XA"), &os(&["en-US"], None), true);
    assert!(e.ctx.allow_pseudo);
    assert_eq!(
        negotiate(&e.ctx, &avail()).chain,
        vec![langid!("en-XA"), langid!("en")]
    );
}

#[test]
fn detect_reads_the_time_zone_and_languages_without_panicking() {
    let o = OsLocale::detect();
    assert!(o.env.iter().all(|(_, v)| !v.is_empty()));
    assert!(o.time_zone.as_deref().is_none_or(|z| !z.is_empty()));
}

#[test]
fn display_casing_handles_more_than_ascii() {
    assert_eq!(case_for_display("ılık şiir", &langid!("tr")), "ILIK ŞİİR");
    assert_eq!(case_for_display("straße", &langid!("de")), "STRASSE");
    assert_eq!(case_for_display("istanbul", &langid!("az")), "İSTANBUL");
}

// --- Fix round 1 ---

#[test]
fn daylight_saving_time_is_applied() {
    // I1: New York observes DST (UTC−4 in July, UTC−5 in January); Istanbul had DST until 2016.
    let p = "{ DATETIME($t, timeStyle: \"short\") }";
    assert_eq!(
        fmt_tz("en-US", "America/New_York", p, t("2026-07-01T16:00:00Z")),
        "12:00\u{202f}PM"
    );
    assert_eq!(
        fmt_tz("en-US", "America/New_York", p, t("2026-01-15T16:00:00Z")),
        "11:00\u{202f}AM"
    );
    assert_eq!(
        fmt_tz("tr-TR", "Europe/Istanbul", p, t("2015-07-01T12:00:00Z")),
        "15:00"
    );
    assert_eq!(
        fmt_tz("tr-TR", "Europe/Istanbul", p, t("2015-01-15T12:00:00Z")),
        "14:00"
    );
}

#[test]
fn turkish_and_english_grouping_of_a_large_decimal() {
    // M7: L3 pinned literally.
    assert_eq!(fmt("tr-TR", "{ $n }", n(110190.85)), "110.190,85");
    assert_eq!(fmt("en-US", "{ $n }", n(110190.85)), "110,190.85");
}

fn format_locale_for(os_list: &[&str], ui: &str) -> String {
    let e = effective(&settings_lang(ui), &os(os_list, None), false);
    negotiate(&e.ctx, &avail()).format_locale
}

#[test]
fn english_ui_with_turkish_region_formats_as_turkish() {
    // I2: macOS reports "English, region Turkey" as `en-TR`; CLDR has no `en-TR` data, and
    // Turkey's language is Turkish, so the format locale is `tr-TR`. The UI stays English.
    let e = effective(&settings_lang("en"), &os(&["en-TR"], None), false);
    let neg = negotiate(&e.ctx, &avail());
    assert_eq!(neg.chain, vec![langid!("en")]);
    assert_eq!(neg.format_locale, "tr-TR");
    assert_eq!(fmt(&neg.format_locale, "{ $n }", n(1234.5)), "1.234,5");
    assert_eq!(
        fmt_tz(
            &neg.format_locale,
            "Europe/Istanbul",
            "{ DATETIME($t, dateStyle: \"long\") }",
            t("2026-11-11T14:05:09Z")
        ),
        "11 Kasım 2026"
    );
    // The same through `ui.regional_format`, and for another language with a Turkish region.
    let mut s = settings_lang("en");
    s.ui.regional_format = LanguagePref::Tag("en-TR".into());
    let e = effective(&s, &os(&["en-US"], None), false);
    assert_eq!(negotiate(&e.ctx, &avail()).format_locale, "tr-TR");
    assert_eq!(format_locale_for(&["fr-TR"], "en"), "tr-TR");
}

#[test]
fn regional_pairs_with_their_own_data_are_kept() {
    for tag in [
        "en-GB", "de-AT", "en-US", "tr-TR", "fr-CA", "fr-CH", "fr-BE", "es-US", "en-HK", "ca-ES",
        "en-DE",
    ] {
        assert_eq!(format_locale_for(&[tag], "en"), tag);
    }
    // Extensions survive the mapping.
    assert_eq!(
        format_locale_for(&["en-TR-u-nu-arab"], "en"),
        "tr-TR-u-nu-arab"
    );
}
